use effective_darkroom::app;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "effective_darkroom=info".into()),
        )
        .init();

    tracing::info!("LibRaw {}", libraw_ffi::version());

    // wgpu's automatic backend selection stalls the event loop after the
    // first frame on Wayland (KDE); pinning Vulkan (GL fallback) avoids it.
    // Power preference defaults to LowPower because the NVIDIA Vulkan driver
    // on this machine fails large texture allocations with spurious OOM (see
    // examples/wgpu_alloc_test.rs); the AMD iGPU handles the pipeline fine.
    // Both are overridable via WGPU_BACKEND / WGPU_POWER_PREF.
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends =
            eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
        if std::env::var("WGPU_POWER_PREF").is_err() {
            setup.power_preference = eframe::wgpu::PowerPreference::LowPower;
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("Effective Darkroom"),
        wgpu_options,
        ..Default::default()
    };
    eframe::run_native(
        "Effective Darkroom",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
