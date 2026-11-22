//! Hardware-accelerated `wgpu` (Vulkan / GLES) analytic SDF glassmorphic renderer.
//!
//! Consumes the exact same renderer-agnostic [`DrawList`] as [`crate::SdfSoftwareRenderer`] in a
//! single instanced render pass with premultiplied alpha blending, plus a blur-once GPU static
//! wallpaper cache for frosted glass backdrops on low-power iGPUs (e.g. Intel UHD 615).

use std::collections::HashMap;
use std::sync::mpsc;

use bytemuck::{Pod, Zeroable};
use veilshell_core::{Rgba, ThemePalette};

use crate::font::{glyph_rows, measure_text_width, GLYPH_ADVANCE, GLYPH_H, GLYPH_W};
use crate::primitives::{DrawCommand, DrawList, RoundedRect, SceneOutput, TextAlign, TextLabel};
use crate::scene::{build_scene, build_scene_layout, build_top_strip_scene, SceneContext};
use crate::{RenderError, RenderFrame, RendererBackend, SceneKind, SnapshotSpec, WallpaperImage};

const SDF_WGSL: &str = include_str!("shaders/sdf.wgsl");
const BLUR_WGSL: &str = include_str!("shaders/blur.wgsl");

const KIND_BACKDROP: u32 = 0;
const KIND_GLOW: u32 = 1;
const KIND_RECT: u32 = 2;
const KIND_GLYPH: u32 = 3;

const INITIAL_INSTANCE_CAPACITY: usize = 512;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ViewportUniforms {
    size: [f32; 2],
    wallpaper_enabled: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct BlurUniforms {
    texel_dir: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuPrimitive {
    rect: [f32; 4],
    color0: [f32; 4],
    color1: [f32; 4],
    params0: [f32; 4],
    params1: [f32; 4],
    meta: [u32; 4],
}

impl GpuPrimitive {
    const ATTRIBS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
        4 => Float32x4,
        5 => Uint32x4,
    ];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }
}

struct OffscreenTarget {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_bytes_per_row: u32,
}

/// GPU-accelerated `wgpu` Vulkan / GLES WGSL SDF renderer with a blur-once wallpaper cache.
pub struct WgpuRenderer {
    pub context: SceneContext,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    adapter_summary: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    sdf_shader: wgpu::ShaderModule,
    sdf_bgl: wgpu::BindGroupLayout,
    sdf_pipeline_layout: wgpu::PipelineLayout,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    blur_bgl: wgpu::BindGroupLayout,
    blur_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    wallpaper_sampler: wgpu::Sampler,
    wallpaper_texture: wgpu::Texture,
    wallpaper_enabled: bool,
    sdf_bind_group: wgpu::BindGroup,
    offscreen: Option<OffscreenTarget>,
}

impl std::fmt::Debug for WgpuRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuRenderer")
            .field("adapter", &self.adapter_summary)
            .field("wallpaper_enabled", &self.wallpaper_enabled)
            .field("pipelines", &self.pipelines.len())
            .finish_non_exhaustive()
    }
}

impl WgpuRenderer {
    /// Initialize a `wgpu` Vulkan/GLES renderer with default [`SceneContext`].
    ///
    /// # Errors
    /// Returns [`RenderError::Gpu`] if no compatible Vulkan/GLES GPU adapter or device is available.
    pub fn new() -> Result<Self, RenderError> {
        Self::with_context(SceneContext::default())
    }

    /// Initialize a `wgpu` Vulkan/GLES renderer with custom dynamic [`SceneContext`].
    ///
    /// Prefers low-power integrated GPUs (ideal for fanless 5W tablets and hybrid-GPU laptops),
    /// falling back to high-performance or software adapters automatically.
    ///
    /// # Errors
    /// Returns [`RenderError::Gpu`] if no compatible GPU adapter or device can be created.
    #[allow(clippy::too_many_lines)]
    pub fn with_context(context: SceneContext) -> Result<Self, RenderError> {
        let (instance, adapter) = create_instance_and_adapter()?;
        let info = adapter.get_info();
        let adapter_summary = format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type);

        let limits = wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("veilshell-wgpu-device"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..wgpu::DeviceDescriptor::default()
        }))
        .map_err(|e| RenderError::Gpu(format!("request_device failed: {e}")))?;

        let sdf_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("veilshell-sdf-wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_WGSL.into()),
        });
        let blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("veilshell-blur-wgsl"),
            source: wgpu::ShaderSource::Wgsl(BLUR_WGSL.into()),
        });

        let sdf_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("veilshell-sdf-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sdf_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("veilshell-sdf-pipeline-layout"),
            bind_group_layouts: &[&sdf_bgl],
            push_constant_ranges: &[],
        });

        let blur_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("veilshell-blur-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let blur_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("veilshell-blur-pipeline-layout"),
            bind_group_layouts: &[&blur_bgl],
            push_constant_ranges: &[],
        });

        let blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("veilshell-blur-pipeline"),
            layout: Some(&blur_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blur_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blur_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("veilshell-viewport-uniforms"),
            size: std::mem::size_of::<ViewportUniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("veilshell-sdf-instances"),
            size: (INITIAL_INSTANCE_CAPACITY * std::mem::size_of::<GpuPrimitive>())
                as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let wallpaper_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("veilshell-wallpaper-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let wallpaper_texture = create_dummy_wallpaper_texture(&device, &queue);
        let wallpaper_view = wallpaper_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sdf_bind_group = create_sdf_bind_group(
            &device,
            &sdf_bgl,
            &uniform_buffer,
            &wallpaper_view,
            &wallpaper_sampler,
        );

        let mut renderer = Self {
            context,
            instance,
            adapter,
            adapter_summary,
            device,
            queue,
            sdf_shader,
            sdf_bgl,
            sdf_pipeline_layout,
            pipelines: HashMap::new(),
            blur_bgl,
            blur_pipeline,
            uniform_buffer,
            instance_buffer,
            instance_capacity: INITIAL_INSTANCE_CAPACITY,
            wallpaper_sampler,
            wallpaper_texture,
            wallpaper_enabled: false,
            sdf_bind_group,
            offscreen: None,
        };
        // Pre-warm the offscreen snapshot pipeline format.
        renderer.ensure_pipeline(wgpu::TextureFormat::Rgba8Unorm);
        Ok(renderer)
    }

    /// Underlying [`wgpu::Instance`] for creating compatible `Wayland` surfaces.
    #[must_use]
    pub const fn instance(&self) -> &wgpu::Instance {
        &self.instance
    }

    /// Active [`wgpu::Adapter`] used for surface capability queries.
    #[must_use]
    pub const fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    /// Active [`wgpu::Device`] used for surface configuration and rendering.
    #[must_use]
    pub const fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Active [`wgpu::Queue`] for submitting render work.
    #[must_use]
    pub const fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Human-readable GPU adapter summary (e.g. `AMD Radeon RX 7700 XT (Vulkan, DiscreteGpu)`).
    #[must_use]
    pub fn adapter_summary(&self) -> &str {
        &self.adapter_summary
    }

    /// Return `true` when a pre-blurred static wallpaper texture is cached on the GPU.
    #[must_use]
    pub const fn has_blurred_wallpaper(&self) -> bool {
        self.wallpaper_enabled
    }

    /// Upload a static wallpaper image and pre-blur it ONCE on the GPU into a half-resolution
    /// frosted glass cache texture.
    ///
    /// Subsequent overlay frames sample this cached texture with zero per-frame blur overhead,
    /// staying strictly within the 5W tablet frame-time budget.
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidViewport`] if the image dimensions or byte length are invalid.
    #[allow(clippy::too_many_lines)]
    pub fn set_wallpaper(&mut self, image: &WallpaperImage) -> Result<(), RenderError> {
        let expected = (image.width as usize)
            .saturating_mul(image.height as usize)
            .saturating_mul(4);
        if image.width == 0 || image.height == 0 || image.rgba.len() != expected {
            return Err(RenderError::InvalidViewport);
        }

        let src_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("veilshell-wallpaper-src"),
            size: wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &src_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width * 4),
                rows_per_image: Some(image.height),
            },
            wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
        );

        // Blur at half resolution (capped at 1280px) to keep VRAM < 3 MiB on 4-8 GB tablets.
        let blur_w = (image.width / 2).clamp(1, 1280);
        let blur_h = (image.height / 2).clamp(1, 1280);
        let blur_size = wgpu::Extent3d {
            width: blur_w,
            height: blur_h,
            depth_or_array_layers: 1,
        };

        let make_blur_target = |label: &'static str| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: blur_size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };

        let scratch = make_blur_target("veilshell-wallpaper-blur-scratch");
        let blurred = make_blur_target("veilshell-wallpaper-blur-final");

        let src_view = src_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let scratch_view = scratch.create_view(&wgpu::TextureViewDescriptor::default());
        let blurred_view = blurred.create_view(&wgpu::TextureViewDescriptor::default());

        let passes = [
            (&src_view, &scratch_view, [1.5 / blur_w as f32, 0.0]),
            (&scratch_view, &blurred_view, [0.0, 1.5 / blur_h as f32]),
            (&blurred_view, &scratch_view, [3.0 / blur_w as f32, 0.0]),
            (&scratch_view, &blurred_view, [0.0, 3.0 / blur_h as f32]),
        ];

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("veilshell-wallpaper-blur-encoder"),
            });

        // Each pass needs its own uniform buffer value because all 4 passes are recorded into
        // one command encoder before submission.
        let pass_resources: Vec<(wgpu::Buffer, wgpu::BindGroup)> = passes
            .iter()
            .map(|(in_view, _, dir)| {
                let ubuf = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("veilshell-blur-uniform"),
                    size: std::mem::size_of::<BlurUniforms>() as wgpu::BufferAddress,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.queue.write_buffer(
                    &ubuf,
                    0,
                    bytemuck::bytes_of(&BlurUniforms {
                        texel_dir: *dir,
                        _pad: [0.0, 0.0],
                    }),
                );
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("veilshell-blur-bg"),
                    layout: &self.blur_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: ubuf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(in_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.wallpaper_sampler),
                        },
                    ],
                });
                (ubuf, bg)
            })
            .collect();

        for ((_, out_view, _), (_, bg)) in passes.iter().zip(pass_resources.iter()) {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("veilshell-blur-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: out_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.blur_pipeline);
            rpass.set_bind_group(0, bg, &[]);
            rpass.draw(0..3, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));

        self.sdf_bind_group = create_sdf_bind_group(
            &self.device,
            &self.sdf_bgl,
            &self.uniform_buffer,
            &blurred_view,
            &self.wallpaper_sampler,
        );
        self.wallpaper_texture = blurred;
        self.wallpaper_enabled = true;
        Ok(())
    }

    /// Clear any cached wallpaper texture and revert overlays to pure theme gradients.
    pub fn clear_wallpaper(&mut self) {
        self.wallpaper_texture = create_dummy_wallpaper_texture(&self.device, &self.queue);
        let view = self
            .wallpaper_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.sdf_bind_group = create_sdf_bind_group(
            &self.device,
            &self.sdf_bgl,
            &self.uniform_buffer,
            &view,
            &self.wallpaper_sampler,
        );
        self.wallpaper_enabled = false;
    }

    /// Render a [`DrawList`] directly into an existing [`wgpu::TextureView`] (such as a `Wayland`
    /// swapchain surface texture) in a single instanced render pass with zero CPU readback.
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidViewport`] if `width` or `height` is zero.
    pub fn render_to_texture_view(
        &mut self,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        list: &DrawList,
    ) -> Result<(), RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::InvalidViewport);
        }

        let primitives = encode_draw_list(list, width as f32, height as f32);
        self.upload_frame_data(width, height, &primitives);
        self.ensure_pipeline(format);

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("veilshell-surface-encoder"),
            });

        self.record_sdf_pass(&mut encoder, view, format, primitives.len() as u32);
        self.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }

    /// Build layout (for hit-testing) and rasterize an interactive shell scene into a CPU [`RenderFrame`].
    ///
    /// # Errors
    /// Returns [`RenderError`] if the viewport is invalid or GPU readback fails.
    pub fn render_interactive(
        &mut self,
        scene: SceneKind,
        width: u32,
        height: u32,
        palette: &ThemePalette,
    ) -> Result<(RenderFrame, SceneOutput), RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::InvalidViewport);
        }
        let layout = build_scene_layout(scene, width as f32, height as f32, palette, &self.context);
        let frame = self.rasterize(width, height, &layout.draw_list)?;
        Ok((frame, layout))
    }

    /// Render the subtle top pull handle strip (`LayerRole::TopStrip`) offscreen.
    ///
    /// # Errors
    /// Returns [`RenderError`] if dimensions are zero or GPU readback fails.
    pub fn render_top_strip(
        &mut self,
        width: u32,
        height: u32,
        palette: &ThemePalette,
    ) -> Result<RenderFrame, RenderError> {
        let list = build_top_strip_scene(
            width as f32,
            height as f32,
            palette,
            self.context.top_drag_y,
        );
        self.rasterize(width, height, &list)
    }

    /// Rasterize an arbitrary [`DrawList`] on the GPU into a straight `RGBA8` [`RenderFrame`].
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidViewport`] if `width` or `height` is zero, or
    /// [`RenderError::Gpu`] if GPU buffer mapping fails.
    pub fn rasterize(
        &mut self,
        width: u32,
        height: u32,
        list: &DrawList,
    ) -> Result<RenderFrame, RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::InvalidViewport);
        }

        let primitives = encode_draw_list(list, width as f32, height as f32);
        self.upload_frame_data(width, height, &primitives);
        self.ensure_pipeline(wgpu::TextureFormat::Rgba8Unorm);
        self.ensure_offscreen_target(width, height);

        let target = self
            .offscreen
            .as_ref()
            .ok_or_else(|| RenderError::Gpu("offscreen render target unavailable".to_owned()))?;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("veilshell-offscreen-encoder"),
            });

        {
            let pipeline = &self.pipelines[&wgpu::TextureFormat::Rgba8Unorm];
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("veilshell-sdf-offscreen-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if !primitives.is_empty() {
                rpass.set_pipeline(pipeline);
                rpass.set_bind_group(0, &self.sdf_bind_group, &[]);
                rpass.set_vertex_buffer(0, self.instance_buffer.slice(..));
                rpass.draw(0..6, 0..primitives.len() as u32);
            }
        }

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &target.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = target.readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device
            .poll(wgpu::PollType::Wait)
            .map_err(|e| RenderError::Gpu(format!("device poll failed: {e}")))?;
        rx.recv()
            .map_err(|_| RenderError::Gpu("GPU readback channel dropped".to_owned()))?
            .map_err(|e| RenderError::Gpu(format!("GPU buffer map failed: {e}")))?;

        let mapped = slice.get_mapped_range();
        let rgba = unpremultiply_padded_rows(
            &mapped,
            width as usize,
            height as usize,
            target.padded_bytes_per_row as usize,
        );
        drop(mapped);
        target.readback.unmap();

        Ok(RenderFrame {
            width,
            height,
            rgba,
        })
    }

    fn upload_frame_data(&mut self, width: u32, height: u32, primitives: &[GpuPrimitive]) {
        let uniforms = ViewportUniforms {
            size: [width as f32, height as f32],
            wallpaper_enabled: u32::from(self.wallpaper_enabled),
            _pad: 0,
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        if primitives.is_empty() {
            return;
        }
        if primitives.len() > self.instance_capacity {
            let new_cap = primitives
                .len()
                .next_power_of_two()
                .max(INITIAL_INSTANCE_CAPACITY);
            self.instance_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("veilshell-sdf-instances"),
                size: (new_cap * std::mem::size_of::<GpuPrimitive>()) as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.instance_capacity = new_cap;
        }
        self.queue
            .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(primitives));
    }

    fn ensure_pipeline(&mut self, format: wgpu::TextureFormat) {
        if self.pipelines.contains_key(&format) {
            return;
        }
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("veilshell-sdf-pipeline"),
                layout: Some(&self.sdf_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.sdf_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[GpuPrimitive::layout()],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.sdf_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });
        self.pipelines.insert(format, pipeline);
    }

    fn record_sdf_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        instance_count: u32,
    ) {
        let pipeline = &self.pipelines[&format];
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("veilshell-sdf-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        if instance_count > 0 {
            rpass.set_pipeline(pipeline);
            rpass.set_bind_group(0, &self.sdf_bind_group, &[]);
            rpass.set_vertex_buffer(0, self.instance_buffer.slice(..));
            rpass.draw(0..6, 0..instance_count);
        }
    }

    fn ensure_offscreen_target(&mut self, width: u32, height: u32) {
        let matches = self
            .offscreen
            .as_ref()
            .is_some_and(|t| t.width == width && t.height == height);
        if matches {
            return;
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("veilshell-offscreen-color"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let unpadded = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded.div_ceil(align) * align;
        let buffer_size = u64::from(padded_bytes_per_row) * u64::from(height);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("veilshell-offscreen-readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        self.offscreen = Some(OffscreenTarget {
            width,
            height,
            texture,
            view,
            readback,
            padded_bytes_per_row,
        });
    }
}

impl RendererBackend for WgpuRenderer {
    fn name(&self) -> &'static str {
        "wgpu-sdf"
    }

    fn render_snapshot(
        &mut self,
        spec: &SnapshotSpec,
        palette: &ThemePalette,
    ) -> Result<RenderFrame, RenderError> {
        let (w, h) = spec.viewport.physical_pixels();
        if w == 0 || h == 0 || !spec.viewport.scale.is_finite() || spec.viewport.scale <= 0.0 {
            return Err(RenderError::InvalidViewport);
        }
        let list = build_scene(spec.scene, w as f32, h as f32, palette, &self.context);
        self.rasterize(w, h, &list)
    }
}

fn create_instance_and_adapter() -> Result<(wgpu::Instance, wgpu::Adapter), RenderError> {
    if let Some(env_backends) = wgpu::Backends::from_env().filter(|b| !b.is_empty()) {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: env_backends,
            ..Default::default()
        });
        let adapter = request_best_adapter(&instance)?;
        return Ok((instance, adapter));
    }

    // Initialize Vulkan first without touching GL: initializing `wgpu::Backends::GL` eagerly
    // on Wayland sessions with lazy XWayland (`DISPLAY=:0`) calls `XOpenDisplay(NULL)` inside
    // `wgpu-hal` and deadlocks waiting for an unstarted XWayland server.
    let vk_instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    match request_best_adapter(&vk_instance) {
        Ok(adapter) => Ok((vk_instance, adapter)),
        Err(vk_err) => {
            if std::env::var_os("WAYLAND_DISPLAY").is_some()
                && std::env::var_os("DISPLAY").is_some()
            {
                return Err(vk_err);
            }
            let gl_instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::GL,
                ..Default::default()
            });
            match request_best_adapter(&gl_instance) {
                Ok(adapter) => Ok((gl_instance, adapter)),
                Err(gl_err) => Err(RenderError::Gpu(format!(
                    "{vk_err}; GLES fallback also failed: {gl_err}"
                ))),
            }
        }
    }
}

fn request_best_adapter(instance: &wgpu::Instance) -> Result<wgpu::Adapter, RenderError> {
    let attempts = [
        wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        },
        wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        },
        wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            compatible_surface: None,
            force_fallback_adapter: true,
        },
    ];

    let mut last_err = None;
    for opts in &attempts {
        match pollster::block_on(instance.request_adapter(opts)) {
            Ok(adapter) => return Ok(adapter),
            Err(err) => last_err = Some(err),
        }
    }

    Err(RenderError::Gpu(format!(
        "no compatible Vulkan/GLES adapter found: {}",
        last_err.map_or_else(|| "unknown error".to_owned(), |e| e.to_string())
    )))
}

fn create_dummy_wallpaper_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("veilshell-wallpaper-dummy"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &[0, 0, 0, 255],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    texture
}

fn create_sdf_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    wallpaper_view: &wgpu::TextureView,
    wallpaper_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("veilshell-sdf-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(wallpaper_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(wallpaper_sampler),
            },
        ],
    })
}

#[inline]
const fn rgba_arr(c: Rgba) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

fn encode_draw_list(list: &DrawList, width: f32, height: f32) -> Vec<GpuPrimitive> {
    let mut out = Vec::with_capacity(list.commands.len() * 3);
    for cmd in &list.commands {
        match cmd {
            DrawCommand::BackdropGradient { top, bottom } => {
                out.push(GpuPrimitive {
                    rect: [0.0, 0.0, width, height],
                    color0: rgba_arr(*top),
                    color1: rgba_arr(*bottom),
                    params0: [0.0; 4],
                    params1: [0.0; 4],
                    meta: [KIND_BACKDROP, 0, 0, 0],
                });
            }
            DrawCommand::RadialGlow {
                cx,
                cy,
                radius,
                color,
            } => {
                if *radius > 1.0 && color.a > 0.001 {
                    out.push(GpuPrimitive {
                        rect: [*cx - *radius, *cy - *radius, *radius * 2.0, *radius * 2.0],
                        color0: rgba_arr(*color),
                        color1: [0.0; 4],
                        params0: [*cx, *cy, *radius, 0.0],
                        params1: [0.0; 4],
                        meta: [KIND_GLOW, 0, 0, 0],
                    });
                }
            }
            DrawCommand::Rect(rect) => push_rect_primitive(&mut out, *rect),
            DrawCommand::Text(label) => push_text_primitives(&mut out, label),
        }
    }
    out
}

fn push_rect_primitive(out: &mut Vec<GpuPrimitive>, rect: RoundedRect) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let pad = if rect.shadow_alpha > 0.001 && rect.shadow_radius > 0.5 {
        rect.shadow_radius + rect.shadow_offset_y.abs() + 2.0
    } else {
        2.0
    };
    out.push(GpuPrimitive {
        rect: [rect.x, rect.y, rect.w, rect.h],
        color0: rgba_arr(rect.fill),
        color1: rgba_arr(rect.border_color),
        params0: [
            rect.radius,
            rect.border_width,
            rect.top_highlight_alpha,
            pad,
        ],
        params1: [
            rect.shadow_radius,
            rect.shadow_offset_y,
            rect.shadow_alpha,
            0.0,
        ],
        meta: [KIND_RECT, 0, 0, 0],
    });
}

fn push_text_primitives(out: &mut Vec<GpuPrimitive>, label: &TextLabel) {
    if label.text.is_empty() || label.color.a <= 0.001 {
        return;
    }
    let scale = label.scale.max(1);
    let total_w = measure_text_width(&label.text, scale) as f32;
    let start_x = match label.align {
        TextAlign::Left => label.x,
        TextAlign::Center => label.x - total_w * 0.5,
        TextAlign::Right => label.x - total_w,
    }
    .round();
    let start_y = label.y.round();
    let advance = (GLYPH_ADVANCE * scale) as f32;
    let gw = (GLYPH_W * scale) as f32;
    let gh = (GLYPH_H * scale) as f32;
    let color = rgba_arr(label.color);

    for (idx, ch) in label.text.chars().enumerate() {
        let rows = glyph_rows(ch);
        if rows == [0; 7] {
            continue;
        }
        let bits_lo = u32::from(rows[0])
            | (u32::from(rows[1]) << 5)
            | (u32::from(rows[2]) << 10)
            | (u32::from(rows[3]) << 15);
        let bits_hi = u32::from(rows[4]) | (u32::from(rows[5]) << 5) | (u32::from(rows[6]) << 10);
        let gx = start_x + idx as f32 * advance;
        out.push(GpuPrimitive {
            rect: [gx, start_y, gw, gh],
            color0: color,
            color1: [0.0; 4],
            params0: [scale as f32, 0.0, 0.0, 0.0],
            params1: [0.0; 4],
            meta: [KIND_GLYPH, bits_lo, bits_hi, 0],
        });
    }
}

fn unpremultiply_padded_rows(
    src: &[u8],
    width: usize,
    height: usize,
    padded_bytes_per_row: usize,
) -> Vec<u8> {
    let mut out = vec![0_u8; width * height * 4];
    for y in 0..height {
        let src_row = &src[y * padded_bytes_per_row..y * padded_bytes_per_row + width * 4];
        let dst_row = &mut out[y * width * 4..(y + 1) * width * 4];
        for (sp, dp) in src_row.chunks_exact(4).zip(dst_row.chunks_exact_mut(4)) {
            let a = sp[3];
            if a == 0 {
                dp.fill(0);
            } else if a == 255 {
                dp.copy_from_slice(sp);
            } else {
                let a_u32 = u32::from(a);
                let half = a_u32 / 2;
                dp[0] = ((u32::from(sp[0]) * 255 + half) / a_u32).min(255) as u8;
                dp[1] = ((u32::from(sp[1]) * 255 + half) / a_u32).min(255) as u8;
                dp[2] = ((u32::from(sp[2]) * 255 + half) / a_u32).min(255) as u8;
                dp[3] = a;
            }
        }
    }
    out
}
