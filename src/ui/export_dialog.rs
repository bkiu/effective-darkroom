//! Export dialog, Lightroom-style: pick a base location (Pictures /
//! Documents / Nextcloud / custom) plus an optional subfolder, JPEG quality,
//! long-edge resize, filename prefix and naming scheme.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::engine::export::{ExportOutcome, ExportSpec, Naming};

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

/// Persisted export settings (mirrors what Lightroom remembers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportPrefs {
    pub base: ExportBase,
    pub subfolder: String,
    pub quality: u8,
    pub resize_enabled: bool,
    pub long_edge: u32,
    pub prefix: String,
    pub naming: Naming,
}

impl Default for ExportPrefs {
    fn default() -> Self {
        Self {
            base: ExportBase::Pictures,
            subfolder: String::new(),
            quality: 90,
            resize_enabled: true,
            long_edge: 3000,
            prefix: String::new(),
            naming: Naming::Original,
        }
    }
}

impl ExportPrefs {
    pub fn dest(&self) -> Option<PathBuf> {
        let base = self.base.path()?;
        let sub = self.subfolder.trim();
        Some(if sub.is_empty() {
            base
        } else {
            base.join(sub)
        })
    }
}

#[derive(Default)]
pub struct ExportDialog {
    pub open: bool,
    pub prefs: ExportPrefs,
    /// (current, total, file name) while an export runs.
    pub progress: Option<(usize, usize, String)>,
    pub last_outcome: Option<ExportOutcome>,
}

impl ExportDialog {
    /// Returns a spec when the user hits Export. `selected` is the number of
    /// images that will be exported.
    pub fn show(&mut self, ctx: &egui::Context, selected: usize) -> Option<ExportSpec> {
        if !self.open {
            return None;
        }
        let mut start = None;
        let mut open = self.open;

        egui::Window::new("Export Photos")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .show(ctx, |ui| {
                if let Some((cur, total, name)) = &self.progress {
                    ui.label(format!("Exporting {cur} of {total}…"));
                    ui.add(
                        egui::ProgressBar::new(*cur as f32 / (*total).max(1) as f32)
                            .text(name.clone()),
                    );
                    return;
                }

                ui.label(format!(
                    "{selected} image{} selected",
                    if selected == 1 { "" } else { "s" }
                ));
                ui.separator();

                ui.label(egui::RichText::new("Export Location").strong());
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
                ui.horizontal(|ui| {
                    ui.label("Subfolder");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.prefs.subfolder)
                            .hint_text("optional")
                            .desired_width(220.0),
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

                ui.add_space(6.0);
                ui.label(egui::RichText::new("File Settings").strong());
                ui.horizontal(|ui| {
                    ui.label("Quality");
                    let mut q = self.prefs.quality as u32;
                    ui.add(egui::Slider::new(&mut q, 1..=100));
                    self.prefs.quality = q as u8;
                });

                ui.add_space(6.0);
                ui.label(egui::RichText::new("Image Sizing").strong());
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.prefs.resize_enabled, "Resize long edge to");
                    ui.add_enabled(
                        self.prefs.resize_enabled,
                        egui::DragValue::new(&mut self.prefs.long_edge)
                            .range(256..=10000)
                            .suffix(" px"),
                    );
                });

                ui.add_space(6.0);
                ui.label(egui::RichText::new("File Naming").strong());
                ui.horizontal(|ui| {
                    ui.label("Prefix");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.prefs.prefix)
                            .hint_text("optional")
                            .desired_width(160.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.prefs.naming, Naming::Original, "Original name");
                    ui.radio_value(&mut self.prefs.naming, Naming::DateTime, "YYYYMMDD_HHMMSS");
                });

                ui.separator();
                if let Some(out) = &self.last_outcome {
                    ui.label(format!("Exported {} images", out.exported));
                    for err in &out.errors {
                        ui.colored_label(egui::Color32::LIGHT_RED, err);
                    }
                }

                let dest = self.prefs.dest();
                let ready = dest.is_some() && selected > 0;
                if ui.add_enabled(ready, egui::Button::new("Export")).clicked() {
                    self.last_outcome = None;
                    start = Some(ExportSpec {
                        image_ids: Vec::new(), // filled by the caller
                        dest: dest.unwrap(),
                        quality: self.prefs.quality,
                        long_edge: self
                            .prefs
                            .resize_enabled
                            .then_some(self.prefs.long_edge),
                        prefix: self.prefs.prefix.trim().to_string(),
                        naming: self.prefs.naming,
                    });
                }
            });

        self.open = open;
        start
    }
}
