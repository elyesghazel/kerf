//! `kerf-core`: the pure, UI- and media-free heart of the editor.
//!
//! It owns the [`Project`] document model and the one path that mutates it:
//! [`Command`], applied through a [`Document`] that turns each command's
//! inverse into undo/redo. The higher-level edits in [`ops`] (split, ripple
//! delete, keep-ranges, ...) are what both the UI and the agent CLI call.

pub mod command;
pub mod document;
pub mod ids;
pub mod ops;
pub mod project;
pub mod time;

pub use command::{Command, EditError};
pub use document::Document;
pub use ids::{ClipId, MarkerId, MediaId, TrackId};
pub use ops::Edge;
pub use project::{
    Clip, ClipContent, ColorGrade, Marker, MediaItem, Project, TextStyle, Timeline, Track, TrackKind, Transform,
};
pub use time::{Time, TimeRange};
