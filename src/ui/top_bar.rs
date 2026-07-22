use crate::app::View;

/// Top bar: app title on the left, module tabs (Library / Develop) on the
/// right, like Lightroom's module picker.
pub struct TopBarAction {
    pub open_import: bool,
}

pub fn show(ui: &mut egui::Ui, view: &mut View) -> TopBarAction {
    let mut action = TopBarAction { open_import: false };
    egui::Panel::top("top_bar").show(ui, |ui| {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Effective Darkroom")
                    .strong()
                    .size(16.0),
            );
            ui.separator();
            if ui.button("Import…").clicked() {
                action.open_import = true;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                for (label, v) in [("Develop", View::Develop), ("Library", View::Library)] {
                    let selected = *view == v;
                    let text = if selected {
                        egui::RichText::new(label).strong()
                    } else {
                        egui::RichText::new(label).weak()
                    };
                    if ui.selectable_label(selected, text).clicked() {
                        *view = v;
                    }
                }
            });
        });
        ui.add_space(2.0);
    });
    action
}
