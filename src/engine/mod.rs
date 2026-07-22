//! GPU develop engine. Renders the current source image through the develop
//! shader into an offscreen texture that egui displays natively; the same
//! path will drive export so preview == export.

pub mod color;
pub mod params;

use std::sync::Arc;

use eframe::egui_wgpu::{self, RenderState};
use eframe::wgpu;
use egui::mutex::RwLock;

use params::EditParams;

/// Pixel data for the source texture.
pub enum SourcePixels<'a> {
    /// 8-bit sRGB-encoded RGBA (JPEG sources; GPU linearizes on sample).
    Srgb8(&'a [u8]),
    /// f16 linear RGBA bit patterns (RAW sources).
    LinearF16(&'a [u16]),
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    wb0: [f32; 4],
    wb1: [f32; 4],
    wb2: [f32; 4],
    exposure_gain: f32,
    saturation: f32,
    vibrance: f32,
    _pad0: f32,
}

struct Source {
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

struct Output {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    egui_id: Option<egui::TextureId>,
    width: u32,
    height: u32,
}

pub struct RenderEngine {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Arc<RwLock<egui_wgpu::Renderer>>,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buf: wgpu::Buffer,
    source: Option<Source>,
    output: Option<Output>,
}

const OUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl RenderEngine {
    pub fn new(rs: &RenderState) -> Self {
        let info = rs.adapter.get_info();
        tracing::info!(
            "render adapter: {} ({:?}, {:?})",
            info.name,
            info.device_type,
            info.backend
        );
        let device = rs.device.clone();
        let queue = rs.queue.clone();

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("develop"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/develop.wgsl").into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("develop-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("develop-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("develop-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(OUT_FORMAT.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("develop-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("develop-uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            device,
            queue,
            renderer: rs.renderer.clone(),
            pipeline,
            bind_group_layout,
            sampler,
            uniform_buf,
            source: None,
            output: None,
        }
    }

    pub fn clear_source(&mut self) {
        self.source = None;
    }

    pub fn has_source(&self) -> bool {
        self.source.is_some()
    }

    /// Upload a new source image; recreates the output target when the size
    /// changes.
    pub fn set_source(&mut self, width: u32, height: u32, pixels: SourcePixels<'_>) {
        let (format, bytes_per_px, data): (_, u32, &[u8]) = match pixels {
            SourcePixels::Srgb8(rgba) => (wgpu::TextureFormat::Rgba8UnormSrgb, 4, rgba),
            SourcePixels::LinearF16(rgba) => (
                wgpu::TextureFormat::Rgba16Float,
                8,
                bytemuck::cast_slice(rgba),
            ),
        };
        assert_eq!(data.len() as u64, width as u64 * height as u64 * bytes_per_px as u64);

        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("develop-src"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * bytes_per_px),
                rows_per_image: Some(height),
            },
            size,
        );

        let view = texture.create_view(&Default::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("develop-bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform_buf.as_entire_binding(),
                },
            ],
        });
        self.source = Some(Source {
            bind_group,
            width,
            height,
        });
        self.ensure_output(width, height);
    }

    fn ensure_output(&mut self, width: u32, height: u32) {
        if let Some(out) = &self.output {
            if out.width == width && out.height == height {
                return;
            }
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("develop-out"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());

        let prev_id = self.output.take().and_then(|o| o.egui_id);
        let egui_id = {
            let mut renderer = self.renderer.write();
            match prev_id {
                Some(id) => {
                    renderer.update_egui_texture_from_wgpu_texture(
                        &self.device,
                        &view,
                        wgpu::FilterMode::Linear,
                        id,
                    );
                    Some(id)
                }
                None => Some(renderer.register_native_texture(
                    &self.device,
                    &view,
                    wgpu::FilterMode::Linear,
                )),
            }
        };
        self.output = Some(Output {
            _texture: texture,
            view,
            egui_id,
            width,
            height,
        });
    }

    fn uniforms(params: &EditParams, ref_temp: f32, ref_tint: f32) -> Uniforms {
        let wb = color::wb_matrix(params.wb_temp, params.wb_tint, ref_temp, ref_tint);
        Uniforms {
            wb0: [wb[0][0], wb[0][1], wb[0][2], 0.0],
            wb1: [wb[1][0], wb[1][1], wb[1][2], 0.0],
            wb2: [wb[2][0], wb[2][1], wb[2][2], 0.0],
            exposure_gain: params.exposure.exp2(),
            saturation: params.saturation / 100.0,
            vibrance: params.vibrance / 100.0,
            _pad0: 0.0,
        }
    }

    /// Render the current source with the given params. Returns the egui
    /// texture to draw plus its pixel size.
    pub fn render(
        &mut self,
        params: &EditParams,
        ref_temp: f32,
        ref_tint: f32,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        let source = self.source.as_ref()?;
        let output = self.output.as_ref()?;

        self.queue.write_buffer(
            &self.uniform_buf,
            0,
            bytemuck::bytes_of(&Self::uniforms(params, ref_temp, ref_tint)),
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("develop-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("develop-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &source.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);

        let id = output.egui_id?;
        Some((id, egui::vec2(source.width as f32, source.height as f32)))
    }
}
