//! Single-image view: continuous zoom (fit through 16:1) with drag panning.
//! Used by both the Library loupe and the Develop module.

use egui::{Color32, Pos2, Rect, Sense, Vec2};

/// Zoom bounds, as screen pixels per image pixel. 1.0 is true 1:1.
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 16.0;
/// One wheel notch is ~50 points of scroll, which this puts at ~1.2x.
const WHEEL_ZOOM_RATE: f32 = 0.004;

pub struct LoupeState {
    /// None = fit to window. Some(z) = z screen pixels per image pixel, so
    /// 1.0 shows one image pixel per physical screen pixel regardless of the
    /// display scale factor — what "100%" means in a photo editor.
    pub zoom: Option<f32>,
    /// Top-left of the viewport in image pixels.
    pub pan: Vec2,
    /// Anchor of the straighten line being drawn (screen coords).
    line_start: Option<Pos2>,
    /// Geometry of the last drawn frame, so zoom commands from outside the
    /// view (toolbar buttons, keyboard) can work in the same units without
    /// knowing the viewport or the image size.
    last_zoom: f32,
    last_rect: Rect,
    last_img: Vec2,
    last_ppp: f32,
}

impl Default for LoupeState {
    fn default() -> Self {
        Self {
            zoom: None,
            pan: Vec2::ZERO,
            line_start: None,
            last_zoom: 1.0,
            last_rect: Rect::ZERO,
            last_img: Vec2::ZERO,
            last_ppp: 1.0,
        }
    }
}

impl LoupeState {
    /// Back to fit-to-window, unpanned.
    pub fn reset_view(&mut self) {
        self.zoom = None;
        self.pan = Vec2::ZERO;
    }

    pub fn is_fit(&self) -> bool {
        self.zoom.is_none()
    }

    /// Zoom actually drawn last frame, as a percentage (100 = one image pixel
    /// per screen pixel).
    pub fn zoom_percent(&self) -> f32 {
        self.last_zoom * 100.0
    }

    /// Set an absolute zoom, keeping the centre of the view put.
    pub fn set_zoom(&mut self, zoom: f32) {
        let centre = self.pan + self.visible_at(self.last_zoom) / 2.0;
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.zoom = Some(zoom);
        self.pan = centre - self.visible_at(zoom) / 2.0;
    }

    /// Multiply the current zoom, keeping the centre of the view put.
    pub fn zoom_by(&mut self, factor: f32) {
        self.set_zoom(self.last_zoom * factor);
    }

    /// Toggle between fit-to-window and 1:1, centred on `anchor` (a screen
    /// position) when one is given.
    pub fn toggle_one_to_one(&mut self, anchor: Option<Pos2>) {
        if self.zoom.is_some() {
            self.reset_view();
        } else {
            match anchor.filter(|p| self.last_rect.contains(*p)) {
                Some(pos) => self.zoom_to_point(1.0, pos),
                None => self.set_zoom(1.0),
            }
        }
    }

    /// Points of screen space per image pixel — the factor the painter works
    /// in. Zoom is stored per physical pixel, so the display scale divides out
    /// here and nowhere else.
    fn scale(&self, zoom: f32) -> f32 {
        zoom / self.last_ppp
    }

    /// Image pixels visible at `zoom`, given the last viewport.
    fn visible_at(&self, zoom: f32) -> Vec2 {
        if self.last_img == Vec2::ZERO {
            return Vec2::ZERO;
        }
        self.last_img.min(self.last_rect.size() / self.scale(zoom))
    }

    /// Top-left of the drawn image in screen coordinates; images smaller than
    /// the viewport are centred in it.
    fn draw_origin(&self, zoom: f32) -> Pos2 {
        let drawn = self.visible_at(zoom) * self.scale(zoom);
        self.last_rect.min + ((self.last_rect.size() - drawn) / 2.0).max(Vec2::ZERO)
    }

    /// Set the zoom so the image point under the screen position `pos` stays
    /// under `pos`.
    fn zoom_to_point(&mut self, zoom: f32, pos: Pos2) {
        let under =
            self.pan + (pos - self.draw_origin(self.last_zoom)) / self.scale(self.last_zoom);
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.zoom = Some(zoom);
        // The draw rect at the new zoom depends only on the zoom (the view is
        // centred), so the pan that re-anchors `under` is exact.
        self.pan = under - (pos - self.draw_origin(zoom)) / self.scale(zoom);
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
        state.reset_view();
    }

    // Geometry the zoom helpers work against. Written before input handling
    // so this frame's wheel notches anchor against this frame's layout, not
    // the one the previous image was drawn at.
    let ppp = ui.ctx().pixels_per_point();
    let fit = fit_zoom(rect, img_size, ppp);
    state.last_rect = rect;
    state.last_img = img_size;
    state.last_ppp = ppp;
    state.last_zoom = state.zoom.unwrap_or(fit);

    if !cropping && !straighten {
        if response.double_clicked() {
            state.toggle_one_to_one(response.interact_pointer_pos());
        }
        // Wheel zoom about the pointer. Ctrl+wheel is left to eframe, which
        // drives the whole-UI zoom factor with it.
        let (scroll, ctrl) = ui.input(|i| (i.smooth_scroll_delta.y, i.modifiers.ctrl));
        if response.hovered() && scroll != 0.0 && !ctrl {
            if let Some(pos) = ui.ctx().pointer_latest_pos() {
                let target = state.last_zoom * (scroll * WHEEL_ZOOM_RATE).exp();
                // Snap back to fit rather than crawling below it.
                if target <= fit {
                    state.reset_view();
                } else {
                    state.zoom_to_point(target, pos);
                }
            }
        }
    }

    let zoom = state.zoom.unwrap_or(fit);
    // Points of screen space per image pixel, for the painter.
    let scale = zoom / ppp;
    if state.zoom.is_some() && !cropping && response.dragged() {
        state.pan -= response.drag_delta() / scale;
    }

    // Clamp the pan to the image and derive the visible window from it.
    let visible = img_size.min(rect.size() / scale);
    let max_pan = (img_size - visible).max(Vec2::ZERO);
    state.pan = state.pan.clamp(Vec2::ZERO, max_pan);
    state.last_zoom = zoom;

    let uv_min = state.pan / img_size;
    let uv_max = (state.pan + visible) / img_size;
    let uv = Rect::from_min_max(
        Pos2::new(uv_min.x, uv_min.y),
        Pos2::new(uv_max.x, uv_max.y),
    );
    let draw_rect = Rect::from_center_size(rect.center(), visible * scale);
    ui.painter().image(tex_id, draw_rect, uv, Color32::WHITE);

    // Panning is only meaningful when the image overflows the viewport.
    if !cropping && !straighten && max_pan != Vec2::ZERO && response.hovered() {
        ui.ctx().set_cursor_icon(if response.dragged() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        });
    }

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

/// Zoom that fits the whole image in the viewport, never magnifying past 1:1.
/// `rect` is in points and `img_size` in image pixels, so the display scale
/// converts between them.
fn fit_zoom(rect: Rect, img_size: Vec2, ppp: f32) -> f32 {
    (rect.width() * ppp / img_size.x)
        .min(rect.height() * ppp / img_size.y)
        .min(1.0)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A 6000x4000 image in a 1200x800 point viewport on a 2x display, laid
    /// out once (as a frame of `show` would leave it).
    fn laid_out() -> LoupeState {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        let img = Vec2::new(6000.0, 4000.0);
        let ppp = 2.0;
        LoupeState {
            last_rect: rect,
            last_img: img,
            last_ppp: ppp,
            last_zoom: fit_zoom(rect, img, ppp),
            ..Default::default()
        }
    }

    /// The image pixel drawn at a screen position, per the current state.
    fn under(state: &LoupeState, pos: Pos2) -> Vec2 {
        state.pan + (pos - state.draw_origin(state.last_zoom)) / state.scale(state.last_zoom)
    }

    #[test]
    fn fit_accounts_for_the_display_scale_and_never_magnifies() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        let img = Vec2::new(6000.0, 4000.0);
        // 1200 points at 1x = 1200 px across 6000 image px.
        assert!((fit_zoom(rect, img, 1.0) - 0.2).abs() < 1e-6);
        // The same viewport at 2x holds twice as many pixels.
        assert!((fit_zoom(rect, img, 2.0) - 0.4).abs() < 1e-6);
        // Smaller than the viewport: shown 1:1, not blown up.
        assert_eq!(fit_zoom(rect, Vec2::new(300.0, 200.0), 1.0), 1.0);
    }

    #[test]
    fn one_to_one_is_one_image_pixel_per_screen_pixel() {
        let mut state = laid_out();
        state.set_zoom(1.0);
        state.last_zoom = 1.0;
        // At 2x, one image pixel occupies half a point — i.e. one device pixel.
        assert!((state.scale(1.0) - 0.5).abs() < 1e-6);
        // A 1200-point viewport therefore shows 2400 image pixels.
        assert!((state.visible_at(1.0).x - 2400.0).abs() < 0.01);
    }

    #[test]
    fn zoom_to_point_pins_the_pixel_under_the_cursor() {
        let mut state = laid_out();
        let cursor = Pos2::new(300.0, 250.0);
        let before = under(&state, cursor);
        for zoom in [0.5, 1.0, 4.0, 16.0] {
            state.zoom_to_point(zoom, cursor);
            state.last_zoom = state.zoom.unwrap();
            let after = under(&state, cursor);
            assert!(
                (after - before).length() < 0.01,
                "zoom {zoom}: {before:?} drifted to {after:?}"
            );
        }
    }

    #[test]
    fn set_zoom_keeps_the_view_centre() {
        let mut state = laid_out();
        let centre = Pos2::new(600.0, 400.0);
        let before = under(&state, centre);
        state.set_zoom(2.0);
        state.last_zoom = state.zoom.unwrap();
        let after = under(&state, centre);
        assert!((after - before).length() < 0.01, "{before:?} -> {after:?}");
    }

    #[test]
    fn zoom_is_clamped_to_its_bounds() {
        let mut state = laid_out();
        state.set_zoom(1000.0);
        assert_eq!(state.zoom, Some(MAX_ZOOM));
        state.set_zoom(0.0001);
        assert_eq!(state.zoom, Some(MIN_ZOOM));
    }

    #[test]
    fn one_to_one_toggles_back_to_fit() {
        let mut state = laid_out();
        assert!(state.is_fit());
        state.toggle_one_to_one(Some(Pos2::new(300.0, 250.0)));
        assert_eq!(state.zoom, Some(1.0));
        assert!(!state.is_fit());
        state.toggle_one_to_one(None);
        assert!(state.is_fit());
        assert_eq!(state.pan, Vec2::ZERO);
    }

    #[test]
    fn anchors_outside_the_view_fall_back_to_the_centre() {
        let mut state = laid_out();
        let mut centred = laid_out();
        state.toggle_one_to_one(Some(Pos2::new(-40.0, -40.0)));
        centred.set_zoom(1.0);
        assert_eq!(state.pan, centred.pan);
    }
}
