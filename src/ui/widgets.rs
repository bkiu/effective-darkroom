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

            // Mono value readout.
            ui.add_sized(
                [VALUE_W, ROW_H],
                egui::Label::new(
                    theme::mono_text(&self.format()).color(if fill_active {
                        theme::text_primary()
                    } else {
                        theme::text_muted()
                    }),
                ),
            );
        });
        changed
    }
}

/// Segmented control; returns true when the selection changed.
pub fn segmented<V: PartialEq + Copy>(
    ui: &mut egui::Ui,
    selected: &mut V,
    options: &[(V, &str)],
) -> bool {
    let mut changed = false;
    let pad = Vec2::new(14.0, 5.0);
    let font = theme::body_font();
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
    let (outer, _) = ui.allocate_exact_size(Vec2::new(total + 4.0, height), Sense::hover());
    ui.painter()
        .rect_filled(outer, CornerRadius::same(theme::RADIUS_MD), theme::bg_canvas());

    let mut x = outer.left() + 2.0;
    for ((value, label), w) in options.iter().zip(&widths) {
        let seg = Rect::from_min_size(
            Pos2::new(x, outer.top() + 2.0),
            Vec2::new(*w, height - 4.0),
        );
        let resp = ui.interact(seg, ui.id().with(("segment", label)), Sense::click());
        let active = *selected == *value;
        if active {
            ui.painter().rect_filled(
                seg,
                CornerRadius::same(theme::RADIUS_SM),
                theme::bg_hover(),
            );
            ui.painter().rect_stroke(
                seg,
                CornerRadius::same(theme::RADIUS_SM),
                Stroke::new(1.0, theme::accent()),
                egui::StrokeKind::Inside,
            );
        } else if resp.hovered() {
            ui.painter().rect_filled(
                seg,
                CornerRadius::same(theme::RADIUS_SM),
                theme::bg_raised(),
            );
        }
        ui.painter().text(
            seg.center(),
            egui::Align2::CENTER_CENTER,
            label,
            font.clone(),
            if active {
                theme::text_primary()
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
