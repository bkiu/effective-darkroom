//! Settings: library default location, catalog location, the move
//! operations for both, and the Immich server connection.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::maintenance::MoveOutcome;
use crate::metadata_transfer::{ApplyOptions, ImportPlan, OnConflict};

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
    /// Edit buffers for the Immich connection; the app syncs these with the
    /// config while the dialog is open.
    pub immich_url: String,
    pub immich_api_key: String,
    /// Result of the last "Test connection": account name or error.
    pub immich_test: Option<Result<String, String>>,
    pub immich_testing: bool,
    /// Outcome of the last metadata export/import, shown under the buttons.
    pub metadata_message: Option<String>,
    /// A bundle that has been read and measured, waiting for the user to
    /// confirm what it should do.
    pending_metadata: Option<PendingMetadata>,
}

/// A metadata file picked for import, with what applying it would do.
struct PendingMetadata {
    path: PathBuf,
    plan: ImportPlan,
    overwrite: bool,
    edits: bool,
    flags: bool,
    collections: bool,
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
    /// Validate the Immich URL + API key in the background.
    TestImmich { url: String, api_key: String },
    /// Write every edit, flag and collection membership to this file.
    ExportMetadata(PathBuf),
    /// Read this bundle and work out what applying it would do; the app
    /// answers with `on_metadata_scanned`.
    ScanMetadata(PathBuf),
    /// Apply the bundle that is currently pending.
    ApplyMetadata { path: PathBuf, options: ApplyOptions },
}

impl SettingsDialog {
    /// The app has read a bundle: show its plan for confirmation.
    pub fn on_metadata_scanned(&mut self, path: PathBuf, plan: ImportPlan) {
        self.metadata_message = None;
        self.pending_metadata = Some(PendingMetadata {
            path,
            plan,
            // Default to the cautious answer; the file is usually newer, but
            // silently replacing local work is the one unrecoverable move.
            overwrite: false,
            edits: true,
            flags: true,
            collections: true,
        });
    }

    /// A finished (or failed) export/import: drop the prompt, show the result.
    pub fn on_metadata_done(&mut self, message: String) {
        self.pending_metadata = None;
        self.metadata_message = Some(message);
    }

    pub fn on_move_done(&mut self, outcome: MoveOutcome) {
        self.move_progress = None;
        self.move_cancel = None;
        self.last_outcome = Some(outcome);
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        library_root: Option<&PathBuf>,
        catalog_dir: &std::path::Path,
    ) -> SettingsAction {
        if !self.open {
            return SettingsAction::None;
        }
        let mut action = SettingsAction::None;
        let mut open = self.open;

        egui::Window::new("Settings")
            .open(&mut open)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .collapsible(false)
            .resizable(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.label(crate::ui::theme::overline_text("Library"));
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
                        if crate::ui::theme::primary_button(ui, "Move library").clicked()
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
                ui.label(crate::ui::theme::overline_text("Catalog"));
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

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.label(crate::ui::theme::overline_text("Transfer"));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Carry develop edits, flags and collections to another \
                         machine. Copy the photos across and import them there \
                         first, then apply the file — photos are matched by \
                         file name and size, never by folder.",
                    )
                    .weak()
                    .small(),
                );
                ui.add_space(6.0);

                let mut cancel_pending = false;
                if let Some(pend) = self.pending_metadata.as_mut() {
                    let plan = &pend.plan;
                    ui.label(
                        egui::RichText::new(
                            pend.path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        )
                        .strong(),
                    );
                    ui.label(format!(
                        "{} photos in the file, {} found in this library",
                        plan.photos, plan.matched
                    ));
                    if plan.unmatched > 0 {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} not here yet — import those photos, then apply \
                                 the file again",
                                plan.unmatched
                            ))
                            .weak()
                            .small(),
                        );
                    }
                    if !plan.new_collections.is_empty() {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} new collection(s): {}",
                                plan.new_collections.len(),
                                plan.new_collections.join(", ")
                            ))
                            .weak()
                            .small(),
                        );
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Apply:");
                        ui.checkbox(&mut pend.edits, "Edits");
                        ui.checkbox(&mut pend.flags, "Flags");
                        ui.checkbox(&mut pend.collections, "Collections");
                    });
                    ui.add_space(4.0);
                    if plan.conflicts > 0 {
                        ui.label(format!(
                            "⚠ {} of those already have edits or flags here:",
                            plan.conflicts
                        ));
                        ui.radio_value(
                            &mut pend.overwrite,
                            false,
                            format!("Keep my work, apply the other {}", 
                                plan.matched.saturating_sub(plan.conflicts)),
                        );
                        ui.radio_value(
                            &mut pend.overwrite,
                            true,
                            format!("Overwrite all {}", plan.matched),
                        );
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let can_apply = plan.matched > 0
                            && (pend.edits || pend.flags || pend.collections);
                        if ui
                            .add_enabled(
                                can_apply,
                                egui::Button::new("Apply"),
                            )
                            .clicked()
                        {
                            action = SettingsAction::ApplyMetadata {
                                path: pend.path.clone(),
                                options: ApplyOptions {
                                    edits: pend.edits,
                                    flags: pend.flags,
                                    collections: pend.collections,
                                    on_conflict: if pend.overwrite {
                                        OnConflict::Overwrite
                                    } else {
                                        OnConflict::Skip
                                    },
                                },
                            };
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_pending = true;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        if ui
                            .button("Export metadata…")
                            .on_hover_text(
                                "Write every edit, flag and collection to one file",
                            )
                            .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_file_name(format!(
                                    "library.{}",
                                    crate::metadata_transfer::EXTENSION
                                ))
                                .add_filter(
                                    "Darkroom metadata",
                                    &[crate::metadata_transfer::EXTENSION],
                                )
                                .save_file()
                            {
                                action = SettingsAction::ExportMetadata(path);
                            }
                        }
                        if ui
                            .button("Import metadata…")
                            .on_hover_text(
                                "Apply a file written by another machine",
                            )
                            .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter(
                                    "Darkroom metadata",
                                    &[crate::metadata_transfer::EXTENSION],
                                )
                                .pick_file()
                            {
                                action = SettingsAction::ScanMetadata(path);
                            }
                        }
                    });
                }
                if cancel_pending {
                    self.pending_metadata = None;
                }
                if let Some(msg) = &self.metadata_message {
                    ui.label(egui::RichText::new(msg).weak());
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.label(crate::ui::theme::overline_text("Immich"));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Server URL");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.immich_url)
                            .hint_text("https://immich.example.com")
                            .desired_width(280.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("API key");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.immich_api_key)
                            .password(true)
                            .hint_text("paste an API key")
                            .desired_width(280.0),
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "Create a key in Immich under Account Settings > API Keys. \
                         Used by the Immich tab of the export dialog; stored in \
                         plain text in config.json.",
                    )
                    .weak()
                    .small(),
                );
                ui.add_space(4.0);
                let can_test = !self.immich_url.trim().is_empty()
                    && !self.immich_api_key.trim().is_empty()
                    && !self.immich_testing;
                if ui
                    .add_enabled(can_test, egui::Button::new("Test connection"))
                    .clicked()
                {
                    self.immich_test = None;
                    self.immich_testing = true;
                    action = SettingsAction::TestImmich {
                        url: self.immich_url.trim().to_string(),
                        api_key: self.immich_api_key.trim().to_string(),
                    };
                }
                if self.immich_testing {
                    ui.label(egui::RichText::new("Testing…").weak());
                }
                match &self.immich_test {
                    Some(Ok(who)) => {
                        ui.label(format!("✔ Connected as {who}"));
                    }
                    Some(Err(e)) => {
                        ui.colored_label(egui::Color32::LIGHT_RED, e);
                    }
                    None => {}
                }
                ui.add_space(4.0);
            });

        self.open = open;
        action
    }
}
