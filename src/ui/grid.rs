//! Virtualized thumbnail grid with click / ctrl-click / shift-click selection.

use std::collections::HashSet;
use std::num::NonZeroUsize;

use egui::{Color32, Rect, Sense, TextureHandle, Vec2};
use lru::LruCache;

use crate::catalog::{Collection, Flag, ImageRecord};
use crate::jobs::Jobs;
use crate::ui::theme;

const CELL: f32 = 160.0;
const PAD: f32 = 8.0;

/// Upper bound on frames spent chasing the bottom of the grid at startup, so a
/// library whose content height never stops growing can't pin the view there.
const SETTLE_FRAMES: u8 = 8;

pub struct GridState {
    pub textures: LruCache<i64, TextureHandle>,
    pub pending: HashSet<i64>,
    pub failed: HashSet<i64>,
    pub selection: HashSet<i64>,
    /// Anchor for shift-click range selection (index into images).
    pub anchor: Option<usize>,
    /// Image the user wants opened in loupe/develop.
    pub activated: Option<i64>,
    /// Frames left to force the grid to the end (the newest photos, since the
    /// library sorts oldest-first). Set at startup and re-applied until the
    /// scroll actually lands at the bottom: egui lays the first frame out
    /// against an unconstrained viewport, so its column count — and with it
    /// the content height egui clamps the offset against — is nothing like
    /// the real one, and a single-shot jump strands the view near the top.
    pub scroll_to_bottom: u8,
    /// One-shot: restore this scroll offset (collection switches remember
    /// where each view was).
    pub restore_scroll: Option<f32>,
    /// Scroll offset as of the last drawn frame, for the app to stash away
    /// before switching views.
    pub scroll_offset: f32,
    /// Content height of the last drawn frame. The startup jump waits for this
    /// to stop changing: egui lays the first frame out against an
    /// unconstrained viewport, and the bottom of *that* layout is nowhere near
    /// the bottom of the real one.
    last_content_h: f32,
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
            scroll_to_bottom: SETTLE_FRAMES,
            restore_scroll: None,
            scroll_offset: 0.0,
            last_content_h: f32::NAN,
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

#[derive(Default)]
pub struct GridResponse {
    /// User chose "Paste settings" from a cell context menu; apply the copied
    /// params to the whole selection.
    pub paste_to_selection: bool,
    /// User chose "Delete…" from a cell context menu.
    pub delete_selection: bool,
    /// Set a flag on the selection.
    pub set_flag: Option<Flag>,
    /// Add the selection to this collection id.
    pub add_to_collection: Option<i64>,
    /// Remove the selection from the currently viewed collection.
    pub remove_from_collection: bool,
}

/// Extra context the grid needs for its context menu.
pub struct GridContext<'a> {
    pub can_paste: bool,
    pub collections: &'a [Collection],
    /// Set when the grid is showing a collection (enables "Remove from…").
    pub viewing_collection: Option<&'a str>,
    /// Whole library is empty (vs. just the current filter matching nothing).
    pub library_empty: bool,
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut GridState,
    images: &[ImageRecord],
    jobs: &Jobs,
    gctx: &GridContext<'_>,
) -> GridResponse {
    let mut response = GridResponse::default();
    if images.is_empty() {
        ui.centered_and_justified(|ui| {
            let msg = if gctx.library_empty {
                "Library is empty. Use Import… to add photos"
            } else {
                "No photos match the current filter"
            };
            ui.label(egui::RichText::new(msg).weak().size(15.0));
        });
        return response;
    }

    let avail = ui.available_width();
    let cols = ((avail + PAD) / (CELL + PAD)).floor().max(1.0) as usize;
    let rows = images.len().div_ceil(cols);
    let row_height = CELL + PAD;

    let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
    let jumping = state.scroll_to_bottom > 0;
    if jumping {
        // Past-the-end offset; egui clamps it to the real maximum. `show_rows`
        // takes the row height *without* spacing and adds `item_spacing.y`
        // between rows, so the content is taller than `rows * row_height` —
        // count the spacing in or the jump lands short of the last row.
        let stride = row_height + ui.spacing().item_spacing.y;
        scroll = scroll.vertical_scroll_offset(rows as f32 * stride + row_height);
        state.restore_scroll = None;
    } else if let Some(offset) = state.restore_scroll.take() {
        scroll = scroll.vertical_scroll_offset(offset);
    }
    let out = scroll.show_rows(ui, row_height, rows, |ui, row_range| {
        for row in row_range {
            ui.horizontal(|ui| {
                ui.add_space(PAD / 2.0);
                for col in 0..cols {
                    let idx = row * cols + col;
                    if idx >= images.len() {
                        break;
                    }
                    cell(ui, state, images, idx, jobs, gctx, &mut response);
                }
            });
        }
    });
    state.scroll_offset = out.state.offset.y;
    if jumping {
        // Only stop once the offset is at the bottom *and* the layout has
        // stopped moving under it. Checking the offset alone gives up on the
        // first frame, whose unconstrained viewport has its own — far
        // shorter — bottom.
        let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
        let settled = (out.content_size.y - state.last_content_h).abs() < 0.5;
        state.scroll_to_bottom = if settled && out.state.offset.y >= max - 0.5 {
            0
        } else {
            state.scroll_to_bottom - 1
        };
    }
    state.last_content_h = out.content_size.y;
    scrollbar_timeline(ui, &out.inner_rect, images, rows, cols);
    response
}

/// Google Photos-style timeline on the grid's scrollbar: year labels pinned
/// where each year starts in the library, month-start dots between them, and
/// a floating month/year pill while the pointer scrubs the track.
fn scrollbar_timeline(
    ui: &egui::Ui,
    inner_rect: &Rect,
    images: &[ImageRecord],
    rows: usize,
    cols: usize,
) {
    use chrono::{Datelike, TimeZone};

    // Everything fits on screen → there is no track to annotate.
    if rows as f32 * (CELL + PAD) <= inner_rect.height() {
        return;
    }

    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("grid-scrollbar-date"),
    ));

    // First index of each year / month as a fraction through the library
    // (capture order, so fractions are monotonic).
    let n = images.len();
    let mut years: Vec<(i32, f32)> = Vec::new();
    let mut month_fracs: Vec<f32> = Vec::new();
    let mut last: Option<(i32, u32)> = None;
    for (i, rec) in images.iter().enumerate() {
        let Some(dt) = rec
            .capture_ts
            .and_then(|ts| chrono::Local.timestamp_opt(ts, 0).earliest())
        else {
            continue;
        };
        let ym = (dt.year(), dt.month());
        let frac = i as f32 / (n - 1).max(1) as f32;
        if last.is_none_or(|(y, _)| y != ym.0) {
            years.push((ym.0, frac));
        } else if last != Some(ym) {
            month_fracs.push(frac);
        }
        last = Some(ym);
    }

    let track_x = inner_rect.right() - 6.0;
    let font = egui::FontId::new(10.5, theme::mono());
    // Year labels, dropping any that would crowd the previous one.
    let mut label_bands: Vec<(f32, f32)> = Vec::new();
    for (year, frac) in &years {
        let galley =
            painter.layout_no_wrap(year.to_string(), font.clone(), theme::text_secondary());
        let size = galley.size();
        let y = (inner_rect.top() + frac * inner_rect.height()).clamp(
            inner_rect.top() + size.y / 2.0,
            inner_rect.bottom() - size.y / 2.0,
        );
        let text_rect =
            Rect::from_min_size(egui::pos2(track_x - 6.0 - size.x, y - size.y / 2.0), size);
        let bg = text_rect.expand2(Vec2::new(5.0, 2.0));
        if label_bands
            .last()
            .is_some_and(|(_, bottom)| bg.top() < bottom + 6.0)
        {
            continue;
        }
        painter.rect_filled(
            bg,
            egui::CornerRadius::same(theme::RADIUS_SM),
            Color32::from_black_alpha(140),
        );
        painter.galley(text_rect.min, galley, theme::text_secondary());
        label_bands.push((bg.top(), bg.bottom()));
    }
    // Month dots, skipping the bands the year labels occupy.
    for frac in &month_fracs {
        let y = inner_rect.top() + frac * inner_rect.height();
        if label_bands
            .iter()
            .any(|(top, bottom)| y >= top - 3.0 && y <= bottom + 3.0)
        {
            continue;
        }
        painter.circle_filled(egui::pos2(track_x, y), 1.5, theme::text_muted());
    }

    let Some(pos) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    // The bar lives between the scroll viewport and the panel's right edge;
    // pad a little inward so the pill also shows while dragging the thumb.
    let over_bar = pos.x >= inner_rect.right() - 6.0
        && pos.x <= ui.max_rect().right() + 2.0
        && (inner_rect.top()..=inner_rect.bottom()).contains(&pos.y);
    if !over_bar {
        return;
    }

    let frac = ((pos.y - inner_rect.top()) / inner_rect.height()).clamp(0.0, 1.0);
    let row = ((rows.saturating_sub(1)) as f32 * frac).round() as usize;
    let idx = (row * cols).min(images.len() - 1);
    // Nearest record with a capture date (library order is by capture time).
    let ts = images[idx..]
        .iter()
        .find_map(|rec| rec.capture_ts)
        .or_else(|| images[..idx].iter().rev().find_map(|rec| rec.capture_ts));
    let Some(label) = ts
        .and_then(|ts| chrono::Local.timestamp_opt(ts, 0).earliest())
        .map(|dt| dt.format("%B %Y").to_string())
    else {
        return;
    };

    let galley = painter.layout_no_wrap(label, theme::body_font(), theme::text_primary());
    let pad = Vec2::new(10.0, 6.0);
    let size = galley.size() + pad * 2.0;
    let y = pos
        .y
        .clamp(inner_rect.top() + size.y / 2.0, inner_rect.bottom() - size.y / 2.0);
    let pill = Rect::from_min_size(
        egui::pos2(inner_rect.right() - size.x - 14.0, y - size.y / 2.0),
        size,
    );
    painter.rect_filled(
        pill,
        egui::CornerRadius::same(theme::RADIUS_MD),
        theme::bg_raised(),
    );
    painter.rect_stroke(
        pill,
        egui::CornerRadius::same(theme::RADIUS_MD),
        egui::Stroke::new(1.0, theme::line_strong()),
        egui::StrokeKind::Inside,
    );
    painter.galley(pill.min + pad, galley, theme::text_primary());
}

/// Small flag indicator in the cell's top-left corner.
pub fn draw_flag_badge(painter: &egui::Painter, rect: Rect, flag: Flag) {
    let (glyph, color) = match flag {
        Flag::Unflagged => return,
        Flag::Pick => ("⚑", theme::flag_pick()),
        Flag::Reject => ("✖", theme::danger()),
    };
    let badge = Rect::from_min_size(rect.min + Vec2::splat(5.0), Vec2::splat(18.0));
    painter.rect_filled(badge, 4.0, Color32::from_black_alpha(170));
    painter.text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(13.0),
        color,
    );
}

fn cell(
    ui: &mut egui::Ui,
    state: &mut GridState,
    images: &[ImageRecord],
    idx: usize,
    jobs: &Jobs,
    gctx: &GridContext<'_>,
    grid_response: &mut GridResponse,
) {
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
        theme::accent().gamma_multiply(0.16)
    } else {
        theme::bg_raised()
    };
    painter.rect_filled(rect, 4.0, bg);

    if let Some(tex) = state.textures.get(&rec.id) {
        let size = tex.size_vec2();
        let scale = ((CELL - PAD) / size.x).min((CELL - PAD) / size.y);
        let img_size = size * scale;
        let img_rect = Rect::from_center_size(rect.center(), img_size);
        // Rejects render dimmed, like Lightroom.
        let tint = if rec.flag == Flag::Reject {
            Color32::from_gray(90)
        } else {
            Color32::WHITE
        };
        painter.image(
            tex.id(),
            img_rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
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
            egui::Stroke::new(2.0, theme::accent()),
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() {
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, theme::line_strong()),
            egui::StrokeKind::Inside,
        );
    }

    draw_flag_badge(painter, rect, rec.flag);

    response.clone().on_hover_text(format!(
        "{}\n{} {}",
        rec.file_name, rec.camera_make, rec.camera_model
    ));

    response.context_menu(|ui| {
        // Right-clicking outside the selection retargets it.
        let n = if state.selection.contains(&rec.id) {
            state.selection.len()
        } else {
            1
        };
        let retarget = |state: &mut GridState| {
            if !state.selection.contains(&rec.id) {
                state.selection.clear();
                state.selection.insert(rec.id);
            }
        };

        ui.label(
            egui::RichText::new(format!("{n} photo{}", if n == 1 { "" } else { "s" }))
                .weak()
                .small(),
        );
        if ui.button("⚑ Pick").clicked() {
            retarget(state);
            grid_response.set_flag = Some(Flag::Pick);
            ui.close();
        }
        if ui.button("Unflag").clicked() {
            retarget(state);
            grid_response.set_flag = Some(Flag::Unflagged);
            ui.close();
        }
        if ui.button("✖ Reject").clicked() {
            retarget(state);
            grid_response.set_flag = Some(Flag::Reject);
            ui.close();
        }
        ui.separator();

        if gctx.collections.is_empty() {
            ui.add_enabled(false, egui::Button::new("Add to collection"))
                .on_disabled_hover_text("Create a collection in the left panel first");
        } else {
            ui.menu_button("Add to collection", |ui| {
                for col in gctx.collections {
                    if ui.button(&col.name).clicked() {
                        retarget(state);
                        grid_response.add_to_collection = Some(col.id);
                        ui.close();
                    }
                }
            });
        }
        if let Some(name) = gctx.viewing_collection {
            if ui.button(format!("Remove from '{name}'")).clicked() {
                retarget(state);
                grid_response.remove_from_collection = true;
                ui.close();
            }
        }
        ui.separator();

        if ui
            .add_enabled(
                gctx.can_paste,
                egui::Button::new(format!("Paste settings to {n} selected")),
            )
            .clicked()
        {
            retarget(state);
            grid_response.paste_to_selection = true;
            ui.close();
        }
        ui.separator();
        if ui
            .button(format!("Delete {n} photo{}…", if n == 1 { "" } else { "s" }))
            .clicked()
        {
            retarget(state);
            grid_response.delete_selection = true;
            ui.close();
        }
    });

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
