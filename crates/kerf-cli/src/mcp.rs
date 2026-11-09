//! `kerf mcp`: a Model Context Protocol server over stdio (JSON-RPC 2.0,
//! one message per line). Every tool is a thin wrapper over [`crate::api`],
//! the same layer the CLI uses.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

use crate::api::{self, ExportArgs, Out, TextArgs};

const PROTOCOL: &str = "2025-06-18";

fn tool(name: &str, desc: &str, props: Value, required: &[&str]) -> Value {
    let mut p = props.as_object().cloned().unwrap_or_default();
    p.insert(
        "project".into(),
        json!({"type": "string", "description": "Path to the .kerf project (default: $KERF_PROJECT)"}),
    );
    json!({
        "name": name,
        "description": desc,
        "inputSchema": {"type": "object", "properties": p, "required": required},
    })
}

const T: &str = "Time: seconds (12.5), MM:SS.ms or HH:MM:SS:FF";

fn tools() -> Vec<Value> {
    let t = |d: &str| json!({"type": "string", "description": format!("{d}. {T}")});
    let ids = |d: &str| json!({"type": "array", "items": {"type": "integer"}, "description": d});
    vec![
        tool("info", "Show the whole timeline compactly: format, tracks, every clip (id, timeline span, source in-point, media, link group, non-default props), markers and media. Call this first and after edits.", json!({}), &[]),
        tool("new_project", "Create a new empty .kerf project.", json!({
            "path": {"type": "string"}, "name": {"type": "string"},
            "preset": {"type": "string", "description": "1080p | 720p | 4k | vertical (1080x1920) | square | 4:5"},
            "fps": {"type": "number"}, "force": {"type": "boolean"}}), &["path"]),
        tool("import", "Import media files into the bin; with append=true also put each at the end of the timeline (video + linked audio).", json!({
            "files": {"type": "array", "items": {"type": "string"}}, "append": {"type": "boolean"}}), &["files"]),
        tool("place", "Place a media item on the timeline at a time (video on a free video track, audio on a linked audio clip).", json!({
            "media": {"type": "integer"}, "at": t("Timeline position"), "source_in": t("In-point inside the media"),
            "duration": t("Length (default: rest of the media)"), "track": {"type": "integer"}}), &["media", "at"]),
        tool("append", "Append a media item after the end of the first video track.", json!({"media": {"type": "integer"}}), &["media"]),
        tool("split", "Split a clip (and its linked audio/video) at a timeline time.", json!({"clip": {"type": "integer"}, "at": t("Timeline time")}), &["clip", "at"]),
        tool("blade", "Split every unlocked track at a timeline time.", json!({"at": t("Timeline time")}), &["at"]),
        tool("delete", "Delete clips. ripple=true closes the hole on those tracks.", json!({"clips": ids("Clip ids"), "ripple": {"type": "boolean"}}), &["clips"]),
        tool("cut_range", "Remove a timeline range from all unlocked tracks (ripple by default, keeps A/V sync).", json!({
            "from": t("Range start"), "to": t("Range end"), "ripple": {"type": "boolean", "default": true}}), &["from", "to"]),
        tool("keep_ranges", "Keep only these timeline ranges, in order, removing everything else (highlight reels, silence removal).", json!({
            "ranges": {"type": "array", "items": {"type": "string"}, "description": "Each 'START-END', e.g. '3.2-7.9'"}}), &["ranges"]),
        tool("close_gaps", "Remove empty stretches where no track has anything.", json!({}), &[]),
        tool("move", "Move clips (with linked partners) by a time delta, optionally up/down tracks.", json!({
            "clips": ids("Clip ids"), "by": {"type": "string", "description": "Delta, e.g. '2.5' or '-1.0'"}, "tracks": {"type": "integer"}}), &["clips", "by"]),
        tool("trim", "Trim a clip's start and/or end edge to timeline times (linked clips follow).", json!({
            "clip": {"type": "integer"}, "start": t("New start"), "end": t("New end")}), &["clip"]),
        tool("set_clip", "Set clip properties. Keys: volume opacity fade_in fade_out fade name start source_in duration color, grade.{exposure,brightness,contrast,saturation,gamma,temperature,tint,vignette}, transform.{scale,x,y,rotation,crop_left,crop_right,crop_top,crop_bottom,flip_h}, text text.{size,x,y,color,background,bold,shadow}. grade=reset / transform=reset.", json!({
            "clip": {"type": "integer"}, "props": {"type": "object", "description": "key → value, e.g. {\"grade.saturation\": 1.2, \"fade_in\": \"0.5\"}"}}), &["clip", "props"]),
        tool("add_text", "Add a title on the top free video track. size is a fraction of frame height (0.08); x/y are the centre in 0..1.", json!({
            "text": {"type": "string"}, "at": t("Start"), "duration": t("Length"), "size": {"type": "number"},
            "x": {"type": "number"}, "y": {"type": "number"}, "color": {"type": "string"}, "background": {"type": "string"}, "bold": {"type": "boolean"}}), &["text", "at", "duration"]),
        tool("add_marker", "Add a timeline marker.", json!({"at": t("Time"), "label": {"type": "string"}}), &["at"]),
        tool("set_format", "Change output format.", json!({"preset": {"type": "string"}, "width": {"type": "integer"}, "height": {"type": "integer"}, "fps": {"type": "number"}}), &[]),
        tool("undo", "Undo the last edit to the project file.", json!({}), &[]),
        tool("redo", "Redo the last undone edit.", json!({}), &[]),
        tool("silences", "Find silent stretches in a media item (id) or file. Returns SOURCE times.", json!({
            "target": {"type": "string", "description": "Media id or file path"}, "db": {"type": "number", "default": -35}, "min": {"type": "number", "default": 0.5}}), &["target"]),
        tool("scenes", "Find scene changes / hard cuts in a media item or file (source times).", json!({
            "target": {"type": "string"}, "threshold": {"type": "number", "default": 0.3}}), &["target"]),
        tool("loudness", "Integrated loudness (LUFS) and true peak of a media item or file.", json!({"target": {"type": "string"}}), &["target"]),
        tool("auto_cut_silence", "Cut silences out of the timeline using the first audio track, keeping speech plus padding. dry_run=true only reports.", json!({
            "db": {"type": "number", "default": -35}, "min": {"type": "number", "default": 0.5}, "pad": {"type": "number", "default": 0.15}, "dry_run": {"type": "boolean"}}), &[]),
        tool("render_frame", "Render the composited frame at a timeline time and return it as an image — use it to check your edit.", json!({
            "at": t("Timeline time"), "width": {"type": "integer", "default": 640}}), &["at"]),
        tool("contact_sheet", "Render a grid of frames sampled evenly across the timeline (labelled with times) — an overview of the whole edit in one image.", json!({
            "cols": {"type": "integer", "default": 4}, "count": {"type": "integer", "default": 12}, "width": {"type": "integer", "default": 1280}}), &[]),
        tool("export", "Render the timeline to a video file. on=<ssh host> renders on another machine (e.g. a Tailscale host).", json!({
            "output": {"type": "string"}, "codec": {"type": "string", "description": "h264 | h265 | vaapi | prores"},
            "quality": {"type": "integer"}, "preset": {"type": "string"}, "width": {"type": "integer"}, "height": {"type": "integer"},
            "fps": {"type": "number"}, "from": t("Range start"), "to": t("Range end"), "on": {"type": "string"}}), &["output"]),
        tool("apply_commands", "Apply raw kerf Command JSON (one object or an array) as one undo step. See AGENTS.md for the format.", json!({
            "commands": {"description": "Command object or array of them"}}), &["commands"]),
    ]
}

struct A<'a>(&'a Map<String, Value>);

impl A<'_> {
    fn s(&self, k: &str) -> Option<String> {
        match self.0.get(k)? {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            Value::Null => None,
            other => Some(other.to_string()),
        }
    }
    fn req(&self, k: &str) -> Result<String> {
        self.s(k).ok_or_else(|| anyhow!("missing argument {k:?}"))
    }
    fn u(&self, k: &str) -> Option<u64> {
        self.0.get(k).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
    }
    fn f(&self, k: &str) -> Option<f64> {
        self.0.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
    }
    fn b(&self, k: &str) -> Option<bool> {
        self.0.get(k).and_then(|v| v.as_bool())
    }
    fn ids(&self, k: &str) -> Vec<u64> {
        self.0
            .get(k)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_u64()).collect())
            .unwrap_or_default()
    }
    fn strs(&self, k: &str) -> Vec<String> {
        self.0
            .get(k)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default()
    }
    fn project(&self) -> Result<PathBuf> {
        self.s("project")
            .or_else(|| std::env::var("KERF_PROJECT").ok())
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("no project: pass \"project\" or set KERF_PROJECT"))
    }
}

fn call(name: &str, args: &Map<String, Value>) -> Result<Out> {
    let a = A(args);
    let pr = || a.project();
    match name {
        "info" => api::info(&pr()?),
        "new_project" => api::new_project(
            &PathBuf::from(a.req("path")?),
            a.s("name"),
            a.s("preset").as_deref(),
            None,
            None,
            a.f("fps"),
            a.b("force").unwrap_or(false),
        ),
        "import" => api::import(&pr()?, &a.strs("files"), a.b("append").unwrap_or(false)),
        "place" => api::place(
            &pr()?,
            a.u("media").ok_or_else(|| anyhow!("missing media"))?,
            &a.req("at")?,
            a.s("source_in").as_deref(),
            a.s("duration").as_deref(),
            a.u("track"),
        ),
        "append" => api::append(&pr()?, a.u("media").ok_or_else(|| anyhow!("missing media"))?),
        "split" => api::split(&pr()?, a.u("clip").ok_or_else(|| anyhow!("missing clip"))?, &a.req("at")?),
        "blade" => api::blade(&pr()?, &a.req("at")?),
        "delete" => api::delete(&pr()?, &a.ids("clips"), a.b("ripple").unwrap_or(false)),
        "cut_range" => api::cut(&pr()?, &a.req("from")?, &a.req("to")?, a.b("ripple").unwrap_or(true)),
        "keep_ranges" => api::keep(&pr()?, &a.strs("ranges")),
        "close_gaps" => api::close_gaps(&pr()?),
        "move" => api::move_clips(&pr()?, &a.ids("clips"), &a.req("by")?, a.u("tracks").map(|x| x as i32).or_else(|| a.f("tracks").map(|x| x as i32)).unwrap_or(0)),
        "trim" => api::trim(&pr()?, a.u("clip").ok_or_else(|| anyhow!("missing clip"))?, a.s("start").as_deref(), a.s("end").as_deref()),
        "set_clip" => {
            let props = args.get("props").and_then(|v| v.as_object()).ok_or_else(|| anyhow!("props must be an object"))?;
            let kvs: Vec<String> = props
                .iter()
                .map(|(k, v)| match v {
                    Value::String(s) => format!("{k}={s}"),
                    other => format!("{k}={other}"),
                })
                .collect();
            api::set(&pr()?, a.u("clip").ok_or_else(|| anyhow!("missing clip"))?, &kvs)
        }
        "add_text" => {
            let (text, at, dur) = (a.req("text")?, a.req("at")?, a.req("duration")?);
            let (color, bg) = (a.s("color"), a.s("background"));
            api::text(
                &pr()?,
                TextArgs {
                    text: &text,
                    at: &at,
                    dur: &dur,
                    size: a.f("size").map(|x| x as f32),
                    x: a.f("x").map(|x| x as f32),
                    y: a.f("y").map(|x| x as f32),
                    color: color.as_deref(),
                    bg: bg.as_deref(),
                    bold: a.b("bold").unwrap_or(false),
                },
            )
        }
        "add_marker" => api::marker_add(&pr()?, &a.req("at")?, &a.s("label").unwrap_or_default()),
        "set_format" => api::format(
            &pr()?,
            a.s("preset").as_deref(),
            a.u("width").map(|x| x as u32),
            a.u("height").map(|x| x as u32),
            a.f("fps"),
        ),
        "undo" => api::undo(&pr()?),
        "redo" => api::redo(&pr()?),
        "silences" => api::silences(pr().ok().as_deref(), &a.req("target")?, a.f("db").unwrap_or(-35.0), a.f("min").unwrap_or(0.5)),
        "scenes" => api::scenes(pr().ok().as_deref(), &a.req("target")?, a.f("threshold").unwrap_or(0.3)),
        "loudness" => api::loudness(pr().ok().as_deref(), &a.req("target")?),
        "auto_cut_silence" => api::auto_cut_silence(
            &pr()?,
            a.f("db").unwrap_or(-35.0),
            a.f("min").unwrap_or(0.5),
            a.f("pad").unwrap_or(0.15),
            a.b("dry_run").unwrap_or(false),
        ),
        "render_frame" => api::frame(&pr()?, &a.req("at")?, a.u("width").unwrap_or(640) as u32, None),
        "contact_sheet" => api::contact(
            &pr()?,
            a.u("cols").unwrap_or(4) as u32,
            a.u("count").unwrap_or(12) as u32,
            a.u("width").unwrap_or(1280) as u32,
            None,
        ),
        "export" => {
            let output = a.req("output")?;
            let (codec, preset, from, to, on) = (a.s("codec"), a.s("preset"), a.s("from"), a.s("to"), a.s("on"));
            api::export(
                &pr()?,
                ExportArgs {
                    output: &output,
                    codec: codec.as_deref(),
                    quality: a.u("quality").map(|x| x as u32),
                    preset: preset.as_deref(),
                    width: a.u("width").map(|x| x as u32),
                    height: a.u("height").map(|x| x as u32),
                    fps: a.f("fps"),
                    from: from.as_deref(),
                    to: to.as_deref(),
                    on: on.as_deref(),
                    quiet: true,
                },
            )
        }
        "apply_commands" => {
            let c = args.get("commands").ok_or_else(|| anyhow!("missing commands"))?;
            let src = match c {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            api::apply_json(&pr()?, &src)
        }
        _ => Err(anyhow!("unknown tool {name:?}")),
    }
}

pub fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(A[(n >> 18) as usize & 63] as char);
        s.push(A[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    s
}

fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let result: Result<Value, (i64, String)> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "kerf", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "kerf is a timeline video editor. Edit a .kerf project: call info first, analyse with silences/scenes, edit with split/cut_range/keep_ranges/set_clip/add_text, look with render_frame or contact_sheet, then export. Clip ids come from info. Every edit is saved and undoable; the kerf studio GUI reloads the project live.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("");
            let empty = Map::new();
            let args = msg["params"]["arguments"].as_object().unwrap_or(&empty);
            Ok(match call(name, args) {
                Ok(out) => {
                    let mut content = vec![json!({"type": "text", "text": out.text})];
                    if let Some(png) = out.png {
                        content.push(json!({"type": "image", "data": base64(&png), "mimeType": "image/png"}));
                    }
                    json!({"content": content, "structuredContent": out.json, "isError": false})
                }
                Err(e) => json!({"content": [{"type": "text", "text": format!("error: {e:#}")}], "isError": true}),
            })
        }
        m if m.starts_with("notifications/") => return None,
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    let id = id?; // notifications get no reply
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, m)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": m}}),
    })
}

pub fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let r: Vec<Value> = batch.iter().filter_map(handle).collect();
                (!r.is_empty()).then(|| Value::Array(r))
            }
            Ok(msg) => handle(&msg),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}})),
        };
        if let Some(r) = reply {
            writeln!(stdout, "{r}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(super::base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(super::base64(b"f"), "Zg==");
    }

    #[test]
    fn notifications_get_no_reply() {
        let r = super::handle(&serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        assert!(r.is_none());
        let r = super::handle(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
        assert!(r["result"]["tools"].as_array().unwrap().len() > 20);
    }
}
