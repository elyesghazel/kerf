//! The program monitor: the preview frame and the transport.

use egui::{pos2, vec2, Rect, RichText, Sense, Stroke, StrokeKind};
use kerf_core::Time;

use crate::app::{KerfApp, Quality};
use crate::theme::{self, *};

pub fn show(app: &mut KerfApp, ui: &mut egui::Ui) {
    let full = ui.available_rect_before_wrap();
    let transport_h = 60.0;
    let meta_h = 28.0;
    let view = Rect::from_min_max(full.min + vec2(20.0, meta_h), pos2(full.right() - 20.0, full.bottom() - transport_h));
    let tl = app.project().timeline.clone();
    let aspect = tl.width as f32 / tl.height as f32;
    let mut fw = view.width();
    let mut fh = fw / aspect;
    if fh > view.height() {
        fh = view.height();
        fw = fh * aspect;
    }
    let frame_rect = Rect::from_center_size(view.center(), vec2(fw.max(2.0), fh.max(2.0)));

    // Pixels matching the on-screen size, capped by format and quality.
    let ppp = ui.ctx().pixels_per_point();
    let q = app.quality.factor();
    let w = ((fw * ppp * q).min(tl.width as f32) / 16.0).round().max(4.0) * 16.0;
    let h = (w / aspect / 2.0).round() * 2.0;
    app.engine.set_size(w as u32, h.max(2.0) as u32);

    let p = ui.painter_at(full);
    p.rect_filled(full, 0, pal::bg());

    // Meta line above the frame: format left, state right.
    let meta_y = full.top() + meta_h / 2.0 + 2.0;
    p.text(
        pos2(frame_rect.left(), meta_y),
        egui::Align2::LEFT_CENTER,
        format!("{}×{} · {} fps", tl.width, tl.height, tl.fps),
        theme::mono(FS_XS),
        pal::faint(),
    );
    let state = if app.engine.is_playing() { "playing" } else { "paused" };
    p.text(pos2(frame_rect.right(), meta_y), egui::Align2::RIGHT_CENTER, state, theme::mono(FS_XS), pal::faint());

    p.rect_filled(frame_rect, 0, egui::Color32::BLACK);
    if let Some(tex) = &app.preview {
        p.image(tex.id(), frame_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
    p.rect_stroke(frame_rect.expand(1.0), 0, Stroke::new(1.0, pal::rule()), StrokeKind::Outside);
    if app.show_safe {
        for (f, a) in [(0.9, 0.35), (0.8, 0.2)] {
            let r = Rect::from_center_size(frame_rect.center(), frame_rect.size() * f);
            p.rect_stroke(r, 0, Stroke::new(1.0, theme::alpha(egui::Color32::WHITE, a)), StrokeKind::Inside);
        }
    }
    if tl.duration() == Time::ZERO {
        // Empty state: dashed box, `// --`, one line, a hint.
        let r = frame_rect.shrink(frame_rect.width().min(frame_rect.height()) * 0.18);
        dashed_rect(&p, r, pal::rule_strong());
        p.text(r.center() - vec2(0.0, 26.0), egui::Align2::CENTER_CENTER, "// --", theme::mono(FS_XS), pal::faint());
        p.text(r.center(), egui::Align2::CENTER_CENTER, "nothing on the timeline yet", theme::mono(FS_MD), pal::ink());
        p.text(
            r.center() + vec2(0.0, 24.0),
            egui::Align2::CENTER_CENTER,
            "drop files on the window, then drag them down",
            theme::mono(FS_SM),
            pal::faint(),
        );
    }
    let resp = ui.interact(frame_rect, ui.scope_id().with("monitor"), Sense::click());
    if resp.clicked() {
        let end = tl.duration();
        app.engine.toggle(end);
    }

    let tr = Rect::from_min_max(pos2(full.left(), full.bottom() - transport_h), full.max);
    p.line_segment([tr.left_top(), tr.right_top()], Stroke::new(1.0, pal::rule()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(tr.shrink2(vec2(20.0, 8.0))));
    transport(app, &mut child);
}

fn dashed_rect(p: &egui::Painter, r: Rect, c: egui::Color32) {
    let s = Stroke::new(1.0, c);
    for (a, b) in [
        (r.left_top(), r.right_top()),
        (r.right_top(), r.right_bottom()),
        (r.right_bottom(), r.left_bottom()),
        (r.left_bottom(), r.left_top()),
    ] {
        p.extend(egui::Shape::dashed_line(&[a, b], s, 5.0, 4.0));
    }
}

fn transport(app: &mut KerfApp, ui: &mut egui::Ui) {
    let fps = app.project().timeline.fps;
    let end = app.project().timeline.duration();
    ui.horizontal_centered(|ui| {
        ui.label(RichText::new(app.playhead.timecode(fps)).font(theme::mono(FS_XL)).color(pal::ink()));
        ui.label(RichText::new(format!("/ {}", end.timecode(fps))).font(theme::mono(FS_XS)).color(pal::faint()));

        let avail = ui.available_width();
        let (center_w, right_w) = (200.0, 200.0);
        let full_w = ui.max_rect().width();
        let ideal = full_w / 2.0 - center_w / 2.0 - (full_w - avail);
        ui.add_space(ideal.min(avail - center_w - right_w).max(6.0));

        let fd = Time::from_secs_f64(1.0 / fps);
        if theme::glyph_button(ui, Glyph::ToStart, "start  (home)").clicked() {
            app.seek(Time::ZERO);
        }
        if theme::glyph_button(ui, Glyph::StepBack, "back one frame  (←)").clicked() {
            let t = app.playhead - fd;
            app.seek(t);
        }
        // Play: the one framed control here.
        let (r, resp) = ui.allocate_exact_size(vec2(40.0, 34.0), Sense::click());
        let p = ui.painter();
        let playing = app.engine.is_playing();
        p.rect_filled(r, R_CTRL, if resp.hovered() { pal::hover() } else { pal::bg() });
        p.rect_stroke(r, R_CTRL, Stroke::new(1.0, if resp.hovered() { pal::ink() } else { pal::rule_strong() }), StrokeKind::Inside);
        draw_glyph(p, r.center(), if playing { Glyph::Pause } else { Glyph::Play }, 14.0, pal::ink());
        if resp.on_hover_text("play / pause  (space)").clicked() {
            app.engine.toggle(end);
        }
        if theme::glyph_button(ui, Glyph::StepFwd, "forward one frame  (→)").clicked() {
            let t = app.playhead + fd;
            app.seek(t);
        }
        if theme::glyph_button(ui, Glyph::ToEnd, "end  (end)").clicked() {
            app.seek(end);
        }
        if theme::icon_button(ui, "loop", app.looping, "loop in → out").clicked() {
            app.looping = !app.looping;
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Output meter: two 4 px bars, coloured by status.
            let (l, r) = app.engine.meters.get();
            let (mr, _) = ui.allocate_exact_size(vec2(12.0, 30.0), Sense::hover());
            let p = ui.painter();
            for (i, v) in [l, r].into_iter().enumerate() {
                let bar = Rect::from_min_size(mr.min + vec2(i as f32 * 7.0, 0.0), vec2(4.0, mr.height()));
                p.rect_filled(bar, 0, pal::hover());
                let db = 20.0 * v.max(1e-5).log10();
                let f = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
                let fill = Rect::from_min_max(pos2(bar.left(), bar.bottom() - bar.height() * f), bar.max);
                let col = if db > -3.0 { pal::bad() } else if db > -12.0 { pal::warn() } else { pal::ok() };
                p.rect_filled(fill, 0, col);
            }
            if app.engine.is_playing() {
                ui.ctx().request_repaint();
            }
            ui.add_space(6.0);
            egui::ComboBox::from_id_salt("quality")
                .width(52.0)
                .selected_text(app.quality.label())
                .show_ui(ui, |ui| {
                    for q in [Quality::Full, Quality::Half, Quality::Quarter] {
                        ui.selectable_value(&mut app.quality, q, q.label());
                    }
                })
                .response
                .on_hover_text("preview resolution");
            if theme::icon_button(ui, "safe", app.show_safe, "safe-area guides").clicked() {
                app.show_safe = !app.show_safe;
            }
            if theme::icon_button(ui, "proxy", app.proxies_on, "preview from proxies").clicked() {
                app.proxies_on = !app.proxies_on;
            }
        });
    });
}
