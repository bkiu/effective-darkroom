//! Confirmation dialog for removing photos from the library, optionally
//! moving the files to the system trash.

#[derive(Default)]
pub struct DeleteDialog {
    pub open: bool,
    pub ids: Vec<i64>,
}

#[derive(Debug, PartialEq, Eq)]
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

        // Keyboard: Enter takes the safe option, T the destructive one, Esc
        // backs out. Read before the window is built so a key and a click on
        // the same frame cannot both land.
        let (enter, escape, trash_key) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::T),
            )
        });
        if enter {
            action = DeleteAction::RemoveFromLibrary;
        } else if trash_key {
            action = DeleteAction::DeleteFromDisk;
        } else if escape {
            cancel_clicked = true;
        }

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
                    if ui.button("Remove from library  (Enter)").clicked() {
                        action = DeleteAction::RemoveFromLibrary;
                    }
                    if crate::ui::theme::danger_button(ui, "Move to trash  (T)").clicked() {
                        action = DeleteAction::DeleteFromDisk;
                    }
                    if ui.button("Cancel  (Esc)").clicked() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    /// Show the dialog once in a headless context with the given keys held.
    fn frame(dialog: &mut DeleteDialog, events: Vec<egui::Event>) -> DeleteAction {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut action = DeleteAction::None;
        let _ = ctx.run_ui(input, |ui| {
            action = dialog.show(ui.ctx());
        });
        action
    }

    fn open_dialog() -> DeleteDialog {
        let mut d = DeleteDialog::default();
        d.request(vec![1, 2, 3]);
        assert!(d.open);
        d
    }

    #[test]
    fn enter_removes_from_the_library() {
        let mut d = open_dialog();
        assert_eq!(
            frame(&mut d, vec![press(egui::Key::Enter)]),
            DeleteAction::RemoveFromLibrary
        );
        assert!(!d.open, "dialog stayed up after acting");
    }

    #[test]
    fn t_moves_to_the_trash() {
        let mut d = open_dialog();
        assert_eq!(
            frame(&mut d, vec![press(egui::Key::T)]),
            DeleteAction::DeleteFromDisk
        );
        assert!(!d.open);
    }

    #[test]
    fn escape_cancels_without_touching_anything() {
        let mut d = open_dialog();
        assert_eq!(
            frame(&mut d, vec![press(egui::Key::Escape)]),
            DeleteAction::None
        );
        assert!(!d.open, "escape left the dialog up");
    }

    #[test]
    fn other_keys_leave_the_dialog_alone() {
        let mut d = open_dialog();
        assert_eq!(frame(&mut d, vec![press(egui::Key::X)]), DeleteAction::None);
        assert!(d.open, "an unrelated key dismissed the dialog");
    }

    #[test]
    fn a_closed_dialog_ignores_the_keyboard() {
        let mut d = DeleteDialog::default();
        assert_eq!(
            frame(&mut d, vec![press(egui::Key::Enter)]),
            DeleteAction::None
        );
        assert!(!d.open);
    }
}
