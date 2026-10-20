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
    // Adapter choice is ours rather than a power preference: see
    // `select_adapter`. In short, a discrete GPU is preferred (on the 3060
    // machine it is the only one whose dmabufs the compositor will import),
    // but not one another process has filled — the work machine's two
    // 3090 Tis are each ~95% held by vLLM, and a develop session there dies
    // of a real out-of-memory. WGPU_POWER_PREF=low picks the integrated GPU
    // regardless.
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

/// A discrete GPU needs at least this much free VRAM to be worth choosing.
/// One 26 MP develop session holds ~0.5 GiB (f16 source, 8-bit output,
/// half-res blur chain) and an export running alongside it on the same
/// device about as much again, plus egui and the swapchain.
const MIN_FREE_VRAM: u64 = 2 << 30;

/// Pick the adapter to render and present with.
///
/// In order of importance:
/// 1. It can present to this window: an adapter the compositor will not
///    accept buffers from cannot present at all, and eframe panics rather
///    than falling back.
/// 2. It has room. A discrete GPU with less than `MIN_FREE_VRAM` free (some
///    other process owns the card) loses to anything that does; integrated
///    GPUs share system RAM and always count as roomy.
/// 3. The requested class of GPU (discrete unless WGPU_POWER_PREF=low),
///    then the other hardware class, and software rasterizers last.
/// 4. More free VRAM — between two equal cards, the emptier one.
/// 5. Vulkan over the GL fallback.
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
    let ranked: Vec<_> = adapters
        .iter()
        .map(|adapter| {
            let info = adapter.get_info();
            let free = free_vram(adapter);
            let roomy = info.device_type != DeviceType::DiscreteGpu
                || free.is_none_or(|f| f >= MIN_FREE_VRAM);
            tracing::debug!(
                "adapter {} ({:?}, {:?}): {} free",
                info.name,
                info.device_type,
                info.backend,
                free.map_or("unknown".into(), mib),
            );
            let class = match info.device_type {
                t if t == wanted => 2,
                DeviceType::DiscreteGpu | DeviceType::IntegratedGpu => 1,
                _ => 0,
            };
            let rank = (
                surface.is_none_or(|s| adapter.is_surface_supported(s)),
                roomy,
                class,
                free.unwrap_or(0),
                info.backend == Backend::Vulkan,
            );
            (adapter, info, free, rank)
        })
        .collect();

    let (chosen, info, free, (presentable, roomy, class, ..)) = ranked
        .iter()
        .max_by_key(|(.., rank)| *rank)
        .ok_or_else(|| "no wgpu adapter available".to_string())?;
    if !presentable {
        // Nothing here can present; let eframe fail with its own message
        // rather than pretending this one will work.
        tracing::error!("no wgpu adapter supports this window surface");
    } else if !roomy {
        tracing::warn!(
            "every usable GPU is nearly full; using {} with {} free",
            info.name,
            free.map_or("unknown".into(), mib),
        );
    } else if *class < 2 {
        // Say why the preferred class lost: full, or unable to present.
        let full = ranked.iter().any(|(.., (p, r, c, ..))| *p && !*r && *c == 2);
        tracing::warn!(
            "preferred {wanted:?} adapter {}; using {} ({:?})",
            if full { "is nearly out of memory" } else { "cannot present" },
            info.name,
            info.device_type
        );
    }
    Ok((*chosen).clone())
}

fn mib(bytes: u64) -> String {
    format!("{} MiB", bytes >> 20)
}

/// Device-local memory still available to this process, summed over the
/// adapter's VRAM heaps, from VK_EXT_memory_budget. The budget accounts for
/// what other processes hold, which is the point: total VRAM says nothing
/// when vLLM has 22 of 24 GiB. `None` when the adapter is not Vulkan, the
/// driver lacks the extension, or the instance predates Vulkan 1.1.
fn free_vram(adapter: &eframe::wgpu::Adapter) -> Option<u64> {
    use ash::vk;
    use eframe::wgpu::hal::api::Vulkan;

    // SAFETY: only read-only physical-device queries are made, and the
    // guard outlives every handle taken from it.
    let hal = unsafe { adapter.as_hal::<Vulkan>() }?;
    let shared = hal.shared_instance();
    if shared.instance_api_version() < vk::API_VERSION_1_1 {
        return None;
    }
    let instance = shared.raw_instance();
    let phys = hal.raw_physical_device();

    // SAFETY: valid physical device of a live instance.
    let exts = unsafe { instance.enumerate_device_extension_properties(phys) }.ok()?;
    let has_budget = exts.iter().any(|e| {
        e.extension_name_as_c_str() == Ok(ash::ext::memory_budget::NAME)
    });
    if !has_budget {
        return None;
    }

    let mut budget = vk::PhysicalDeviceMemoryBudgetPropertiesEXT::default();
    let mut props = vk::PhysicalDeviceMemoryProperties2::default().push_next(&mut budget);
    // SAFETY: Vulkan 1.1 core query; the budget struct is valid to chain
    // whenever the device supports VK_EXT_memory_budget (checked above).
    unsafe { instance.get_physical_device_memory_properties2(phys, &mut props) };
    let heaps = props.memory_properties.memory_heaps_as_slice();
    let free = heaps
        .iter()
        .enumerate()
        .filter(|(_, heap)| heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
        .map(|(i, _)| budget.heap_budget[i].saturating_sub(budget.heap_usage[i]))
        .sum();
    Some(free)
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
