//! Confirmation dialog for removing photos from the library, optionally
//! moving the files to the system trash.

#[derive(Default)]
pub struct DeleteDialog {
    pub open: bool,
    pub ids: Vec<i64>,
}

#[derive(PartialEq, Eq)]
pub enum DeleteAction {
    None,
    /// Remove catalog entries but leave the files on disk.
    RemoveFromLibrary,
    /// Remove catalog entries and move the files to the trash.
    DeleteFromDisk,
}

impl DeleteDialog {
    pub fn request(&mut self, ids: Vec<i64>) {
        if !ids.is_empty() {
            self.ids = ids;
            self.open = true;
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> DeleteAction {
        if !self.open {
            return DeleteAction::None;
        }
        let mut action = DeleteAction::None;
        let mut cancel_clicked = false;
        let mut open = self.open;
        let n = self.ids.len();

        egui::Window::new("Delete Photos")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.label(format!(
                    "Delete {n} photo{}?",
                    if n == 1 { "" } else { "s" }
                ));
                ui.label(
                    egui::RichText::new(
                        "\"Remove\" keeps the files on disk but forgets them (and their \
                         edits). \"Move to trash\" also puts the files in the system trash.",
                    )
                    .weak()
                    .small(),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Remove from library").clicked() {
                        action = DeleteAction::RemoveFromLibrary;
                    }
                    if crate::ui::theme::danger_button(ui, "Move to trash").clicked() {
                        action = DeleteAction::DeleteFromDisk;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel_clicked = true;
                    }
                });
            });

        if action != DeleteAction::None || cancel_clicked {
            open = false;
        }
        self.open = open;
        action
    }
}
