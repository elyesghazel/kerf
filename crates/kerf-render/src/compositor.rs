//! The compositor: turns "project at time t" into one RGBA frame.
//!
//! The preview and the exporter run this same code, so what you see while
//! editing is what gets rendered. Pixel work is split across cores with rayon;
//! decoding happens in ffmpeg child processes running alongside.

use std::collections::HashMap;

use kerf_core::{Clip, ClipContent, ClipId, ColorGrade, MediaItem, Project, TextStyle, Time, TrackKind, Transform};
use kerf_media::{Frame, VideoStream};
use rayon::prelude::*;

use crate::text::TextRenderer;

/// Fit `(mw, mh)` inside `(w, h)` keeping aspect; even sizes for ffmpeg.
pub fn fit(mw: u32, mh: u32, w: u32, h: u32) -> (u32, u32) {
    let s = (w as f64 / mw.max(1) as f64).min(h as f64 / mh.max(1) as f64);
    let fw = ((mw as f64 * s).round() as u32).max(2) & !1;
    let fh = ((mh as f64 * s).round() as u32).max(2) & !1;
    (fw.min(w.max(2)), fh.min(h.max(2)))
}

struct Reader {
    path: String,
    stream: Option<VideoStream>,
    frame: Frame,
    /// Source time of the frame in `frame`, if valid.
    have: Option<f64>,
    scratch: Frame,
    used: u64,
}

pub struct Compositor {
    readers: HashMap<ClipId, Reader>,
    stills: HashMap<(String, u32, u32), Frame>,
    text: TextRenderer,
    /// Decode proxies instead of originals when a media item has one.
    pub use_proxies: bool,
    /// Sharper scaling (export).
    pub hq: bool,
    /// GPU decode for long-running streams.
    pub accel: bool,
    tick: u64,
}

impl Default for Compositor {
    fn default() -> Self {
        Compositor {
            readers: HashMap::new(),
            stills: HashMap::new(),
            text: TextRenderer::default(),
            use_proxies: true,
            hq: false,
            accel: false,
            tick: 0,
        }
    }
}

impl Compositor {
    pub fn for_export() -> Self {
        let mut c = Compositor::default();
        c.use_proxies = false;
        c.hq = true;
        c.accel = true;
        c
    }

    /// Drop all decoders (e.g. when the preview size changes).
    pub fn reset(&mut self) {
        self.readers.clear();
        self.stills.clear();
    }

    /// Render the project at `t` into `out` (any size; the output format is
    /// scaled to it).
    pub fn render(&mut self, project: &Project, t: Time, out: &mut Frame) {
        self.tick += 1;
        out.fill(project.timeline.background);
        let fps = project.timeline.fps;
        let layers: Vec<Clip> = project
            .timeline
            .tracks
            .iter()
            .filter(|tr| tr.kind == TrackKind::Video && !tr.hidden)
            .filter_map(|tr| tr.clip_at(t).cloned())
            .collect();
        // Scale factor between the project format and this output.
        let sx = out.w as f32 / project.timeline.width as f32;
        for clip in layers {
            let alpha = (clip.opacity * clip.fade_gain(t)).clamp(0.0, 1.0);
            if alpha <= 0.001 {
                continue;
            }
            match &clip.content {
                ClipContent::Media { media } => {
                    let Some(item) = project.media_item(*media) else { continue };
                    if !item.has_video {
                        continue;
                    }
                    let src_t = clip.source_time_at(t).unwrap_or(clip.source_in).as_secs_f64();
                    let (fw, fh) = fit(item.width, item.height, out.w, out.h);
                    if let Some(frame) = self.frame_for(&clip, item, src_t, fw, fh, fps) {
                        draw_frame(out, frame, &clip.transform, &clip.grade, alpha);
                    }
                }
                ClipContent::Solid { color } => {
                    let c = *color;
                    let a = alpha * c[3] as f32 / 255.0;
                    out.data.par_chunks_mut(4).for_each(|px| blend_px(px, [c[0], c[1], c[2]], a));
                }
                ClipContent::Text(style) => {
                    let style = style.clone();
                    draw_text(&mut self.text, out, &style, alpha, sx);
                }
            }
        }
        // Forget decoders for clips that left the playhead.
        let tick = self.tick;
        self.readers.retain(|_, r| tick - r.used < 90);
    }

    fn frame_for(&mut self, clip: &Clip, item: &MediaItem, src_t: f64, w: u32, h: u32, fps: f64) -> Option<&Frame> {
        let path = match (&item.proxy, self.use_proxies) {
            (Some(p), true) if std::path::Path::new(p).exists() => p.clone(),
            _ => item.path.clone(),
        };
        if item.is_image {
            let key = (path.clone(), w, h);
            if !self.stills.contains_key(&key) {
                let f = kerf_media::grab_frame(&path, 0.0, w, h, true).ok()?;
                self.stills.insert(key.clone(), f);
            }
            return self.stills.get(&key);
        }
        let tick = self.tick;
        let r = self.readers.entry(clip.id).or_insert_with(|| Reader {
            path: path.clone(),
            stream: None,
            frame: Frame::new(w, h),
            have: None,
            scratch: Frame::new(w, h),
            used: tick,
        });
        r.used = tick;
        if r.path != path || r.frame.w != w || r.frame.h != h {
            *r = Reader {
                path: path.clone(),
                stream: None,
                frame: Frame::new(w, h),
                have: None,
                scratch: Frame::new(w, h),
                used: tick,
            };
        }
        let half = 0.5 / fps;
        if let Some(have) = r.have {
            if (src_t - have).abs() < half {
                return Some(&r.frame);
            }
        }
        // Reuse the running stream if `src_t` is a little ahead of it.
        let reusable = r
            .stream
            .as_ref()
            .map(|s| !s.eof && src_t >= s.next_time() - half && src_t < s.next_time() + 3.0)
            .unwrap_or(false);
        if !reusable {
            r.stream = VideoStream::open(&path, src_t, w, h, fps, self.hq, self.accel).ok();
        }
        let s = r.stream.as_mut()?;
        s.skip_to(src_t, &mut r.scratch);
        let at = s.next_time();
        if s.read(&mut r.frame) {
            r.have = Some(at);
            Some(&r.frame)
        } else {
            // Past the end of the file: hold the last good frame.
            r.have.map(|_| &r.frame)
        }
    }
}

#[inline]
fn blend_px(px: &mut [u8], c: [u8; 3], a: f32) {
    let ia = 1.0 - a;
    px[0] = (px[0] as f32 * ia + c[0] as f32 * a) as u8;
    px[1] = (px[1] as f32 * ia + c[1] as f32 * a) as u8;
    px[2] = (px[2] as f32 * ia + c[2] as f32 * a) as u8;
}

/// Per-channel lookup tables for the parts of a grade that act per channel.
struct GradeLut {
    pre: [[u8; 256]; 3],
    post: [u8; 256],
    sat: f32,
    vignette: f32,
    identity: bool,
}

impl GradeLut {
    fn new(g: &ColorGrade) -> Self {
        let mut pre = [[0u8; 256]; 3];
        let gain = 2f32.powf(g.exposure);
        let shift = [g.temperature * 0.08, -g.tint * 0.06, -g.temperature * 0.08];
        for (ch, lut) in pre.iter_mut().enumerate() {
            for (i, v) in lut.iter_mut().enumerate() {
                let mut c = i as f32 / 255.0;
                c = c * gain + shift[ch];
                c = (c - 0.5) * g.contrast + 0.5 + g.brightness * 0.5;
                *v = (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
        let mut post = [0u8; 256];
        let inv = 1.0 / g.gamma.clamp(0.1, 5.0);
        for (i, v) in post.iter_mut().enumerate() {
            *v = ((i as f32 / 255.0).powf(inv) * 255.0 + 0.5) as u8;
        }
        GradeLut {
            pre,
            post,
            sat: g.saturation,
            vignette: g.vignette,
            identity: g.is_identity(),
        }
    }

    #[inline]
    fn apply(&self, p: [u8; 3]) -> [u8; 3] {
        let r = self.pre[0][p[0] as usize] as f32;
        let g = self.pre[1][p[1] as usize] as f32;
        let b = self.pre[2][p[2] as usize] as f32;
        let (r, g, b) = if (self.sat - 1.0).abs() > 1e-3 {
            let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            (l + (r - l) * self.sat, l + (g - l) * self.sat, l + (b - l) * self.sat)
        } else {
            (r, g, b)
        };
        [
            self.post[r.clamp(0.0, 255.0) as usize],
            self.post[g.clamp(0.0, 255.0) as usize],
            self.post[b.clamp(0.0, 255.0) as usize],
        ]
    }
}

/// Draw a decoded frame (already fitted to the output) with transform,
/// grade and opacity.
fn draw_frame(out: &mut Frame, src: &Frame, tf: &Transform, grade: &ColorGrade, alpha: f32) {
    let lut = GradeLut::new(grade);
    let (ow, oh) = (out.w as i32, out.h as i32);
    let (sw, sh) = (src.w as f32, src.h as f32);
    let simple = tf.is_identity();
    let (cx, cy) = (ow as f32 / 2.0 + tf.x * ow as f32, oh as f32 / 2.0 + tf.y * oh as f32);
    let scale = tf.scale.max(0.01);
    let (sin, cos) = (-tf.rotation.to_radians()).sin_cos();
    // Crop window in source pixels.
    let (cl, cr) = (tf.crop_left * sw, sw - tf.crop_right * sw);
    let (ct, cb) = (tf.crop_top * sh, sh - tf.crop_bottom * sh);
    let vig = lut.vignette;
    let row_bytes = out.w as usize * 4;
    let src_row = src.w as usize * 4;
    out.data.par_chunks_mut(row_bytes).enumerate().for_each(|(y, row)| {
        let y = y as i32;
        if simple {
            let oy = (oh - src.h as i32) / 2;
            let ox = (ow - src.w as i32) / 2;
            let sy = y - oy;
            if sy < 0 || sy >= src.h as i32 {
                return;
            }
            let srow = &src.data[sy as usize * src_row..(sy as usize + 1) * src_row];
            for sx in 0..src.w as usize {
                let x = ox + sx as i32;
                if x < 0 || x >= ow {
                    continue;
                }
                let s = &srow[sx * 4..sx * 4 + 4];
                let mut c = if lut.identity { [s[0], s[1], s[2]] } else { lut.apply([s[0], s[1], s[2]]) };
                if vig > 0.0 {
                    c = vignette(c, sx as f32 / sw, sy as f32 / sh, vig);
                }
                let px = &mut row[x as usize * 4..x as usize * 4 + 4];
                let a = alpha * s[3] as f32 / 255.0;
                if a >= 0.999 {
                    px[0] = c[0];
                    px[1] = c[1];
                    px[2] = c[2];
                } else {
                    blend_px(px, c, a);
                }
            }
            return;
        }
        for x in 0..ow {
            // Inverse-map the output pixel into source space.
            let dx = (x as f32 + 0.5 - cx) / scale;
            let dy = (y as f32 + 0.5 - cy) / scale;
            let mut u = dx * cos - dy * sin + sw / 2.0;
            let v = dx * sin + dy * cos + sh / 2.0;
            if tf.flip_h {
                u = sw - u;
            }
            if u < cl || u >= cr || v < ct || v >= cb {
                continue;
            }
            let s = bilinear(src, u - 0.5, v - 0.5);
            let mut c = if lut.identity { [s[0], s[1], s[2]] } else { lut.apply([s[0], s[1], s[2]]) };
            if vig > 0.0 {
                c = vignette(c, u / sw, v / sh, vig);
            }
            let a = alpha * s[3] as f32 / 255.0;
            blend_px(&mut row[x as usize * 4..x as usize * 4 + 4], c, a);
        }
    });
}

#[inline]
fn vignette(c: [u8; 3], u: f32, v: f32, amount: f32) -> [u8; 3] {
    let dx = u - 0.5;
    let dy = v - 0.5;
    let d = (dx * dx + dy * dy) * 2.0; // 0 centre .. 1 corner
    let t = ((d - 0.15) / 0.85).clamp(0.0, 1.0);
    let f = 1.0 - amount * t * t * (3.0 - 2.0 * t);
    [(c[0] as f32 * f) as u8, (c[1] as f32 * f) as u8, (c[2] as f32 * f) as u8]
}

#[inline]
fn bilinear(f: &Frame, u: f32, v: f32) -> [u8; 4] {
    let x0 = u.floor().clamp(0.0, f.w as f32 - 1.0);
    let y0 = v.floor().clamp(0.0, f.h as f32 - 1.0);
    let x1 = (x0 + 1.0).min(f.w as f32 - 1.0);
    let y1 = (y0 + 1.0).min(f.h as f32 - 1.0);
    let fx = (u - x0).clamp(0.0, 1.0);
    let fy = (v - y0).clamp(0.0, 1.0);
    let at = |x: f32, y: f32| {
        let i = (y as usize * f.w as usize + x as usize) * 4;
        &f.data[i..i + 4]
    };
    let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
    let mut o = [0u8; 4];
    for k in 0..4 {
        let top = a[k] as f32 * (1.0 - fx) + b[k] as f32 * fx;
        let bot = c[k] as f32 * (1.0 - fx) + d[k] as f32 * fx;
        o[k] = (top * (1.0 - fy) + bot * fy) as u8;
    }
    o
}

fn draw_text(tr: &mut TextRenderer, out: &mut Frame, s: &TextStyle, alpha: f32, _sx: f32) {
    if s.text.trim().is_empty() {
        return;
    }
    let px = ((s.size * out.h as f32).round() as u32).clamp(4, 2000);
    let mask = tr.mask(&s.text, px, s.bold);
    let x0 = (s.x * out.w as f32 - mask.w as f32 / 2.0) as i32;
    let y0 = (s.y * out.h as f32 - mask.h as f32 / 2.0) as i32;
    let (ow, oh) = (out.w as i32, out.h as i32);
    let row_bytes = out.w as usize * 4;
    if let Some(bg) = s.background {
        let pad = (px as f32 * 0.25) as i32;
        let a = alpha * bg[3] as f32 / 255.0;
        let (bx0, by0) = (x0 - pad, y0 - pad / 2);
        let (bx1, by1) = (x0 + mask.w as i32 + pad, y0 + mask.h as i32 + pad / 2);
        out.data.par_chunks_mut(row_bytes).enumerate().for_each(|(y, row)| {
            let y = y as i32;
            if y < by0 || y >= by1 {
                return;
            }
            for x in bx0.max(0)..bx1.min(ow) {
                blend_px(&mut row[x as usize * 4..x as usize * 4 + 4], [bg[0], bg[1], bg[2]], a);
            }
        });
    }
    let shadow = if s.shadow { ((px as f32 * 0.05).round() as i32).max(1) } else { 0 };
    let col = s.color;
    let ca = alpha * col[3] as f32 / 255.0;
    out.data.par_chunks_mut(row_bytes).enumerate().for_each(|(y, row)| {
        let y = y as i32;
        for pass in 0..2 {
            let (off, color, strength) = if pass == 0 {
                if shadow == 0 {
                    continue;
                }
                (shadow, [0u8, 0, 0], ca * 0.55)
            } else {
                (0, [col[0], col[1], col[2]], ca)
            };
            let my = y - y0 - off;
            if my < 0 || my >= mask.h as i32 {
                continue;
            }
            let mrow = &mask.data[my as usize * mask.w as usize..(my as usize + 1) * mask.w as usize];
            for (mx, &m) in mrow.iter().enumerate() {
                if m == 0 {
                    continue;
                }
                let x = x0 + off + mx as i32;
                if x < 0 || x >= ow || y >= oh {
                    continue;
                }
                blend_px(&mut row[x as usize * 4..x as usize * 4 + 4], color, strength * m as f32 / 255.0);
            }
        }
    });
}
