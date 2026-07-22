//! Virtualized thumbnail grid with click / ctrl-click / shift-click selection.

use std::collections::HashSet;
use std::num::NonZeroUsize;

use egui::{Color32, Rect, Sense, TextureHandle, Vec2};
use lru::LruCache;

use crate::catalog::ImageRecord;
use crate::jobs::Jobs;

const CELL: f32 = 160.0;
const PAD: f32 = 8.0;

pub struct GridState {
    pub textures: LruCache<i64, TextureHandle>,
    pub pending: HashSet<i64>,
    pub failed: HashSet<i64>,
    pub selection: HashSet<i64>,
    /// Anchor for shift-click range selection (index into images).
    pub anchor: Option<usize>,
    /// Image the user wants opened in loupe/develop.
    pub activated: Option<i64>,
}

impl Default for GridState {
    fn default() -> Self {
        Self {
            textures: LruCache::new(NonZeroUsize::new(1024).unwrap()),
            pending: HashSet::new(),
            failed: HashSet::new(),
            selection: HashSet::new(),
            anchor: None,
            activated: None,
        }
    }
}

impl GridState {
    pub fn on_thumb_ready(&mut self, ctx: &egui::Context, image_id: i64, img: egui::ColorImage) {
        self.pending.remove(&image_id);
        let handle = ctx.load_texture(
            format!("thumb-{image_id}"),
            img,
            egui::TextureOptions::LINEAR,
        );
        self.textures.put(image_id, handle);
    }

    pub fn on_thumb_failed(&mut self, image_id: i64) {
        self.pending.remove(&image_id);
        self.failed.insert(image_id);
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut GridState, images: &[ImageRecord], jobs: &Jobs) {
    if images.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new("Library is empty — use Import… to add photos")
                    .weak()
                    .size(15.0),
            );
        });
        return;
    }

    let avail = ui.available_width();
    let cols = ((avail + PAD) / (CELL + PAD)).floor().max(1.0) as usize;
    let rows = images.len().div_ceil(cols);
    let row_height = CELL + PAD;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, row_height, rows, |ui, row_range| {
            for row in row_range {
                ui.horizontal(|ui| {
                    ui.add_space(PAD / 2.0);
                    for col in 0..cols {
                        let idx = row * cols + col;
                        if idx >= images.len() {
                            break;
                        }
                        cell(ui, state, images, idx, jobs);
                    }
                });
            }
        });
}

fn cell(ui: &mut egui::Ui, state: &mut GridState, images: &[ImageRecord], idx: usize, jobs: &Jobs) {
    let rec = &images[idx];
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(CELL), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }

    // Lazily request the thumbnail the first time the cell becomes visible.
    if state.textures.peek(&rec.id).is_none()
        && !state.pending.contains(&rec.id)
        && !state.failed.contains(&rec.id)
    {
        state.pending.insert(rec.id);
        jobs.request_thumb(rec.clone());
    }

    let selected = state.selection.contains(&rec.id);
    let painter = ui.painter();
    let bg = if selected {
        ui.visuals().selection.bg_fill.gamma_multiply(0.4)
    } else {
        Color32::from_gray(28)
    };
    painter.rect_filled(rect, 4.0, bg);

    if let Some(tex) = state.textures.get(&rec.id) {
        let size = tex.size_vec2();
        let scale = ((CELL - PAD) / size.x).min((CELL - PAD) / size.y);
        let img_size = size * scale;
        let img_rect = Rect::from_center_size(rect.center(), img_size);
        painter.image(
            tex.id(),
            img_rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else if state.failed.contains(&rec.id) {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "⚠",
            egui::FontId::proportional(24.0),
            Color32::from_gray(120),
        );
    }

    if selected {
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }

    response.clone().on_hover_text(format!(
        "{}\n{} {}",
        rec.file_name, rec.camera_make, rec.camera_model
    ));

    if response.double_clicked() {
        state.activated = Some(rec.id);
    } else if response.clicked() {
        let mods = ui.input(|i| i.modifiers);
        if mods.shift {
            let anchor = state.anchor.unwrap_or(idx);
            let (lo, hi) = (anchor.min(idx), anchor.max(idx));
            if !mods.ctrl {
                state.selection.clear();
            }
            for r in &images[lo..=hi] {
                state.selection.insert(r.id);
            }
        } else if mods.ctrl {
            if !state.selection.insert(rec.id) {
                state.selection.remove(&rec.id);
            }
            state.anchor = Some(idx);
        } else {
            state.selection.clear();
            state.selection.insert(rec.id);
            state.anchor = Some(idx);
        }
    }
}
