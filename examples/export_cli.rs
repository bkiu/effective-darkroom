//! Headless batch export of catalog images (testing the export pipeline).
//!
//!   cargo run --example export_cli -- <dest> [--quality N] [--long-edge N] [--prefix P] [--date-names]

use eframe::wgpu;
use effective_darkroom::catalog::Catalog;
use effective_darkroom::engine::export::{run_blocking, ExportSpec, Naming};
use effective_darkroom::engine::RenderEngine;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag_value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let dest = std::path::PathBuf::from(&args[0]);
    let quality: u8 = flag_value("--quality").and_then(|v| v.parse().ok()).unwrap_or(90);
    let long_edge: Option<u32> = flag_value("--long-edge").and_then(|v| v.parse().ok());
    let prefix = flag_value("--prefix").unwrap_or_default();
    let naming = if args.iter().any(|a| a == "--date-names") {
        Naming::DateTime
    } else {
        Naming::Original
    };

    let catalog = Catalog::open_default()?;
    let ids: Vec<i64> = catalog.all_images()?.iter().map(|r| r.id).collect();
    let spec = ExportSpec {
        image_ids: ids,
        dest,
        format: if args.iter().any(|a| a == "--dng") {
            effective_darkroom::engine::export::ExportFormat::Dng
        } else {
            effective_darkroom::engine::export::ExportFormat::Jpeg
        },
        quality,
        long_edge,
        prefix,
        naming,
    };

    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
    let instance = wgpu::Instance::new(desc);
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        ..Default::default()
    }))?;
    let (device, queue) = block_on(adapter.request_device(&Default::default()))?;
    let mut engine = RenderEngine::new_headless(device, queue);

    let outcome = run_blocking(&mut engine, &catalog, &spec, |cur, total, name| {
        println!("[{cur}/{total}] {name}");
    })?;
    println!("exported={} errors={:?}", outcome.exported, outcome.errors);
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
