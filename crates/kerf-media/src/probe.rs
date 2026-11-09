//! `ffprobe` wrapper.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::ff::ffprobe_bin;

#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub duration: f64,
    /// Display size, after rotation metadata is applied.
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_video: bool,
    pub has_audio: bool,
    pub is_image: bool,
    pub video_codec: String,
    pub audio_codec: String,
}

fn ratio(s: &str) -> Option<f64> {
    let (a, b) = s.split_once('/')?;
    let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
    (b != 0.0 && a > 0.0).then(|| a / b)
}

pub fn probe(path: &str) -> Result<Probe> {
    let out = std::process::Command::new(ffprobe_bin())
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams", path])
        .output()
        .context("running ffprobe (is ffmpeg installed?)")?;
    if !out.status.success() {
        return Err(anyhow!("ffprobe failed on {path}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let v: Value = serde_json::from_slice(&out.stdout)?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let fmt_name = v["format"]["format_name"].as_str().unwrap_or("");
    let mut p = Probe {
        duration: v["format"]["duration"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0),
        width: 0,
        height: 0,
        fps: 30.0,
        has_video: false,
        has_audio: false,
        is_image: false,
        video_codec: String::new(),
        audio_codec: String::new(),
    };
    for s in &streams {
        match s["codec_type"].as_str() {
            Some("video") if !p.has_video => {
                // Cover art inside audio files is not a video.
                if s["disposition"]["attached_pic"].as_i64() == Some(1) {
                    continue;
                }
                p.has_video = true;
                p.video_codec = s["codec_name"].as_str().unwrap_or("").into();
                p.width = s["width"].as_u64().unwrap_or(0) as u32;
                p.height = s["height"].as_u64().unwrap_or(0) as u32;
                p.fps = s["avg_frame_rate"]
                    .as_str()
                    .and_then(ratio)
                    .or_else(|| s["r_frame_rate"].as_str().and_then(ratio))
                    .filter(|f| *f < 400.0)
                    .unwrap_or(30.0);
                // Phones store portrait video as landscape + a rotation flag;
                // ffmpeg rotates on decode, so report the displayed size.
                let rot = s["side_data_list"]
                    .as_array()
                    .and_then(|l| l.iter().find_map(|d| d["rotation"].as_f64()))
                    .or_else(|| s["tags"]["rotate"].as_str().and_then(|r| r.parse().ok()))
                    .unwrap_or(0.0);
                if (rot.abs() as i64 / 90) % 2 == 1 {
                    std::mem::swap(&mut p.width, &mut p.height);
                }
                if p.duration <= 0.0 {
                    p.duration = s["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
                }
                let still = matches!(p.video_codec.as_str(), "png" | "mjpeg" | "webp" | "bmp" | "tiff" | "gif")
                    && (fmt_name.contains("image2") || fmt_name.ends_with("_pipe"));
                let frames = s["nb_frames"].as_str().and_then(|n| n.parse::<u64>().ok());
                p.is_image = still || (fmt_name.ends_with("_pipe") && frames.unwrap_or(1) <= 1);
            }
            Some("audio") if !p.has_audio => {
                p.has_audio = true;
                p.audio_codec = s["codec_name"].as_str().unwrap_or("").into();
            }
            _ => {}
        }
    }
    if p.is_image {
        p.duration = 3600.0;
        p.has_audio = false;
    }
    if !p.has_video && !p.has_audio {
        return Err(anyhow!("{path}: no audio or video streams"));
    }
    Ok(p)
}
