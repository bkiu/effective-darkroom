//! Headless check: how much VRAM can each adapter actually hand out?
//!
//! Allocates 128 MiB textures until one fails, catching the error in a scope
//! instead of panicking. Shows what the develop engine has to live inside when
//! another process (vLLM, a game) already holds most of a card.

use eframe::wgpu;

fn main() {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(desc);
    for adapter in block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN)) {
        let info = adapter.get_info();
        let Ok((device, _queue)) = block_on(adapter.request_device(&Default::default())) else {
            continue;
        };
        let mut held = Vec::new();
        loop {
            let scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: 4096, height: 4096, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            if block_on(scope.pop()).is_some() || held.len() >= 256 {
                break;
            }
            held.push(t);
        }
        println!("{} ({:?}): {} MiB", info.name, info.device_type, held.len() * 128);
    }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::task::{Context, Poll, Wake, Waker};
    struct W(std::thread::Thread);
    impl Wake for W {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(std::sync::Arc::new(W(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut f = std::pin::pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park();
    }
}
