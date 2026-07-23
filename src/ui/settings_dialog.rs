//! Settings: library default location, catalog location, and the move
//! operations for both.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::maintenance::MoveOutcome;

#[derive(Default)]
pub struct SettingsDialog {
    pub open: bool,
    /// Destination picked for a pending library move, awaiting confirmation.
    pending_library_move: Option<PathBuf>,
    pub move_cancel: Option<Arc<AtomicBool>>,
    /// (current, total, file name) while a library move runs.
    pub move_progress: Option<(usize, usize, String)>,
    pub last_outcome: Option<MoveOutcome>,
    pub catalog_message: Option<String>,
}

/// What the settings dialog wants the app to do this frame.
pub enum SettingsAction {
    None,
    /// Change the default import destination (no files moved).
    SetLibraryRoot(PathBuf),
    /// Move all library files to this new root.
    StartLibraryMove(PathBuf),
    /// Move catalog.db + thumbs to this directory.
    MoveCatalog(PathBuf),
}

impl SettingsDialog {
    pub fn on_move_done(&mut self, outcome: MoveOutcome) {
        self.move_progress = None;
        self.move_cancel = None;
        self.last_outcome = Some(outcome);
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        library_root: Option<&PathBuf>,
        catalog_dir: &PathBuf,
    ) -> SettingsAction {
        if !self.open {
            return SettingsAction::None;
        }
        let mut action = SettingsAction::None;
        let mut open = self.open;

        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Library").strong());
                ui.add_space(4.0);
                ui.label("Default location for imported photos:");
                ui.label(
                    egui::RichText::new(
                        library_root
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "not set".into()),
                    )
                    .weak(),
                );
                ui.add_space(4.0);

                if let Some((cur, total, name)) = self.move_progress.clone() {
                    ui.label(format!("Moving library: {cur} of {total}…"));
                    ui.add(
                        egui::ProgressBar::new(cur as f32 / total.max(1) as f32).text(name),
                    );
                    let stopping = self
                        .move_cancel
                        .as_ref()
                        .is_some_and(|c| c.load(Ordering::Relaxed));
                    if stopping {
                        ui.label(
                            egui::RichText::new("Stopping after the current file…").weak(),
                        );
                    } else if ui.button("Stop").clicked() {
                        if let Some(c) = &self.move_cancel {
                            c.store(true, Ordering::Relaxed);
                        }
                    }
                } else if let Some(dest) = self.pending_library_move.clone() {
                    ui.label(format!("Move all library photos to: {}", dest.display()));
                    ui.label(
                        egui::RichText::new(
                            "Files are moved preserving their date folders; the catalog \
                             is updated to the new paths. Photos stored outside the \
                             library folder are left untouched.",
                        )
                        .weak()
                        .small(),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .button(egui::RichText::new("Move library").strong())
                            .clicked()
                        {
                            action = SettingsAction::StartLibraryMove(dest.clone());
                            self.pending_library_move = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending_library_move = None;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        if ui
                            .button("Change default…")
                            .on_hover_text(
                                "Future imports go here; existing photos stay put",
                            )
                            .clicked()
                        {
                            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                action = SettingsAction::SetLibraryRoot(dir);
                            }
                        }
                        if library_root.is_some()
                            && ui
                                .button("Move library…")
                                .on_hover_text(
                                    "Relocate all library photos to a new folder",
                                )
                                .clicked()
                        {
                            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                self.pending_library_move = Some(dir);
                                self.last_outcome = None;
                            }
                        }
                    });
                }

                if let Some(out) = &self.last_outcome {
                    let mut msg = format!("Moved {} photos", out.moved);
                    if out.skipped_outside > 0 {
                        msg += &format!(
                            ", {} outside the library left in place",
                            out.skipped_outside
                        );
                    }
                    if out.cancelled {
                        msg += " (stopped early)";
                    }
                    ui.label(msg);
                    for err in out.errors.iter().take(8) {
                        ui.colored_label(egui::Color32::LIGHT_RED, err);
                    }
                    if out.errors.len() > 8 {
                        ui.label(format!("… and {} more errors", out.errors.len() - 8));
                    }
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Catalog").strong());
                ui.add_space(4.0);
                ui.label("Database and thumbnail cache:");
                ui.label(egui::RichText::new(catalog_dir.display().to_string()).weak());
                ui.add_space(4.0);
                if ui
                    .button("Move catalog…")
                    .on_hover_text(
                        "Move the database and thumbnail cache to another folder",
                    )
                    .clicked()
                {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        action = SettingsAction::MoveCatalog(dir);
                    }
                }
                if let Some(msg) = &self.catalog_message {
                    ui.label(egui::RichText::new(msg).weak());
                }
                ui.add_space(4.0);
            });

        self.open = open;
        action
    }
}
