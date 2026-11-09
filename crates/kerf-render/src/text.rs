//! Title rendering: text rasterised with ab_glyph into cached alpha masks.

use std::collections::HashMap;

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};

/// An 8-bit coverage mask.
pub struct Mask {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

pub struct TextRenderer {
    regular: FontArc,
    bold: FontArc,
    cache: HashMap<(String, u32, bool), std::sync::Arc<Mask>>,
}

fn load_system(candidates: &[&str]) -> Option<FontArc> {
    candidates
        .iter()
        .filter_map(|p| std::fs::read(p).ok())
        .find_map(|b| FontArc::try_from_vec(b).ok())
}

const BOLD: &[&str] = &[
    "/usr/share/fonts/inter/Inter-Bold.otf",
    "/usr/share/fonts/TTF/Inter-Bold.ttf",
    "/usr/share/fonts/noto/NotoSans-Bold.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
];
const REGULAR: &[&str] = &[
    "/usr/share/fonts/inter/Inter-Medium.otf",
    "/usr/share/fonts/TTF/Inter-Medium.ttf",
    "/usr/share/fonts/noto/NotoSans-Medium.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Medium.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
];

impl Default for TextRenderer {
    fn default() -> Self {
        let builtin = FontArc::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).expect("builtin font");
        let custom = std::env::var("KERF_FONT").ok().and_then(|p| load_system(&[p.as_str()]));
        let regular = custom.clone().or_else(|| load_system(REGULAR)).unwrap_or_else(|| builtin.clone());
        let bold = custom.or_else(|| load_system(BOLD)).unwrap_or_else(|| regular.clone());
        TextRenderer {
            regular,
            bold,
            cache: HashMap::new(),
        }
    }
}

impl TextRenderer {
    /// Rasterise `text` (multi-line, centred) at `px` pixels.
    pub fn mask(&mut self, text: &str, px: u32, bold: bool) -> std::sync::Arc<Mask> {
        let key = (text.to_string(), px, bold);
        if let Some(m) = self.cache.get(&key) {
            return m.clone();
        }
        if self.cache.len() > 256 {
            self.cache.clear();
        }
        let font = if bold { &self.bold } else { &self.regular };
        let scaled = font.as_scaled(PxScale::from(px as f32));
        let line_h = (scaled.ascent() - scaled.descent() + scaled.line_gap()).ceil();
        let lines: Vec<&str> = text.split('\n').collect();
        let widths: Vec<f32> = lines
            .iter()
            .map(|l| {
                let mut w = 0.0;
                let mut prev = None;
                for ch in l.chars() {
                    let id = scaled.glyph_id(ch);
                    if let Some(p) = prev {
                        w += scaled.kern(p, id);
                    }
                    w += scaled.h_advance(id);
                    prev = Some(id);
                }
                w
            })
            .collect();
        let pad = (px as f32 * 0.1).ceil();
        let w = (widths.iter().cloned().fold(0.0, f32::max) + pad * 2.0).ceil().max(1.0) as u32;
        let h = (line_h * lines.len() as f32 + pad * 2.0).ceil().max(1.0) as u32;
        let mut data = vec![0u8; (w * h) as usize];
        for (li, line) in lines.iter().enumerate() {
            let mut x = pad + (w as f32 - pad * 2.0 - widths[li]) / 2.0;
            let y = pad + li as f32 * line_h + scaled.ascent();
            let mut prev = None;
            for ch in line.chars() {
                let id = scaled.glyph_id(ch);
                if let Some(p) = prev {
                    x += scaled.kern(p, id);
                }
                let g = id.with_scale_and_position(scaled.scale(), ab_glyph::point(x, y));
                x += scaled.h_advance(id);
                prev = Some(id);
                if let Some(o) = font.outline_glyph(g) {
                    let b = o.px_bounds();
                    o.draw(|gx, gy, c| {
                        let px_ = gx as i32 + b.min.x as i32;
                        let py_ = gy as i32 + b.min.y as i32;
                        if px_ >= 0 && py_ >= 0 && (px_ as u32) < w && (py_ as u32) < h {
                            let i = (py_ as u32 * w + px_ as u32) as usize;
                            data[i] = data[i].max((c * 255.0) as u8);
                        }
                    });
                }
            }
        }
        let m = std::sync::Arc::new(Mask { w, h, data });
        self.cache.insert(key, m.clone());
        m
    }
}
