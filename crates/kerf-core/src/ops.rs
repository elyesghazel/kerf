//! High-level edits built out of primitive [`Command`]s.
//!
//! These are what the UI tools and the agent CLI call: "split here", "ripple
//! delete these", "cut 12.3s-15s out of everything", "keep only these ranges".
//! Each one lands as a single undo step.

use std::collections::HashMap;

use crate::command::{Command, EditError};
use crate::document::Document;
use crate::ids::{ClipId, MediaId, TrackId};
use crate::project::{Clip, ClipContent, Marker, MediaItem, TextStyle, Track, TrackKind};
use crate::time::{Time, TimeRange};

/// Which end of a clip a trim moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

impl Document {
    /// Import a media item, assigning it a fresh id.
    pub fn add_media(&mut self, mut item: MediaItem) -> Result<MediaId, EditError> {
        item.id = self.ids().media();
        let id = item.id;
        self.apply(Command::AddMedia { item })?;
        Ok(id)
    }

    /// Place a media item on the timeline at `at`: its video on a video track
    /// and its audio on a linked audio clip. Tracks are picked (or created) so
    /// nothing overlaps. Returns the new clip ids.
    pub fn place(
        &mut self,
        media: MediaId,
        at: Time,
        source_in: Time,
        duration: Option<Time>,
        video_track: Option<TrackId>,
        audio_track: Option<TrackId>,
    ) -> Result<Vec<ClipId>, EditError> {
        let item = self
            .project()
            .media_item(media)
            .ok_or(EditError::NoMedia(media))?
            .clone();
        let dur = duration.unwrap_or_else(|| {
            if item.is_image {
                Time::from_secs_f64(5.0)
            } else {
                item.duration - source_in
            }
        });
        let range = TimeRange::new(at, dur);
        let mut cmds = Vec::new();
        let mut ids = Vec::new();
        let link = if item.has_video && item.has_audio {
            Some(self.ids().link())
        } else {
            None
        };
        let name = item.name.clone();
        let mut make = |doc: &mut Document, kind: TrackKind, want: Option<TrackId>, cmds: &mut Vec<Command>| {
            let track = match want {
                Some(t) => t,
                None => doc.free_track(kind, range, cmds),
            };
            let mut c = Clip::new(doc.ids().clip(), ClipContent::Media { media }, at, source_in, dur);
            c.name = name.clone();
            c.link = link;
            ids.push(c.id);
            cmds.push(Command::AddClip { track, clip: c });
        };
        if item.has_video {
            make(self, TrackKind::Video, video_track, &mut cmds);
        }
        if item.has_audio {
            make(self, TrackKind::Audio, audio_track, &mut cmds);
        }
        self.apply_all(cmds)?;
        Ok(ids)
    }

    /// Append a media item after everything on the first video track.
    pub fn append(&mut self, media: MediaId) -> Result<Vec<ClipId>, EditError> {
        let p = self.project();
        let item = p.media_item(media).ok_or(EditError::NoMedia(media))?;
        let kind = if item.has_video { TrackKind::Video } else { TrackKind::Audio };
        let track = p.first_track(kind);
        let at = track
            .and_then(|t| p.timeline.track(t))
            .map(|t| t.end())
            .unwrap_or(Time::ZERO);
        // Audio goes on the first audio track if it is free there.
        let atrack = p.first_track(TrackKind::Audio).filter(|a| {
            !p.timeline
                .track(*a)
                .unwrap()
                .would_overlap(TimeRange::new(at, item.duration), None)
        });
        let (v, a) = if kind == TrackKind::Video { (track, atrack) } else { (None, track) };
        self.place(media, at, Time::ZERO, None, v, a)
    }

    /// The first unlocked track of `kind` that is free over `range`, or a new
    /// one (whose creation is pushed onto `cmds`).
    fn free_track(&mut self, kind: TrackKind, range: TimeRange, cmds: &mut Vec<Command>) -> TrackId {
        // Tracks created earlier in this same batch count as taken.
        let pending: Vec<TrackId> = cmds
            .iter()
            .filter_map(|c| match c {
                Command::AddClip { track, .. } => Some(*track),
                _ => None,
            })
            .collect();
        if let Some(t) = self
            .project()
            .timeline
            .tracks
            .iter()
            .filter(|t| t.kind == kind && !t.locked && !pending.contains(&t.id))
            .find(|t| !t.would_overlap(range, None))
        {
            return t.id;
        }
        let (id, idx, name) = self.new_track_slot(kind);
        cmds.push(Command::AddTrack {
            track: Track::new(id, kind, name),
            index: idx,
        });
        id
    }

    /// A fresh track id plus where a new track of `kind` goes and its name.
    pub fn new_track_slot(&mut self, kind: TrackKind) -> (TrackId, usize, String) {
        let tl = &self.project().timeline;
        let n = tl.tracks.iter().filter(|t| t.kind == kind).count() + 1;
        let idx = match kind {
            // New video tracks go on top of the existing video stack.
            TrackKind::Video => tl
                .tracks
                .iter()
                .rposition(|t| t.kind == TrackKind::Video)
                .map(|i| i + 1)
                .unwrap_or(0),
            TrackKind::Audio => tl.tracks.len(),
        };
        let prefix = if kind == TrackKind::Video { "V" } else { "A" };
        (self.ids().track(), idx, format!("{prefix}{n}"))
    }

    pub fn add_track(&mut self, kind: TrackKind) -> Result<TrackId, EditError> {
        let (id, index, name) = self.new_track_slot(kind);
        self.apply(Command::AddTrack {
            track: Track::new(id, kind, name),
            index,
        })?;
        Ok(id)
    }

    /// Split a clip (and its linked partners) at timeline instant `t`.
    /// Returns the ids of the new right-hand pieces.
    pub fn split(&mut self, clip: ClipId, t: Time) -> Result<Vec<ClipId>, EditError> {
        let targets = self.project().timeline.linked(clip);
        if targets.is_empty() {
            return Err(EditError::NoClip(clip));
        }
        let new_link = self
            .project()
            .timeline
            .find_clip(clip)
            .and_then(|(_, c)| c.link)
            .map(|_| self.ids().link());
        let mut cmds = Vec::new();
        let mut out = Vec::new();
        for (track, id) in targets {
            let c = self.project().timeline.track(track).unwrap().clip(id).unwrap().clone();
            if t <= c.start || t >= c.end() {
                continue;
            }
            let (left, mut right) = split_clip(&c, t, self.ids().clip());
            right.link = new_link;
            out.push(right.id);
            cmds.push(Command::UpdateClip { track, clip: left });
            cmds.push(Command::AddClip { track, clip: right });
        }
        if cmds.is_empty() {
            return Err(EditError::Bounds("split point is outside the clip"));
        }
        self.apply_all(cmds)?;
        Ok(out)
    }

    /// Blade every unlocked track at `t`.
    pub fn split_all(&mut self, t: Time) -> Result<usize, EditError> {
        let mut cmds = Vec::new();
        let mut links: HashMap<u64, u64> = HashMap::new();
        let hits: Vec<(TrackId, Clip)> = self
            .project()
            .timeline
            .tracks
            .iter()
            .filter(|tr| !tr.locked)
            .filter_map(|tr| tr.clip_at(t).filter(|c| c.start < t).map(|c| (tr.id, c.clone())))
            .collect();
        for (track, c) in hits {
            let (left, mut right) = split_clip(&c, t, self.ids().clip());
            if let Some(l) = c.link {
                if !links.contains_key(&l) {
                    let nl = self.ids().link();
                    links.insert(l, nl);
                }
                right.link = links.get(&l).copied();
            }
            cmds.push(Command::UpdateClip { track, clip: left });
            cmds.push(Command::AddClip { track, clip: right });
        }
        let n = cmds.len() / 2;
        self.apply_all(cmds)?;
        Ok(n)
    }

    /// Delete clips. With `ripple`, later clips on the same tracks slide left
    /// to close the hole.
    pub fn delete(&mut self, clips: &[ClipId], ripple: bool) -> Result<(), EditError> {
        let mut per_track: HashMap<TrackId, Vec<ClipId>> = HashMap::new();
        for &id in clips {
            let (t, _) = self
                .project()
                .timeline
                .find_clip(id)
                .ok_or(EditError::NoClip(id))?;
            per_track.entry(t).or_default().push(id);
        }
        let mut cmds = Vec::new();
        for (track, ids) in per_track {
            let tr = self.project().timeline.track(track).unwrap();
            if !ripple {
                for id in ids {
                    cmds.push(Command::RemoveClip { track, clip: id });
                }
                continue;
            }
            let removed: Vec<TimeRange> = tr
                .clips
                .iter()
                .filter(|c| ids.contains(&c.id))
                .map(|c| c.timeline_range())
                .collect();
            let clips = tr
                .clips
                .iter()
                .filter(|c| !ids.contains(&c.id))
                .map(|c| {
                    let shift: i64 = removed
                        .iter()
                        .filter(|r| r.end() <= c.start)
                        .map(|r| r.duration.0)
                        .sum();
                    let mut c = c.clone();
                    c.start = Time(c.start.0 - shift);
                    c
                })
                .collect();
            cmds.push(Command::SetTrackClips { track, clips });
        }
        self.apply_all(cmds)
    }

    /// Cut a timeline range out of every unlocked track. With `ripple` the
    /// rest of the timeline closes up (A/V stays in sync because every track
    /// shifts by the same amount).
    pub fn delete_range(&mut self, range: TimeRange, ripple: bool) -> Result<(), EditError> {
        if range.duration <= Time::ZERO {
            return Err(EditError::Bounds("empty range"));
        }
        let mut links: HashMap<u64, u64> = HashMap::new();
        let mut cmds = Vec::new();
        let tracks: Vec<Track> = self.project().timeline.tracks.clone();
        for tr in tracks.iter().filter(|t| !t.locked) {
            let clips = cut_track(&tr.clips, range, ripple, self.ids(), &mut links);
            if clips != tr.clips {
                cmds.push(Command::SetTrackClips { track: tr.id, clips });
            }
        }
        let markers: Vec<Marker> = self.project().timeline.markers.clone();
        for m in markers {
            if range.contains(m.time) {
                cmds.push(Command::RemoveMarker { id: m.id });
            } else if ripple && m.time >= range.end() {
                cmds.push(Command::RemoveMarker { id: m.id });
                let mut m2 = m.clone();
                m2.time = m.time - range.duration;
                cmds.push(Command::AddMarker { marker: m2 });
            }
        }
        self.apply_all(cmds)
    }

    /// Keep only `keep` (timeline ranges) and close everything else up. This
    /// is what silence removal and "make a highlight reel of these bits" use.
    pub fn keep_ranges(&mut self, keep: &[TimeRange]) -> Result<(), EditError> {
        let total = self.project().timeline.duration();
        let mut keep: Vec<TimeRange> = keep.iter().copied().filter(|r| r.duration > Time::ZERO).collect();
        keep.sort_by_key(|r| r.start);
        let mut drop = Vec::new();
        let mut cursor = Time::ZERO;
        for r in &keep {
            if r.start > cursor {
                drop.push(TimeRange::from_bounds(cursor, r.start));
            }
            cursor = cursor.max(r.end());
        }
        if cursor < total {
            drop.push(TimeRange::from_bounds(cursor, total));
        }
        self.delete_ranges(&drop)
    }

    /// Ripple-delete several ranges as one undo step.
    pub fn delete_ranges(&mut self, ranges: &[TimeRange]) -> Result<(), EditError> {
        let mut ranges: Vec<TimeRange> = ranges.iter().copied().filter(|r| r.duration > Time::ZERO).collect();
        // From the end backwards, so earlier ranges keep their coordinates.
        ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
        let mut scratch = Document::new(self.project().clone());
        for r in ranges {
            scratch.delete_range(r, true)?;
        }
        let new = scratch.into_project();
        self.apply(Command::Replace { project: Box::new(new) })
    }

    /// Remove empty stretches where no track has anything.
    pub fn close_gaps(&mut self) -> Result<usize, EditError> {
        let mut spans: Vec<TimeRange> = self.project().timeline.clips().map(|(_, c)| c.timeline_range()).collect();
        spans.sort_by_key(|r| r.start);
        let mut gaps = Vec::new();
        let mut cursor = Time::ZERO;
        for s in spans {
            if s.start > cursor {
                gaps.push(TimeRange::from_bounds(cursor, s.start));
            }
            cursor = cursor.max(s.end());
        }
        let n = gaps.len();
        if n > 0 {
            self.delete_ranges(&gaps)?;
        }
        Ok(n)
    }

    /// Move clips (with their linked partners) by `delta`; with `track_delta`
    /// they also hop that many tracks of the same kind (+ = up the stack).
    pub fn move_clips(&mut self, clips: &[ClipId], delta: Time, track_delta: i32) -> Result<(), EditError> {
        let mut all: Vec<(TrackId, ClipId)> = Vec::new();
        for &c in clips {
            if self.project().timeline.find_clip(c).is_none() {
                return Err(EditError::NoClip(c));
            }
            for l in self.project().timeline.linked(c) {
                if !all.contains(&l) {
                    all.push(l);
                }
            }
        }
        let tl = &self.project().timeline;
        // Pull everything off first, then put it back, so clips moving past
        // each other don't trip the overlap check.
        let mut removes = Vec::new();
        let mut adds = Vec::new();
        for (track, id) in &all {
            let c = tl.track(*track).unwrap().clip(*id).unwrap();
            let dest = if track_delta == 0 || !clips.contains(id) {
                *track
            } else {
                shift_track(tl, *track, track_delta).unwrap_or(*track)
            };
            let mut moved = c.clone();
            moved.start = Time((c.start.0 + delta.0).max(0));
            removes.push(Command::RemoveClip { track: *track, clip: *id });
            adds.push(Command::AddClip { track: dest, clip: moved });
        }
        removes.extend(adds);
        self.apply_all(removes)
    }

    /// Trim one edge of a clip (and its linked partners) to timeline time `to`.
    pub fn trim(&mut self, clip: ClipId, edge: Edge, to: Time) -> Result<(), EditError> {
        if self.project().timeline.find_clip(clip).is_none() {
            return Err(EditError::NoClip(clip));
        }
        let mut cmds = Vec::new();
        for (track, id) in self.project().timeline.linked(clip) {
            let c = self.project().timeline.track(track).unwrap().clip(id).unwrap();
            let (start, source_in, duration) = match edge {
                Edge::Start => {
                    let d = to - c.start;
                    (to, c.source_in + d, c.duration - d)
                }
                Edge::End => (c.start, c.source_in, to - c.start),
            };
            cmds.push(Command::TrimClip { track, clip: id, start, source_in, duration });
        }
        self.apply_all(cmds)
    }

    /// Change a clip's properties in one undo step.
    pub fn edit_clip(&mut self, clip: ClipId, f: impl FnOnce(&mut Clip)) -> Result<(), EditError> {
        let (track, c) = self.project().timeline.find_clip(clip).ok_or(EditError::NoClip(clip))?;
        let mut c = c.clone();
        f(&mut c);
        c.id = clip;
        self.apply(Command::UpdateClip { track, clip: c })
    }

    /// Add a title. Goes on the top video track that is free, or a new one.
    pub fn add_text(&mut self, style: TextStyle, start: Time, duration: Time) -> Result<ClipId, EditError> {
        let range = TimeRange::new(start, duration);
        let mut cmds = Vec::new();
        let tl = &self.project().timeline;
        let top_free = tl
            .tracks
            .iter()
            .rev()
            .filter(|t| t.kind == TrackKind::Video && !t.locked)
            .take_while(|t| !t.would_overlap(range, None))
            .last()
            .map(|t| t.id);
        let track = match top_free {
            Some(t) => t,
            None => {
                let (id, index, name) = self.new_track_slot(TrackKind::Video);
                cmds.push(Command::AddTrack { track: Track::new(id, TrackKind::Video, name), index });
                id
            }
        };
        let mut c = Clip::new(self.ids().clip(), ClipContent::Text(style.clone()), start, Time::ZERO, duration);
        c.name = style.text.lines().next().unwrap_or("Title").chars().take(32).collect();
        let id = c.id;
        cmds.push(Command::AddClip { track, clip: c });
        self.apply_all(cmds)?;
        Ok(id)
    }

    pub fn add_marker(&mut self, time: Time, label: impl Into<String>) -> Result<(), EditError> {
        let marker = Marker {
            id: self.ids().marker(),
            time,
            label: label.into(),
            color: [255, 196, 0],
        };
        self.apply(Command::AddMarker { marker })
    }
}

fn shift_track(tl: &crate::project::Timeline, from: TrackId, delta: i32) -> Option<TrackId> {
    let kind = tl.track(from)?.kind;
    let same: Vec<TrackId> = tl.tracks.iter().filter(|t| t.kind == kind).map(|t| t.id).collect();
    let i = same.iter().position(|t| *t == from)? as i32;
    // Video tracks stack upwards, audio tracks downwards in the UI.
    let j = i + delta;
    same.get(j.clamp(0, same.len() as i32 - 1) as usize).copied()
}

/// Split `c` at `t` (strictly inside it) into a left and right piece.
pub fn split_clip(c: &Clip, t: Time, right_id: ClipId) -> (Clip, Clip) {
    let mut left = c.clone();
    let mut right = c.clone();
    left.duration = t - c.start;
    left.fade_out = Time::ZERO;
    right.id = right_id;
    right.start = t;
    right.source_in = c.source_in + (t - c.start);
    right.duration = c.end() - t;
    right.fade_in = Time::ZERO;
    (left, right)
}

/// Remove `range` from one track's clip list.
fn cut_track(
    clips: &[Clip],
    range: TimeRange,
    ripple: bool,
    ids: &mut crate::ids::IdGen,
    links: &mut HashMap<u64, u64>,
) -> Vec<Clip> {
    let (rs, re, len) = (range.start, range.end(), range.duration);
    let mut out = Vec::new();
    for c in clips {
        let (cs, ce) = (c.start, c.end());
        if ce <= rs {
            out.push(c.clone());
        } else if cs >= re {
            let mut c = c.clone();
            if ripple {
                c.start = c.start - len;
            }
            out.push(c);
        } else if cs >= rs && ce <= re {
            // Entirely inside: gone.
        } else if cs < rs && ce > re {
            // Spans the range: keep both ends.
            let (left, mut right) = split_clip(c, re, ids.clip());
            let mut left = left;
            left.duration = rs - cs;
            if ripple {
                right.start = rs;
            }
            if let Some(l) = c.link {
                right.link = Some(*links.entry(l).or_insert_with(|| ids.link()));
            }
            out.push(left);
            out.push(right);
        } else if cs < rs {
            // Tail overlaps: trim the end.
            let mut c = c.clone();
            c.duration = rs - cs;
            c.fade_out = Time::ZERO;
            out.push(c);
        } else {
            // Head overlaps: trim the start.
            let mut c = c.clone();
            let d = re - cs;
            c.source_in += d;
            c.duration = c.duration - d;
            c.start = if ripple { rs } else { re };
            c.fade_in = Time::ZERO;
            out.push(c);
        }
    }
    out.sort_by_key(|c| c.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Project;

    fn doc_with_clip(secs: f64) -> (Document, MediaId, Vec<ClipId>) {
        let mut doc = Document::new(Project::default());
        let m = doc
            .add_media(MediaItem {
                id: MediaId(0),
                path: "x.mp4".into(),
                name: "x".into(),
                duration: Time::from_secs_f64(secs),
                width: 1920,
                height: 1080,
                fps: 30.0,
                has_video: true,
                has_audio: true,
                is_image: false,
                proxy: None,
            })
            .unwrap();
        let ids = doc.append(m).unwrap();
        (doc, m, ids)
    }

    #[test]
    fn place_links_video_and_audio() {
        let (doc, _, ids) = doc_with_clip(10.0);
        assert_eq!(ids.len(), 2);
        assert_eq!(doc.project().timeline.linked(ids[0]).len(), 2);
    }

    #[test]
    fn split_splits_both_linked() {
        let (mut doc, _, ids) = doc_with_clip(10.0);
        let right = doc.split(ids[0], Time::from_secs_f64(4.0)).unwrap();
        assert_eq!(right.len(), 2);
        let n: usize = doc.project().timeline.tracks.iter().map(|t| t.clips.len()).sum();
        assert_eq!(n, 4);
        // Right halves are linked to each other, not to the left halves.
        assert_eq!(doc.project().timeline.linked(right[0]).len(), 2);
        doc.undo().unwrap();
        let n: usize = doc.project().timeline.tracks.iter().map(|t| t.clips.len()).sum();
        assert_eq!(n, 2);
    }

    #[test]
    fn delete_range_ripples_everything() {
        let (mut doc, _, _) = doc_with_clip(10.0);
        doc.delete_range(TimeRange::from_bounds(Time::from_secs_f64(2.0), Time::from_secs_f64(5.0)), true)
            .unwrap();
        assert_eq!(doc.project().timeline.duration(), Time::from_secs_f64(7.0));
        for t in &doc.project().timeline.tracks {
            assert!(t.is_valid());
        }
        let v = doc.project().primary_video_track().unwrap();
        let clips = &doc.project().timeline.track(v).unwrap().clips;
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[1].start, Time::from_secs_f64(2.0));
        assert_eq!(clips[1].source_in, Time::from_secs_f64(5.0));
    }

    #[test]
    fn keep_ranges_builds_highlight() {
        let (mut doc, _, _) = doc_with_clip(20.0);
        doc.keep_ranges(&[
            TimeRange::from_bounds(Time::from_secs_f64(1.0), Time::from_secs_f64(3.0)),
            TimeRange::from_bounds(Time::from_secs_f64(10.0), Time::from_secs_f64(12.0)),
        ])
        .unwrap();
        assert_eq!(doc.project().timeline.duration(), Time::from_secs_f64(4.0));
        assert!(doc.undo().unwrap());
        assert_eq!(doc.project().timeline.duration(), Time::from_secs_f64(20.0));
    }

    #[test]
    fn ripple_delete_shifts_later_clips() {
        let (mut doc, m, a) = doc_with_clip(5.0);
        let b = doc.append(m).unwrap();
        doc.delete(&[a[0], a[1]], true).unwrap();
        let (_, c) = doc.project().timeline.find_clip(b[0]).unwrap();
        assert_eq!(c.start, Time::ZERO);
    }

    #[test]
    fn move_with_link() {
        let (mut doc, _, ids) = doc_with_clip(5.0);
        doc.move_clips(&[ids[0]], Time::from_secs_f64(2.0), 0).unwrap();
        for id in ids {
            assert_eq!(doc.project().timeline.find_clip(id).unwrap().1.start, Time::from_secs_f64(2.0));
        }
    }

    #[test]
    fn text_goes_on_top() {
        let (mut doc, _, _) = doc_with_clip(5.0);
        let id = doc.add_text(TextStyle::new("Hello"), Time::ZERO, Time::from_secs_f64(2.0)).unwrap();
        let (t, _) = doc.project().timeline.find_clip(id).unwrap();
        assert_eq!(doc.project().timeline.track(t).unwrap().name, "V2");
    }

    #[test]
    fn overlap_is_rejected_and_batch_rolls_back() {
        let (mut doc, m, _) = doc_with_clip(5.0);
        let v = doc.project().primary_video_track().unwrap();
        let before = doc.project().clone();
        let r = doc.place(m, Time::from_secs_f64(1.0), Time::ZERO, None, Some(v), None);
        assert_eq!(r.unwrap_err(), EditError::Overlap);
        assert_eq!(doc.project().timeline, before.timeline);
    }

    #[test]
    fn project_json_roundtrip() {
        let (doc, _, _) = doc_with_clip(5.0);
        let s = serde_json::to_string(doc.project()).unwrap();
        let back: Project = serde_json::from_str(&s).unwrap();
        assert_eq!(&back, doc.project());
    }
}
