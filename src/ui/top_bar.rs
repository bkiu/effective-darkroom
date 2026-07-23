use crate::app::View;
use crate::ui::{theme, widgets};

/// Top bar doubles as the window titlebar (custom chrome): logo + title on
/// the left, actions, segmented module switcher and window controls on the
/// right. Empty space drags the window; double-click toggles maximize.
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
        let bar_rect = ui.max_rect();
        ui.horizontal(|ui| {
            if let Some(logo) = logo {
                ui.add(
                    egui::Image::new(logo).fit_to_exact_size(egui::Vec2::splat(24.0)),
                );
                ui.add_space(4.0);
            }
            ui.label(
                egui::RichText::new("Effective Darkroom")
                    .font(theme::heading_font())
                    .color(theme::text_primary()),
            );
            ui.add_space(14.0);
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
                // Window controls.
                let ctx = ui.ctx().clone();
                if ui
                    .button(egui::RichText::new("✖").color(theme::text_secondary()))
                    .on_hover_text("Close")
                    .clicked()
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui
                    .button(egui::RichText::new("☐").color(theme::text_secondary()))
                    .on_hover_text("Maximize")
                    .clicked()
                {
                    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
                if ui
                    .button(egui::RichText::new("–").color(theme::text_secondary()))
                    .on_hover_text("Minimize")
                    .clicked()
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                ui.add_space(10.0);
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

        // Drag anywhere on the empty bar; double-click toggles maximize.
        let drag = ui.interact(
            bar_rect,
            ui.id().with("titlebar-drag"),
            egui::Sense::click_and_drag(),
        );
        if drag.double_clicked() {
            let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        } else if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
    });
    action
}
