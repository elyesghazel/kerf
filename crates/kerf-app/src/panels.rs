//! Side panels: media pool / titles / markers on the left, the inspector
//! (edit, color, audio, deliver) on the right.

use std::ops::RangeInclusive;

use egui::{pos2, vec2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind};
use kerf_core::{Clip, ClipContent, ClipId, Command, MediaId, TextStyle, Time, TrackKind};

use crate::app::{KerfApp, LeftTab, Workspace};
use crate::theme::{self, *};

// ====================================================================== left

pub fn left_panel(app: &mut KerfApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for (t, n) in [(LeftTab::Media, "media"), (LeftTab::Titles, "titles"), (LeftTab::Markers, "markers")] {
            if theme::pill(ui, app.left_tab == t, n).clicked() {
                app.left_tab = t;
            }
        }
    });
    ui.add_space(8.0);
    match app.left_tab {
        LeftTab::Media => media_pool(app, ui),
        LeftTab::Titles => titles(app, ui),
        LeftTab::Markers => markers(app, ui),
    }
}

fn media_pool(app: &mut KerfApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if ui.button("import").on_hover_text("Ctrl+I, or drop files on the window").clicked() {
            app.import_dialog();
        }
        let need: Vec<_> = app.project().media.iter().filter(|m| m.has_video && !m.is_image && m.proxy.is_none()).cloned().collect();
        if !need.is_empty()
            && ui
                .button("make proxies")
                .on_hover_text("make 540p proxies of all videos: much smoother scrubbing on a laptop")
                .clicked()
        {
            for m in need {
                app.assets.make_proxy(&m);
            }
        }
    });
    ui.add_space(6.0);
    let media = app.project().media.clone();
    if media.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("// --").font(theme::mono(FS_XS)).color(pal::faint()));
            ui.label(RichText::new("no media yet").color(pal::ink()));
            ui.label(RichText::new("drop videos, audio or images\nanywhere on the window").font(theme::mono(FS_XS)).color(pal::faint()));
        });
        return;
    }
    let used: std::collections::HashSet<MediaId> = app.project().timeline.clips().filter_map(|(_, c)| c.media()).collect();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let cols = 2;
        let gap = 8.0;
        let w = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).floor();
        let thumb_h = (w * 9.0 / 16.0).round();
        let card_h = thumb_h + 38.0;
        for row in media.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for m in row {
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, card_h), Sense::click_and_drag());
                    resp.dnd_set_drag_payload(m.id);
                    let hot = resp.hovered() || resp.dragged();
                    let p = ui.painter();
                    p.rect_filled(rect, R_CARD, pal::raised());
                    p.rect_stroke(rect, R_CARD, Stroke::new(1.0, if hot { pal::rule_strong() } else { pal::rule() }), StrokeKind::Inside);
                    let tr = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(w - 2.0, thumb_h - 1.0));
                    p.rect_filled(tr, 0, pal::sunken());
                    if m.has_video {
                        let idx = app.assets.thumbs(m.id).map(|t| t.frames.len() / 3).unwrap_or(0);
                        if let Some((tex, sz)) = app.assets.thumb_texture(ui.ctx(), m.id, idx) {
                            let fit = (tr.width() / sz.x).min(tr.height() / sz.y);
                            let ir = Rect::from_center_size(tr.center(), sz * fit);
                            p.image(tex.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                    } else {
                        // Audio-only: draw its waveform.
                        if let Some(wv) = app.assets.wave(m.id) {
                            let n = (tr.width() / 2.0) as usize;
                            let dur = m.duration.as_secs_f64();
                            for i in 0..n {
                                let a = dur * i as f64 / n as f64;
                                let b = dur * (i + 1) as f64 / n as f64;
                                let h = wv.peak(a, b) * tr.height() * 0.42;
                                let x = tr.left() + i as f32 * 2.0 + 1.0;
                                p.line_segment([pos2(x, tr.center().y - h), pos2(x, tr.center().y + h)], Stroke::new(1.2, theme::alpha(pal::ok(), 0.8)));
                            }
                        } else {
                            p.text(tr.center(), egui::Align2::CENTER_CENTER, "audio", theme::mono(FS_XS), pal::faint());
                        }
                    }
                    // Duration badge.
                    if !m.is_image {
                        let secs = m.duration.as_secs_f64().round() as i64;
                        let d = if secs >= 3600 {
                            format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
                        } else {
                            format!("{}:{:02}", secs / 60, secs % 60)
                        };
                        let g = p.layout_no_wrap(d, theme::mono(FS_2XS), Color32::from_gray(230));
                        let br = Rect::from_min_size(tr.right_bottom() - g.size() - vec2(10.0, 6.0), g.size() + vec2(6.0, 2.0));
                        p.rect_filled(br, R_CTRL, theme::alpha(Color32::BLACK, 0.7));
                        p.galley(br.min + vec2(3.0, 1.0), g, Color32::from_gray(230));
                    }
                    // Proxy state.
                    if let Some(pr) = app.assets.proxy_progress(m.id) {
                        let bar = Rect::from_min_size(pos2(tr.left(), tr.bottom() - 3.0), vec2(tr.width() * pr, 3.0));
                        p.rect_filled(bar, 1, pal::ink());
                        ui.ctx().request_repaint();
                    } else if m.proxy.is_some() {
                        let g = p.layout_no_wrap("proxy".into(), theme::mono(FS_2XS), Color32::from_gray(230));
                        let br = Rect::from_min_size(tr.left_top() + vec2(5.0, 5.0), g.size() + vec2(8.0, 2.0));
                        p.rect_filled(br, R_CTRL, theme::alpha(Color32::BLACK, 0.7));
                        p.galley(br.min + vec2(4.0, 1.0), g, Color32::from_gray(230));
                    }
                    let name: String = m.name.chars().take(28).collect();
                    p.text(
                        pos2(rect.left() + 8.0, tr.bottom() + 6.0),
                        egui::Align2::LEFT_TOP,
                        name,
                        theme::prop(11.5),
                        pal::body(),
                    );
                    let kind = match (m.is_image, m.has_video, m.has_audio) {
                        (true, ..) => "image".to_string(),
                        (_, true, true) => format!("{}×{} · a/v", m.width, m.height),
                        (_, true, false) => format!("{}×{} · video", m.width, m.height),
                        _ => "audio".to_string(),
                    };
                    p.text(
                        pos2(rect.left() + 8.0, tr.bottom() + 21.0),
                        egui::Align2::LEFT_TOP,
                        kind,
                        theme::prop(9.5),
                        if used.contains(&m.id) { pal::ink() } else { pal::faint() },
                    );
                    p.line_segment([tr.left_bottom(), tr.right_bottom()], Stroke::new(1.0, pal::rule()));
                    if resp.dragged() {
                        if let Some(pp) = ui.ctx().pointer_interact_pos() {
                            let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("dnd")));
                            let r = Rect::from_min_size(pp + vec2(10.0, 10.0), vec2(120.0, 26.0));
                            lp.rect_filled(r, 6, theme::alpha(pal::raised(), 0.95));
                            lp.rect_stroke(r, 6, Stroke::new(1.0, pal::ink()), StrokeKind::Inside);
                            lp.text(r.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, format!("+ {}", m.name.chars().take(14).collect::<String>()), theme::prop(11.0), pal::body());
                        }
                    }
                    if resp.double_clicked() {
                        let id = m.id;
                        if let Some(ids) = app.edit(|d| d.append(id)) {
                            app.selection = ids;
                        }
                    }
                    let resp = resp.on_hover_text(format!("{}\n{}\ndrag to the timeline · double-click to append", m.name, m.path));
                    let m2 = m.clone();
                    resp.context_menu(|ui| {
                        if ui.button("append to timeline").clicked() {
                            if let Some(ids) = app.edit(|d| d.append(m2.id)) {
                                app.selection = ids;
                            }
                            ui.close();
                        }
                        if ui.button("insert at playhead").clicked() {
                            let t = app.playhead;
                            app.edit(|d| d.place(m2.id, t, Time::ZERO, None, None, None));
                            ui.close();
                        }
                        if m2.has_video && !m2.is_image && ui.button("generate proxy").clicked() {
                            app.assets.make_proxy(&m2);
                            ui.close();
                        }
                        if ui.button("copy path").clicked() {
                            ui.ctx().copy_text(m2.path.clone());
                            ui.close();
                        }
                        if ui.button("remove from project").clicked() {
                            app.edit(|d| d.apply(Command::RemoveMedia { id: m2.id }));
                            ui.close();
                        }
                    });
                }
            });
            ui.add_space(gap);
        }
    });
}

fn titles(app: &mut KerfApp, ui: &mut egui::Ui) {
    ui.label(RichText::new("click to add at the playhead").size(11.0).color(pal::dim()));
    ui.add_space(6.0);
    let presets: Vec<(&str, TextStyle)> = vec![
        ("big title", TextStyle { size: 0.13, bold: true, y: 0.5, ..TextStyle::new("TITLE") }),
        ("lower third", TextStyle { size: 0.055, bold: true, x: 0.5, y: 0.84, background: Some([8, 10, 16, 190]), ..TextStyle::new("Name Surname") }),
        ("caption", TextStyle { size: 0.05, y: 0.9, background: Some([0, 0, 0, 150]), ..TextStyle::new("Caption text") }),
        ("reel hook", TextStyle { size: 0.075, bold: true, y: 0.3, color: [255, 255, 255, 255], ..TextStyle::new("wait for it…") }),
        ("centered", TextStyle { size: 0.1, bold: true, y: 0.5, ..TextStyle::new("kerf") }),
        ("corner tag", TextStyle { size: 0.04, bold: true, x: 0.88, y: 0.08, background: Some([196, 112, 112, 230]), ..TextStyle::new("new") }),
    ];
    for (name, style) in presets {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
        let p = ui.painter();
        p.rect_filled(rect, R_CARD, if resp.hovered() { pal::hover() } else { pal::raised() });
        p.rect_stroke(rect, R_CARD, Stroke::new(1.0, if resp.hovered() { pal::rule_strong() } else { pal::rule() }), StrokeKind::Inside);
        let sw = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(60.0, 34.0));
        p.rect_filled(sw, 5, pal::bg());
        let c = Color32::from_rgba_unmultiplied(style.color[0], style.color[1], style.color[2], 255);
        if let Some(bg) = style.background {
            let br = Rect::from_center_size(pos2(sw.left() + sw.width() * style.x, sw.top() + sw.height() * style.y), vec2(30.0, 7.0));
            p.rect_filled(br, 1, Color32::from_rgba_unmultiplied(bg[0], bg[1], bg[2], bg[3]));
        }
        p.text(
            pos2(sw.left() + sw.width() * style.x, sw.top() + sw.height() * style.y),
            egui::Align2::CENTER_CENTER,
            "Aa",
            theme::prop((style.size * 90.0).clamp(7.0, 14.0)),
            c,
        );
        p.text(rect.left_center() + vec2(78.0, 0.0), egui::Align2::LEFT_CENTER, name, theme::prop(12.5), pal::body());
        let nudge = if resp.hovered() { 3.0 } else { 0.0 };
        p.text(rect.right_center() - vec2(14.0 - nudge, 0.0), egui::Align2::RIGHT_CENTER, "→", theme::mono(FS_SM), if resp.hovered() { pal::ink() } else { pal::faint() });
        if resp.clicked() {
            let t = app.playhead;
            if let Some(id) = app.edit(|d| d.add_text(style.clone(), t, Time::from_secs_f64(3.0))) {
                app.selection = vec![id];
                app.workspace = Workspace::Edit;
            }
        }
        ui.add_space(4.0);
    }
    ui.add_space(8.0);
    ui.label(RichText::new("solid mattes").size(11.0).color(pal::dim()));
    ui.horizontal(|ui| {
        for (n, c) in [("black", [0, 0, 0, 255]), ("white", [255, 255, 255, 255]), ("night", [8, 10, 18, 255])] {
            if ui.button(n).clicked() {
                let t = app.playhead;
                app.edit(|d| {
                    let (id, idx, name) = d.new_track_slot(TrackKind::Video);
                    let mut clip = Clip::new(d.ids().clip(), ClipContent::Solid { color: c }, t, Time::ZERO, Time::from_secs_f64(3.0));
                    clip.name = format!("{n} matte");
                    // Mattes go on a fresh bottom track so they sit behind.
                    let _ = idx;
                    d.apply_all(vec![
                        Command::AddTrack { track: kerf_core::Track::new(id, TrackKind::Video, name), index: 0 },
                        Command::AddClip { track: id, clip },
                    ])
                });
            }
        }
    });
}

fn markers(app: &mut KerfApp, ui: &mut egui::Ui) {
    if ui.button("add at playhead  (m)").clicked() {
        let t = app.playhead;
        let n = app.project().timeline.markers.len() + 1;
        app.edit(|d| d.add_marker(t, format!("Marker {n}")));
    }
    ui.add_space(6.0);
    let fps = app.project().timeline.fps;
    let ms = app.project().timeline.markers.clone();
    if ms.is_empty() {
        ui.label(RichText::new("No markers yet. Agents can drop them too:\nkerf marker add --at 12.5 \"best moment\"").size(11.0).color(pal::faint()));
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for m in ms {
            ui.horizontal(|ui| {
                let c = Color32::from_rgb(m.color[0], m.color[1], m.color[2]);
                ui.label(RichText::new("◆").color(c));
                if ui
                    .add(egui::Button::new(RichText::new(m.time.timecode(fps)).family(egui::FontFamily::Monospace).color(pal::ink())).frame(false))
                    .clicked()
                {
                    app.seek(m.time);
                }
                let mut label = m.label.clone();
                let r = ui.add(egui::TextEdit::singleline(&mut label).desired_width(ui.available_width() - 30.0));
                if r.lost_focus() && label != m.label {
                    let mut m2 = m.clone();
                    m2.label = label;
                    app.edit(|d| d.apply_all(vec![Command::RemoveMarker { id: m.id }, Command::AddMarker { marker: m2 }]));
                }
                if ui.small_button("×").clicked() {
                    app.edit(|d| d.apply(Command::RemoveMarker { id: m.id }));
                }
            });
        }
    });
}

// ===================================================================== right

pub fn right_panel(app: &mut KerfApp, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match app.workspace {
        Workspace::Edit => edit_inspector(app, ui),
        Workspace::Color => color_panel(app, ui),
        Workspace::Audio => audio_panel(app, ui),
        Workspace::Deliver => crate::export_ui::panel(app, ui),
    });
    // A released slider ends its undo gesture.
    if ui.input(|i| i.pointer.any_released()) {
        app.doc.end_merge();
    }
}

/// The clip the inspector edits: the first selected one, preferring video.
fn focus_clip(app: &KerfApp) -> Option<Clip> {
    let tl = &app.project().timeline;
    let mut best: Option<Clip> = None;
    for id in &app.selection {
        if let Some((t, c)) = tl.find_clip(*id) {
            let video = tl.track(t).map(|t| t.kind == TrackKind::Video).unwrap_or(false);
            if video {
                return Some(c.clone());
            }
            best.get_or_insert(c.clone());
        }
    }
    best
}

fn slider(
    app: &mut KerfApp,
    ui: &mut egui::Ui,
    id: ClipId,
    key: &str,
    label: &str,
    v: f32,
    range: RangeInclusive<f32>,
    default: f32,
    set: impl Fn(&mut Clip, f32),
) {
    ui.horizontal(|ui| {
        let (lr, _) = ui.allocate_exact_size(vec2(96.0, 20.0), Sense::hover());
        ui.painter().text(lr.left_center(), egui::Align2::LEFT_CENTER, label, theme::mono(FS_XS), pal::dim());
        let mut x = v;
        ui.spacing_mut().slider_width = (ui.available_width() - 64.0 - 24.0).max(50.0);
        let r = ui.add(egui::Slider::new(&mut x, range).max_decimals(2));
        if r.changed() {
            app.edit_clip_live(id, key, |c| set(c, x));
        }
        if (v - default).abs() > 1e-4 && ui.add(egui::Button::new(RichText::new("×").color(pal::faint())).frame(false)).on_hover_text("reset").clicked() {
            app.edit(|d| d.edit_clip(id, |c| set(c, default)));
        }
    });
}

fn edit_inspector(app: &mut KerfApp, ui: &mut egui::Ui) {
    let Some(clip) = focus_clip(app) else {
        project_info(app, ui);
        return;
    };
    let id = clip.id;
    let fps = app.project().timeline.fps;
    let kind = match &clip.content {
        ClipContent::Media { .. } => "media",
        ClipContent::Text(_) => "title",
        ClipContent::Solid { .. } => "matte",
    };
    ui.horizontal(|ui| {
        theme::tag(ui, &kind.to_lowercase(), None);
        let mut name = clip.name.clone();
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(ui.available_width()).font(theme::prop(14.0)));
        if r.changed() {
            app.edit_clip_live(id, "name", |c| c.name = name.clone());
        }
    });

    theme::section(ui, "timing");
    egui::Grid::new("timing").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        let row = |ui: &mut egui::Ui, k: &str, v: String| {
            ui.label(RichText::new(k).size(11.5).color(pal::dim()));
            ui.label(RichText::new(v).family(egui::FontFamily::Monospace).color(pal::body()));
            ui.end_row();
        };
        row(ui, "start", clip.start.timecode(fps));
        row(ui, "end", clip.end().timecode(fps));
        row(ui, "duration", clip.duration.timecode(fps));
        if clip.media().is_some() {
            row(ui, "source in", clip.source_in.timecode(fps));
        }
    });

    if let ClipContent::Text(style) = &clip.content {
        theme::section(ui, "title");
        let mut text = style.text.clone();
        let r = ui.add(egui::TextEdit::multiline(&mut text).desired_rows(3).desired_width(f32::INFINITY));
        if r.changed() {
            app.edit_clip_live(id, "text", |c| {
                if let ClipContent::Text(s) = &mut c.content {
                    s.text = text.clone();
                }
            });
        }
        let s = style.clone();
        let set = |f: fn(&mut TextStyle, f32)| {
            move |c: &mut Clip, v: f32| {
                if let ClipContent::Text(s) = &mut c.content {
                    f(s, v)
                }
            }
        };
        slider(app, ui, id, "tsize", "size", s.size, 0.01..=0.4, 0.08, set(|s, v| s.size = v));
        slider(app, ui, id, "tx", "position x", s.x, 0.0..=1.0, 0.5, set(|s, v| s.x = v));
        slider(app, ui, id, "ty", "position y", s.y, 0.0..=1.0, 0.82, set(|s, v| s.y = v));
        ui.horizontal(|ui| {
            let mut col = s.color;
            ui.label(RichText::new("color").size(11.5).color(pal::dim()));
            if ui.color_edit_button_srgba_unmultiplied(&mut col).changed() {
                app.edit_clip_live(id, "tcolor", |c| {
                    if let ClipContent::Text(s) = &mut c.content {
                        s.color = col
                    }
                });
            }
            let mut has_bg = s.background.is_some();
            if ui.checkbox(&mut has_bg, "box").changed() {
                app.edit(|d| {
                    d.edit_clip(id, |c| {
                        if let ClipContent::Text(s) = &mut c.content {
                            s.background = if has_bg { Some([0, 0, 0, 170]) } else { None }
                        }
                    })
                });
            }
            if let Some(mut bg) = s.background {
                if ui.color_edit_button_srgba_unmultiplied(&mut bg).changed() {
                    app.edit_clip_live(id, "tbg", |c| {
                        if let ClipContent::Text(s) = &mut c.content {
                            s.background = Some(bg)
                        }
                    });
                }
            }
        });
        ui.horizontal(|ui| {
            let (mut b, mut sh) = (s.bold, s.shadow);
            if ui.checkbox(&mut b, "bold").changed() || ui.checkbox(&mut sh, "shadow").changed() {
                app.edit(|d| {
                    d.edit_clip(id, |c| {
                        if let ClipContent::Text(s) = &mut c.content {
                            s.bold = b;
                            s.shadow = sh;
                        }
                    })
                });
            }
        });
    }

    if let ClipContent::Solid { color } = clip.content {
        theme::section(ui, "matte");
        let mut col = color;
        if ui.color_edit_button_srgba_unmultiplied(&mut col).changed() {
            app.edit_clip_live(id, "solid", |c| c.content = ClipContent::Solid { color: col });
        }
    }

    let is_video = app
        .project()
        .timeline
        .find_clip(id)
        .and_then(|(t, _)| app.project().timeline.track(t))
        .map(|t| t.kind == TrackKind::Video)
        .unwrap_or(false);
    if is_video {
        theme::section(ui, "transform");
        let tf = clip.transform;
        slider(app, ui, id, "scale", "zoom", tf.scale, 0.1..=4.0, 1.0, |c, v| c.transform.scale = v);
        slider(app, ui, id, "posx", "position x", tf.x, -1.0..=1.0, 0.0, |c, v| c.transform.x = v);
        slider(app, ui, id, "posy", "position y", tf.y, -1.0..=1.0, 0.0, |c, v| c.transform.y = v);
        slider(app, ui, id, "rot", "rotation", tf.rotation, -180.0..=180.0, 0.0, |c, v| c.transform.rotation = v);
        if clip.media().is_some() {
            slider(app, ui, id, "cl", "crop left", tf.crop_left, 0.0..=0.5, 0.0, |c, v| c.transform.crop_left = v);
            slider(app, ui, id, "cr", "crop right", tf.crop_right, 0.0..=0.5, 0.0, |c, v| c.transform.crop_right = v);
            slider(app, ui, id, "ct", "crop top", tf.crop_top, 0.0..=0.5, 0.0, |c, v| c.transform.crop_top = v);
            slider(app, ui, id, "cb", "crop bottom", tf.crop_bottom, 0.0..=0.5, 0.0, |c, v| c.transform.crop_bottom = v);
            ui.horizontal(|ui| {
                let mut f = tf.flip_h;
                if ui.checkbox(&mut f, "flip horizontal").changed() {
                    app.edit(|d| d.edit_clip(id, |c| c.transform.flip_h = f));
                }
                if ui.button("fill frame").on_hover_text("zoom so the clip covers the whole frame (landscape to vertical)").clicked() {
                    if let Some(item) = clip.media().and_then(|m| app.project().media_item(m)) {
                        let tl = &app.project().timeline;
                        let (fw, fh) = kerf_render::fit(item.width, item.height, tl.width, tl.height);
                        let s = (tl.width as f32 / fw as f32).max(tl.height as f32 / fh as f32);
                        app.edit(|d| d.edit_clip(id, |c| c.transform.scale = s));
                    }
                }
            });
        }
        theme::section(ui, "compositing");
        slider(app, ui, id, "opacity", "opacity", clip.opacity, 0.0..=1.0, 1.0, |c, v| c.opacity = v);
    }
    theme::section(ui, "fades");
    let max = (clip.duration.as_secs_f64() / 2.0) as f32;
    slider(app, ui, id, "fin", "fade in (s)", clip.fade_in.as_secs_f64() as f32, 0.0..=max, 0.0, |c, v| c.fade_in = Time::from_secs_f64(v as f64));
    slider(app, ui, id, "fout", "fade out (s)", clip.fade_out.as_secs_f64() as f32, 0.0..=max, 0.0, |c, v| c.fade_out = Time::from_secs_f64(v as f64));
    if !is_video {
        theme::section(ui, "audio");
        volume_slider(app, ui, id, clip.volume);
    }
}

fn volume_slider(app: &mut KerfApp, ui: &mut egui::Ui, id: ClipId, volume: f32) {
    let db = 20.0 * volume.max(1e-4).log10();
    slider(app, ui, id, "vol", "volume (db)", db.max(-60.0), -60.0..=12.0, 0.0, |c, v| {
        c.volume = if v <= -59.9 { 0.0 } else { 10f32.powf(v / 20.0) }
    });
}

fn project_info(app: &mut KerfApp, ui: &mut egui::Ui) {
    theme::section(ui, "project");
    let mut name = app.project().name.clone();
    if ui.add(egui::TextEdit::singleline(&mut name).font(theme::prop(15.0)).desired_width(f32::INFINITY)).lost_focus()
        && name != app.project().name
    {
        app.edit(|d| d.apply(Command::Rename { name }));
    }
    let tl = app.project().timeline.clone();
    ui.label(
        RichText::new(format!(
            "{}×{} · {} fps · {}",
            tl.width,
            tl.height,
            tl.fps,
            tl.duration().timecode(tl.fps)
        ))
        .family(egui::FontFamily::Monospace)
        .color(pal::dim()),
    );
    ui.horizontal(|ui| {
        ui.label(RichText::new("background").size(11.5).color(pal::dim()));
        let mut bg = tl.background;
        if ui.color_edit_button_srgb(&mut bg).changed() {
            app.edit(|d| d.apply(Command::SetFormat { width: tl.width, height: tl.height, fps: tl.fps, background: bg }));
        }
    });
    theme::section(ui, "selection");
    ui.label(RichText::new("click a clip to inspect it. shift-click adds, alt-click picks one side of a linked a/v pair.").size(11.5).color(pal::faint()));
    theme::section(ui, "agents");
    let p = app.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "(save first)".into());
    ui.label(RichText::new("Agents cut this project with the kerf CLI or MCP server; edits show up here live:").size(11.5).color(pal::dim()));
    let cmd = format!("kerf -p {p} info");
    ui.horizontal(|ui| {
        ui.label(RichText::new(&cmd).family(egui::FontFamily::Monospace).size(11.0).color(pal::info()));
        if ui.small_button("copy").clicked() {
            ui.ctx().copy_text(cmd.clone());
        }
    });
}

// ===================================================================== color

fn color_panel(app: &mut KerfApp, ui: &mut egui::Ui) {
    theme::section(ui, "scopes");
    scopes(app, ui);
    let Some(clip) = focus_clip(app) else {
        ui.add_space(10.0);
        ui.label(RichText::new("select a video clip to grade it.").color(pal::dim()));
        return;
    };
    let id = clip.id;
    let g = clip.grade;
    theme::section(ui, "looks");
    ui.horizontal_wrapped(|ui| {
        let looks: [(&str, kerf_core::ColorGrade); 7] = [
            ("neutral", kerf_core::ColorGrade::default()),
            ("punchy", kerf_core::ColorGrade { contrast: 1.18, saturation: 1.25, ..Default::default() }),
            ("warm", kerf_core::ColorGrade { temperature: 0.45, saturation: 1.08, ..Default::default() }),
            ("cool", kerf_core::ColorGrade { temperature: -0.4, tint: 0.05, ..Default::default() }),
            ("faded", kerf_core::ColorGrade { contrast: 0.82, brightness: 0.06, saturation: 0.85, ..Default::default() }),
            ("mono", kerf_core::ColorGrade { saturation: 0.0, contrast: 1.15, ..Default::default() }),
            ("cinematic", kerf_core::ColorGrade { contrast: 1.12, saturation: 0.9, temperature: 0.12, vignette: 0.35, gamma: 0.95, ..Default::default() }),
        ];
        for (n, look) in looks {
            if theme::pill(ui, g == look, n).clicked() {
                let sel = app.selection.clone();
                app.edit(|d| {
                    let mut cmds = vec![];
                    for cid in sel {
                        if let Some((t, c)) = d.project().timeline.find_clip(cid) {
                            if d.project().timeline.track(t).map(|t| t.kind == TrackKind::Video).unwrap_or(false) {
                                let mut c = c.clone();
                                c.grade = look;
                                cmds.push(Command::UpdateClip { track: t, clip: c });
                            }
                        }
                    }
                    d.apply_all(cmds)
                });
            }
        }
    });
    theme::section(ui, "primaries");
    slider(app, ui, id, "exp", "exposure", g.exposure, -3.0..=3.0, 0.0, |c, v| c.grade.exposure = v);
    slider(app, ui, id, "con", "contrast", g.contrast, 0.0..=2.0, 1.0, |c, v| c.grade.contrast = v);
    slider(app, ui, id, "bri", "lift", g.brightness, -1.0..=1.0, 0.0, |c, v| c.grade.brightness = v);
    slider(app, ui, id, "gam", "gamma", g.gamma, 0.2..=3.0, 1.0, |c, v| c.grade.gamma = v);
    slider(app, ui, id, "sat", "saturation", g.saturation, 0.0..=2.0, 1.0, |c, v| c.grade.saturation = v);
    theme::section(ui, "balance");
    slider(app, ui, id, "temp", "temperature", g.temperature, -1.0..=1.0, 0.0, |c, v| c.grade.temperature = v);
    slider(app, ui, id, "tint", "tint", g.tint, -1.0..=1.0, 0.0, |c, v| c.grade.tint = v);
    theme::section(ui, "finishing");
    slider(app, ui, id, "vig", "vignette", g.vignette, 0.0..=1.0, 0.0, |c, v| c.grade.vignette = v);
    ui.add_space(8.0);
    if app.selection.len() > 1 && ui.button("copy this grade to all selected").clicked() {
        let sel = app.selection.clone();
        app.edit(|d| {
            let mut cmds = vec![];
            for cid in sel {
                if let Some((t, c)) = d.project().timeline.find_clip(cid) {
                    let mut c = c.clone();
                    c.grade = g;
                    cmds.push(Command::UpdateClip { track: t, clip: c });
                }
            }
            d.apply_all(cmds)
        });
    }
}

/// RGB parade-style histogram plus a luma waveform of the preview frame.
fn scopes(app: &mut KerfApp, ui: &mut egui::Ui) {
    let w = ui.available_width();
    let (hr, _) = ui.allocate_exact_size(vec2(w, 96.0), Sense::hover());
    let (wr, _) = ui.allocate_exact_size(vec2(w, 110.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(hr, R_CARD, pal::sunken());
    p.rect_filled(wr, R_CARD, pal::sunken());
    p.rect_stroke(hr, R_CARD, Stroke::new(1.0, pal::rule()), StrokeKind::Inside);
    p.rect_stroke(wr, R_CARD, Stroke::new(1.0, pal::rule()), StrokeKind::Inside);
    let Some(f) = app.preview_frame.clone() else { return };
    let mut hist = [[0u32; 64]; 4];
    const WC: usize = 96;
    const WR: usize = 48;
    let mut wave = vec![0u16; WC * WR];
    let step = ((f.w * f.h) as usize / 60_000).max(1);
    for (i, px) in f.data.chunks_exact(4).enumerate().step_by(step) {
        let (r, g, b) = (px[0] as usize, px[1] as usize, px[2] as usize);
        let l = (r * 54 + g * 183 + b * 19) >> 8;
        hist[0][r >> 2] += 1;
        hist[1][g >> 2] += 1;
        hist[2][b >> 2] += 1;
        hist[3][l >> 2] += 1;
        let x = (i % f.w as usize) * WC / f.w as usize;
        let y = WR - 1 - (l * WR / 256);
        wave[y * WC + x] = wave[y * WC + x].saturating_add(1);
    }
    let maxh = hist.iter().flat_map(|h| h.iter()).copied().max().unwrap_or(1).max(1) as f32;
    let inner = hr.shrink(6.0);
    for (ch, col) in [(0, pal::bad()), (1, pal::ok()), (2, pal::info()), (3, pal::ink())] {
        let pts: Vec<egui::Pos2> = (0..64)
            .map(|i| {
                let x = inner.left() + inner.width() * i as f32 / 63.0;
                let v = (hist[ch][i] as f32 / maxh).sqrt();
                pos2(x, inner.bottom() - v * inner.height())
            })
            .collect();
        let a = if ch == 3 { 0.5 } else { 0.85 };
        p.add(egui::Shape::line(pts, Stroke::new(1.3, theme::alpha(col, a))));
    }
    p.text(hr.left_top() + vec2(8.0, 5.0), egui::Align2::LEFT_TOP, "histogram", theme::mono(FS_2XS), pal::faint());
    let maxw = wave.iter().copied().max().unwrap_or(1).max(1) as f32;
    let inner = wr.shrink(6.0);
    let cw = inner.width() / WC as f32;
    let ch = inner.height() / WR as f32;
    for y in 0..WR {
        for x in 0..WC {
            let v = wave[y * WC + x];
            if v == 0 {
                continue;
            }
            let a = (v as f32 / maxw).sqrt().clamp(0.08, 1.0);
            let r = Rect::from_min_size(pos2(inner.left() + x as f32 * cw, inner.top() + y as f32 * ch), vec2(cw + 0.5, ch + 0.5));
            p.rect_filled(r, 0, theme::alpha(pal::body(), a * 0.85));
        }
    }
    for lvl in [0.0, 0.5, 1.0] {
        let y = inner.bottom() - inner.height() * lvl;
        p.line_segment([pos2(inner.left(), y), pos2(inner.right(), y)], Stroke::new(0.5, theme::alpha(pal::ink(), 0.12)));
    }
    p.text(wr.left_top() + vec2(8.0, 5.0), egui::Align2::LEFT_TOP, "luma waveform", theme::mono(FS_2XS), pal::faint());
    let _ = CornerRadius::ZERO;
}

// ===================================================================== audio

fn audio_panel(app: &mut KerfApp, ui: &mut egui::Ui) {
    theme::section(ui, "mixer");
    let tracks: Vec<_> = app.project().timeline.audio_tracks().cloned().collect();
    for t in tracks {
        ui.horizontal(|ui| {
            ui.add_sized([34.0, 18.0], egui::Label::new(RichText::new(&t.name).strong().color(pal::ok())));
            let mut muted = t.muted;
            if ui.toggle_value(&mut muted, "M").on_hover_text("mute").changed() {
                app.edit(|d| d.apply(Command::UpdateTrack { id: t.id, name: t.name.clone(), muted, hidden: t.hidden, locked: t.locked, volume: t.volume }));
            }
            let mut db = 20.0 * t.volume.max(1e-4).log10();
            let r = ui.add(egui::Slider::new(&mut db, -60.0..=12.0).suffix(" dB").max_decimals(1));
            if r.changed() {
                let v = if db <= -59.9 { 0.0 } else { 10f32.powf(db / 20.0) };
                let cmd = Command::UpdateTrack { id: t.id, name: t.name.clone(), muted: t.muted, hidden: t.hidden, locked: t.locked, volume: v };
                let key = format!("trackvol:{}", t.id.0);
                if let Err(e) = app.doc.apply_merge(cmd, &key) {
                    app.toast(e.to_string(), pal::bad());
                }
            }
        });
    }
    ui.add_space(4.0);
    if ui.button("+ Audio track").clicked() {
        app.edit(|d| d.add_track(TrackKind::Audio));
    }
    // Live meter.
    theme::section(ui, "output");
    let (l, r) = app.engine.meters.get();
    for (n, v) in [("L", l), ("R", r)] {
        ui.horizontal(|ui| {
            ui.label(RichText::new(n).family(egui::FontFamily::Monospace).color(pal::dim()));
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width() - 50.0, 10.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect, 3, pal::bg());
            let db = 20.0 * v.max(1e-5).log10();
            let f = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
            let fill = Rect::from_min_size(rect.min, vec2(rect.width() * f, rect.height()));
            p.rect_filled(fill, 3, if db > -3.0 { pal::bad() } else if db > -12.0 { pal::warn() } else { pal::ok() });
            ui.label(RichText::new(format!("{:>5.1}", db.max(-60.0))).family(egui::FontFamily::Monospace).size(10.5).color(pal::dim()));
        });
    }
    if let Some(clip) = focus_clip(app) {
        if clip.media().is_some() {
            theme::section(ui, "selected clip");
            volume_slider(app, ui, clip.id, clip.volume);
            let id = clip.id;
            let max = (clip.duration.as_secs_f64() / 2.0) as f32;
            slider(app, ui, id, "fin", "fade in (s)", clip.fade_in.as_secs_f64() as f32, 0.0..=max, 0.0, |c, v| c.fade_in = Time::from_secs_f64(v as f64));
            slider(app, ui, id, "fout", "fade out (s)", clip.fade_out.as_secs_f64() as f32, 0.0..=max, 0.0, |c, v| c.fade_out = Time::from_secs_f64(v as f64));
        }
    }
    theme::section(ui, "tip");
    ui.label(
        RichText::new("Agents can remove silences for you:\nkerf auto-cut-silence --db -35 --min 0.6")
            .size(11.0)
            .family(egui::FontFamily::Monospace)
            .color(pal::faint()),
    );
}
