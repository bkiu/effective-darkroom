//! Headless render through the develop engine — used for pipeline
//! verification and base-curve fitting comparisons.
//!
//!   cargo run --example render_cli -- <input> <output.png|jpg> [--params params.json] [--long-edge N] [--no-base]
//!
//! `params.json` is an EditParams JSON (defaults when omitted).

use eframe::wgpu;
use effective_darkroom::catalog::{FileKind, ImageRecord};
use effective_darkroom::decode::full::{self, FullPixels};
use effective_darkroom::engine::curve::BaseCurve;
use effective_darkroom::engine::params::EditParams;
use effective_darkroom::engine::{RenderEngine, SourcePixels};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag_value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let mut positional = Vec::new();
    let mut skip = false;
    for a in &args {
        if skip {
            skip = false;
            continue;
        }
        if a == "--params" || a == "--long-edge" {
            skip = true;
        } else if !a.starts_with("--") {
            positional.push(a.clone());
        }
    }
    let input = std::path::PathBuf::from(&positional[0]);
    let output = std::path::PathBuf::from(&positional[1]);
    let params: EditParams = match flag_value("--params") {
        Some(p) => EditParams::from_json(&std::fs::read_to_string(p)?),
        None => EditParams::default(),
    };
    let long_edge: Option<u32> = flag_value("--long-edge").and_then(|v| v.parse().ok());
    let no_base = args.iter().any(|a| a == "--no-base");

    let kind = FileKind::from_path(&input).expect("unsupported input type");
    let rec = ImageRecord {
        id: 0,
        abs_path: input.clone(),
        file_name: input
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        kind,
        capture_ts: None,
        imported_ts: 0,
        width: 0,
        height: 0,
        orientation: 1,
        camera_make: String::new(),
        camera_model: String::new(),
        file_size: 0,
    };

    // Decode.
    let t = std::time::Instant::now();
    let (image, camera_model) = match kind {
        FileKind::Jpeg => (full::decode_preview(&rec)?, String::new()),
        FileKind::Raw => {
            let raw = libraw_ffi::RawFile::open(&input)?;
            let model = raw.metadata().model;
            drop(raw);
            (full::decode_linear_raw(&rec)?, model)
        }
    };
    eprintln!("decoded {}x{} in {:?}", image.width, image.height, t.elapsed());

    // Headless device.
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
    let instance = wgpu::Instance::new(desc);
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        ..Default::default()
    }))?;
    eprintln!("adapter: {}", adapter.get_info().name);
    let (device, queue) = block_on(adapter.request_device(&Default::default()))?;

    let mut engine = RenderEngine::new_headless(device, queue);
    match &image.pixels {
        FullPixels::Srgb8(rgba) => {
            engine.set_source(image.width, image.height, SourcePixels::Srgb8(rgba))
        }
        FullPixels::LinearF16(rgba) => {
            engine.set_source(image.width, image.height, SourcePixels::LinearF16(rgba))
        }
    }

    let (ref_temp, ref_tint) = match image.as_shot {
        Some(w) => (w.temp, w.tint),
        None => (6500.0, 0.0),
    };
    // If the params still have the default temp, adopt as-shot (mirrors GUI).
    let mut params = params;
    if params.wb_temp == 6500.0 && params.wb_tint == 0.0 && kind == FileKind::Raw {
        params.wb_temp = ref_temp;
        params.wb_tint = ref_tint;
    }
    let base = match (kind, no_base) {
        (FileKind::Raw, false) => Some(BaseCurve::for_camera(&camera_model)),
        _ => None,
    };

    let t = std::time::Instant::now();
    let _ = engine.render(&params, ref_temp, ref_tint, base.as_ref());
    let rendered = engine.read_output().expect("readback");
    eprintln!("rendered + readback in {:?}", t.elapsed());

    let out_img = match long_edge {
        Some(edge) => {
            let (w, h) = (rendered.width(), rendered.height());
            let scale = edge as f32 / w.max(h) as f32;
            image::imageops::resize(
                &rendered,
                ((w as f32 * scale) as u32).max(1),
                ((h as f32 * scale) as u32).max(1),
                image::imageops::FilterType::Lanczos3,
            )
        }
        None => rendered,
    };
    image::DynamicImage::ImageRgba8(out_img)
        .to_rgb8()
        .save(&output)?;
    eprintln!("wrote {}", output.display());
    Ok(())
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::sync::mpsc;
    use std::task::{Context, Poll, Wake, Waker};
    struct W(mpsc::Sender<()>);
    impl Wake for W {
        fn wake(self: std::sync::Arc<Self>) {
            let _ = self.0.send(());
        }
    }
    let (tx, rx) = mpsc::channel();
    let waker = Waker::from(std::sync::Arc::new(W(tx)));
    let mut cx = Context::from_waker(&waker);
    let mut f = Box::pin(f);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => {
                let _ = rx.recv();
            }
        }
    }
}
