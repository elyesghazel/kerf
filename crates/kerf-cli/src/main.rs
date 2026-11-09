//! `kerf`: headless editing for humans in a terminal and for AI agents.
//!
//! Every subcommand edits a `.kerf` project file in place (atomically, with
//! undo history in a sidecar). The studio GUI watches the same file, so an
//! agent cutting here shows up live in an open editor.

mod api;
mod history;
mod image;
mod mcp;
mod remote;

use std::io::Read;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};

use api::{ExportArgs, Out, TextArgs};

#[derive(Parser)]
#[command(name = "kerf", version, about = "kerf: a fast timeline video editor you can drive from the shell or an AI agent", long_about = None)]
struct Cli {
    /// The .kerf project file
    #[arg(short, long, global = true, env = "KERF_PROJECT")]
    project: Option<PathBuf>,
    /// Machine-readable JSON output
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new project
    New {
        file: PathBuf,
        #[arg(long)]
        name: Option<String>,
        /// 1080p, 720p, 4k, vertical, square, 4:5
        #[arg(long)]
        preset: Option<String>,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
        #[arg(long)]
        fps: Option<f64>,
        #[arg(long)]
        force: bool,
    },
    /// Import media files into the bin
    Import {
        #[arg(required = true)]
        files: Vec<String>,
        /// Also append each to the end of the timeline
        #[arg(long)]
        append: bool,
    },
    /// Show the timeline: tracks, clips, markers, media
    Info,
    /// List media in the bin
    Media,
    /// Probe a media file
    Probe { file: String },
    /// Place media on the timeline
    Place {
        media: u64,
        #[arg(long)]
        at: String,
        /// Source in-point
        #[arg(long = "in")]
        src_in: Option<String>,
        #[arg(long)]
        dur: Option<String>,
        #[arg(long)]
        track: Option<u64>,
    },
    /// Append media after the end of the first video track
    Append { media: u64 },
    /// Split a clip (and linked clips) at a timeline time
    Split {
        clip: u64,
        #[arg(long)]
        at: String,
    },
    /// Split every unlocked track at a time
    Blade {
        #[arg(long)]
        at: String,
    },
    /// Delete clips
    Delete {
        #[arg(required = true)]
        clips: Vec<u64>,
        #[arg(long)]
        ripple: bool,
    },
    /// Cut a time range out of all unlocked tracks (ripples by default)
    Cut {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        no_ripple: bool,
    },
    /// Keep only the given START-END ranges, close everything else up
    Keep {
        #[arg(required = true)]
        ranges: Vec<String>,
    },
    /// Remove empty stretches on the timeline
    CloseGaps,
    /// Move clips by a time delta
    Move {
        #[arg(required = true)]
        clips: Vec<u64>,
        #[arg(long, allow_hyphen_values = true)]
        by: String,
        /// Hop this many tracks (+ up, - down)
        #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
        tracks: i32,
    },
    /// Trim a clip edge to a timeline time
    Trim {
        clip: u64,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
    },
    /// Set clip properties: key=value ...
    Set {
        clip: u64,
        #[arg(required = true, allow_hyphen_values = true)]
        props: Vec<String>,
    },
    /// Add a title
    Text {
        text: String,
        #[arg(long)]
        at: String,
        #[arg(long, default_value = "3")]
        dur: String,
        /// Fraction of frame height
        #[arg(long)]
        size: Option<f32>,
        #[arg(long)]
        x: Option<f32>,
        #[arg(long)]
        y: Option<f32>,
        #[arg(long)]
        color: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        #[arg(long)]
        bold: bool,
    },
    /// Markers
    Marker {
        #[command(subcommand)]
        cmd: MarkerCmd,
    },
    /// Tracks
    Track {
        #[command(subcommand)]
        cmd: TrackCmd,
    },
    /// Change output format
    Format {
        #[arg(long)]
        preset: Option<String>,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
        #[arg(long)]
        fps: Option<f64>,
    },
    /// Apply raw Command JSON (file or - for stdin) as one undo step
    Apply { source: String },
    /// Undo the last edit
    Undo,
    /// Redo
    Redo,
    /// Find silences in a media item (id) or file
    Silences {
        target: String,
        #[arg(long, default_value_t = -35.0, allow_hyphen_values = true)]
        db: f64,
        #[arg(long, default_value_t = 0.5)]
        min: f64,
    },
    /// Find scene changes in a media item (id) or file
    Scenes {
        target: String,
        #[arg(long, default_value_t = 0.3)]
        threshold: f64,
    },
    /// Measure loudness of a media item (id) or file
    Loudness { target: String },
    /// Remove silences from the timeline (based on the first audio track)
    AutoCutSilence {
        #[arg(long, default_value_t = -35.0, allow_hyphen_values = true)]
        db: f64,
        #[arg(long, default_value_t = 0.5)]
        min: f64,
        #[arg(long, default_value_t = 0.15)]
        pad: f64,
        #[arg(long)]
        dry_run: bool,
    },
    /// Render one frame to PNG
    Frame {
        #[arg(long)]
        at: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 960)]
        width: u32,
    },
    /// Render a contact sheet of frames across the timeline
    Contact {
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 4)]
        cols: u32,
        #[arg(long, default_value_t = 12)]
        count: u32,
        #[arg(long, default_value_t = 1280)]
        width: u32,
    },
    /// Generate low-res proxies for smooth preview (ids or "all")
    Proxy { media: Vec<String> },
    /// Render the timeline to a file
    Export {
        #[arg(short, long)]
        output: String,
        /// h264, h265, vaapi, prores
        #[arg(long)]
        codec: Option<String>,
        #[arg(long)]
        quality: Option<u32>,
        #[arg(long)]
        preset: Option<String>,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
        #[arg(long)]
        fps: Option<f64>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        /// Render on another machine over SSH (e.g. a Tailscale host)
        #[arg(long)]
        on: Option<String>,
        #[arg(short, long)]
        quiet: bool,
    },
    /// Open the project in kerf studio
    Open,
    /// Run an MCP server on stdio for AI agents
    Mcp,
}

#[derive(Subcommand)]
enum MarkerCmd {
    Add {
        #[arg(long)]
        at: String,
        label: Option<String>,
    },
    Rm { id: u64 },
}

#[derive(Subcommand)]
enum TrackCmd {
    /// video | audio
    Add { kind: String },
    /// key=value: name mute hidden locked volume
    Set {
        id: u64,
        #[arg(required = true)]
        props: Vec<String>,
    },
    Rm { id: u64 },
}

fn run(cli: Cli) -> Result<Option<Out>> {
    let proj = || {
        cli.project
            .clone()
            .ok_or_else(|| anyhow!("no project: pass -p <file.kerf> or set KERF_PROJECT"))
    };
    let out = match cli.cmd {
        Cmd::New { ref file, ref name, ref preset, width, height, fps, force } => {
            api::new_project(file, name.clone(), preset.as_deref(), width, height, fps, force)?
        }
        Cmd::Import { ref files, append } => api::import(&proj()?, files, append)?,
        Cmd::Info => api::info(&proj()?)?,
        Cmd::Media => api::media_list(&proj()?)?,
        Cmd::Probe { ref file } => api::probe(file)?,
        Cmd::Place { media, ref at, ref src_in, ref dur, track } => {
            api::place(&proj()?, media, at, src_in.as_deref(), dur.as_deref(), track)?
        }
        Cmd::Append { media } => api::append(&proj()?, media)?,
        Cmd::Split { clip, ref at } => api::split(&proj()?, clip, at)?,
        Cmd::Blade { ref at } => api::blade(&proj()?, at)?,
        Cmd::Delete { ref clips, ripple } => api::delete(&proj()?, clips, ripple)?,
        Cmd::Cut { ref from, ref to, no_ripple } => api::cut(&proj()?, from, to, !no_ripple)?,
        Cmd::Keep { ref ranges } => api::keep(&proj()?, ranges)?,
        Cmd::CloseGaps => api::close_gaps(&proj()?)?,
        Cmd::Move { ref clips, ref by, tracks } => api::move_clips(&proj()?, clips, by, tracks)?,
        Cmd::Trim { clip, ref start, ref end } => api::trim(&proj()?, clip, start.as_deref(), end.as_deref())?,
        Cmd::Set { clip, ref props } => api::set(&proj()?, clip, props)?,
        Cmd::Text { ref text, ref at, ref dur, size, x, y, ref color, ref bg, bold } => api::text(
            &proj()?,
            TextArgs { text, at, dur, size, x, y, color: color.as_deref(), bg: bg.as_deref(), bold },
        )?,
        Cmd::Marker { ref cmd } => match cmd {
            MarkerCmd::Add { at, label } => api::marker_add(&proj()?, at, label.as_deref().unwrap_or(""))?,
            MarkerCmd::Rm { id } => api::marker_rm(&proj()?, *id)?,
        },
        Cmd::Track { ref cmd } => match cmd {
            TrackCmd::Add { kind } => api::track_add(&proj()?, kind)?,
            TrackCmd::Set { id, props } => api::track_set(&proj()?, *id, props)?,
            TrackCmd::Rm { id } => api::track_rm(&proj()?, *id)?,
        },
        Cmd::Format { ref preset, width, height, fps } => api::format(&proj()?, preset.as_deref(), width, height, fps)?,
        Cmd::Apply { ref source } => {
            let src = if source == "-" {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                std::fs::read_to_string(source)?
            };
            api::apply_json(&proj()?, &src)?
        }
        Cmd::Undo => api::undo(&proj()?)?,
        Cmd::Redo => api::redo(&proj()?)?,
        Cmd::Silences { ref target, db, min } => api::silences(cli.project.as_deref(), target, db, min)?,
        Cmd::Scenes { ref target, threshold } => api::scenes(cli.project.as_deref(), target, threshold)?,
        Cmd::Loudness { ref target } => api::loudness(cli.project.as_deref(), target)?,
        Cmd::AutoCutSilence { db, min, pad, dry_run } => api::auto_cut_silence(&proj()?, db, min, pad, dry_run)?,
        Cmd::Frame { ref at, ref output, width } => {
            let o = output.clone().unwrap_or_else(|| PathBuf::from("frame.png"));
            api::frame(&proj()?, at, width, Some(&o))?
        }
        Cmd::Contact { ref output, cols, count, width } => {
            let o = output.clone().unwrap_or_else(|| PathBuf::from("contact.png"));
            api::contact(&proj()?, cols, count, width, Some(&o))?
        }
        Cmd::Proxy { ref media } => api::proxy(&proj()?, media, cli.json)?,
        Cmd::Export { ref output, ref codec, quality, ref preset, width, height, fps, ref from, ref to, ref on, quiet } => {
            api::export(
                &proj()?,
                ExportArgs {
                    output,
                    codec: codec.as_deref(),
                    quality,
                    preset: preset.as_deref(),
                    width,
                    height,
                    fps,
                    from: from.as_deref(),
                    to: to.as_deref(),
                    on: on.as_deref(),
                    quiet,
                },
            )?
        }
        Cmd::Open => api::open(&proj()?)?,
        Cmd::Mcp => {
            mcp::serve()?;
            return Ok(None);
        }
    };
    Ok(Some(out))
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli) {
        Ok(Some(out)) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&out.json).unwrap());
            } else {
                println!("{}", out.text);
            }
        }
        Ok(None) => {}
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({"ok": false, "error": format!("{e:#}")}));
            } else {
                eprintln!("error: {e:#}");
            }
            std::process::exit(1);
        }
    }
}
