//! Proxy media: a 540p, all-intra-ish H.264 copy that decodes and seeks
//! several times faster than 4K HEVC phone footage. Previews use it; exports
//! always go back to the original.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use anyhow::{anyhow, Result};

use crate::assets::{cache_dir, media_key};
use crate::ff::{ffmpeg, hwaccel};

pub fn proxy_path(path: &str) -> PathBuf {
    let d = cache_dir().join("proxies");
    let _ = std::fs::create_dir_all(&d);
    d.join(format!("{}.mp4", media_key(path)))
}

/// An existing proxy for `path`, if one was made before.
pub fn existing_proxy(path: &str) -> Option<String> {
    let p = proxy_path(path);
    p.exists().then(|| p.to_string_lossy().into_owned())
}

/// Make (or reuse) the proxy. `progress` gets 0..1.
pub fn make_proxy(path: &str, duration: f64, mut progress: impl FnMut(f32)) -> Result<String> {
    let out = proxy_path(path);
    if out.exists() {
        return Ok(out.to_string_lossy().into_owned());
    }
    let tmp = out.with_extension("part.mp4");
    let mut cmd = ffmpeg();
    if hwaccel() {
        cmd.args(["-hwaccel", "auto"]);
    }
    cmd.args(["-y", "-i", path, "-map", "0:v:0", "-an", "-sn"]);
    cmd.args([
        "-vf",
        "scale=-2:'min(540,ih)':flags=fast_bilinear,format=yuv420p",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-tune",
        "fastdecode",
        "-crf",
        "27",
        "-g",
        "8",
        "-bf",
        "0",
        "-progress",
        "pipe:1",
        "-nostats",
    ]);
    cmd.arg(&tmp);
    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().unwrap();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(us) = line.strip_prefix("out_time_us=") {
            if let Ok(us) = us.parse::<f64>() {
                progress(((us / 1e6) / duration.max(0.01)).clamp(0.0, 1.0) as f32);
            }
        }
    }
    let st = child.wait()?;
    if !st.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow!("proxy encode failed for {path}"));
    }
    std::fs::rename(&tmp, &out)?;
    progress(1.0);
    Ok(out.to_string_lossy().into_owned())
}
