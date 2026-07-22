//! The develop side panel: Basic (white balance, tone, presence) and the
//! Tone Curve. Layout follows Lightroom's develop module.

use crate::engine::params::{EditParams, TEMP_MAX, TEMP_MIN};
use crate::ui::tone_curve::{self, ToneCurveState};

pub struct PanelResponse {
    pub changed: bool,
    pub reset: bool,
}

pub fn show(
    ui: &mut egui::Ui,
    params: &mut EditParams,
    as_shot: Option<(f32, f32)>,
    curve_state: &mut ToneCurveState,
    histogram: Option<&[[u32; 256]; 3]>,
) -> PanelResponse {
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

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("White Balance").strong());
                if let Some((temp, tint)) = as_shot {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("As Shot").clicked() {
                            params.wb_temp = temp;
                            params.wb_tint = tint;
                            changed = true;
                        }
                    });
                }
            });
            changed |= slider(ui, "Temp", &mut params.wb_temp, TEMP_MIN..=TEMP_MAX, true);
            changed |= slider(ui, "Tint", &mut params.wb_tint, -150.0..=150.0, false);

            ui.add_space(8.0);
            ui.label(egui::RichText::new("Tone").strong());
            changed |= slider(ui, "Exposure", &mut params.exposure, -5.0..=5.0, false);
            changed |= slider(ui, "Contrast", &mut params.contrast, -100.0..=100.0, false);
            changed |= slider(ui, "Highlights", &mut params.highlights, -100.0..=100.0, false);
            changed |= slider(ui, "Shadows", &mut params.shadows, -100.0..=100.0, false);
            changed |= slider(ui, "Whites", &mut params.whites, -100.0..=100.0, false);
            changed |= slider(ui, "Blacks", &mut params.blacks, -100.0..=100.0, false);

            ui.add_space(8.0);
            ui.label(egui::RichText::new("Presence").strong());
            changed |= slider(ui, "Texture", &mut params.texture, -100.0..=100.0, false);
            changed |= slider(ui, "Clarity", &mut params.clarity, -100.0..=100.0, false);
            changed |= slider(ui, "Dehaze", &mut params.dehaze, -100.0..=100.0, false);
            ui.add_space(4.0);
            changed |= slider(ui, "Vibrance", &mut params.vibrance, -100.0..=100.0, false);
            changed |= slider(ui, "Saturation", &mut params.saturation, -100.0..=100.0, false);

            ui.add_space(10.0);
            ui.separator();
            changed |= tone_curve::show(ui, curve_state, &mut params.tone_curve, histogram);
            ui.add_space(8.0);
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
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_sized([70.0, 18.0], egui::Label::new(label));
        let slider = egui::Slider::new(value, range)
            .logarithmic(logarithmic)
            .fixed_decimals(if logarithmic { 0 } else { 2 });
        let r = ui.add_sized([ui.available_width(), 18.0], slider);
        changed = r.changed();
    });
    changed
}
