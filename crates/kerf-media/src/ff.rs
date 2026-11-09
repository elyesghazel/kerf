//! Locating and spawning the ffmpeg binaries.

use std::process::{Command, Stdio};

/// `ffmpeg`, or `$KERF_FFMPEG`.
pub fn ffmpeg_bin() -> String {
    std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}

/// `ffprobe`, or `$KERF_FFPROBE`.
pub fn ffprobe_bin() -> String {
    std::env::var("KERF_FFPROBE").unwrap_or_else(|_| "ffprobe".into())
}

/// A quiet ffmpeg command with piped stdout and no stdin.
pub fn ffmpeg() -> Command {
    let mut c = Command::new(ffmpeg_bin());
    c.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    c
}

/// Whether to ask ffmpeg for GPU decoding (`-hwaccel auto`). On by default;
/// `KERF_HWACCEL=none` turns it off.
pub fn hwaccel() -> bool {
    !matches!(std::env::var("KERF_HWACCEL").as_deref(), Ok("none") | Ok("0") | Ok("off"))
}

pub fn fmt_secs(s: f64) -> String {
    format!("{:.6}", s.max(0.0))
}
