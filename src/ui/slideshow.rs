//! Full-screen slideshow: the current photo on black with no chrome at all,
//! for flicking through a shoot the way Lightroom's full-screen preview
//! does. Entered and left with F; Esc also leaves.
//!
//! The keyboard is deliberately narrow — wheel, arrows and space to move,
//! P/U/X to flag — because there is nothing on screen to click. Flagging
//! would otherwise be invisible, so it answers with a toast at the bottom
//! centre, as Lightroom's full-screen preview does.

use std::time::Instant;

use egui::{Color32, Pos2, Rect, Vec2};

use crate::catalog::Flag;
use crate::ui::theme;

/// Points of wheel travel that advance one photo. egui reports one mouse
/// notch as 50 points, smoothed across a few frames, so this accumulates.
const NOTCH: f32 = 50.0;

/// How long the "how to get out of here" hint stays up, and how long it then
/// takes to fade.
const HINT_HOLD: f32 = 1.8;
const HINT_FADE: f32 = 0.9;

/// The same, for the toast that reports a flag change. Shorter: it confirms
/// something the user just did rather than teaching them anything, and it
/// has to be gone before the next photo.
const TOAST_HOLD: f32 = 1.0;
const TOAST_FADE: f32 = 0.45;

/// A brief note at the bottom centre saying what a keystroke just did.
struct Toast {
    text: String,
    tint: Color32,
    at: Instant,
}

#[derive(Default)]
pub struct SlideshowState {
    pub active: bool,
    /// Wheel travel not yet spent on a photo change.
    scroll: f32,
    /// When the mode was entered, for the fading hint.
    entered: Option<Instant>,
    /// The last action worth reporting, still fading out.
    toast: Option<Toast>,
}

impl SlideshowState {
    pub fn enter(&mut self) {
        self.active = true;
        self.scroll = 0.0;
        self.entered = Some(Instant::now());
        self.toast = None;
    }

    pub fn leave(&mut self) {
        self.active = false;
        self.scroll = 0.0;
        self.entered = None;
        self.toast = None;
    }

    /// Report a flag change. The hint is dismissed with it: both sit at the
    /// bottom centre, and once the user is flagging they have clearly found
    /// their way around.
    pub fn flag_toast(&mut self, flag: Flag) {
        let (text, tint) = match flag {
            Flag::Pick => ("⚑  Picked", theme::flag_pick()),
            Flag::Reject => ("✖  Rejected", theme::danger()),
            Flag::Unflagged => ("⚐  Unflagged", theme::text_primary()),
        };
        self.entered = None;
        self.toast = Some(Toast {
            text: text.to_string(),
            tint,
            at: Instant::now(),
        });
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
            .map(|t| fade_alpha(t.elapsed().as_secs_f32(), HINT_HOLD, HINT_FADE))
            .unwrap_or(0.0)
    }

    fn toast_alpha(&self) -> f32 {
        self.toast
            .as_ref()
            .map(|t| fade_alpha(t.at.elapsed().as_secs_f32(), TOAST_HOLD, TOAST_FADE))
            .unwrap_or(0.0)
    }

    /// Whether anything on the overlay still needs animating (the caller has
    /// to keep asking for frames while it fades).
    pub fn overlay_visible(&self) -> bool {
        self.hint_alpha() > 0.0 || self.toast_alpha() > 0.0
    }
}

/// Opaque for `hold` seconds, then a linear fade over `fade` seconds.
fn fade_alpha(elapsed: f32, hold: f32, fade: f32) -> f32 {
    ((hold + fade - elapsed) / fade).clamp(0.0, 1.0)
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

    // The toast replaces the hint rather than stacking on it: they share the
    // spot, and `flag_toast` has already cancelled the hint anyway.
    let toast_alpha = state.toast_alpha();
    if toast_alpha > 0.0 {
        if let Some(toast) = &state.toast {
            draw_pill(ui, rect, &toast.text, toast.tint, 15.0, toast_alpha);
        }
    } else {
        let alpha = state.hint_alpha();
        if alpha > 0.0 {
            // ASCII plus the interpunct the info overlay already uses: the
            // bundled font has no verified arrow glyphs.
            let hint = "Scroll or arrow keys to move  ·  P / U / X to flag  ·  Esc to exit";
            draw_pill(ui, rect, hint, Color32::from_gray(215), 13.0, alpha);
        }
    }
}

/// The bottom-centre pill both the hint and the toast are drawn as.
fn draw_pill(ui: &egui::Ui, rect: Rect, text: &str, tint: Color32, size: f32, alpha: f32) {
    let scale = |a: u8| ((a as f32) * alpha).round().clamp(0.0, 255.0) as u8;
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(size),
        tint.gamma_multiply(alpha),
    );
    let pad = Vec2::new(14.0, 8.0);
    let pill = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.bottom() - 46.0),
        galley.size() + pad * 2.0,
    );
    painter.rect_filled(pill, 255, Color32::from_black_alpha(scale(150)));
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
        let a = |t| fade_alpha(t, HINT_HOLD, HINT_FADE);
        assert_eq!(a(0.0), 1.0);
        assert!((a(HINT_HOLD) - 1.0).abs() < 1e-5);
        assert!((a(HINT_HOLD + HINT_FADE / 2.0) - 0.5).abs() < 1e-5);
        assert_eq!(a(HINT_HOLD + HINT_FADE), 0.0);
        assert_eq!(a(60.0), 0.0);
    }

    #[test]
    fn a_flag_toast_replaces_the_hint() {
        let mut state = SlideshowState::default();
        state.enter();
        assert!(state.hint_alpha() > 0.0);
        state.flag_toast(Flag::Reject);
        assert_eq!(state.hint_alpha(), 0.0, "the hint gives up its spot");
        assert!(state.toast_alpha() > 0.0);
        assert!(state.overlay_visible());
    }

    #[test]
    fn the_toast_is_briefer_than_the_hint() {
        const { assert!(TOAST_HOLD + TOAST_FADE < HINT_HOLD + HINT_FADE) };
        let a = |t| fade_alpha(t, TOAST_HOLD, TOAST_FADE);
        assert_eq!(a(TOAST_HOLD), 1.0);
        assert_eq!(a(TOAST_HOLD + TOAST_FADE), 0.0);
    }

    #[test]
    fn leaving_clears_the_toast() {
        let mut state = SlideshowState::default();
        state.enter();
        state.flag_toast(Flag::Pick);
        state.leave();
        assert!(!state.overlay_visible());
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
