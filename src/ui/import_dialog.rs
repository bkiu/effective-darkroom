//! Import dialog. Flow: configure a source (files/folders or camera) and
//! destination → scan → pick photos with checkboxes → import with progress
//! and a Stop button.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use egui::{Color32, Rect, Sense, Vec2};

use crate::importer::camera::{self, CameraDesc};
use crate::importer::{
    CandidateSource, ImportCandidate, ImportMode, ImportOutcome, ImportSource, ImportSpec,
    ScanSource,
};

const CELL: f32 = 132.0;
const PAD: f32 = 6.0;

#[derive(Default)]
pub struct ImportDialog {
    pub open: bool,
    stage: Stage,
    source_kind: SourceKind,
    sources: Vec<PathBuf>,
    cameras: Vec<CameraDesc>,
    selected_camera: usize,
    camera_error: Option<String>,
    pub dest_root: Option<PathBuf>,
    mode: ImportModeChoice,

    // Picker state.
    pub scan_generation: u64,
    scan_cancel: Option<Arc<AtomicBool>>,
    candidates: Vec<ImportCandidate>,
    checked: HashSet<usize>,
    thumbs: HashMap<usize, egui::TextureHandle>,
    scan_error: Option<String>,
    scan_pending: bool,

    // Import state.
    import_cancel: Option<Arc<AtomicBool>>,
    /// (current, total, file name) while an import runs.
    pub progress: Option<(usize, usize, String)>,
    pub last_outcome: Option<ImportOutcome>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Stage {
    #[default]
    Configure,
    Pick,
    Importing,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    #[default]
    Files,
    Camera,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum ImportModeChoice {
    #[default]
    Copy,
    Move,
}

/// What the dialog wants the app to kick off this frame.
pub enum DialogAction {
    None,
    StartScan(ScanSource),
    StartImport(ImportSpec, Arc<AtomicBool>),
}

impl ImportDialog {
    fn cancel_scan(&mut self) {
        if let Some(c) = &self.scan_cancel {
            c.store(true, Ordering::Relaxed);
        }
        self.scan_cancel = None;
        self.scan_pending = false;
    }

    fn reset_picker(&mut self) {
        self.cancel_scan();
        self.candidates.clear();
        self.checked.clear();
        self.thumbs.clear();
        self.scan_error = None;
    }

    fn refresh_cameras(&mut self) {
        match camera::detect() {
            Ok(cams) => {
                self.camera_error = if cams.is_empty() {
                    Some("No camera found — is it connected and powered on?".into())
                } else {
                    None
                };
                self.cameras = cams;
                self.selected_camera = 0;
            }
            Err(e) => {
                self.cameras.clear();
                self.camera_error = Some(format!("{e:#}"));
            }
        }
    }

    // --- results from the job system ---

    pub fn on_scan_list(
        &mut self,
        generation: u64,
        candidates: Vec<ImportCandidate>,
    ) {
        if generation != self.scan_generation {
            return;
        }
        self.scan_pending = false;
        // New photos start checked; already-imported ones unchecked.
        self.checked = candidates
            .iter()
            .filter(|c| !c.already_imported)
            .map(|c| c.key)
            .collect();
        self.candidates = candidates;
    }

    pub fn on_scan_thumb(
        &mut self,
        ctx: &egui::Context,
        generation: u64,
        key: usize,
        image: egui::ColorImage,
    ) {
        if generation != self.scan_generation {
            return;
        }
        let handle =
            ctx.load_texture(format!("import-thumb-{key}"), image, egui::TextureOptions::LINEAR);
        self.thumbs.insert(key, handle);
    }

    pub fn on_scan_failed(&mut self, generation: u64, error: String) {
        if generation != self.scan_generation {
            return;
        }
        self.scan_pending = false;
        self.scan_error = Some(error);
    }

    pub fn on_import_done(&mut self, outcome: ImportOutcome) {
        self.progress = None;
        self.import_cancel = None;
        self.stage = Stage::Configure;
        self.reset_picker();
        // Close on clean completion; stay open only to show errors.
        if outcome.errors.is_empty() {
            self.open = false;
            self.last_outcome = None;
            self.sources.clear();
        } else {
            self.last_outcome = Some(outcome);
        }
    }

    /// Show the dialog; returns what the app should start this frame.
    pub fn show(&mut self, ctx: &egui::Context) -> DialogAction {
        if !self.open {
            return DialogAction::None;
        }
        let mut action = DialogAction::None;
        let mut open = self.open;

        egui::Window::new("Import Photos")
            .open(&mut open)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .collapsible(false)
            .resizable(true)
            .default_width(640.0)
            .default_height(560.0)
            .show(ctx, |ui| match self.stage {
                Stage::Configure => self.configure_ui(ui, &mut action),
                Stage::Pick => self.pick_ui(ui, &mut action),
                Stage::Importing => self.importing_ui(ui),
            });

        if !open {
            // Dialog closed: stop any in-flight scan (imports keep running).
            self.cancel_scan();
            if self.stage != Stage::Importing {
                self.stage = Stage::Configure;
            }
        }
        self.open = open;
        action
    }

    fn configure_ui(&mut self, ui: &mut egui::Ui, action: &mut DialogAction) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if crate::ui::theme::toggle_button(
                ui,
                self.source_kind == SourceKind::Files,
                "Files / Folders",
            )
            .clicked()
            {
                self.source_kind = SourceKind::Files;
            }
            if crate::ui::theme::toggle_button(
                ui,
                self.source_kind == SourceKind::Camera,
                "📷 Camera",
            )
            .clicked()
            {
                self.source_kind = SourceKind::Camera;
                if self.cameras.is_empty() {
                    self.refresh_cameras();
                }
            }
        });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(8.0);

        match self.source_kind {
            SourceKind::Files => self.files_section(ui),
            SourceKind::Camera => self.camera_section(ui),
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label(crate::ui::theme::overline_text("Destination"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Choose…").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.dest_root = Some(dir);
                }
            }
            match &self.dest_root {
                Some(d) => ui.label(d.display().to_string()),
                None => ui.label(egui::RichText::new("no destination chosen").weak()),
            };
        });
        ui.label(
            egui::RichText::new("Photos are organized into YYYY/MM-DD folders by capture date.")
                .weak()
                .small(),
        );
        if self.source_kind == SourceKind::Files {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.mode, ImportModeChoice::Copy, "Copy");
                ui.radio_value(&mut self.mode, ImportModeChoice::Move, "Move");
            });
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        if let Some(out) = &self.last_outcome {
            let mut msg = format!("Imported {} photos", out.imported);
            if out.skipped_duplicates > 0 {
                msg += &format!(", skipped {} duplicates", out.skipped_duplicates);
            }
            if out.cancelled {
                msg += " (stopped early)";
            }
            ui.label(msg);
            for err in &out.errors {
                ui.colored_label(Color32::LIGHT_RED, err);
            }
        }

        let scan_source = match self.source_kind {
            SourceKind::Files if !self.sources.is_empty() => {
                Some(ScanSource::Paths(self.sources.clone()))
            }
            SourceKind::Camera => self
                .cameras
                .get(self.selected_camera)
                .map(|c| ScanSource::Camera(c.clone())),
            _ => None,
        };
        let ready = scan_source.is_some() && self.dest_root.is_some();
        let next_clicked = if ready {
            crate::ui::theme::primary_button(ui, "Next: choose photos…").clicked()
        } else {
            ui.add_enabled(false, egui::Button::new("Next: choose photos…"))
                .clicked()
        };
        if next_clicked {
            self.last_outcome = None;
            self.reset_picker();
            self.scan_generation += 1;
            self.scan_pending = true;
            let cancel = Arc::new(AtomicBool::new(false));
            self.scan_cancel = Some(cancel);
            self.stage = Stage::Pick;
            *action = DialogAction::StartScan(scan_source.unwrap());
        }
    }

    fn pick_ui(&mut self, ui: &mut egui::Ui, action: &mut DialogAction) {
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                self.stage = Stage::Configure;
                self.reset_picker();
                return;
            }
            ui.separator();
            if ui.button("All").clicked() {
                self.checked = self.candidates.iter().map(|c| c.key).collect();
            }
            if ui.button("None").clicked() {
                self.checked.clear();
            }
            if ui.button("New only").clicked() {
                self.checked = self
                    .candidates
                    .iter()
                    .filter(|c| !c.already_imported)
                    .map(|c| c.key)
                    .collect();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let n = self.checked.len();
                if ui
                    .add_enabled(
                        n > 0,
                        egui::Button::new(format!("Import {n} photo{}", if n == 1 { "" } else { "s" })),
                    )
                    .clicked()
                {
                    if let Some(spec) = self.build_spec() {
                        let cancel = Arc::new(AtomicBool::new(false));
                        self.import_cancel = Some(cancel.clone());
                        self.progress =
                            Some((0, self.checked.len(), String::from("starting…")));
                        self.stage = Stage::Importing;
                        self.cancel_scan();
                        *action = DialogAction::StartImport(spec, cancel);
                    }
                }
                if self.scan_pending {
                    ui.spinner();
                    ui.label(egui::RichText::new("scanning…").weak());
                } else if self.thumbs.len() < self.candidates.len() {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new(format!(
                            "previews {}/{}",
                            self.thumbs.len(),
                            self.candidates.len()
                        ))
                        .weak(),
                    );
                }
            });
        });
        if let Some(err) = &self.scan_error {
            ui.colored_label(Color32::LIGHT_RED, err);
        }
        ui.separator();

        if self.candidates.is_empty() && !self.scan_pending {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("No importable photos found").weak());
            });
            return;
        }

        let avail = ui.available_width();
        let cols = ((avail + PAD) / (CELL + PAD)).floor().max(1.0) as usize;
        let rows = self.candidates.len().div_ceil(cols);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, CELL + PAD + 16.0, rows, |ui, row_range| {
                for row in row_range {
                    ui.horizontal(|ui| {
                        for col in 0..cols {
                            let idx = row * cols + col;
                            if idx >= self.candidates.len() {
                                break;
                            }
                            self.candidate_cell(ui, idx);
                        }
                    });
                }
            });
    }

    fn candidate_cell(&mut self, ui: &mut egui::Ui, idx: usize) {
        let cand = &self.candidates[idx];
        let key = cand.key;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(CELL, CELL + 16.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let img_rect = Rect::from_min_size(rect.min, Vec2::splat(CELL));
        let checked = self.checked.contains(&key);

        let painter = ui.painter();
        painter.rect_filled(img_rect, 4.0, Color32::from_gray(26));
        if let Some(tex) = self.thumbs.get(&key) {
            let size = tex.size_vec2();
            let scale = ((CELL - PAD) / size.x).min((CELL - PAD) / size.y);
            let draw = Rect::from_center_size(img_rect.center(), size * scale);
            let tint = if checked {
                Color32::WHITE
            } else {
                Color32::from_gray(110)
            };
            painter.image(
                tex.id(),
                draw,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                tint,
            );
        }
        if checked {
            painter.rect_stroke(
                img_rect,
                4.0,
                egui::Stroke::new(2.0, crate::ui::theme::accent()),
                egui::StrokeKind::Inside,
            );
        }
        // Checkbox mark, top-left.
        let cb = Rect::from_min_size(img_rect.min + Vec2::splat(6.0), Vec2::splat(16.0));
        painter.rect_filled(cb, 3.0, Color32::from_black_alpha(160));
        if checked {
            painter.text(
                cb.center(),
                egui::Align2::CENTER_CENTER,
                "✔",
                egui::FontId::proportional(13.0),
                Color32::from_rgb(120, 200, 255),
            );
        }
        if cand.already_imported {
            painter.text(
                img_rect.right_top() + Vec2::new(-6.0, 6.0),
                egui::Align2::RIGHT_TOP,
                "already imported",
                egui::FontId::proportional(9.0),
                Color32::from_rgb(230, 200, 90),
            );
        }
        painter.text(
            Rect::from_min_max(img_rect.left_bottom(), rect.max).center(),
            egui::Align2::CENTER_CENTER,
            &cand.name,
            egui::FontId::proportional(10.5),
            Color32::from_gray(if checked { 220 } else { 130 }),
        );

        if response.clicked()
            && !self.checked.insert(key) {
                self.checked.remove(&key);
            }
    }

    fn importing_ui(&mut self, ui: &mut egui::Ui) {
        if let Some((cur, total, name)) = self.progress.clone() {
            if total == 0 {
                ui.label(name);
                ui.spinner();
            } else {
                ui.label(format!("Importing {cur} of {total}…"));
                ui.add(egui::ProgressBar::new(cur as f32 / total.max(1) as f32).text(name));
            }
            ui.add_space(6.0);
            let stopping = self
                .import_cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Relaxed));
            if stopping {
                ui.label(egui::RichText::new("Stopping after the current file…").weak());
            } else if ui.button("Stop").clicked() {
                if let Some(c) = &self.import_cancel {
                    c.store(true, Ordering::Relaxed);
                }
            }
        }
    }

    fn build_spec(&self) -> Option<ImportSpec> {
        let dest_root = self.dest_root.clone()?;
        let picked: Vec<&ImportCandidate> = self
            .candidates
            .iter()
            .filter(|c| self.checked.contains(&c.key))
            .collect();
        if picked.is_empty() {
            return None;
        }
        let source = match &picked[0].source {
            CandidateSource::Path(_) => ImportSource::Paths(
                picked
                    .iter()
                    .filter_map(|c| match &c.source {
                        CandidateSource::Path(p) => Some(p.clone()),
                        CandidateSource::Card(_) => None,
                    })
                    .collect(),
            ),
            CandidateSource::Card(_) => {
                let desc = self.cameras.get(self.selected_camera)?.clone();
                ImportSource::Camera {
                    desc,
                    files: picked
                        .iter()
                        .filter_map(|c| match &c.source {
                            CandidateSource::Card(f) => Some(f.clone()),
                            CandidateSource::Path(_) => None,
                        })
                        .collect(),
                }
            }
        };
        Some(ImportSpec {
            source,
            dest_root,
            mode: match self.mode {
                ImportModeChoice::Copy => ImportMode::Copy,
                ImportModeChoice::Move => ImportMode::Move,
            },
        })
    }

    pub fn scan_cancel_flag(&self) -> Option<Arc<AtomicBool>> {
        self.scan_cancel.clone()
    }

    fn files_section(&mut self, ui: &mut egui::Ui) {
        ui.label(crate::ui::theme::overline_text("Sources"));
        ui.horizontal(|ui| {
            if ui.button("Add files…").clicked() {
                if let Some(files) = rfd::FileDialog::new()
                    .add_filter(
                        "Photos",
                        &["cr3", "CR3", "raf", "RAF", "jpg", "JPG", "jpeg", "JPEG"],
                    )
                    .pick_files()
                {
                    self.sources.extend(files);
                    self.sources.sort();
                    self.sources.dedup();
                }
            }
            if ui.button("Add folder…").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.sources.push(dir);
                    self.sources.sort();
                    self.sources.dedup();
                }
            }
            if !self.sources.is_empty() && ui.button("Clear").clicked() {
                self.sources.clear();
            }
        });
        egui::ScrollArea::vertical()
            .max_height(120.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let mut remove: Option<usize> = None;
                for (i, s) in self.sources.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button("✖").clicked() {
                            remove = Some(i);
                        }
                        ui.label(s.display().to_string());
                    });
                }
                if let Some(i) = remove {
                    self.sources.remove(i);
                }
            });
    }

    fn camera_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(crate::ui::theme::overline_text("Camera"));
            if ui.button("Refresh").clicked() {
                self.refresh_cameras();
            }
        });
        if let Some(err) = &self.camera_error {
            ui.colored_label(Color32::LIGHT_RED, err);
        }
        for (i, cam) in self.cameras.iter().enumerate() {
            ui.radio_value(
                &mut self.selected_camera,
                i,
                format!("{} ({})", cam.model, cam.port),
            );
        }
        if !self.cameras.is_empty() {
            ui.label(
                egui::RichText::new(
                    "The next step lists the photos on the card so you can choose what to \
                     import. Nothing is ever deleted from the card.",
                )
                .weak()
                .small(),
            );
        }
    }
}
