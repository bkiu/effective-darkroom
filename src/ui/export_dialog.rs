//! Export dialog, Lightroom-style: pick a base location (Pictures /
//! Documents / Nextcloud / custom) plus an optional subfolder, JPEG quality,
//! long-edge resize, filename prefix and naming scheme. A second tab uploads
//! to an Immich server instead (configured in Settings).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::engine::export::{ExportFormat, ExportOutcome, ExportSpec, Naming};
use crate::immich::{Album, AlbumTarget, ImmichExportSpec};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportBase {
    Pictures,
    Documents,
    Nextcloud,
    Custom(PathBuf),
}

impl ExportBase {
    fn label(&self) -> String {
        match self {
            ExportBase::Pictures => "Pictures".into(),
            ExportBase::Documents => "Documents".into(),
            ExportBase::Nextcloud => "Nextcloud".into(),
            ExportBase::Custom(p) => p.display().to_string(),
        }
    }

    fn path(&self) -> Option<PathBuf> {
        match self {
            ExportBase::Pictures => dirs::picture_dir(),
            ExportBase::Documents => dirs::document_dir(),
            ExportBase::Nextcloud => dirs::home_dir().map(|h| h.join("Nextcloud")),
            ExportBase::Custom(p) => Some(p.clone()),
        }
    }
}

/// Which surface the export goes to (the dialog's tabs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportTarget {
    Disk,
    Immich,
}

/// Album handling for the Immich tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlbumMode {
    NoAlbum,
    Existing,
    New,
}

/// Persisted export settings (mirrors what Lightroom remembers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportPrefs {
    pub target: ExportTarget,
    pub base: ExportBase,
    pub use_subfolder: bool,
    pub subfolder: String,
    pub format: ExportFormat,
    pub dng_bake: bool,
    pub quality: u8,
    pub resize_enabled: bool,
    pub long_edge: u32,
    pub prefix: String,
    pub naming: Naming,
    pub immich_quality: u8,
    pub immich_resize_enabled: bool,
    pub immich_long_edge: u32,
    pub immich_album_mode: AlbumMode,
    pub immich_new_album: String,
    /// Last-picked existing album as (id, name); the name is just for display.
    pub immich_existing_album: Option<(String, String)>,
}

impl Default for ExportPrefs {
    fn default() -> Self {
        Self {
            target: ExportTarget::Disk,
            base: ExportBase::Pictures,
            use_subfolder: false,
            subfolder: String::new(),
            format: ExportFormat::Jpeg,
            dng_bake: false,
            quality: 90,
            resize_enabled: true,
            long_edge: 3000,
            prefix: String::new(),
            naming: Naming::Original,
            immich_quality: 90,
            immich_resize_enabled: true,
            immich_long_edge: 3000,
            immich_album_mode: AlbumMode::NoAlbum,
            immich_new_album: String::new(),
            immich_existing_album: None,
        }
    }
}

impl ExportPrefs {
    pub fn dest(&self) -> Option<PathBuf> {
        let base = self.base.path()?;
        let sub = self.subfolder.trim();
        Some(if self.use_subfolder && !sub.is_empty() {
            base.join(sub)
        } else {
            base
        })
    }
}

/// What the user asked the dialog to start.
pub enum ExportRequest {
    Disk(ExportSpec),
    /// `url`, `api_key` and `image_ids` are filled by the caller.
    Immich(ImmichExportSpec),
}

#[derive(Default)]
pub struct ExportDialog {
    pub open: bool,
    pub prefs: ExportPrefs,
    /// (current, total, file name) while an export runs.
    pub progress: Option<(usize, usize, String)>,
    pub last_outcome: Option<ExportOutcome>,
    /// Album list from the server, or the fetch error.
    pub albums: Option<Result<Vec<Album>, String>>,
    pub albums_loading: bool,
    /// Set when the dialog wants the album list (re)fetched; the app consumes
    /// it and spawns the fetch job.
    pub fetch_albums: bool,
}

impl ExportDialog {
    /// Returns a request when the user hits Export. `selected` is the number
    /// of images that will be exported; `immich_configured` says whether a
    /// server URL and API key are set in Settings.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        selected: usize,
        immich_configured: bool,
    ) -> Option<ExportRequest> {
        if !self.open {
            return None;
        }
        let mut start = None;
        let mut open = self.open;

        egui::Window::new("Export Photos")
            .open(&mut open)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .show(ctx, |ui| {
                if let Some((cur, total, name)) = &self.progress {
                    let verb = match self.prefs.target {
                        ExportTarget::Disk => "Exporting",
                        ExportTarget::Immich => "Uploading",
                    };
                    ui.label(format!("{verb} {cur} of {total}…"));
                    ui.add(
                        egui::ProgressBar::new(*cur as f32 / (*total).max(1) as f32)
                            .text(name.clone()),
                    );
                    return;
                }

                ui.add_space(4.0);
                ui.label(format!(
                    "{selected} image{} selected",
                    if selected == 1 { "" } else { "s" }
                ));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for (value, label) in [
                        (ExportTarget::Disk, "Save to disk"),
                        (ExportTarget::Immich, "Immich"),
                    ] {
                        let active = self.prefs.target == value;
                        if crate::ui::theme::toggle_button(ui, active, label).clicked()
                            && !active
                        {
                            self.prefs.target = value;
                        }
                    }
                });
                ui.separator();
                ui.add_space(8.0);

                match self.prefs.target {
                    ExportTarget::Disk => self.disk_tab(ui),
                    ExportTarget::Immich => self.immich_tab(ui, immich_configured),
                }

                ui.add_space(12.0);
                ui.label(crate::ui::theme::overline_text("File Naming"));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Prefix");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.prefs.prefix)
                            .hint_text("optional")
                            .desired_width(160.0),
                    );
                });
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.prefs.naming, Naming::Original, "Original name");
                    ui.radio_value(&mut self.prefs.naming, Naming::DateTime, "YYYYMMDD_HHMMSS");
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);
                if let Some(out) = &self.last_outcome {
                    ui.label(format!("Exported {} images", out.exported));
                    for err in &out.errors {
                        ui.colored_label(egui::Color32::LIGHT_RED, err);
                    }
                }

                let ready = selected > 0
                    && match self.prefs.target {
                        ExportTarget::Disk => self.prefs.dest().is_some(),
                        ExportTarget::Immich => {
                            immich_configured
                                && match self.prefs.immich_album_mode {
                                    AlbumMode::NoAlbum => true,
                                    AlbumMode::New => {
                                        !self.prefs.immich_new_album.trim().is_empty()
                                    }
                                    AlbumMode::Existing => {
                                        self.prefs.immich_existing_album.is_some()
                                    }
                                }
                        }
                    };
                let label = match self.prefs.target {
                    ExportTarget::Disk => "Export",
                    ExportTarget::Immich => "Upload",
                };
                let export_clicked = if ready {
                    crate::ui::theme::primary_button(ui, label).clicked()
                } else {
                    ui.add_enabled(false, egui::Button::new(label)).clicked()
                };
                if export_clicked {
                    self.last_outcome = None;
                    start = Some(self.build_request());
                }
            });

        self.open = open;
        start
    }

    fn build_request(&self) -> ExportRequest {
        match self.prefs.target {
            ExportTarget::Disk => ExportRequest::Disk(ExportSpec {
                image_ids: Vec::new(), // filled by the caller
                dest: self.prefs.dest().expect("checked by ready"),
                format: self.prefs.format,
                dng_bake: self.prefs.dng_bake,
                quality: self.prefs.quality,
                long_edge: self.prefs.resize_enabled.then_some(self.prefs.long_edge),
                prefix: self.prefs.prefix.trim().to_string(),
                naming: self.prefs.naming,
            }),
            ExportTarget::Immich => ExportRequest::Immich(ImmichExportSpec {
                image_ids: Vec::new(), // filled by the caller
                url: String::new(),    // filled by the caller
                api_key: String::new(),
                quality: self.prefs.immich_quality,
                long_edge: self
                    .prefs
                    .immich_resize_enabled
                    .then_some(self.prefs.immich_long_edge),
                prefix: self.prefs.prefix.trim().to_string(),
                naming: self.prefs.naming,
                album: match self.prefs.immich_album_mode {
                    AlbumMode::NoAlbum => AlbumTarget::None,
                    AlbumMode::New => {
                        AlbumTarget::New(self.prefs.immich_new_album.trim().to_string())
                    }
                    AlbumMode::Existing => {
                        let (id, name) = self
                            .prefs
                            .immich_existing_album
                            .clone()
                            .expect("checked by ready");
                        AlbumTarget::Existing { id, name }
                    }
                },
            }),
        }
    }

    fn disk_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(crate::ui::theme::overline_text("Export Location"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Export to");
            let current_label = self.prefs.base.label();
            egui::ComboBox::from_id_salt("export-base")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for base in
                        [ExportBase::Pictures, ExportBase::Documents, ExportBase::Nextcloud]
                    {
                        let label = base.label();
                        ui.selectable_value(&mut self.prefs.base, base, label);
                    }
                    if ui.button("Other folder…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.prefs.base = ExportBase::Custom(dir);
                        }
                        ui.close();
                    }
                });
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.prefs.use_subfolder, "Put in subfolder");
            ui.add_enabled(
                self.prefs.use_subfolder,
                egui::TextEdit::singleline(&mut self.prefs.subfolder)
                    .hint_text("subfolder name")
                    .desired_width(200.0),
            );
        });
        match self.prefs.dest() {
            Some(dest) => {
                ui.label(
                    egui::RichText::new(format!("Saving to: {}", dest.display()))
                        .weak()
                        .small(),
                );
            }
            None => {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    "This location doesn't exist on this system",
                );
            }
        }

        ui.add_space(12.0);
        ui.label(crate::ui::theme::overline_text("File Settings"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Format");
            ui.radio_value(&mut self.prefs.format, ExportFormat::Jpeg, "JPEG");
            ui.radio_value(&mut self.prefs.format, ExportFormat::Dng, "DNG");
        });
        match self.prefs.format {
            ExportFormat::Jpeg => {
                ui.horizontal(|ui| {
                    ui.label("Quality");
                    let mut q = self.prefs.quality as u32;
                    ui.add(egui::Slider::new(&mut q, 1..=100));
                    self.prefs.quality = q as u8;
                });

                ui.add_space(12.0);
                ui.label(crate::ui::theme::overline_text("Image Sizing"));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.prefs.resize_enabled, "Resize long edge to");
                    ui.add_enabled(
                        self.prefs.resize_enabled,
                        egui::DragValue::new(&mut self.prefs.long_edge)
                            .range(256..=10000)
                            .suffix(" px"),
                    );
                });
            }
            ExportFormat::Dng => {
                ui.checkbox(
                    &mut self.prefs.dng_bake,
                    "Bake develop edits into the DNG",
                );
                let note = if self.prefs.dng_bake {
                    "Writes a linear DNG with your edits applied to the pixels \
                     (16-bit, EXIF carried over). Not re-white-balanceable like \
                     a true raw."
                } else {
                    "Lossless conversion of the original raw data. Develop \
                     edits are not baked in."
                };
                ui.label(egui::RichText::new(note).weak().small());
                ui.label(
                    egui::RichText::new("JPEG sources are skipped either way.")
                        .weak()
                        .small(),
                );
            }
        }
    }

    fn immich_tab(&mut self, ui: &mut egui::Ui, configured: bool) {
        if !configured {
            ui.label(
                "Uploading needs an Immich server URL and API key. \
                 Set both in Settings first.",
            );
            return;
        }
        // Uploads always go through the develop pipeline as JPEGs.
        if self.albums.is_none() && !self.albums_loading {
            self.fetch_albums = true;
        }

        ui.label(crate::ui::theme::overline_text("File Settings"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Quality");
            let mut q = self.prefs.immich_quality as u32;
            ui.add(egui::Slider::new(&mut q, 1..=100));
            self.prefs.immich_quality = q as u8;
        });

        ui.add_space(12.0);
        ui.label(crate::ui::theme::overline_text("Image Sizing"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.prefs.immich_resize_enabled, "Resize long edge to");
            ui.add_enabled(
                self.prefs.immich_resize_enabled,
                egui::DragValue::new(&mut self.prefs.immich_long_edge)
                    .range(256..=10000)
                    .suffix(" px"),
            );
        });

        ui.add_space(12.0);
        ui.label(crate::ui::theme::overline_text("Album"));
        ui.add_space(4.0);
        ui.radio_value(
            &mut self.prefs.immich_album_mode,
            AlbumMode::NoAlbum,
            "Don't add to an album",
        );
        ui.radio_value(
            &mut self.prefs.immich_album_mode,
            AlbumMode::Existing,
            "Add to existing album",
        );
        if self.prefs.immich_album_mode == AlbumMode::Existing {
            let mut refetch = false;
            ui.indent("immich-existing-album", |ui| {
                if self.albums_loading && self.albums.is_none() {
                    ui.label(egui::RichText::new("Loading albums…").weak());
                }
                match &self.albums {
                    None => {}
                    Some(Err(e)) => {
                        ui.colored_label(
                            egui::Color32::LIGHT_RED,
                            format!("Couldn't load albums: {e}"),
                        );
                        refetch = ui.button("Retry").clicked();
                    }
                    Some(Ok(albums)) => {
                        ui.horizontal(|ui| {
                            let current = self
                                .prefs
                                .immich_existing_album
                                .as_ref()
                                .map(|(_, name)| name.clone())
                                .unwrap_or_else(|| "Choose an album…".into());
                            egui::ComboBox::from_id_salt("immich-album")
                                .selected_text(current)
                                .show_ui(ui, |ui| {
                                    for album in albums {
                                        let is_sel = self
                                            .prefs
                                            .immich_existing_album
                                            .as_ref()
                                            .is_some_and(|(id, _)| *id == album.id);
                                        let text = format!(
                                            "{} ({})",
                                            album.name, album.asset_count
                                        );
                                        if ui.selectable_label(is_sel, text).clicked() {
                                            self.prefs.immich_existing_album = Some((
                                                album.id.clone(),
                                                album.name.clone(),
                                            ));
                                        }
                                    }
                                    if albums.is_empty() {
                                        ui.label(
                                            egui::RichText::new("No albums on the server")
                                                .weak(),
                                        );
                                    }
                                });
                            refetch = ui.button("Refresh").clicked();
                        });
                    }
                }
            });
            if refetch {
                self.albums = None;
                self.fetch_albums = true;
            }
        }
        ui.radio_value(
            &mut self.prefs.immich_album_mode,
            AlbumMode::New,
            "Create new album",
        );
        if self.prefs.immich_album_mode == AlbumMode::New {
            ui.indent("immich-new-album", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.prefs.immich_new_album)
                        .hint_text("album name")
                        .desired_width(240.0),
                );
            });
        }
    }
}
