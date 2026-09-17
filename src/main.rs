use effective_darkroom::app;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "effective_darkroom=info".into()),
        )
        .init();

    tracing::info!("LibRaw {}", libraw_ffi::version());
    prefer_x11();

    // Vulkan with a GL fallback — in practice Vulkan, since GL reports no
    // surface-compatible adapter on this compositor.
    //
    // Adapter choice is ours rather than a power preference. The discrete GPU
    // is the one KWin composites on, and it is the only one whose dmabufs the
    // compositor will import here: presenting on the AMD iGPU dies with
    // "importing the supplied dmabufs failed" and takes the Wayland
    // connection with it. (This used to be the other way round, when the
    // machine had a 3090 Ti whose driver failed the engine's large texture
    // allocations. The current 3060 passes examples/wgpu_alloc_test, so the
    // iGPU workaround is no longer needed.) WGPU_POWER_PREF=low picks the
    // integrated GPU instead.
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends =
            eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
        let prefer_discrete = !matches!(
            std::env::var("WGPU_POWER_PREF").as_deref(),
            Ok("low") | Ok("low_power") | Ok("LowPower")
        );
        setup.native_adapter_selector = Some(std::sync::Arc::new(move |adapters, surface| {
            select_adapter(adapters, surface, prefer_discrete)
        }));
    }

    let icon = image::load_from_memory(include_bytes!("../assets/icon-small.png"))
        .map(|img| {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            egui::IconData {
                rgba: rgba.into_raw(),
                width,
                height,
            }
        })
        .ok();

    // ED_TITLE: dev hook so test instances are distinguishable from the
    // user's running copy (window activation scripts match by caption).
    let title =
        std::env::var("ED_TITLE").unwrap_or_else(|_| "Effective Darkroom".to_string());
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([900.0, 600.0])
        .with_title(title)
        .with_app_id("effective-darkroom");
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        wgpu_options,
        ..Default::default()
    };
    eframe::run_native(
        "Effective Darkroom",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}

/// Pick the adapter to render and present with.
///
/// Surface compatibility outranks everything: an adapter the compositor will
/// not accept buffers from cannot present at all, and eframe panics rather
/// than falling back. Among the usable ones, prefer the requested class of
/// GPU, then Vulkan over the GL fallback.
fn select_adapter(
    adapters: &[eframe::wgpu::Adapter],
    surface: Option<&eframe::wgpu::Surface<'_>>,
    prefer_discrete: bool,
) -> Result<eframe::wgpu::Adapter, String> {
    use eframe::wgpu::{Backend, DeviceType};

    let wanted = if prefer_discrete {
        DeviceType::DiscreteGpu
    } else {
        DeviceType::IntegratedGpu
    };
    let rank = |adapter: &eframe::wgpu::Adapter| {
        let info = adapter.get_info();
        (
            surface.is_none_or(|s| adapter.is_surface_supported(s)),
            info.device_type == wanted,
            info.backend == Backend::Vulkan,
        )
    };

    let chosen = adapters
        .iter()
        .max_by_key(|adapter| rank(adapter))
        .ok_or_else(|| "no wgpu adapter available".to_string())?;
    let (presentable, preferred, _) = rank(chosen);
    if !presentable {
        // Nothing here can present; let eframe fail with its own message
        // rather than pretending this one will work.
        tracing::error!("no wgpu adapter supports this window surface");
    } else if !preferred {
        let info = chosen.get_info();
        tracing::warn!(
            "preferred {wanted:?} adapter cannot present; using {} ({:?})",
            info.name,
            info.device_type
        );
    }
    Ok(chosen.clone())
}

/// Route the window through XWayland instead of winit's native Wayland
/// backend.
///
/// On this workstation's KDE Wayland session winit 0.30 paints exactly one
/// frame and then spins the event loop at 100% CPU inside
/// `wayland_backend`'s `prepare_read`, never redrawing again: the compositor
/// stops delivering frame callbacks and winit never emits another
/// `RedrawRequested`. It reproduces with a bare eframe app
/// (`examples/repaint_mode_test`) on every adapter including llvmpipe, so it
/// is neither our rendering code nor the GPU. The same app on XWayland runs
/// normally.
///
/// winit 0.29 dropped `WINIT_UNIX_BACKEND`; the backend is now chosen from
/// the environment alone, with Wayland winning whenever `WAYLAND_DISPLAY` is
/// set. So hide those variables — but only when there is an X display to fall
/// back to, so a session without XWayland still gets Wayland. `ED_WAYLAND=1`
/// opts back in; retest after a winit or KWin update.
fn prefer_x11() {
    if std::env::var_os("ED_WAYLAND").is_some() {
        return;
    }
    let has_x11 = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
    let on_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var_os("WAYLAND_SOCKET").is_some();
    if !(has_x11 && on_wayland) {
        return;
    }
    // Safe here: this runs before eframe builds the event loop and before any
    // thread of ours exists.
    std::env::remove_var("WAYLAND_DISPLAY");
    std::env::remove_var("WAYLAND_SOCKET");
    tracing::info!("presenting through XWayland (set ED_WAYLAND=1 to force native Wayland)");
}
