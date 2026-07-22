//! The Basic panel: white balance, tone, presence sliders. Layout follows
//! Lightroom's develop module.

use crate::engine::params::{EditParams, TEMP_MAX, TEMP_MIN};

/// Sliders not yet wired into the render pipeline are shown disabled.
const UNWIRED_HINT: &str = "Not wired up yet — coming with the full tone pipeline";

pub struct PanelResponse {
    pub changed: bool,
    pub reset: bool,
}

pub fn show(ui: &mut egui::Ui, params: &mut EditParams, is_raw: bool) -> PanelResponse {
    let mut changed = false;
    let mut reset = false;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("Basic");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Reset").clicked() {
                        reset = true;
                    }
                });
            });
            ui.separator();

            ui.label(egui::RichText::new("White Balance").strong());
            let _ = is_raw; // As-Shot label handling arrives with RAW decode
            changed |= slider(ui, "Temp", &mut params.wb_temp, TEMP_MIN..=TEMP_MAX, true);
            changed |= slider(ui, "Tint", &mut params.wb_tint, -150.0..=150.0, false);

            ui.add_space(8.0);
            ui.label(egui::RichText::new("Tone").strong());
            changed |= slider(ui, "Exposure", &mut params.exposure, -5.0..=5.0, false);
            disabled_slider(ui, "Contrast", &mut params.contrast);
            disabled_slider(ui, "Highlights", &mut params.highlights);
            disabled_slider(ui, "Shadows", &mut params.shadows);
            disabled_slider(ui, "Whites", &mut params.whites);
            disabled_slider(ui, "Blacks", &mut params.blacks);

            ui.add_space(8.0);
            ui.label(egui::RichText::new("Presence").strong());
            disabled_slider(ui, "Texture", &mut params.texture);
            disabled_slider(ui, "Clarity", &mut params.clarity);
            disabled_slider(ui, "Dehaze", &mut params.dehaze);
            ui.add_space(4.0);
            changed |= slider(ui, "Vibrance", &mut params.vibrance, -100.0..=100.0, false);
            changed |= slider(ui, "Saturation", &mut params.saturation, -100.0..=100.0, false);
        });

    PanelResponse { changed, reset }
}

fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    logarithmic: bool,
) -> bool {
    let mut resp = None;
    ui.horizontal(|ui| {
        ui.add_sized([70.0, 18.0], egui::Label::new(label));
        let slider = egui::Slider::new(value, range)
            .logarithmic(logarithmic)
            .fixed_decimals(if logarithmic { 0 } else { 2 });
        let r = ui.add_sized([ui.available_width(), 18.0], slider);
        let double_clicked = r.double_clicked();
        resp = Some((r.changed(), double_clicked));
    });
    let (changed, _) = resp.unwrap();
    changed
}

fn disabled_slider(ui: &mut egui::Ui, label: &str, value: &mut f32) {
    ui.horizontal(|ui| {
        ui.add_sized([70.0, 18.0], egui::Label::new(egui::RichText::new(label).weak()));
        ui.add_enabled_ui(false, |ui| {
            ui.add_sized(
                [ui.available_width(), 18.0],
                egui::Slider::new(value, -100.0..=100.0).fixed_decimals(0),
            );
        })
        .response
        .on_hover_text(UNWIRED_HINT);
    });
}
