//! Import dialog: pick sources and a destination root, choose copy or move,
//! watch progress.

use std::path::PathBuf;

use crate::importer::{ImportMode, ImportOutcome, ImportSpec};

#[derive(Default)]
pub struct ImportDialog {
    pub open: bool,
    sources: Vec<PathBuf>,
    pub dest_root: Option<PathBuf>,
    mode: ImportModeChoice,
    /// (current, total, file name) while an import runs.
    pub progress: Option<(usize, usize, String)>,
    pub last_outcome: Option<ImportOutcome>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum ImportModeChoice {
    #[default]
    Copy,
    Move,
}

impl ImportDialog {
    pub fn running(&self) -> bool {
        self.progress.is_some()
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
            .default_width(460.0)
            .show(ctx, |ui| {
                if let Some((cur, total, name)) = &self.progress {
                    ui.label(format!("Importing {cur} of {total}…"));
                    ui.add(
                        egui::ProgressBar::new(*cur as f32 / (*total).max(1) as f32)
                            .text(name.clone()),
                    );
                    return;
                }

                ui.heading("Sources");
                ui.horizontal(|ui| {
                    if ui.button("Add files…").clicked() {
                        if let Some(files) = rfd::FileDialog::new()
                            .add_filter("Photos", &["cr3", "CR3", "raf", "RAF", "jpg", "JPG", "jpeg", "JPEG"])
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
                    egui::RichText::new("Photos are organized into YYYY/MM-DD folders by capture date.")
                        .weak()
                        .small(),
                );

                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.mode, ImportModeChoice::Copy, "Copy");
                    ui.radio_value(&mut self.mode, ImportModeChoice::Move, "Move");
                });

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

                let ready = !self.sources.is_empty() && self.dest_root.is_some();
                if ui
                    .add_enabled(ready, egui::Button::new("Import"))
                    .clicked()
                {
                    self.last_outcome = None;
                    start = Some(ImportSpec {
                        sources: self.sources.clone(),
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
}
