//! Background generation of thumbnails, waveforms and proxies, plus the
//! UI-side texture cache for them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crossbeam_channel::{unbounded, Sender};
use egui::{ColorImage, TextureHandle, TextureOptions};
use kerf_core::{MediaId, MediaItem};
use kerf_media::{Thumbs, Waveform};

enum Job {
    Thumbs(MediaItem),
    Wave(MediaItem),
    Proxy(MediaItem),
}

#[derive(Default)]
pub struct Shared {
    pub thumbs: HashMap<MediaId, Arc<Thumbs>>,
    pub waves: HashMap<MediaId, Arc<Waveform>>,
    /// Proxy progress 0..1 while generating.
    pub proxy_progress: HashMap<MediaId, f32>,
    /// Finished proxies waiting to be written into the project.
    pub proxy_done: Vec<(MediaId, String)>,
    pub errors: Vec<String>,
}

pub struct Assets {
    tx: Sender<Job>,
    proxy_tx: Sender<Job>,
    pub shared: Arc<Mutex<Shared>>,
    requested: std::collections::HashSet<(MediaId, u8)>,
    textures: HashMap<(MediaId, usize), TextureHandle>,
}

impl Assets {
    pub fn new(ctx: egui::Context) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (tx, rx) = unbounded::<Job>();
        // Two workers for quick assets, one for proxies (they are heavy).
        for i in 0..2 {
            let rx = rx.clone();
            let shared = shared.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("kerf-assets-{i}"))
                .spawn(move || {
                    for job in rx {
                        run(job, &shared);
                        ctx.request_repaint();
                    }
                })
                .unwrap();
        }
        let (proxy_tx, prx) = unbounded::<Job>();
        {
            let shared = shared.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name("kerf-proxy".into())
                .spawn(move || {
                    for job in prx {
                        if let Job::Proxy(m) = job {
                            let id = m.id;
                            let sh = shared.clone();
                            let c2 = ctx.clone();
                            let r = kerf_media::proxy::make_proxy(&m.path, m.duration.as_secs_f64(), |p| {
                                sh.lock().unwrap().proxy_progress.insert(id, p);
                                c2.request_repaint();
                            });
                            let mut s = shared.lock().unwrap();
                            s.proxy_progress.remove(&id);
                            match r {
                                Ok(path) => s.proxy_done.push((id, path)),
                                Err(e) => s.errors.push(format!("proxy for {}: {e}", m.name)),
                            }
                            ctx.request_repaint();
                        }
                    }
                })
                .unwrap();
        }
        Assets {
            tx,
            proxy_tx,
            shared,
            requested: Default::default(),
            textures: HashMap::new(),
        }
    }

    /// Make sure thumbnails/waveforms are (being) generated for every item.
    pub fn ensure(&mut self, media: &[MediaItem]) {
        for m in media {
            if m.has_video && self.requested.insert((m.id, 0)) {
                let _ = self.tx.send(Job::Thumbs(m.clone()));
            }
            if m.has_audio && self.requested.insert((m.id, 1)) {
                let _ = self.tx.send(Job::Wave(m.clone()));
            }
        }
    }

    pub fn make_proxy(&mut self, m: &MediaItem) {
        if self.requested.insert((m.id, 2)) {
            self.shared.lock().unwrap().proxy_progress.insert(m.id, 0.0);
            let _ = self.proxy_tx.send(Job::Proxy(m.clone()));
        }
    }

    pub fn thumbs(&self, id: MediaId) -> Option<Arc<Thumbs>> {
        self.shared.lock().unwrap().thumbs.get(&id).cloned()
    }

    pub fn wave(&self, id: MediaId) -> Option<Arc<Waveform>> {
        self.shared.lock().unwrap().waves.get(&id).cloned()
    }

    pub fn proxy_progress(&self, id: MediaId) -> Option<f32> {
        self.shared.lock().unwrap().proxy_progress.get(&id).copied()
    }

    /// Texture for thumbnail `idx` of `id`.
    pub fn thumb_texture(&mut self, ctx: &egui::Context, id: MediaId, idx: usize) -> Option<(TextureHandle, egui::Vec2)> {
        let th = self.thumbs(id)?;
        let idx = idx.min(th.frames.len().saturating_sub(1));
        let size = egui::vec2(th.w as f32, th.h as f32);
        if let Some(t) = self.textures.get(&(id, idx)) {
            return Some((t.clone(), size));
        }
        let img = ColorImage::from_rgba_unmultiplied([th.w as usize, th.h as usize], th.frames.get(idx)?);
        let tex = ctx.load_texture(format!("thumb-{}-{idx}", id.0), img, TextureOptions::LINEAR);
        self.textures.insert((id, idx), tex.clone());
        Some((tex, size))
    }
}

fn run(job: Job, shared: &Arc<Mutex<Shared>>) {
    match job {
        Job::Thumbs(m) => match Thumbs::load_or_make(&m.path, m.duration.as_secs_f64().min(36000.0), m.width, m.height) {
            Ok(t) => {
                shared.lock().unwrap().thumbs.insert(m.id, Arc::new(t));
            }
            Err(e) => shared.lock().unwrap().errors.push(format!("thumbnails for {}: {e}", m.name)),
        },
        Job::Wave(m) => match Waveform::load_or_make(&m.path) {
            Ok(w) => {
                shared.lock().unwrap().waves.insert(m.id, Arc::new(w));
            }
            Err(e) => shared.lock().unwrap().errors.push(format!("waveform for {}: {e}", m.name)),
        },
        Job::Proxy(_) => {}
    }
}
