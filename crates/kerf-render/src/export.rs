//! Rendering a project to a finished file.

use std::io::{Read, Write};
use std::process::Stdio;

use anyhow::{anyhow, Context, Result};
use kerf_core::{Project, Time, TimeRange};
use kerf_media::{ff, Frame, SAMPLE_RATE};
use serde::{Deserialize, Serialize};

use crate::{Compositor, Mixer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    /// libx264, plays everywhere.
    H264,
    /// libx265, smaller files.
    H265,
    /// Intel/AMD GPU H.264 (VA-API), fastest on a laptop iGPU.
    H264Vaapi,
    /// ProRes 422 for further editing.
    Prores,
}

impl Codec {
    pub fn parse(s: &str) -> Option<Codec> {
        Some(match s {
            "h264" | "x264" => Codec::H264,
            "h265" | "hevc" | "x265" => Codec::H265,
            "vaapi" | "h264_vaapi" | "gpu" => Codec::H264Vaapi,
            "prores" => Codec::Prores,
            _ => return None,
        })
    }

    pub const ALL: [Codec; 4] = [Codec::H264, Codec::H265, Codec::H264Vaapi, Codec::Prores];

    pub fn label(self) -> &'static str {
        match self {
            Codec::H264 => "H.264 (x264)",
            Codec::H265 => "H.265 (x265)",
            Codec::H264Vaapi => "H.264 GPU (VA-API)",
            Codec::Prores => "ProRes 422",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSettings {
    pub output: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub codec: Codec,
    /// Quality: CRF for x264/x265 (lower = better), QP for VA-API.
    pub quality: u32,
    /// x264/x265 preset.
    pub preset: String,
    pub audio_kbps: u32,
    /// Only this part of the timeline (default: all of it).
    pub range: Option<TimeRange>,
}

impl ExportSettings {
    pub fn for_project(p: &Project, output: impl Into<String>) -> Self {
        ExportSettings {
            output: output.into(),
            width: p.timeline.width,
            height: p.timeline.height,
            fps: p.timeline.fps,
            codec: Codec::H264,
            quality: 20,
            preset: "veryfast".into(),
            audio_kbps: 192,
            range: None,
        }
    }
}

/// Progress callback: `(fraction, stage)`; return `false` to cancel.
pub fn export(project: &Project, s: &ExportSettings, mut progress: impl FnMut(f32, &str) -> bool) -> Result<()> {
    let range = s
        .range
        .unwrap_or_else(|| TimeRange::new(Time::ZERO, project.timeline.duration()));
    if range.duration <= Time::ZERO {
        return Err(anyhow!("nothing to export: the timeline is empty"));
    }
    let (w, h) = (s.width & !1, s.height & !1);
    let fps = s.fps;
    let n_frames = (range.duration.as_secs_f64() * fps).round().max(1.0) as u64;

    // 1. Mix the audio into a temp file (fast: no video decode involved).
    let tmp_audio = kerf_media::cache_dir().join(format!("export-{}.f32", std::process::id()));
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp_audio)?);
        let mut mixer = Mixer::default();
        let total = (range.duration.as_secs_f64() * SAMPLE_RATE as f64).ceil() as i64;
        let start = (range.start.as_secs_f64() * SAMPLE_RATE as f64).round() as i64;
        let chunk = SAMPLE_RATE as i64 / 2;
        let mut buf = vec![0f32; chunk as usize * 2];
        let mut done = 0;
        while done < total {
            let n = chunk.min(total - done);
            let b = &mut buf[..n as usize * 2];
            mixer.mix(project, start + done, b);
            let bytes: Vec<u8> = b.iter().flat_map(|x| x.to_le_bytes()).collect();
            f.write_all(&bytes)?;
            done += n;
            if !progress(0.1 * done as f32 / total as f32, "mixing audio") {
                let _ = std::fs::remove_file(&tmp_audio);
                return Err(anyhow!("cancelled"));
            }
        }
    }

    // 2. Encoder process fed raw frames on stdin.
    let mut cmd = std::process::Command::new(ff::ffmpeg_bin());
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    if s.codec == Codec::H264Vaapi {
        cmd.args(["-vaapi_device", "/dev/dri/renderD128"]);
    }
    cmd.args(["-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-r", &format!("{fps}"), "-i", "pipe:0"]);
    cmd.args(["-f", "f32le", "-ar", &SAMPLE_RATE.to_string(), "-ac", "2"]).arg("-i").arg(&tmp_audio);
    cmd.args(["-map", "0:v", "-map", "1:a"]);
    let q = s.quality.to_string();
    match s.codec {
        Codec::H264 => {
            cmd.args(["-c:v", "libx264", "-preset", &s.preset, "-crf", &q, "-pix_fmt", "yuv420p"]);
        }
        Codec::H265 => {
            cmd.args(["-c:v", "libx265", "-preset", &s.preset, "-crf", &q, "-pix_fmt", "yuv420p", "-tag:v", "hvc1"]);
        }
        Codec::H264Vaapi => {
            cmd.args(["-vf", "format=nv12,hwupload", "-c:v", "h264_vaapi", "-qp", &q]);
        }
        Codec::Prores => {
            cmd.args(["-c:v", "prores_ks", "-profile:v", "2", "-pix_fmt", "yuv422p10le"]);
        }
    }
    if s.codec == Codec::Prores {
        cmd.args(["-c:a", "pcm_s16le"]);
    } else {
        cmd.args(["-c:a", "aac", "-b:a", &format!("{}k", s.audio_kbps), "-movflags", "+faststart"]);
    }
    cmd.args(["-shortest"]).arg(&s.output);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().context("spawning ffmpeg encoder")?;
    let mut stdin = child.stdin.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    // 3. Composite every frame and stream it out.
    let mut comp = Compositor::for_export();
    let mut frame = Frame::new(w, h);
    let mut result = Ok(());
    for i in 0..n_frames {
        let t = range.start + Time::from_secs_f64(i as f64 / fps);
        comp.render(project, t, &mut frame);
        if stdin.write_all(&frame.data).is_err() {
            result = Err(anyhow!("encoder exited early"));
            break;
        }
        if !progress(0.1 + 0.9 * (i + 1) as f32 / n_frames as f32, "rendering") {
            result = Err(anyhow!("cancelled"));
            break;
        }
    }
    drop(stdin);
    let status = child.wait()?;
    let err = err_thread.join().unwrap_or_default();
    let _ = std::fs::remove_file(&tmp_audio);
    result?;
    if !status.success() {
        let _ = std::fs::remove_file(&s.output);
        return Err(anyhow!("ffmpeg encoder failed: {}", err.trim()));
    }
    Ok(())
}
