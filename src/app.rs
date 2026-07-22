use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, ImageRecord};
use crate::jobs::{JobResult, Jobs};
use crate::ui::{grid, import_dialog::ImportDialog, top_bar};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Library,
    Develop,
}

/// Small persisted app config (not part of the catalog).
#[derive(Default, Serialize, Deserialize)]
pub struct AppConfig {
    pub last_import_dest: Option<PathBuf>,
}

impl AppConfig {
    fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("effective-darkroom/config.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

pub struct App {
    catalog: Catalog,
    images: Vec<ImageRecord>,
    jobs: Jobs,
    view: View,
    grid: grid::GridState,
    import: ImportDialog,
    config: AppConfig,
    startup_error: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let config = AppConfig::load();
        let (catalog, images, startup_error) = match Catalog::open_default() {
            Ok(cat) => match cat.all_images() {
                Ok(imgs) => (cat, imgs, None),
                Err(e) => (cat, Vec::new(), Some(format!("failed to list images: {e:#}"))),
            },
            Err(e) => {
                // An unusable catalog is fatal for everything else; surface it.
                panic!("cannot open catalog: {e:#}");
            }
        };

        let mut import = ImportDialog::default();
        import.dest_root = config.last_import_dest.clone();

        Self {
            catalog,
            images,
            jobs: Jobs::new(cc.egui_ctx.clone()),
            view: View::Library,
            grid: grid::GridState::default(),
            import,
            config,
            startup_error,
        }
    }

    fn drain_results(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.jobs.results_rx.try_recv() {
            match result {
                JobResult::ThumbReady { image_id, image } => {
                    self.grid.on_thumb_ready(ctx, image_id, image);
                }
                JobResult::ThumbFailed { image_id, error } => {
                    tracing::warn!("thumbnail failed for image {image_id}: {error}");
                    self.grid.on_thumb_failed(image_id);
                }
                JobResult::ImportProgress {
                    current,
                    total,
                    file_name,
                } => {
                    self.import.progress = Some((current, total, file_name));
                }
                JobResult::ImportDone(outcome) => {
                    self.import.progress = None;
                    self.import.last_outcome = Some(outcome);
                    match self.catalog.all_images() {
                        Ok(imgs) => self.images = imgs,
                        Err(e) => tracing::error!("reload images: {e:#}"),
                    }
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain_results(&ctx);

        let action = top_bar::show(ui, &mut self.view);
        if action.open_import {
            self.import.open = true;
        }

        if let Some(spec) = self.import.show(&ctx) {
            self.config.last_import_dest = Some(spec.dest_root.clone());
            self.config.save();
            self.import.progress = Some((0, 0, String::from("scanning…")));
            crate::importer::run(spec, self.jobs.results_tx.clone(), ctx.clone());
        }

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(err) = &self.startup_error {
                ui.colored_label(egui::Color32::LIGHT_RED, err);
            }
            match self.view {
                View::Library => {
                    grid::show(ui, &mut self.grid, &self.images, &self.jobs);
                }
                View::Develop => {
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new("Develop module coming soon").weak());
                    });
                }
            }
        });
    }
}
