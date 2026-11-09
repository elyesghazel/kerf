//! `kerf export --on <host>`: render on another machine over SSH.
//!
//! Media is rsynced into the remote's `~/.cache/kerf/remote-media/` (keyed by
//! the same media key the cache uses, so a second export re-sends nothing),
//! a path-rewritten copy of the project goes along, the remote `kerf export`
//! runs with its progress streamed back, and the result is pulled home.
//! Any SSH host works; Tailscale names (`pc`, `vps`, `laptop`) are just hosts.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;

use crate::api::{load, settings, ExportArgs, Out};

/// Only characters that need no quoting in bash or fish.
fn safe(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-/+=:,@".contains(c))
}

/// Run `cmd` on `host` under bash (whatever the login shell is), with the
/// usual user bin dirs on PATH. `cmd` must contain no single quotes.
fn remote(host: &str, cmd: &str) -> Command {
    let mut c = Command::new("ssh");
    c.args(["-o", "BatchMode=yes", host, "--"]);
    c.arg(format!(
        "bash -lc 'export PATH=$HOME/.cargo/bin:$HOME/.local/bin:$PATH; {cmd}'"
    ));
    c
}

fn run(mut c: Command, what: &str) -> Result<String> {
    let out = c.stdin(Stdio::null()).output().with_context(|| what.to_string())?;
    if !out.status.success() {
        bail!("{what} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn export_remote(path: &Path, host: &str, a: &ExportArgs) -> Result<Out> {
    if !safe(host) {
        bail!("odd host name {host:?}");
    }
    let mut p = load(path)?;
    let s = settings(&p, a)?;
    let log = |m: &str| {
        if !a.quiet {
            eprintln!("[{host}] {m}");
        }
    };

    // `KERF_REMOTE_BIN` points at a kerf that isn't on the remote PATH.
    let bin = std::env::var("KERF_REMOTE_BIN").unwrap_or_else(|_| "kerf".into());
    if !safe(&bin) {
        bail!("odd KERF_REMOTE_BIN");
    }
    log("checking kerf on the remote");
    let kerf = run(remote(host, &format!("command -v {bin} || true")), "ssh")?;
    if kerf.is_empty() {
        bail!(
            "`kerf` is not installed on {host}. Install it there from this repo with \
             `cargo install --path crates/kerf-cli` (needs ffmpeg on that machine too), then retry."
        );
    }
    let home = run(remote(host, "mkdir -p $HOME/.cache/kerf/remote-media $HOME/.cache/kerf/remote-out && echo $HOME"), "ssh mkdir")?;
    let home = home.lines().last().unwrap_or("").to_string();
    if !safe(&home) {
        bail!("unexpected remote home {home:?}");
    }

    // Ship media that the timeline actually uses.
    let used: std::collections::HashSet<_> = p.timeline.clips().filter_map(|(_, c)| c.media()).collect();
    for m in p.media.iter_mut() {
        let ext = Path::new(&m.path)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .filter(|e| safe(e))
            .unwrap_or_default();
        let remote_path = format!("{home}/.cache/kerf/remote-media/{}{ext}", kerf_media::media_key(&m.path));
        if used.contains(&m.id) {
            log(&format!("syncing {}", m.name));
            let st = Command::new("rsync")
                .args(["-a", "--partial", "--inplace"])
                .arg(&m.path)
                .arg(format!("{host}:{remote_path}"))
                .status()
                .context("running rsync")?;
            if !st.success() {
                bail!("rsync of {} to {host} failed", m.path);
            }
        }
        m.path = remote_path;
        m.proxy = None;
    }

    let job = format!("job-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs());
    let local_tmp = kerf_media::cache_dir().join(format!("{job}.kerf"));
    p.save(&local_tmp).map_err(|e| anyhow!(e))?;
    let rdir = format!("{home}/.cache/kerf/remote-out");
    let rproj = format!("{rdir}/{job}.kerf");
    let ext = Path::new(a.output).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
    if !safe(&ext) {
        bail!("odd output extension");
    }
    let rout = format!("{rdir}/{job}.{ext}");
    let st = Command::new("rsync").arg(&local_tmp).arg(format!("{host}:{rproj}")).status()?;
    let _ = std::fs::remove_file(&local_tmp);
    if !st.success() {
        bail!("copying the project to {host} failed");
    }

    // Build the remote export command from the resolved settings.
    let mut cmd = format!(
        "{bin} export -p {rproj} -o {rout} --codec {} --quality {} --preset {} --width {} --height {} --fps {}",
        match s.codec {
            kerf_render::Codec::H264 => "h264",
            kerf_render::Codec::H265 => "h265",
            kerf_render::Codec::H264Vaapi => "vaapi",
            kerf_render::Codec::Prores => "prores",
        },
        s.quality,
        s.preset,
        s.width,
        s.height,
        s.fps
    );
    if let Some(r) = s.range {
        cmd.push_str(&format!(" --from {} --to {}", r.start.as_secs_f64(), r.end().as_secs_f64()));
    }
    if !safe(&s.preset) {
        bail!("odd preset");
    }
    log("rendering");
    let started = std::time::Instant::now();
    let st = remote(host, &cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(if a.quiet { Stdio::null() } else { Stdio::inherit() })
        .status()?;
    if !st.success() {
        let _ = remote(host, &format!("rm -f {rproj}")).status();
        bail!("remote export on {host} failed");
    }
    log("fetching result");
    let st = Command::new("rsync").arg(format!("{host}:{rout}")).arg(a.output).status()?;
    let _ = remote(host, &format!("rm -f {rproj} {rout}")).stdout(Stdio::null()).status();
    if !st.success() {
        bail!("fetching {rout} from {host} failed");
    }
    let size = std::fs::metadata(a.output).map(|m| m.len()).unwrap_or(0);
    let took = started.elapsed().as_secs_f64();
    Ok(Out::new(
        format!("exported {} on {host} ({:.1} MB) in {took:.1}s", a.output, size as f64 / 1e6),
        json!({"ok": true, "output": a.output, "host": host, "bytes": size, "seconds": took}),
    ))
}
