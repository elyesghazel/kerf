//! The single mutation path for a project.
//!
//! Every edit is a [`Command`]. Applying one mutates the [`Project`] *and*
//! returns the [`Command`] that reverses it. That one property gives undo/redo
//! (see [`crate::document::Document`]), and because commands are `serde`
//! serializable they are also what agents send: a human and an agent edit
//! through exactly the same, auditable door.

use serde::{Deserialize, Serialize};

use crate::ids::{ClipId, MarkerId, MediaId, TrackId};
use crate::project::{Clip, Marker, MediaItem, Project, Track};
use crate::time::{Time, TimeRange};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    /// Import a source file into the media bin.
    AddMedia { item: MediaItem },
    /// Remove a media item. Fails if any clip still references it.
    RemoveMedia { id: MediaId },
    /// Replace a media item's metadata (e.g. after a proxy was generated).
    UpdateMedia { item: MediaItem },
    /// Place a clip on a track.
    AddClip { track: TrackId, clip: Clip },
    /// Remove a clip from a track.
    RemoveClip { track: TrackId, clip: ClipId },
    /// Slide a clip to a new start time, optionally onto another track.
    MoveClip {
        track: TrackId,
        clip: ClipId,
        new_start: Time,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_track: Option<TrackId>,
    },
    /// Set a clip's timeline start, source in-point and length (a trim).
    TrimClip {
        track: TrackId,
        clip: ClipId,
        start: Time,
        source_in: Time,
        duration: Time,
    },
    /// Replace every property of a clip (same id, same track).
    UpdateClip { track: TrackId, clip: Clip },
    /// Replace all clips on a track at once (ripple edits, range deletes).
    SetTrackClips { track: TrackId, clips: Vec<Clip> },
    /// Insert a track at `index` (clamped to the end).
    AddTrack { track: Track, index: usize },
    RemoveTrack { id: TrackId },
    /// Change a track's name, mute, visibility, lock and volume.
    UpdateTrack {
        id: TrackId,
        name: String,
        muted: bool,
        hidden: bool,
        locked: bool,
        volume: f32,
    },
    AddMarker { marker: Marker },
    RemoveMarker { id: MarkerId },
    /// Change the output format.
    SetFormat {
        width: u32,
        height: u32,
        fps: f64,
        background: [u8; 3],
    },
    Rename { name: String },
    /// Several commands as one undo step. All-or-nothing.
    Batch { commands: Vec<Command> },
    /// Swap in a whole new project (an external edit, e.g. from an agent).
    Replace { project: Box<Project> },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EditError {
    #[error("no track {0}")]
    NoTrack(TrackId),
    #[error("no clip {0}")]
    NoClip(ClipId),
    #[error("no media {0}")]
    NoMedia(MediaId),
    #[error("no marker {0}")]
    NoMarker(MarkerId),
    #[error("media {0} is still used by {1} clip(s)")]
    MediaInUse(MediaId, usize),
    #[error("clip would overlap another clip on the track")]
    Overlap,
    #[error("track {0} is locked")]
    Locked(TrackId),
    #[error("track kinds differ")]
    WrongTrackKind,
    #[error("invalid: {0}")]
    Bounds(&'static str),
    #[error("{0}")]
    Other(String),
}

fn track_mut(p: &mut Project, id: TrackId) -> Result<&mut Track, EditError> {
    p.timeline.track_mut(id).ok_or(EditError::NoTrack(id))
}

fn unlocked_mut(p: &mut Project, id: TrackId) -> Result<&mut Track, EditError> {
    let t = track_mut(p, id)?;
    if t.locked {
        return Err(EditError::Locked(id));
    }
    Ok(t)
}

impl Command {
    /// Apply this command to `project`, returning the inverse command.
    ///
    /// On error the project is left unchanged (commands validate before they
    /// mutate, and batches roll back), so a failed command never corrupts the
    /// document.
    pub fn apply(self, project: &mut Project) -> Result<Command, EditError> {
        match self {
            Command::AddMedia { item } => {
                let id = item.id;
                if project.media_item(id).is_some() {
                    return Err(EditError::Other(format!("media {id} exists")));
                }
                project.media.push(item);
                Ok(Command::RemoveMedia { id })
            }

            Command::RemoveMedia { id } => {
                let uses = project
                    .timeline
                    .clips()
                    .filter(|(_, c)| c.media() == Some(id))
                    .count();
                if uses > 0 {
                    return Err(EditError::MediaInUse(id, uses));
                }
                let idx = project
                    .media
                    .iter()
                    .position(|m| m.id == id)
                    .ok_or(EditError::NoMedia(id))?;
                let item = project.media.remove(idx);
                Ok(Command::AddMedia { item })
            }

            Command::UpdateMedia { item } => {
                let slot = project
                    .media
                    .iter_mut()
                    .find(|m| m.id == item.id)
                    .ok_or(EditError::NoMedia(item.id))?;
                let old = std::mem::replace(slot, item);
                Ok(Command::UpdateMedia { item: old })
            }

            Command::AddClip { track, clip } => {
                validate_clip(project, &clip)?;
                let t = unlocked_mut(project, track)?;
                if t.would_overlap(clip.timeline_range(), None) {
                    return Err(EditError::Overlap);
                }
                if t.clip(clip.id).is_some() {
                    return Err(EditError::Other(format!("clip {} exists", clip.id)));
                }
                let id = clip.id;
                t.clips.push(clip);
                t.resort();
                Ok(Command::RemoveClip { track, clip: id })
            }

            Command::RemoveClip { track, clip } => {
                let t = unlocked_mut(project, track)?;
                let idx = t.clip_index(clip).ok_or(EditError::NoClip(clip))?;
                let removed = t.clips.remove(idx);
                Ok(Command::AddClip {
                    track,
                    clip: removed,
                })
            }

            Command::MoveClip {
                track,
                clip,
                new_start,
                to_track,
            } => {
                if new_start < Time::ZERO {
                    return Err(EditError::Bounds("start before zero"));
                }
                let dest = to_track.unwrap_or(track);
                let src_kind = track_mut(project, track)?.kind;
                let dst = unlocked_mut(project, dest)?;
                if dst.kind != src_kind {
                    return Err(EditError::WrongTrackKind);
                }
                let src = unlocked_mut(project, track)?;
                let current = src.clip(clip).ok_or(EditError::NoClip(clip))?;
                let old_start = current.start;
                let range = TimeRange::new(new_start, current.duration);
                let ignore = if dest == track { Some(clip) } else { None };
                if track_mut(project, dest)?.would_overlap(range, ignore) {
                    return Err(EditError::Overlap);
                }
                let src = track_mut(project, track)?;
                let idx = src.clip_index(clip).unwrap();
                let mut c = src.clips.remove(idx);
                c.start = new_start;
                let dst = track_mut(project, dest)?;
                dst.clips.push(c);
                dst.resort();
                Ok(Command::MoveClip {
                    track: dest,
                    clip,
                    new_start: old_start,
                    to_track: if dest == track { None } else { Some(track) },
                })
            }

            Command::TrimClip {
                track,
                clip,
                start,
                source_in,
                duration,
            } => {
                let t = unlocked_mut(project, track)?;
                let mut c = t.clip(clip).ok_or(EditError::NoClip(clip))?.clone();
                c.start = start;
                c.source_in = source_in;
                c.duration = duration;
                validate_clip(project, &c)?;
                let t = track_mut(project, track)?;
                if t.would_overlap(c.timeline_range(), Some(clip)) {
                    return Err(EditError::Overlap);
                }
                let slot = t.clip_mut(clip).unwrap();
                let inverse = Command::TrimClip {
                    track,
                    clip,
                    start: slot.start,
                    source_in: slot.source_in,
                    duration: slot.duration,
                };
                *slot = c;
                t.resort();
                Ok(inverse)
            }

            Command::UpdateClip { track, clip } => {
                validate_clip(project, &clip)?;
                let t = unlocked_mut(project, track)?;
                if t.clip(clip.id).is_none() {
                    return Err(EditError::NoClip(clip.id));
                }
                if t.would_overlap(clip.timeline_range(), Some(clip.id)) {
                    return Err(EditError::Overlap);
                }
                let slot = t.clip_mut(clip.id).unwrap();
                let old = std::mem::replace(slot, clip);
                t.resort();
                Ok(Command::UpdateClip { track, clip: old })
            }

            Command::SetTrackClips { track, mut clips } => {
                for c in &clips {
                    validate_clip(project, c)?;
                }
                clips.sort_by_key(|c| c.start);
                if !clips.windows(2).all(|w| w[0].end() <= w[1].start) {
                    return Err(EditError::Overlap);
                }
                let t = unlocked_mut(project, track)?;
                let old = std::mem::replace(&mut t.clips, clips);
                Ok(Command::SetTrackClips { track, clips: old })
            }

            Command::AddTrack { track, index } => {
                if project.timeline.track(track.id).is_some() {
                    return Err(EditError::Other(format!("track {} exists", track.id)));
                }
                let id = track.id;
                let i = index.min(project.timeline.tracks.len());
                project.timeline.tracks.insert(i, track);
                Ok(Command::RemoveTrack { id })
            }

            Command::RemoveTrack { id } => {
                let i = project
                    .timeline
                    .track_index(id)
                    .ok_or(EditError::NoTrack(id))?;
                let track = project.timeline.tracks.remove(i);
                Ok(Command::AddTrack { track, index: i })
            }

            Command::UpdateTrack {
                id,
                name,
                muted,
                hidden,
                locked,
                volume,
            } => {
                let t = track_mut(project, id)?;
                let inv = Command::UpdateTrack {
                    id,
                    name: std::mem::replace(&mut t.name, name),
                    muted: std::mem::replace(&mut t.muted, muted),
                    hidden: std::mem::replace(&mut t.hidden, hidden),
                    locked: std::mem::replace(&mut t.locked, locked),
                    volume: std::mem::replace(&mut t.volume, volume.clamp(0.0, 4.0)),
                };
                Ok(inv)
            }

            Command::AddMarker { marker } => {
                let id = marker.id;
                project.timeline.markers.push(marker);
                project.timeline.markers.sort_by_key(|m| m.time);
                Ok(Command::RemoveMarker { id })
            }

            Command::RemoveMarker { id } => {
                let i = project
                    .timeline
                    .markers
                    .iter()
                    .position(|m| m.id == id)
                    .ok_or(EditError::NoMarker(id))?;
                let marker = project.timeline.markers.remove(i);
                Ok(Command::AddMarker { marker })
            }

            Command::SetFormat {
                width,
                height,
                fps,
                background,
            } => {
                if width < 16 || height < 16 || width > 8192 || height > 8192 {
                    return Err(EditError::Bounds("resolution out of range"));
                }
                if !(1.0..=240.0).contains(&fps) {
                    return Err(EditError::Bounds("fps out of range"));
                }
                let tl = &mut project.timeline;
                let inv = Command::SetFormat {
                    width: std::mem::replace(&mut tl.width, width & !1),
                    height: std::mem::replace(&mut tl.height, height & !1),
                    fps: std::mem::replace(&mut tl.fps, fps),
                    background: std::mem::replace(&mut tl.background, background),
                };
                Ok(inv)
            }

            Command::Rename { name } => Ok(Command::Rename {
                name: std::mem::replace(&mut project.name, name),
            }),

            Command::Batch { commands } => {
                let mut inverses = Vec::with_capacity(commands.len());
                for cmd in commands {
                    match cmd.apply(project) {
                        Ok(inv) => inverses.push(inv),
                        Err(e) => {
                            // Roll back what already went through.
                            for inv in inverses.into_iter().rev() {
                                let _ = inv.apply(project);
                            }
                            return Err(e);
                        }
                    }
                }
                inverses.reverse();
                Ok(Command::Batch { commands: inverses })
            }

            Command::Replace { project: new } => {
                let old = std::mem::replace(project, *new);
                Ok(Command::Replace {
                    project: Box::new(old),
                })
            }
        }
    }
}

fn validate_clip(project: &Project, clip: &Clip) -> Result<(), EditError> {
    if clip.duration <= Time::ZERO {
        return Err(EditError::Bounds("non-positive duration"));
    }
    if clip.start < Time::ZERO {
        return Err(EditError::Bounds("start before zero"));
    }
    if clip.source_in < Time::ZERO {
        return Err(EditError::Bounds("source_in before zero"));
    }
    if let Some(m) = clip.media() {
        let item = project.media_item(m).ok_or(EditError::NoMedia(m))?;
        // One frame of slack for container-reported durations.
        let slack = Time::from_secs_f64(1.0 / item.fps.max(1.0));
        if !item.is_image && clip.source_in + clip.duration > item.duration + slack {
            return Err(EditError::Bounds("clip runs past end of media"));
        }
    }
    Ok(())
}
