//! The timeline: ruler, track headers, clips with thumbnails and waveforms,
//! and all the direct-manipulation edits (move, trim, fades, blade, drop).

use egui::{pos2, vec2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind};
use kerf_core::{Clip, ClipContent, ClipId, Command, Edge, MediaId, Time, Track, TrackId, TrackKind};

use crate::app::{KerfApp, Tool, Workspace};
use crate::theme::{self, *};

const HEAD_W: f32 = 132.0;
const RULER_H: f32 = 28.0;
const VIDEO_H: f32 = 66.0;
const AUDIO_H: f32 = 52.0;
const EDGE_PX: f32 = 7.0;
const SNAP_PX: f32 = 9.0;

#[derive(Clone, Debug)]
enum Drag {
    None,
    Scrub,
    Pan { scroll: f64, x: f32 },
    Move {
        ids: Vec<ClipId>,
        grab: Time,
        delta: Time,
        origin_lane: usize,
        track_delta: i32,
        solo: bool,
        targets: Vec<Time>,
        edges: Vec<Time>,
        kind: TrackKind,
    },
    Trim {
        clip: ClipId,
        edge: Edge,
        to: Time,
        lo: Time,
        hi: Time,
        targets: Vec<Time>,
    },
    Fade { clip: ClipId, out: bool, len: Time },
}

pub struct TimelineState {
    pub pps: f32,
    pub scroll: f64,
    pub vscroll: f32,
    pub follow: bool,
    pub fit_pending: bool,
    drag: Drag,
    ctx_clip: Option<ClipId>,
    /// Last laid-out geometry: x of time zero is `left - scroll*pps`.
    pub left: f32,
    pub lanes: Vec<(TrackId, Rect)>,
    pub ruler_y: f32,
}

impl Default for TimelineState {
    fn default() -> Self {
        TimelineState {
            pps: 60.0,
            scroll: 0.0,
            vscroll: 0.0,
            follow: true,
            fit_pending: true,
            drag: Drag::None,
            ctx_clip: None,
            left: 0.0,
            lanes: vec![],
            ruler_y: 0.0,
        }
    }
}

impl TimelineState {
    /// Screen x of timeline time `t` (as of the last frame).
    pub fn x_of(&self, t: Time) -> f32 {
        self.left + ((t.as_secs_f64() - self.scroll) as f32) * self.pps
    }

    pub fn lane_y(&self, track: TrackId) -> Option<f32> {
        self.lanes.iter().find(|(t, _)| *t == track).map(|(_, r)| r.center().y + 6.0)
    }

    pub fn zoom_by(&mut self, f: f32) {
        self.pps = (self.pps * f).clamp(1.0, 4000.0);
    }
}

struct Lane {
    track: TrackId,
    kind: TrackKind,
    rect: Rect,
}

fn lanes_for(app: &KerfApp, area: Rect, vscroll: f32) -> Vec<Lane> {
    let tl = &app.project().timeline;
    let mut order: Vec<&Track> = tl.tracks.iter().filter(|t| t.kind == TrackKind::Video).collect();
    order.reverse();
    order.extend(tl.tracks.iter().filter(|t| t.kind == TrackKind::Audio));
    let mut y = area.top() - vscroll;
    order
        .into_iter()
        .map(|t| {
            let h = if t.kind == TrackKind::Video { VIDEO_H } else { AUDIO_H };
            let r = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), h));
            y += h + 2.0;
            Lane { track: t.id, kind: t.kind, rect: r }
        })
        .collect()
}

pub fn show(app: &mut KerfApp, ui: &mut egui::Ui) {
    toolbar(app, ui);
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let head = Rect::from_min_max(full.min, pos2(full.left() + HEAD_W, full.bottom()));
    let lanes_area = Rect::from_min_max(pos2(head.right(), full.top()), full.max);
    let ruler = Rect::from_min_max(lanes_area.min, pos2(lanes_area.right(), lanes_area.top() + RULER_H));
    let body = Rect::from_min_max(pos2(lanes_area.left(), ruler.bottom()), lanes_area.max);
    let left = lanes_area.left();

    let duration = app.project().timeline.duration();
    if app.tl.fit_pending && lanes_area.width() > 50.0 {
        app.tl.fit_pending = false;
        let secs = duration.as_secs_f64().max(10.0);
        app.tl.pps = ((lanes_area.width() - 60.0) as f64 / secs) as f32;
        app.tl.scroll = 0.0;
    }

    // Track stack height and vertical scroll.
    let total_h: f32 = app
        .project()
        .timeline
        .tracks
        .iter()
        .map(|t| if t.kind == TrackKind::Video { VIDEO_H } else { AUDIO_H } + 2.0)
        .sum();
    let max_v = (total_h - body.height() + 8.0).max(0.0);
    app.tl.vscroll = app.tl.vscroll.clamp(0.0, max_v);

    let resp = ui.interact(lanes_area, ui.scope_id().with("timeline"), Sense::click_and_drag());

    // ---- scroll & zoom
    if resp.hovered() || resp.dragged() {
        let (scroll, zoom, ptr) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
        if zoom != 1.0 {
            let px = ptr.map(|p| p.x).unwrap_or(left);
            let anchor = app.tl.scroll + ((px - left) / app.tl.pps) as f64;
            app.tl.zoom_by(zoom);
            app.tl.scroll = anchor - ((px - left) / app.tl.pps) as f64;
        }
        if scroll.x != 0.0 {
            app.tl.scroll -= (scroll.x / app.tl.pps) as f64;
        }
        if scroll.y != 0.0 {
            if max_v > 0.0 && body.contains(ptr.unwrap_or(Pos2::ZERO)) {
                app.tl.vscroll = (app.tl.vscroll - scroll.y).clamp(0.0, max_v);
            } else {
                app.tl.scroll -= (scroll.y / app.tl.pps) as f64;
            }
        }
    }
    // Follow the playhead while playing.
    let visible = (lanes_area.width() / app.tl.pps) as f64;
    if app.tl.follow && app.engine.is_playing() {
        let ph = app.playhead.as_secs_f64();
        if ph > app.tl.scroll + visible * 0.92 || ph < app.tl.scroll {
            app.tl.scroll = ph - visible * 0.1;
        }
    }
    app.tl.scroll = app.tl.scroll.max(-(30.0 / app.tl.pps) as f64);

    let pps = app.tl.pps;
    let scroll = app.tl.scroll;
    let x_of = |t: Time| left + ((t.as_secs_f64() - scroll) as f32) * pps;
    let t_of = |x: f32| Time::from_secs_f64(scroll + ((x - left) / pps) as f64);
    let fps = app.project().timeline.fps;
    let lanes = lanes_for(app, body, app.tl.vscroll);
    app.tl.left = left;
    app.tl.lanes = lanes.iter().map(|l| (l.track, l.rect)).collect();
    app.tl.ruler_y = ruler.center().y;
    let lane_at = |p: Pos2| lanes.iter().position(|l| l.rect.contains(p) && body.contains(p));

    let painter = ui.painter_at(full);
    painter.rect_filled(full, 0, PANEL);
    for (i, l) in lanes.iter().enumerate() {
        let r = l.rect.intersect(body);
        if r.height() <= 0.0 {
            continue;
        }
        let shade = if i % 2 == 0 { Color32::from_rgb(10, 13, 19) } else { Color32::from_rgb(12, 15, 22) };
        painter.rect_filled(r, 0, shade);
    }

    // In/out region.
    if let (Some(a), Some(b)) = (app.mark_in, app.mark_out) {
        if b > a {
            let r = Rect::from_x_y_ranges(x_of(a)..=x_of(b), body.y_range());
            painter.rect_filled(r.intersect(body), 0, theme::alpha(VIOLET, 0.05));
        }
    }

    // ---- hit testing
    let ptr = ui.input(|i| i.pointer.hover_pos());
    let fd = Time::from_secs_f64(1.0 / fps);
    #[derive(Clone, Copy, PartialEq)]
    enum Hit {
        Body,
        Edge(Edge),
        Fade(bool),
    }
    let hit_test = |app: &KerfApp, p: Pos2| -> Option<(ClipId, Hit)> {
        let li = lane_at(p)?;
        let lane = &lanes[li];
        let track = app.project().timeline.track(lane.track)?;
        let t = t_of(p.x);
        let mut best = None;
        for c in &track.clips {
            let (x0, x1) = (x_of(c.start), x_of(c.end()));
            if p.x < x0 - EDGE_PX || p.x > x1 + EDGE_PX {
                continue;
            }
            let w = x1 - x0;
            // Fade handles sit in the top corners.
            if p.y < lane.rect.top() + 12.0 && w > 30.0 {
                let fi = x_of(c.start + c.fade_in);
                let fo = x_of(c.end() - c.fade_out);
                if (p.x - fi).abs() < 7.0 {
                    return Some((c.id, Hit::Fade(false)));
                }
                if (p.x - fo).abs() < 7.0 {
                    return Some((c.id, Hit::Fade(true)));
                }
            }
            if w > 14.0 && (p.x - x0).abs() <= EDGE_PX {
                best = Some((c.id, Hit::Edge(Edge::Start)));
            } else if w > 14.0 && (p.x - x1).abs() <= EDGE_PX {
                best = Some((c.id, Hit::Edge(Edge::End)));
            } else if t >= c.start && t < c.end() && best.is_none() {
                best = Some((c.id, Hit::Body));
            }
        }
        best
    };
    let snap_targets = |exclude: &[ClipId], app: &KerfApp| -> Vec<Time> {
        let tl = &app.project().timeline;
        let mut v: Vec<Time> = tl
            .clips()
            .filter(|(_, c)| !exclude.contains(&c.id))
            .flat_map(|(_, c)| [c.start, c.end()])
            .chain(tl.markers.iter().map(|m| m.time))
            .chain([Time::ZERO, app.playhead])
            .chain(app.mark_in)
            .chain(app.mark_out)
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let snap = |t: Time, targets: &[Time], on: bool| -> (Time, Option<Time>) {
        if !on {
            return (t, None);
        }
        let mut best: Option<(f32, Time)> = None;
        for &s in targets {
            let d = ((s.as_secs_f64() - t.as_secs_f64()) as f32 * pps).abs();
            if d < SNAP_PX && best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, s));
            }
        }
        match best {
            Some((_, s)) => (s, Some(s)),
            None => (t, None),
        }
    };

    // Cursor feedback.
    if let Some(p) = ptr {
        if lanes_area.contains(p) {
            match (&app.tl.drag, app.tool) {
                (Drag::None, Tool::Blade) if body.contains(p) => ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair),
                (Drag::None, _) => match hit_test(app, p) {
                    Some((_, Hit::Edge(_))) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
                    Some((_, Hit::Fade(_))) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn),
                    Some((_, Hit::Body)) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
                    None => {}
                },
                (Drag::Move { .. }, _) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing),
                (Drag::Trim { .. }, _) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
                _ => {}
            }
        }
    }

    // ---- drag start
    let (origin, middle, alt, shift) = ui.input(|i| {
        (i.pointer.press_origin(), i.pointer.middle_down(), i.modifiers.alt, i.modifiers.shift)
    });
    let snapping = app.snapping;
    if resp.drag_started() {
        if let Some(o) = origin {
            app.tl.drag = if middle {
                Drag::Pan { scroll: app.tl.scroll, x: o.x }
            } else if ruler.contains(o) || app.tool == Tool::Blade {
                Drag::Scrub
            } else {
                match hit_test(app, o) {
                    Some((id, Hit::Edge(edge))) => {
                        let (tid, c) = app.project().timeline.find_clip(id).unwrap();
                        let c = c.clone();
                        let track = app.project().timeline.track(tid).unwrap();
                        let prev_end = track.clips.iter().filter(|x| x.end() <= c.start).map(|x| x.end()).max().unwrap_or(Time::ZERO);
                        let next_start = track.clips.iter().filter(|x| x.start >= c.end()).map(|x| x.start).min().unwrap_or(Time(i64::MAX / 4));
                        let media_len = c
                            .media()
                            .and_then(|m| app.project().media_item(m))
                            .filter(|m| !m.is_image)
                            .map(|m| m.duration);
                        let (lo, hi, to) = match edge {
                            Edge::Start => {
                                let lo = match media_len {
                                    Some(_) => prev_end.max(c.start - c.source_in),
                                    None => prev_end,
                                };
                                (lo.max(Time::ZERO), c.end() - fd, c.start)
                            }
                            Edge::End => {
                                let hi = match media_len {
                                    Some(len) => next_start.min(c.start + (len - c.source_in)),
                                    None => next_start,
                                };
                                (c.start + fd, hi, c.end())
                            }
                        };
                        Drag::Trim { clip: id, edge, to, lo, hi, targets: snap_targets(&[id], app) }
                    }
                    Some((id, Hit::Fade(out))) => {
                        let c = app.project().timeline.find_clip(id).unwrap().1;
                        Drag::Fade { clip: id, out, len: if out { c.fade_out } else { c.fade_in } }
                    }
                    Some((id, Hit::Body)) => {
                        if !app.selection.contains(&id) {
                            if alt {
                                app.selection = vec![id];
                            } else {
                                app.select_with_links(id, shift);
                            }
                        }
                        let ids: Vec<ClipId> = if alt { vec![id] } else { app.selection.clone() };
                        let mut all = ids.clone();
                        if !alt {
                            for i in &ids {
                                for (_, l) in app.project().timeline.linked(*i) {
                                    if !all.contains(&l) {
                                        all.push(l);
                                    }
                                }
                            }
                        }
                        let edges = all
                            .iter()
                            .filter_map(|i| app.project().timeline.find_clip(*i))
                            .flat_map(|(_, c)| [c.start, c.end()])
                            .collect();
                        Drag::Move {
                            targets: snap_targets(&all, app),
                            ids: all,
                            grab: t_of(o.x),
                            delta: Time::ZERO,
                            origin_lane: lane_at(o).unwrap_or(0),
                            track_delta: 0,
                            solo: alt,
                            edges,
                            kind: lane_at(o).map(|i| lanes[i].kind).unwrap_or(TrackKind::Video),
                        }
                    }
                    None => Drag::Scrub,
                }
            };
        }
    }

    // ---- dragging
    let mut snap_line: Option<Time> = None;
    if resp.dragged() {
        if let Some(p) = resp.interact_pointer_pos() {
            let t = t_of(p.x);
            let playhead = app.playhead;
            match &mut app.tl.drag {
                Drag::Scrub => {
                    let targets = snap_targets(&[], app);
                    let (t, _) = snap(t, &targets, snapping);
                    app.seek(t);
                }
                Drag::Pan { scroll, x } => {
                    app.tl.scroll = *scroll - ((p.x - *x) / pps) as f64;
                }
                Drag::Move { grab, delta, origin_lane, track_delta, targets, edges, ids, .. } => {
                    let mut d = t - *grab;
                    let min_start = ids
                        .iter()
                        .filter_map(|i| app.doc.project().timeline.find_clip(*i).map(|(_, c)| c.start))
                        .min()
                        .unwrap_or(Time::ZERO);
                    if min_start + d < Time::ZERO {
                        d = -min_start;
                    }
                    if snapping {
                        let mut best: Option<(f32, Time, Time)> = None;
                        for e in edges.iter() {
                            let moved = *e + d;
                            for &s in targets.iter() {
                                let dist = ((s - moved).as_secs_f64() as f32 * pps).abs();
                                if dist < SNAP_PX && best.map(|b| dist < b.0).unwrap_or(true) {
                                    best = Some((dist, s - *e, s));
                                }
                            }
                        }
                        if let Some((_, nd, s)) = best {
                            d = nd;
                            snap_line = Some(s);
                        }
                    }
                    *delta = d;
                    if let Some(li) = lane_at(p) {
                        let (ok, ck) = (lanes[*origin_lane].kind, lanes[li].kind);
                        if ok == ck {
                            *track_delta = if ok == TrackKind::Video {
                                *origin_lane as i32 - li as i32
                            } else {
                                li as i32 - *origin_lane as i32
                            };
                        }
                    }
                }
                Drag::Trim { to, lo, hi, targets, .. } => {
                    let (s, line) = snap(t, targets, snapping);
                    *to = s.max(*lo).min(*hi).round_to_frame(fps);
                    snap_line = line;
                }
                Drag::Fade { clip, out, len } => {
                    if let Some((_, c)) = app.doc.project().timeline.find_clip(*clip) {
                        let l = if *out { c.end() - t } else { t - c.start };
                        *len = l.max(Time::ZERO).min(Time(c.duration.0 / 2)).round_to_frame(fps);
                    }
                }
                Drag::None => {}
            }
            let _ = playhead;
        }
    }

    // ---- drag end: commit as one undo step
    if resp.drag_stopped() {
        let drag = std::mem::replace(&mut app.tl.drag, Drag::None);
        match drag {
            Drag::Move { ids, delta, track_delta, solo, kind, .. } if delta != Time::ZERO || track_delta != 0 => {
                if solo {
                    let mut cmds = vec![];
                    let mut adds = vec![];
                    for id in &ids {
                        if let Some((tr, c)) = app.project().timeline.find_clip(*id) {
                            let mut c2 = c.clone();
                            c2.start = Time((c.start.0 + delta.0).max(0));
                            let dest = shift_track(app, tr, track_delta);
                            cmds.push(Command::RemoveClip { track: tr, clip: *id });
                            adds.push(Command::AddClip { track: dest, clip: c2 });
                        }
                    }
                    cmds.extend(adds);
                    app.edit(|d| d.apply_all(cmds));
                } else {
                    // Only clips of the dragged kind hop tracks; linked
                    // partners on the other kind just follow in time.
                    let primary: Vec<ClipId> = ids
                        .iter()
                        .copied()
                        .filter(|i| app.selection.contains(i) && clip_kind(app, *i) == Some(kind))
                        .collect();
                    let primary = if primary.is_empty() { ids.clone() } else { primary };
                    app.edit(|d| move_selected(d, &primary, delta, track_delta));
                }
            }
            Drag::Trim { clip, edge, to, .. } => {
                app.edit(|d| d.trim(clip, edge, to));
            }
            Drag::Fade { clip, out, len } => {
                app.edit(|d| {
                    d.edit_clip(clip, |c| {
                        if out {
                            c.fade_out = len
                        } else {
                            c.fade_in = len
                        }
                    })
                });
            }
            _ => {}
        }
    }

    // ---- clicks
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let t = t_of(p.x);
            if ruler.contains(p) {
                // Clicking a marker jumps exactly to it.
                let mk = app
                    .project()
                    .timeline
                    .markers
                    .iter()
                    .find(|m| (x_of(m.time) - p.x).abs() < 6.0)
                    .map(|m| m.time);
                app.seek(mk.unwrap_or(t));
            } else if app.tool == Tool::Blade {
                if let Some((id, _)) = hit_test(app, p) {
                    let near_ph = ((x_of(app.playhead) - p.x).abs()) < SNAP_PX;
                    let at = if near_ph { app.playhead } else { t.round_to_frame(fps) };
                    app.edit(|d| d.split(id, at));
                }
            } else {
                match hit_test(app, p) {
                    Some((id, _)) => {
                        if alt {
                            app.selection = vec![id];
                        } else {
                            app.select_with_links(id, shift);
                        }
                    }
                    None => {
                        app.selection.clear();
                        app.seek(t);
                    }
                }
            }
        }
    }
    if resp.double_clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            if let Some((id, _)) = hit_test(app, p) {
                app.select_with_links(id, false);
                if app.workspace == Workspace::Deliver {
                    app.workspace = Workspace::Edit;
                }
            }
        }
    }
    if resp.secondary_clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            app.tl.ctx_clip = hit_test(app, p).map(|(id, _)| id);
            if let Some(id) = app.tl.ctx_clip {
                if !app.selection.contains(&id) {
                    app.select_with_links(id, false);
                }
            }
        }
    }
    resp.context_menu(|ui| clip_menu(app, ui));

    // ---- drops from the media pool
    let mut drop_ghost: Option<(usize, Time, Time)> = None;
    if let Some(m) = resp.dnd_hover_payload::<MediaId>() {
        if let (Some(p), Some(item)) = (ptr, app.project().media_item(*m)) {
            let dur = if item.is_image { Time::from_secs_f64(5.0) } else { item.duration };
            let targets = snap_targets(&[], app);
            let (t, line) = snap(t_of(p.x).max(Time::ZERO), &targets, snapping);
            snap_line = line;
            if let Some(li) = lane_at(p) {
                drop_ghost = Some((li, t, dur));
            }
        }
    }
    if let Some(m) = resp.dnd_release_payload::<MediaId>() {
        if let Some(p) = ptr {
            let targets = snap_targets(&[], app);
            let (t, _) = snap(t_of(p.x).max(Time::ZERO), &targets, snapping);
            let (vt, at) = match lane_at(p).map(|i| (lanes[i].kind, lanes[i].track)) {
                Some((TrackKind::Video, tr)) => (Some(tr), None),
                Some((TrackKind::Audio, tr)) => (None, Some(tr)),
                None => (None, None),
            };
            let m = *m;
            if let Some(ids) = app.edit(|d| d.place(m, t, Time::ZERO, None, vt, at)) {
                app.selection = ids;
            }
        }
    }

    // ---- paint clips
    let clip_rect = |c: &Clip, lane: &Lane| -> Rect {
        Rect::from_x_y_ranges(x_of(c.start)..=x_of(c.end()), (lane.rect.top() + 3.0)..=(lane.rect.bottom() - 3.0))
    };
    let body_painter = ui.painter_at(body);
    let drag = app.tl.drag.clone();
    let moving: Vec<ClipId> = match &drag {
        Drag::Move { ids, .. } => ids.clone(),
        _ => vec![],
    };
    let ctx = ui.ctx().clone();
    for (li, lane) in lanes.iter().enumerate() {
        if lane.rect.bottom() < body.top() || lane.rect.top() > body.bottom() {
            continue;
        }
        let track = app.project().timeline.track(lane.track).unwrap().clone();
        for c in &track.clips {
            let mut r = clip_rect(c, lane);
            if r.right() < body.left() || r.left() > body.right() {
                continue;
            }
            let mut ghost = false;
            let mut view = c.clone();
            if let Drag::Trim { clip, edge, to, .. } = &drag {
                // Linked partners preview the trim too.
                let linked: Vec<ClipId> = app.project().timeline.linked(*clip).into_iter().map(|x| x.1).collect();
                if linked.contains(&c.id) {
                    match edge {
                        Edge::Start => {
                            let d = *to - c.start;
                            view.start = *to;
                            view.source_in += d;
                            view.duration = view.duration - d;
                        }
                        Edge::End => view.duration = *to - c.start,
                    }
                    r = clip_rect(&view, lane);
                }
            }
            if let Drag::Fade { clip, out, len } = &drag {
                if *clip == c.id {
                    if *out {
                        view.fade_out = *len
                    } else {
                        view.fade_in = *len
                    }
                }
            }
            if moving.contains(&c.id) {
                ghost = true;
            }
            let sel = app.selection.contains(&c.id);
            draw_clip(app, &ctx, &body_painter, r, &view, &track, sel, if ghost { 0.35 } else { 1.0 }, pps, body);
        }
        // Ghosts of clips being moved, drawn on their destination lane.
        if let Drag::Move { ids, delta, track_delta, kind, .. } = &drag {
            for (tid, c) in app.project().timeline.clips().map(|(t, c)| (t.id, c.clone())).collect::<Vec<_>>() {
                if !ids.contains(&c.id) {
                    continue;
                }
                let same_kind = clip_kind(app, c.id) == Some(*kind);
                let dest = if same_kind && (app.selection.contains(&c.id) || matches!(drag, Drag::Move { solo: true, .. })) {
                    shift_track(app, tid, *track_delta)
                } else {
                    tid
                };
                if dest != lane.track {
                    continue;
                }
                let mut v = c.clone();
                v.start = Time((c.start.0 + delta.0).max(0));
                let r = clip_rect(&v, lane);
                let tr = app.project().timeline.track(tid).unwrap().clone();
                draw_clip(app, &ctx, &body_painter, r, &v, &tr, true, 0.9, pps, body);
            }
        }
        if let Some((gl, t, dur)) = drop_ghost {
            if gl == li {
                let r = Rect::from_x_y_ranges(x_of(t)..=x_of(t + dur), (lane.rect.top() + 3.0)..=(lane.rect.bottom() - 3.0));
                body_painter.rect_filled(r, 6, theme::alpha(CYAN, 0.12));
                theme::glow_rect(&body_painter, r, 6, CYAN);
            }
        }
    }

    // ---- ruler
    painter.rect_filled(ruler, 0, BG);
    painter.line_segment([ruler.left_bottom(), ruler.right_bottom()], Stroke::new(1.0, LINE));
    let step = nice_step(pps, fps);
    let first = (scroll / step).floor() as i64;
    let last = ((scroll + visible) / step).ceil() as i64;
    for i in first..=last {
        let t = i as f64 * step;
        if t < 0.0 {
            continue;
        }
        let x = left + ((t - scroll) as f32) * pps;
        painter.line_segment([pos2(x, ruler.bottom() - 9.0), pos2(x, ruler.bottom())], Stroke::new(1.0, TEXT_FAINT));
        painter.text(
            pos2(x + 4.0, ruler.top() + 4.0),
            egui::Align2::LEFT_TOP,
            short_tc(t, fps, step),
            theme::mono(10.0),
            TEXT_DIM,
        );
        for k in 1..5 {
            let xm = x + (k as f32) * (step as f32) * pps / 5.0;
            painter.line_segment([pos2(xm, ruler.bottom() - 4.0), pos2(xm, ruler.bottom())], Stroke::new(1.0, LINE));
        }
    }
    if let (Some(a), Some(b)) = (app.mark_in, app.mark_out) {
        if b > a {
            let r = Rect::from_x_y_ranges(x_of(a)..=x_of(b), (ruler.bottom() - 5.0)..=ruler.bottom());
            painter.rect_filled(r.intersect(ruler), 0, theme::alpha(VIOLET, 0.8));
        }
    }
    for t in [app.mark_in, app.mark_out].into_iter().flatten() {
        let x = x_of(t);
        painter.line_segment([pos2(x, ruler.top()), pos2(x, ruler.bottom())], Stroke::new(1.5, VIOLET));
    }
    for m in app.project().timeline.markers.clone() {
        let x = x_of(m.time);
        if x < ruler.left() || x > ruler.right() {
            continue;
        }
        let c = Color32::from_rgb(m.color[0], m.color[1], m.color[2]);
        let y = ruler.bottom() - 8.0;
        painter.add(egui::Shape::convex_polygon(
            vec![pos2(x, y - 6.0), pos2(x + 5.0, y), pos2(x, y + 6.0), pos2(x - 5.0, y)],
            c,
            Stroke::NONE,
        ));
        body_painter.line_segment([pos2(x, body.top()), pos2(x, body.bottom())], Stroke::new(1.0, theme::alpha(c, 0.25)));
        if let Some(p) = ptr {
            if (p.x - x).abs() < 6.0 && ruler.contains(p) {
                resp.clone().on_hover_text(format!("{}  {}", m.label, m.time.timecode(fps)));
            }
        }
    }

    // ---- snap guide, blade hover, playhead
    if let Some(s) = snap_line {
        let x = x_of(s);
        painter.line_segment([pos2(x, ruler.top()), pos2(x, body.bottom())], Stroke::new(1.0, theme::alpha(AMBER, 0.9)));
    }
    if app.tool == Tool::Blade {
        if let Some(p) = ptr {
            if let Some(li) = lane_at(p) {
                let near_ph = ((x_of(app.playhead) - p.x).abs()) < SNAP_PX;
                let x = if near_ph { x_of(app.playhead) } else { p.x };
                let lr = lanes[li].rect;
                theme::glow_line(&body_painter, pos2(x, lr.top()), pos2(x, lr.bottom()), MAGENTA, 1.5);
            }
        }
    }
    let px = x_of(app.playhead);
    if px >= lanes_area.left() - 1.0 && px <= lanes_area.right() + 1.0 {
        theme::glow_line(&painter, pos2(px, ruler.top() + 12.0), pos2(px, body.bottom()), CYAN, 1.5);
        let top = ruler.top() + 2.0;
        painter.add(egui::Shape::convex_polygon(
            vec![
                pos2(px - 7.0, top),
                pos2(px + 7.0, top),
                pos2(px + 7.0, top + 10.0),
                pos2(px, top + 17.0),
                pos2(px - 7.0, top + 10.0),
            ],
            CYAN,
            Stroke::NONE,
        ));
        painter.circle_filled(pos2(px, top + 17.0), 8.0, theme::alpha(CYAN, 0.08));
    }

    // ---- track headers (drawn last so they cover clips scrolled under them)
    headers(app, ui, head, body, &lanes);
}

fn clip_kind(app: &KerfApp, id: ClipId) -> Option<TrackKind> {
    let tl = &app.project().timeline;
    tl.find_clip(id).and_then(|(t, _)| tl.track(t)).map(|t| t.kind)
}

fn shift_track(app: &KerfApp, from: TrackId, delta: i32) -> TrackId {
    if delta == 0 {
        return from;
    }
    let tl = &app.project().timeline;
    let Some(kind) = tl.track(from).map(|t| t.kind) else { return from };
    let same: Vec<TrackId> = tl.tracks.iter().filter(|t| t.kind == kind).map(|t| t.id).collect();
    let i = same.iter().position(|t| *t == from).unwrap_or(0) as i32;
    same[(i + delta).clamp(0, same.len() as i32 - 1) as usize]
}

/// Move the selection (and links) by `delta`, hopping `track_delta` tracks.
fn move_selected(d: &mut kerf_core::Document, ids: &[ClipId], delta: Time, track_delta: i32) -> Result<(), kerf_core::EditError> {
    d.move_clips(ids, delta, track_delta)
}

fn clip_menu(app: &mut KerfApp, ui: &mut egui::Ui) {
    let Some(id) = app.tl.ctx_clip else {
        if ui.button("Add marker here").clicked() {
            let t = app.playhead;
            app.edit(|d| d.add_marker(t, "Marker"));
            ui.close();
        }
        if ui.button("Close all gaps").clicked() {
            app.edit(|d| d.close_gaps());
            ui.close();
        }
        return;
    };
    let ph = app.playhead;
    if ui.button("Split at playhead").clicked() {
        app.edit(|d| d.split(id, ph));
        ui.close();
    }
    if ui.button("Delete").clicked() {
        app.delete_selection(false);
        ui.close();
    }
    if ui.button("Ripple delete").clicked() {
        app.delete_selection(true);
        ui.close();
    }
    ui.separator();
    if ui.button("Fade in + out 0.5 s").clicked() {
        app.edit(|d| {
            d.edit_clip(id, |c| {
                c.fade_in = Time::from_secs_f64(0.5);
                c.fade_out = Time::from_secs_f64(0.5);
            })
        });
        ui.close();
    }
    if ui.button("Remove fades").clicked() {
        app.edit(|d| {
            d.edit_clip(id, |c| {
                c.fade_in = Time::ZERO;
                c.fade_out = Time::ZERO;
            })
        });
        ui.close();
    }
    let linked = app.project().timeline.find_clip(id).and_then(|(_, c)| c.link).is_some();
    if linked && ui.button("Unlink audio/video").clicked() {
        let ids: Vec<ClipId> = app.project().timeline.linked(id).into_iter().map(|x| x.1).collect();
        app.edit(|d| {
            let mut cmds = vec![];
            for i in ids {
                let (tr, c) = d.project().timeline.find_clip(i).unwrap();
                let mut c = c.clone();
                c.link = None;
                cmds.push(Command::UpdateClip { track: tr, clip: c });
            }
            d.apply_all(cmds)
        });
        ui.close();
    }
    if ui.button("Jump to clip start").clicked() {
        if let Some((_, c)) = app.project().timeline.find_clip(id) {
            let s = c.start;
            app.seek(s);
        }
        ui.close();
    }
    if ui.button("Color grade…").clicked() {
        app.workspace = Workspace::Color;
        ui.close();
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_clip(
    app: &mut KerfApp,
    ctx: &egui::Context,
    p: &egui::Painter,
    r: Rect,
    c: &Clip,
    track: &Track,
    selected: bool,
    a: f32,
    pps: f32,
    clip_to: Rect,
) {
    if r.width() < 1.0 {
        return;
    }
    let (base, accent) = match &c.content {
        ClipContent::Media { .. } if track.kind == TrackKind::Audio => (CLIP_AUDIO, LIME),
        ClipContent::Media { .. } => (CLIP_VIDEO, CYAN),
        ClipContent::Text(_) => (CLIP_TEXT, VIOLET),
        ClipContent::Solid { color } => (Color32::from_rgb(color[0] / 2, color[1] / 2, color[2] / 2), AMBER),
    };
    let rr = CornerRadius::same(6);
    p.rect_filled(r, rr, theme::alpha(base, a));
    let inner = p.with_clip_rect(r.shrink(1.0).intersect(clip_to));
    let label_h = 16.0;

    match &c.content {
        ClipContent::Media { media } if track.kind == TrackKind::Video => {
            if let Some(th) = app.assets.thumbs(*media) {
                let tile_h = r.height() - label_h - 2.0;
                let tile_w = (th.w as f32 / th.h as f32 * tile_h).max(8.0);
                let mut x = r.left();
                // Start at the first visible tile.
                if x < clip_to.left() {
                    x += ((clip_to.left() - x) / tile_w).floor() * tile_w;
                }
                while x < r.right().min(clip_to.right()) {
                    let src = c.source_in.as_secs_f64() + ((x + tile_w / 2.0 - r.left()) / pps) as f64;
                    let idx = (src / th.interval as f64).floor().max(0.0) as usize;
                    if let Some((tex, _)) = app.assets.thumb_texture(ctx, *media, idx) {
                        let tr = Rect::from_min_size(pos2(x, r.top() + label_h), vec2(tile_w, tile_h));
                        inner.image(tex.id(), tr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), theme::alpha(Color32::WHITE, 0.85 * a));
                    }
                    x += tile_w;
                }
            }
        }
        ClipContent::Media { media } => {
            if let Some(w) = app.assets.wave(*media) {
                let mid = r.center().y + label_h / 2.0 - 2.0;
                let half = (r.height() - label_h) / 2.0 - 3.0;
                let gain = (c.volume * track.volume).clamp(0.0, 2.0);
                let x0 = r.left().max(clip_to.left());
                let x1 = r.right().min(clip_to.right());
                let mut x = x0;
                let col = theme::alpha(LIME, 0.75 * a);
                while x < x1 {
                    let s0 = c.source_in.as_secs_f64() + ((x - r.left()) / pps) as f64;
                    let s1 = s0 + (2.0 / pps) as f64;
                    let tl_t = c.start + Time::from_secs_f64(((x - r.left()) / pps) as f64);
                    let pk = (w.peak(s0, s1) * gain * c.fade_gain(tl_t)).min(1.0);
                    let h = (pk * half).max(0.5);
                    inner.line_segment([pos2(x, mid - h), pos2(x, mid + h)], Stroke::new(1.4, col));
                    x += 2.0;
                }
            }
        }
        ClipContent::Text(s) => {
            inner.text(
                pos2(r.left() + 8.0, r.center().y + 6.0),
                egui::Align2::LEFT_CENTER,
                format!("T  {}", s.text.replace('\n', " ")),
                theme::prop(12.0),
                theme::alpha(Color32::WHITE, 0.9 * a),
            );
        }
        ClipContent::Solid { .. } => {}
    }

    // Label strip.
    let strip = Rect::from_min_size(r.min, vec2(r.width(), label_h));
    inner.rect_filled(strip, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, theme::alpha(Color32::BLACK, 0.35 * a));
    let mut name = if c.name.is_empty() { "clip".to_string() } else { c.name.clone() };
    if c.link.is_some() {
        name = format!("⛓ {name}");
    }
    inner.text(pos2(r.left() + 6.0, r.top() + 2.0), egui::Align2::LEFT_TOP, name, theme::prop(10.5), theme::alpha(TEXT, a));
    if !c.grade.is_identity() {
        inner.circle_filled(pos2(r.right() - 9.0, r.top() + 8.0), 3.0, theme::alpha(MAGENTA, a));
    }

    // Fades: darkened ramps + handles.
    let fade_col = theme::alpha(Color32::BLACK, 0.45 * a);
    if c.fade_in > Time::ZERO {
        let fx = r.left() + (c.fade_in.as_secs_f64() as f32) * pps;
        inner.add(egui::Shape::convex_polygon(vec![r.left_top(), pos2(fx, r.top()), r.left_bottom()], fade_col, Stroke::NONE));
        inner.line_segment([r.left_bottom(), pos2(fx, r.top())], Stroke::new(1.0, theme::alpha(accent, 0.8 * a)));
    }
    if c.fade_out > Time::ZERO {
        let fx = r.right() - (c.fade_out.as_secs_f64() as f32) * pps;
        inner.add(egui::Shape::convex_polygon(vec![pos2(fx, r.top()), r.right_top(), r.right_bottom()], fade_col, Stroke::NONE));
        inner.line_segment([pos2(fx, r.top()), r.right_bottom()], Stroke::new(1.0, theme::alpha(accent, 0.8 * a)));
    }
    if selected && r.width() > 30.0 {
        for fx in [
            r.left() + (c.fade_in.as_secs_f64() as f32) * pps,
            r.right() - (c.fade_out.as_secs_f64() as f32) * pps,
        ] {
            inner.circle_filled(pos2(fx, r.top() + 5.0), 3.5, Color32::WHITE);
            inner.circle_stroke(pos2(fx, r.top() + 5.0), 3.5, Stroke::new(1.0, accent));
        }
    }

    if track.hidden || track.muted {
        p.rect_filled(r, rr, theme::alpha(Color32::BLACK, 0.5));
    }
    if selected {
        theme::glow_rect(p, r, 6, accent);
    } else {
        p.rect_stroke(r, rr, Stroke::new(1.0, theme::alpha(theme::lerp(base, accent, 0.45), a)), StrokeKind::Inside);
    }
}

fn headers(app: &mut KerfApp, ui: &mut egui::Ui, head: Rect, body: Rect, lanes: &[Lane]) {
    let p = ui.painter_at(head);
    p.rect_filled(head, 0, PANEL);
    let corner = Rect::from_min_size(head.min, vec2(head.width(), RULER_H));
    p.rect_filled(corner, 0, BG);
    let fps = app.project().timeline.fps;
    p.text(
        corner.left_center() + vec2(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        app.playhead.timecode(fps),
        theme::mono(11.5),
        CYAN,
    );
    let clip_rect = Rect::from_min_max(pos2(head.left(), body.top()), pos2(head.right(), body.bottom()));
    let p = ui.painter_at(clip_rect);
    for lane in lanes {
        let r = Rect::from_x_y_ranges(head.x_range(), lane.rect.y_range());
        if r.bottom() < body.top() || r.top() > body.bottom() {
            continue;
        }
        let Some(track) = app.project().timeline.track(lane.track).cloned() else { continue };
        let accent = if track.kind == TrackKind::Video { CYAN } else { LIME };
        p.rect_filled(r.shrink2(vec2(2.0, 1.0)), 6, PANEL_HI);
        p.rect_filled(Rect::from_min_size(r.min + vec2(2.0, 6.0), vec2(3.0, r.height() - 12.0)), 2, theme::alpha(accent, 0.8));
        p.text(r.left_top() + vec2(12.0, 8.0), egui::Align2::LEFT_TOP, &track.name, theme::prop(12.5), TEXT);

        // Buttons: visibility/mute, lock.
        let b1 = Rect::from_min_size(pos2(r.left() + 10.0, r.bottom() - 26.0), vec2(26.0, 20.0));
        let b2 = b1.translate(vec2(30.0, 0.0));
        let (icon1, on1, tip1) = if track.kind == TrackKind::Video {
            (if track.hidden { "○" } else { "👁" }, track.hidden, "Hide track")
        } else {
            (if track.muted { "🔇" } else { "🔊" }, track.muted, "Mute track")
        };
        for (i, (b, icon, on, tip)) in [(b1, icon1, on1, tip1), (b2, if track.locked { "🔒" } else { "🔓" }, track.locked, "Lock track")]
            .into_iter()
            .enumerate()
        {
            if !clip_rect.intersects(b) {
                continue;
            }
            let resp = ui.interact(b.intersect(clip_rect), ui.scope_id().with(("hdr", track.id.0, i)), Sense::click()).on_hover_text(tip);
            let col = if on { AMBER } else if resp.hovered() { Color32::WHITE } else { TEXT_DIM };
            if on {
                p.rect_filled(b, 5, theme::alpha(AMBER, 0.15));
            } else if resp.hovered() {
                p.rect_filled(b, 5, RAISED);
            }
            p.text(b.center(), egui::Align2::CENTER_CENTER, icon, theme::prop(12.0), col);
            if resp.clicked() {
                let mut t = track.clone();
                match (i, track.kind) {
                    (0, TrackKind::Video) => t.hidden = !t.hidden,
                    (0, TrackKind::Audio) => t.muted = !t.muted,
                    _ => t.locked = !t.locked,
                }
                app.edit(|d| {
                    d.apply(Command::UpdateTrack { id: t.id, name: t.name, muted: t.muted, hidden: t.hidden, locked: t.locked, volume: t.volume })
                });
            }
        }
        // Header context menu: add / remove tracks.
        let hr = ui.interact(r.intersect(clip_rect), ui.scope_id().with(("hdrbody", track.id.0)), Sense::click());
        hr.context_menu(|ui| {
            if ui.button("Add video track").clicked() {
                app.edit(|d| d.add_track(TrackKind::Video));
                ui.close();
            }
            if ui.button("Add audio track").clicked() {
                app.edit(|d| d.add_track(TrackKind::Audio));
                ui.close();
            }
            let empty = track.clips.is_empty();
            if ui.add_enabled(empty, egui::Button::new("Delete track (must be empty)")).clicked() {
                app.edit(|d| d.apply(Command::RemoveTrack { id: track.id }));
                ui.close();
            }
        });
    }
    p.line_segment([pos2(head.right(), head.top()), pos2(head.right(), head.bottom())], Stroke::new(1.0, LINE));
}

fn toolbar(app: &mut KerfApp, ui: &mut egui::Ui) {
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(10, 4)).fill(PANEL).show(ui, |ui| {
        ui.horizontal(|ui| {
            if theme::icon_button(ui, "⬉", app.tool == Tool::Select, "Select tool (A)").clicked() {
                app.tool = Tool::Select;
            }
            if theme::icon_button(ui, "✂", app.tool == Tool::Blade, "Blade tool (B)").clicked() {
                app.tool = Tool::Blade;
            }
            ui.separator();
            if theme::icon_button(ui, "🔪", false, "Split at playhead (S)").clicked() {
                app.split_at_playhead();
            }
            if theme::icon_button(ui, "🗑", false, "Ripple delete selection (Shift+Del)").clicked() {
                app.delete_selection(true);
            }
            if theme::icon_button(ui, "⬅", false, "Close all gaps").clicked() {
                if let Some(n) = app.edit(|d| d.close_gaps()) {
                    app.toast(format!("Closed {n} gap(s)"), CYAN);
                }
            }
            if theme::icon_button(ui, "T", false, "Add title at playhead (T)").clicked() {
                let t = app.playhead;
                app.edit(|d| d.add_text(kerf_core::TextStyle::new("Title"), t, Time::from_secs_f64(3.0)));
            }
            if theme::icon_button(ui, "📌", false, "Add marker (M)").clicked() {
                let t = app.playhead;
                let n = app.project().timeline.markers.len() + 1;
                app.edit(|d| d.add_marker(t, format!("Marker {n}")));
            }
            ui.separator();
            if theme::icon_button(ui, "🎯", app.snapping, "Snapping (N)").clicked() {
                app.snapping = !app.snapping;
            }
            if theme::icon_button(ui, "⏩", app.tl.follow, "Follow playhead").clicked() {
                app.tl.follow = !app.tl.follow;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, "⛶", false, "Zoom to fit (Shift+Z)").clicked() {
                    app.tl.fit_pending = true;
                }
                let mut z = app.tl.pps.ln();
                if ui.add(egui::Slider::new(&mut z, 0.0..=8.3).show_value(false)).changed() {
                    app.tl.pps = z.exp();
                }
                ui.label(egui::RichText::new("ZOOM").size(10.0).color(TEXT_FAINT));
                if !app.selection.is_empty() {
                    ui.label(egui::RichText::new(format!("{} selected", app.selection.len())).size(11.0).color(CYAN));
                }
            });
        });
    });
}

fn nice_step(pps: f32, fps: f64) -> f64 {
    let f = 1.0 / fps;
    let steps = [
        f, 2.0 * f, 5.0 * f, 10.0 * f, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0,
    ];
    for s in steps {
        if s as f32 * pps >= 86.0 {
            return s;
        }
    }
    3600.0
}

fn short_tc(t: f64, fps: f64, step: f64) -> String {
    let total = t.round_ties_even() as i64;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if step < 1.0 {
        let fr = ((t - t.floor()) * fps).round() as i64;
        let s = t.floor() as i64;
        return format!("{:02}:{:02}:{:02}", s / 60, s % 60, fr);
    }
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}
