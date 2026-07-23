//! Single-image view: fit-to-window or 1:1 with drag panning. Used by both
//! the Library loupe and the Develop module.

use egui::{Color32, Pos2, Rect, Sense, Vec2};

#[derive(Default)]
pub struct LoupeState {
    pub one_to_one: bool,
    /// Pan offset in image pixels at 1:1.
    pub pan: Vec2,
    /// Anchor of the straighten line being drawn (screen coords).
    line_start: Option<Pos2>,
}

impl LoupeState {
    pub fn reset_view(&mut self) {
        self.one_to_one = false;
        self.pan = Vec2::ZERO;
    }
}

#[derive(Default)]
pub struct LoupeResponse {
    pub crop_changed: bool,
    /// Angle correction from a completed straighten-line drag (additive).
    pub straighten_delta: Option<f32>,
}

/// Draw the rendered image (if any) into the available space, with an
/// optional EXIF info overlay, crop-edit overlay, and straighten line tool.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut LoupeState,
    tex: Option<(egui::TextureId, Vec2)>,
    loading: bool,
    info: Option<&[String]>,
    crop: Option<&mut crate::ui::crop_overlay::CropEdit<'_>>,
    straighten: bool,
) -> LoupeResponse {
    let mut out = LoupeResponse::default();
    let mut crop_changed = false;
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    ui.painter().rect_filled(rect, 0.0, Color32::from_gray(16));

    let Some((tex_id, img_size)) = tex else {
        if loading {
            ui.put(rect, egui::Spinner::new().size(32.0));
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No image selected",
                egui::FontId::proportional(15.0),
                Color32::from_gray(120),
            );
        }
        return out;
    };

    let cropping = crop.is_some();
    if cropping || straighten {
        // Crop tool: always fit, no pan/zoom; drags belong to the overlay.
        state.one_to_one = false;
        state.pan = Vec2::ZERO;
    }

    if !cropping && !straighten && response.double_clicked() {
        state.one_to_one = !state.one_to_one;
        // Center the zoom on the click point.
        if state.one_to_one {
            if let Some(pos) = response.interact_pointer_pos() {
                let fit_scale = (rect.width() / img_size.x).min(rect.height() / img_size.y);
                let fit_size = img_size * fit_scale;
                let fit_rect = Rect::from_center_size(rect.center(), fit_size);
                let rel = ((pos - fit_rect.min) / fit_scale).to_pos2();
                state.pan = rel.to_vec2() - rect.size() / 2.0;
            }
        } else {
            state.pan = Vec2::ZERO;
        }
    }

    let draw_rect;
    let uv;
    if state.one_to_one {
        if !cropping && response.dragged() {
            state.pan -= response.drag_delta();
        }
        let view = rect.size();
        let max_pan = (img_size - view).max(Vec2::ZERO);
        state.pan = state.pan.clamp(Vec2::ZERO, max_pan);
        // Image smaller than viewport in either axis: center it.
        let offset = ((view - img_size) / 2.0).max(Vec2::ZERO);
        let visible = img_size.min(view);
        let uv_min = state.pan / img_size;
        let uv_max = (state.pan + visible) / img_size;
        uv = Rect::from_min_max(
            Pos2::new(uv_min.x, uv_min.y),
            Pos2::new(uv_max.x, uv_max.y),
        );
        draw_rect = Rect::from_min_size(rect.min + offset, visible);
    } else {
        let scale = (rect.width() / img_size.x)
            .min(rect.height() / img_size.y)
            .min(1.0);
        let size = img_size * scale;
        draw_rect = Rect::from_center_size(rect.center(), size);
        uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    }
    ui.painter().image(tex_id, draw_rect, uv, Color32::WHITE);

    if straighten {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if response.drag_started() {
            state.line_start = response.interact_pointer_pos();
        }
        if let (Some(a), Some(b)) = (state.line_start, response.interact_pointer_pos()) {
            if response.dragged() || response.drag_stopped() {
                let painter = ui.painter();
                painter.line_segment(
                    [a, b],
                    egui::Stroke::new(1.5, crate::ui::theme::accent()),
                );
                painter.circle_filled(a, 3.0, crate::ui::theme::accent());
                painter.circle_filled(b, 3.0, crate::ui::theme::accent());
            }
            if response.drag_stopped() {
                let d = b - a;
                if d.length() > 20.0 {
                    // Fold the line angle to its deviation from the nearest
                    // axis; rotating by the negative deviation levels it.
                    let raw = d.y.atan2(d.x).to_degrees();
                    let mut dev = raw.rem_euclid(90.0);
                    if dev >= 45.0 {
                        dev -= 90.0;
                    }
                    out.straighten_delta = Some(-dev);
                }
                state.line_start = None;
            }
        }
        if state.line_start.is_some()
            && !response.dragged()
            && !response.drag_stopped()
            && !response.is_pointer_button_down_on()
        {
            state.line_start = None;
        }
    } else {
        state.line_start = None;
    }

    if let Some(edit) = crop {
        crop_changed = crate::ui::crop_overlay::show(ui, draw_rect, edit);
    }

    if let Some(lines) = info {
        draw_info_overlay(ui, rect, lines);
    }

    if loading {
        // A decode for this image is still in flight (e.g. RAW linear pass).
        ui.put(
            Rect::from_min_size(rect.min + Vec2::splat(8.0), Vec2::splat(20.0)),
            egui::Spinner::new().size(20.0),
        );
    }
    out.crop_changed = crop_changed;
    out
}

fn draw_info_overlay(ui: &egui::Ui, rect: Rect, lines: &[String]) {
    use crate::ui::theme;
    let painter = ui.painter();
    let margin = 14.0;
    let pad = 10.0;
    let title_font = egui::FontId::new(15.0, theme::semibold());
    let body_font = egui::FontId::new(12.5, theme::mono());
    let line_gap = 4.0;

    // Measure.
    let mut galleys = Vec::new();
    let mut size = Vec2::ZERO;
    for (i, line) in lines.iter().enumerate() {
        let font = if i == 0 { &title_font } else { &body_font };
        let galley = painter.layout_no_wrap(
            line.clone(),
            font.clone(),
            Color32::from_gray(if i == 0 { 240 } else { 205 }),
        );
        size.x = size.x.max(galley.size().x);
        size.y += galley.size().y + if i > 0 { line_gap } else { 0.0 };
        galleys.push(galley);
    }

    let bg = Rect::from_min_size(
        rect.min + Vec2::splat(margin),
        size + Vec2::splat(2.0 * pad),
    );
    painter.rect_filled(bg, 6.0, Color32::from_black_alpha(170));
    let mut cursor = bg.min + Vec2::splat(pad);
    for galley in galleys {
        let h = galley.size().y;
        painter.galley(cursor, galley, Color32::WHITE);
        cursor.y += h + line_gap;
    }
}
