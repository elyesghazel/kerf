//! The kerf look, from the elyesghazel design system (design.elyesghazel.ch):
//! black, one monospace family, hairlines, nearly square corners, white for
//! what matters, three greys for everything else, and one muted red that
//! marks "now" (the playhead). Dark and light, like every eg. site.

use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Pos2, Rect, Stroke, StrokeKind, TextStyle, Vec2};

static LIGHT: AtomicBool = AtomicBool::new(false);

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

/// Design tokens (`tokens.css`). Use these, never raw colours.
pub mod pal {
    use super::is_light;
    use egui::Color32;

    const fn rgb(v: u32) -> Color32 {
        Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
    fn pick(dark: u32, light: u32) -> Color32 {
        rgb(if is_light() { light } else { dark })
    }

    pub fn bg() -> Color32 {
        pick(0x000000, 0xfafafa)
    }
    pub fn raised() -> Color32 {
        pick(0x0f0f0f, 0xffffff)
    }
    pub fn sunken() -> Color32 {
        pick(0x050505, 0xf2f2f2)
    }
    pub fn hover() -> Color32 {
        pick(0x1a1a1a, 0xebebeb)
    }
    pub fn ink() -> Color32 {
        pick(0xffffff, 0x0a0a0a)
    }
    pub fn body() -> Color32 {
        pick(0xdcdcdc, 0x2b2b2b)
    }
    pub fn dim() -> Color32 {
        pick(0xa6a6a6, 0x555555)
    }
    pub fn faint() -> Color32 {
        pick(0x8a8a8a, 0x707070)
    }
    pub fn rule() -> Color32 {
        pick(0x333333, 0xd4d4d4)
    }
    pub fn rule_strong() -> Color32 {
        pick(0x666666, 0x888888)
    }
    /// The one colour: marks "now" (the playhead) and nothing else.
    pub fn accent() -> Color32 {
        pick(0xc47070, 0xa94848)
    }
    pub fn ok() -> Color32 {
        pick(0x6cc28a, 0x1e7a44)
    }
    pub fn warn() -> Color32 {
        pick(0xd9ad5b, 0x8f5f00)
    }
    pub fn bad() -> Color32 {
        pick(0xff5f56, 0xd0201f)
    }
    pub fn info() -> Color32 {
        pick(0x74a7e6, 0x1f5fb4)
    }
    /// 12 % wash of a status colour (tag fills, hover washes).
    pub fn soft(c: Color32) -> Color32 {
        super::alpha(c, 0.12)
    }

    // Timeline content. Clips stay neutral; type shows through content
    // (thumbnails, waveform, text), not colour.
    pub fn clip_video() -> Color32 {
        pick(0x141414, 0xe6e6e6)
    }
    pub fn clip_audio() -> Color32 {
        pick(0x0d0d0d, 0xf0f0f0)
    }
    pub fn clip_text() -> Color32 {
        pick(0x1f1f1f, 0xdedede)
    }
    pub fn lane_a() -> Color32 {
        pick(0x050505, 0xf5f5f5)
    }
    pub fn lane_b() -> Color32 {
        pick(0x080808, 0xf0f0f0)
    }
}

pub fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Same family (there is only one); kept so call sites read naturally.
pub fn prop(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

/// Type scale (`--fs-*`), tightened for a dense desktop tool.
pub const FS_XL: f32 = 22.0;
pub const FS_MD: f32 = 15.0;
pub const FS_SM: f32 = 12.5;
pub const FS_XS: f32 = 11.0;
pub const FS_2XS: f32 = 10.0;

pub const R_CTRL: u8 = 2;
pub const R_CARD: u8 = 4;

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    System,
    Dark,
    Light,
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("jbm".into(), FontData::from_static(include_bytes!("../assets/JetBrainsMono-Regular.ttf")).into());
    fonts.font_data.insert("jbm-bold".into(), FontData::from_static(include_bytes!("../assets/JetBrainsMono-Bold.ttf")).into());
    // Safety nets for file names in other scripts; never chosen for UI.
    fonts.font_data.insert("emoji-icons".into(), FontData::from_static(epaint_default_fonts::EMOJI_ICON).into());
    fonts.font_data.insert("noto-emoji".into(), FontData::from_static(epaint_default_fonts::NOTO_EMOJI_REGULAR).into());
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        let f = fonts.families.entry(fam).or_default();
        f.insert(0, "jbm".into());
        f.push("emoji-icons".into());
        f.push("noto-emoji".into());
    }
    let ub = fonts.families[&FontFamily::Proportional].clone();
    let mut b = vec!["jbm-bold".to_string()];
    b.extend(ub.into_iter().filter(|n| n != "jbm"));
    fonts.families.insert(FontFamily::Name("bold".into()), b);
    ctx.set_fonts(fonts);
}

/// Apply the palette for `light` (call again when the theme changes).
pub fn apply(ctx: &egui::Context, light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
    ctx.set_theme(if light { egui::Theme::Light } else { egui::Theme::Dark });
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, mono(FS_XS)),
            (TextStyle::Body, mono(FS_SM)),
            (TextStyle::Button, mono(FS_SM)),
            (TextStyle::Heading, mono(FS_XL)),
            (TextStyle::Monospace, mono(FS_SM)),
        ]
        .into();
        let sp = &mut style.spacing;
        sp.item_spacing = egui::vec2(8.0, 6.0);
        sp.button_padding = egui::vec2(10.0, 4.0);
        sp.interact_size.y = 26.0;
        sp.slider_width = 150.0;
        sp.slider_rail_height = 2.0;
        sp.window_margin = Margin::same(16);
        sp.menu_margin = Margin::same(6);
        sp.icon_width = 13.0;
        sp.icon_width_inner = 7.0;

        let v = &mut style.visuals;
        v.dark_mode = !light;
        v.override_text_color = None;
        v.panel_fill = pal::bg();
        v.window_fill = pal::raised();
        v.window_stroke = Stroke::new(1.0, pal::rule());
        v.window_corner_radius = CornerRadius::same(R_CARD);
        v.menu_corner_radius = CornerRadius::same(R_CARD);
        v.extreme_bg_color = pal::sunken();
        v.text_edit_bg_color = Some(pal::sunken());
        v.faint_bg_color = pal::raised();
        v.code_bg_color = pal::sunken();
        v.hyperlink_color = pal::ink();
        v.warn_fg_color = pal::warn();
        v.error_fg_color = pal::bad();
        v.selection.bg_fill = alpha(pal::rule_strong(), 0.55);
        v.selection.stroke = Stroke::new(1.0, pal::ink());
        v.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.55 };
        v.slider_trailing_fill = true;
        v.striped = false;
        v.indent_has_left_vline = false;
        // Only floating things cast a shadow (`--shadow-pop`).
        let pop = egui::Shadow {
            offset: [0, 6],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(if light { 40 } else { 150 }),
        };
        v.window_shadow = pop;
        v.popup_shadow = pop;

        let r = CornerRadius::same(R_CTRL);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = pal::bg();
        w.noninteractive.weak_bg_fill = pal::bg();
        w.noninteractive.bg_stroke = Stroke::new(1.0, pal::rule());
        w.noninteractive.fg_stroke = Stroke::new(1.0, pal::body());
        w.noninteractive.corner_radius = r;

        // Secondary buttons: hairline in rule-strong, transparent.
        w.inactive.bg_fill = pal::rule();
        w.inactive.weak_bg_fill = Color32::TRANSPARENT;
        w.inactive.bg_stroke = Stroke::new(1.0, pal::rule_strong());
        w.inactive.fg_stroke = Stroke::new(1.0, pal::body());
        w.inactive.corner_radius = r;
        w.inactive.expansion = 0.0;

        w.hovered.bg_fill = pal::rule_strong();
        w.hovered.weak_bg_fill = pal::hover();
        w.hovered.bg_stroke = Stroke::new(1.0, pal::ink());
        w.hovered.fg_stroke = Stroke::new(1.0, pal::ink());
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = pal::ink();
        w.active.weak_bg_fill = pal::hover();
        w.active.bg_stroke = Stroke::new(1.0, pal::ink());
        w.active.fg_stroke = Stroke::new(1.0, pal::ink());
        w.active.corner_radius = r;
        w.active.expansion = 0.0;

        w.open.bg_fill = pal::hover();
        w.open.weak_bg_fill = pal::hover();
        w.open.bg_stroke = Stroke::new(1.0, pal::rule_strong());
        w.open.fg_stroke = Stroke::new(1.0, pal::ink());
        w.open.corner_radius = r;
    });
}

// ------------------------------------------------------------- components

/// `// LABEL ─────────── aside`: the signature section label.
pub fn section(ui: &mut egui::Ui, title: &str) {
    section_aside(ui, title, None);
}

pub fn section_aside(ui: &mut egui::Ui, title: &str, aside: Option<&str>) {
    ui.add_space(10.0);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 16.0), egui::Sense::hover());
    let p = ui.painter();
    let y = rect.center().y;
    let slashes = p.text(rect.left_center(), egui::Align2::LEFT_CENTER, "//", mono(FS_XS), pal::faint());
    let label = tracked(p, &title.to_uppercase(), FS_XS, pal::ink());
    let lr = Rect::from_min_size(egui::pos2(slashes.right() + 8.0, y - label.size().y / 2.0), label.size());
    p.galley(lr.min, label, pal::ink());
    let mut end = rect.right();
    if let Some(a) = aside {
        let r = p.text(rect.right_center(), egui::Align2::RIGHT_CENTER, a, mono(FS_XS), pal::faint());
        end = r.left() - 10.0;
    }
    if end > lr.right() + 10.0 {
        p.line_segment([egui::pos2(lr.right() + 10.0, y), egui::pos2(end, y)], Stroke::new(1.0, pal::rule()));
    }
    ui.add_space(4.0);
}

/// Uppercase label text with the 0.2em `--tracking-label`.
pub fn tracked(p: &egui::Painter, text: &str, size: f32, color: Color32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: mono(size),
            color,
            extra_letter_spacing: size * 0.2,
            ..Default::default()
        },
    );
    p.layout_job(job)
}

/// Nav item: lowercase word, `--dim`, `--ink` when current with a 1 px
/// underline at `underline_y` (the header's bottom edge).
pub fn nav(ui: &mut egui::Ui, selected: bool, text: &str, underline_y: f32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), mono(FS_SM), pal::dim());
    let size = egui::vec2(galley.size().x + 4.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let color = if selected || resp.hovered() { pal::ink() } else { pal::dim() };
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
    if selected {
        ui.painter().line_segment(
            [egui::pos2(rect.left(), underline_y - 0.5), egui::pos2(rect.right(), underline_y - 0.5)],
            Stroke::new(1.0, pal::ink()),
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Kept for call sites: the workspace/tab switch is a nav item without an
/// underline target (underline right under the word).
pub fn pill(ui: &mut egui::Ui, selected: bool, text: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), mono(FS_SM), pal::dim());
    let size = egui::vec2(galley.size().x + 2.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let color = if selected || resp.hovered() { pal::ink() } else { pal::dim() };
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
    if selected {
        ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, pal::ink()));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small (28 px) text or glyph button. `active` = a toggle that is on.
pub fn icon_button(ui: &mut egui::Ui, label: &str, active: bool, tip: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), mono(FS_SM), pal::dim());
    let w = (galley.size().x + 16.0).max(28.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 26.0), egui::Sense::click());
    let p = ui.painter();
    let hot = resp.hovered();
    if active {
        p.rect_filled(rect, R_CTRL, pal::hover());
        p.rect_stroke(rect, R_CTRL, Stroke::new(1.0, pal::rule_strong()), StrokeKind::Inside);
    } else if hot {
        p.rect_stroke(rect, R_CTRL, Stroke::new(1.0, pal::rule_strong()), StrokeKind::Inside);
    }
    let c = if active || hot { pal::ink() } else { pal::dim() };
    p.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, c);
    resp.on_hover_text(tip)
}

/// `.btn-primary`: ink fill, bg text. One per view.
pub fn primary_button(ui: &mut egui::Ui, text: &str, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 36.0), egui::Sense::click());
    let enabled = ui.is_enabled();
    let p = ui.painter();
    let fill = if resp.hovered() { pal::body() } else { pal::ink() };
    p.rect_filled(rect, R_CTRL, if enabled { fill } else { alpha(pal::ink(), 0.4) });
    p.text(rect.center(), egui::Align2::CENTER_CENTER, text, mono(FS_SM), pal::bg());
    resp
}

/// A neutral or status tag: 2 px radius, 11 px text, status tags lead with a dot.
pub fn tag(ui: &mut egui::Ui, text: &str, status: Option<Color32>) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), mono(FS_XS), pal::dim());
    let dot = if status.is_some() { 12.0 } else { 0.0 };
    let size = egui::vec2(galley.size().x + 14.0 + dot, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    let p = ui.painter();
    match status {
        Some(c) => {
            p.rect_filled(rect, R_CTRL, pal::soft(c));
            p.circle_filled(egui::pos2(rect.left() + 10.0, rect.center().y), 2.5, c);
            p.galley_with_override_text_color(egui::pos2(rect.left() + 7.0 + dot, rect.center().y - galley.size().y / 2.0), galley, c);
        }
        None => {
            p.rect_stroke(rect, R_CTRL, Stroke::new(1.0, pal::rule()), StrokeKind::Inside);
            p.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, pal::dim());
        }
    }
    resp
}

/// Media-control glyphs drawn as shapes (JetBrains Mono has none).
#[derive(Clone, Copy)]
pub enum Glyph {
    Play,
    Pause,
    StepBack,
    StepFwd,
    ToStart,
    ToEnd,
}

pub fn draw_glyph(p: &egui::Painter, c: Pos2, g: Glyph, size: f32, color: Color32) {
    let s = size / 2.0;
    let tri = |dir: f32, cx: f32| {
        egui::Shape::convex_polygon(
            vec![
                egui::pos2(cx - s * 0.55 * dir, c.y - s * 0.7),
                egui::pos2(cx + s * 0.6 * dir, c.y),
                egui::pos2(cx - s * 0.55 * dir, c.y + s * 0.7),
            ],
            color,
            Stroke::NONE,
        )
    };
    let bar = |x: f32, w: f32| Rect::from_center_size(egui::pos2(x, c.y), Vec2::new(w, s * 1.4));
    match g {
        Glyph::Play => {
            p.add(tri(1.0, c.x + s * 0.1));
        }
        Glyph::Pause => {
            p.rect_filled(bar(c.x - s * 0.32, s * 0.32), 0, color);
            p.rect_filled(bar(c.x + s * 0.32, s * 0.32), 0, color);
        }
        Glyph::StepBack => {
            p.add(tri(-1.0, c.x));
        }
        Glyph::StepFwd => {
            p.add(tri(1.0, c.x));
        }
        Glyph::ToStart => {
            p.rect_filled(bar(c.x - s * 0.6, s * 0.18), 0, color);
            p.add(tri(-1.0, c.x + s * 0.15));
        }
        Glyph::ToEnd => {
            p.rect_filled(bar(c.x + s * 0.6, s * 0.18), 0, color);
            p.add(tri(1.0, c.x - s * 0.15));
        }
    }
}

/// A 28 px square button with a drawn glyph.
pub fn glyph_button(ui: &mut egui::Ui, g: Glyph, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
    let p = ui.painter();
    let hot = resp.hovered();
    if hot {
        p.rect_stroke(rect, R_CTRL, Stroke::new(1.0, pal::rule_strong()), StrokeKind::Inside);
    }
    draw_glyph(p, rect.center(), g, 12.0, if hot { pal::ink() } else { pal::dim() });
    resp.on_hover_text(tip)
}
