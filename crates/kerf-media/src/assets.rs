//! Cached per-file assets: timeline thumbnails and audio waveforms.
//!
//! Assets live in `~/.cache/kerf/<key>.*`, keyed by path + size + mtime, so a
//! project opens instantly the second time: nothing is regenerated, and the
//! project file itself stays a tiny JSON edit list.

use std::io::Read;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};

use crate::ff::ffmpeg;

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let d = base.join("kerf");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Stable identity of a media file: FNV-1a over path, size and mtime.
pub fn media_key(path: &str) -> String {
    let meta = std::fs::metadata(path).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in path.bytes().chain(size.to_le_bytes()).chain(mtime.to_le_bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// A strip of small frames sampled every `interval` seconds.
#[derive(Clone)]
pub struct Thumbs {
    pub w: u32,
    pub h: u32,
    pub interval: f32,
    /// RGBA, `w*h*4` bytes per thumbnail.
    pub frames: Vec<Vec<u8>>,
}

impl Thumbs {
    pub const HEIGHT: u32 = 54;

    /// Load from cache or generate. Uses keyframes only, so it is fast even on
    /// long 4K files.
    pub fn load_or_make(path: &str, duration: f64, src_w: u32, src_h: u32) -> Result<Thumbs> {
        let file = cache_dir().join(format!("{}.thumbs", media_key(path)));
        if let Ok(t) = Self::read(&file) {
            return Ok(t);
        }
        let h = Self::HEIGHT;
        let w = (((src_w.max(1) as f64 / src_h.max(1) as f64) * h as f64).round() as u32).clamp(16, 200) & !1;
        let interval = (duration / 120.0).max(1.0) as f32;
        let mut cmd = ffmpeg();
        cmd.args(["-skip_frame", "nokey", "-i", path, "-an", "-sn", "-vsync", "vfr"]);
        cmd.args([
            "-vf",
            &format!("fps=1/{interval},scale={w}:{h}:flags=fast_bilinear,format=rgba"),
            "-f",
            "rawvideo",
            "pipe:1",
        ]);
        let out = cmd.output().context("ffmpeg thumbnails")?;
        let sz = (w * h * 4) as usize;
        let mut frames: Vec<Vec<u8>> = out.stdout.chunks_exact(sz).map(|c| c.to_vec()).collect();
        if frames.is_empty() {
            // Keyframe-only decode found nothing (e.g. an image); grab one frame.
            frames.push(crate::grab_frame(path, 0.0, w, h, false)?.data);
        }
        let t = Thumbs { w, h, interval, frames };
        let _ = t.write(&file);
        Ok(t)
    }

    /// The thumbnail closest to source time `t`.
    pub fn at(&self, t: f64) -> Option<&[u8]> {
        if self.frames.is_empty() {
            return None;
        }
        let i = ((t / self.interval as f64).floor().max(0.0) as usize).min(self.frames.len() - 1);
        Some(&self.frames[i])
    }

    fn write(&self, file: &PathBuf) -> std::io::Result<()> {
        let mut b = Vec::new();
        b.extend_from_slice(b"KTH1");
        b.extend_from_slice(&self.w.to_le_bytes());
        b.extend_from_slice(&self.h.to_le_bytes());
        b.extend_from_slice(&self.interval.to_le_bytes());
        b.extend_from_slice(&(self.frames.len() as u32).to_le_bytes());
        for f in &self.frames {
            b.extend_from_slice(f);
        }
        std::fs::write(file, b)
    }

    fn read(file: &PathBuf) -> Result<Thumbs> {
        let b = std::fs::read(file)?;
        if b.len() < 20 || &b[..4] != b"KTH1" {
            return Err(anyhow!("bad thumbs cache"));
        }
        let u = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let (w, h, n) = (u(4), u(8), u(16) as usize);
        let interval = f32::from_le_bytes(b[12..16].try_into().unwrap());
        let sz = (w * h * 4) as usize;
        if b.len() != 20 + n * sz {
            return Err(anyhow!("truncated thumbs cache"));
        }
        let frames = b[20..].chunks_exact(sz).map(|c| c.to_vec()).collect();
        Ok(Thumbs { w, h, interval, frames })
    }
}

/// Peak amplitude envelope, [`Waveform::RATE`] values per second.
#[derive(Clone)]
pub struct Waveform {
    pub peaks: Vec<u8>,
}

impl Waveform {
    pub const RATE: f64 = 100.0;

    pub fn load_or_make(path: &str) -> Result<Waveform> {
        let file = cache_dir().join(format!("{}.peaks", media_key(path)));
        if let Ok(b) = std::fs::read(&file) {
            if b.len() > 4 && &b[..4] == b"KWF1" {
                return Ok(Waveform { peaks: b[4..].to_vec() });
            }
        }
        let mut cmd = ffmpeg();
        cmd.args(["-i", path, "-vn", "-sn", "-ac", "1", "-ar", "8000", "-f", "s16le", "pipe:1"]);
        let mut child = cmd.stderr(std::process::Stdio::null()).spawn()?;
        let mut out = child.stdout.take().unwrap();
        let per = (8000.0 / Self::RATE) as usize * 2;
        let mut buf = vec![0u8; per];
        let mut peaks = Vec::new();
        loop {
            let mut got = 0;
            while got < per {
                match out.read(&mut buf[got..]) {
                    Ok(0) | Err(_) => break,
                    Ok(k) => got += k,
                }
            }
            if got < 2 {
                break;
            }
            let m = buf[..got & !1]
                .chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs())
                .max()
                .unwrap_or(0);
            // sqrt curve so quiet speech is still visible.
            peaks.push(((m as f32 / 32768.0).sqrt() * 255.0) as u8);
            if got < per {
                break;
            }
        }
        let _ = child.wait();
        let mut b = b"KWF1".to_vec();
        b.extend_from_slice(&peaks);
        let _ = std::fs::write(&file, b);
        Ok(Waveform { peaks })
    }

    /// Max peak (0..1) over source seconds `[a, b)`.
    pub fn peak(&self, a: f64, b: f64) -> f32 {
        let i = (a * Self::RATE).max(0.0) as usize;
        let j = ((b * Self::RATE).ceil() as usize).max(i + 1).min(self.peaks.len());
        if i >= j {
            return 0.0;
        }
        self.peaks[i..j].iter().copied().max().unwrap_or(0) as f32 / 255.0
    }
}
