//! Design-system widgets: the adjustment slider (bipolar center-fill with
//! mono value readout) and the segmented control.

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};

use crate::ui::theme;

pub struct Adjustment<'a> {
    pub label: &'a str,
    pub value: &'a mut f32,
    pub min: f32,
    pub max: f32,
    /// Value restored on double-click.
    pub default: f32,
    /// Fill from the center (± controls) instead of from the left.
    pub bipolar: bool,
    pub decimals: usize,
    /// Log-scale mapping (Kelvin temperature).
    pub logarithmic: bool,
    /// Increment for the arrow keys, both while hovering and while editing
    /// the value.
    pub step: f32,
}

const LABEL_W: f32 = 72.0;
const VALUE_W: f32 = 52.0;
const ROW_H: f32 = 20.0;
const TRACK_H: f32 = 4.0;

impl Adjustment<'_> {
    fn to_t(&self, v: f32) -> f32 {
        if self.logarithmic {
            let (lo, hi) = (self.min.ln(), self.max.ln());
            ((v.max(1e-3).ln() - lo) / (hi - lo)).clamp(0.0, 1.0)
        } else {
            ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
        }
    }

    #[allow(clippy::wrong_self_convention)]
    fn from_t(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if self.logarithmic {
            let (lo, hi) = (self.min.ln(), self.max.ln());
            (lo + t * (hi - lo)).exp()
        } else {
            self.min + t * (self.max - self.min)
        }
    }

    fn format(&self) -> String {
        let v = *self.value;
        if self.decimals == 0 {
            format!("{}", v.round() as i64)
        } else if self.bipolar && v > 0.0 {
            format!("+{v:.prec$}", prec = self.decimals)
        } else {
            format!("{v:.prec$}", prec = self.decimals)
        }
    }

    /// Returns true when the value changed.
    pub fn show(self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.add_sized(
                [LABEL_W, ROW_H],
                egui::Label::new(
                    egui::RichText::new(self.label).color(theme::text_secondary()),
                ),
            );

            let track_w = (ui.available_width() - VALUE_W - 8.0).max(40.0);
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(track_w, ROW_H), Sense::click_and_drag());

            // Arrow keys while the pointer is over the track, so a value can
            // be nudged without clicking into the readout first. Skipped
            // while the readout is being typed into, which handles its own
            // arrows — otherwise both fire and the value moves two steps.
            let edit_id = ui.id().with(("adjust-edit", self.label));
            let typing = ui.memory(|m| m.data.get_temp::<String>(edit_id)).is_some();
            if resp.hovered() && !typing {
                let (up, down) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::ArrowUp),
                        i.key_pressed(egui::Key::ArrowDown),
                    )
                });
                if up != down {
                    let step = if up { self.step } else { -self.step };
                    let new = (*self.value + step).clamp(self.min, self.max);
                    if new != *self.value {
                        *self.value = new;
                        changed = true;
                    }
                }
            }

            // Interaction first so the paint reflects this frame's value.
            if resp.double_clicked() {
                if *self.value != self.default {
                    *self.value = self.default;
                    changed = true;
                }
            } else if resp.dragged() || resp.clicked() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let t = (pos.x - rect.left()) / rect.width();
                    let new = self.from_t(t);
                    let quant = if self.decimals == 0 {
                        new.round()
                    } else {
                        let k = 10f32.powi(self.decimals as i32);
                        (new * k).round() / k
                    };
                    if quant != *self.value {
                        *self.value = quant;
                        changed = true;
                    }
                }
            }

            let painter = ui.painter();
            let track = Rect::from_center_size(
                rect.center(),
                Vec2::new(rect.width(), TRACK_H),
            );
            painter.rect_filled(track, CornerRadius::same(2), theme::bg_raised());

            let t = self.to_t(*self.value);
            let x = track.left() + t * track.width();
            let fill_active = *self.value != self.default;
            let fill_color = if fill_active {
                theme::accent()
            } else {
                theme::line_strong()
            };
            if self.bipolar {
                let cx = track.center().x;
                let (a, b) = if x >= cx { (cx, x) } else { (x, cx) };
                if (b - a) > 0.5 {
                    painter.rect_filled(
                        Rect::from_min_max(Pos2::new(a, track.top()), Pos2::new(b, track.bottom())),
                        CornerRadius::same(2),
                        fill_color,
                    );
                }
                // Center notch.
                painter.line_segment(
                    [
                        Pos2::new(cx, track.top() - 2.0),
                        Pos2::new(cx, track.bottom() + 2.0),
                    ],
                    Stroke::new(1.0, theme::line_subtle()),
                );
            } else if x - track.left() > 0.5 {
                painter.rect_filled(
                    Rect::from_min_max(track.left_top(), Pos2::new(x, track.bottom())),
                    CornerRadius::same(2),
                    fill_color,
                );
            }
            // Handle.
            let r = if resp.hovered() || resp.dragged() { 6.0 } else { 5.0 };
            painter.circle(
                Pos2::new(x, track.center().y),
                r,
                theme::text_primary(),
                Stroke::new(1.0, theme::bg_canvas()),
            );

            // Mono value readout; click to edit, arrows step while editing.
            let focus_id = ui.id().with(("adjust-edit-focus", self.label));
            let editing: Option<String> =
                ui.memory(|m| m.data.get_temp(edit_id));
            if let Some(mut text) = editing {
                let te = egui::TextEdit::singleline(&mut text)
                    .font(egui::FontId::new(12.5, theme::mono()))
                    .horizontal_align(egui::Align::RIGHT)
                    .margin(egui::Margin::symmetric(4, 2));
                let te_resp = ui.add_sized([VALUE_W, ROW_H], te);
                // Focus only on entry; re-requesting every frame would keep
                // the field alive forever (lost_focus could never fire).
                let focus_pending: bool =
                    ui.memory(|m| m.data.get_temp(focus_id).unwrap_or(false));
                if focus_pending {
                    te_resp.request_focus();
                    ui.memory_mut(|m| m.data.insert_temp(focus_id, false));
                }

                let mut commit_value = |ui: &egui::Ui, txt: &str| {
                    if let Ok(v) = txt.trim().parse::<f32>() {
                        let v = v.clamp(self.min, self.max);
                        if v != *self.value {
                            *self.value = v;
                            changed = true;
                        }
                    }
                    let _ = ui; // silence unused in some paths
                };

                let (up, down, enter, escape) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::ArrowUp),
                        i.key_pressed(egui::Key::ArrowDown),
                        i.key_pressed(egui::Key::Enter),
                        i.key_pressed(egui::Key::Escape),
                    )
                });
                if up || down {
                    let base = text.trim().parse::<f32>().unwrap_or(*self.value);
                    let stepped = (base + if up { self.step } else { -self.step })
                        .clamp(self.min, self.max);
                    if stepped != *self.value {
                        *self.value = stepped;
                        changed = true;
                    }
                    text = self.format();
                    ui.memory_mut(|m| m.data.insert_temp(edit_id, text.clone()));
                } else if escape {
                    ui.memory_mut(|m| m.data.remove_temp::<String>(edit_id));
                } else if enter || te_resp.lost_focus() {
                    commit_value(ui, &text);
                    ui.memory_mut(|m| m.data.remove_temp::<String>(edit_id));
                } else {
                    ui.memory_mut(|m| m.data.insert_temp(edit_id, text.clone()));
                }
            } else {
                let label = egui::Label::new(
                    theme::mono_text(&self.format()).color(if fill_active {
                        theme::text_primary()
                    } else {
                        theme::text_muted()
                    }),
                )
                .sense(Sense::click());
                let lresp = ui.add_sized([VALUE_W, ROW_H], label);
                if lresp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                }
                if lresp.clicked() {
                    let raw = if self.decimals == 0 {
                        format!("{}", self.value.round() as i64)
                    } else {
                        format!("{:.prec$}", *self.value, prec = self.decimals)
                    };
                    ui.memory_mut(|m| {
                        m.data.insert_temp(edit_id, raw);
                        m.data.insert_temp(focus_id, true);
                    });
                }
            }
        });
        changed
    }
}

/// Small chevron for collapsing a panel; points down while the thing it
/// controls is showing. Drawn with strokes rather than typed as a glyph:
/// egui's bundled fonts have no caret we have verified coverage for, and an
/// unlucky one renders as tofu.
pub fn caret(ui: &mut egui::Ui, pointing_down: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(26.0, 20.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_SM),
            theme::bg_raised(),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let color = if resp.hovered() {
        theme::text_primary()
    } else {
        theme::text_muted()
    };
    let c = rect.center();
    let (half_w, half_h) = (5.0, 2.8);
    let (left, tip, right) = if pointing_down {
        (
            Pos2::new(c.x - half_w, c.y - half_h),
            Pos2::new(c.x, c.y + half_h),
            Pos2::new(c.x + half_w, c.y - half_h),
        )
    } else {
        (
            Pos2::new(c.x - half_w, c.y + half_h),
            Pos2::new(c.x, c.y - half_h),
            Pos2::new(c.x + half_w, c.y + half_h),
        )
    };
    let stroke = Stroke::new(1.6, color);
    ui.painter().line_segment([left, tip], stroke);
    ui.painter().line_segment([tip, right], stroke);
    resp
}

/// Segmented control; returns true when the selection changed.
pub fn segmented<V: PartialEq + Copy>(
    ui: &mut egui::Ui,
    selected: &mut V,
    options: &[(V, &str)],
) -> bool {
    let mut changed = false;
    let pad = Vec2::new(16.0, 7.0);
    let font = egui::FontId::new(13.0, theme::semibold());
    let height = ui.text_style_height(&egui::TextStyle::Body) + pad.y * 2.0;

    // Measure segments.
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, label)| {
            ui.painter()
                .layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
                + pad.x * 2.0
        })
        .collect();
    let total: f32 = widths.iter().sum();
    let (outer, _) =
        ui.allocate_exact_size(Vec2::new(total + 8.0, height), Sense::hover());
    // Recessed near-black well; rounding on the shared token scale so the
    // control matches the rest of the UI (LG outer, MD segments ≈ concentric).
    let well = CornerRadius::same(theme::RADIUS_LG);
    ui.painter().rect_filled(outer, well, theme::bg_canvas());
    ui.painter().rect_stroke(
        outer,
        well,
        Stroke::new(1.0, theme::line_subtle()),
        egui::StrokeKind::Inside,
    );

    let mut x = outer.left() + 4.0;
    for ((value, label), w) in options.iter().zip(&widths) {
        let seg = Rect::from_min_size(
            Pos2::new(x, outer.top() + 4.0),
            Vec2::new(*w, height - 8.0),
        );
        let resp = ui.interact(seg, ui.id().with(("segment", label)), Sense::click());
        let active = *selected == *value;
        let seg_radius = CornerRadius::same(theme::RADIUS_MD);
        if active {
            ui.painter().rect_filled(seg, seg_radius, theme::accent());
        } else if resp.hovered() {
            ui.painter().rect_filled(seg, seg_radius, theme::bg_raised());
        }
        ui.painter().text(
            seg.center(),
            egui::Align2::CENTER_CENTER,
            label,
            font.clone(),
            if active {
                Color32::WHITE
            } else {
                theme::text_muted()
            },
        );
        if resp.clicked() && !active {
            *selected = *value;
            changed = true;
        }
        x += w;
    }
    changed
}


/// Sidebar list row: name left, mono count right, teal-tinted active state.
pub fn collection_row(
    ui: &mut egui::Ui,
    name: &str,
    count: usize,
    active: bool,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 38.0), Sense::click());
    let radius = CornerRadius::same(theme::RADIUS_LG);
    if active {
        ui.painter()
            .rect_filled(rect, radius, theme::accent().gamma_multiply(0.13));
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, theme::accent().gamma_multiply(0.55)),
            egui::StrokeKind::Inside,
        );
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, theme::bg_raised());
    }
    let pad = 12.0;
    ui.painter().text(
        Pos2::new(rect.left() + pad, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        if active {
            egui::FontId::new(13.5, theme::semibold())
        } else {
            theme::body_font()
        },
        if active {
            theme::text_primary()
        } else {
            theme::text_secondary()
        },
    );
    ui.painter().text(
        Pos2::new(rect.right() - pad, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        count.to_string(),
        egui::FontId::new(12.5, theme::mono()),
        theme::text_muted(),
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adjustment<'a>(value: &'a mut f32, min: f32, max: f32, log: bool) -> Adjustment<'a> {
        Adjustment {
            label: "Temp",
            value,
            min,
            max,
            default: 0.0,
            bipolar: false,
            decimals: 0,
            logarithmic: log,
            step: 50.0,
        }
    }

    #[test]
    fn log_track_position_round_trips() {
        // The white-balance slider's Kelvin range. A drag reads the handle
        // position back out of `from_t`, so a mapping that is not its own
        // inverse makes the handle jump away from the pointer.
        let mut v = 6500.0;
        let adj = adjustment(&mut v, 2000.0, 50000.0, true);
        for value in [2000.0, 3200.0, 6500.0, 20000.0, 50000.0] {
            let t = adj.to_t(value);
            assert!((0.0..=1.0).contains(&t), "{value} mapped outside the track");
            assert!(
                (adj.from_t(t) - value).abs() < value * 0.001,
                "{value} round-tripped to {}",
                adj.from_t(t)
            );
        }
    }

    #[test]
    fn log_track_spreads_the_low_end_out() {
        // The point of the log scale: daylight sits near the middle instead
        // of being crushed into the first few pixels as it would linearly.
        let mut v = 6500.0;
        let adj = adjustment(&mut v, 2000.0, 50000.0, true);
        assert!((0.3..0.5).contains(&adj.to_t(6500.0)));
        assert!(adj.to_t(3200.0) > 0.1, "tungsten is crushed against the left end");
    }

    #[test]
    fn out_of_range_values_stay_on_the_track() {
        let mut v = 6500.0;
        let adj = adjustment(&mut v, 2000.0, 50000.0, true);
        assert_eq!(adj.to_t(1.0), 0.0);
        assert_eq!(adj.to_t(99_999_999.0), 1.0);
    }

    /// Run `Adjustment` through a headless egui context so the hover /
    /// arrow-key wiring is exercised for real, not just its arithmetic.
    fn drive(events_per_frame: Vec<Vec<egui::Event>>, value: &mut f32) -> bool {
        let ctx = egui::Context::default();
        let mut changed = false;
        for events in events_per_frame {
            let input = egui::RawInput {
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 200.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| {
                changed = Adjustment {
                    label: "Exposure",
                    value,
                    min: -5.0,
                    max: 5.0,
                    default: 0.0,
                    bipolar: true,
                    decimals: 2,
                    logarithmic: false,
                    step: 0.1,
                }
                .show(ui);
            });
        }
        changed
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    /// Somewhere along the track: past the label column, short of the readout.
    const OVER_TRACK: egui::Pos2 = egui::Pos2::new(200.0, 10.0);
    const OFF_WIDGET: egui::Pos2 = egui::Pos2::new(200.0, 190.0);

    #[test]
    fn arrows_step_the_value_while_the_track_is_hovered() {
        let mut v = 0.0;
        // Frame one parks the pointer, frame two presses Up.
        let changed = drive(
            vec![
                vec![egui::Event::PointerMoved(OVER_TRACK)],
                vec![key(egui::Key::ArrowUp)],
            ],
            &mut v,
        );
        assert!(changed, "hovering the track and pressing Up did nothing");
        assert!((v - 0.1).abs() < 1e-5, "expected one +0.1 step, got {v}");

        drive(
            vec![
                vec![egui::Event::PointerMoved(OVER_TRACK)],
                vec![key(egui::Key::ArrowDown)],
                vec![key(egui::Key::ArrowDown)],
            ],
            &mut v,
        );
        assert!((v - -0.1).abs() < 1e-5, "expected two -0.1 steps, got {v}");
    }

    #[test]
    fn arrows_do_nothing_with_the_pointer_elsewhere() {
        let mut v = 0.0;
        let changed = drive(
            vec![
                vec![egui::Event::PointerMoved(OFF_WIDGET)],
                vec![key(egui::Key::ArrowUp)],
            ],
            &mut v,
        );
        assert!(!changed && v == 0.0, "stepped a slider that was not hovered");
    }

    #[test]
    fn hover_stepping_respects_the_range() {
        let mut v = 4.95;
        drive(
            vec![
                vec![egui::Event::PointerMoved(OVER_TRACK)],
                vec![key(egui::Key::ArrowUp)],
                vec![key(egui::Key::ArrowUp)],
            ],
            &mut v,
        );
        assert_eq!(v, 5.0, "walked past the maximum");
    }

    #[test]
    fn linear_track_maps_both_ends_and_the_middle() {
        let mut v = 0.0;
        let adj = adjustment(&mut v, -100.0, 100.0, false);
        assert_eq!(adj.to_t(-100.0), 0.0);
        assert_eq!(adj.to_t(100.0), 1.0);
        assert!((adj.to_t(0.0) - 0.5).abs() < 1e-6);
        assert!((adj.from_t(0.5) - 0.0).abs() < 1e-4);
    }
}
