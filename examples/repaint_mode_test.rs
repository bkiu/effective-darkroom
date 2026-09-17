//! Does the winit event loop keep running here?
//!
//! Prints a frame counter. A healthy session prints continuously; the KDE
//! Wayland stall this machine hit prints `frame 1` and then spins the main
//! thread at 100% CPU forever (see `prefer_x11` in src/main.rs). Knobs, all
//! optional, to narrow down a stall:
//!
//!   ED_REPAINT=after     schedule with `request_repaint_after` (default: now)
//!   ED_PRESENT=fifo|immediate|mailbox|autonovsync
//!   ED_LATENCY=<n>       desired_maximum_frame_latency
//!   ED_ADAPTER=<substr>  force an adapter by name, e.g. llvmpipe
//!
//! Stalling on `ED_ADAPTER=llvmpipe` means the GPU is not the culprit.

struct T(u64, bool);

impl eframe::App for T {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0 += 1;
        eprintln!("frame {}", self.0);
        if self.1 {
            ui.ctx().request_repaint();
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label(format!("frame {}", self.0));
        });
    }
}

fn main() -> eframe::Result {
    let now = std::env::var("ED_REPAINT").as_deref() != Ok("after");
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
        if let Ok(want) = std::env::var("ED_ADAPTER") {
            setup.native_adapter_selector = Some(std::sync::Arc::new(move |adapters, _surface| {
                for adapter in adapters {
                    eprintln!("candidate: {}", adapter.get_info().name);
                }
                adapters
                    .iter()
                    .find(|a| {
                        a.get_info().name.to_lowercase().contains(&want.to_lowercase())
                    })
                    .cloned()
                    .ok_or_else(|| format!("no adapter matching {want}"))
            }));
        }
    }
    wgpu_options.surface.present_mode = match std::env::var("ED_PRESENT").as_deref() {
        Ok("immediate") => eframe::wgpu::PresentMode::Immediate,
        Ok("mailbox") => eframe::wgpu::PresentMode::Mailbox,
        Ok("fifo") => eframe::wgpu::PresentMode::Fifo,
        Ok("autonovsync") => eframe::wgpu::PresentMode::AutoNoVsync,
        _ => eframe::wgpu::PresentMode::AutoVsync,
    };
    if let Ok(v) = std::env::var("ED_LATENCY") {
        wgpu_options.surface.desired_maximum_frame_latency = v.parse().ok();
    }
    eprintln!(
        "present={:?} latency={:?}",
        wgpu_options.surface.present_mode, wgpu_options.surface.desired_maximum_frame_latency
    );
    eframe::run_native(
        "repaint-mode-test",
        eframe::NativeOptions { wgpu_options, ..Default::default() },
        Box::new(move |_| Ok(Box::new(T(0, now)))),
    )
}
