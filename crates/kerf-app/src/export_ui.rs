//! The Deliver workspace: export presets, a background render job, and
//! rendering on another machine (PC / VPS over Tailscale) through the CLI.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use egui::{vec2, Rect, RichText, Sense};
use kerf_core::{Project, Time, TimeRange};
use kerf_render::{Codec, ExportSettings};

use crate::app::KerfApp;
use crate::theme::{self, *};

#[derive(Default)]
struct Job {
    progress: f32,
    stage: String,
    done: Option<Result<String, String>>,
    cancel: bool,
}

pub struct ExportUi {
    pub settings: ExportSettings,
    pub use_marks: bool,
    pub remote: String,
    job: Option<Arc<Mutex<Job>>>,
    started: Option<Instant>,
    last_result: Option<Result<String, String>>,
}

impl Default for ExportUi {
    fn default() -> Self {
        ExportUi {
            settings: ExportSettings::for_project(&Project::default(), "out.mp4"),
            use_marks: false,
            remote: String::new(),
            job: None,
            started: None,
            last_result: None,
        }
    }
}

impl ExportUi {
    /// Reset the format to the project's and pick an output path next to it.
    pub fn init(&mut self, p: &Project, path: Option<&Path>) {
        let out = path
            .map(|pp| pp.with_extension("mp4"))
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
                home.join("Videos").join(format!("{}.mp4", p.name))
            });
        let codec = self.settings.codec;
        self.settings = ExportSettings::for_project(p, out.to_string_lossy());
        self.settings.codec = codec;
        if codec == Codec::Prores {
            self.settings.output = out.with_extension("mov").to_string_lossy().into_owned();
        }
    }

    pub fn running(&self) -> bool {
        self.job.as_ref().map(|j| j.lock().unwrap().done.is_none()).unwrap_or(false)
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(j) = &self.job {
            let done = j.lock().unwrap().done.clone();
            if let Some(r) = done {
                self.last_result = Some(r);
                self.job = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }

    fn start_local(&mut self, project: Project) {
        let job = Arc::new(Mutex::new(Job::default()));
        let j2 = job.clone();
        let s = self.settings.clone();
        std::thread::Builder::new()
            .name("kerf-export".into())
            .spawn(move || {
                let r = kerf_render::export(&project, &s, |p, stage| {
                    let mut g = j2.lock().unwrap();
                    g.progress = p;
                    if g.stage != stage {
                        g.stage = stage.to_string();
                    }
                    !g.cancel
                });
                j2.lock().unwrap().done = Some(r.map(|_| s.output.clone()).map_err(|e| e.to_string()));
            })
            .unwrap();
        self.job = Some(job);
        self.started = Some(Instant::now());
        self.last_result = None;
    }

    /// Render on another machine via `kerf export --on <host>`.
    fn start_remote(&mut self, project_path: &Path) {
        let job = Arc::new(Mutex::new(Job { stage: format!("rendering on {}", self.remote), ..Default::default() }));
        let j2 = job.clone();
        let s = self.settings.clone();
        let host = self.remote.trim().to_string();
        let proj = project_path.to_path_buf();
        std::thread::spawn(move || {
            let codec = match s.codec {
                Codec::H264 => "h264",
                Codec::H265 => "h265",
                Codec::H264Vaapi => "vaapi",
                Codec::Prores => "prores",
            };
            let mut cmd = std::process::Command::new(kerf_cli_bin());
            cmd.arg("-p").arg(&proj).args(["export", "-o", &s.output, "--codec", codec]);
            cmd.args(["--quality", &s.quality.to_string(), "--width", &s.width.to_string(), "--height", &s.height.to_string()]);
            cmd.args(["--fps", &s.fps.to_string(), "--on", &host]);
            if let Some(r) = s.range {
                cmd.args(["--from", &format!("{}", r.start.as_secs_f64()), "--to", &format!("{}", r.end().as_secs_f64())]);
            }
            cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
            let r = match cmd.spawn() {
                Err(e) => Err(format!("could not run the kerf CLI: {e}")),
                Ok(mut child) => {
                    use std::io::Read;
                    // Progress lines come on stderr as "... NN%".
                    let mut err = child.stderr.take().unwrap();
                    let mut buf = [0u8; 512];
                    let mut log = String::new();
                    loop {
                        if j2.lock().unwrap().cancel {
                            let _ = child.kill();
                            break;
                        }
                        match err.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                                if let Some(pct) = last_percent(&s) {
                                    j2.lock().unwrap().progress = pct / 100.0;
                                }
                                log.push_str(&s);
                                if log.len() > 8000 {
                                    log.drain(..4000);
                                }
                            }
                        }
                    }
                    match child.wait() {
                        Ok(st) if st.success() => Ok(s.output.clone()),
                        _ => Err(log.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("remote export failed").to_string()),
                    }
                }
            };
            j2.lock().unwrap().done = Some(r);
        });
        self.job = Some(job);
        self.started = Some(Instant::now());
        self.last_result = None;
    }
}

fn kerf_cli_bin() -> std::path::PathBuf {
    // Prefer the `kerf` next to this binary (same build), else PATH.
    if let Ok(exe) = std::env::current_exe() {
        let sib = exe.with_file_name("kerf");
        if sib.exists() {
            return sib;
        }
    }
    "kerf".into()
}

fn last_percent(s: &str) -> Option<f32> {
    let i = s.rfind('%')?;
    let num: String = s[..i].chars().rev().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<Vec<_>>().into_iter().rev().collect();
    num.parse().ok()
}

pub fn panel(app: &mut KerfApp, ui: &mut egui::Ui) {
    let tl = app.project().timeline.clone();
    theme::section(ui, "presets");
    let presets: [(&str, u32, u32, f64, Codec, u32); 6] = [
        ("youtube", 1920, 1080, tl.fps, Codec::H264, 20),
        ("reel", 1080, 1920, tl.fps, Codec::H264, 20),
        ("4k master", 3840, 2160, tl.fps, Codec::H265, 22),
        ("fast draft", 1280, 720, tl.fps, Codec::H264, 28),
        ("gpu quick", tl.width, tl.height, tl.fps, Codec::H264Vaapi, 24),
        ("prores master", tl.width, tl.height, tl.fps, Codec::Prores, 0),
    ];
    let e = &mut app.export;
    {
        for (n, w, h, fps, codec, q) in presets {
            let on = e.settings.width == w && e.settings.height == h && e.settings.codec == codec && e.settings.quality == q;
            // `.rows`: label, value, selected one in ink with a left bar.
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
            let pa = ui.painter();
            if resp.hovered() {
                pa.rect_filled(r, 0, pal::hover());
            }
            if on {
                pa.rect_filled(Rect::from_min_size(r.min, vec2(2.0, r.height())), 0, pal::ink());
            }
            let codec_short = match codec {
                Codec::H264 => "h.264",
                Codec::H265 => "h.265",
                Codec::H264Vaapi => "h.264 gpu",
                Codec::Prores => "prores",
            };
            pa.text(r.left_center() + vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, n, theme::mono(FS_SM), if on { pal::ink() } else { pal::body() });
            pa.text(r.right_center() - vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER, format!("{w}×{h} · {codec_short}"), theme::mono(FS_XS), pal::faint());
            if resp.clicked() {
                e.settings.width = w;
                e.settings.height = h;
                e.settings.fps = fps;
                e.settings.codec = codec;
                e.settings.quality = q;
                e.settings.preset = if q >= 28 { "ultrafast".into() } else { "veryfast".into() };
                let ext = if codec == Codec::Prores { "mov" } else { "mp4" };
                e.settings.output = Path::new(&e.settings.output).with_extension(ext).to_string_lossy().into_owned();
            }
        }
    }
    if (e.settings.width as f32 / e.settings.height as f32 - tl.width as f32 / tl.height as f32).abs() > 0.01 {
        ui.label(
            RichText::new("output aspect differs from the timeline: the frame gets bars. change timeline › format to recompose.")
                .size(11.0)
                .color(pal::warn()),
        );
    }

    theme::section(ui, "video");
    egui::Grid::new("exp").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label(RichText::new("codec").color(pal::dim()));
        egui::ComboBox::from_id_salt("codec").selected_text(e.settings.codec.label()).show_ui(ui, |ui| {
            for c in Codec::ALL {
                ui.selectable_value(&mut e.settings.codec, c, c.label());
            }
        });
        ui.end_row();
        ui.label(RichText::new("size").color(pal::dim()));
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut e.settings.width).range(16..=7680));
            ui.label("×");
            ui.add(egui::DragValue::new(&mut e.settings.height).range(16..=4320));
        });
        ui.end_row();
        ui.label(RichText::new("frame rate").color(pal::dim()));
        ui.add(egui::DragValue::new(&mut e.settings.fps).range(1.0..=120.0).speed(0.1));
        ui.end_row();
        if e.settings.codec != Codec::Prores {
            ui.label(RichText::new("quality").color(pal::dim()));
            ui.add(egui::Slider::new(&mut e.settings.quality, 14..=36).text("crf").clamping(egui::SliderClamping::Always));
            ui.end_row();
        }
        if matches!(e.settings.codec, Codec::H264 | Codec::H265) {
            ui.label(RichText::new("speed").color(pal::dim()));
            egui::ComboBox::from_id_salt("preset").selected_text(e.settings.preset.clone()).show_ui(ui, |ui| {
                for p in ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow"] {
                    ui.selectable_value(&mut e.settings.preset, p.to_string(), p);
                }
            });
            ui.end_row();
        }
        ui.label(RichText::new("audio").color(pal::dim()));
        ui.add(egui::DragValue::new(&mut e.settings.audio_kbps).range(64..=320).suffix(" kbps"));
        ui.end_row();
    });

    theme::section(ui, "range");
    let has_marks = matches!((app.mark_in, app.mark_out), (Some(a), Some(b)) if b > a);
    ui.add_enabled(has_marks, egui::Checkbox::new(&mut e.use_marks, "only in → out  (set with i / o)"));
    e.settings.range = if e.use_marks && has_marks {
        Some(TimeRange::from_bounds(app.mark_in.unwrap(), app.mark_out.unwrap()))
    } else {
        None
    };
    let dur = e.settings.range.map(|r| r.duration).unwrap_or(tl.duration());
    ui.label(RichText::new(format!("{} · {} frames", dur.timecode(tl.fps), (dur.as_secs_f64() * e.settings.fps).round())).family(egui::FontFamily::Monospace).color(pal::dim()));

    theme::section(ui, "output");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut e.settings.output).desired_width(ui.available_width() - 36.0));
        if ui.button("…").clicked() {
            let ext = if e.settings.codec == Codec::Prores { "mov" } else { "mp4" };
            if let Some(p) = rfd::FileDialog::new().add_filter("video", &[ext]).set_file_name(
                Path::new(&e.settings.output).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
            ).save_file() {
                e.settings.output = p.to_string_lossy().into_owned();
            }
        }
    });

    theme::section(ui, "render on");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut e.remote).hint_text("this machine  (or an ssh host: pc, vps)").desired_width(ui.available_width()));
    });
    ui.label(
        RichText::new("any ssh/tailscale host with kerf installed. media is synced once and cached there.")
            .size(10.5)
            .color(pal::faint()),
    );

    ui.add_space(12.0);
    let running = e.running();
    let empty = dur <= Time::ZERO;
    if let Some(job) = e.job.clone() {
        let (p, stage) = {
            let g = job.lock().unwrap();
            (g.progress, g.stage.clone())
        };
        // `.meter`: a 4 px bar, info while in progress.
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
        let pa = ui.painter();
        pa.rect_filled(r, 0, pal::hover());
        let fill = Rect::from_min_size(r.min, vec2(r.width() * p.clamp(0.0, 1.0), r.height()));
        pa.rect_filled(fill, 0, pal::info());
        ui.add_space(4.0);
        let el = e.started.map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0);
        let eta = if p > 0.02 { el / p - el } else { 0.0 };
        ui.label(RichText::new(format!("{stage} · {:.0}% · {:.0}s · ~{:.0}s left", p * 100.0, el, eta)).font(theme::mono(FS_XS)).color(pal::dim()));
        if ui.button("cancel").clicked() {
            job.lock().unwrap().cancel = true;
        }
    } else {
        let label = if e.remote.trim().is_empty() { "render" } else { "render remotely" };
        let w = ui.available_width();
        let clicked = ui.add_enabled_ui(!running && !empty, |ui| theme::primary_button(ui, label, w)).inner.clicked();
        if clicked {
            if e.remote.trim().is_empty() {
                let p = app.project().clone();
                app.export.start_local(p);
            } else {
                app.save();
                if let Some(path) = app.path.clone() {
                    app.export.start_remote(&path);
                }
            }
        }
    }
    let e = &mut app.export;
    if let Some(r) = &e.last_result {
        ui.add_space(6.0);
        match r {
            Ok(path) => {
                ui.label(RichText::new(format!("rendered {path}")).color(pal::ok()));
                let path = path.clone();
                ui.horizontal(|ui| {
                    if ui.button("open").clicked() {
                        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                    }
                    if ui.button("show folder").clicked() {
                        if let Some(d) = Path::new(&path).parent() {
                            let _ = std::process::Command::new("xdg-open").arg(d).spawn();
                        }
                    }
                });
            }
            Err(err) => {
                ui.label(RichText::new(format!("{err}")).color(pal::bad()));
            }
        }
    }
}
