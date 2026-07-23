use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, Collection, FileKind, Flag, ImageRecord};
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
    /// UI zoom factor (egui zoom; adjustable with Ctrl+= / Ctrl+-).
    pub ui_zoom: Option<f32>,
    /// Last-used export settings.
    pub export: Option<crate::ui::export_dialog::ExportPrefs>,
    /// Show the EXIF info overlay in the loupe (toggled with I).
    pub show_info_overlay: Option<bool>,
    /// Custom catalog directory (Settings -> Move catalog). Also read
    /// directly by catalog::data_dir() for background workers.
    pub catalog_dir: Option<PathBuf>,
    /// Legacy field, migrated into `library_root`.
    pub last_import_dest: Option<PathBuf>,
}

/// Default UI zoom: egui's stock text is small on a desktop monitor.
const DEFAULT_UI_ZOOM: f32 = 1.25;

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
        // Merge over the existing file so fields written by a newer build
        // are preserved when an older binary saves (and vice versa).
        let mut merged = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        if let (Some(obj), Ok(serde_json::Value::Object(new))) =
            (merged.as_object_mut(), serde_json::to_value(self))
        {
            for (k, v) in new {
                obj.insert(k, v);
            }
        }
        if let Ok(json) = serde_json::to_string_pretty(&merged) {
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
    /// Capture metadata for the info overlay (from the decode).
    shot_info: Option<crate::importer::metadata::CaptureInfo>,
    /// Source pixel dimensions of the engine source (for crop geometry).
    src_dims: Option<(u32, u32)>,
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
    /// Images passing the current collection + flag filters, in library
    /// order. This is what the grid, filmstrip and arrow keys navigate.
    visible: Vec<ImageRecord>,
    visible_dirty: bool,
    collections: Vec<Collection>,
    /// None = All Photos.
    active_collection: Option<i64>,
    collection_members: std::collections::HashSet<i64>,
    filter_pick: bool,
    filter_unflagged: bool,
    filter_reject: bool,
    /// In-progress "new collection" name entry in the side panel.
    new_collection_name: Option<String>,
    new_collection_focus_pending: bool,
    jobs: Jobs,
    engine: Option<RenderEngine>,
    view: View,
    library_mode: LibraryMode,
    grid: grid::GridState,
    loupe: loupe::LoupeState,
    import: ImportDialog,
    export: ExportDialog,
    delete: DeleteDialog,
    settings: crate::ui::settings_dialog::SettingsDialog,
    config: AppConfig,
    /// First-run prompt asking where the library should live.
    library_prompt_open: bool,
    curve_state: ToneCurveState,
    /// Develop settings captured by "Copy" for pasting onto other images.
    copied_params: Option<EditParams>,
    current: Option<CurrentImage>,
    /// EXIF info overlay in the loupe, toggled with I.
    show_info: bool,
    /// Crop tool state (overlay active, aspect choice, auto status).
    crop_ui: develop_panel::CropUiState,
    /// Scroll the filmstrip to the current photo on the next frame.
    strip_scroll_pending: bool,
    latest_zoom: f32,
    decode_generation: u64,
    /// Latest rendered output for the loupe.
    rendered: Option<(egui::TextureId, egui::Vec2)>,
    logo: Option<egui::TextureHandle>,
    startup_error: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::ui::theme::apply(&cc.egui_ctx);

        let early_config = AppConfig::load();
        let zoom = early_config.ui_zoom.unwrap_or(DEFAULT_UI_ZOOM);
        cc.egui_ctx.set_zoom_factor(zoom);

        let engine = cc
            .wgpu_render_state
            .as_ref()
            .map(RenderEngine::new);
        if engine.is_none() {
            tracing::error!("no wgpu render state; develop module disabled");
        }

        let mut config = early_config;
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

        let logo = image::load_from_memory(include_bytes!("../assets/icon.png"))
            .ok()
            .map(|img| {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                cc.egui_ctx.load_texture(
                    "app-logo",
                    egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
                    egui::TextureOptions::LINEAR,
                )
            });

        let collections = catalog.list_collections().unwrap_or_default();
        Self {
            catalog,
            images,
            visible: Vec::new(),
            visible_dirty: true,
            collections,
            active_collection: None,
            collection_members: Default::default(),
            filter_pick: true,
            filter_unflagged: true,
            filter_reject: true,
            new_collection_name: None,
            new_collection_focus_pending: false,
            jobs: Jobs::new(cc.egui_ctx.clone()),
            engine,
            view,
            library_mode: LibraryMode::Grid,
            grid: grid_state,
            loupe: loupe::LoupeState::default(),
            import,
            export: {
                let mut d = ExportDialog::default();
                if let Some(prefs) = &config.export {
                    d.prefs = prefs.clone();
                }
                d
            },
            delete: DeleteDialog::default(),
            settings: Default::default(),
            show_info: config.show_info_overlay.unwrap_or(false),
            crop_ui: develop_panel::CropUiState {
                // ED_STARTUP_CROP=1: open with the crop tool active (dev hook)
                tool_active: std::env::var("ED_STARTUP_CROP").is_ok(),
                line_tool_active: false,
                aspect: Default::default(),
                auto_pending: false,
                auto_failed: false,
            },
            config,
            library_prompt_open,
            curve_state: ToneCurveState::default(),
            copied_params: None,
            current: None,
            strip_scroll_pending: false,
            latest_zoom: zoom,
            decode_generation: 0,
            rendered: None,
            logo,
            startup_error,
        }
    }

    /// Recompute the filtered view when images, filters or the active
    /// collection changed.
    fn ensure_visible(&mut self) {
        if !self.visible_dirty {
            return;
        }
        self.visible_dirty = false;
        if let Some(cid) = self.active_collection {
            self.collection_members = self.catalog.collection_image_ids(cid).unwrap_or_default();
        }
        let in_collection = |r: &ImageRecord| match self.active_collection {
            Some(_) => self.collection_members.contains(&r.id),
            None => true,
        };
        let passes_flag = |r: &ImageRecord| match r.flag {
            Flag::Pick => self.filter_pick,
            Flag::Unflagged => self.filter_unflagged,
            Flag::Reject => self.filter_reject,
        };
        self.visible = self
            .images
            .iter()
            .filter(|r| in_collection(r) && passes_flag(r))
            .cloned()
            .collect();
        // Drop selection entries that are no longer visible.
        let visible_ids: std::collections::HashSet<i64> =
            self.visible.iter().map(|r| r.id).collect();
        self.grid.selection.retain(|id| visible_ids.contains(id));
    }

    fn reload_collections(&mut self) {
        self.collections = self.catalog.list_collections().unwrap_or_default();
        if let Some(cid) = self.active_collection {
            if !self.collections.iter().any(|c| c.id == cid) {
                self.active_collection = None;
            }
        }
        self.visible_dirty = true;
    }

    /// Flag the grid selection (falling back to the current image).
    fn set_flag_selection(&mut self, flag: Flag) {
        let ids: Vec<i64> = if self.grid.selection.is_empty() {
            self.current.as_ref().map(|c| c.id).into_iter().collect()
        } else {
            self.grid.selection.iter().copied().collect()
        };
        if ids.is_empty() {
            return;
        }
        if let Err(e) = self.catalog.set_flag(&ids, flag) {
            tracing::error!("set flag: {e:#}");
            return;
        }
        for rec in &mut self.images {
            if ids.contains(&rec.id) {
                rec.flag = flag;
            }
        }
        self.visible_dirty = true;
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
            shot_info: None,
            src_dims: None,
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
        self.strip_scroll_pending = true;
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
        self.ensure_visible();
        let Some(cur_id) = self.current.as_ref().map(|c| c.id) else {
            // No current image: pick the first visible.
            if let Some(first) = self.visible.first() {
                self.set_current(first.id);
            }
            return;
        };
        let idx = self.visible.iter().position(|r| r.id == cur_id);
        match idx {
            Some(idx) => {
                let new_idx =
                    (idx as i64 + delta).clamp(0, self.visible.len() as i64 - 1);
                let new_id = self.visible[new_idx as usize].id;
                self.set_current(new_id);
            }
            None => {
                // Current image filtered out: jump to the first visible.
                if let Some(first) = self.visible.first() {
                    self.set_current(first.id);
                }
            }
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
                    cur.shot_info = Some(image.shot_info.clone());
                    cur.src_dims = Some((image.width, image.height));

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
                    self.visible_dirty = true;
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
                JobResult::MoveLibraryProgress {
                    current,
                    total,
                    file_name,
                } => {
                    self.settings.move_progress = Some((current, total, file_name));
                }
                JobResult::MoveLibraryDone(outcome) => {
                    self.settings.on_move_done(outcome);
                    match self.catalog.all_images() {
                        Ok(imgs) => self.images = imgs,
                        Err(e) => tracing::error!("reload images: {e:#}"),
                    }
                    self.visible_dirty = true;
                    // Thumbs cache keys by id, so cached thumbnails stay valid.
                }
                JobResult::AutoStraighten { image_id, angle } => {
                    self.crop_ui.auto_pending = false;
                    match angle {
                        Some(a) => {
                            if let Some(cur) = &mut self.current {
                                if cur.id == image_id {
                                    cur.params.crop.angle = a;
                                    cur.dirty_since = Some(Instant::now());
                                }
                            }
                        }
                        None => self.crop_ui.auto_failed = true,
                    }
                }
                JobResult::ExportDone(outcome) => {
                    self.export.progress = None;
                    // Close on clean success; stay open to show errors.
                    if outcome.errors.is_empty() {
                        self.export.open = false;
                        self.export.last_outcome = None;
                    } else {
                        self.export.last_outcome = Some(outcome);
                    }
                }
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (left, right, key_g, key_e, key_d, key_del, key_i) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::G),
                i.key_pressed(egui::Key::E),
                i.key_pressed(egui::Key::D),
                i.key_pressed(egui::Key::Delete),
                i.key_pressed(egui::Key::I),
            )
        });
        if key_i {
            self.show_info = !self.show_info;
            self.config.show_info_overlay = Some(self.show_info);
            self.config.save();
        }
        if key_del {
            // In the grid the selection is deleted; in loupe/develop the
            // current photo is (they're usually the same set).
            let ids: Vec<i64> = if !self.grid.selection.is_empty() {
                self.grid.selection.iter().copied().collect()
            } else {
                self.current.as_ref().map(|c| c.id).into_iter().collect()
            };
            self.delete.request(ids);
        }
        let (key_p, key_u, key_x) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::P),
                i.key_pressed(egui::Key::U),
                i.key_pressed(egui::Key::X),
            )
        });
        if key_p {
            self.set_flag_selection(Flag::Pick);
        }
        if key_u {
            self.set_flag_selection(Flag::Unflagged);
        }
        if key_x {
            self.set_flag_selection(Flag::Reject);
        }
        if left {
            self.step_current(-1);
        }
        if right {
            self.step_current(1);
        }
        let key_q = ctx.input(|i| i.key_pressed(egui::Key::Q));
        if key_g || key_q {
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
        self.ensure_visible();
        let id = self
            .grid
            .selection
            .iter()
            .next()
            .copied()
            .or_else(|| self.visible.first().map(|r| r.id));
        if let Some(id) = id {
            self.set_current(id);
        }
    }

    /// Move catalog.db + thumbs to a new directory and reopen. Runs inline;
    /// the data is small (the database plus the thumbnail cache).
    fn move_catalog(&mut self, new_dir: PathBuf) {
        let current = crate::catalog::data_dir();
        if current == new_dir {
            self.settings.catalog_message =
                Some("That is already the catalog location".into());
            return;
        }
        // Flush pending edits, then release the connection by swapping in an
        // in-memory placeholder while files move.
        self.flush_edits();
        let result = (|| -> anyhow::Result<()> {
            self.catalog.checkpoint()?;
            self.catalog = Catalog::open_at(std::path::Path::new(":memory:"))?;
            crate::maintenance::move_catalog_files(&current, &new_dir)?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.config.catalog_dir = Some(new_dir.clone());
                self.config.save();
                self.settings.catalog_message =
                    Some(format!("Catalog moved to {}", new_dir.display()));
            }
            Err(e) => {
                tracing::error!("move catalog: {e:#}");
                self.settings.catalog_message = Some(format!("Move failed: {e:#}"));
            }
        }
        // Reopen from the (possibly updated) configured location.
        match Catalog::open_default() {
            Ok(cat) => {
                self.catalog = cat;
                match self.catalog.all_images() {
                    Ok(imgs) => self.images = imgs,
                    Err(e) => tracing::error!("reload images: {e:#}"),
                }
                self.visible_dirty = true;
                self.reload_collections();
            }
            Err(e) => {
                self.startup_error = Some(format!("failed to reopen catalog: {e:#}"));
            }
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

        // Refresh state. Remember where we were so loupe/develop can advance
        // to the next photo instead of going blank.
        let deleted_current = self
            .current
            .as_ref()
            .is_some_and(|c| ids.contains(&c.id));
        let prior_index = self.current.as_ref().and_then(|c| {
            self.visible.iter().position(|r| r.id == c.id)
        });
        if deleted_current {
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
        self.reload_collections();

        let in_single_view = self.view == View::Develop
            || (self.view == View::Library && self.library_mode == LibraryMode::Loupe);
        if deleted_current && in_single_view {
            self.ensure_visible();
            if let Some(idx) = prior_index {
                let idx = idx.min(self.visible.len().saturating_sub(1));
                if let Some(rec) = self.visible.get(idx) {
                    let id = rec.id;
                    self.set_current(id);
                }
            }
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

    /// Left side panel: collection list with create/delete.
    fn show_collections_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("collections")
            .default_size(210.0)
            .resizable(true)
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(10, 10))
                    .show(ui, |ui| {
                        self.collections_panel_contents(ui);
                    });
            });
    }

    fn collections_panel_contents(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
                    ui.label(crate::ui::theme::overline_text("Collections"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("+").on_hover_text("New collection").clicked()
                            && self.new_collection_name.is_none()
                        {
                            self.new_collection_name = Some(String::new());
                            self.new_collection_focus_pending = true;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(6.0);

                let total = self.images.len();
                if crate::ui::widgets::collection_row(
                    ui,
                    "All Photos",
                    total,
                    self.active_collection.is_none(),
                )
                .clicked()
                {
                    self.active_collection = None;
                    self.visible_dirty = true;
                }

                let mut delete_collection = None;
                for col in &self.collections {
                    let selected = self.active_collection == Some(col.id);
                    let resp = crate::ui::widgets::collection_row(
                        ui,
                        &col.name,
                        col.image_count,
                        selected,
                    );
                    if resp.clicked() {
                        self.active_collection = Some(col.id);
                        self.visible_dirty = true;
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Delete collection").clicked() {
                            delete_collection = Some(col.id);
                            ui.close();
                        }
                    });
                }
                if let Some(cid) = delete_collection {
                    if let Err(e) = self.catalog.delete_collection(cid) {
                        tracing::error!("delete collection: {e:#}");
                    }
                    self.reload_collections();
                }

                if let Some(name) = &mut self.new_collection_name {
                    ui.add_space(4.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(name).hint_text("collection name"),
                    );
                    // Focus the field only when it first appears; grabbing
                    // focus every frame would swallow the Enter confirm.
                    if self.new_collection_focus_pending {
                        resp.request_focus();
                        self.new_collection_focus_pending = false;
                    }
                    let enter = resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let trimmed = name.trim().to_string();
                    let mut confirmed = enter;
                    let mut cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!trimmed.is_empty(), egui::Button::new("Create"))
                            .clicked()
                        {
                            confirmed = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancelled = true;
                        }
                    });
                    if confirmed && !trimmed.is_empty() {
                        match self.catalog.create_collection(&trimmed) {
                            Ok(cid) => {
                                self.reload_collections();
                                self.active_collection = Some(cid);
                            }
                            Err(e) => tracing::error!("create collection: {e:#}"),
                        }
                        self.new_collection_name = None;
                    } else if cancelled {
                        self.new_collection_name = None;
                    }
                }
    }

    /// Flag filter row above the grid.
    fn show_filter_row(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let (n_pick, n_reject) = self.images.iter().fold((0, 0), |(p, r), rec| {
            match rec.flag {
                Flag::Pick => (p + 1, r),
                Flag::Reject => (p, r + 1),
                Flag::Unflagged => (p, r),
            }
        });
        let n_unflagged = self.images.len() - n_pick - n_reject;
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(egui::RichText::new("Show:").weak());
            for (label, on, hover) in [
                ("⚑", &mut self.filter_pick, format!("Picked ({n_pick})")),
                ("⚐", &mut self.filter_unflagged, format!("Unflagged ({n_unflagged})")),
                ("✖", &mut self.filter_reject, format!("Rejected ({n_reject})")),
            ] {
                if ui
                    .selectable_label(*on, label)
                    .on_hover_text(hover)
                    .clicked()
                {
                    *on = !*on;
                    self.visible_dirty = true;
                }
            }
            if let Some(cid) = self.active_collection {
                if let Some(col) = self.collections.iter().find(|c| c.id == cid) {
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("Collection: {}", col.name)).weak(),
                    );
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("{} photos", self.visible.len())).weak(),
                );
            });
        });
    }

    /// Lines for the loupe's EXIF overlay (None when the overlay is off).
    fn info_lines(&self) -> Option<Vec<String>> {
        if !self.show_info {
            return None;
        }
        let cur = self.current.as_ref()?;
        let rec = self.images.iter().find(|r| r.id == cur.id)?;
        let mut lines = vec![rec.file_name.clone()];

        let camera = format!("{} {}", rec.camera_make, rec.camera_model)
            .trim()
            .to_string();
        if !camera.is_empty() {
            lines.push(camera);
        }

        if let Some(info) = &cur.shot_info {
            let mut expo = Vec::new();
            if info.shutter > 0.0 {
                expo.push(if info.shutter >= 1.0 {
                    format!("{:.1}s", info.shutter)
                } else {
                    format!("1/{}s", (1.0 / info.shutter).round() as u32)
                });
            }
            if info.aperture > 0.0 {
                expo.push(format!("f/{:.1}", info.aperture));
            }
            if info.iso > 0.0 {
                expo.push(format!("ISO {}", info.iso as u32));
            }
            if info.focal_len > 0.0 {
                expo.push(format!("{:.0}mm", info.focal_len));
            }
            if !expo.is_empty() {
                lines.push(expo.join("  ·  "));
            }
        }

        if let Some(ts) = rec.capture_ts {
            use chrono::TimeZone;
            if let Some(dt) = chrono::Local.timestamp_opt(ts, 0).earliest() {
                lines.push(dt.format("%Y-%m-%d %H:%M:%S").to_string());
            }
        }
        let mb = rec.file_size as f64 / (1024.0 * 1024.0);
        lines.push(format!(
            "{} x {}  ·  {:.1} MB  ·  {}",
            rec.width,
            rec.height,
            mb,
            rec.kind.as_str().to_uppercase()
        ));
        Some(lines)
    }

    /// Toolbar above the filmstrip in the single-image views: flag buttons
    /// (current state highlighted) and delete.
    fn show_loupe_toolbar(&mut self, ui: &mut egui::Ui) {
        let Some(cur_id) = self.current.as_ref().map(|c| c.id) else {
            return;
        };
        let flag = self
            .images
            .iter()
            .find(|r| r.id == cur_id)
            .map(|r| r.flag)
            .unwrap_or_default();

        let mut set: Option<Flag> = None;
        let mut delete = false;
        egui::Panel::bottom("loupe_toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                for (label, value, hover) in [
                    ("⚑ Pick", Flag::Pick, "Flag as pick (P)"),
                    ("⚐ Unflag", Flag::Unflagged, "Remove flag (U)"),
                    ("✖ Reject", Flag::Reject, "Flag as reject (X)"),
                ] {
                    let active = flag == value;
                    if ui
                        .selectable_label(active, label)
                        .on_hover_text(hover)
                        .clicked()
                        && !active
                    {
                        set = Some(value);
                    }
                }
                ui.separator();
                if ui
                    .button(egui::RichText::new("🗑 Delete").color(
                        egui::Color32::from_rgb(235, 130, 120),
                    ))
                    .on_hover_text("Delete this photo (Del)")
                    .clicked()
                {
                    delete = true;
                }
            });
            ui.add_space(4.0);
        });

        if let Some(flag) = set {
            self.set_flag_selection(flag);
        }
        if delete {
            let ids: Vec<i64> = if !self.grid.selection.is_empty() {
                self.grid.selection.iter().copied().collect()
            } else {
                vec![cur_id]
            };
            self.delete.request(ids);
        }
    }

    /// Bottom filmstrip for the single-image views; switches images on click.
    fn show_filmstrip(&mut self, ui: &mut egui::Ui) {
        self.ensure_visible();
        let mut clicked = None;
        egui::Panel::bottom("filmstrip")
            .exact_size(crate::ui::filmstrip::STRIP_HEIGHT)
            .show(ui, |ui| {
                clicked = crate::ui::filmstrip::show(
                    ui,
                    &mut self.grid,
                    &self.visible,
                    &self.jobs,
                    self.current.as_ref().map(|c| c.id),
                    self.strip_scroll_pending,
                );
            });
        self.strip_scroll_pending = false;
        if let Some(id) = clicked {
            self.set_current(id);
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
        self.rendered = engine.render(&cur.params, ref_temp, ref_tint, base, self.crop_ui.tool_active);
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

        let action = top_bar::show(
            ui,
            &mut self.view,
            self.grid.selection.len(),
            self.logo.as_ref(),
        );
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
        if action.open_settings {
            self.settings.open = true;
            self.settings.catalog_message = None;
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

        {
            use crate::ui::settings_dialog::SettingsAction;
            let catalog_dir = crate::catalog::data_dir();
            match self
                .settings
                .show(&ctx, self.config.library_root.as_ref(), &catalog_dir)
            {
                SettingsAction::None => {}
                SettingsAction::SetLibraryRoot(dir) => {
                    self.config.library_root = Some(dir.clone());
                    self.config.save();
                    self.import.dest_root = Some(dir);
                }
                SettingsAction::StartLibraryMove(new_root) => {
                    if let Some(old_root) = self.config.library_root.clone() {
                        let cancel =
                            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                        self.settings.move_cancel = Some(cancel.clone());
                        self.settings.move_progress =
                            Some((0, 0, String::from("starting…")));
                        // The new root becomes the import default up front so
                        // parallel imports land in the right place.
                        self.config.library_root = Some(new_root.clone());
                        self.config.save();
                        self.import.dest_root = Some(new_root.clone());
                        crate::maintenance::move_library(
                            old_root,
                            new_root,
                            self.jobs.results_tx.clone(),
                            ctx.clone(),
                            cancel,
                        );
                    }
                }
                SettingsAction::MoveCatalog(new_dir) => {
                    self.move_catalog(new_dir);
                }
            }
        }

        if let Some(mut spec) = self.export.show(&ctx, self.grid.selection.len()) {
            // Remember the export settings for next time.
            if self.config.export.as_ref() != Some(&self.export.prefs) {
                self.config.export = Some(self.export.prefs.clone());
                self.config.save();
            }
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
                self.ensure_visible();
                egui::CentralPanel::default().show(ui, |ui| {
                    if let Some(err) = &self.startup_error {
                        ui.colored_label(egui::Color32::LIGHT_RED, err);
                    }
                    match self.library_mode {
                        LibraryMode::Grid => {
                            self.show_collections_panel(ui);
                            self.ensure_visible();
                            self.show_filter_row(ui);
                            let viewing_collection = self
                                .active_collection
                                .and_then(|cid| {
                                    self.collections.iter().find(|c| c.id == cid)
                                })
                                .map(|c| c.name.clone());
                            let gctx = grid::GridContext {
                                can_paste: self.copied_params.is_some(),
                                collections: &self.collections,
                                viewing_collection: viewing_collection.as_deref(),
                                library_empty: self.images.is_empty(),
                            };
                            let resp = grid::show(
                                ui,
                                &mut self.grid,
                                &self.visible,
                                &self.jobs,
                                &gctx,
                            );
                            if let Some(flag) = resp.set_flag {
                                self.set_flag_selection(flag);
                            }
                            if let Some(cid) = resp.add_to_collection {
                                let ids: Vec<i64> =
                                    self.grid.selection.iter().copied().collect();
                                if let Err(e) = self.catalog.add_to_collection(cid, &ids)
                                {
                                    tracing::error!("add to collection: {e:#}");
                                }
                                self.reload_collections();
                            }
                            if resp.remove_from_collection {
                                if let Some(cid) = self.active_collection {
                                    let ids: Vec<i64> =
                                        self.grid.selection.iter().copied().collect();
                                    if let Err(e) =
                                        self.catalog.remove_from_collection(cid, &ids)
                                    {
                                        tracing::error!(
                                            "remove from collection: {e:#}"
                                        );
                                    }
                                    self.reload_collections();
                                }
                            }
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
                            self.show_filmstrip(ui);
                            self.show_loupe_toolbar(ui);
                            self.maybe_render();
                            let loading =
                                self.current.as_ref().is_some_and(|c| c.loading);
                            let info = self.info_lines();
                            loupe::show(
                                ui,
                                &mut self.loupe,
                                self.rendered,
                                loading,
                                info.as_deref(),
                                None,
                                false,
                            );
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
                                &mut self.crop_ui,
                            );
                            if resp.crop_toggled {
                                cur.rendered_params = None; // render mode changed
                            }
                            if resp.aspect_changed
                                && crate::ui::crop_overlay::apply_aspect(
                                    &mut cur.params.crop.rect,
                                    self.crop_ui.aspect,
                                    cur.src_dims
                                        .map(|(w, h)| w as f32 / h as f32)
                                        .unwrap_or(1.5),
                                ) {
                                    cur.dirty_since = Some(Instant::now());
                                }
                            if resp.auto_straighten {
                                if let Some(rec) =
                                    self.images.iter().find(|r| r.id == cur.id).cloned()
                                {
                                    self.crop_ui.auto_pending = true;
                                    self.crop_ui.auto_failed = false;
                                    self.jobs.start_auto_straighten(rec, ctx.clone());
                                }
                            }
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
                    self.show_filmstrip(ui);
                    self.show_loupe_toolbar(ui);
                    self.maybe_render();
                    let loading = self.current.as_ref().is_some_and(|c| c.loading);
                    let info = self.info_lines();
                    let aspect = self.crop_ui.aspect;
                    let tool_active = self.crop_ui.tool_active;
                    let mut edit_storage = None;
                    if tool_active {
                        if let Some(cur) = &mut self.current {
                            if let Some((w, h)) = cur.src_dims {
                                let safe_uv = crate::engine::crop::safe_area_in_preview(
                                    &cur.params.crop,
                                    w,
                                    h,
                                );
                                edit_storage =
                                    Some(crate::ui::crop_overlay::CropEdit {
                                        rect: &mut cur.params.crop.rect,
                                        safe_uv,
                                        safe_px_aspect: w as f32 / h as f32,
                                        aspect,
                                    });
                            }
                        }
                    }
                    let lresp = loupe::show(
                        ui,
                        &mut self.loupe,
                        self.rendered,
                        loading,
                        info.as_deref(),
                        edit_storage.as_mut(),
                        self.crop_ui.line_tool_active,
                    );
                    if let Some(delta) = lresp.straighten_delta {
                        if let Some(cur) = &mut self.current {
                            cur.params.crop.angle =
                                (cur.params.crop.angle + delta).clamp(-45.0, 45.0);
                            cur.dirty_since = Some(Instant::now());
                        }
                        // One line per activation, like Lightroom's ruler.
                        self.crop_ui.line_tool_active = false;
                    }
                    if lresp.crop_changed {
                        if let Some(cur) = &mut self.current {
                            cur.dirty_since = Some(Instant::now());
                        }
                    }
                });
            }
        }

        self.latest_zoom = ctx.zoom_factor();
    }

    fn on_exit(&mut self) {
        self.flush_edits();
        if self.config.ui_zoom != Some(self.latest_zoom) {
            self.config.ui_zoom = Some(self.latest_zoom);
            self.config.save();
        }
    }
}
