//! Export dialog: JPEG quality, long-edge resize, filename prefix and naming
//! scheme, destination folder, batch progress.

use std::path::PathBuf;

use crate::engine::export::{ExportOutcome, ExportSpec, Naming};

pub struct ExportDialog {
    pub open: bool,
    dest: Option<PathBuf>,
    quality: u8,
    resize_enabled: bool,
    long_edge: u32,
    prefix: String,
    naming: Naming,
    /// (current, total, file name) while an export runs.
    pub progress: Option<(usize, usize, String)>,
    pub last_outcome: Option<ExportOutcome>,
}

impl Default for ExportDialog {
    fn default() -> Self {
        Self {
            open: false,
            dest: None,
            quality: 90,
            resize_enabled: false,
            long_edge: 2048,
            prefix: String::new(),
            naming: Naming::Original,
            progress: None,
            last_outcome: None,
        }
    }
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
            .default_width(400.0)
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

                ui.horizontal(|ui| {
                    ui.label("Destination");
                    if ui.button("Choose…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.dest = Some(dir);
                        }
                    }
                    match &self.dest {
                        Some(d) => ui.label(d.display().to_string()),
                        None => ui.label(egui::RichText::new("not set").weak()),
                    };
                });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Quality");
                    let mut q = self.quality as u32;
                    ui.add(egui::Slider::new(&mut q, 1..=100));
                    self.quality = q as u8;
                });

                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.resize_enabled, "Resize long edge to");
                    ui.add_enabled(
                        self.resize_enabled,
                        egui::DragValue::new(&mut self.long_edge)
                            .range(256..=10000)
                            .suffix(" px"),
                    );
                });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Prefix");
                    ui.text_edit_singleline(&mut self.prefix);
                });
                ui.horizontal(|ui| {
                    ui.label("Filename");
                    ui.radio_value(&mut self.naming, Naming::Original, "Original name");
                    ui.radio_value(&mut self.naming, Naming::DateTime, "YYYYMMDD_HHMMSS");
                });

                ui.separator();
                if let Some(out) = &self.last_outcome {
                    ui.label(format!("Exported {} images", out.exported));
                    for err in &out.errors {
                        ui.colored_label(egui::Color32::LIGHT_RED, err);
                    }
                }

                let ready = self.dest.is_some() && selected > 0;
                if ui
                    .add_enabled(ready, egui::Button::new("Export"))
                    .clicked()
                {
                    self.last_outcome = None;
                    start = Some(ExportSpec {
                        image_ids: Vec::new(), // filled by the caller
                        dest: self.dest.clone().unwrap(),
                        quality: self.quality,
                        long_edge: self.resize_enabled.then_some(self.long_edge),
                        prefix: self.prefix.trim().to_string(),
                        naming: self.naming,
                    });
                }
            });

        self.open = open;
        start
    }
}
