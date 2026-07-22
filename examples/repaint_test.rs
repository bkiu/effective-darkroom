//! Minimal eframe repaint check: logs a frame counter; should print
//! continuously if repaint scheduling works.

struct T(u64);

impl eframe::App for T {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        eprintln!("logic tick");
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0 += 1;
        eprintln!("frame {}", self.0);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label(format!("frame {}", self.0));
        });
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "repaint-test",
        Default::default(),
        Box::new(|_| Ok(Box::new(T(0)))),
    )
}
