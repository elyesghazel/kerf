//! Streaming decode through ffmpeg pipes.
//!
//! A [`VideoStream`] is one long-lived ffmpeg process emitting raw RGBA frames
//! at a fixed rate, already scaled to the size the compositor wants. Reading
//! sequentially is as fast as ffmpeg can decode; seeking means restarting the
//! process at the new position (fast with short-GOP proxies).

use std::io::Read;
use std::process::{Child, ChildStdout};

use anyhow::{anyhow, Context, Result};

use crate::ff::{ffmpeg, fmt_secs, hwaccel};

pub const SAMPLE_RATE: u32 = 48_000;

/// An RGBA8 image.
#[derive(Clone)]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl Frame {
    pub fn new(w: u32, h: u32) -> Self {
        Frame {
            w,
            h,
            data: vec![0; (w * h * 4) as usize],
        }
    }

    pub fn fill(&mut self, rgb: [u8; 3]) {
        for px in self.data.chunks_exact_mut(4) {
            px.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Frame({}x{})", self.w, self.h)
    }
}

struct Proc {
    child: Child,
    out: ChildStdout,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(mut cmd: std::process::Command) -> Result<Proc> {
    let mut child = cmd.spawn().context("spawning ffmpeg (is it installed?)")?;
    // stderr is drained in the background so a chatty ffmpeg never blocks.
    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            let s = s.trim();
            if !s.is_empty() {
                log::debug!("ffmpeg: {}", &s[..s.len().min(400)]);
            }
        });
    }
    let out = child.stdout.take().ok_or_else(|| anyhow!("no ffmpeg stdout"))?;
    Ok(Proc { child, out })
}

/// Sequential video frames from `start` onwards.
pub struct VideoStream {
    proc: Proc,
    pub w: u32,
    pub h: u32,
    pub fps: f64,
    pub start: f64,
    /// Frames read so far.
    pub index: u64,
    pub eof: bool,
}

impl VideoStream {
    /// Open `path` at source time `start` (seconds), decoding at `fps` and
    /// scaling to exactly `w`x`h`. `hq` picks a sharper (slower) scaler.
    pub fn open(path: &str, start: f64, w: u32, h: u32, fps: f64, hq: bool, accel: bool) -> Result<Self> {
        let mut cmd = ffmpeg();
        if accel && hwaccel() {
            cmd.args(["-hwaccel", "auto"]);
        }
        cmd.args(["-ss", &fmt_secs(start), "-i", path, "-an", "-sn", "-dn"]);
        let flags = if hq { "bicubic" } else { "fast_bilinear" };
        cmd.args([
            "-vf",
            &format!("fps={fps},scale={w}:{h}:flags={flags},format=rgba"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ]);
        Ok(VideoStream {
            proc: spawn(cmd)?,
            w,
            h,
            fps,
            start,
            index: 0,
            eof: false,
        })
    }

    /// Source time of the next frame [`read`](Self::read) will return.
    pub fn next_time(&self) -> f64 {
        self.start + self.index as f64 / self.fps
    }

    /// Read the next frame into `frame` (which must be `w`x`h`).
    /// Returns `false` at end of stream.
    pub fn read(&mut self, frame: &mut Frame) -> bool {
        if self.eof {
            return false;
        }
        match self.proc.out.read_exact(&mut frame.data) {
            Ok(()) => {
                self.index += 1;
                true
            }
            Err(_) => {
                self.eof = true;
                false
            }
        }
    }

    /// Skip frames until `t` is the next one (forward only).
    pub fn skip_to(&mut self, t: f64, scratch: &mut Frame) {
        while !self.eof && self.next_time() + 0.5 / self.fps < t {
            self.read(scratch);
        }
    }
}

/// Decode a single frame at `t` (accurate seek).
pub fn grab_frame(path: &str, t: f64, w: u32, h: u32, hq: bool) -> Result<Frame> {
    let mut cmd = ffmpeg();
    cmd.args(["-ss", &fmt_secs(t), "-i", path, "-an", "-sn", "-frames:v", "1"]);
    let flags = if hq { "bicubic" } else { "fast_bilinear" };
    cmd.args(["-vf", &format!("scale={w}:{h}:flags={flags},format=rgba"), "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"]);
    let mut p = spawn(cmd)?;
    let mut f = Frame::new(w, h);
    p.out
        .read_exact(&mut f.data)
        .map_err(|_| anyhow!("no frame at {t:.3}s in {path}"))?;
    Ok(f)
}

/// Sequential interleaved stereo f32 audio at [`SAMPLE_RATE`] from `start`.
pub struct AudioStream {
    proc: Proc,
    pub start: f64,
    /// Sample frames (L+R pairs) read so far.
    pub pos: u64,
    pub eof: bool,
    buf: Vec<u8>,
}

impl AudioStream {
    pub fn open(path: &str, start: f64) -> Result<Self> {
        let mut cmd = ffmpeg();
        cmd.args(["-ss", &fmt_secs(start), "-i", path, "-vn", "-sn", "-dn"]);
        cmd.args(["-f", "f32le", "-ac", "2", "-ar", &SAMPLE_RATE.to_string(), "pipe:1"]);
        Ok(AudioStream {
            proc: spawn(cmd)?,
            start,
            pos: 0,
            eof: false,
            buf: Vec::new(),
        })
    }

    /// Fill `out` (interleaved stereo) with the next samples; zero-pads at EOF.
    pub fn read(&mut self, out: &mut [f32]) {
        let n = out.len() * 4;
        self.buf.resize(n, 0);
        let mut got = 0;
        while got < n && !self.eof {
            match self.proc.out.read(&mut self.buf[got..]) {
                Ok(0) | Err(_) => self.eof = true,
                Ok(k) => got += k,
            }
        }
        let whole = got / 4;
        for (i, s) in out.iter_mut().enumerate() {
            *s = if i < whole {
                f32::from_le_bytes(self.buf[i * 4..i * 4 + 4].try_into().unwrap())
            } else {
                0.0
            };
        }
        self.pos += (out.len() / 2) as u64;
    }
}
