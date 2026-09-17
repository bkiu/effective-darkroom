//! The develop side panel: Basic (white balance, tone, presence) and the
//! Tone Curve. Layout follows Lightroom's develop module.

use crate::engine::params::{EditParams, TEMP_DEFAULT, TEMP_MAX, TEMP_MIN};
use crate::ui::crop_overlay::AspectChoice;
use crate::ui::theme;
use crate::ui::tone_curve::{self, ToneCurveState};
use crate::ui::widgets::Adjustment;

pub struct PanelResponse {
    pub changed: bool,
    pub reset: bool,
    pub copy: bool,
    pub paste: bool,
    pub crop_toggled: bool,
    pub auto_straighten: bool,
    pub aspect_changed: bool,
}

pub struct CropUiState {
    pub tool_active: bool,
    /// Draw-a-line straighten tool active in the loupe.
    pub line_tool_active: bool,
    pub aspect: AspectChoice,
    pub auto_pending: bool,
    pub auto_failed: bool,
}

pub fn show(
    ui: &mut egui::Ui,
    params: &mut EditParams,
    as_shot: Option<(f32, f32)>,
    curve_state: &mut ToneCurveState,
    histogram: Option<&[[u32; 256]; 3]>,
    can_paste: bool,
    crop_ui: &mut CropUiState,
) -> PanelResponse {
    let mut changed = false;
    let mut reset = false;
    let mut copy = false;
    let mut paste = false;
    let mut crop_toggled = false;
    let mut auto_straighten = false;
    let mut aspect_changed = false;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(theme::section_text("Basic"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Reset").clicked() {
                        reset = true;
                    }
                    if ui
                        .add_enabled(can_paste, egui::Button::new("Paste"))
                        .on_hover_text("Apply copied develop settings")
                        .clicked()
                    {
                        paste = true;
                    }
                    if ui
                        .button("Copy")
                        .on_hover_text("Copy develop settings")
                        .clicked()
                    {
                        copy = true;
                    }
                });
            });
            ui.separator();

            theme::section(ui, "Crop", true, |ui| {
                ui.horizontal(|ui| {
                    if theme::toggle_button(ui, crop_ui.tool_active, "✂ Crop")
                        .on_hover_text("Toggle the crop overlay (rule-of-thirds grid)")
                        .clicked()
                    {
                        crop_ui.tool_active = !crop_ui.tool_active;
                        crop_toggled = true;
                    }
                    if ui
                        .button("Reset")
                        .on_hover_text("Clear crop and rotation")
                        .clicked()
                    {
                        params.crop = Default::default();
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_sized([70.0, 18.0], egui::Label::new("Aspect"));
                    theme::combo_box("crop-aspect")
                        .selected_text(crop_ui.aspect.label())
                        .show_ui(ui, |ui| {
                            for a in AspectChoice::ALL {
                                if ui
                                    .selectable_value(&mut crop_ui.aspect, a, a.label())
                                    .clicked()
                                {
                                    aspect_changed = true;
                                }
                            }
                        });
                });
            });

            theme::section(ui, "Straighten", true, |ui| {
                ui.horizontal(|ui| {
                    if theme::toggle_button(ui, crop_ui.line_tool_active, "📐 Draw line")
                        .on_hover_text(
                            "Drag a line along something that should be level or \
                             plumb; the image rotates to match",
                        )
                        .clicked()
                    {
                        crop_ui.line_tool_active = !crop_ui.line_tool_active;
                    }
                    if crop_ui.auto_pending {
                        ui.spinner();
                        ui.label(egui::RichText::new("analyzing…").weak());
                    } else if ui
                        .button("Auto")
                        .on_hover_text("Detect dominant lines and straighten")
                        .clicked()
                    {
                        auto_straighten = true;
                    }
                });
                if crop_ui.auto_failed {
                    ui.label(
                        egui::RichText::new("No clear lines found to straighten by")
                            .weak()
                            .small(),
                    );
                }
                changed |= slider(ui, "Angle", &mut params.crop.angle, -45.0..=45.0, false);
            });

            theme::section(ui, "White Balance", true, |ui| {
                if let Some((temp, tint)) = as_shot {
                    ui.horizontal(|ui| {
                        if ui.small_button("As Shot").clicked() {
                            params.wb_temp = temp;
                            params.wb_tint = tint;
                            changed = true;
                        }
                    });
                }
                changed |= slider(ui, "Temp", &mut params.wb_temp, TEMP_MIN..=TEMP_MAX, true);
                changed |= slider(ui, "Tint", &mut params.wb_tint, -150.0..=150.0, false);
            });

            theme::section(ui, "Tone", true, |ui| {
                changed |= slider(ui, "Exposure", &mut params.exposure, -5.0..=5.0, false);
                changed |= slider(ui, "Contrast", &mut params.contrast, -100.0..=100.0, false);
                changed |=
                    slider(ui, "Highlights", &mut params.highlights, -100.0..=100.0, false);
                changed |= slider(ui, "Shadows", &mut params.shadows, -100.0..=100.0, false);
                changed |= slider(ui, "Whites", &mut params.whites, -100.0..=100.0, false);
                changed |= slider(ui, "Blacks", &mut params.blacks, -100.0..=100.0, false);
            });

            theme::section(ui, "Presence", true, |ui| {
                changed |= slider(ui, "Texture", &mut params.texture, -100.0..=100.0, false);
                changed |= slider(ui, "Clarity", &mut params.clarity, -100.0..=100.0, false);
                changed |= slider(ui, "Dehaze", &mut params.dehaze, -100.0..=100.0, false);
                ui.add_space(4.0);
                changed |= slider(ui, "Vibrance", &mut params.vibrance, -100.0..=100.0, false);
                changed |=
                    slider(ui, "Saturation", &mut params.saturation, -100.0..=100.0, false);
            });

            theme::section(ui, "Detail", false, |ui| {
                changed |= slider(ui, "Color NR", &mut params.color_nr, 0.0..=100.0, false);
            });

            theme::section(ui, "Tone Curve", true, |ui| {
                changed |= tone_curve::show(ui, curve_state, &mut params.tone_curve, histogram);
            });
            ui.add_space(8.0);
        });

    PanelResponse {
        changed,
        reset,
        copy,
        paste,
        crop_toggled,
        auto_straighten,
        aspect_changed,
    }
}

/// Bipolar +/- adjustment.
fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    logarithmic: bool,
) -> bool {
    let (min, max) = (*range.start(), *range.end());
    let bipolar = min < 0.0 && max > 0.0;
    let default = if logarithmic { TEMP_DEFAULT } else { 0.0 };
    Adjustment {
        label,
        value,
        min,
        max,
        default,
        bipolar,
        decimals: if logarithmic { 0 } else { 2 },
        logarithmic,
        step: if logarithmic {
            50.0
        } else if max <= 10.0 {
            0.1
        } else {
            1.0
        },
        suffix: "",
        width: None,
    }
    .show(ui)
}
