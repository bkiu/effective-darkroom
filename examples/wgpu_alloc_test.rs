//! Headless check: can each adapter allocate the develop engine's textures?

use eframe::wgpu;

fn main() {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
    let instance = wgpu::Instance::new(desc);
    for adapter in pollster_block(instance.enumerate_adapters(wgpu::Backends::all())) {
        let info = adapter.get_info();
        println!("== {} ({:?}, {:?})", info.name, info.device_type, info.backend);
        let (device, queue) = match pollster_block(adapter.request_device(&Default::default())) {
            Ok(dq) => dq,
            Err(e) => {
                println!("   device error: {e}");
                continue;
            }
        };
        device.on_uncaptured_error(std::sync::Arc::new(|e: wgpu::Error| println!("   UNCAPTURED: {e}")));
        let (w, h) = (6240u32, 4160u32);
        let src = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("src"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let data = vec![128u8; (w * h * 4) as usize];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &src,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let _out = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("out"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        queue.submit([]);
        let _ = device.poll(wgpu::PollType::Poll);
        println!("   ok");
    }
}

fn pollster_block<F: std::future::Future>(f: F) -> F::Output {
    // Tiny local block_on to avoid a pollster dep.
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
