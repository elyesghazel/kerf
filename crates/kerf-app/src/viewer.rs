//! The program monitor: preview frame with HUD overlays and the transport.

use egui::{pos2, vec2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind};
use kerf_core::Time;

use crate::app::{KerfApp, Quality};
use crate::theme::{self, *};

pub fn show(app: &mut KerfApp, ui: &mut egui::Ui) {
    let full = ui.available_rect_before_wrap();
    let transport_h = 64.0;
    let view = Rect::from_min_max(full.min + vec2(14.0, 12.0), pos2(full.right() - 14.0, full.bottom() - transport_h));
    let tl = app.project().timeline.clone();
    let aspect = tl.width as f32 / tl.height as f32;
    let mut fw = view.width();
    let mut fh = fw / aspect;
    if fh > view.height() {
        fh = view.height();
        fw = fh * aspect;
    }
    let frame_rect = Rect::from_center_size(view.center(), vec2(fw.max(2.0), fh.max(2.0)));

    // Ask the engine for pixels matching the on-screen size (capped by the
    // project format and the quality setting).
    let ppp = ui.ctx().pixels_per_point();
    let q = app.quality.factor();
    let w = ((fw * ppp * q).min(tl.width as f32) / 16.0).round().max(4.0) * 16.0;
    let h = (w / aspect / 2.0).round() * 2.0;
    app.engine.set_size(w as u32, h.max(2.0) as u32);

    let p = ui.painter_at(full);
    p.rect_filled(full, 0, BG);
    // Subtle dot grid behind the monitor.
    let step = 22.0;
    let mut y = full.top() + 6.0;
    while y < full.bottom() - transport_h {
        let mut x = full.left() + 6.0;
        while x < full.right() {
            p.circle_filled(pos2(x, y), 0.8, Color32::from_rgb(22, 27, 38));
            x += step;
        }
        y += step;
    }

    p.rect_filled(frame_rect, 0, Color32::BLACK);
    if let Some(tex) = &app.preview {
        p.image(tex.id(), frame_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    // HUD corner brackets.
    let c = theme::alpha(CYAN, 0.55);
    let l = 16.0;
    let fr = frame_rect.expand(5.0);
    for (corner, dx, dy) in [
        (fr.left_top(), 1.0, 1.0),
        (fr.right_top(), -1.0, 1.0),
        (fr.left_bottom(), 1.0, -1.0),
        (fr.right_bottom(), -1.0, -1.0),
    ] {
        p.line_segment([corner, corner + vec2(l * dx, 0.0)], Stroke::new(1.5, c));
        p.line_segment([corner, corner + vec2(0.0, l * dy)], Stroke::new(1.5, c));
    }
    if app.show_safe {
        for (f, a) in [(0.9, 0.35), (0.8, 0.22)] {
            let r = Rect::from_center_size(frame_rect.center(), frame_rect.size() * f);
            p.rect_stroke(r, 0, Stroke::new(1.0, theme::alpha(Color32::WHITE, a)), StrokeKind::Inside);
        }
        let cc = frame_rect.center();
        p.line_segment([cc - vec2(8.0, 0.0), cc + vec2(8.0, 0.0)], Stroke::new(1.0, theme::alpha(Color32::WHITE, 0.4)));
        p.line_segment([cc - vec2(0.0, 8.0), cc + vec2(0.0, 8.0)], Stroke::new(1.0, theme::alpha(Color32::WHITE, 0.4)));
    }
    // HUD text.
    let hud = format!(
        "{}×{}  ·  {} fps  ·  {}",
        tl.width,
        tl.height,
        tl.fps,
        if app.engine.is_playing() { "PLAYING" } else { "PAUSED" }
    );
    p.text(fr.left_top() + vec2(4.0, -14.0), egui::Align2::LEFT_BOTTOM, hud, theme::mono(10.0), TEXT_FAINT);
    if tl.duration() == Time::ZERO {
        p.text(
            frame_rect.center() - vec2(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            "Drop media here or press Ctrl+I",
            theme::prop(16.0),
            TEXT_DIM,
        );
        p.text(
            frame_rect.center() + vec2(0.0, 16.0),
            egui::Align2::CENTER_CENTER,
            "then drag it onto the timeline, or double-click it",
            theme::prop(12.0),
            TEXT_FAINT,
        );
    }
    // Clicking the monitor toggles playback.
    let resp = ui.interact(frame_rect, ui.scope_id().with("monitor"), Sense::click());
    if resp.clicked() {
        let end = tl.duration();
        app.engine.toggle(end);
    }

    // ---- transport
    let tr = Rect::from_min_max(pos2(full.left(), full.bottom() - transport_h), full.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(tr.shrink2(vec2(16.0, 10.0))));
    transport(app, &mut child);
}

fn transport(app: &mut KerfApp, ui: &mut egui::Ui) {
    let fps = app.project().timeline.fps;
    let end = app.project().timeline.duration();
    ui.horizontal_centered(|ui| {
        // Glowing timecode.
        let tc = app.playhead.timecode(fps);
        let (r, _) = ui.allocate_exact_size(vec2(150.0, 34.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, CornerRadius::same(8), PANEL_HI);
        p.rect_stroke(r, CornerRadius::same(8), Stroke::new(1.0, theme::alpha(CYAN, 0.25)), StrokeKind::Inside);
        p.text(r.center() + vec2(0.0, 1.0), egui::Align2::CENTER_CENTER, &tc, theme::mono(19.0), theme::alpha(CYAN, 0.25));
        p.text(r.center(), egui::Align2::CENTER_CENTER, &tc, theme::mono(18.0), CYAN);
        ui.label(RichText::new(format!("/ {}", end.timecode(fps))).family(egui::FontFamily::Monospace).color(TEXT_FAINT).size(11.0));

        // Centre the transport buttons, but never push them into the right group.
        let avail = ui.available_width();
        let (center_w, right_w) = (250.0, 190.0);
        let full_w = ui.max_rect().width();
        let used = full_w - avail;
        let ideal = full_w / 2.0 - center_w / 2.0 - used;
        ui.add_space(ideal.min(avail - center_w - right_w).max(6.0));
        let fd = Time::from_secs_f64(1.0 / fps);
        if theme::icon_button(ui, "⏮", false, "Start (Home)").clicked() {
            app.seek(Time::ZERO);
        }
        if theme::icon_button(ui, "◀", false, "Back one frame (‹)").clicked() {
            let t = app.playhead - fd;
            app.seek(t);
        }
        // Big play button.
        let (r, resp) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::click());
        let p = ui.painter();
        let playing = app.engine.is_playing();
        let hot = resp.hovered();
        p.circle_filled(r.center(), 20.0, theme::alpha(CYAN, if hot { 0.24 } else { 0.14 }));
        p.circle_stroke(r.center(), 20.0, Stroke::new(1.5, CYAN));
        p.circle_stroke(r.center(), 24.0, Stroke::new(3.0, theme::alpha(CYAN, 0.08)));
        if playing {
            for dx in [-4.5, 4.5] {
                p.rect_filled(Rect::from_center_size(r.center() + vec2(dx, 0.0), vec2(4.0, 14.0)), 1, Color32::WHITE);
            }
        } else {
            let c = r.center() + vec2(2.0, 0.0);
            p.add(egui::Shape::convex_polygon(
                vec![c + vec2(-6.0, -8.0), c + vec2(8.0, 0.0), c + vec2(-6.0, 8.0)],
                Color32::WHITE,
                Stroke::NONE,
            ));
        }
        if resp.on_hover_text("Play / pause (Space)").clicked() {
            app.engine.toggle(end);
        }
        if theme::icon_button(ui, "▶", false, "Forward one frame (›)").clicked() {
            let t = app.playhead + fd;
            app.seek(t);
        }
        if theme::icon_button(ui, "⏭", false, "End (End)").clicked() {
            app.seek(end);
        }
        if theme::icon_button(ui, "🔁", app.looping, "Loop In›Out").clicked() {
            app.looping = !app.looping;
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Audio meters.
            let (l, r) = app.engine.meters.get();
            let (mr, _) = ui.allocate_exact_size(vec2(14.0, 34.0), Sense::hover());
            let p = ui.painter();
            for (i, v) in [l, r].into_iter().enumerate() {
                let bar = Rect::from_min_size(mr.min + vec2(i as f32 * 7.0, 0.0), vec2(5.0, mr.height()));
                p.rect_filled(bar, 2, PANEL_HI);
                let db = 20.0 * v.max(1e-5).log10();
                let f = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
                let fill = Rect::from_min_max(pos2(bar.left(), bar.bottom() - bar.height() * f), bar.max);
                let col = if db > -3.0 { RED } else if db > -12.0 { AMBER } else { LIME };
                p.rect_filled(fill, 2, col);
            }
            if app.engine.is_playing() {
                ui.ctx().request_repaint();
            }
            ui.add_space(8.0);
            egui::ComboBox::from_id_salt("quality")
                .width(56.0)
                .selected_text(app.quality.label())
                .show_ui(ui, |ui| {
                    for q in [Quality::Full, Quality::Half, Quality::Quarter] {
                        ui.selectable_value(&mut app.quality, q, q.label());
                    }
                });
            if theme::icon_button(ui, "⊞", app.show_safe, "Safe-area guides").clicked() {
                app.show_safe = !app.show_safe;
            }
            if theme::icon_button(ui, "⚡", app.proxies_on, "Use proxies for preview").clicked() {
                app.proxies_on = !app.proxies_on;
            }
        });
    });
}
