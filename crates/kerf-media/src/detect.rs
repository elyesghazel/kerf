//! Content analysis agents use to find cut points: silence and scene changes.

use anyhow::{Context, Result};

use crate::ff::ffmpeg_bin;

fn run_stderr(args: &[&str]) -> Result<String> {
    let out = std::process::Command::new(ffmpeg_bin())
        .args(["-hide_banner", "-nostdin"])
        .args(args)
        .output()
        .context("running ffmpeg")?;
    Ok(String::from_utf8_lossy(&out.stderr).into_owned())
}

fn num_after(line: &str, key: &str) -> Option<f64> {
    let rest = &line[line.find(key)? + key.len()..];
    let tok: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == 'e')
        .collect();
    tok.parse().ok()
}

/// Silent stretches `(start, end)` in seconds: quieter than `noise_db`
/// (e.g. -35) for at least `min_secs`.
pub fn silences(path: &str, noise_db: f64, min_secs: f64, duration: f64) -> Result<Vec<(f64, f64)>> {
    let filter = format!("silencedetect=noise={noise_db}dB:d={min_secs}");
    let log = run_stderr(&["-i", path, "-vn", "-sn", "-af", &filter, "-f", "null", "-"])?;
    let mut out = Vec::new();
    let mut open: Option<f64> = None;
    for line in log.lines() {
        if let Some(s) = num_after(line, "silence_start:") {
            open = Some(s.max(0.0));
        } else if let Some(e) = num_after(line, "silence_end:") {
            out.push((open.take().unwrap_or(0.0), e));
        }
    }
    if let Some(s) = open {
        out.push((s, duration));
    }
    Ok(out)
}

/// Timestamps (seconds) where the picture changes sharply. `threshold` is
/// 0..1, ~0.3 is a good start.
pub fn scenes(path: &str, threshold: f64) -> Result<Vec<f64>> {
    let filter = format!("scale=160:-2,select='gt(scene\\,{threshold})',showinfo");
    let log = run_stderr(&["-i", path, "-an", "-sn", "-vf", &filter, "-f", "null", "-"])?;
    Ok(log
        .lines()
        .filter(|l| l.contains("Parsed_showinfo"))
        .filter_map(|l| num_after(l, "pts_time:"))
        .collect())
}

/// Integrated loudness (LUFS) and true peak (dBTP) of a file's audio.
pub fn loudness(path: &str) -> Result<(f64, f64)> {
    let log = run_stderr(&["-i", path, "-vn", "-sn", "-af", "ebur128=peak=true", "-f", "null", "-"])?;
    let mut lufs = f64::NAN;
    let mut peak = f64::NAN;
    // The summary block comes last; take the last values seen.
    for line in log.lines() {
        if let Some(v) = num_after(line, "I:") {
            lufs = v;
        }
        if let Some(v) = num_after(line, "Peak:") {
            peak = v;
        }
    }
    Ok((lufs, peak))
}
