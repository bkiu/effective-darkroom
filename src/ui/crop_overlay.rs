//! Interactive crop overlay drawn over the loupe while the crop tool is
//! active: dimmed surround, thirds grid, draggable corners/edges.

use egui::{Color32, CursorIcon, Pos2, Rect, Sense, Stroke, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AspectChoice {
    #[default]
    Free,
    Original,
    Square,
    R3x2,
    R2x3,
    R4x3,
    R16x9,
}

impl AspectChoice {
    pub const ALL: [AspectChoice; 7] = [
        AspectChoice::Free,
        AspectChoice::Original,
        AspectChoice::Square,
        AspectChoice::R3x2,
        AspectChoice::R2x3,
        AspectChoice::R4x3,
        AspectChoice::R16x9,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AspectChoice::Free => "Free",
            AspectChoice::Original => "Original",
            AspectChoice::Square => "1 : 1",
            AspectChoice::R3x2 => "3 : 2",
            AspectChoice::R2x3 => "2 : 3",
            AspectChoice::R4x3 => "4 : 3",
            AspectChoice::R16x9 => "16 : 9",
        }
    }

    /// Target pixel aspect (w/h); `original` is the safe-area aspect.
    pub fn ratio(self, original: f32) -> Option<f32> {
        match self {
            AspectChoice::Free => None,
            AspectChoice::Original => Some(original),
            AspectChoice::Square => Some(1.0),
            AspectChoice::R3x2 => Some(1.5),
            AspectChoice::R2x3 => Some(2.0 / 3.0),
            AspectChoice::R4x3 => Some(4.0 / 3.0),
            AspectChoice::R16x9 => Some(16.0 / 9.0),
        }
    }
}

const MIN_SIZE: f32 = 0.05;

/// Everything the overlay needs. `rect` is normalized within the safe area.
pub struct CropEdit<'a> {
    pub rect: &'a mut [f32; 4],
    /// Safe area within the displayed (full rotated preview) texture, uv.
    pub safe_uv: [f32; 4],
    /// Pixel aspect (w/h) of the safe area, for aspect-locked drags.
    pub safe_px_aspect: f32,
    pub aspect: AspectChoice,
}

/// Draw + edit. `image_rect` is the on-screen rect of the displayed preview.
pub fn show(ui: &mut egui::Ui, image_rect: Rect, edit: &mut CropEdit<'_>) -> bool {
    let mut changed = false;

    let safe = Rect::from_min_size(
        image_rect.min
            + Vec2::new(
                edit.safe_uv[0] * image_rect.width(),
                edit.safe_uv[1] * image_rect.height(),
            ),
        Vec2::new(
            edit.safe_uv[2] * image_rect.width(),
            edit.safe_uv[3] * image_rect.height(),
        ),
    );
    let r = *edit.rect;
    let crop = Rect::from_min_size(
        safe.min + Vec2::new(r[0] * safe.width(), r[1] * safe.height()),
        Vec2::new(r[2] * safe.width(), r[3] * safe.height()),
    );

    // Dim outside the crop.
    let dim = Color32::from_black_alpha(140);
    let painter = ui.painter_at(image_rect);
    painter.rect_filled(
        Rect::from_min_max(image_rect.min, Pos2::new(image_rect.max.x, crop.min.y)),
        0.0,
        dim,
    );
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(image_rect.min.x, crop.max.y), image_rect.max),
        0.0,
        dim,
    );
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(image_rect.min.x, crop.min.y),
            Pos2::new(crop.min.x, crop.max.y),
        ),
        0.0,
        dim,
    );
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(crop.max.x, crop.min.y),
            Pos2::new(image_rect.max.x, crop.max.y),
        ),
        0.0,
        dim,
    );

    // Thirds grid + border.
    let grid = Stroke::new(1.0, Color32::from_white_alpha(70));
    for i in 1..3 {
        let f = i as f32 / 3.0;
        painter.line_segment(
            [
                Pos2::new(crop.min.x + f * crop.width(), crop.min.y),
                Pos2::new(crop.min.x + f * crop.width(), crop.max.y),
            ],
            grid,
        );
        painter.line_segment(
            [
                Pos2::new(crop.min.x, crop.min.y + f * crop.height()),
                Pos2::new(crop.max.x, crop.min.y + f * crop.height()),
            ],
            grid,
        );
    }
    painter.rect_stroke(
        crop,
        0.0,
        Stroke::new(1.5, Color32::from_white_alpha(220)),
        egui::StrokeKind::Outside,
    );

    // Handles.
    let hs = 14.0;
    let corner = |p: Pos2| Rect::from_center_size(p, Vec2::splat(hs));
    let handles: [(&str, Rect, CursorIcon); 8] = [
        ("nw", corner(crop.left_top()), CursorIcon::ResizeNwSe),
        ("ne", corner(crop.right_top()), CursorIcon::ResizeNeSw),
        ("sw", corner(crop.left_bottom()), CursorIcon::ResizeNeSw),
        ("se", corner(crop.right_bottom()), CursorIcon::ResizeNwSe),
        (
            "n",
            Rect::from_center_size(crop.center_top(), Vec2::new(crop.width() - hs, hs)),
            CursorIcon::ResizeVertical,
        ),
        (
            "s",
            Rect::from_center_size(crop.center_bottom(), Vec2::new(crop.width() - hs, hs)),
            CursorIcon::ResizeVertical,
        ),
        (
            "w",
            Rect::from_center_size(crop.left_center(), Vec2::new(hs, crop.height() - hs)),
            CursorIcon::ResizeHorizontal,
        ),
        (
            "e",
            Rect::from_center_size(crop.right_center(), Vec2::new(hs, crop.height() - hs)),
            CursorIcon::ResizeHorizontal,
        ),
    ];
    for (_, hr, _) in handles.iter().take(4) {
        painter.rect_filled(hr.shrink(3.0), 2.0, Color32::from_white_alpha(230));
    }

    // The target aspect expressed in safe-normalized units (w_norm/h_norm).
    let aspect_norm = edit
        .aspect
        .ratio(edit.safe_px_aspect)
        .map(|a| a / edit.safe_px_aspect);

    // Interactions: handles first (higher priority), then move.
    let to_norm = |d: Vec2| Vec2::new(d.x / safe.width(), d.y / safe.height());
    let mut rect = *edit.rect;
    let mut consumed = false;

    for (name, hr, icon) in handles {
        let resp = ui.interact(
            hr,
            ui.id().with(("crop-handle", name)),
            Sense::click_and_drag(),
        );
        if resp.hovered() {
            ui.ctx().set_cursor_icon(icon);
        }
        if resp.dragged() {
            // With an aspect lock, only corner handles resize.
            if aspect_norm.is_some() && name.len() == 1 {
                consumed = true;
                break;
            }
            let d = to_norm(resp.drag_delta());
            let (mut x0, mut y0) = (rect[0], rect[1]);
            let (mut x1, mut y1) = (rect[0] + rect[2], rect[1] + rect[3]);
            if name.contains('w') {
                x0 = (x0 + d.x).clamp(0.0, x1 - MIN_SIZE);
            }
            if name.contains('e') {
                x1 = (x1 + d.x).clamp(x0 + MIN_SIZE, 1.0);
            }
            if name.contains('n') {
                y0 = (y0 + d.y).clamp(0.0, y1 - MIN_SIZE);
            }
            if name.contains('s') {
                y1 = (y1 + d.y).clamp(y0 + MIN_SIZE, 1.0);
            }
            if let Some(a) = aspect_norm {
                // Corner drag: derive height from width, anchored opposite.
                let w = x1 - x0;
                let h = (w / a).clamp(MIN_SIZE, 1.0);
                let w = h * a;
                match name {
                    "nw" => {
                        x0 = x1 - w;
                        y0 = y1 - h;
                    }
                    "ne" => {
                        x1 = x0 + w;
                        y0 = y1 - h;
                    }
                    "sw" => {
                        x0 = x1 - w;
                        y1 = y0 + h;
                    }
                    _ => {
                        x1 = x0 + w;
                        y1 = y0 + h;
                    }
                }
                // Clamp back inside; shrink if needed keeping aspect.
                if x0 < 0.0 || y0 < 0.0 || x1 > 1.0 || y1 > 1.0 {
                    consumed = true;
                    break;
                }
            }
            rect = [x0, y0, x1 - x0, y1 - y0];
            consumed = true;
            break;
        }
        if resp.is_pointer_button_down_on() {
            consumed = true;
        }
    }

    if !consumed {
        let resp = ui.interact(
            crop.shrink(hs / 2.0),
            ui.id().with("crop-move"),
            Sense::click_and_drag(),
        );
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Move);
        }
        if resp.dragged() {
            let d = to_norm(resp.drag_delta());
            rect[0] = (rect[0] + d.x).clamp(0.0, 1.0 - rect[2]);
            rect[1] = (rect[1] + d.y).clamp(0.0, 1.0 - rect[3]);
        }
    }

    if rect != *edit.rect {
        *edit.rect = rect;
        changed = true;
    }
    changed
}

/// Snap the rect to a target aspect (used when the aspect choice changes),
/// keeping the center and fitting within the safe area.
pub fn apply_aspect(rect: &mut [f32; 4], aspect: AspectChoice, safe_px_aspect: f32) -> bool {
    let Some(a) = aspect.ratio(safe_px_aspect) else {
        return false;
    };
    let a_norm = a / safe_px_aspect;
    let (cx, cy) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    let (mut w, mut h) = (rect[2], rect[3]);
    // Fit the target aspect within the current extents.
    if w / h > a_norm {
        w = h * a_norm;
    } else {
        h = w / a_norm;
    }
    // Scale up as much as possible around the center.
    let scale = (1.0f32 / w).min(1.0 / h).min(
        (2.0 * cx.min(1.0 - cx) / w).min(2.0 * cy.min(1.0 - cy) / h),
    );
    if scale > 1.0 {
        w *= scale.min(1.0 / w.max(h / a_norm));
        h = w / a_norm;
    }
    let new = [
        (cx - w / 2.0).clamp(0.0, 1.0 - w),
        (cy - h / 2.0).clamp(0.0, 1.0 - h),
        w.min(1.0),
        h.min(1.0),
    ];
    if new != *rect {
        *rect = new;
        true
    } else {
        false
    }
}
