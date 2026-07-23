use crate::app::View;
use crate::ui::{theme, widgets};

/// Top bar: logo + title on the left, actions and the Library/Develop
/// toggle on the right. Window chrome is the system's (KDE).
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
    let frame = egui::Frame::new()
        .fill(theme::bg_canvas())
        .inner_margin(egui::Margin::symmetric(12, 10));
    egui::Panel::top("top_bar").frame(frame).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if let Some(logo) = logo {
                ui.add(
                    egui::Image::new(logo).fit_to_exact_size(egui::Vec2::splat(24.0)),
                );
            }
            ui.label(
                egui::RichText::new("Effective Darkroom")
                    .font(theme::heading_font())
                    .color(theme::text_primary()),
            );
            ui.add_space(10.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            if ui.button("Import…").clicked() {
                action.open_import = true;
            }
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
                if ui.button("⚙").on_hover_text("Settings").clicked() {
                    action.open_settings = true;
                }
                ui.add_space(10.0);
                widgets::segmented(
                    ui,
                    view,
                    &[(View::Library, "Library"), (View::Develop, "Develop")],
                );
            });
        });
    });
    action
}
