//! `kerf-render`: compositor, audio mixer and exporter.
//!
//! Shared by the studio preview, the agent CLI and the exporter, so a frame
//! an agent inspects with `kerf frame` is pixel-for-pixel the exported one.

pub mod compositor;
pub mod export;
pub mod mixer;
pub mod text;

pub use compositor::{fit, Compositor};
pub use export::{export, Codec, ExportSettings};
pub use mixer::Mixer;

use kerf_core::{MediaId, MediaItem, Time};

/// Probe a file and turn it into a media item ready for `Document::add_media`.
/// Picks up an existing proxy from the cache.
pub fn import_item(path: &str) -> anyhow::Result<MediaItem> {
    let abs = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string());
    let p = kerf_media::probe(&abs)?;
    let name = std::path::Path::new(&abs)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(MediaItem {
        id: MediaId(0),
        proxy: kerf_media::proxy::existing_proxy(&abs),
        path: abs,
        name,
        duration: Time::from_secs_f64(p.duration),
        width: p.width.max(2),
        height: p.height.max(2),
        fps: p.fps,
        has_video: p.has_video,
        has_audio: p.has_audio,
        is_image: p.is_image,
    })
}

/// Render a single frame of the project at `t`, `w` pixels wide.
pub fn render_frame(project: &kerf_core::Project, t: Time, w: u32) -> kerf_media::Frame {
    let tl = &project.timeline;
    let h = ((w as f64 * tl.height as f64 / tl.width as f64).round() as u32).max(2) & !1;
    let mut comp = Compositor::default();
    comp.use_proxies = false;
    comp.hq = true;
    let mut f = kerf_media::Frame::new(w & !1, h);
    comp.render(project, t, &mut f);
    f
}
