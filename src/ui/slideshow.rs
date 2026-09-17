//! Full-screen slideshow: the current photo on black with no chrome at all,
//! for flicking through a shoot the way Lightroom's full-screen preview
//! does. Entered and left with F; Esc also leaves.
//!
//! Navigation is deliberately narrow — wheel, arrows, space — because there
//! is nothing on screen to click and no way to show what else a key would
//! have done.

use std::time::Instant;

use egui::{Color32, Pos2, Rect, Vec2};

/// Points of wheel travel that advance one photo. egui reports one mouse
/// notch as 50 points, smoothed across a few frames, so this accumulates.
const NOTCH: f32 = 50.0;

/// How long the "how to get out of here" hint stays up, and how long it then
/// takes to fade.
const HINT_HOLD: f32 = 1.8;
const HINT_FADE: f32 = 0.9;

#[derive(Default)]
pub struct SlideshowState {
    pub active: bool,
    /// Wheel travel not yet spent on a photo change.
    scroll: f32,
    /// When the mode was entered, for the fading hint.
    entered: Option<Instant>,
}

impl SlideshowState {
    pub fn enter(&mut self) {
        self.active = true;
        self.scroll = 0.0;
        self.entered = Some(Instant::now());
    }

    pub fn leave(&mut self) {
        self.active = false;
        self.scroll = 0.0;
        self.entered = None;
    }

    /// Photos to step for this frame's wheel travel: scrolling down moves
    /// forward through the library, as in every other list.
    ///
    /// At most one photo per frame, and the remainder is dropped rather than
    /// banked — a hard flick should not send the view racing on after the
    /// wheel has stopped.
    pub fn wheel_step(&mut self, delta: f32) -> i64 {
        self.scroll += delta;
        if self.scroll <= -NOTCH {
            self.scroll = 0.0;
            1
        } else if self.scroll >= NOTCH {
            self.scroll = 0.0;
            -1
        } else {
            0
        }
    }

    fn hint_alpha(&self) -> f32 {
        self.entered
            .map(|t| hint_alpha(t.elapsed().as_secs_f32()))
            .unwrap_or(0.0)
    }

    /// Whether the hint still needs animating (the caller has to keep asking
    /// for frames while it fades).
    pub fn hint_visible(&self) -> bool {
        self.hint_alpha() > 0.0
    }
}

fn hint_alpha(elapsed: f32) -> f32 {
    ((HINT_HOLD + HINT_FADE - elapsed) / HINT_FADE).clamp(0.0, 1.0)
}

/// Where the photo lands: centred in `rect`, scaled to fit, never magnified
/// past 1:1 — the same rule the loupe's fit uses. `rect` is in points and
/// `img` in image pixels.
fn fit_rect(rect: Rect, img: Vec2, ppp: f32) -> Rect {
    let scale = (rect.width() * ppp / img.x)
        .min(rect.height() * ppp / img.y)
        .min(1.0)
        / ppp;
    Rect::from_center_size(rect.center(), img * scale)
}

/// Fill the available space with black and draw the photo in it.
pub fn show(
    ui: &mut egui::Ui,
    tex: Option<(egui::TextureId, Vec2)>,
    loading: bool,
    state: &SlideshowState,
) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, 0.0, Color32::BLACK);

    match tex {
        Some((tex_id, img)) if img.x > 0.0 && img.y > 0.0 => {
            let draw = fit_rect(rect, img, ui.ctx().pixels_per_point());
            ui.painter().image(
                tex_id,
                draw,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        _ => {
            if !loading {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No image selected",
                    egui::FontId::proportional(15.0),
                    Color32::from_gray(120),
                );
            }
        }
    }

    if loading {
        ui.put(
            Rect::from_center_size(rect.center(), Vec2::splat(32.0)),
            egui::Spinner::new().size(32.0),
        );
    }

    let alpha = state.hint_alpha();
    if alpha > 0.0 {
        draw_hint(ui, rect, alpha);
    }
}

fn draw_hint(ui: &egui::Ui, rect: Rect, alpha: f32) {
    let fade = |a: f32| (a * alpha).round().clamp(0.0, 255.0) as u8;
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(
        // ASCII plus the interpunct the info overlay already uses: the
        // bundled font has no verified arrow glyphs.
        "Scroll or arrow keys to move  ·  Esc to exit".to_string(),
        egui::FontId::proportional(13.0),
        Color32::from_white_alpha(fade(215.0)),
    );
    let pad = Vec2::new(14.0, 8.0);
    let pill = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.bottom() - 46.0),
        galley.size() + pad * 2.0,
    );
    painter.rect_filled(pill, 255, Color32::from_black_alpha(fade(150.0)));
    painter.galley(pill.min + pad, galley, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wheel_notch_advances_exactly_one_photo() {
        let mut state = SlideshowState::default();
        // Smoothed over several frames, one notch still only steps once.
        assert_eq!(state.wheel_step(-20.0), 0);
        assert_eq!(state.wheel_step(-20.0), 0);
        assert_eq!(state.wheel_step(-20.0), 1);
        assert_eq!(state.wheel_step(-20.0), 0);
    }

    #[test]
    fn scroll_direction_matches_list_order() {
        let mut state = SlideshowState::default();
        assert_eq!(state.wheel_step(-NOTCH), 1, "scrolling down goes forward");
        assert_eq!(state.wheel_step(NOTCH), -1, "scrolling up goes back");
    }

    #[test]
    fn a_hard_flick_does_not_bank_steps() {
        let mut state = SlideshowState::default();
        assert_eq!(state.wheel_step(-NOTCH * 6.0), 1);
        // The overshoot is dropped: the next frame starts from nothing.
        assert_eq!(state.wheel_step(0.0), 0);
        assert_eq!(state.wheel_step(-NOTCH + 1.0), 0);
    }

    #[test]
    fn entering_resets_pending_wheel_travel() {
        let mut state = SlideshowState::default();
        state.wheel_step(-NOTCH + 1.0);
        state.enter();
        assert_eq!(state.wheel_step(-NOTCH + 1.0), 0);
    }

    #[test]
    fn the_hint_holds_then_fades_out() {
        assert_eq!(hint_alpha(0.0), 1.0);
        assert!((hint_alpha(HINT_HOLD) - 1.0).abs() < 1e-5);
        assert!((hint_alpha(HINT_HOLD + HINT_FADE / 2.0) - 0.5).abs() < 1e-5);
        assert_eq!(hint_alpha(HINT_HOLD + HINT_FADE), 0.0);
        assert_eq!(hint_alpha(60.0), 0.0);
    }

    #[test]
    fn the_photo_is_centred_and_fitted() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        // 3:2 image, wider than the 3:2 viewport is tall: width-limited.
        let draw = fit_rect(rect, Vec2::new(6000.0, 4000.0), 1.0);
        assert!((draw.width() - 1200.0).abs() < 1e-3);
        assert!((draw.height() - 800.0).abs() < 1e-3);
        assert_eq!(draw.center(), rect.center());

        // Portrait: height-limited, letterboxed left and right.
        let draw = fit_rect(rect, Vec2::new(4000.0, 6000.0), 1.0);
        assert!((draw.height() - 800.0).abs() < 1e-3);
        assert!((draw.width() - 533.333).abs() < 0.01);

        // Smaller than the screen: shown 1:1, not blown up.
        let draw = fit_rect(rect, Vec2::new(300.0, 200.0), 1.0);
        assert_eq!(draw.size(), Vec2::new(300.0, 200.0));
    }

    #[test]
    fn fit_accounts_for_the_display_scale() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        // At 2x the viewport holds twice the pixels, but the drawn size is
        // still in points, so it fills the same space.
        let draw = fit_rect(rect, Vec2::new(6000.0, 4000.0), 2.0);
        assert!((draw.width() - 1200.0).abs() < 1e-3);
    }
}
