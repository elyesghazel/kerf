//! The operation layer shared by the CLI and the MCP server.
//!
//! Every function takes plain arguments (times as strings so both "12.5",
//! "1:02.5" and "00:01:02:15" work), does its work on the project file and
//! returns an [`Out`]: human text, a JSON value, and optionally a PNG.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use kerf_core::{
    ClipContent, ClipId, Command, Document, Edge, MarkerId, MediaId, Project, TextStyle, Time, TimeRange, TrackId,
    TrackKind,
};
use serde_json::{json, Value};

use crate::history;
use crate::image;

pub struct Out {
    pub text: String,
    pub json: Value,
    pub png: Option<Vec<u8>>,
}

impl Out {
    pub fn new(text: impl Into<String>, json: Value) -> Self {
        Out {
            text: text.into(),
            json,
            png: None,
        }
    }
}

// ---------------------------------------------------------------- helpers

pub fn load(path: &Path) -> Result<Project> {
    Project::load(path).map_err(|e| anyhow!(e))
}

/// Load, edit, and (if anything changed) snapshot + save atomically.
fn mutate(path: &Path, f: impl FnOnce(&mut Document) -> Result<Out>) -> Result<Out> {
    let before = load(path)?;
    let mut doc = Document::new(before.clone());
    let mut out = f(&mut doc)?;
    if doc.revision() > 0 {
        history::push(path, &before)?;
        doc.project().save(path).map_err(|e| anyhow!(e))?;
        let d = doc.project().timeline.duration();
        out.text.push_str(&format!("\n(timeline now {})", fmt_t(d, doc.project().timeline.fps)));
    }
    Ok(out)
}

pub fn parse_t(s: &str, fps: f64) -> Result<Time> {
    Time::parse(s, fps).ok_or_else(|| anyhow!("can't parse time {s:?} (use 12.5, 1:02.5 or HH:MM:SS:FF)"))
}

fn fmt_t(t: Time, fps: f64) -> String {
    format!("{:.3}s [{}]", t.as_secs_f64(), t.timecode(fps))
}

fn secs(t: Time) -> f64 {
    (t.as_secs_f64() * 1000.0).round() / 1000.0
}

fn ids_json(ids: &[ClipId]) -> Value {
    json!(ids.iter().map(|c| c.0).collect::<Vec<_>>())
}

fn fmt_ids(ids: &[ClipId]) -> String {
    ids.iter().map(|c| c.0.to_string()).collect::<Vec<_>>().join(", ")
}

fn e(err: kerf_core::EditError) -> anyhow::Error {
    anyhow!("{err}")
}

pub fn parse_color(s: &str) -> Result<[u8; 4]> {
    let h = s.trim().trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map_err(|_| anyhow!("bad colour {s:?}"));
    match h.len() {
        6 => Ok([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Ok([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => match h {
            "white" => Ok([255; 4]),
            "black" => Ok([0, 0, 0, 255]),
            _ => bail!("bad colour {s:?} (use #RRGGBB or #RRGGBBAA)"),
        },
    }
}

fn color_hex(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
    }
}

fn parse_bool(v: &str) -> Result<bool> {
    match v {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("expected true/false, got {v:?}"),
    }
}

fn parse_f(v: &str) -> Result<f32> {
    v.parse::<f32>().map_err(|_| anyhow!("expected a number, got {v:?}"))
}

pub fn preset(name: &str) -> Result<(u32, u32)> {
    Ok(match name {
        "1080p" | "hd" => (1920, 1080),
        "720p" => (1280, 720),
        "4k" | "uhd" | "2160p" => (3840, 2160),
        "vertical" | "reel" | "9:16" | "portrait" => (1080, 1920),
        "square" | "1:1" => (1080, 1080),
        "4:5" => (1080, 1350),
        _ => bail!("unknown preset {name:?} (1080p, 720p, 4k, vertical, square, 4:5)"),
    })
}

// ---------------------------------------------------------------- project

pub fn new_project(
    path: &Path,
    name: Option<String>,
    preset_name: Option<&str>,
    w: Option<u32>,
    h: Option<u32>,
    fps: Option<f64>,
    force: bool,
) -> Result<Out> {
    if path.exists() && !force {
        bail!("{} exists (use --force to overwrite)", path.display());
    }
    let mut p = Project::default();
    p.name = name.unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    });
    if let Some(pr) = preset_name {
        let (pw, ph) = preset(pr)?;
        p.timeline.width = pw;
        p.timeline.height = ph;
    }
    if let Some(w) = w {
        p.timeline.width = w & !1;
    }
    if let Some(h) = h {
        p.timeline.height = h & !1;
    }
    if let Some(f) = fps {
        p.timeline.fps = f;
    }
    p.save(path).map_err(|e| anyhow!(e))?;
    history::clear(path);
    let tl = &p.timeline;
    Ok(Out::new(
        format!("created {} ({}x{} @ {}fps)", path.display(), tl.width, tl.height, tl.fps),
        json!({"ok": true, "path": path, "width": tl.width, "height": tl.height, "fps": tl.fps}),
    ))
}

/// A compact, LLM-friendly view of the whole timeline.
pub fn info(path: &Path) -> Result<Out> {
    let p = load(path)?;
    Ok(Out::new(describe(&p), serde_json::to_value(&p)?))
}

pub fn describe(p: &Project) -> String {
    let tl = &p.timeline;
    let fps = tl.fps;
    let mut s = String::new();
    s.push_str(&format!(
        "project {:?}  {}x{} @ {}fps  duration {}\n",
        p.name,
        tl.width,
        tl.height,
        fps,
        fmt_t(tl.duration(), fps)
    ));
    s.push_str("tracks (video tracks listed top layer first, then audio):\n");
    let mut order: Vec<&kerf_core::Track> = tl.video_tracks().collect();
    order.reverse();
    order.extend(tl.audio_tracks());
    for tr in order {
        let mut flags = vec![];
        if tr.muted {
            flags.push("muted".to_string());
        }
        if tr.hidden {
            flags.push("hidden".to_string());
        }
        if tr.locked {
            flags.push("locked".to_string());
        }
        if (tr.volume - 1.0).abs() > 1e-3 {
            flags.push(format!("volume={}", tr.volume));
        }
        let kind = if tr.kind == TrackKind::Video { "video" } else { "audio" };
        s.push_str(&format!(
            "  {} (track {}, {kind}{}){}\n",
            tr.name,
            tr.id.0,
            if flags.is_empty() { String::new() } else { format!(", {}", flags.join(" ")) },
            if tr.clips.is_empty() { " — empty" } else { "" }
        ));
        for c in &tr.clips {
            s.push_str(&format!(
                "    clip {:<4} {:>9.3} → {:>9.3}  ({:.3}s)  ",
                c.id.0,
                c.start.as_secs_f64(),
                c.end().as_secs_f64(),
                c.duration.as_secs_f64()
            ));
            match &c.content {
                ClipContent::Media { media } => {
                    let name = p.media_item(*media).map(|m| m.name.as_str()).unwrap_or("?");
                    s.push_str(&format!("media {} {:?} src_in {:.3}", media.0, name, c.source_in.as_secs_f64()));
                }
                ClipContent::Text(t) => s.push_str(&format!("TEXT {:?}", t.text)),
                ClipContent::Solid { color } => s.push_str(&format!("SOLID {}", color_hex(*color))),
            }
            if let Some(l) = c.link {
                s.push_str(&format!("  link {l}"));
            }
            let props = clip_props(c);
            if !props.is_empty() {
                s.push_str(&format!("  [{}]", props.join(" ")));
            }
            s.push('\n');
        }
    }
    if !tl.markers.is_empty() {
        s.push_str("markers:\n");
        for m in &tl.markers {
            s.push_str(&format!("  marker {} at {} {:?}\n", m.id.0, fmt_t(m.time, fps), m.label));
        }
    }
    s.push_str("media:\n");
    if p.media.is_empty() {
        s.push_str("  (none — `kerf import <files>`)\n");
    }
    for m in &p.media {
        s.push_str(&format!("  {}\n", media_line(m)));
    }
    s
}

fn media_line(m: &kerf_core::MediaItem) -> String {
    let kind = match (m.is_image, m.has_video, m.has_audio) {
        (true, _, _) => "image",
        (_, true, true) => "video+audio",
        (_, true, false) => "video",
        _ => "audio",
    };
    format!(
        "media {:<3} {:?} {} {:.3}s {}x{} {:.3}fps{}  {}",
        m.id.0,
        m.name,
        kind,
        m.duration.as_secs_f64(),
        m.width,
        m.height,
        m.fps,
        if m.proxy.is_some() { " proxy" } else { "" },
        m.path
    )
}

fn clip_props(c: &kerf_core::Clip) -> Vec<String> {
    let mut v = vec![];
    if !c.name.is_empty() {
        if let ClipContent::Media { .. } = c.content {
        } else {
            v.push(format!("name={:?}", c.name));
        }
    }
    if (c.volume - 1.0).abs() > 1e-3 {
        v.push(format!("volume={}", c.volume));
    }
    if (c.opacity - 1.0).abs() > 1e-3 {
        v.push(format!("opacity={}", c.opacity));
    }
    if c.fade_in.0 > 0 {
        v.push(format!("fade_in={}", secs(c.fade_in)));
    }
    if c.fade_out.0 > 0 {
        v.push(format!("fade_out={}", secs(c.fade_out)));
    }
    let g = c.grade;
    let dg = kerf_core::ColorGrade::default();
    for (k, a, b) in [
        ("exposure", g.exposure, dg.exposure),
        ("brightness", g.brightness, dg.brightness),
        ("contrast", g.contrast, dg.contrast),
        ("saturation", g.saturation, dg.saturation),
        ("gamma", g.gamma, dg.gamma),
        ("temperature", g.temperature, dg.temperature),
        ("tint", g.tint, dg.tint),
        ("vignette", g.vignette, dg.vignette),
    ] {
        if (a - b).abs() > 1e-4 {
            v.push(format!("grade.{k}={a}"));
        }
    }
    let t = c.transform;
    let dt = kerf_core::Transform::default();
    for (k, a, b) in [
        ("scale", t.scale, dt.scale),
        ("x", t.x, dt.x),
        ("y", t.y, dt.y),
        ("rotation", t.rotation, dt.rotation),
        ("crop_left", t.crop_left, 0.0),
        ("crop_right", t.crop_right, 0.0),
        ("crop_top", t.crop_top, 0.0),
        ("crop_bottom", t.crop_bottom, 0.0),
    ] {
        if (a - b).abs() > 1e-4 {
            v.push(format!("transform.{k}={a}"));
        }
    }
    if t.flip_h {
        v.push("transform.flip_h=true".into());
    }
    if let ClipContent::Text(s) = &c.content {
        v.push(format!(
            "size={} x={} y={} color={}{}{}",
            s.size,
            s.x,
            s.y,
            color_hex(s.color),
            s.background.map(|b| format!(" bg={}", color_hex(b))).unwrap_or_default(),
            if s.bold { " bold" } else { "" }
        ));
    }
    v
}

pub fn media_list(path: &Path) -> Result<Out> {
    let p = load(path)?;
    let text = if p.media.is_empty() {
        "no media".to_string()
    } else {
        p.media.iter().map(media_line).collect::<Vec<_>>().join("\n")
    };
    Ok(Out::new(text, serde_json::to_value(&p.media)?))
}

pub fn probe(file: &str) -> Result<Out> {
    let pr = kerf_media::probe(file)?;
    let j = json!({
        "duration": pr.duration, "width": pr.width, "height": pr.height, "fps": pr.fps,
        "has_video": pr.has_video, "has_audio": pr.has_audio, "is_image": pr.is_image,
        "video_codec": pr.video_codec, "audio_codec": pr.audio_codec,
    });
    Ok(Out::new(
        format!(
            "{file}: {:.3}s {}x{} {:.3}fps video={}({}) audio={}({}) image={}",
            pr.duration, pr.width, pr.height, pr.fps, pr.has_video, pr.video_codec, pr.has_audio, pr.audio_codec, pr.is_image
        ),
        j,
    ))
}

// ---------------------------------------------------------------- edits

pub fn import(path: &Path, files: &[String], append: bool) -> Result<Out> {
    mutate(path, |doc| {
        let mut lines = vec![];
        let mut items = vec![];
        for f in files {
            let item = kerf_render::import_item(f).with_context(|| format!("importing {f}"))?;
            let id = doc.add_media(item).map_err(e)?;
            let m = doc.project().media_item(id).unwrap().clone();
            let mut line = media_line(&m);
            let mut clips = vec![];
            if append {
                clips = doc.append(id).map_err(e)?;
                line.push_str(&format!("\n    → appended as clip(s) {}", fmt_ids(&clips)));
            }
            lines.push(line);
            items.push(json!({"media": id.0, "name": m.name, "duration": secs(m.duration), "clips": ids_json(&clips)}));
        }
        Ok(Out::new(format!("imported:\n  {}", lines.join("\n  ")), json!({"ok": true, "imported": items})))
    })
}

pub fn place(path: &Path, media: u64, at: &str, src_in: Option<&str>, dur: Option<&str>, track: Option<u64>) -> Result<Out> {
    mutate(path, |doc| {
        let fps = doc.project().timeline.fps;
        let at = parse_t(at, fps)?;
        let si = src_in.map(|s| parse_t(s, fps)).transpose()?.unwrap_or(Time::ZERO);
        let d = dur.map(|s| parse_t(s, fps)).transpose()?;
        let (vt, at_) = match track.map(TrackId) {
            Some(t) => match doc.project().timeline.track(t).map(|t| t.kind) {
                Some(TrackKind::Video) => (Some(t), None),
                Some(TrackKind::Audio) => (None, Some(t)),
                None => bail!("no track {}", t.0),
            },
            None => (None, None),
        };
        let ids = doc.place(MediaId(media), at, si, d, vt, at_).map_err(e)?;
        Ok(Out::new(
            format!("placed media {media} at {} as clip(s) {}", fmt_t(at, fps), fmt_ids(&ids)),
            json!({"ok": true, "clips": ids_json(&ids)}),
        ))
    })
}

pub fn append(path: &Path, media: u64) -> Result<Out> {
    mutate(path, |doc| {
        let ids = doc.append(MediaId(media)).map_err(e)?;
        Ok(Out::new(format!("appended media {media} as clip(s) {}", fmt_ids(&ids)), json!({"ok": true, "clips": ids_json(&ids)})))
    })
}

pub fn split(path: &Path, clip: u64, at: &str) -> Result<Out> {
    mutate(path, |doc| {
        let fps = doc.project().timeline.fps;
        let t = parse_t(at, fps)?;
        let ids = doc.split(ClipId(clip), t).map_err(e)?;
        Ok(Out::new(
            format!("split clip {clip} (and linked) at {} → new right-hand clip(s) {}", fmt_t(t, fps), fmt_ids(&ids)),
            json!({"ok": true, "new_clips": ids_json(&ids)}),
        ))
    })
}

pub fn blade(path: &Path, at: &str) -> Result<Out> {
    mutate(path, |doc| {
        let fps = doc.project().timeline.fps;
        let t = parse_t(at, fps)?;
        let n = doc.split_all(t).map_err(e)?;
        Ok(Out::new(format!("bladed {n} clip(s) at {}", fmt_t(t, fps)), json!({"ok": true, "split": n})))
    })
}

pub fn delete(path: &Path, clips: &[u64], ripple: bool) -> Result<Out> {
    mutate(path, |doc| {
        let ids: Vec<ClipId> = clips.iter().map(|c| ClipId(*c)).collect();
        doc.delete(&ids, ripple).map_err(e)?;
        Ok(Out::new(
            format!("deleted clip(s) {}{}", fmt_ids(&ids), if ripple { " (ripple)" } else { "" }),
            json!({"ok": true}),
        ))
    })
}

pub fn cut(path: &Path, from: &str, to: &str, ripple: bool) -> Result<Out> {
    mutate(path, |doc| {
        let fps = doc.project().timeline.fps;
        let (a, b) = (parse_t(from, fps)?, parse_t(to, fps)?);
        if b <= a {
            bail!("--to must be after --from");
        }
        doc.delete_range(TimeRange::from_bounds(a, b), ripple).map_err(e)?;
        Ok(Out::new(
            format!("cut {:.3}s–{:.3}s from all unlocked tracks{}", a.as_secs_f64(), b.as_secs_f64(), if ripple { " (ripple)" } else { " (gap left)" }),
            json!({"ok": true}),
        ))
    })
}

pub fn parse_range(s: &str, fps: f64) -> Result<TimeRange> {
    // "a-b"; split on the first '-' that isn't leading.
    let i = s[1..].find('-').map(|i| i + 1).ok_or_else(|| anyhow!("range {s:?} must look like START-END"))?;
    let (a, b) = (parse_t(&s[..i], fps)?, parse_t(&s[i + 1..], fps)?);
    if b <= a {
        bail!("range {s:?} ends before it starts");
    }
    Ok(TimeRange::from_bounds(a, b))
}

pub fn keep(path: &Path, ranges: &[String]) -> Result<Out> {
    mutate(path, |doc| {
        let fps = doc.project().timeline.fps;
        let rs = ranges.iter().map(|r| parse_range(r, fps)).collect::<Result<Vec<_>>>()?;
        if rs.is_empty() {
            bail!("give at least one START-END range to keep");
        }
        doc.keep_ranges(&rs).map_err(e)?;
        Ok(Out::new(format!("kept {} range(s), everything else removed and closed up", rs.len()), json!({"ok": true})))
    })
}

pub fn close_gaps(path: &Path) -> Result<Out> {
    mutate(path, |doc| {
        let n = doc.close_gaps().map_err(e)?;
        Ok(Out::new(format!("closed {n} gap(s)"), json!({"ok": true, "gaps": n})))
    })
}

pub fn move_clips(path: &Path, clips: &[u64], by: &str, tracks: i32) -> Result<Out> {
    mutate(path, |doc| {
        let fps = doc.project().timeline.fps;
        let neg = by.trim().starts_with('-');
        let mut d = parse_t(by.trim().trim_start_matches(['-', '+']), fps)?;
        if neg {
            d = -d;
        }
        let ids: Vec<ClipId> = clips.iter().map(|c| ClipId(*c)).collect();
        doc.move_clips(&ids, d, tracks).map_err(e)?;
        Ok(Out::new(
            format!("moved clip(s) {} by {:+.3}s{}", fmt_ids(&ids), d.as_secs_f64(), if tracks != 0 { format!(", {tracks:+} track(s)") } else { String::new() }),
            json!({"ok": true}),
        ))
    })
}

pub fn trim(path: &Path, clip: u64, start: Option<&str>, end: Option<&str>) -> Result<Out> {
    mutate(path, |doc| {
        let fps = doc.project().timeline.fps;
        if start.is_none() && end.is_none() {
            bail!("give --start and/or --end (timeline times)");
        }
        if let Some(s) = start {
            doc.trim(ClipId(clip), Edge::Start, parse_t(s, fps)?).map_err(e)?;
        }
        if let Some(s) = end {
            doc.trim(ClipId(clip), Edge::End, parse_t(s, fps)?).map_err(e)?;
        }
        let (_, c) = doc.project().timeline.find_clip(ClipId(clip)).unwrap();
        Ok(Out::new(
            format!("clip {clip} now {:.3}–{:.3}s (src_in {:.3})", c.start.as_secs_f64(), c.end().as_secs_f64(), c.source_in.as_secs_f64()),
            json!({"ok": true, "start": secs(c.start), "end": secs(c.end()), "source_in": secs(c.source_in)}),
        ))
    })
}

fn apply_kv(c: &mut kerf_core::Clip, k: &str, v: &str, fps: f64) -> Result<()> {
    let f = || parse_f(v);
    let t = || parse_t(v, fps);
    match k {
        "volume" => c.volume = f()?.clamp(0.0, 4.0),
        "opacity" => c.opacity = f()?.clamp(0.0, 1.0),
        "fade_in" => c.fade_in = t()?,
        "fade_out" => c.fade_out = t()?,
        "fade" => {
            c.fade_in = t()?;
            c.fade_out = t()?;
        }
        "name" => c.name = v.to_string(),
        "start" => c.start = t()?,
        "source_in" | "in" => c.source_in = t()?,
        "duration" | "dur" => c.duration = t()?,
        "color" => match &mut c.content {
            ClipContent::Solid { color } => *color = parse_color(v)?,
            ClipContent::Text(s) => s.color = parse_color(v)?,
            _ => bail!("color= applies to text and solid clips (for media use grade.*)"),
        },
        _ if k.starts_with("grade.") => {
            let g = &mut c.grade;
            let x = f()?;
            match &k[6..] {
                "exposure" => g.exposure = x,
                "brightness" => g.brightness = x,
                "contrast" => g.contrast = x,
                "saturation" => g.saturation = x,
                "gamma" => g.gamma = x,
                "temperature" => g.temperature = x,
                "tint" => g.tint = x,
                "vignette" => g.vignette = x,
                other => bail!("unknown grade key {other:?} (exposure brightness contrast saturation gamma temperature tint vignette)"),
            }
        }
        "grade" if v == "reset" => c.grade = Default::default(),
        "transform" if v == "reset" => c.transform = Default::default(),
        _ if k.starts_with("transform.") => {
            let tr = &mut c.transform;
            let key = &k[10..];
            if key == "flip_h" {
                tr.flip_h = parse_bool(v)?;
                return Ok(());
            }
            let x = f()?;
            match key {
                "scale" => tr.scale = x,
                "x" => tr.x = x,
                "y" => tr.y = x,
                "rotation" => tr.rotation = x,
                "crop_left" => tr.crop_left = x.clamp(0.0, 0.5),
                "crop_right" => tr.crop_right = x.clamp(0.0, 0.5),
                "crop_top" => tr.crop_top = x.clamp(0.0, 0.5),
                "crop_bottom" => tr.crop_bottom = x.clamp(0.0, 0.5),
                other => bail!("unknown transform key {other:?} (scale x y rotation crop_* flip_h)"),
            }
        }
        _ if k == "text" || k.starts_with("text.") => {
            let ClipContent::Text(s) = &mut c.content else {
                bail!("{k} only applies to text clips");
            };
            match k {
                "text" => {
                    let short = |t: &str| t.lines().next().unwrap_or("Title").chars().take(32).collect::<String>();
                    if c.name.is_empty() || c.name == short(&s.text) {
                        c.name = short(&v.replace("\\n", "\n"));
                    }
                    s.text = v.replace("\\n", "\n");
                }
                "text.size" => s.size = f()?,
                "text.x" => s.x = f()?,
                "text.y" => s.y = f()?,
                "text.color" => s.color = parse_color(v)?,
                "text.background" | "text.bg" => {
                    s.background = if v == "none" || v.is_empty() { None } else { Some(parse_color(v)?) }
                }
                "text.bold" => s.bold = parse_bool(v)?,
                "text.shadow" => s.shadow = parse_bool(v)?,
                other => bail!("unknown text key {other:?} (text text.size text.x text.y text.color text.background text.bold text.shadow)"),
            }
        }
        _ => bail!("unknown key {k:?}"),
    }
    Ok(())
}

pub fn set(path: &Path, clip: u64, kvs: &[String]) -> Result<Out> {
    mutate(path, |doc| {
        let fps = doc.project().timeline.fps;
        let (track, c) = doc
            .project()
            .timeline
            .find_clip(ClipId(clip))
            .ok_or_else(|| anyhow!("no clip {clip}"))?;
        let mut c = c.clone();
        for kv in kvs {
            let (k, v) = kv.split_once('=').ok_or_else(|| anyhow!("expected key=value, got {kv:?}"))?;
            apply_kv(&mut c, k.trim(), v.trim(), fps)?;
        }
        doc.apply(Command::UpdateClip { track, clip: c }).map_err(e)?;
        let c = doc.project().timeline.find_clip(ClipId(clip)).unwrap().1;
        Ok(Out::new(
            format!("clip {clip}: {}", {
                let p = clip_props(c);
                if p.is_empty() { "all defaults".to_string() } else { p.join(" ") }
            }),
            json!({"ok": true, "clip": c}),
        ))
    })
}

pub struct TextArgs<'a> {
    pub text: &'a str,
    pub at: &'a str,
    pub dur: &'a str,
    pub size: Option<f32>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub color: Option<&'a str>,
    pub bg: Option<&'a str>,
    pub bold: bool,
}

pub fn text(path: &Path, a: TextArgs) -> Result<Out> {
    mutate(path, |doc| {
        let fps = doc.project().timeline.fps;
        let mut s = TextStyle::new(a.text.replace("\\n", "\n"));
        if let Some(v) = a.size {
            s.size = v;
        }
        if let Some(v) = a.x {
            s.x = v;
        }
        if let Some(v) = a.y {
            s.y = v;
        }
        if let Some(v) = a.color {
            s.color = parse_color(v)?;
        }
        if let Some(v) = a.bg {
            s.background = Some(parse_color(v)?);
        }
        s.bold = a.bold;
        let (start, dur) = (parse_t(a.at, fps)?, parse_t(a.dur, fps)?);
        let id = doc.add_text(s, start, dur).map_err(e)?;
        let (tr, _) = doc.project().timeline.find_clip(id).unwrap();
        let tname = doc.project().timeline.track(tr).unwrap().name.clone();
        Ok(Out::new(
            format!("added text clip {} on {tname} at {} for {:.3}s", id.0, fmt_t(start, fps), dur.as_secs_f64()),
            json!({"ok": true, "clip": id.0, "track": tr.0}),
        ))
    })
}

pub fn marker_add(path: &Path, at: &str, label: &str) -> Result<Out> {
    mutate(path, |doc| {
        let fps = doc.project().timeline.fps;
        let t = parse_t(at, fps)?;
        doc.add_marker(t, label).map_err(e)?;
        let id = doc.project().timeline.markers.iter().find(|m| m.time == t && m.label == label).map(|m| m.id.0);
        Ok(Out::new(format!("marker {} at {}", id.unwrap_or(0), fmt_t(t, fps)), json!({"ok": true, "marker": id})))
    })
}

pub fn marker_rm(path: &Path, id: u64) -> Result<Out> {
    mutate(path, |doc| {
        doc.apply(Command::RemoveMarker { id: MarkerId(id) }).map_err(e)?;
        Ok(Out::new(format!("removed marker {id}"), json!({"ok": true})))
    })
}

pub fn track_add(path: &Path, kind: &str) -> Result<Out> {
    mutate(path, |doc| {
        let k = match kind {
            "video" | "v" => TrackKind::Video,
            "audio" | "a" => TrackKind::Audio,
            _ => bail!("track kind must be video or audio"),
        };
        let id = doc.add_track(k).map_err(e)?;
        let name = doc.project().timeline.track(id).unwrap().name.clone();
        Ok(Out::new(format!("added track {name} (id {})", id.0), json!({"ok": true, "track": id.0, "name": name})))
    })
}

pub fn track_set(path: &Path, id: u64, kvs: &[String]) -> Result<Out> {
    mutate(path, |doc| {
        let t = doc.project().timeline.track(TrackId(id)).ok_or_else(|| anyhow!("no track {id}"))?.clone();
        let (mut name, mut muted, mut hidden, mut locked, mut volume) = (t.name, t.muted, t.hidden, t.locked, t.volume);
        for kv in kvs {
            let (k, v) = kv.split_once('=').ok_or_else(|| anyhow!("expected key=value, got {kv:?}"))?;
            match k {
                "name" => name = v.into(),
                "mute" | "muted" => muted = parse_bool(v)?,
                "hidden" | "hide" => hidden = parse_bool(v)?,
                "locked" | "lock" => locked = parse_bool(v)?,
                "volume" => volume = parse_f(v)?,
                _ => bail!("unknown track key {k:?} (name mute hidden locked volume)"),
            }
        }
        doc.apply(Command::UpdateTrack { id: TrackId(id), name: name.clone(), muted, hidden, locked, volume }).map_err(e)?;
        Ok(Out::new(
            format!("track {id} {name}: muted={muted} hidden={hidden} locked={locked} volume={volume}"),
            json!({"ok": true}),
        ))
    })
}

pub fn track_rm(path: &Path, id: u64) -> Result<Out> {
    mutate(path, |doc| {
        doc.apply(Command::RemoveTrack { id: TrackId(id) }).map_err(e)?;
        Ok(Out::new(format!("removed track {id}"), json!({"ok": true})))
    })
}

pub fn format(path: &Path, preset_name: Option<&str>, w: Option<u32>, h: Option<u32>, fps: Option<f64>) -> Result<Out> {
    mutate(path, |doc| {
        let tl = &doc.project().timeline;
        let (mut width, mut height, mut f, bg) = (tl.width, tl.height, tl.fps, tl.background);
        if let Some(p) = preset_name {
            (width, height) = preset(p)?;
        }
        width = w.unwrap_or(width);
        height = h.unwrap_or(height);
        f = fps.unwrap_or(f);
        doc.apply(Command::SetFormat { width, height, fps: f, background: bg }).map_err(e)?;
        Ok(Out::new(format!("format {width}x{height} @ {f}fps"), json!({"ok": true, "width": width, "height": height, "fps": f})))
    })
}

pub fn apply_json(path: &Path, src: &str) -> Result<Out> {
    let v: Value = serde_json::from_str(src).context("parsing commands JSON")?;
    let cmds: Vec<Command> = match v {
        Value::Array(a) => a
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()
            .context("bad command in array")?,
        other => vec![serde_json::from_value(other).context("bad command")?],
    };
    let n = cmds.len();
    mutate(path, |doc| {
        doc.apply_all(cmds).map_err(e)?;
        Ok(Out::new(format!("applied {n} command(s) as one undo step"), json!({"ok": true, "applied": n})))
    })
}

pub fn undo(path: &Path) -> Result<Out> {
    let cur = load(path)?;
    match history::undo(path, &cur)? {
        Some(p) => {
            p.save(path).map_err(|e| anyhow!(e))?;
            Ok(Out::new(format!("undone (timeline now {})", fmt_t(p.timeline.duration(), p.timeline.fps)), json!({"ok": true})))
        }
        None => Ok(Out::new("nothing to undo", json!({"ok": false, "reason": "nothing to undo"}))),
    }
}

pub fn redo(path: &Path) -> Result<Out> {
    let cur = load(path)?;
    match history::redo(path, &cur)? {
        Some(p) => {
            p.save(path).map_err(|e| anyhow!(e))?;
            Ok(Out::new(format!("redone (timeline now {})", fmt_t(p.timeline.duration(), p.timeline.fps)), json!({"ok": true})))
        }
        None => Ok(Out::new("nothing to redo", json!({"ok": false, "reason": "nothing to redo"}))),
    }
}

// ---------------------------------------------------------------- analysis

/// Resolve a media id (needs a project) or a file path to (path, duration).
fn target(project: Option<&Path>, t: &str) -> Result<(String, f64)> {
    if let (Ok(id), Some(p)) = (t.parse::<u64>(), project) {
        if !Path::new(t).exists() {
            let proj = load(p)?;
            let m = proj.media_item(MediaId(id)).ok_or_else(|| anyhow!("no media {id} in project"))?;
            return Ok((m.path.clone(), m.duration.as_secs_f64()));
        }
    }
    let pr = kerf_media::probe(t)?;
    Ok((t.to_string(), pr.duration))
}

pub fn silences(project: Option<&Path>, t: &str, db: f64, min: f64) -> Result<Out> {
    let (path, dur) = target(project, t)?;
    let s = kerf_media::detect::silences(&path, db, min, dur)?;
    let total: f64 = s.iter().map(|(a, b)| b - a).sum();
    let mut text = format!("{} silent stretch(es) quieter than {db}dB for ≥{min}s, {total:.2}s total (source times):", s.len());
    for (a, b) in &s {
        text.push_str(&format!("\n  {a:.3} – {b:.3}  ({:.2}s)", b - a));
    }
    Ok(Out::new(text, json!({"silences": s.iter().map(|(a,b)| json!([a,b])).collect::<Vec<_>>(), "duration": dur})))
}

pub fn scenes(project: Option<&Path>, t: &str, threshold: f64) -> Result<Out> {
    let (path, _) = target(project, t)?;
    let s = kerf_media::detect::scenes(&path, threshold)?;
    let text = format!(
        "{} scene change(s) (threshold {threshold}, source times): {}",
        s.len(),
        s.iter().map(|x| format!("{x:.3}")).collect::<Vec<_>>().join(", ")
    );
    Ok(Out::new(text, json!({"scenes": s})))
}

pub fn loudness(project: Option<&Path>, t: &str) -> Result<Out> {
    let (path, _) = target(project, t)?;
    let (i, p) = kerf_media::detect::loudness(&path)?;
    Ok(Out::new(
        format!("integrated loudness {i:.1} LUFS, true peak {p:.1} dBTP (social/web target ≈ -14 LUFS, -1 dBTP)"),
        json!({"lufs": i, "true_peak_db": p}),
    ))
}

/// Cut silent stretches out of the timeline based on the first audio track
/// that has clips. Keeps speech plus `pad` seconds either side.
pub fn auto_cut_silence(path: &Path, db: f64, min: f64, pad: f64, dry_run: bool) -> Result<Out> {
    let run = |doc: &mut Document| -> Result<Out> {
        let p = doc.project().clone();
        let track = p
            .timeline
            .audio_tracks()
            .find(|t| t.clips.iter().any(|c| c.media().is_some()))
            .ok_or_else(|| anyhow!("no audio clips on the timeline"))?;
        let mut cache: std::collections::HashMap<MediaId, Vec<(f64, f64)>> = Default::default();
        let mut speech: Vec<(f64, f64)> = vec![];
        for c in &track.clips {
            let Some(mid) = c.media() else { continue };
            let m = p.media_item(mid).ok_or_else(|| anyhow!("missing media {}", mid.0))?;
            if !cache.contains_key(&mid) {
                let s = kerf_media::detect::silences(&m.path, db, min, m.duration.as_secs_f64())?;
                cache.insert(mid, s);
            }
            let (si, so) = (c.source_in.as_secs_f64(), (c.source_in + c.duration).as_secs_f64());
            let off = c.start.as_secs_f64() - si;
            // Speech = clip span minus silences, in timeline time.
            let mut cursor = si;
            for &(a, b) in &cache[&mid] {
                let (a, b) = (a.max(si), b.min(so));
                if b <= a {
                    continue;
                }
                if a > cursor {
                    speech.push((cursor + off, a + off));
                }
                cursor = cursor.max(b);
            }
            if cursor < so {
                speech.push((cursor + off, so + off));
            }
        }
        let total = p.timeline.duration().as_secs_f64();
        let mut padded: Vec<(f64, f64)> = speech
            .into_iter()
            .map(|(a, b)| ((a - pad).max(0.0), (b + pad).min(total)))
            .collect();
        padded.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut merged: Vec<(f64, f64)> = vec![];
        for r in padded {
            match merged.last_mut() {
                Some(l) if r.0 <= l.1 => l.1 = l.1.max(r.1),
                _ => merged.push(r),
            }
        }
        let kept: f64 = merged.iter().map(|(a, b)| b - a).sum();
        let ranges: Vec<TimeRange> = merged
            .iter()
            .map(|&(a, b)| TimeRange::from_bounds(Time::from_secs_f64(a), Time::from_secs_f64(b)))
            .collect();
        let mut text = format!(
            "speech ranges on {} (timeline): {} — keeps {kept:.2}s of {total:.2}s, removes {:.2}s",
            track.name,
            merged.len(),
            total - kept
        );
        for (a, b) in &merged {
            text.push_str(&format!("\n  {a:.3} – {b:.3}"));
        }
        if merged.is_empty() {
            bail!("everything is silent at {db}dB — try a lower --db (e.g. -45)");
        }
        if !dry_run {
            doc.keep_ranges(&ranges).map_err(e)?;
        } else {
            text.push_str("\n(dry run: nothing changed)");
        }
        Ok(Out::new(text, json!({"ok": true, "keep": merged.iter().map(|(a,b)| json!([a,b])).collect::<Vec<_>>(), "removed_secs": total - kept})))
    };
    mutate(path, run)
}

// ---------------------------------------------------------------- seeing

pub fn frame(path: &Path, at: &str, width: u32, out: Option<&Path>) -> Result<Out> {
    let p = load(path)?;
    let t = parse_t(at, p.timeline.fps)?;
    let f = kerf_render::render_frame(&p, t, width);
    let png = image::encode_png(&f)?;
    let mut text = format!("frame at {} ({}x{})", fmt_t(t, p.timeline.fps), f.w, f.h);
    if let Some(o) = out {
        std::fs::write(o, &png)?;
        text.push_str(&format!(" → {}", o.display()));
    }
    let mut r = Out::new(text, json!({"ok": true, "path": out, "width": f.w, "height": f.h}));
    r.png = Some(png);
    Ok(r)
}

pub fn contact(path: &Path, cols: u32, count: u32, width: u32, out: Option<&Path>) -> Result<Out> {
    let p = load(path)?;
    let (sheet, times) = image::contact_sheet(&p, cols.max(1), count.max(1), width)?;
    let png = image::encode_png(&sheet)?;
    let fps = p.timeline.fps;
    let mut text = format!(
        "contact sheet {}x{}: {} frames at {}",
        sheet.w,
        sheet.h,
        times.len(),
        times.iter().map(|t| format!("{:.2}s", t.as_secs_f64())).collect::<Vec<_>>().join(", ")
    );
    let _ = fps;
    if let Some(o) = out {
        std::fs::write(o, &png)?;
        text.push_str(&format!(" → {}", o.display()));
    }
    let mut r = Out::new(text, json!({"ok": true, "path": out, "times": times.iter().map(|t| secs(*t)).collect::<Vec<_>>()}));
    r.png = Some(png);
    Ok(r)
}

// ---------------------------------------------------------------- proxies / export

pub fn proxy(path: &Path, which: &[String], quiet: bool) -> Result<Out> {
    mutate(path, |doc| {
        let all = which.is_empty() || which.iter().any(|w| w == "all");
        let items: Vec<kerf_core::MediaItem> = doc
            .project()
            .media
            .iter()
            .filter(|m| m.has_video && !m.is_image)
            .filter(|m| all || which.iter().any(|w| w == &m.id.0.to_string()))
            .cloned()
            .collect();
        let mut done = vec![];
        for mut m in items {
            let dur = m.duration.as_secs_f64();
            let name = m.name.clone();
            let px = kerf_media::proxy::make_proxy(&m.path, dur, |f| {
                if !quiet {
                    eprint!("\rproxy {name}: {:>3.0}%", f * 100.0);
                }
            })?;
            if !quiet {
                eprintln!();
            }
            if m.proxy.as_deref() != Some(&px) {
                m.proxy = Some(px.clone());
                doc.apply(Command::UpdateMedia { item: m.clone() }).map_err(e)?;
            }
            done.push(json!({"media": m.id.0, "proxy": px}));
        }
        Ok(Out::new(format!("{} proxy file(s) ready", done.len()), json!({"ok": true, "proxies": done})))
    })
}

pub struct ExportArgs<'a> {
    pub output: &'a str,
    pub codec: Option<&'a str>,
    pub quality: Option<u32>,
    pub preset: Option<&'a str>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub on: Option<&'a str>,
    pub quiet: bool,
}

pub fn settings(p: &Project, a: &ExportArgs) -> Result<kerf_render::ExportSettings> {
    let mut s = kerf_render::ExportSettings::for_project(p, a.output);
    if let Some(c) = a.codec {
        s.codec = kerf_render::Codec::parse(c).ok_or_else(|| anyhow!("unknown codec {c:?} (h264 h265 vaapi prores)"))?;
        if s.codec == kerf_render::Codec::H265 {
            s.quality = 24;
        }
    }
    if let Some(q) = a.quality {
        s.quality = q;
    }
    if let Some(pr) = a.preset {
        s.preset = pr.into();
    }
    if let Some(w) = a.width {
        s.width = w;
    }
    if let Some(h) = a.height {
        s.height = h;
    }
    if let Some(f) = a.fps {
        s.fps = f;
    }
    let fps = p.timeline.fps;
    if a.from.is_some() || a.to.is_some() {
        let start = a.from.map(|x| parse_t(x, fps)).transpose()?.unwrap_or(Time::ZERO);
        let end = a.to.map(|x| parse_t(x, fps)).transpose()?.unwrap_or(p.timeline.duration());
        if end <= start {
            bail!("--to must be after --from");
        }
        s.range = Some(TimeRange::from_bounds(start, end));
    }
    Ok(s)
}

pub fn export(path: &Path, a: ExportArgs) -> Result<Out> {
    if let Some(host) = a.on {
        return crate::remote::export_remote(path, host, &a);
    }
    let p = load(path)?;
    let s = settings(&p, &a)?;
    let started = std::time::Instant::now();
    let mut last = -1i32;
    kerf_render::export(&p, &s, |f, stage| {
        let pct = (f * 100.0) as i32;
        if !a.quiet && pct != last {
            last = pct;
            eprint!("\r{stage:<14} {pct:>3}%");
        }
        true
    })?;
    if !a.quiet {
        eprintln!();
    }
    let secs_taken = started.elapsed().as_secs_f64();
    let size = std::fs::metadata(a.output).map(|m| m.len()).unwrap_or(0);
    Ok(Out::new(
        format!(
            "exported {} ({}x{} {:?}, {:.1} MB) in {secs_taken:.1}s",
            a.output,
            s.width,
            s.height,
            s.codec,
            size as f64 / 1e6
        ),
        json!({"ok": true, "output": a.output, "bytes": size, "seconds": secs_taken}),
    ))
}

pub fn open(path: &Path) -> Result<Out> {
    let abs: PathBuf = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    std::process::Command::new("kerf-studio")
        .arg(&abs)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("launching kerf-studio (is it installed and on PATH?)")?;
    Ok(Out::new(format!("opened {} in kerf studio", abs.display()), json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_and_colors() {
        let r = parse_range("1:02.5-1:10", 30.0).unwrap();
        assert_eq!(r.start, Time::from_secs_f64(62.5));
        assert_eq!(r.end(), Time::from_secs_f64(70.0));
        assert!(parse_range("5-3", 30.0).is_err());
        assert_eq!(parse_color("#ff000080").unwrap(), [255, 0, 0, 128]);
        assert_eq!(parse_color("00ff00").unwrap(), [0, 255, 0, 255]);
    }

    #[test]
    fn set_keys() {
        let mut c = kerf_core::Clip::new(
            ClipId(1),
            ClipContent::Text(TextStyle::new("a")),
            Time::ZERO,
            Time::ZERO,
            Time::from_secs_f64(1.0),
        );
        apply_kv(&mut c, "grade.saturation", "0", 30.0).unwrap();
        apply_kv(&mut c, "text", "Hi\\nthere", 30.0).unwrap();
        apply_kv(&mut c, "fade_in", "0.5", 30.0).unwrap();
        assert_eq!(c.grade.saturation, 0.0);
        assert_eq!(c.fade_in, Time::from_secs_f64(0.5));
        assert!(matches!(&c.content, ClipContent::Text(s) if s.text == "Hi\nthere"));
        assert!(apply_kv(&mut c, "bogus", "1", 30.0).is_err());
    }
}
