use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, FileKind, ImageRecord};
use crate::decode::full::{AsShotWb, FullPixels};
use crate::engine::curve::BaseCurve;
use crate::engine::params::{EditParams, PARAMS_VERSION};
use crate::engine::{RenderEngine, SourcePixels};
use crate::jobs::{JobResult, Jobs};
use crate::ui::{
    delete_dialog::{DeleteAction, DeleteDialog},
    develop_panel,
    export_dialog::ExportDialog,
    grid,
    import_dialog::{DialogAction, ImportDialog},
    loupe,
    tone_curve::ToneCurveState,
    top_bar,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Library,
    Develop,
}

/// Within the Library module: thumbnail grid or single-image loupe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryMode {
    Grid,
    Loupe,
}

const SAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

/// Small persisted app config (not part of the catalog).
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Default destination for imports; asked for once on first startup.
    pub library_root: Option<PathBuf>,
    /// Legacy field, migrated into `library_root`.
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

/// Editing state for the currently selected image.
struct CurrentImage {
    id: i64,
    params: EditParams,
    /// Params as last written to the catalog (for debounced saves).
    saved_params: EditParams,
    /// Params as last rendered (dirty check).
    rendered_params: Option<EditParams>,
    dirty_since: Option<Instant>,
    /// Waiting on a decode; the render source is stale or absent.
    loading: bool,
    /// True if the catalog had an edit when this image was opened.
    had_saved_edit: bool,
    /// As-shot WB estimate (RAW only), used as the WB reference point.
    as_shot: Option<AsShotWb>,
    /// Per-camera base render curve (RAW only).
    base_curve: Option<BaseCurve>,
    /// True once the engine source is the linear RAW decode (the base curve
    /// only applies then; the embedded preview is already camera-toned).
    source_is_linear: bool,
}

impl CurrentImage {
    /// The white point the source pixels are balanced to.
    fn ref_wb(&self) -> (f32, f32) {
        match self.as_shot {
            Some(ws) => (ws.temp, ws.tint),
            None => (crate::engine::params::TEMP_DEFAULT, 0.0),
        }
    }
}

pub struct App {
    catalog: Catalog,
    images: Vec<ImageRecord>,
    jobs: Jobs,
    engine: Option<RenderEngine>,
    view: View,
    library_mode: LibraryMode,
    grid: grid::GridState,
    loupe: loupe::LoupeState,
    import: ImportDialog,
    export: ExportDialog,
    delete: DeleteDialog,
    config: AppConfig,
    /// First-run prompt asking where the library should live.
    library_prompt_open: bool,
    curve_state: ToneCurveState,
    /// Develop settings captured by "Copy" for pasting onto other images.
    copied_params: Option<EditParams>,
    current: Option<CurrentImage>,
    decode_generation: u64,
    /// Latest rendered output for the loupe.
    rendered: Option<(egui::TextureId, egui::Vec2)>,
    startup_error: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let engine = cc
            .wgpu_render_state
            .as_ref()
            .map(RenderEngine::new);
        if engine.is_none() {
            tracing::error!("no wgpu render state; develop module disabled");
        }

        let mut config = AppConfig::load();
        // Migrate the pre-library_root config field.
        if config.library_root.is_none() {
            if let Some(dest) = config.last_import_dest.take() {
                config.library_root = Some(dest);
                config.save();
            }
        }

        let (catalog, images, startup_error) = match Catalog::open_default() {
            Ok(cat) => match cat.all_images() {
                Ok(imgs) => (cat, imgs, None),
                Err(e) => (cat, Vec::new(), Some(format!("failed to list images: {e:#}"))),
            },
            Err(e) => panic!("cannot open catalog: {e:#}"),
        };

        let mut import = ImportDialog::default();
        import.dest_root = config.library_root.clone();
        let library_prompt_open = config.library_root.is_none();

        // Dev helpers (useful for automated screenshots/tests):
        // ED_STARTUP_VIEW=develop opens the develop module on startup;
        // ED_STARTUP_IMAGE=<substring> preselects the first matching file.
        let view = match std::env::var("ED_STARTUP_VIEW").as_deref() {
            Ok("develop") => View::Develop,
            _ => View::Library,
        };
        let mut grid_state = grid::GridState::default();
        if let Ok(pat) = std::env::var("ED_STARTUP_IMAGE") {
            if let Some(rec) = images.iter().find(|r| r.file_name.contains(&pat)) {
                grid_state.selection.insert(rec.id);
            }
        }

        Self {
            catalog,
            images,
            jobs: Jobs::new(cc.egui_ctx.clone()),
            engine,
            view,
            library_mode: LibraryMode::Grid,
            grid: grid_state,
            loupe: loupe::LoupeState::default(),
            import,
            export: ExportDialog::default(),
            delete: DeleteDialog::default(),
            config,
            library_prompt_open,
            curve_state: ToneCurveState::default(),
            copied_params: None,
            current: None,
            decode_generation: 0,
            rendered: None,
            startup_error,
        }
    }

    fn image_index(&self, id: i64) -> Option<usize> {
        self.images.iter().position(|r| r.id == id)
    }

    /// Switch the selected/edited image, saving pending edits first.
    fn set_current(&mut self, id: i64) {
        if self.current.as_ref().is_some_and(|c| c.id == id) {
            return;
        }
        self.flush_edits();

        let Some(rec) = self.images.iter().find(|r| r.id == id).cloned() else {
            return;
        };
        let params = match self.catalog.load_edit(id) {
            Ok(Some((_ver, json))) => EditParams::from_json(&json),
            Ok(None) => {
                let mut p = EditParams::default();
                // Camera JPEGs are already denoised in-camera.
                if rec.kind == FileKind::Jpeg {
                    p.color_nr = 0.0;
                }
                p
            }
            Err(e) => {
                tracing::error!("load edit for {id}: {e:#}");
                EditParams::default()
            }
        };
        let had_saved_edit = matches!(self.catalog.load_edit(id), Ok(Some(_)));
        let base_curve = (rec.kind == FileKind::Raw)
            .then(|| BaseCurve::for_camera(&rec.camera_model));
        self.current = Some(CurrentImage {
            id,
            saved_params: params.clone(),
            rendered_params: None,
            params,
            dirty_since: None,
            loading: true,
            had_saved_edit,
            as_shot: None,
            base_curve,
            source_is_linear: false,
        });
        self.rendered = None;
        self.loupe.reset_view();
        if let Some(engine) = &mut self.engine {
            engine.clear_source();
        }
        self.grid.selection.clear();
        self.grid.selection.insert(id);

        self.decode_generation += 1;
        self.jobs.request_full_decode(self.decode_generation, rec);
    }

    /// Write pending edit changes for the current image to the catalog.
    fn flush_edits(&mut self) {
        if let Some(cur) = &mut self.current {
            if cur.params != cur.saved_params {
                if let Err(e) =
                    self.catalog
                        .save_edit(cur.id, PARAMS_VERSION, &cur.params.to_json())
                {
                    tracing::error!("save edit for {}: {e:#}", cur.id);
                } else {
                    cur.saved_params = cur.params.clone();
                }
            }
            cur.dirty_since = None;
        }
    }

    fn step_current(&mut self, delta: i64) {
        let Some(cur_id) = self.current.as_ref().map(|c| c.id) else {
            // No current image: pick the first.
            if let Some(first) = self.images.first() {
                self.set_current(first.id);
            }
            return;
        };
        if let Some(idx) = self.image_index(cur_id) {
            let new_idx = (idx as i64 + delta).clamp(0, self.images.len() as i64 - 1);
            let new_id = self.images[new_idx as usize].id;
            self.set_current(new_id);
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
                JobResult::FullReady {
                    image_id,
                    generation,
                    image,
                    is_final,
                } => {
                    if generation != self.decode_generation {
                        continue; // stale
                    }
                    let Some(cur) = &mut self.current else { continue };
                    if cur.id != image_id {
                        continue;
                    }
                    cur.loading = !is_final;
                    cur.rendered_params = None; // force re-render

                    if let Some(as_shot) = image.as_shot {
                        cur.as_shot = Some(as_shot);
                        // A RAW opened for the first time adopts the As-Shot
                        // values so the sliders show real Kelvin numbers and
                        // the render starts at identity.
                        if !cur.had_saved_edit
                            && cur.params.wb_temp
                                == crate::engine::params::TEMP_DEFAULT
                            && cur.params.wb_tint == 0.0
                            && cur.params == cur.saved_params
                        {
                            cur.params.wb_temp = as_shot.temp;
                            cur.params.wb_tint = as_shot.tint;
                            cur.saved_params = cur.params.clone();
                        }
                    }

                    if let Some(engine) = &mut self.engine {
                        match &image.pixels {
                            FullPixels::Srgb8(rgba) => {
                                cur.source_is_linear = false;
                                engine.set_source(
                                    image.width,
                                    image.height,
                                    SourcePixels::Srgb8(rgba),
                                );
                            }
                            FullPixels::LinearF16(rgba) => {
                                cur.source_is_linear = true;
                                engine.set_source(
                                    image.width,
                                    image.height,
                                    SourcePixels::LinearF16(rgba),
                                );
                            }
                        }
                    }
                }
                JobResult::FullFailed {
                    image_id,
                    generation,
                    error,
                } => {
                    if generation == self.decode_generation {
                        tracing::error!("full decode failed for {image_id}: {error}");
                        if let Some(cur) = &mut self.current {
                            if cur.id == image_id {
                                cur.loading = false;
                            }
                        }
                    }
                }
                JobResult::ImportProgress {
                    current,
                    total,
                    file_name,
                } => {
                    self.import.progress = Some((current, total, file_name));
                }
                JobResult::ImportDone(outcome) => {
                    self.import.on_import_done(outcome);
                    match self.catalog.all_images() {
                        Ok(imgs) => self.images = imgs,
                        Err(e) => tracing::error!("reload images: {e:#}"),
                    }
                }
                JobResult::ImportScanList {
                    generation,
                    candidates,
                } => {
                    self.import.on_scan_list(generation, candidates);
                }
                JobResult::ImportScanThumb {
                    generation,
                    key,
                    image,
                } => {
                    self.import.on_scan_thumb(ctx, generation, key, image);
                }
                JobResult::ImportScanFailed { generation, error } => {
                    self.import.on_scan_failed(generation, error);
                }
                JobResult::ExportProgress {
                    current,
                    total,
                    file_name,
                } => {
                    self.export.progress = Some((current, total, file_name));
                }
                JobResult::ExportDone(outcome) => {
                    self.export.progress = None;
                    self.export.last_outcome = Some(outcome);
                }
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (left, right, key_g, key_e, key_d, key_del) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::G),
                i.key_pressed(egui::Key::E),
                i.key_pressed(egui::Key::D),
                i.key_pressed(egui::Key::Delete),
            )
        });
        if key_del && !self.grid.selection.is_empty() {
            self.delete
                .request(self.grid.selection.iter().copied().collect());
        }
        if left {
            self.step_current(-1);
        }
        if right {
            self.step_current(1);
        }
        if key_g {
            self.view = View::Library;
            self.library_mode = LibraryMode::Grid;
        }
        if key_e {
            self.view = View::Library;
            self.library_mode = LibraryMode::Loupe;
            self.ensure_current_from_selection();
        }
        if key_d {
            self.view = View::Develop;
            self.ensure_current_from_selection();
        }
    }

    /// Entering loupe/develop with no current image: use the grid selection
    /// or first image.
    fn ensure_current_from_selection(&mut self) {
        if self.current.is_some() {
            return;
        }
        let id = self
            .grid
            .selection
            .iter()
            .next()
            .copied()
            .or_else(|| self.images.first().map(|r| r.id));
        if let Some(id) = id {
            self.set_current(id);
        }
    }

    /// Remove the images queued in the delete dialog from the catalog,
    /// optionally moving the files to the system trash.
    fn delete_images(&mut self, from_disk: bool) {
        let ids = std::mem::take(&mut self.delete.ids);
        if ids.is_empty() {
            return;
        }
        let records: Vec<ImageRecord> = self
            .images
            .iter()
            .filter(|r| ids.contains(&r.id))
            .cloned()
            .collect();

        if let Err(e) = self.catalog.delete_images(&ids) {
            tracing::error!("delete from catalog: {e:#}");
            return;
        }
        for rec in &records {
            let thumb = crate::catalog::thumbs_dir().join(format!("{}.jpg", rec.id));
            let _ = std::fs::remove_file(thumb);
            if from_disk {
                if let Err(e) = trash::delete(&rec.abs_path) {
                    tracing::error!("trash {}: {e:#}", rec.abs_path.display());
                }
            }
        }

        // Refresh state.
        if self
            .current
            .as_ref()
            .is_some_and(|c| ids.contains(&c.id))
        {
            self.current = None;
            self.rendered = None;
            if let Some(engine) = &mut self.engine {
                engine.clear_source();
            }
        }
        for id in &ids {
            self.grid.selection.remove(id);
            self.grid.textures.pop(id);
        }
        match self.catalog.all_images() {
            Ok(imgs) => self.images = imgs,
            Err(e) => tracing::error!("reload images: {e:#}"),
        }
    }

    /// Apply the copied develop settings to every selected image.
    fn paste_to_selection(&mut self) {
        let Some(params) = self.copied_params.clone() else { return };
        let json = params.to_json();
        for &id in &self.grid.selection {
            if let Err(e) = self.catalog.save_edit(id, PARAMS_VERSION, &json) {
                tracing::error!("paste settings to {id}: {e:#}");
            }
            // Keep the in-memory state in sync if the current image was hit.
            if let Some(cur) = &mut self.current {
                if cur.id == id {
                    cur.params = params.clone();
                    cur.saved_params = params.clone();
                    cur.dirty_since = None;
                }
            }
        }
    }

    /// Re-render through the engine if params changed or a new source landed.
    fn maybe_render(&mut self) {
        let Some(engine) = &mut self.engine else { return };
        let Some(cur) = &mut self.current else { return };
        if !engine.has_source() {
            return;
        }
        if cur.rendered_params.as_ref() == Some(&cur.params) {
            return;
        }
        let (ref_temp, ref_tint) = cur.ref_wb();
        let base = if cur.source_is_linear {
            cur.base_curve.as_ref()
        } else {
            None
        };
        self.rendered = engine.render(&cur.params, ref_temp, ref_tint, base);
        cur.rendered_params = Some(cur.params.clone());
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain_results(&ctx);
        self.handle_keys(&ctx);

        if self.library_prompt_open {
            egui::Window::new("Welcome to Effective Darkroom")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(&ctx, |ui| {
                    ui.label("Choose where your photo library should live.");
                    ui.label(
                        egui::RichText::new(
                            "Imported photos are copied here into YYYY/MM-DD folders. \
                             This becomes the default import destination.",
                        )
                        .weak()
                        .small(),
                    );
                    ui.add_space(8.0);
                    if ui.button("Choose library folder…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.config.library_root = Some(dir.clone());
                            self.config.save();
                            self.import.dest_root = Some(dir);
                            self.library_prompt_open = false;
                        }
                    }
                });
        }

        let action = top_bar::show(ui, &mut self.view, self.grid.selection.len());
        if action.open_import {
            self.import.open = true;
            // Default the destination to the library location.
            if self.import.dest_root.is_none() {
                self.import.dest_root = self.config.library_root.clone();
            }
        }
        if action.open_export {
            self.export.open = true;
        }
        if self.view == View::Develop {
            self.ensure_current_from_selection();
        }

        match self.import.show(&ctx) {
            DialogAction::None => {}
            DialogAction::StartScan(source) => {
                let cancel = self
                    .import
                    .scan_cancel_flag()
                    .unwrap_or_default();
                crate::importer::scan(
                    source,
                    self.jobs.results_tx.clone(),
                    ctx.clone(),
                    self.import.scan_generation,
                    cancel,
                );
            }
            DialogAction::StartImport(spec, cancel) => {
                if self.config.library_root.is_none() {
                    self.config.library_root = Some(spec.dest_root.clone());
                    self.config.save();
                }
                crate::importer::run(spec, self.jobs.results_tx.clone(), ctx.clone(), cancel);
            }
        }

        match self.delete.show(&ctx) {
            DeleteAction::None => {}
            DeleteAction::RemoveFromLibrary => self.delete_images(false),
            DeleteAction::DeleteFromDisk => self.delete_images(true),
        }

        if let Some(mut spec) = self.export.show(&ctx, self.grid.selection.len()) {
            // Export in library order; make sure pending edits are saved so
            // the export renders what the user sees.
            self.flush_edits();
            spec.image_ids = self
                .images
                .iter()
                .filter(|r| self.grid.selection.contains(&r.id))
                .map(|r| r.id)
                .collect();
            if let Some(engine) = &self.engine {
                let (device, queue) = engine.device_queue();
                self.export.progress =
                    Some((0, spec.image_ids.len(), String::from("starting…")));
                self.jobs.start_export(spec, device, queue, ctx.clone());
            }
        }

        // Grid double-click opens the loupe.
        if let Some(id) = self.grid.activated.take() {
            self.set_current(id);
            self.library_mode = LibraryMode::Loupe;
        }

        // Debounced edit persistence.
        if let Some(cur) = &self.current {
            if cur
                .dirty_since
                .is_some_and(|t| t.elapsed() >= SAVE_DEBOUNCE)
            {
                self.flush_edits();
            } else if cur.dirty_since.is_some() {
                ctx.request_repaint_after(SAVE_DEBOUNCE);
            }
        }

        match self.view {
            View::Library => {
                egui::CentralPanel::default().show(ui, |ui| {
                    if let Some(err) = &self.startup_error {
                        ui.colored_label(egui::Color32::LIGHT_RED, err);
                    }
                    match self.library_mode {
                        LibraryMode::Grid => {
                            let resp = grid::show(
                                ui,
                                &mut self.grid,
                                &self.images,
                                &self.jobs,
                                self.copied_params.is_some(),
                            );
                            if resp.paste_to_selection {
                                self.paste_to_selection();
                            }
                            if resp.delete_selection {
                                self.delete.request(
                                    self.grid.selection.iter().copied().collect(),
                                );
                            }
                            if let Some(&id) = self.grid.selection.iter().next() {
                                if self.grid.selection.len() == 1 {
                                    // Keep current in sync for E/D shortcuts.
                                    if self.current.as_ref().map(|c| c.id) != Some(id) {
                                        self.set_current(id);
                                    }
                                }
                            }
                        }
                        LibraryMode::Loupe => {
                            self.maybe_render();
                            let loading =
                                self.current.as_ref().is_some_and(|c| c.loading);
                            loupe::show(ui, &mut self.loupe, self.rendered, loading);
                        }
                    }
                });
            }
            View::Develop => {
                if let Some(engine) = &mut self.engine {
                    engine.poll_histogram();
                }
                let histogram = self
                    .engine
                    .as_ref()
                    .and_then(|e| e.histogram.clone());
                egui::Panel::right("develop_panel")
                    .default_size(320.0)
                    .resizable(true)
                    .show(ui, |ui| {
                        if let Some(cur) = &mut self.current {
                            let as_shot = cur.as_shot;
                            let resp = develop_panel::show(
                                ui,
                                &mut cur.params,
                                as_shot.map(|w| (w.temp, w.tint)),
                                &mut self.curve_state,
                                histogram.as_deref(),
                                self.copied_params.is_some(),
                            );
                            if resp.copy {
                                self.copied_params = Some(cur.params.clone());
                            }
                            if resp.paste {
                                if let Some(p) = &self.copied_params {
                                    cur.params = p.clone();
                                }
                            }
                            if resp.reset {
                                cur.params = EditParams::default();
                                if let Some(w) = as_shot {
                                    cur.params.wb_temp = w.temp;
                                    cur.params.wb_tint = w.tint;
                                }
                            }
                            if resp.changed || resp.reset || resp.paste {
                                cur.dirty_since = Some(Instant::now());
                            }
                        } else {
                            ui.label(
                                egui::RichText::new("Select an image in the Library").weak(),
                            );
                        }
                    });
                egui::CentralPanel::default().show(ui, |ui| {
                    self.maybe_render();
                    let loading = self.current.as_ref().is_some_and(|c| c.loading);
                    loupe::show(ui, &mut self.loupe, self.rendered, loading);
                });
            }
        }
    }

    fn on_exit(&mut self) {
        self.flush_edits();
    }
}
