use effective_darkroom::app;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "effective_darkroom=info".into()),
        )
        .init();

    tracing::info!("LibRaw {}", libraw_ffi::version());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("Effective Darkroom"),
        ..Default::default()
    };
    eframe::run_native(
        "Effective Darkroom",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
