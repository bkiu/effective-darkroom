//! Import dialog: pick files/folders or a connected camera, choose the
//! destination (prefilled with the library location), watch progress.

use std::path::PathBuf;

use crate::importer::camera::{self, CameraDesc};
use crate::importer::{ImportMode, ImportOutcome, ImportSource, ImportSpec};

#[derive(Default)]
pub struct ImportDialog {
    pub open: bool,
    source_kind: SourceKind,
    sources: Vec<PathBuf>,
    cameras: Vec<CameraDesc>,
    selected_camera: usize,
    camera_error: Option<String>,
    pub dest_root: Option<PathBuf>,
    mode: ImportModeChoice,
    /// (current, total, file name) while an import runs.
    pub progress: Option<(usize, usize, String)>,
    pub last_outcome: Option<ImportOutcome>,
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

impl ImportDialog {
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

    /// Returns an ImportSpec when the user hits Import.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<ImportSpec> {
        if !self.open {
            return None;
        }
        let mut start: Option<ImportSpec> = None;
        let mut open = self.open;

        egui::Window::new("Import Photos")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(480.0)
            .show(ctx, |ui| {
                if let Some((cur, total, name)) = &self.progress {
                    if *total == 0 {
                        ui.label(name.clone());
                        ui.spinner();
                    } else {
                        ui.label(format!("Importing {cur} of {total}…"));
                        ui.add(
                            egui::ProgressBar::new(*cur as f32 / (*total).max(1) as f32)
                                .text(name.clone()),
                        );
                    }
                    return;
                }

                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.source_kind, SourceKind::Files, "Files / Folders");
                    let was_camera = self.source_kind == SourceKind::Camera;
                    ui.selectable_value(&mut self.source_kind, SourceKind::Camera, "📷 Camera");
                    if self.source_kind == SourceKind::Camera
                        && !was_camera
                        && self.cameras.is_empty()
                    {
                        self.refresh_cameras();
                    }
                });
                ui.separator();

                match self.source_kind {
                    SourceKind::Files => self.files_section(ui),
                    SourceKind::Camera => self.camera_section(ui),
                }

                ui.separator();
                ui.heading("Destination");
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
                    egui::RichText::new(
                        "Photos are organized into YYYY/MM-DD folders by capture date.",
                    )
                    .weak()
                    .small(),
                );

                if self.source_kind == SourceKind::Files {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.mode, ImportModeChoice::Copy, "Copy");
                        ui.radio_value(&mut self.mode, ImportModeChoice::Move, "Move");
                    });
                }

                ui.separator();
                if let Some(out) = &self.last_outcome {
                    let mut msg = format!("Imported {} photos", out.imported);
                    if out.skipped_duplicates > 0 {
                        msg += &format!(", skipped {} duplicates", out.skipped_duplicates);
                    }
                    ui.label(msg);
                    for err in &out.errors {
                        ui.colored_label(egui::Color32::LIGHT_RED, err);
                    }
                }

                let source = match self.source_kind {
                    SourceKind::Files if !self.sources.is_empty() => {
                        Some(ImportSource::Paths(self.sources.clone()))
                    }
                    SourceKind::Camera => self
                        .cameras
                        .get(self.selected_camera)
                        .map(|c| ImportSource::Camera(c.clone())),
                    _ => None,
                };
                let ready = source.is_some() && self.dest_root.is_some();
                if ui.add_enabled(ready, egui::Button::new("Import")).clicked() {
                    self.last_outcome = None;
                    start = Some(ImportSpec {
                        source: source.unwrap(),
                        dest_root: self.dest_root.clone().unwrap(),
                        mode: match self.mode {
                            ImportModeChoice::Copy => ImportMode::Copy,
                            ImportModeChoice::Move => ImportMode::Move,
                        },
                    });
                }
            });

        self.open = open;
        start
    }

    fn files_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sources");
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
                        if ui.small_button("✕").clicked() {
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
            ui.heading("Camera");
            if ui.button("Refresh").clicked() {
                self.refresh_cameras();
            }
        });
        if let Some(err) = &self.camera_error {
            ui.colored_label(egui::Color32::LIGHT_RED, err);
        }
        for (i, cam) in self.cameras.iter().enumerate() {
            ui.radio_value(&mut self.selected_camera, i, format!("{} ({})", cam.model, cam.port));
        }
        if !self.cameras.is_empty() {
            ui.label(
                egui::RichText::new(
                    "New photos on the card will be copied in; already-imported ones are skipped. \
                     Nothing is deleted from the card.",
                )
                .weak()
                .small(),
            );
        }
    }
}
