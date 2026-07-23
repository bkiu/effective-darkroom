//! Design system v2 tokens and egui style application.
//!
//! Source of truth: "Effective Darkroom Design System" (Claude Design).
//! Colors are authored in oklch and converted here; reference the token
//! constants — never hardcode raw colors in UI code.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, Vec2};

/// oklch → sRGB Color32 (D65, clamped).
pub const fn oklch(l: f32, c: f32, h_deg: f32) -> Oklch {
    Oklch { l, c, h_deg }
}

pub struct Oklch {
    l: f32,
    c: f32,
    h_deg: f32,
}

impl Oklch {
    pub fn color32(&self) -> Color32 {
        let h = self.h_deg.to_radians();
        let (a, b) = (self.c * h.cos(), self.c * h.sin());
        // oklab → LMS'
        let l_ = self.l + 0.396_337_78 * a + 0.215_803_76 * b;
        let m_ = self.l - 0.105_561_346 * a - 0.063_854_17 * b;
        let s_ = self.l - 0.089_484_18 * a - 1.291_485_5 * b;
        let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
        // LMS → linear sRGB
        let r = 4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3;
        let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3;
        let bl = -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;
        let enc = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            let e = if x <= 0.003_130_8 {
                12.92 * x
            } else {
                1.055 * x.powf(1.0 / 2.4) - 0.055
            };
            (e * 255.0).round() as u8
        };
        Color32::from_rgb(enc(r), enc(g), enc(bl))
    }
}

// --- Surfaces & lines ---
pub fn bg_canvas() -> Color32 {
    oklch(0.115, 0.004, 240.0).color32()
}
pub fn bg_panel() -> Color32 {
    oklch(0.185, 0.006, 240.0).color32()
}
pub fn bg_raised() -> Color32 {
    oklch(0.225, 0.007, 240.0).color32()
}
pub fn bg_hover() -> Color32 {
    oklch(0.275, 0.008, 240.0).color32()
}
pub fn line_subtle() -> Color32 {
    oklch(0.32, 0.008, 240.0).color32()
}
pub fn line_strong() -> Color32 {
    oklch(0.42, 0.008, 240.0).color32()
}

// --- Text ---
pub fn text_primary() -> Color32 {
    oklch(0.97, 0.003, 240.0).color32()
}
pub fn text_secondary() -> Color32 {
    oklch(0.74, 0.005, 240.0).color32()
}
pub fn text_muted() -> Color32 {
    oklch(0.55, 0.006, 240.0).color32()
}

// --- Accent & status ---
pub fn accent() -> Color32 {
    oklch(0.66, 0.12, 216.0).color32()
}
pub fn accent_hover() -> Color32 {
    oklch(0.72, 0.13, 216.0).color32()
}
/// Text on top of accent fills.
pub fn on_accent() -> Color32 {
    oklch(0.15, 0.02, 216.0).color32()
}
pub fn danger() -> Color32 {
    oklch(0.62, 0.19, 25.0).color32()
}
pub fn flag_pick() -> Color32 {
    oklch(0.80, 0.13, 78.0).color32()
}

// --- Radii (design: sm 3, md 5, lg 8) ---
pub const RADIUS_SM: u8 = 3;
pub const RADIUS_MD: u8 = 5;
pub const RADIUS_LG: u8 = 8;

/// Named font families registered in `install_fonts`.
pub const FONT_SEMIBOLD: &str = "plex-semibold";

pub fn mono() -> FontFamily {
    FontFamily::Monospace
}

pub fn semibold() -> FontFamily {
    FontFamily::Name(FONT_SEMIBOLD.into())
}

// Type scale (design: Display 26, Heading 19, Section 15, Body 13, Overline 11)
pub fn display_font() -> FontId {
    FontId::new(26.0, semibold())
}
pub fn heading_font() -> FontId {
    FontId::new(19.0, semibold())
}
pub fn section_font() -> FontId {
    FontId::new(15.0, semibold())
}
pub fn body_font() -> FontId {
    FontId::new(13.0, FontFamily::Proportional)
}
pub fn overline_font() -> FontId {
    FontId::new(11.0, mono())
}

/// Section header text ("Basic", "White Balance", …).
pub fn section_text(s: &str) -> egui::RichText {
    egui::RichText::new(s)
        .font(section_font())
        .color(text_primary())
}

/// Overline group header (mono caps, muted).
pub fn overline_text(s: &str) -> egui::RichText {
    egui::RichText::new(s.to_uppercase())
        .font(overline_font())
        .color(text_muted())
}

/// Monospace data text (EXIF, values, counts).
pub fn mono_text(s: &str) -> egui::RichText {
    egui::RichText::new(s).font(FontId::new(12.5, mono()))
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "plex-sans".into(),
        egui::FontData::from_static(include_bytes!(
            "../../assets/fonts/IBMPlexSans-Regular.ttf"
        ))
        .into(),
    );
    fonts.font_data.insert(
        FONT_SEMIBOLD.into(),
        egui::FontData::from_static(include_bytes!(
            "../../assets/fonts/IBMPlexSans-SemiBold.ttf"
        ))
        .into(),
    );
    fonts.font_data.insert(
        "plex-mono".into(),
        egui::FontData::from_static(include_bytes!(
            "../../assets/fonts/IBMPlexMono-Regular.ttf"
        ))
        .into(),
    );

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "plex-sans".into());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "plex-mono".into());
    // Semibold family: the face plus the emoji fallbacks.
    let mut semi = vec![FONT_SEMIBOLD.to_string()];
    if let Some(prop) = fonts.families.get(&FontFamily::Proportional) {
        semi.extend(prop.iter().skip(1).cloned());
    }
    fonts
        .families
        .insert(FontFamily::Name(FONT_SEMIBOLD.into()), semi);

    ctx.set_fonts(fonts);
}

/// Apply the design system to the egui context (visuals, spacing, fonts).
pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);

    ctx.all_styles_mut(|style| {
        // Type scale.
        style
            .text_styles
            .insert(egui::TextStyle::Heading, heading_font());
        style.text_styles.insert(egui::TextStyle::Body, body_font());
        style
            .text_styles
            .insert(egui::TextStyle::Button, body_font());
        style
            .text_styles
            .insert(egui::TextStyle::Small, FontId::new(11.0, FontFamily::Proportional));
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, FontId::new(12.5, mono()));

        // Spacing on the 4px base.
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(12.0, 5.0);
        style.spacing.menu_margin = Margin::same(8);
        style.spacing.window_margin = Margin::same(16);

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = Some(text_secondary());
        v.panel_fill = bg_panel();
        v.window_fill = bg_panel();
        v.extreme_bg_color = bg_canvas();
        v.faint_bg_color = bg_raised();
        v.window_stroke = Stroke::new(1.0, line_subtle());
        v.window_corner_radius = CornerRadius::same(RADIUS_LG);
        v.menu_corner_radius = CornerRadius::same(RADIUS_MD);

        v.widgets.noninteractive.bg_fill = bg_panel();
        v.widgets.noninteractive.weak_bg_fill = bg_panel();
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, line_subtle());
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text_secondary());
        v.widgets.noninteractive.corner_radius = CornerRadius::same(RADIUS_MD);

        v.widgets.inactive.bg_fill = bg_raised();
        v.widgets.inactive.weak_bg_fill = bg_raised();
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, line_subtle());
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, text_secondary());
        v.widgets.inactive.corner_radius = CornerRadius::same(RADIUS_MD);

        v.widgets.hovered.bg_fill = bg_hover();
        v.widgets.hovered.weak_bg_fill = bg_hover();
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, line_strong());
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, text_primary());
        v.widgets.hovered.corner_radius = CornerRadius::same(RADIUS_MD);

        v.widgets.active.bg_fill = bg_hover();
        v.widgets.active.weak_bg_fill = bg_hover();
        v.widgets.active.bg_stroke = Stroke::new(1.0, accent());
        v.widgets.active.fg_stroke = Stroke::new(1.0, text_primary());
        v.widgets.active.corner_radius = CornerRadius::same(RADIUS_MD);

        v.widgets.open.bg_fill = bg_raised();
        v.widgets.open.weak_bg_fill = bg_raised();
        v.widgets.open.bg_stroke = Stroke::new(1.0, accent());
        v.widgets.open.corner_radius = CornerRadius::same(RADIUS_MD);

        // Solid accent for selected/active states (design: filled cyan with
        // white content), not a dimmed tint.
        v.selection.bg_fill = accent();
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        v.hyperlink_color = accent();
        v.error_fg_color = danger();
        v.warn_fg_color = flag_pick();
        v.slider_trailing_fill = true;
    });
}

/// Primary (accent-filled) button, per the design's `primary` spec.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let rich = egui::RichText::new(text).color(on_accent()).font(FontId::new(
        13.0,
        semibold(),
    ));
    let button = egui::Button::new(rich)
        .fill(accent())
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(RADIUS_MD));
    let resp = ui.add(button);
    if resp.hovered() {
        ui.painter().rect_filled(
            resp.rect,
            CornerRadius::same(RADIUS_MD),
            accent_hover().gamma_multiply(0.35),
        );
    }
    resp
}

/// Danger (destructive) button.
pub fn danger_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let rich = egui::RichText::new(text).color(danger());
    ui.add(
        egui::Button::new(rich)
            .fill(bg_raised())
            .stroke(Stroke::new(1.0, danger().gamma_multiply(0.6)))
            .corner_radius(CornerRadius::same(RADIUS_MD)),
    )
}

/// Ghost button: no fill until hover.
pub fn ghost_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(text_secondary()))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklch_conversion_sane() {
        // White-ish and black-ish stay near the gray axis.
        let w = oklch(0.97, 0.003, 240.0).color32();
        assert!(w.r() > 235 && w.g() > 235 && w.b() > 235);
        let k = oklch(0.115, 0.004, 240.0).color32();
        assert!(k.r() < 25 && k.g() < 25 && k.b() < 30);
        // The accent is a cyan-blue: blue/green above red.
        let a = accent().color32_debug();
        assert!(a.2 > a.0 && a.1 > a.0, "accent rgb: {a:?}");
    }

    trait Dbg {
        fn color32_debug(&self) -> (u8, u8, u8);
    }
    impl Dbg for Color32 {
        fn color32_debug(&self) -> (u8, u8, u8) {
            (self.r(), self.g(), self.b())
        }
    }
}
