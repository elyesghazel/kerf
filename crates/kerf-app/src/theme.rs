//! The kerf look: near-black blue glass, cyan + violet neon accents.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(7, 9, 14);
pub const PANEL: Color32 = Color32::from_rgb(12, 15, 22);
pub const PANEL_HI: Color32 = Color32::from_rgb(18, 22, 32);
pub const RAISED: Color32 = Color32::from_rgb(25, 30, 43);
pub const LINE: Color32 = Color32::from_rgb(34, 40, 56);
pub const TEXT: Color32 = Color32::from_rgb(214, 222, 238);
pub const TEXT_DIM: Color32 = Color32::from_rgb(124, 135, 160);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(74, 84, 108);

pub const CYAN: Color32 = Color32::from_rgb(46, 233, 255);
pub const VIOLET: Color32 = Color32::from_rgb(154, 108, 255);
pub const MAGENTA: Color32 = Color32::from_rgb(255, 72, 196);
pub const LIME: Color32 = Color32::from_rgb(98, 255, 170);
pub const AMBER: Color32 = Color32::from_rgb(255, 196, 0);
pub const RED: Color32 = Color32::from_rgb(255, 84, 104);

pub const CLIP_VIDEO: Color32 = Color32::from_rgb(20, 56, 84);
pub const CLIP_AUDIO: Color32 = Color32::from_rgb(14, 58, 44);
pub const CLIP_TEXT: Color32 = Color32::from_rgb(58, 36, 104);
pub const CLIP_SOLID: Color32 = Color32::from_rgb(70, 48, 30);

pub fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn prop(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

fn try_font(paths: &[&str]) -> Option<Vec<u8>> {
    paths.iter().find_map(|p| std::fs::read(p).ok())
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    // Prefer a crisp UI sans and JetBrains Mono for timecodes when installed;
    // otherwise egui's bundled fonts are used.
    let sans = try_font(&[
        "/usr/share/fonts/inter/InterVariable.ttf",
        "/usr/share/fonts/inter/Inter-Medium.otf",
        "/usr/share/fonts/TTF/Inter-Medium.ttf",
        "/usr/share/fonts/noto/NotoSans-Medium.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    ]);
    let home = std::env::var("HOME").unwrap_or_default();
    let local = |f: &str| format!("{home}/.local/share/fonts/{f}");
    let l1 = local("JetBrainsMonoNF/JetBrainsMonoNerdFont-Medium.ttf");
    let l2 = local("JetBrainsMonoNerdFont-Medium.ttf");
    let monof = try_font(&[
        &l1,
        &l2,
        "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Medium.ttf",
        "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf",
        "/usr/share/fonts/TTF/JetBrainsMono-Medium.ttf",
        "/usr/share/fonts/TTF/JetBrainsMono-Regular.ttf",
    ]);
    if let Some(b) = sans {
        fonts.font_data.insert("ui".into(), FontData::from_owned(b).into());
        fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "ui".into());
    }
    if let Some(b) = monof {
        fonts.font_data.insert("mono".into(), FontData::from_owned(b).into());
        fonts.families.get_mut(&FontFamily::Monospace).unwrap().insert(0, "mono".into());
        // Nerd-font icons as a fallback for the UI font too.
        fonts.families.get_mut(&FontFamily::Proportional).unwrap().push("mono".into());
    }
    // Symbol fallbacks so icons render everywhere (egui 0.37 no longer adds
    // these to the families by default).
    fonts.font_data.insert("emoji-icons".into(), FontData::from_static(epaint_default_fonts::EMOJI_ICON).into());
    fonts.font_data.insert("noto-emoji".into(), FontData::from_static(epaint_default_fonts::NOTO_EMOJI_REGULAR).into());
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        let f = fonts.families.get_mut(&fam).unwrap();
        f.push("emoji-icons".into());
        f.push("noto-emoji".into());
    }
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, prop(10.5)),
            (TextStyle::Body, prop(13.0)),
            (TextStyle::Button, prop(12.5)),
            (TextStyle::Heading, prop(17.0)),
            (TextStyle::Monospace, mono(12.5)),
        ]
        .into();
        let sp = &mut style.spacing;
        sp.item_spacing = egui::vec2(8.0, 6.0);
        sp.button_padding = egui::vec2(10.0, 4.0);
        sp.interact_size.y = 24.0;
        sp.slider_width = 150.0;
        sp.window_margin = Margin::same(12);
        sp.menu_margin = Margin::same(8);

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = None;
        v.panel_fill = PANEL;
        v.window_fill = PANEL_HI;
        v.window_stroke = Stroke::new(1.0, LINE);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        v.extreme_bg_color = BG;
        v.faint_bg_color = PANEL_HI;
        v.code_bg_color = RAISED;
        v.hyperlink_color = CYAN;
        v.warn_fg_color = AMBER;
        v.error_fg_color = RED;
        v.selection.bg_fill = alpha(CYAN, 0.28);
        v.selection.stroke = Stroke::new(1.0, CYAN);
        v.window_shadow = egui::Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(160),
        };
        v.popup_shadow = v.window_shadow;

        let r = CornerRadius::same(6);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = PANEL;
        w.noninteractive.weak_bg_fill = PANEL;
        w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
        w.noninteractive.corner_radius = r;

        w.inactive.bg_fill = RAISED;
        w.inactive.weak_bg_fill = RAISED;
        w.inactive.bg_stroke = Stroke::new(1.0, LINE);
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.corner_radius = r;

        w.hovered.bg_fill = lerp(RAISED, CYAN, 0.12);
        w.hovered.weak_bg_fill = lerp(RAISED, CYAN, 0.12);
        w.hovered.bg_stroke = Stroke::new(1.0, alpha(CYAN, 0.6));
        w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = lerp(RAISED, CYAN, 0.25);
        w.active.weak_bg_fill = lerp(RAISED, CYAN, 0.25);
        w.active.bg_stroke = Stroke::new(1.0, CYAN);
        w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.active.corner_radius = r;
        w.active.expansion = 0.0;

        w.open.bg_fill = PANEL_HI;
        w.open.weak_bg_fill = PANEL_HI;
        w.open.bg_stroke = Stroke::new(1.0, alpha(CYAN, 0.5));
        w.open.fg_stroke = Stroke::new(1.0, TEXT);
        w.open.corner_radius = r;
    });
}

/// A thin neon line with a soft glow around it.
pub fn glow_line(p: &egui::Painter, a: egui::Pos2, b: egui::Pos2, color: Color32, width: f32) {
    p.line_segment([a, b], Stroke::new(width * 7.0, alpha(color, 0.06)));
    p.line_segment([a, b], Stroke::new(width * 3.5, alpha(color, 0.14)));
    p.line_segment([a, b], Stroke::new(width, color));
}

/// A rounded rect outline with a soft neon halo.
pub fn glow_rect(p: &egui::Painter, rect: egui::Rect, r: u8, color: Color32) {
    for (grow, a) in [(4.0, 0.06), (2.0, 0.14)] {
        p.rect_stroke(
            rect.expand(grow),
            CornerRadius::same(r.saturating_add(grow as u8)),
            Stroke::new(2.0, alpha(color, a)),
            egui::StrokeKind::Outside,
        );
    }
    p.rect_stroke(rect, CornerRadius::same(r), Stroke::new(1.5, color), egui::StrokeKind::Inside);
}

/// Section header: small caps label with an accent tick.
pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(3.0, 12.0), egui::Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(2), CYAN);
        ui.label(egui::RichText::new(title.to_uppercase()).size(10.5).color(TEXT_DIM).strong());
    });
    ui.add_space(2.0);
}

/// A pill-shaped toggle button used in the workspace tabs and toolbars.
pub fn pill(ui: &mut egui::Ui, selected: bool, text: &str) -> egui::Response {
    let font = prop(11.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, TEXT);
    let size = egui::vec2(galley.size().x + 22.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = ui.painter();
    let hovered = resp.hovered();
    if selected {
        p.rect_filled(rect, CornerRadius::same(12), alpha(CYAN, 0.14));
        glow_rect(p, rect, 12, alpha(CYAN, 0.9));
    } else if hovered {
        p.rect_filled(rect, CornerRadius::same(12), RAISED);
    }
    let color = if selected {
        CYAN
    } else if hovered {
        TEXT
    } else {
        TEXT_DIM
    };
    p.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

/// A square icon button.
pub fn icon_button(ui: &mut egui::Ui, icon: &str, active: bool, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 26.0), egui::Sense::click());
    let p = ui.painter();
    if active {
        p.rect_filled(rect, CornerRadius::same(6), alpha(CYAN, 0.16));
        p.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, alpha(CYAN, 0.7)), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), RAISED);
    }
    let c = if active { CYAN } else if resp.hovered() { Color32::WHITE } else { TEXT_DIM };
    p.text(rect.center(), egui::Align2::CENTER_CENTER, icon, prop(14.0), c);
    resp.on_hover_text(tip)
}
