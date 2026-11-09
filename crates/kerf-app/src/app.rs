//! Top-level app state, layout, shortcuts, saving and the live agent link.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use egui::{Color32, Key, Modifiers, RichText, TextureHandle};
use kerf_core::{ClipId, Command, Document, EditError, Project, Time, TrackKind};
use kerf_media::Frame;

use crate::assets::Assets;
use crate::engine::Engine;
use crate::export_ui::ExportUi;
use crate::theme::{self, *};
use crate::timeline::TimelineState;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Workspace {
    Edit,
    Color,
    Audio,
    Deliver,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LeftTab {
    Media,
    Titles,
    Markers,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Blade,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Full,
    Half,
    Quarter,
}

impl Quality {
    pub fn factor(self) -> f32 {
        match self {
            Quality::Full => 1.0,
            Quality::Half => 0.5,
            Quality::Quarter => 0.25,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Quality::Full => "full",
            Quality::Half => "1/2",
            Quality::Quarter => "1/4",
        }
    }
}

pub struct Toast {
    pub text: String,
    pub at: Instant,
    pub color: Color32,
}

pub struct KerfApp {
    pub doc: Document,
    pub path: Option<PathBuf>,
    saved_rev: u64,
    sent_rev: u64,
    last_change: Instant,
    disk_mtime: Option<SystemTime>,
    last_watch: Instant,
    pub agent_flash: Option<Instant>,

    pub engine: Engine,
    pub assets: Assets,
    pub preview: Option<TextureHandle>,
    pub preview_frame: Option<Arc<Frame>>,
    pub render_ms: f32,
    pub scope_dirty: bool,

    pub playhead: Time,
    last_seek: Time,
    pub selection: Vec<ClipId>,
    pub tool: Tool,
    pub snapping: bool,
    pub tl: TimelineState,
    pub workspace: Workspace,
    pub left_tab: LeftTab,
    pub toasts: Vec<Toast>,
    pub export: ExportUi,
    pub proxies_on: bool,
    pub quality: Quality,
    pub mark_in: Option<Time>,
    pub mark_out: Option<Time>,
    pub looping: bool,
    pub show_safe: bool,
    pub show_help: bool,
    pub recent: Vec<String>,
    pub theme_mode: theme::Mode,
    theme_applied: bool,
}

impl KerfApp {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let theme_mode = load_settings().theme;
        let mut toasts = Vec::new();
        let (project, path) = match path {
            Some(p) if p.exists() => match Project::load(&p) {
                Ok(pr) => (pr, Some(p)),
                Err(e) => {
                    toasts.push(Toast { text: format!("Could not open: {e}"), at: Instant::now(), color: pal::bad() });
                    (Project::default(), None)
                }
            },
            Some(p) => {
                // A path that does not exist yet: start a new project there.
                let mut pr = Project::default();
                pr.name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                (pr, Some(p))
            }
            None => (Project::default(), None),
        };
        let shared = Arc::new(project.clone());
        let engine = Engine::new(cc.egui_ctx.clone(), shared);
        let mut assets = Assets::new(cc.egui_ctx.clone());
        assets.ensure(&project.media);
        let disk_mtime = path.as_ref().and_then(|p| mtime(p));
        let mut app = KerfApp {
            doc: Document::new(project),
            path,
            saved_rev: 0,
            sent_rev: 0,
            last_change: Instant::now(),
            disk_mtime,
            last_watch: Instant::now(),
            agent_flash: None,
            engine,
            assets,
            preview: None,
            preview_frame: None,
            render_ms: 0.0,
            scope_dirty: true,
            playhead: Time::ZERO,
            last_seek: Time(-1),
            selection: vec![],
            tool: Tool::Select,
            snapping: true,
            tl: TimelineState::default(),
            workspace: Workspace::Edit,
            left_tab: LeftTab::Media,
            toasts,
            export: ExportUi::default(),
            proxies_on: true,
            quality: Quality::Half,
            mark_in: None,
            mark_out: None,
            looping: false,
            show_safe: false,
            show_help: false,
            recent: load_recent(),
            theme_mode,
            theme_applied: false,
        };
        if !app.engine.audio_ok {
            app.toast("No audio device: playback is silent", pal::warn());
        }
        if let Some(p) = app.path.clone() {
            app.remember_recent(&p);
        }
        app.export.init(app.doc.project(), app.path.as_deref());
        app
    }

    pub fn project(&self) -> &Project {
        self.doc.project()
    }

    pub fn toast(&mut self, text: impl Into<String>, color: Color32) {
        self.toasts.push(Toast { text: text.into(), at: Instant::now(), color });
    }

    /// Run an edit; errors become a toast instead of a crash.
    pub fn edit<T>(&mut self, f: impl FnOnce(&mut Document) -> Result<T, EditError>) -> Option<T> {
        match f(&mut self.doc) {
            Ok(v) => Some(v),
            Err(e) => {
                self.toast(format!("Can't do that: {e}"), pal::bad());
                None
            }
        }
    }

    /// Coalesced clip property edit (sliders): one undo step per gesture.
    pub fn edit_clip_live(&mut self, id: ClipId, key: &str, f: impl FnOnce(&mut kerf_core::Clip)) {
        let Some((track, c)) = self.project().timeline.find_clip(id) else { return };
        let mut c = c.clone();
        f(&mut c);
        let k = format!("{}:{key}", id.0);
        if let Err(e) = self.doc.apply_merge(Command::UpdateClip { track, clip: c }, &k) {
            self.toast(format!("Can't do that: {e}"), pal::bad());
        }
    }

    pub fn seek(&mut self, t: Time) {
        let end = self.project().timeline.duration().max(Time::ZERO);
        let fps = self.project().timeline.fps;
        self.playhead = t.clamp_range(Time::ZERO, end.max(Time::ZERO)).round_to_frame(fps);
        self.engine.seek(self.playhead);
        self.last_seek = self.playhead;
    }

    pub fn frame_dur(&self) -> Time {
        Time::from_secs_f64(1.0 / self.project().timeline.fps)
    }

    // ---------------------------------------------------------------- files

    fn default_path(&self) -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let dir = home.join("Videos").join("kerf");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!("{}.kerf", date_stamp()))
    }

    pub fn save(&mut self) {
        if self.path.is_none() {
            self.path = Some(self.default_path());
            let p = self.path.clone().unwrap();
            self.remember_recent(&p);
            self.export.init(self.doc.project(), Some(&p));
        }
        let path = self.path.clone().unwrap();
        match self.project().save(&path) {
            Ok(()) => {
                self.saved_rev = self.doc.revision();
                self.disk_mtime = mtime(&path);
            }
            Err(e) => self.toast(format!("Save failed: {e}"), pal::bad()),
        }
    }

    pub fn save_as(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("kerf project", &["kerf"])
            .set_file_name(format!("{}.kerf", self.project().name))
            .save_file()
        {
            let p = if p.extension().is_none() { p.with_extension("kerf") } else { p };
            self.path = Some(p.clone());
            self.save();
            self.remember_recent(&p);
            self.toast(format!("Saved as {}", p.display()), pal::ok());
        }
    }

    pub fn open_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("kerf project", &["kerf"]).pick_file() {
            self.open_path(&p);
        }
    }

    /// Opening replaces the project in this window. A project is just a small
    /// JSON edit list and every asset is cached by file, so this is instant.
    pub fn open_path(&mut self, p: &Path) {
        if self.doc.revision() != self.saved_rev {
            self.save();
        }
        match Project::load(p) {
            Ok(pr) => {
                self.assets.ensure(&pr.media);
                self.doc = Document::new(pr);
                self.saved_rev = 0;
                self.sent_rev = u64::MAX;
                self.path = Some(p.to_path_buf());
                self.disk_mtime = mtime(p);
                self.selection.clear();
                self.seek(Time::ZERO);
                self.remember_recent(p);
                self.export.init(self.doc.project(), Some(p));
                self.tl.fit_pending = true;
            }
            Err(e) => self.toast(format!("Could not open: {e}"), pal::bad()),
        }
    }

    pub fn new_window(&self, path: Option<&Path>) {
        let exe = std::env::current_exe().unwrap_or_else(|_| "kerf-studio".into());
        let mut c = std::process::Command::new(exe);
        if let Some(p) = path {
            c.arg(p);
        }
        let _ = c.spawn();
    }

    fn remember_recent(&mut self, p: &Path) {
        let s = p.to_string_lossy().into_owned();
        self.recent.retain(|r| r != &s);
        self.recent.insert(0, s);
        self.recent.truncate(12);
        save_recent(&self.recent);
    }

    pub fn import_dialog(&mut self) {
        if let Some(files) = rfd::FileDialog::new()
            .add_filter("media", &["mp4", "mov", "mkv", "webm", "avi", "m4v", "mts", "mp3", "wav", "flac", "m4a", "aac", "ogg", "opus", "png", "jpg", "jpeg", "webp", "gif"])
            .pick_files()
        {
            self.import(&files);
        }
    }

    pub fn import(&mut self, files: &[PathBuf]) {
        let mut n = 0;
        for f in files {
            let s = f.to_string_lossy().into_owned();
            if self.project().media.iter().any(|m| m.path == s) {
                continue;
            }
            match kerf_render::import_item(&s) {
                Ok(item) => {
                    if self.edit(|d| d.add_media(item)).is_some() {
                        n += 1;
                    }
                }
                Err(e) => self.toast(format!("{}: {e}", f.display()), pal::bad()),
            }
        }
        if n > 0 {
            self.assets.ensure(&self.doc.project().media.clone());
            self.toast(format!("Imported {n} file{}", if n == 1 { "" } else { "s" }), pal::ok());
        }
    }

    // ------------------------------------------------------------- edits

    pub fn split_at_playhead(&mut self) {
        let t = self.playhead;
        let sel: Vec<ClipId> = self
            .selection
            .iter()
            .copied()
            .filter(|id| {
                self.project()
                    .timeline
                    .find_clip(*id)
                    .map(|(_, c)| c.start < t && c.end() > t)
                    .unwrap_or(false)
            })
            .collect();
        if sel.is_empty() {
            self.edit(|d| d.split_all(t));
        } else {
            self.edit(|d| {
                let mut cmds_ok = Ok(());
                for id in sel {
                    // A linked partner may already have been split with it.
                    if d.project().timeline.find_clip(id).map(|(_, c)| c.start < t && c.end() > t).unwrap_or(false) {
                        if let Err(e) = d.split(id, t) {
                            cmds_ok = Err(e);
                        }
                    }
                }
                cmds_ok
            });
        }
    }

    pub fn delete_selection(&mut self, ripple: bool) {
        if self.selection.is_empty() {
            return;
        }
        let mut ids = vec![];
        for id in &self.selection {
            for (_, l) in self.project().timeline.linked(*id) {
                if !ids.contains(&l) {
                    ids.push(l);
                }
            }
        }
        if self.edit(|d| d.delete(&ids, ripple)).is_some() {
            self.selection.clear();
        }
    }

    pub fn select_with_links(&mut self, id: ClipId, additive: bool) {
        let linked: Vec<ClipId> = self.project().timeline.linked(id).into_iter().map(|(_, c)| c).collect();
        if additive {
            if self.selection.contains(&id) {
                self.selection.retain(|c| !linked.contains(c));
            } else {
                self.selection.extend(linked);
            }
        } else {
            self.selection = linked;
        }
    }

    /// Edit points (clip edges) for up/down navigation.
    fn edit_point(&self, forward: bool) -> Option<Time> {
        let pts = self.project().timeline.snap_points();
        let t = self.playhead;
        if forward {
            pts.into_iter().find(|p| *p > t)
        } else {
            pts.into_iter().rev().find(|p| *p < t)
        }
    }

    // ------------------------------------------------------------ per frame

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let cmd = Modifiers::COMMAND;
        let shift = Modifiers::SHIFT;
        let cs = Modifiers::COMMAND | Modifiers::SHIFT;
        let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
        let end = self.project().timeline.duration();
        let fd = self.frame_dur();

        if k(cs, Key::Z) || k(cmd, Key::Y) {
            self.edit(|d| d.redo());
        }
        if k(cmd, Key::Z) {
            self.edit(|d| d.undo());
        }
        if k(cs, Key::S) {
            self.save_as();
        }
        if k(cmd, Key::S) {
            self.save();
            self.toast("Saved", pal::ok());
        }
        if k(cmd, Key::O) {
            self.open_dialog();
        }
        if k(cmd, Key::I) {
            self.import_dialog();
        }
        if k(cmd, Key::N) {
            self.new_window(None);
        }
        if k(cmd, Key::E) {
            self.workspace = Workspace::Deliver;
        }
        if k(cmd, Key::A) {
            self.selection = self.project().timeline.clips().map(|(_, c)| c.id).collect();
        }
        if k(cmd, Key::B) || k(Modifiers::NONE, Key::S) {
            self.split_at_playhead();
        }
        if k(Modifiers::NONE, Key::Space) {
            self.engine.toggle(end);
        }
        if k(Modifiers::NONE, Key::K) && self.engine.is_playing() {
            self.engine.pause();
        }
        if k(Modifiers::NONE, Key::L) && !self.engine.is_playing() {
            self.engine.play_from(self.playhead);
        }
        if k(Modifiers::NONE, Key::J) {
            let t = self.playhead - Time::from_secs_f64(2.0);
            self.seek(t);
        }
        let step = |s: &mut Self, d: Time| {
            if s.engine.is_playing() {
                s.engine.pause();
            }
            let t = s.playhead + d;
            s.seek(t);
        };
        if k(shift, Key::ArrowLeft) {
            step(self, -Time::from_secs_f64(1.0));
        }
        if k(shift, Key::ArrowRight) {
            step(self, Time::from_secs_f64(1.0));
        }
        if k(Modifiers::NONE, Key::ArrowLeft) {
            step(self, -fd);
        }
        if k(Modifiers::NONE, Key::ArrowRight) {
            step(self, fd);
        }
        if k(Modifiers::NONE, Key::ArrowUp) {
            if let Some(t) = self.edit_point(false) {
                self.seek(t);
            }
        }
        if k(Modifiers::NONE, Key::ArrowDown) {
            if let Some(t) = self.edit_point(true) {
                self.seek(t);
            }
        }
        if k(Modifiers::NONE, Key::Home) {
            self.seek(Time::ZERO);
        }
        if k(Modifiers::NONE, Key::End) {
            self.seek(end);
        }
        if k(Modifiers::NONE, Key::I) {
            self.mark_in = Some(self.playhead);
        }
        if k(Modifiers::NONE, Key::O) {
            self.mark_out = Some(self.playhead);
        }
        if k(Modifiers::ALT, Key::X) {
            self.mark_in = None;
            self.mark_out = None;
        }
        if k(Modifiers::NONE, Key::B) {
            self.tool = Tool::Blade;
        }
        if k(Modifiers::NONE, Key::A) || k(Modifiers::NONE, Key::V) {
            self.tool = Tool::Select;
        }
        if k(Modifiers::NONE, Key::N) {
            self.snapping = !self.snapping;
            let s = if self.snapping { "Snapping on" } else { "Snapping off" };
            self.toast(s, pal::ink());
        }
        if k(Modifiers::NONE, Key::M) {
            let t = self.playhead;
            let n = self.project().timeline.markers.len() + 1;
            self.edit(|d| d.add_marker(t, format!("Marker {n}")));
        }
        if k(Modifiers::NONE, Key::T) {
            let t = self.playhead;
            self.edit(|d| d.add_text(kerf_core::TextStyle::new("Title"), t, Time::from_secs_f64(3.0)));
        }
        if k(shift, Key::Delete) || k(shift, Key::Backspace) {
            self.delete_selection(true);
        }
        if k(Modifiers::NONE, Key::Delete) || k(Modifiers::NONE, Key::Backspace) {
            self.delete_selection(false);
        }
        if k(Modifiers::NONE, Key::Escape) {
            self.selection.clear();
            self.tool = Tool::Select;
        }
        if k(Modifiers::NONE, Key::Plus) || k(Modifiers::NONE, Key::Equals) {
            self.tl.zoom_by(1.25);
        }
        if k(Modifiers::NONE, Key::Minus) {
            self.tl.zoom_by(0.8);
        }
        if k(shift, Key::Z) {
            self.tl.fit_pending = true;
        }
        if k(Modifiers::NONE, Key::F1) || k(shift, Key::Questionmark) {
            self.show_help = !self.show_help;
        }
    }

    /// Reload the project when something else (an agent) wrote the file.
    fn watch_disk(&mut self) {
        if self.last_watch.elapsed() < Duration::from_millis(400) {
            return;
        }
        self.last_watch = Instant::now();
        let Some(path) = self.path.clone() else { return };
        let m = mtime(&path);
        if m.is_none() || m == self.disk_mtime {
            return;
        }
        self.disk_mtime = m;
        match Project::load(&path) {
            Ok(p) if &p != self.project() => {
                // Undoable, so an agent edit you don't like is one Ctrl+Z away.
                self.doc.apply(Command::Replace { project: Box::new(p) }).ok();
                self.saved_rev = self.doc.revision();
                let valid: Vec<ClipId> = self.selection.iter().copied().filter(|c| self.project().timeline.find_clip(*c).is_some()).collect();
                self.selection = valid;
                self.agent_flash = Some(Instant::now());
                self.toast("Synced external edit (Ctrl+Z to undo)", pal::info());
            }
            Ok(_) => {}
            Err(e) => log::warn!("reload: {e}"),
        }
    }

    fn pump(&mut self, ctx: &egui::Context) {
        // Finished assets.
        let (done, errors) = {
            let mut s = self.assets.shared.lock().unwrap();
            (std::mem::take(&mut s.proxy_done), std::mem::take(&mut s.errors))
        };
        for e in errors {
            self.toast(e, pal::bad());
        }
        for (id, path) in done {
            if let Some(m) = self.project().media_item(id) {
                let mut m = m.clone();
                m.proxy = Some(path);
                self.edit(|d| d.apply(Command::UpdateMedia { item: m }));
                self.toast("Proxy ready", pal::ok());
            }
        }
        // Newest rendered frame.
        let mut latest = None;
        while let Ok(f) = self.engine.frames.try_recv() {
            latest = Some(f);
        }
        if let Some(r) = latest {
            let img = egui::ColorImage::from_rgba_unmultiplied([r.frame.w as usize, r.frame.h as usize], &r.frame.data);
            match &mut self.preview {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.preview = Some(ctx.load_texture("preview", img, egui::TextureOptions::LINEAR)),
            }
            self.render_ms = self.render_ms * 0.8 + r.ms * 0.2;
            self.preview_frame = Some(Arc::new(r.frame));
            self.scope_dirty = true;
        }
    }

    fn after_ui(&mut self) {
        let rev = self.doc.revision();
        if rev != self.sent_rev {
            self.sent_rev = rev;
            self.last_change = Instant::now();
            self.engine.set_project(Arc::new(self.project().clone()));
            self.assets.ensure(&self.doc.project().media.clone());
            if self.path.is_none() && rev > 0 {
                // Projects are always on disk: the first edit gives it a home.
                self.save();
            }
        }
        if !self.engine.is_playing() && self.playhead != self.last_seek {
            self.engine.seek(self.playhead);
            self.last_seek = self.playhead;
        }
        // Autosave shortly after the last change (saving is a ~1 ms write).
        if rev != self.saved_rev && self.last_change.elapsed() > Duration::from_millis(350) && self.path.is_some() {
            self.save();
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let h = 48.0;
        egui::Panel::top("top")
            .exact_size(h)
            .frame(egui::Frame::NONE.fill(pal::bg()).inner_margin(egui::Margin::symmetric(16, 0)))
            .show(ui, |ui| {
                let bottom = ui.max_rect().bottom();
                ui.painter().line_segment(
                    [egui::pos2(ui.max_rect().left() - 16.0, bottom - 0.5), egui::pos2(ui.max_rect().right() + 16.0, bottom - 0.5)],
                    egui::Stroke::new(1.0, pal::rule()),
                );
                ui.horizontal_centered(|ui| {
                    // Wordmark: `kerf` in ink, the period faint, accent on hover.
                    let g1 = ui.painter().layout_no_wrap("kerf".into(), theme::bold(16.0), pal::ink());
                    let g2 = ui.painter().layout_no_wrap(".".into(), theme::bold(16.0), pal::faint());
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(g1.size().x + g2.size().x, 22.0), egui::Sense::hover());
                    let y = r.center().y - g1.size().y / 2.0;
                    let dot = if resp.hovered() { pal::accent() } else { pal::faint() };
                    let w1 = g1.size().x;
                    ui.painter().galley(egui::pos2(r.left(), y), g1, pal::ink());
                    ui.painter().galley_with_override_text_color(egui::pos2(r.left() + w1, y), g2, dot);
                    ui.label(RichText::new("/").color(pal::faint()));
                    ui.label(RichText::new(self.project().name.to_lowercase()).color(pal::dim()));
                    ui.add_space(16.0);
                    // Menus read as nav words: no frame until hovered.
                    ui.scope(|ui| {
                        let w = &mut ui.style_mut().visuals.widgets;
                        w.inactive.bg_stroke = egui::Stroke::NONE;
                        w.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                        w.inactive.fg_stroke = egui::Stroke::new(1.0, pal::dim());
                        w.hovered.bg_stroke = egui::Stroke::NONE;
                        w.open.bg_stroke = egui::Stroke::NONE;
                        ui.spacing_mut().button_padding.x = 6.0;
                        self.menus(ui);
                    });

                    // Workspaces as nav, centred, underline on the header's edge.
                    let avail = ui.available_width();
                    ui.add_space((avail / 2.0 - 190.0).max(8.0));
                    ui.spacing_mut().item_spacing.x = 20.0;
                    for (w, name) in [
                        (Workspace::Edit, "edit"),
                        (Workspace::Color, "color"),
                        (Workspace::Audio, "audio"),
                        (Workspace::Deliver, "deliver"),
                    ] {
                        if theme::nav(ui, self.workspace == w, name, bottom).clicked() {
                            self.workspace = w;
                        }
                    }
                    ui.spacing_mut().item_spacing.x = 8.0;

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Theme toggle: a half-filled circle.
                        let (tr, tresp) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
                        let c = tr.center();
                        let col = if tresp.hovered() { pal::ink() } else { pal::dim() };
                        ui.painter().circle_stroke(c, 7.0, egui::Stroke::new(1.2, col));
                        let half: Vec<egui::Pos2> = (0..=16)
                            .map(|i| {
                                let a = std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 16.0;
                                c + egui::vec2(a.cos(), a.sin()) * 7.0
                            })
                            .collect();
                        ui.painter().add(egui::Shape::convex_polygon(half, col, egui::Stroke::NONE));
                        if tresp.on_hover_text("theme: system → dark → light").clicked() {
                            self.theme_mode = match self.theme_mode {
                                theme::Mode::System => theme::Mode::Dark,
                                theme::Mode::Dark => theme::Mode::Light,
                                theme::Mode::Light => theme::Mode::System,
                            };
                            save_settings(&Settings { theme: self.theme_mode });
                        }
                        ui.add_space(8.0);
                        // The agent link, as a status tag.
                        let flash = self.agent_flash.map(|t| t.elapsed().as_secs_f32()).unwrap_or(99.0);
                        let r = if self.path.is_none() {
                            theme::tag(ui, "agent link off", None)
                        } else if flash < 3.0 {
                            theme::tag(ui, "agent edit synced", Some(pal::info()))
                        } else {
                            theme::tag(ui, "agent link", Some(pal::ok()))
                        };
                        if let Some(p) = &self.path {
                            r.on_hover_text(format!(
                                "agents edit this project with\n  kerf -p {} <command>\nor the mcp server (kerf mcp).\nchanges show up here live and are undoable.",
                                p.display()
                            ));
                        } else {
                            r.on_hover_text("save the project to let agents edit it");
                        }
                        ui.add_space(8.0);
                        if theme::icon_button(ui, "redo", false, "ctrl+shift+z").clicked() {
                            self.edit(|d| d.redo());
                        }
                        if theme::icon_button(ui, "undo", false, "ctrl+z").clicked() {
                            self.edit(|d| d.undo());
                        }
                    });
                });
            });
    }

    fn menus(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, label: &str, key: &str| ui.add(egui::Button::new(label).shortcut_text(RichText::new(key).color(pal::faint())));
        ui.menu_button("file", |ui| {
            if item(ui, "new window", "ctrl+n").clicked() {
                self.new_window(None);
            }
            if item(ui, "open…", "ctrl+o").clicked() {
                self.open_dialog();
            }
            if item(ui, "open in new window…", "").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("kerf project", &["kerf"]).pick_file() {
                    self.new_window(Some(&p));
                }
            }
            ui.menu_button("open recent", |ui| {
                let recent = self.recent.clone();
                if recent.is_empty() {
                    ui.label(RichText::new("nothing yet").color(pal::faint()));
                }
                for r in recent {
                    let name = Path::new(&r).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(r.clone());
                    if ui.button(name).on_hover_text(&r).clicked() {
                        self.open_path(Path::new(&r));
                    }
                }
            });
            ui.separator();
            if item(ui, "save", "ctrl+s").clicked() {
                self.save();
            }
            if item(ui, "save as…", "ctrl+shift+s").clicked() {
                self.save_as();
            }
            ui.separator();
            if item(ui, "import media…", "ctrl+i").clicked() {
                self.import_dialog();
            }
            if item(ui, "export…", "ctrl+e").clicked() {
                self.workspace = Workspace::Deliver;
            }
        });
        ui.menu_button("edit", |ui| {
            if ui.add_enabled(self.doc.can_undo(), egui::Button::new("undo").shortcut_text("ctrl+z")).clicked() {
                self.edit(|d| d.undo());
            }
            if ui.add_enabled(self.doc.can_redo(), egui::Button::new("redo").shortcut_text("ctrl+shift+z")).clicked() {
                self.edit(|d| d.redo());
            }
            ui.separator();
            if item(ui, "split at playhead", "s").clicked() {
                self.split_at_playhead();
            }
            if item(ui, "delete", "del").clicked() {
                self.delete_selection(false);
            }
            if item(ui, "ripple delete", "shift+del").clicked() {
                self.delete_selection(true);
            }
            if item(ui, "close all gaps", "").clicked() {
                if let Some(n) = self.edit(|d| d.close_gaps()) {
                    self.toast(format!("closed {n} gap(s)"), pal::ok());
                }
            }
            if let (Some(a), Some(b)) = (self.mark_in, self.mark_out) {
                if b > a && item(ui, "cut out in → out", "").clicked() {
                    self.edit(|d| d.delete_range(kerf_core::TimeRange::from_bounds(a, b), true));
                    self.mark_in = None;
                    self.mark_out = None;
                }
            }
            ui.separator();
            if item(ui, "keyboard shortcuts", "f1").clicked() {
                self.show_help = true;
            }
        });
        ui.menu_button("timeline", |ui| {
            if item(ui, "add video track", "").clicked() {
                self.edit(|d| d.add_track(TrackKind::Video));
            }
            if item(ui, "add audio track", "").clicked() {
                self.edit(|d| d.add_track(TrackKind::Audio));
            }
            ui.separator();
            ui.label(RichText::new("// format").font(theme::mono(FS_XS)).color(pal::faint()));
            let tl = self.project().timeline.clone();
            for (name, w, h) in [
                ("1080p", 1920, 1080),
                ("4k uhd", 3840, 2160),
                ("720p", 1280, 720),
                ("vertical", 1080, 1920),
                ("square", 1080, 1080),
                ("portrait 4:5", 1080, 1350),
            ] {
                let on = tl.width == w && tl.height == h;
                let label = format!("{name:<13}{w}×{h}");
                if ui.selectable_label(on, label).clicked() {
                    self.edit(|d| d.apply(Command::SetFormat { width: w, height: h, fps: tl.fps, background: tl.background }));
                    self.export.init(self.doc.project(), self.path.as_deref());
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                for fps in [24.0, 25.0, 30.0, 50.0, 60.0] {
                    if ui.selectable_label((tl.fps - fps).abs() < 0.01, format!("{fps}")).clicked() {
                        self.edit(|d| d.apply(Command::SetFormat { width: tl.width, height: tl.height, fps, background: tl.background }));
                        self.export.init(self.doc.project(), self.path.as_deref());
                    }
                }
                ui.label(RichText::new("fps").color(pal::faint()));
            });
        });
        ui.menu_button("view", |ui| {
            ui.checkbox(&mut self.proxies_on, "preview from proxies");
            ui.checkbox(&mut self.show_safe, "safe-area guides");
            ui.checkbox(&mut self.looping, "loop in → out");
            ui.checkbox(&mut self.snapping, "snapping");
            ui.checkbox(&mut self.tl.follow, "follow playhead");
            ui.separator();
            for (m, n) in [(theme::Mode::System, "theme: system"), (theme::Mode::Dark, "theme: dark"), (theme::Mode::Light, "theme: light")] {
                if ui.selectable_label(self.theme_mode == m, n).clicked() {
                    self.theme_mode = m;
                    save_settings(&Settings { theme: m });
                }
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(egui::Frame::NONE.fill(pal::bg()).inner_margin(egui::Margin::symmetric(16, 0)))
            .show(ui, |ui| {
                let top = ui.max_rect().top();
                ui.painter().line_segment(
                    [egui::pos2(ui.max_rect().left() - 16.0, top + 0.5), egui::pos2(ui.max_rect().right() + 16.0, top + 0.5)],
                    egui::Stroke::new(1.0, pal::rule()),
                );
                ui.horizontal_centered(|ui| {
                    let tl = &self.project().timeline;
                    let s = |t: String| RichText::new(t).font(theme::mono(FS_2XS)).color(pal::faint());
                    let tool = if self.tool == Tool::Blade { "blade" } else { "select" };
                    let line = format!(
                        "{}×{} · {} fps · {} clips · {} media · render {:.0} ms · preview {} {} · {}{}",
                        tl.width,
                        tl.height,
                        tl.fps,
                        tl.clips().count(),
                        self.project().media.len(),
                        self.render_ms,
                        self.quality.label(),
                        if self.proxies_on { "proxy" } else { "orig" },
                        tool,
                        if self.snapping { " · snap" } else { "" },
                    );
                    ui.label(s(line));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(p) = &self.path {
                            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.label(s(name)).on_hover_text(p.display().to_string());
                        } else {
                            ui.label(s("unsaved · the first edit saves to ~/videos/kerf".into()));
                        }
                    });
                });
            });
    }

    /// One toast at a time, bottom-centre, ~2.6 s, fades and rises in.
    fn draw_toasts(&mut self, ctx: &egui::Context) {
        let life = 2.6;
        self.toasts.retain(|t| t.at.elapsed().as_secs_f32() < life);
        let Some(t) = self.toasts.last() else { return };
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("toasts")));
        let age = t.at.elapsed().as_secs_f32();
        let a = (age / 0.2).min(1.0).min((life - age) / 0.3).clamp(0.0, 1.0);
        let rise = (1.0 - (age / 0.2).min(1.0)) * 10.0;
        let mut text = t.text.clone();
        if let Some(f) = text.get(0..1) {
            text = f.to_lowercase() + &text[1..];
        }
        let galley = painter.layout_no_wrap(text, theme::mono(FS_SM), theme::alpha(pal::body(), a));
        let w = galley.size().x + 40.0;
        let rect = egui::Rect::from_center_size(egui::pos2(screen.center().x, screen.bottom() - 64.0 + rise), egui::vec2(w, 34.0));
        // `--shadow-pop`: only floating things get a shadow.
        painter.add(egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: egui::Color32::from_black_alpha((90.0 * a) as u8) }.as_shape(rect, theme::R_CARD));
        painter.rect_filled(rect, theme::R_CARD, theme::alpha(pal::raised(), a));
        painter.rect_stroke(rect, theme::R_CARD, egui::Stroke::new(1.0, theme::alpha(pal::rule_strong(), a)), egui::StrokeKind::Inside);
        painter.circle_filled(rect.left_center() + egui::vec2(16.0, 0.0), 3.0, theme::alpha(t.color, a));
        painter.galley(rect.left_center() + egui::vec2(28.0, -galley.size().y / 2.0), galley, pal::body());
        ctx.request_repaint_after(Duration::from_millis(30));
    }

    fn help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_help;
        egui::Window::new(RichText::new("// keyboard").font(theme::mono(FS_XS)).color(pal::faint()))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .show(ctx, |ui| {
                egui::Grid::new("keys").num_columns(2).spacing([28.0, 7.0]).show(ui, |ui| {
                    for (k, d) in [
                        ("space  k  l", "play-pause, pause, play"),
                        ("j", "back 2 s"),
                        ("← →  (shift)", "one frame (one second)"),
                        ("↑ ↓", "previous / next edit point"),
                        ("home  end", "start / end"),
                        ("i  o  alt+x", "mark in, out, clear"),
                        ("s  ctrl+b", "split at the playhead"),
                        ("b  a", "blade / select tool"),
                        ("del  shift+del", "delete / ripple delete"),
                        ("m  t", "add marker / title"),
                        ("n", "snapping"),
                        ("+  −  shift+z", "zoom in, out, fit"),
                        ("ctrl+wheel", "zoom at the mouse"),
                        ("alt+drag", "move without the linked clip"),
                        ("ctrl+z  ctrl+shift+z", "undo / redo"),
                        ("ctrl+s  ctrl+o  ctrl+i", "save, open, import"),
                        ("ctrl+n", "new window"),
                        ("ctrl+e", "deliver"),
                    ] {
                        ui.label(RichText::new(k).color(pal::ink()));
                        ui.label(RichText::new(d).color(pal::dim()));
                        ui.end_row();
                    }
                });
            });
        self.show_help = open;
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Settings {
    #[serde(default)]
    theme: theme::Mode,
}

fn settings_file() -> PathBuf {
    recent_file().with_file_name("settings.json")
}

fn load_settings() -> Settings {
    std::fs::read_to_string(settings_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_settings(s: &Settings) {
    let _ = std::fs::write(settings_file(), serde_json::to_string_pretty(s).unwrap_or_default());
}

impl eframe::App for KerfApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let light = match self.theme_mode {
            theme::Mode::Light => true,
            theme::Mode::Dark => false,
            theme::Mode::System => ctx.system_theme() == Some(egui::Theme::Light),
        };
        if light != theme::is_light() || !self.theme_applied {
            theme::apply(&ctx, light);
            self.theme_applied = true;
        }
        // Files dropped onto the window are imported.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if !dropped.is_empty() {
            let (projects, media): (Vec<PathBuf>, Vec<PathBuf>) =
                dropped.into_iter().partition(|p| p.extension().map(|e| e == "kerf").unwrap_or(false));
            if let Some(p) = projects.first() {
                self.open_path(p);
            }
            self.import(&media);
        }
        self.pump(&ctx);
        self.watch_disk();
        self.shortcuts(&ctx);
        self.engine.set_proxies(self.proxies_on);

        // Playback clock.
        if self.engine.is_playing() {
            let end = self.project().timeline.duration();
            let mut t = self.engine.now();
            let (lin, lout) = (self.mark_in.unwrap_or(Time::ZERO), self.mark_out.unwrap_or(end));
            if self.looping && lout > lin && t >= lout {
                self.engine.play_from(lin);
                t = lin;
            } else if t >= end {
                self.engine.pause();
                t = end;
                self.engine.seek(end);
            }
            self.playhead = t;
            self.last_seek = t;
            ctx.request_repaint();
        }

        self.top_bar(ui);
        self.status_bar(ui);
        egui::Panel::bottom("timeline")
            .resizable(true)
            .default_size(330.0)
            .size_range(180.0..=800.0)
            .frame(egui::Frame::NONE.fill(pal::bg()).stroke(egui::Stroke::new(1.0, pal::rule())))
            .show(ui, |ui| crate::timeline::show(self, ui));
        egui::Panel::left("left")
            .resizable(true)
            .default_size(270.0)
            .size_range(200.0..=480.0)
            .frame(egui::Frame::NONE.fill(pal::bg()).inner_margin(egui::Margin::same(10)).stroke(egui::Stroke::new(1.0, pal::rule())))
            .show(ui, |ui| crate::panels::left_panel(self, ui));
        egui::Panel::right("right")
            .resizable(true)
            .default_size(320.0)
            .size_range(260.0..=520.0)
            .frame(egui::Frame::NONE.fill(pal::bg()).inner_margin(egui::Margin::same(10)).stroke(egui::Stroke::new(1.0, pal::rule())))
            .show(ui, |ui| crate::panels::right_panel(self, ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(pal::bg()))
            .show(ui, |ui| crate::viewer::show(self, ui));

        self.draw_toasts(&ctx);
        self.help_window(&ctx);
        self.export.poll(&ctx);
        self.after_ui();
        ctx.request_repaint_after(Duration::from_millis(400));
    }

    fn on_exit(&mut self) {
        if self.doc.revision() != self.saved_rev && self.path.is_some() {
            self.save();
        }
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

fn recent_file() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    let d = base.join("kerf");
    let _ = std::fs::create_dir_all(&d);
    d.join("recent.json")
}

fn load_recent() -> Vec<String> {
    std::fs::read_to_string(recent_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_recent(r: &[String]) {
    let _ = std::fs::write(recent_file(), serde_json::to_string_pretty(r).unwrap_or_default());
}

/// `2026-11-09-0812` in local-ish time (UTC; good enough for a file name).
fn date_stamp() -> String {
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}-{:02}{:02}", rem / 3600, (rem / 60) % 60)
}
