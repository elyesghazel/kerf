//! The project document: a media bin plus a timeline of tracks and clips.
//!
//! This module is intentionally just *data* + read-only queries. All mutation
//! goes through [`crate::command`], so there is a single, auditable edit path.

use serde::{Deserialize, Serialize};

use crate::ids::{ClipId, IdGen, MarkerId, MediaId, TrackId};
use crate::time::{Time, TimeRange};

/// A piece of source media imported into the project (a file on disk).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaItem {
    pub id: MediaId,
    pub path: String,
    #[serde(default)]
    pub name: String,
    /// Full duration of the source (images get a nominal long duration).
    pub duration: Time,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    #[serde(default = "yes")]
    pub has_video: bool,
    #[serde(default)]
    pub has_audio: bool,
    #[serde(default)]
    pub is_image: bool,
    /// A low-res, short-GOP copy used for smooth preview on weak machines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
}

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}

/// What a clip shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClipContent {
    /// A window into a media file.
    Media { media: MediaId },
    /// A generated title.
    Text(TextStyle),
    /// A flat colour matte.
    Solid { color: [u8; 4] },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub text: String,
    /// Font size as a fraction of the output height (0.08 = 8%).
    #[serde(default = "default_text_size")]
    pub size: f32,
    #[serde(default = "white")]
    pub color: [u8; 4],
    /// Optional box behind the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<[u8; 4]>,
    /// Centre of the text block, 0..1 across / down the frame.
    #[serde(default = "half")]
    pub x: f32,
    #[serde(default = "default_text_y")]
    pub y: f32,
    #[serde(default = "yes")]
    pub shadow: bool,
    #[serde(default)]
    pub bold: bool,
}

fn default_text_size() -> f32 {
    0.08
}
fn white() -> [u8; 4] {
    [255, 255, 255, 255]
}
fn half() -> f32 {
    0.5
}
fn default_text_y() -> f32 {
    0.82
}

impl TextStyle {
    pub fn new(text: impl Into<String>) -> Self {
        TextStyle {
            text: text.into(),
            size: default_text_size(),
            color: white(),
            background: None,
            x: 0.5,
            y: default_text_y(),
            shadow: true,
            bold: false,
        }
    }
}

/// Primary colour correction, applied per clip.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorGrade {
    /// Exposure in stops, -3..3.
    pub exposure: f32,
    /// Additive lift, -1..1.
    pub brightness: f32,
    /// 0..2, 1 = neutral.
    pub contrast: f32,
    /// 0..2, 1 = neutral; 0 = black and white.
    pub saturation: f32,
    /// Midtone gamma, 0.2..3, 1 = neutral.
    pub gamma: f32,
    /// Warm (+) / cool (-), -1..1.
    pub temperature: f32,
    /// Magenta (+) / green (-), -1..1.
    pub tint: f32,
    /// Edge darkening, 0..1.
    pub vignette: f32,
}

impl Default for ColorGrade {
    fn default() -> Self {
        ColorGrade {
            exposure: 0.0,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            gamma: 1.0,
            temperature: 0.0,
            tint: 0.0,
            vignette: 0.0,
        }
    }
}

impl ColorGrade {
    pub fn is_identity(&self) -> bool {
        *self == ColorGrade::default()
    }
}

/// Spatial placement of a clip in the frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    /// 1 = fit to frame.
    pub scale: f32,
    /// Offset as a fraction of the frame size.
    pub x: f32,
    pub y: f32,
    /// Degrees, clockwise.
    pub rotation: f32,
    /// Crop fractions of the source, 0..0.5 each.
    pub crop_left: f32,
    pub crop_right: f32,
    pub crop_top: f32,
    pub crop_bottom: f32,
    /// Mirror horizontally.
    pub flip_h: bool,
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            scale: 1.0,
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            crop_left: 0.0,
            crop_right: 0.0,
            crop_top: 0.0,
            crop_bottom: 0.0,
            flip_h: false,
        }
    }
}

impl Transform {
    pub fn is_identity(&self) -> bool {
        *self == Transform::default()
    }
}

/// One clip on a track: a window `[source_in, source_in + duration)` of its
/// content, placed at `start` on the timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub content: ClipContent,
    /// Where the clip begins on the timeline.
    pub start: Time,
    /// In-point within the source media.
    #[serde(default)]
    pub source_in: Time,
    /// Length of the clip on the timeline (and within the source).
    pub duration: Time,
    #[serde(default)]
    pub name: String,
    /// Linked clips (the video and audio of one file) move and split together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<u64>,
    #[serde(default = "one")]
    pub volume: f32,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub fade_in: Time,
    #[serde(default)]
    pub fade_out: Time,
    #[serde(default)]
    pub grade: ColorGrade,
    #[serde(default)]
    pub transform: Transform,
}

impl Clip {
    pub fn new(id: ClipId, content: ClipContent, start: Time, source_in: Time, duration: Time) -> Self {
        Clip {
            id,
            content,
            start,
            source_in,
            duration,
            name: String::new(),
            link: None,
            volume: 1.0,
            opacity: 1.0,
            fade_in: Time::ZERO,
            fade_out: Time::ZERO,
            grade: ColorGrade::default(),
            transform: Transform::default(),
        }
    }

    pub fn end(&self) -> Time {
        self.start + self.duration
    }

    pub fn media(&self) -> Option<MediaId> {
        match self.content {
            ClipContent::Media { media } => Some(media),
            _ => None,
        }
    }

    pub fn timeline_range(&self) -> TimeRange {
        TimeRange::new(self.start, self.duration)
    }

    /// Map a timeline instant to the corresponding instant in the source media.
    /// Returns `None` if `t` is outside this clip.
    pub fn source_time_at(&self, t: Time) -> Option<Time> {
        if self.timeline_range().contains(t) {
            Some(self.source_in + (t - self.start))
        } else {
            None
        }
    }

    /// 0..1 envelope from the fades at timeline instant `t`.
    pub fn fade_gain(&self, t: Time) -> f32 {
        let local = (t - self.start).micros() as f32;
        let remain = (self.end() - t).micros() as f32;
        let mut g = 1.0f32;
        if self.fade_in.0 > 0 {
            g = g.min(local / self.fade_in.0 as f32);
        }
        if self.fade_out.0 > 0 {
            g = g.min(remain / self.fade_out.0 as f32);
        }
        g.clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
}

/// An ordered lane of clips. Clips on a track may not overlap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub name: String,
    pub clips: Vec<Clip>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default = "one")]
    pub volume: f32,
}

impl Track {
    pub fn new(id: TrackId, kind: TrackKind, name: impl Into<String>) -> Self {
        Track {
            id,
            kind,
            name: name.into(),
            clips: Vec::new(),
            muted: false,
            hidden: false,
            locked: false,
            volume: 1.0,
        }
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == id)
    }

    pub fn clip_mut(&mut self, id: ClipId) -> Option<&mut Clip> {
        self.clips.iter_mut().find(|c| c.id == id)
    }

    pub fn clip_index(&self, id: ClipId) -> Option<usize> {
        self.clips.iter().position(|c| c.id == id)
    }

    /// The clip playing at timeline instant `t`, if any.
    pub fn clip_at(&self, t: Time) -> Option<&Clip> {
        self.clips.iter().find(|c| c.timeline_range().contains(t))
    }

    /// Would a clip occupying `range` collide with an existing clip
    /// (optionally ignoring one, e.g. the clip being moved)?
    pub fn would_overlap(&self, range: TimeRange, ignore: Option<ClipId>) -> bool {
        self.clips
            .iter()
            .filter(|c| Some(c.id) != ignore)
            .any(|c| c.timeline_range().overlaps(range))
    }

    pub fn end(&self) -> Time {
        self.clips.iter().map(|c| c.end()).max().unwrap_or(Time::ZERO)
    }

    /// Keep clips sorted by start time — the invariant the UI and queries rely on.
    pub(crate) fn resort(&mut self) {
        self.clips.sort_by_key(|c| c.start);
    }

    /// True if the clips are sorted and none overlap.
    pub fn is_valid(&self) -> bool {
        self.clips.windows(2).all(|w| w[0].end() <= w[1].start)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub id: MarkerId,
    pub time: Time,
    #[serde(default)]
    pub label: String,
    #[serde(default = "marker_color")]
    pub color: [u8; 3],
}

fn marker_color() -> [u8; 3] {
    [255, 196, 0]
}

/// The timeline: a stack of tracks plus its playback format.
///
/// Video tracks composite in list order: a later video track sits on top.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub tracks: Vec<Track>,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default = "bg")]
    pub background: [u8; 3],
}

fn bg() -> [u8; 3] {
    [0, 0, 0]
}

impl Default for Timeline {
    fn default() -> Self {
        Timeline {
            tracks: Vec::new(),
            fps: 30.0,
            width: 1920,
            height: 1080,
            markers: Vec::new(),
            background: bg(),
        }
    }
}

impl Timeline {
    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn track_index(&self, id: TrackId) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == id)
    }

    /// The end of the last clip across all tracks — the project's total length.
    pub fn duration(&self) -> Time {
        self.tracks.iter().map(|t| t.end()).max().unwrap_or(Time::ZERO)
    }

    /// Find a clip anywhere on the timeline.
    pub fn find_clip(&self, id: ClipId) -> Option<(TrackId, &Clip)> {
        self.tracks
            .iter()
            .find_map(|t| t.clip(id).map(|c| (t.id, c)))
    }

    pub fn clips(&self) -> impl Iterator<Item = (&Track, &Clip)> {
        self.tracks.iter().flat_map(|t| t.clips.iter().map(move |c| (t, c)))
    }

    /// All clips sharing a link group with `id` (including itself).
    pub fn linked(&self, id: ClipId) -> Vec<(TrackId, ClipId)> {
        let Some((tr, c)) = self.find_clip(id) else {
            return vec![];
        };
        match c.link {
            None => vec![(tr, id)],
            Some(l) => self
                .clips()
                .filter(|(_, c)| c.link == Some(l))
                .map(|(t, c)| (t.id, c.id))
                .collect(),
        }
    }

    pub fn video_tracks(&self) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(|t| t.kind == TrackKind::Video)
    }

    pub fn audio_tracks(&self) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(|t| t.kind == TrackKind::Audio)
    }

    /// Every clip edge and marker: the points edits snap to.
    pub fn snap_points(&self) -> Vec<Time> {
        let mut v: Vec<Time> = self
            .clips()
            .flat_map(|(_, c)| [c.start, c.end()])
            .chain(self.markers.iter().map(|m| m.time))
            .collect();
        v.push(Time::ZERO);
        v.sort();
        v.dedup();
        v
    }
}

/// The whole editable document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    #[serde(default = "format_tag")]
    pub format: String,
    pub name: String,
    pub media: Vec<MediaItem>,
    pub timeline: Timeline,
    pub ids: IdGen,
}

fn format_tag() -> String {
    "kerf/1".into()
}

impl Default for Project {
    fn default() -> Self {
        let mut ids = IdGen::default();
        // A fresh project starts with two video and two audio tracks.
        let tracks = vec![
            Track::new(ids.track(), TrackKind::Video, "V1"),
            Track::new(ids.track(), TrackKind::Video, "V2"),
            Track::new(ids.track(), TrackKind::Audio, "A1"),
            Track::new(ids.track(), TrackKind::Audio, "A2"),
        ];
        Project {
            format: format_tag(),
            name: "Untitled".into(),
            media: vec![],
            timeline: Timeline {
                tracks,
                ..Timeline::default()
            },
            ids,
        }
    }
}

impl Project {
    pub fn media_item(&self, id: MediaId) -> Option<&MediaItem> {
        self.media.iter().find(|m| m.id == id)
    }

    pub fn first_track(&self, kind: TrackKind) -> Option<TrackId> {
        self.timeline
            .tracks
            .iter()
            .find(|t| t.kind == kind)
            .map(|t| t.id)
    }

    /// The first video track, the default edit target.
    pub fn primary_video_track(&self) -> Option<TrackId> {
        self.first_track(TrackKind::Video)
    }

    pub fn load(path: &std::path::Path) -> Result<Project, String> {
        let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&s).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Atomic save: write a temp file next to the target, then rename.
    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("kerf.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}
