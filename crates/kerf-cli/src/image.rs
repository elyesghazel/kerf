//! PNG encoding and contact sheets, so agents can *look* at an edit.

use anyhow::Result;
use kerf_core::{Project, Time};
use kerf_media::Frame;
use kerf_render::{text::TextRenderer, Compositor};

pub fn encode_png(f: &Frame) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, f.w, f.h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header()?;
        w.write_image_data(&f.data)?;
        w.finish()?;
    }
    Ok(buf)
}

/// A grid of `count` frames sampled evenly over the timeline, each labelled
/// with its time. Returns the sheet and the sample times.
pub fn contact_sheet(p: &Project, cols: u32, count: u32, width: u32) -> Result<(Frame, Vec<Time>)> {
    let dur = p.timeline.duration();
    if dur <= Time::ZERO {
        anyhow::bail!("the timeline is empty");
    }
    let gap = 4u32;
    let cols = cols.min(count);
    let rows = count.div_ceil(cols);
    let cw = ((width - gap * (cols + 1)) / cols) & !1;
    let ch = ((cw as f64 * p.timeline.height as f64 / p.timeline.width as f64).round() as u32).max(2) & !1;
    let sw = cols * cw + gap * (cols + 1);
    let sh = rows * ch + gap * (rows + 1);
    let mut sheet = Frame::new(sw, sh);
    sheet.fill([24, 26, 32]);
    let mut comp = Compositor::default();
    comp.use_proxies = true;
    let mut cell = Frame::new(cw, ch);
    let mut text = TextRenderer::default();
    let fps = p.timeline.fps;
    let mut times = vec![];
    for i in 0..count {
        let t = Time::from_secs_f64((i as f64 + 0.5) * dur.as_secs_f64() / count as f64).round_to_frame(fps);
        times.push(t);
        comp.render(p, t, &mut cell);
        let (cx, cy) = (gap + (i % cols) * (cw + gap), gap + (i / cols) * (ch + gap));
        for y in 0..ch {
            let src = &cell.data[(y * cw * 4) as usize..((y + 1) * cw * 4) as usize];
            let o = (((cy + y) * sw + cx) * 4) as usize;
            sheet.data[o..o + src.len()].copy_from_slice(src);
        }
        // Label: "#i  12.34s" in a dark box, bottom-left.
        let label = format!("#{}  {:.2}s", i + 1, t.as_secs_f64());
        let px = (ch / 9).clamp(10, 28);
        let m = text.mask(&label, px, true);
        let (lx, ly) = (cx + 2, cy + ch.saturating_sub(m.h + 2));
        for y in 0..m.h.min(ch) {
            for x in 0..m.w.min(cw.saturating_sub(4)) {
                let o = (((ly + y) * sw + lx + x) * 4) as usize;
                let a = m.data[(y * m.w + x) as usize] as f32 / 255.0;
                for k in 0..3 {
                    let bg = sheet.data[o + k] as f32 * 0.35;
                    sheet.data[o + k] = (bg * (1.0 - a) + 255.0 * a) as u8;
                }
            }
        }
    }
    Ok((sheet, times))
}
