//! The audio mixer: sums every audible clip into interleaved stereo f32.

use std::collections::HashMap;

use kerf_core::{ClipId, Project, Time, TrackKind};
use kerf_media::{AudioStream, SAMPLE_RATE};

struct Reader {
    stream: AudioStream,
    path: String,
    used: u64,
}

#[derive(Default)]
pub struct Mixer {
    readers: HashMap<ClipId, Reader>,
    scratch: Vec<f32>,
    tick: u64,
}

const SR: i64 = SAMPLE_RATE as i64;

fn to_samples(t: Time) -> i64 {
    (t.micros() as i128 * SR as i128 / 1_000_000) as i64
}

impl Mixer {
    pub fn reset(&mut self) {
        self.readers.clear();
    }

    /// Mix `out.len() / 2` sample frames starting at timeline sample `start`.
    pub fn mix(&mut self, project: &Project, start: i64, out: &mut [f32]) {
        self.tick += 1;
        out.fill(0.0);
        let frames = (out.len() / 2) as i64;
        let end = start + frames;
        let any_solo = false;
        let _ = any_solo;
        for tr in project.timeline.tracks.iter().filter(|t| t.kind == TrackKind::Audio && !t.muted) {
            for clip in &tr.clips {
                let Some(mid) = clip.media() else { continue };
                let Some(item) = project.media_item(mid) else { continue };
                if !item.has_audio {
                    continue;
                }
                let cs = to_samples(clip.start);
                let ce = to_samples(clip.end());
                let a = cs.max(start);
                let b = ce.min(end);
                if a >= b {
                    continue;
                }
                let src_pos = to_samples(clip.source_in) + (a - cs);
                let r = self.readers.entry(clip.id);
                let r = match r {
                    std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
                    std::collections::hash_map::Entry::Vacant(v) => {
                        let Ok(s) = AudioStream::open(&item.path, src_pos as f64 / SR as f64) else { continue };
                        v.insert(Reader { stream: s, path: item.path.clone(), used: 0 })
                    }
                };
                let expected = (r.stream.start * SR as f64).round() as i64 + r.stream.pos as i64;
                if (expected - src_pos).abs() > 2 || r.path != item.path {
                    match AudioStream::open(&item.path, src_pos as f64 / SR as f64) {
                        Ok(s) => {
                            r.stream = s;
                            r.path = item.path.clone();
                        }
                        Err(_) => continue,
                    }
                }
                r.used = self.tick;
                let n = (b - a) as usize;
                self.scratch.resize(n * 2, 0.0);
                r.stream.read(&mut self.scratch);
                let base_gain = clip.volume * tr.volume;
                let off = (a - start) as usize;
                // Fades are evaluated every 64 samples: smooth and cheap.
                for chunk in 0..n.div_ceil(64) {
                    let i0 = chunk * 64;
                    let i1 = (i0 + 64).min(n);
                    let t = Time((a + i0 as i64) * 1_000_000 / SR);
                    let g = base_gain * clip.fade_gain(t);
                    for i in i0..i1 {
                        out[(off + i) * 2] += self.scratch[i * 2] * g;
                        out[(off + i) * 2 + 1] += self.scratch[i * 2 + 1] * g;
                    }
                }
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
        let tick = self.tick;
        self.readers.retain(|_, r| tick - r.used < 50);
    }
}
