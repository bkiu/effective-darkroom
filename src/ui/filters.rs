//! Faceted library filters (left panel): file type, camera, capture date
//! (year + month chips), flags and edit status. Within a facet, checked
//! options are OR'd; facets combine with AND; an empty facet matches all.

use std::collections::{BTreeMap, HashSet};

use chrono::{Datelike, TimeZone};
use egui::{CornerRadius, Sense, Stroke, Vec2};

use crate::catalog::{FileKind, Flag, ImageRecord};
use crate::ui::theme;

#[derive(Default, Clone, PartialEq)]
pub struct FilterState {
    pub enabled: bool,
    pub file_raw: bool,
    pub file_jpeg: bool,
    pub cameras: HashSet<String>,
    pub years: HashSet<i32>,
    /// (year, month) chips; independent of the year checkbox.
    pub months: HashSet<(i32, u32)>,
    pub flag_pick: bool,
    pub flag_unflagged: bool,
    pub flag_reject: bool,
    pub edited: bool,
    pub unedited: bool,
}

impl FilterState {
    pub fn any_active(&self) -> bool {
        self.file_raw
            || self.file_jpeg
            || !self.cameras.is_empty()
            || !self.years.is_empty()
            || !self.months.is_empty()
            || self.flag_pick
            || self.flag_unflagged
            || self.flag_reject
            || self.edited
            || self.unedited
    }

    pub fn clear(&mut self) {
        let enabled = self.enabled;
        *self = FilterState::default();
        self.enabled = enabled;
    }

    /// Does a record pass the current filters?
    pub fn matches(&self, rec: &ImageRecord, edited_ids: &HashSet<i64>) -> bool {
        if !self.enabled {
            return true;
        }
        // File type facet.
        if self.file_raw || self.file_jpeg {
            let ok = match rec.kind {
                FileKind::Raw => self.file_raw,
                FileKind::Jpeg => self.file_jpeg,
            };
            if !ok {
                return false;
            }
        }
        // Camera facet.
        if !self.cameras.is_empty() && !self.cameras.contains(camera_label(rec).as_str()) {
            return false;
        }
        // Date facet: year checked = whole year; month chips are OR'd in.
        if !self.years.is_empty() || !self.months.is_empty() {
            let Some((y, m)) = year_month(rec) else {
                return false;
            };
            if !self.years.contains(&y) && !self.months.contains(&(y, m)) {
                return false;
            }
        }
        // Flags facet.
        if self.flag_pick || self.flag_unflagged || self.flag_reject {
            let ok = match rec.flag {
                Flag::Pick => self.flag_pick,
                Flag::Unflagged => self.flag_unflagged,
                Flag::Reject => self.flag_reject,
            };
            if !ok {
                return false;
            }
        }
        // Edit status facet.
        if self.edited || self.unedited {
            let has = edited_ids.contains(&rec.id);
            if !(has && self.edited || !has && self.unedited) {
                return false;
            }
        }
        true
    }
}

pub fn camera_label(rec: &ImageRecord) -> String {
    let s = format!("{} {}", rec.camera_make, rec.camera_model);
    let s = s.trim().to_string();
    if s.is_empty() {
        "Unknown".into()
    } else {
        s
    }
}

fn year_month(rec: &ImageRecord) -> Option<(i32, u32)> {
    let dt = chrono::Local.timestamp_opt(rec.capture_ts?, 0).earliest()?;
    Some((dt.year(), dt.month()))
}

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Draw the filter facets; returns true when any filter changed.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut FilterState,
    images: &[ImageRecord],
    edited_ids: &HashSet<i64>,
) -> bool {
    let mut changed = false;

    // Facet counts over the whole library.
    let mut n_raw = 0usize;
    let mut n_jpeg = 0usize;
    let mut cameras: BTreeMap<String, usize> = BTreeMap::new();
    // year -> (count, month -> count); newest year first.
    let mut dates: BTreeMap<std::cmp::Reverse<i32>, (usize, BTreeMap<u32, usize>)> =
        BTreeMap::new();
    let (mut n_pick, mut n_unflagged, mut n_reject) = (0usize, 0usize, 0usize);
    let mut n_edited = 0usize;
    for rec in images {
        match rec.kind {
            FileKind::Raw => n_raw += 1,
            FileKind::Jpeg => n_jpeg += 1,
        }
        *cameras.entry(camera_label(rec)).or_default() += 1;
        if let Some((y, m)) = year_month(rec) {
            let entry = dates.entry(std::cmp::Reverse(y)).or_default();
            entry.0 += 1;
            *entry.1.entry(m).or_default() += 1;
        }
        match rec.flag {
            Flag::Pick => n_pick += 1,
            Flag::Unflagged => n_unflagged += 1,
            Flag::Reject => n_reject += 1,
        }
        if edited_ids.contains(&rec.id) {
            n_edited += 1;
        }
    }

    ui.horizontal(|ui| {
        ui.label(theme::overline_text("Filters"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Always occupy the space so the column doesn't jump when the
            // button appears/disappears.
            if ui
                .add_visible(state.any_active(), egui::Button::new("Clear").small())
                .clicked()
            {
                state.clear();
                changed = true;
            }
        });
    });
    ui.add_space(2.0);

    facet_header(ui, "File type");
    changed |= filter_row(ui, &mut state.file_raw, "RAW", n_raw);
    changed |= filter_row(ui, &mut state.file_jpeg, "JPEG", n_jpeg);

    facet_header(ui, "Camera");
    for (camera, count) in &cameras {
        let mut on = state.cameras.contains(camera);
        if filter_row(ui, &mut on, camera, *count) {
            if on {
                state.cameras.insert(camera.clone());
            } else {
                state.cameras.remove(camera);
            }
            changed = true;
        }
    }

    facet_header(ui, "Date");
    for (std::cmp::Reverse(year), (count, months)) in &dates {
        let mut on = state.years.contains(year);
        if filter_row(ui, &mut on, &year.to_string(), *count) {
            if on {
                state.years.insert(*year);
            } else {
                state.years.remove(year);
            }
            changed = true;
        }
        ui.horizontal_wrapped(|ui| {
            ui.add_space(24.0);
            for (month, mcount) in months.iter().rev() {
                let key = (*year, *month);
                let active = state.months.contains(&key);
                if month_chip(
                    ui,
                    active,
                    MONTH_NAMES[(*month - 1) as usize],
                    *mcount,
                ) {
                    if active {
                        state.months.remove(&key);
                    } else {
                        state.months.insert(key);
                    }
                    changed = true;
                }
            }
        });
        ui.add_space(2.0);
    }

    facet_header(ui, "Flags");
    changed |= filter_row(ui, &mut state.flag_pick, "⚑ Picked", n_pick);
    changed |= filter_row(ui, &mut state.flag_unflagged, "⚐ Unflagged", n_unflagged);
    changed |= filter_row(ui, &mut state.flag_reject, "✖ Rejected", n_reject);

    facet_header(ui, "Edits");
    changed |= filter_row(ui, &mut state.edited, "Edited", n_edited);
    changed |= filter_row(
        ui,
        &mut state.unedited,
        "Unedited",
        images.len() - n_edited,
    );

    changed
}

fn facet_header(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .font(egui::FontId::new(10.5, theme::mono()))
            .color(theme::text_muted()),
    );
    ui.add_space(2.0);
}

/// Checkbox row: box + label left, mono count right.
fn filter_row(ui: &mut egui::Ui, checked: &mut bool, label: &str, count: usize) -> bool {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), theme::bg_raised());
    }
    // Checkbox.
    let cb = egui::Rect::from_center_size(
        egui::Pos2::new(rect.left() + 12.0, rect.center().y),
        Vec2::splat(15.0),
    );
    if *checked {
        painter.rect_filled(cb, CornerRadius::same(3), theme::accent());
        painter.text(
            cb.center(),
            egui::Align2::CENTER_CENTER,
            "✔",
            egui::FontId::proportional(11.0),
            egui::Color32::WHITE,
        );
    } else {
        painter.rect_stroke(
            cb,
            CornerRadius::same(3),
            Stroke::new(1.0, theme::line_strong()),
            egui::StrokeKind::Inside,
        );
    }
    painter.text(
        egui::Pos2::new(cb.right() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        theme::body_font(),
        if *checked {
            theme::text_primary()
        } else {
            theme::text_secondary()
        },
    );
    painter.text(
        egui::Pos2::new(rect.right() - 6.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        count.to_string(),
        egui::FontId::new(12.0, theme::mono()),
        theme::text_muted(),
    );
    if resp.clicked() {
        *checked = !*checked;
        return true;
    }
    false
}

/// Small pill chip "July 77"; returns true when clicked.
fn month_chip(ui: &mut egui::Ui, active: bool, name: &str, count: usize) -> bool {
    let font = egui::FontId::new(11.5, egui::FontFamily::Proportional);
    let text = format!("{name}  {count}");
    let galley = ui
        .painter()
        .layout_no_wrap(text.clone(), font.clone(), egui::Color32::WHITE);
    let size = galley.size() + Vec2::new(18.0, 8.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let pill = (rect.height() / 2.0) as u8;
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, CornerRadius::same(pill), theme::accent());
    } else {
        painter.rect_filled(rect, CornerRadius::same(pill), theme::bg_raised());
        painter.rect_stroke(
            rect,
            CornerRadius::same(pill),
            Stroke::new(1.0, theme::line_subtle()),
            egui::StrokeKind::Inside,
        );
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        font,
        if active {
            egui::Color32::WHITE
        } else {
            theme::text_secondary()
        },
    );
    resp.clicked()
}
