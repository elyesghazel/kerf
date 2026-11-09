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
            Quality::Full => "Full",
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
}

impl KerfApp {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        theme::install(&cc.egui_ctx);
        let mut toasts = Vec::new();
        let (project, path) = match path {
            Some(p) if p.exists() => match Project::load(&p) {
                Ok(pr) => (pr, Some(p)),
                Err(e) => {
                    toasts.push(Toast { text: format!("Could not open: {e}"), at: Instant::now(), color: RED });
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
        };
        if !app.engine.audio_ok {
            app.toast("No audio device: playback is silent", AMBER);
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
                self.toast(format!("Can't do that: {e}"), RED);
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
            self.toast(format!("Can't do that: {e}"), RED);
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
            Err(e) => self.toast(format!("Save failed: {e}"), RED),
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
            self.toast(format!("Saved as {}", p.display()), LIME);
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
            Err(e) => self.toast(format!("Could not open: {e}"), RED),
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
                Err(e) => self.toast(format!("{}: {e}", f.display()), RED),
            }
        }
        if n > 0 {
            self.assets.ensure(&self.doc.project().media.clone());
            self.toast(format!("Imported {n} file{}", if n == 1 { "" } else { "s" }), LIME);
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
            self.toast("Saved", LIME);
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
            self.toast(s, CYAN);
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
                self.toast("Synced external edit (Ctrl+Z to undo)", VIOLET);
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
            self.toast(e, RED);
        }
        for (id, path) in done {
            if let Some(m) = self.project().media_item(id) {
                let mut m = m.clone();
                m.proxy = Some(path);
                self.edit(|d| d.apply(Command::UpdateMedia { item: m }));
                self.toast("Proxy ready", LIME);
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
        egui::Panel::top("top")
            .exact_size(44.0)
            .frame(egui::Frame::NONE.fill(BG).inner_margin(egui::Margin::symmetric(12, 0)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    // Logo: a neon blade mark.
                    let (r, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                    let p = ui.painter();
                    let c = r.center();
                    theme::glow_line(p, c + egui::vec2(-7.0, 8.0), c + egui::vec2(7.0, -8.0), CYAN, 2.5);
                    p.circle_filled(c + egui::vec2(-7.0, 8.0), 2.5, VIOLET);
                    ui.label(RichText::new("kerf").size(18.0).strong().color(Color32::WHITE));
                    ui.add_space(6.0);
                    self.menus(ui);
                    ui.add_space(10.0);
                    let dirty = self.doc.revision() != self.saved_rev;
                    ui.label(RichText::new(&self.project().name).color(TEXT_DIM));
                    if dirty {
                        ui.label(RichText::new("⏺").color(AMBER).size(9.0));
                    }

                    // Workspace tabs, centred.
                    let avail = ui.available_width();
                    ui.add_space((avail / 2.0 - 220.0).max(8.0));
                    for (w, name) in [
                        (Workspace::Edit, "✂  EDIT"),
                        (Workspace::Color, "🎨  COLOR"),
                        (Workspace::Audio, "♫  AUDIO"),
                        (Workspace::Deliver, "📤  DELIVER"),
                    ] {
                        if theme::pill(ui, self.workspace == w, name).clicked() {
                            self.workspace = w;
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // The agent link: lit when a project file is being watched.
                        let flash = self.agent_flash.map(|t| t.elapsed().as_secs_f32()).unwrap_or(99.0);
                        let (txt, col) = if self.path.is_none() {
                            ("AGENT LINK · unsaved", TEXT_FAINT)
                        } else if flash < 3.0 {
                            ("AGENT EDIT SYNCED", VIOLET)
                        } else {
                            ("AGENT LINK · LIVE", LIME)
                        };
                        let r = ui.label(RichText::new(txt).size(10.5).color(col).strong());
                        let dot = r.rect.left_center() - egui::vec2(9.0, 0.0);
                        ui.painter().circle_filled(dot, 3.5, col);
                        ui.painter().circle_filled(dot, 7.0, theme::alpha(col, 0.15));
                        if let Some(p) = &self.path {
                            r.on_hover_text(format!(
                                "Agents edit this project with:\n  kerf -p {} <command>\nor the MCP server (kerf mcp).\nChanges appear here live and are undoable.",
                                p.display()
                            ));
                        }
                        ui.add_space(14.0);
                        if theme::icon_button(ui, "⟳", false, "Redo (Ctrl+Shift+Z)").clicked() {
                            self.edit(|d| d.redo());
                        }
                        if theme::icon_button(ui, "⟲", false, "Undo (Ctrl+Z)").clicked() {
                            self.edit(|d| d.undo());
                        }
                    });
                });
            });
    }

    fn menus(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            if ui.button("New window          Ctrl+N").clicked() {
                self.new_window(None);
            }
            if ui.button("Open…               Ctrl+O").clicked() {
                self.open_dialog();
            }
            if ui.button("Open in new window…").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("kerf project", &["kerf"]).pick_file() {
                    self.new_window(Some(&p));
                }
            }
            ui.menu_button("Open recent", |ui| {
                let recent = self.recent.clone();
                if recent.is_empty() {
                    ui.label(RichText::new("Nothing yet").color(TEXT_FAINT));
                }
                for r in recent {
                    let name = Path::new(&r).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(r.clone());
                    if ui.button(name).on_hover_text(&r).clicked() {
                        self.open_path(Path::new(&r));
                    }
                }
            });
            ui.separator();
            if ui.button("Save                Ctrl+S").clicked() {
                self.save();
            }
            if ui.button("Save as…            Ctrl+Shift+S").clicked() {
                self.save_as();
            }
            ui.separator();
            if ui.button("Import media…       Ctrl+I").clicked() {
                self.import_dialog();
            }
            if ui.button("Export…             Ctrl+E").clicked() {
                self.workspace = Workspace::Deliver;
            }
        });
        ui.menu_button("Edit", |ui| {
            if ui.add_enabled(self.doc.can_undo(), egui::Button::new("Undo   Ctrl+Z")).clicked() {
                self.edit(|d| d.undo());
            }
            if ui.add_enabled(self.doc.can_redo(), egui::Button::new("Redo   Ctrl+Shift+Z")).clicked() {
                self.edit(|d| d.redo());
            }
            ui.separator();
            if ui.button("Split at playhead     S").clicked() {
                self.split_at_playhead();
            }
            if ui.button("Delete                Del").clicked() {
                self.delete_selection(false);
            }
            if ui.button("Ripple delete         Shift+Del").clicked() {
                self.delete_selection(true);
            }
            if ui.button("Close all gaps").clicked() {
                if let Some(n) = self.edit(|d| d.close_gaps()) {
                    self.toast(format!("Closed {n} gap(s)"), CYAN);
                }
            }
            if let (Some(a), Some(b)) = (self.mark_in, self.mark_out) {
                if b > a && ui.button("Cut out In›Out (ripple)").clicked() {
                    self.edit(|d| d.delete_range(kerf_core::TimeRange::from_bounds(a, b), true));
                    self.mark_in = None;
                    self.mark_out = None;
                }
            }
            ui.separator();
            if ui.button("Keyboard shortcuts    F1").clicked() {
                self.show_help = true;
            }
        });
        ui.menu_button("Timeline", |ui| {
            if ui.button("Add video track").clicked() {
                self.edit(|d| d.add_track(TrackKind::Video));
            }
            if ui.button("Add audio track").clicked() {
                self.edit(|d| d.add_track(TrackKind::Audio));
            }
            ui.separator();
            ui.label(RichText::new("FORMAT").size(10.0).color(TEXT_DIM));
            let tl = self.project().timeline.clone();
            for (name, w, h) in [
                ("1080p  1920×1080", 1920, 1080),
                ("4K UHD  3840×2160", 3840, 2160),
                ("720p  1280×720", 1280, 720),
                ("Vertical  1080×1920", 1080, 1920),
                ("Square  1080×1080", 1080, 1080),
                ("Portrait 4:5  1080×1350", 1080, 1350),
            ] {
                let on = tl.width == w && tl.height == h;
                if ui.selectable_label(on, name).clicked() {
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
                ui.label(RichText::new("fps").color(TEXT_DIM));
            });
        });
        ui.menu_button("View", |ui| {
            ui.checkbox(&mut self.proxies_on, "Use proxies in preview");
            ui.checkbox(&mut self.show_safe, "Safe-area guides");
            ui.checkbox(&mut self.looping, "Loop playback (In›Out)");
            ui.checkbox(&mut self.snapping, "Snapping (N)");
            ui.checkbox(&mut self.tl.follow, "Follow playhead");
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(egui::Frame::NONE.fill(BG).inner_margin(egui::Margin::symmetric(12, 0)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let tl = &self.project().timeline;
                    let s = |t: String| RichText::new(t).size(10.5).color(TEXT_DIM).family(egui::FontFamily::Monospace);
                    ui.label(s(format!("{}×{} · {} fps", tl.width, tl.height, tl.fps)));
                    ui.label(s(format!("· {} clips · {} media", tl.clips().count(), self.project().media.len())));
                    ui.label(s(format!("· render {:.1} ms", self.render_ms)));
                    ui.label(s(format!("· preview {} {}", self.quality.label(), if self.proxies_on { "proxy" } else { "orig" })));
                    let tool = if self.tool == Tool::Blade { "BLADE" } else { "SELECT" };
                    ui.label(s(format!("· tool {tool}")));
                    if self.snapping {
                        ui.label(RichText::new("· SNAP").size(10.5).color(CYAN));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(p) = &self.path {
                            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.label(s(name)).on_hover_text(p.display().to_string());
                        } else {
                            ui.label(s("unsaved · first edit saves to ~/Videos/kerf".into()));
                        }
                    });
                });
            });
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(4));
        if self.toasts.is_empty() {
            return;
        }
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("toasts")));
        let mut y = screen.bottom() - 40.0;
        for t in self.toasts.iter().rev().take(4) {
            let age = t.at.elapsed().as_secs_f32();
            let a = (4.0 - age).clamp(0.0, 0.4) / 0.4;
            let galley = painter.layout_no_wrap(t.text.clone(), theme::prop(12.5), theme::alpha(TEXT, a));
            let w = galley.size().x + 36.0;
            let rect = egui::Rect::from_min_size(egui::pos2(screen.right() - w - 16.0, y - 34.0), egui::vec2(w, 32.0));
            painter.rect_filled(rect, 8, theme::alpha(PANEL_HI, 0.96 * a));
            painter.rect_stroke(rect, 8, egui::Stroke::new(1.0, theme::alpha(t.color, 0.7 * a)), egui::StrokeKind::Inside);
            painter.circle_filled(rect.left_center() + egui::vec2(14.0, 0.0), 3.5, theme::alpha(t.color, a));
            painter.galley(rect.left_center() + egui::vec2(26.0, -galley.size().y / 2.0), galley, TEXT);
            y -= 40.0;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_help;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                egui::Grid::new("keys").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                    for (k, d) in [
                        ("Space / K / L", "Play-pause / pause / play"),
                        ("J", "Jump back 2 s"),
                        ("‹ ›  (Shift)", "Step a frame (a second)"),
                        ("⬆ ⬇", "Previous / next edit point"),
                        ("Home / End", "Start / end"),
                        ("I / O / Alt+X", "Mark in / out / clear"),
                        ("S  or  Ctrl+B", "Split at playhead"),
                        ("B / A", "Blade / select tool"),
                        ("Del / Shift+Del", "Delete / ripple delete"),
                        ("M / T", "Add marker / title"),
                        ("N", "Toggle snapping"),
                        ("+ / − / Shift+Z", "Zoom in / out / fit"),
                        ("Ctrl+wheel", "Zoom at mouse"),
                        ("Alt+drag", "Move without linked clip"),
                        ("Ctrl+Z / Ctrl+Shift+Z", "Undo / redo"),
                        ("Ctrl+S / Ctrl+O / Ctrl+I", "Save / open / import"),
                        ("Ctrl+N", "New window"),
                        ("Ctrl+E", "Deliver (export)"),
                    ] {
                        ui.label(RichText::new(k).family(egui::FontFamily::Monospace).color(CYAN));
                        ui.label(d);
                        ui.end_row();
                    }
                });
            });
        self.show_help = open;
    }
}

impl eframe::App for KerfApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
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
            .frame(egui::Frame::NONE.fill(PANEL).stroke(egui::Stroke::new(1.0, LINE)))
            .show(ui, |ui| crate::timeline::show(self, ui));
        egui::Panel::left("left")
            .resizable(true)
            .default_size(270.0)
            .size_range(200.0..=480.0)
            .frame(egui::Frame::NONE.fill(PANEL).inner_margin(egui::Margin::same(10)).stroke(egui::Stroke::new(1.0, LINE)))
            .show(ui, |ui| crate::panels::left_panel(self, ui));
        egui::Panel::right("right")
            .resizable(true)
            .default_size(320.0)
            .size_range(260.0..=520.0)
            .frame(egui::Frame::NONE.fill(PANEL).inner_margin(egui::Margin::same(10)).stroke(egui::Stroke::new(1.0, LINE)))
            .show(ui, |ui| crate::panels::right_panel(self, ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
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
