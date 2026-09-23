//! Shared chrome for the modal dialogs.
//!
//! Dialogs grow with their contents, and Settings in particular is taller
//! than a short screen. egui clamps a window's *frame* to the viewport but
//! lets the contents run on underneath it, so the bottom of a tall dialog
//! was simply unreachable — no scrollbar, nothing to drag.
//!
//! So every dialog is built from [`window`], which keeps a margin between it
//! and the edge of the app, and puts its body in [`body`], which starts
//! scrolling once there is no more room.

use egui::{Align2, Context, Id, Rect, ScrollArea, Ui, Window};

/// Gap kept between a dialog and the edge of the app window. Without it a
/// dialog that has run out of room butts against the frame and stops
/// reading as something floating above the app.
pub const MARGIN: f32 = 24.0;

/// The rect a dialog may occupy. On a viewport too small to give up
/// `MARGIN` the margin shrinks rather than the dialog vanishing.
pub fn bounds(ctx: &Context) -> Rect {
    let frame = ctx.content_rect();
    let margin = MARGIN
        .min(frame.width() / 8.0)
        .min(frame.height() / 8.0)
        .max(0.0);
    frame.shrink(margin)
}

/// The outer window every dialog is built from. `constrain_to` does double
/// duty in egui: it both keeps the window inside the rect and caps how large
/// it may grow, which is what stops a tall dialog from overflowing.
///
/// The id is pinned to the title rather than left to egui's hash of the
/// title's rich text, so [`id_of`] can find the window again.
pub fn window(ctx: &Context, title: &str) -> Window<'static> {
    let bounds = bounds(ctx);
    Window::new(title.to_owned())
        .id(id_of(title))
        .pivot(Align2::CENTER_CENTER)
        .default_pos(bounds.center())
        .collapsible(false)
        .constrain_to(bounds)
}

/// The id [`window`] gives a dialog with this title.
pub fn id_of(title: &str) -> Id {
    Id::new(("dialog", title))
}

/// Wrap a dialog body so it scrolls instead of running off the screen.
///
/// Vertical auto-shrink stays on so a dialog that does fit is still only as
/// tall as its contents; horizontal is off so the body fills the window's
/// width rather than the widest row deciding it.
pub fn body<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, add_contents)
        .inner
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// What a dialog turned into when it was run headlessly.
    pub struct Shown {
        pub ctx: Context,
        /// The window frame egui gave it.
        pub window: Rect,
        /// Everything it actually painted, after clipping. This is the one
        /// that catches contents spilling out from under the frame — the
        /// frame itself egui has always clamped.
        pub painted: Rect,
        /// The viewport it was given.
        pub screen: Rect,
    }

    /// Run a dialog headlessly on a screen of the given size.
    ///
    /// Several frames, because egui sizes a window from what its contents
    /// measured last frame: the first pass has nothing to go on.
    pub fn show_dialog(w: f32, h: f32, title: &str, mut show: impl FnMut(&Context)) -> Shown {
        let ctx = Context::default();
        // The dialogs reach for the app's own font families, so a bare
        // context would panic on the first semibold label.
        crate::ui::theme::install_fonts(&ctx);
        let screen = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let mut output = None;
        for _ in 0..4 {
            output = Some(ctx.run_ui(input.clone(), |ui| show(ui.ctx())));
        }
        let painted = output
            .expect("ran at least once")
            .shapes
            .iter()
            .filter_map(|s| {
                let r = s.shape.visual_bounding_rect().intersect(s.clip_rect);
                (r.is_positive() && r.is_finite()).then_some(r)
            })
            .fold(Rect::NOTHING, |acc, r| acc.union(r));
        let window = ctx
            .memory(|m| m.area_rect(id_of(title)))
            .unwrap_or_else(|| panic!("dialog {title:?} was never shown"));
        Shown {
            ctx,
            window,
            painted,
            screen,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::show_dialog;
    use super::*;

    fn tall_dialog(ctx: &Context) {
        window(ctx, "Tall").resizable(false).default_width(400.0).show(ctx, |ui| {
            body(ui, |ui| {
                for i in 0..200 {
                    ui.label(format!("row {i}"));
                }
            })
        });
    }

    #[test]
    fn a_dialog_is_kept_clear_of_the_frame() {
        let ctx = Context::default();
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 1000.0),
                )),
                ..Default::default()
            },
            |_| {},
        );
        assert_eq!(bounds(&ctx), ctx.content_rect().shrink(MARGIN));
    }

    #[test]
    fn a_tiny_viewport_gives_up_the_margin_not_the_dialog() {
        let ctx = Context::default();
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(120.0, 80.0),
                )),
                ..Default::default()
            },
            |_| {},
        );
        let b = bounds(&ctx);
        assert!(b.width() > 0.0 && b.height() > 0.0);
        assert!(b.height() >= ctx.content_rect().height() * 0.7);
    }

    #[test]
    fn a_dialog_taller_than_the_screen_is_cut_down_to_fit() {
        let shown = show_dialog(1200.0, 500.0, "Tall", tall_dialog);
        let b = bounds(&shown.ctx);
        assert!(
            b.contains_rect(shown.window),
            "window {:?} escaped its bounds {b:?}",
            shown.window
        );
        // Nothing drawn outside the frame either: this is what a dialog
        // without a scrolling body gets wrong.
        assert!(
            shown.screen.contains_rect(shown.painted),
            "painted {:?} ran off the screen",
            shown.painted
        );
        // And it really did use the room it was given, rather than
        // collapsing to some minimum.
        assert!(
            shown.window.height() > b.height() * 0.8,
            "window {:?} too short",
            shown.window
        );
    }

    /// The whole reason `body` auto-shrinks vertically: a dialog with room
    /// to spare should still look like a dialog, not a full-height panel.
    #[test]
    fn a_dialog_that_fits_is_only_as_tall_as_its_contents() {
        let shown = show_dialog(1200.0, 900.0, "Short", |ctx| {
            window(ctx, "Short")
                .resizable(false)
                .default_width(400.0)
                .show(ctx, |ui| {
                    body(ui, |ui| {
                        for i in 0..3 {
                            ui.label(format!("row {i}"));
                        }
                    })
                });
        });
        assert!(
            shown.window.height() < 200.0,
            "a three-line dialog should not fill the screen: {:?}",
            shown.window
        );
    }
}
