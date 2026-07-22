//! Tone curve editor: draggable points over a histogram backdrop, with
//! composite + per-channel (R/G/B) point curves.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

use crate::engine::curve::MonotoneCubic;
use crate::engine::params::ToneCurves;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Channel {
    #[default]
    Composite,
    Red,
    Green,
    Blue,
}

impl Channel {
    fn color(self) -> Color32 {
        match self {
            Channel::Composite => Color32::from_gray(230),
            Channel::Red => Color32::from_rgb(235, 90, 90),
            Channel::Green => Color32::from_rgb(90, 220, 90),
            Channel::Blue => Color32::from_rgb(110, 130, 250),
        }
    }
}

#[derive(Default)]
pub struct ToneCurveState {
    pub channel: Channel,
    drag: Option<usize>,
}

const CURVE_SIZE: f32 = 240.0;
const POINT_RADIUS: f32 = 4.0;
const HIT_RADIUS: f32 = 10.0;

pub fn show(
    ui: &mut egui::Ui,
    state: &mut ToneCurveState,
    curves: &mut ToneCurves,
    histogram: Option<&[[u32; 256]; 3]>,
) -> bool {
    let mut changed = false;

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Tone Curve").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Linear").clicked() {
                *active_points_mut(curves, state.channel) = vec![[0.0, 0.0], [1.0, 1.0]];
                changed = true;
            }
        });
    });
    ui.horizontal(|ui| {
        for (label, ch) in [
            ("RGB", Channel::Composite),
            ("R", Channel::Red),
            ("G", Channel::Green),
            ("B", Channel::Blue),
        ] {
            let selected = state.channel == ch;
            let text = egui::RichText::new(label).color(if selected {
                ch.color()
            } else {
                Color32::from_gray(110)
            });
            if ui.selectable_label(selected, text).clicked() {
                state.channel = ch;
                state.drag = None;
            }
        }
    });

    let width = ui.available_width().min(CURVE_SIZE + 60.0);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, CURVE_SIZE), Sense::click_and_drag());
    let painter = ui.painter_at(rect.expand(2.0));

    painter.rect_filled(rect, 4.0, Color32::from_gray(24));
    for i in 1..4 {
        let f = i as f32 / 4.0;
        let x = rect.left() + f * rect.width();
        let y = rect.top() + f * rect.height();
        let grid = Stroke::new(1.0, Color32::from_gray(40));
        painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], grid);
        painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], grid);
    }

    if let Some(hist) = histogram {
        draw_histogram(&painter, rect, hist);
    }

    let to_screen = |p: [f32; 2]| -> Pos2 {
        Pos2::new(
            rect.left() + p[0] * rect.width(),
            rect.bottom() - p[1] * rect.height(),
        )
    };
    let from_screen = |pos: Pos2| -> [f32; 2] {
        [
            ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
            ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0),
        ]
    };

    // Draw the active channel's curve (others faint).
    for (ch, faint) in [
        (Channel::Red, true),
        (Channel::Green, true),
        (Channel::Blue, true),
        (Channel::Composite, true),
        (state.channel, false),
    ] {
        if faint && ch == state.channel {
            continue;
        }
        let pts = active_points(curves, ch);
        if faint && MonotoneCubic::is_identity(pts) {
            continue; // don't clutter with identity lines
        }
        let spline = MonotoneCubic::new(pts);
        let mut path = Vec::with_capacity(101);
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            path.push(to_screen([x, spline.eval(x).clamp(0.0, 1.0)]));
        }
        let color = if faint {
            ch.color().gamma_multiply(0.25)
        } else {
            ch.color()
        };
        painter.add(egui::Shape::line(path, Stroke::new(if faint { 1.0 } else { 1.6 }, color)));
    }

    // Control points of the active channel.
    let points = active_points_mut(curves, state.channel);
    for (i, p) in points.iter().enumerate() {
        let pos = to_screen(*p);
        let active = state.drag == Some(i);
        painter.circle(
            pos,
            if active { POINT_RADIUS + 1.5 } else { POINT_RADIUS },
            Color32::from_gray(15),
            Stroke::new(1.5, state.channel.color()),
        );
    }

    // Interaction.
    if let Some(pointer) = response.interact_pointer_pos() {
        if response.drag_started() || (response.clicked() && state.drag.is_none()) {
            // Find an existing point near the pointer.
            let near = points
                .iter()
                .enumerate()
                .map(|(i, p)| (i, to_screen(*p).distance(pointer)))
                .filter(|(_, d)| *d < HIT_RADIUS)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            match near {
                Some(i) => state.drag = Some(i),
                None => {
                    // Add a new point at the pointer x.
                    let p = from_screen(pointer);
                    let idx = points.partition_point(|q| q[0] < p[0]);
                    points.insert(idx, p);
                    state.drag = Some(idx);
                    changed = true;
                }
            }
        }
        if let Some(i) = state.drag {
            if response.dragged() && i < points.len() {
                let mut p = from_screen(pointer);
                // Endpoints keep their x; interior points stay between
                // neighbors.
                if i == 0 {
                    p[0] = points[0][0];
                } else if i == points.len() - 1 {
                    p[0] = points[i][0];
                } else {
                    let lo = points[i - 1][0] + 0.01;
                    let hi = points[i + 1][0] - 0.01;
                    p[0] = p[0].clamp(lo, hi.max(lo));
                }
                if points[i] != p {
                    points[i] = p;
                    changed = true;
                }
            }
        }
    }
    if response.drag_stopped() || (!response.dragged() && !response.clicked()) {
        if !response.is_pointer_button_down_on() {
            state.drag = None;
        }
    }
    // Double-click an interior point to remove it.
    if response.double_clicked() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let near = points
                .iter()
                .enumerate()
                .skip(1)
                .take(points.len().saturating_sub(2))
                .map(|(i, p)| (i, to_screen(*p).distance(pointer)))
                .filter(|(_, d)| *d < HIT_RADIUS)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            if let Some(i) = near {
                points.remove(i);
                state.drag = None;
                changed = true;
            }
        }
    }

    changed
}

fn active_points(curves: &ToneCurves, ch: Channel) -> &Vec<[f32; 2]> {
    match ch {
        Channel::Composite => &curves.composite,
        Channel::Red => &curves.red,
        Channel::Green => &curves.green,
        Channel::Blue => &curves.blue,
    }
}

fn active_points_mut(curves: &mut ToneCurves, ch: Channel) -> &mut Vec<[f32; 2]> {
    match ch {
        Channel::Composite => &mut curves.composite,
        Channel::Red => &mut curves.red,
        Channel::Green => &mut curves.green,
        Channel::Blue => &mut curves.blue,
    }
}

fn draw_histogram(painter: &egui::Painter, rect: Rect, hist: &[[u32; 256]; 3]) {
    let max = hist
        .iter()
        .flat_map(|ch| ch.iter())
        .copied()
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let colors = [
        Color32::from_rgb(200, 80, 80),
        Color32::from_rgb(80, 200, 80),
        Color32::from_rgb(90, 110, 220),
    ];
    // Filled with vertical bars (epaint only fills convex polygons).
    let bar_w = (rect.width() / 256.0).ceil();
    for (ch, color) in hist.iter().zip(colors) {
        let fill = color.gamma_multiply(0.18);
        for (i, &count) in ch.iter().enumerate() {
            if count == 0 {
                continue;
            }
            let x = rect.left() + (i as f32 / 255.0) * rect.width();
            // Sqrt scale keeps small counts visible.
            let h = (count as f32 / max).sqrt() * rect.height() * 0.9;
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(x, rect.bottom() - h),
                    Pos2::new(x + bar_w, rect.bottom()),
                ),
                0.0,
                fill,
            );
        }
    }
}
