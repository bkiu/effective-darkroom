//! Horizontal filmstrip shown under the loupe/develop views, like
//! Lightroom's. Reuses the grid's thumbnail cache.

use egui::{Color32, Rect, Sense, Vec2};

use crate::catalog::ImageRecord;
use crate::jobs::Jobs;
use crate::ui::grid::GridState;
use crate::ui::theme;

const STRIP_CELL: f32 = 76.0;
const PAD: f32 = 4.0;

/// Height the bottom panel should reserve.
pub const STRIP_HEIGHT: f32 = STRIP_CELL + 14.0;

/// Returns the id of a clicked photo. `scroll_to_current` centers the strip
/// on the current photo (pass true when the current image just changed).
pub fn show(
    ui: &mut egui::Ui,
    state: &mut GridState,
    images: &[ImageRecord],
    jobs: &Jobs,
    current: Option<i64>,
    scroll_to_current: bool,
) -> Option<i64> {
    let mut clicked = None;
    egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(PAD);
                for rec in images {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(STRIP_CELL), Sense::click());
                    let is_current = current == Some(rec.id);
                    if is_current && scroll_to_current {
                        ui.scroll_to_rect(rect.expand(STRIP_CELL), None);
                    }
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }

                    if state.textures.peek(&rec.id).is_none()
                        && !state.pending.contains(&rec.id)
                        && !state.failed.contains(&rec.id)
                    {
                        state.pending.insert(rec.id);
                        jobs.request_thumb(rec.clone());
                    }

                    let painter = ui.painter();
                    painter.rect_filled(rect, 3.0, theme::bg_raised());
                    if let Some(tex) = state.textures.get(&rec.id) {
                        let size = tex.size_vec2();
                        let scale =
                            ((STRIP_CELL - PAD) / size.x).min((STRIP_CELL - PAD) / size.y);
                        let draw = Rect::from_center_size(rect.center(), size * scale);
                        painter.image(
                            tex.id(),
                            draw,
                            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    if is_current {
                        painter.rect_stroke(
                            rect,
                            3.0,
                            egui::Stroke::new(2.0, theme::accent()),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let dot = match rec.flag {
                        crate::catalog::Flag::Pick => Some(theme::flag_pick()),
                        crate::catalog::Flag::Reject => Some(theme::danger()),
                        crate::catalog::Flag::Unflagged => None,
                    };
                    if let Some(color) = dot {
                        painter.circle_filled(
                            rect.right_top() + Vec2::new(-7.0, 7.0),
                            4.0,
                            color,
                        );
                    }
                    if response.clicked() {
                        clicked = Some(rec.id);
                    }
                    response.on_hover_text(&rec.file_name);
                }
                ui.add_space(PAD);
            });
        });
    clicked
}
