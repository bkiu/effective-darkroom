//! GPU develop engine. Renders the current source image through the develop
//! shader into an offscreen texture that egui displays natively; the same
//! path will drive export so preview == export.
//!
//! Passes per render:
//!   1. (texture/clarity only) adjusted-luma at half res, separable gaussian
//!      small blur; downsample to 1/8 and separable large blur
//!   2. develop pass (WB → exposure → dehaze → encode → local contrast →
//!      tone LUT → vibrance/saturation) into the display texture
//!   3. the same develop pass into a small 256×256 target, read back
//!      asynchronously for the histogram

pub mod color;
pub mod curve;
pub mod params;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use eframe::egui_wgpu::{self, RenderState};
use eframe::wgpu;
use egui::mutex::RwLock;

use curve::BaseCurve;
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
    texture_amt: f32,
    clarity_amt: f32,
    dehaze_amt: f32,
    use_blur: f32,
    _pad0: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurUniforms {
    dir: [f32; 2],
    sigma: f32,
    _pad0: f32,
}

const SMALL_SIGMA: f32 = 3.0;
const LARGE_SIGMA: f32 = 10.0; // at 1/8 resolution
const HIST_SIZE: u32 = 256;

struct Source {
    view: wgpu::TextureView,
    _texture: wgpu::Texture,
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

/// A render target + view for an intermediate pass.
struct Target {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Blur chain resources, tied to the current source.
struct Aux {
    half_a: Target,
    half_b: Target,
    half_final: Target,
    eighth_a: Target,
    eighth_b: Target,
    eighth_final: Target,
    bg_luma: wgpu::BindGroup,
    bg_h_half: wgpu::BindGroup,
    bg_v_half: wgpu::BindGroup,
    bg_down: wgpu::BindGroup,
    bg_h_8: wgpu::BindGroup,
    bg_v_8: wgpu::BindGroup,
    half_size: (u32, u32),
    eighth_size: (u32, u32),
}

struct HistReadback {
    tex: Target,
    buf: wgpu::Buffer,
    mapped: Arc<AtomicBool>,
    in_flight: bool,
}

pub struct RenderEngine {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Present in the GUI; None for headless (export/tools) use.
    renderer: Option<Arc<RwLock<egui_wgpu::Renderer>>>,

    develop_pipeline: wgpu::RenderPipeline,
    develop_bgl: wgpu::BindGroupLayout,
    luma_pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    blur_bgl: wgpu::BindGroupLayout,

    sampler: wgpu::Sampler,
    uniform_buf: wgpu::Buffer,
    blur_bufs: [wgpu::Buffer; 5], // h_half, v_half, down, h_8, v_8
    lut_tex: wgpu::Texture,
    lut_view: wgpu::TextureView,
    dummy_r16: Target,

    source: Option<Source>,
    aux: Option<Aux>,
    develop_bind_group: Option<wgpu::BindGroup>,
    bind_group_uses_blur: bool,
    output: Option<Output>,

    hist: HistReadback,
    /// Latest RGB histogram of the rendered image (256 bins per channel).
    pub histogram: Option<Box<[[u32; 256]; 3]>>,
}

const OUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const BLUR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

fn texture_bgl_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn uniform_bgl_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl RenderEngine {
    pub fn new(rs: &RenderState) -> Self {
        let info = rs.adapter.get_info();
        tracing::info!(
            "render adapter: {} ({:?}, {:?})",
            info.name,
            info.device_type,
            info.backend
        );
        Self::build(
            rs.device.clone(),
            rs.queue.clone(),
            Some(rs.renderer.clone()),
        )
    }

    /// Engine without an egui renderer, for export and headless tools.
    pub fn new_headless(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self::build(device, queue, None)
    }

    fn build(
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderer: Option<Arc<RwLock<egui_wgpu::Renderer>>>,
    ) -> Self {

        let develop_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("develop"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/develop.wgsl").into()),
        });
        let blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blur.wgsl").into()),
        });

        let develop_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("develop-bgl"),
            entries: &[
                texture_bgl_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform_bgl_entry(2),
                texture_bgl_entry(3),
                texture_bgl_entry(4),
                texture_bgl_entry(5),
            ],
        });
        let blur_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur-bgl"),
            entries: &[
                texture_bgl_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform_bgl_entry(2),
                uniform_bgl_entry(3),
            ],
        });

        let make_pipeline = |label: &str,
                             shader: &wgpu::ShaderModule,
                             bgl: &wgpu::BindGroupLayout,
                             entry: &str,
                             format: wgpu::TextureFormat| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(bgl)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let develop_pipeline = make_pipeline(
            "develop-pipeline",
            &develop_shader,
            &develop_bgl,
            "fs_main",
            OUT_FORMAT,
        );
        let luma_pipeline = make_pipeline(
            "luma-pipeline",
            &blur_shader,
            &blur_bgl,
            "fs_luma",
            BLUR_FORMAT,
        );
        let blur_pipeline = make_pipeline(
            "blur-pipeline",
            &blur_shader,
            &blur_bgl,
            "fs_blur",
            BLUR_FORMAT,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("develop-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
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
        let blur_bufs = std::array::from_fn(|i| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("blur-uniforms-{i}")),
                size: std::mem::size_of::<BlurUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });

        let lut_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tone-lut"),
            size: wgpu::Extent3d {
                width: curve::LUT_SIZE as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let lut_view = lut_tex.create_view(&Default::default());

        let dummy_r16 = Self::make_target(&device, 1, 1, BLUR_FORMAT, "dummy-r16");

        let hist_tex = Self::make_target(&device, HIST_SIZE, HIST_SIZE, OUT_FORMAT, "hist");
        let hist_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hist-readback"),
            size: (HIST_SIZE * HIST_SIZE * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            device,
            queue,
            renderer,
            develop_pipeline,
            develop_bgl,
            luma_pipeline,
            blur_pipeline,
            blur_bgl,
            sampler,
            uniform_buf,
            blur_bufs,
            lut_tex,
            lut_view,
            dummy_r16,
            source: None,
            aux: None,
            develop_bind_group: None,
            bind_group_uses_blur: false,
            output: None,
            hist: HistReadback {
                tex: hist_tex,
                buf: hist_buf,
                mapped: Arc::new(AtomicBool::new(false)),
                in_flight: false,
            },
            histogram: None,
        }
    }

    fn make_target(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        label: &str,
    ) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Target {
            _texture: texture,
            view,
        }
    }

    pub fn clear_source(&mut self) {
        self.source = None;
        self.aux = None;
        self.develop_bind_group = None;
        self.histogram = None;
    }

    pub fn has_source(&self) -> bool {
        self.source.is_some()
    }

    /// Upload a new source image; recreates dependent resources when needed.
    pub fn set_source(&mut self, width: u32, height: u32, pixels: SourcePixels<'_>) {
        let (format, bytes_per_px, data): (_, u32, &[u8]) = match pixels {
            SourcePixels::Srgb8(rgba) => (wgpu::TextureFormat::Rgba8UnormSrgb, 4, rgba),
            SourcePixels::LinearF16(rgba) => (
                wgpu::TextureFormat::Rgba16Float,
                8,
                bytemuck::cast_slice(rgba),
            ),
        };
        assert_eq!(
            data.len() as u64,
            width as u64 * height as u64 * bytes_per_px as u64
        );

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

        self.source = Some(Source {
            view: texture.create_view(&Default::default()),
            _texture: texture,
            width,
            height,
        });
        self.aux = None;
        self.develop_bind_group = None;
        self.ensure_output(width, height);
    }

    fn ensure_output(&mut self, width: u32, height: u32) {
        if let Some(out) = &self.output {
            if out.width == width && out.height == height {
                return;
            }
        }
        let target = Self::make_target(&self.device, width, height, OUT_FORMAT, "develop-out");

        let prev_id = self.output.take().and_then(|o| o.egui_id);
        let egui_id = self.renderer.as_ref().map(|renderer| {
            let mut renderer = renderer.write();
            match prev_id {
                Some(id) => {
                    renderer.update_egui_texture_from_wgpu_texture(
                        &self.device,
                        &target.view,
                        wgpu::FilterMode::Linear,
                        id,
                    );
                    id
                }
                None => renderer.register_native_texture(
                    &self.device,
                    &target.view,
                    wgpu::FilterMode::Linear,
                ),
            }
        });
        self.output = Some(Output {
            _texture: target._texture,
            view: target.view,
            egui_id,
            width,
            height,
        });
    }

    fn blur_bind_group(&self, label: &str, input: &wgpu::TextureView, buf: &wgpu::Buffer) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.blur_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buf.as_entire_binding(),
                },
            ],
        })
    }

    fn ensure_aux(&mut self) {
        if self.aux.is_some() {
            return;
        }
        let Some(source) = &self.source else { return };
        let (hw, hh) = ((source.width / 2).max(1), (source.height / 2).max(1));
        let (ew, eh) = ((source.width / 8).max(1), (source.height / 8).max(1));

        let half_a = Self::make_target(&self.device, hw, hh, BLUR_FORMAT, "half-a");
        let half_b = Self::make_target(&self.device, hw, hh, BLUR_FORMAT, "half-b");
        let half_final = Self::make_target(&self.device, hw, hh, BLUR_FORMAT, "half-final");
        let eighth_a = Self::make_target(&self.device, ew, eh, BLUR_FORMAT, "eighth-a");
        let eighth_b = Self::make_target(&self.device, ew, eh, BLUR_FORMAT, "eighth-b");
        let eighth_final = Self::make_target(&self.device, ew, eh, BLUR_FORMAT, "eighth-final");

        let blur_params = [
            BlurUniforms {
                dir: [1.0 / hw as f32, 0.0],
                sigma: SMALL_SIGMA,
                _pad0: 0.0,
            },
            BlurUniforms {
                dir: [0.0, 1.0 / hh as f32],
                sigma: SMALL_SIGMA,
                _pad0: 0.0,
            },
            BlurUniforms {
                dir: [0.0, 0.0],
                sigma: 0.0,
                _pad0: 0.0,
            },
            BlurUniforms {
                dir: [1.0 / ew as f32, 0.0],
                sigma: LARGE_SIGMA,
                _pad0: 0.0,
            },
            BlurUniforms {
                dir: [0.0, 1.0 / eh as f32],
                sigma: LARGE_SIGMA,
                _pad0: 0.0,
            },
        ];
        for (buf, p) in self.blur_bufs.iter().zip(blur_params.iter()) {
            self.queue.write_buffer(buf, 0, bytemuck::bytes_of(p));
        }

        let bg_luma = self.blur_bind_group("bg-luma", &source.view, &self.blur_bufs[0]);
        let bg_h_half = self.blur_bind_group("bg-h-half", &half_a.view, &self.blur_bufs[0]);
        let bg_v_half = self.blur_bind_group("bg-v-half", &half_b.view, &self.blur_bufs[1]);
        let bg_down = self.blur_bind_group("bg-down", &half_a.view, &self.blur_bufs[2]);
        let bg_h_8 = self.blur_bind_group("bg-h-8", &eighth_a.view, &self.blur_bufs[3]);
        let bg_v_8 = self.blur_bind_group("bg-v-8", &eighth_b.view, &self.blur_bufs[4]);

        self.aux = Some(Aux {
            half_a,
            half_b,
            half_final,
            eighth_a,
            eighth_b,
            eighth_final,
            bg_luma,
            bg_h_half,
            bg_v_half,
            bg_down,
            bg_h_8,
            bg_v_8,
            half_size: (hw, hh),
            eighth_size: (ew, eh),
        });
    }

    fn ensure_develop_bind_group(&mut self, use_blur: bool) {
        if self.develop_bind_group.is_some() && self.bind_group_uses_blur == use_blur {
            return;
        }
        let Some(source) = &self.source else { return };
        let (small, large) = match (&self.aux, use_blur) {
            (Some(aux), true) => (&aux.half_final.view, &aux.eighth_final.view),
            _ => (&self.dummy_r16.view, &self.dummy_r16.view),
        };
        self.develop_bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("develop-bg"),
            layout: &self.develop_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(small),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(large),
                },
            ],
        }));
        self.bind_group_uses_blur = use_blur;
    }

    fn upload_lut(&self, params: &EditParams, base: Option<&BaseCurve>) {
        let lut = curve::build_lut(params, base);
        let mut data = Vec::with_capacity(curve::LUT_SIZE * 4);
        for i in 0..curve::LUT_SIZE {
            data.push(half::f16::from_f32(lut.r[i]).to_bits());
            data.push(half::f16::from_f32(lut.g[i]).to_bits());
            data.push(half::f16::from_f32(lut.b[i]).to_bits());
            data.push(half::f16::from_f32(1.0).to_bits());
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.lut_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(curve::LUT_SIZE as u32 * 8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: curve::LUT_SIZE as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }

    fn uniforms(
        params: &EditParams,
        ref_temp: f32,
        ref_tint: f32,
        baseline_ev: f32,
        use_blur: bool,
    ) -> Uniforms {
        let wb = color::wb_matrix(params.wb_temp, params.wb_tint, ref_temp, ref_tint);
        Uniforms {
            wb0: [wb[0][0], wb[0][1], wb[0][2], 0.0],
            wb1: [wb[1][0], wb[1][1], wb[1][2], 0.0],
            wb2: [wb[2][0], wb[2][1], wb[2][2], 0.0],
            exposure_gain: (params.exposure + baseline_ev).exp2(),
            saturation: params.saturation / 100.0,
            vibrance: params.vibrance / 100.0,
            texture_amt: params.texture / 100.0,
            clarity_amt: params.clarity / 100.0,
            dehaze_amt: params.dehaze / 100.0,
            use_blur: if use_blur { 1.0 } else { 0.0 },
            _pad0: 0.0,
        }
    }

    fn run_pass(
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        bind_group: &wgpu::BindGroup,
        target: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Render the current source with the given params. Returns the egui
    /// texture to draw plus its pixel size.
    pub fn render(
        &mut self,
        params: &EditParams,
        ref_temp: f32,
        ref_tint: f32,
        base: Option<&BaseCurve>,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        self.source.as_ref()?;
        self.output.as_ref()?;

        let use_blur = params.texture != 0.0 || params.clarity != 0.0;
        let baseline_ev = base.map(|b| b.baseline_ev).unwrap_or(0.0);

        self.queue.write_buffer(
            &self.uniform_buf,
            0,
            bytemuck::bytes_of(&Self::uniforms(
                params,
                ref_temp,
                ref_tint,
                baseline_ev,
                use_blur,
            )),
        );
        self.upload_lut(params, base);
        if use_blur {
            self.ensure_aux();
        }
        self.ensure_develop_bind_group(use_blur);

        let bind_group = self.develop_bind_group.as_ref()?;
        let source = self.source.as_ref()?;
        let output = self.output.as_ref()?;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("develop-encoder"),
            });

        if use_blur {
            if let Some(aux) = &self.aux {
                Self::run_pass(&mut encoder, &self.luma_pipeline, &aux.bg_luma, &aux.half_a.view);
                Self::run_pass(&mut encoder, &self.blur_pipeline, &aux.bg_h_half, &aux.half_b.view);
                Self::run_pass(&mut encoder, &self.blur_pipeline, &aux.bg_v_half, &aux.half_final.view);
                Self::run_pass(&mut encoder, &self.blur_pipeline, &aux.bg_down, &aux.eighth_a.view);
                Self::run_pass(&mut encoder, &self.blur_pipeline, &aux.bg_h_8, &aux.eighth_b.view);
                Self::run_pass(&mut encoder, &self.blur_pipeline, &aux.bg_v_8, &aux.eighth_final.view);
            }
        }

        Self::run_pass(&mut encoder, &self.develop_pipeline, bind_group, &output.view);
        Self::run_pass(&mut encoder, &self.develop_pipeline, bind_group, &self.hist.tex.view);

        let queue_hist = !self.hist.in_flight;
        if queue_hist {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.hist.tex._texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.hist.buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(HIST_SIZE * 4),
                        rows_per_image: Some(HIST_SIZE),
                    },
                },
                wgpu::Extent3d {
                    width: HIST_SIZE,
                    height: HIST_SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }

        self.queue.submit([encoder.finish()]);

        if queue_hist {
            self.hist.in_flight = true;
            self.hist.mapped.store(false, Ordering::SeqCst);
            let mapped = self.hist.mapped.clone();
            self.hist
                .buf
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |res| {
                    if res.is_ok() {
                        mapped.store(true, Ordering::SeqCst);
                    }
                });
        }

        let id = output.egui_id?;
        Some((id, egui::vec2(source.width as f32, source.height as f32)))
    }

    /// Blocking readback of the last rendered output (export / headless).
    pub fn read_output(&self) -> Option<image::RgbaImage> {
        let output = self.output.as_ref()?;
        let (w, h) = (output.width, output.height);
        // Rows must be 256-byte aligned for texture→buffer copies.
        let padded = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("export-readback"),
            size: padded as u64 * h as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("export-copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &output._texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = buf.slice(..);
        let done = Arc::new(AtomicBool::new(false));
        {
            let done = done.clone();
            slice.map_async(wgpu::MapMode::Read, move |res| {
                if res.is_ok() {
                    done.store(true, Ordering::SeqCst);
                }
            });
        }
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        if !done.load(Ordering::SeqCst) {
            return None;
        }
        let data = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h {
            let start = (row * padded) as usize;
            pixels.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        drop(data);
        buf.unmap();
        image::RgbaImage::from_raw(w, h, pixels)
    }

    /// Poll the async histogram readback; call once per frame.
    pub fn poll_histogram(&mut self) {
        if !self.hist.in_flight {
            return;
        }
        let _ = self.device.poll(wgpu::PollType::Poll);
        if !self.hist.mapped.load(Ordering::SeqCst) {
            return;
        }
        {
            let range = self.hist.buf.slice(..).get_mapped_range();
            let mut bins = Box::new([[0u32; 256]; 3]);
            for px in range.chunks_exact(4) {
                bins[0][px[0] as usize] += 1;
                bins[1][px[1] as usize] += 1;
                bins[2][px[2] as usize] += 1;
            }
            self.histogram = Some(bins);
        }
        self.hist.buf.unmap();
        self.hist.in_flight = false;
    }
}
