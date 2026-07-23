use crate::app::View;

/// Top bar: logo + app title on the left, module tabs (Library / Develop)
/// on the right, like Lightroom's module picker.
pub struct TopBarAction {
    pub open_import: bool,
    pub open_export: bool,
    pub open_settings: bool,
}

pub fn show(
    ui: &mut egui::Ui,
    view: &mut View,
    selection_count: usize,
    logo: Option<&egui::TextureHandle>,
) -> TopBarAction {
    let mut action = TopBarAction {
        open_import: false,
        open_export: false,
        open_settings: false,
    };
    egui::Panel::top("top_bar").show(ui, |ui| {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            if let Some(logo) = logo {
                ui.add(
                    egui::Image::new(logo)
                        .fit_to_exact_size(egui::Vec2::splat(26.0)),
                );
                ui.add_space(6.0);
            }
            ui.label(
                egui::RichText::new("Effective Darkroom")
                    .strong()
                    .size(18.0),
            );
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            if ui.button("Import…").clicked() {
                action.open_import = true;
            }
            ui.add_space(2.0);
            let export_label = if selection_count > 1 {
                format!("Export ({selection_count})…")
            } else {
                "Export…".to_string()
            };
            if ui
                .add_enabled(selection_count > 0, egui::Button::new(export_label))
                .clicked()
            {
                action.open_export = true;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                if ui
                    .button("⚙")
                    .on_hover_text("Settings")
                    .clicked()
                {
                    action.open_settings = true;
                }
                ui.add_space(6.0);
                for (label, v) in [("Develop", View::Develop), ("Library", View::Library)] {
                    let selected = *view == v;
                    let text = if selected {
                        egui::RichText::new(label).strong().size(15.0)
                    } else {
                        egui::RichText::new(label).weak().size(15.0)
                    };
                    if ui.selectable_label(selected, text).clicked() {
                        *view = v;
                    }
                    ui.add_space(4.0);
                }
            });
        });
        ui.add_space(8.0);
    });
    action
}
