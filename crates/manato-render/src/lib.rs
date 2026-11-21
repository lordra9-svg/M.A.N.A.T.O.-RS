//! wgpu renderer for M.A.N.A.T.O.-RS.
//!
//! The game is rendered into a fixed 320x240 texture and then blitted to the
//! window with nearest-neighbour scaling, which keeps the original low
//! resolution pixel look and makes offscreen screenshots trivial.

#![allow(clippy::too_many_arguments)]

pub mod atlas;
pub mod draw;
pub mod fightscreen;
pub mod frontend;
pub mod model;
pub mod scene;
pub mod screenfont;
pub mod text;

pub use wgpu;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Context as _;
use bytemuck::cast_slice;
use manato_sim::SimState;
use wgpu::util::DeviceExt as _;

use atlas::{bank, SpriteAtlas};
use draw::{
    blend_state, create_blit_shader, create_sprite_shader, vertex_layout, Blend, Painter, TexRef,
};
use frontend::{FrontendDeps, OptionsView, SelectView, TitleView};
use scene::SceneDeps;
use screenfont::ScreenFont;
use text::FontAtlas;

pub const GAME_WIDTH: u32 = 320;
pub const GAME_HEIGHT: u32 = 240;

const GAME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.015,
    g: 0.015,
    b: 0.02,
    a: 1.0,
};

type AppliedPalette = (Option<(i32, i32)>, Option<Vec<[u8; 4]>>);

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,

    game_texture: wgpu::Texture,
    game_view: wgpu::TextureView,
    /// Previous frame copy bound as `bgl_RenderedTexture` for grab shaders.
    grab_texture: wgpu::Texture,
    grab_view: wgpu::TextureView,
    /// Current-frame scene copy sampled by model refraction.
    refraction_texture: wgpu::Texture,
    refraction_view: wgpu::TextureView,
    /// Clamped sampler for screen-space grab reads (model refraction).
    grab_sampler: wgpu::Sampler,
    depth_view: wgpu::TextureView,

    sprite_bgl: wgpu::BindGroupLayout,
    custom_bgl: wgpu::BindGroupLayout,
    custom_layout: wgpu::PipelineLayout,
    shader_sampler: wgpu::Sampler,
    /// GLSL vertex module paired with GLSL fragment shaders.
    glsl_vertex_module: wgpu::ShaderModule,
    pipelines: [wgpu::RenderPipeline; 3],
    pipelines_mask: [wgpu::RenderPipeline; 3],

    atlas: SpriteAtlas,
    font: FontAtlas,
    screen_fonts: std::collections::BTreeMap<u32, ScreenFont>,
    /// Last palette remap applied to each character sprite bank.
    applied_palettes: std::collections::HashMap<u8, AppliedPalette>,
    /// Compiled `ShaderSet` pipelines (WGSL files containing both stages).
    custom_shaders: HashMap<String, [wgpu::RenderPipeline; 3]>,
    /// Shader names that failed to compile or use unsupported GLSL.
    failed_shaders: HashSet<String>,
    /// Seconds since the match started, fed to custom shader uniforms.
    shader_time: f32,
    /// Optional diagnostic line drawn over the match (spectator/broadcast status).
    status_line: Option<String>,
    /// Signature of in-match storyboard overlays baked into the atlas.
    overlay_signature: Vec<(usize, String)>,
    /// Atlas bank for each `(overlay storyboard pointer, scene index)` bg.
    overlay_bg_banks: HashMap<(usize, usize), u8>,
    /// Atlas banks holding per-character palette recolors for projectiles.
    projectile_banks: HashMap<(usize, (i32, i32)), u8>,
    /// Recolored sprite files backing `projectile_banks`, retained so overlay
    /// atlas rebuilds keep the palette banks resident.
    projectile_palette_files: Vec<(u8, manato_formats::SffFile)>,
    model_pipeline: wgpu::RenderPipeline,
    model_bgl: wgpu::BindGroupLayout,
    model_sampler: wgpu::Sampler,
    model_white_view: wgpu::TextureView,
    model_lut_view: wgpu::TextureView,
    model_lut_sampler: wgpu::Sampler,
    model_draws: Vec<(wgpu::Buffer, u32, wgpu::BindGroup)>,
    model_asset: Option<model::ModelAsset>,
    model_time: f32,
    /// Standalone textures for `ShaderSet tex1`/`tex2` sprite references.
    shader_textures: HashMap<(u8, i32, i32), wgpu::TextureView>,
    solid: wgpu::BindGroup,
    shadow: wgpu::BindGroup,

    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group: wgpu::BindGroup,
    blit_uniform: wgpu::Buffer,

    vertex_buffer: wgpu::Buffer,
    vertex_capacity: u64,
    painter: Painter,
}

fn create_glsl_vertex_shader(device: &wgpu::Device) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("manato GLSL vertex shader"),
        source: wgpu::ShaderSource::Glsl {
            shader: r#"#version 450
layout(location = 0) in vec2 position;
layout(location = 1) in vec2 uv_in;
layout(location = 2) in vec4 color_in;
layout(location = 3) in float z;
layout(location = 0) out vec2 uv;
layout(location = 1) out vec4 color;
void main() {
    gl_Position = vec4(
        position.x / 320.0 * 2.0 - 1.0,
        1.0 - position.y / 240.0 * 2.0,
        z,
        1.0
    );
    uv = uv_in;
    color = color_in;
}
"#
            .into(),
            stage: wgpu::naga::ShaderStage::Vertex,
            defines: &[],
        },
    })
}

impl Renderer {
    /// Creates a renderer without a window surface. Used for screenshots and
    /// tests.
    pub fn new_offscreen() -> anyhow::Result<Self> {
        Self::new_offscreen_with_backend(wgpu::Backends::PRIMARY)
    }

    /// Creates an offscreen renderer with an explicit backend mask.
    pub fn new_offscreen_with_backend(backends: wgpu::Backends) -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        Self::new_with_instance(instance, None, GAME_FORMAT)
    }

    /// Creates a renderer that can present to `target_format`.
    pub fn new_for_surface(
        instance: wgpu::Instance,
        surface: &wgpu::Surface<'_>,
        target_format: wgpu::TextureFormat,
    ) -> anyhow::Result<Self> {
        Self::new_with_instance(instance, Some(surface), target_format)
    }

    fn new_with_instance(
        instance: wgpu::Instance,
        compatible_surface: Option<&wgpu::Surface<'_>>,
        target_format: wgpu::TextureFormat,
    ) -> anyhow::Result<Self> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface,
            force_fallback_adapter: false,
        }))
        .context("no suitable GPU adapter found")?;
        let info = adapter.get_info();
        log::info!(
            "GPU adapter: {} (backend {:?}, type {:?})",
            info.name,
            info.backend,
            info.device_type
        );

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("manato device"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .context("failed to create GPU device")?;

        let sprite_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("manato sprite bind group layout"),
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
            ],
        });

        let sprite_shader = create_sprite_shader(&device, false);
        let mask_shader = create_sprite_shader(&device, true);
        let sprite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("manato sprite pipeline layout"),
            bind_group_layouts: &[&sprite_bgl],
            push_constant_ranges: &[],
        });
        // Custom shaders additionally expose `shaderparam`, `pal`, `tex1`,
        // and `tex2`. Textures currently alias the sprite atlas page.
        let custom_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("manato custom shader bind group layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let custom_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("manato custom shader layout"),
            bind_group_layouts: &[&custom_bgl],
            push_constant_ranges: &[],
        });
        let model_bgl = model::create_bind_group_layout(&device);
        let model_pipeline = model::create_pipeline(&device, GAME_FORMAT, &model_bgl);
        let (model_lut_view, model_lut_sampler) = model::create_brdf_lut_texture(&device, &queue);
        let model_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("manato model sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let grab_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("manato grab sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let white_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("manato model white"),
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
                texture: &white_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255u8, 255, 255, 255],
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
        let model_white_view = white_texture.create_view(&Default::default());
        let shader_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("manato shader sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let make_pipeline = |blend: Blend, shader: &wgpu::ShaderModule| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("manato sprite pipeline"),
                layout: Some(&sprite_layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: &[vertex_layout()],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: GAME_FORMAT,
                        blend: Some(blend_state(blend)),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        let pipelines = [
            make_pipeline(Blend::Normal, &sprite_shader),
            make_pipeline(Blend::Add, &sprite_shader),
            make_pipeline(Blend::Sub, &sprite_shader),
        ];
        let pipelines_mask = [
            make_pipeline(Blend::Normal, &mask_shader),
            make_pipeline(Blend::Add, &mask_shader),
            make_pipeline(Blend::Sub, &mask_shader),
        ];

        let atlas = SpriteAtlas::build(&device, &queue, &sprite_bgl, &[]);

        // The atlas needs the loaded content, which is only available once the
        // match exists; the renderer therefore rebuilds it lazily.
        let font = FontAtlas::build(&device, &queue, &sprite_bgl);

        // 1x1 white texture for solid rectangles.
        let solid_texture = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("manato solid"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: GAME_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[255, 255, 255, 255],
        );
        let solid = make_texture_bind_group(
            &device,
            &sprite_bgl,
            &solid_texture,
            "manato solid bind group",
        );

        let shadow_texture = create_shadow_texture(&device, &queue);
        let shadow = make_texture_bind_group(
            &device,
            &sprite_bgl,
            &shadow_texture,
            "manato shadow bind group",
        );

        let game_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("manato game target"),
            size: wgpu::Extent3d {
                width: GAME_WIDTH,
                height: GAME_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: GAME_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let game_view = game_texture.create_view(&Default::default());
        let grab_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("manato grab target"),
            size: wgpu::Extent3d {
                width: GAME_WIDTH,
                height: GAME_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: GAME_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let grab_view = grab_texture.create_view(&Default::default());
        let refraction_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("manato refraction target"),
            size: wgpu::Extent3d {
                width: GAME_WIDTH,
                height: GAME_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: GAME_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let refraction_view = refraction_texture.create_view(&Default::default());
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &grab_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &vec![0u8; (GAME_WIDTH * GAME_HEIGHT * 4) as usize],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(GAME_WIDTH * 4),
                rows_per_image: Some(GAME_HEIGHT),
            },
            wgpu::Extent3d {
                width: GAME_WIDTH,
                height: GAME_HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("manato depth target"),
            size: wgpu::Extent3d {
                width: GAME_WIDTH,
                height: GAME_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_texture.create_view(&Default::default());

        // Blit resources.
        let blit_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("manato blit bind group layout"),
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
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let blit_shader = create_blit_shader(&device);
        let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("manato blit pipeline layout"),
            bind_group_layouts: &[&blit_bgl],
            push_constant_ranges: &[],
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("manato blit pipeline"),
            layout: Some(&blit_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("manato blit sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let blit_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("manato blit uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("manato blit bind group"),
            layout: &blit_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&game_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&blit_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: blit_uniform.as_entire_binding(),
                },
            ],
        });

        let vertex_capacity = 4096 * std::mem::size_of::<draw::Vertex>() as u64;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("manato vertex buffer"),
            size: vertex_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let glsl_vertex_module = create_glsl_vertex_shader(&device);
        Ok(Self {
            device,
            queue,
            game_texture,
            game_view,
            grab_texture,
            grab_view,
            refraction_texture,
            refraction_view,
            grab_sampler,
            depth_view,
            sprite_bgl,
            custom_bgl,
            custom_layout,
            shader_sampler,
            glsl_vertex_module,
            pipelines,
            pipelines_mask,
            atlas,
            font,
            screen_fonts: std::collections::BTreeMap::new(),
            applied_palettes: std::collections::HashMap::new(),
            custom_shaders: HashMap::new(),
            failed_shaders: HashSet::new(),
            shader_time: 0.0,
            status_line: None,
            overlay_signature: Vec::new(),
            overlay_bg_banks: HashMap::new(),
            projectile_banks: HashMap::new(),
            projectile_palette_files: Vec::new(),
            shader_textures: HashMap::new(),
            model_pipeline,
            model_bgl,
            model_sampler,
            model_white_view,
            model_lut_view,
            model_lut_sampler,
            model_draws: Vec::new(),
            model_asset: None,
            model_time: 0.0,
            solid,
            shadow,
            blit_pipeline,
            blit_bind_group,
            blit_uniform,
            vertex_buffer,
            vertex_capacity,
            painter: Painter::default(),
        })
    }

    /// Rebuilds the sprite atlas from the content in `sim`. Call once after
    /// loading a match (or when the assets change).
    pub fn load_assets(&mut self, sim: &SimState) {
        let assets = &sim.assets;
        // Materialize per-character palette copies for projectile ownpal/remappal.
        // The region is shared by every character/palette combo and only falls
        // back to the owner bank once all 64 slots are occupied.
        let mut palettes: HashMap<(usize, (i32, i32)), u8> = HashMap::new();
        let mut palette_files: Vec<(u8, manato_formats::SffFile)> = Vec::new();
        let mut next_bank = bank::PROJECTILE_PALETTE_BASE as u16;
        let limit = bank::PROJECTILE_PALETTE_BASE as u16 + bank::PROJECTILE_PALETTE_COUNT as u16;
        for (i, character) in assets.characters.iter().enumerate() {
            let mut keys: Vec<(i32, i32)> = character.palettes.keys().copied().collect();
            keys.sort();
            for key in keys {
                if next_bank >= limit {
                    log::warn!(
                        "projectile palette banks exhausted at {} combos; falling back to owner banks",
                        palette_files.len()
                    );
                    break;
                }
                let palette = &character.palettes[&key];
                let mut file = character.sprites.clone();
                for sprite in &mut file.sprites {
                    if let Some(rgba) = sprite.recolor_rgba(palette) {
                        sprite.rgba = rgba;
                    }
                }
                palettes.insert((i, key), next_bank as u8);
                palette_files.push((next_bank as u8, file));
                next_bank += 1;
            }
        }
        self.projectile_banks = palettes;
        self.projectile_palette_files = palette_files.clone();
        let mut banks: Vec<(u8, &manato_formats::SffFile)> =
            vec![(bank::STAGE, &assets.stage.sprites)];
        for (i, character) in assets.characters.iter().enumerate() {
            banks.push((bank::CHARACTER_BASE + i as u8, &character.sprites));
        }
        banks.push((bank::EFFECTS, &assets.effects.sprites));
        if let Some(screenpack) = &assets.screenpack {
            banks.push((bank::SCREENPACK, &screenpack.sprites));
        }
        for (index, file) in &palette_files {
            banks.push((*index, file));
        }
        self.atlas = SpriteAtlas::build(&self.device, &self.queue, &self.sprite_bgl, &banks);
        self.overlay_signature.clear();
        self.overlay_bg_banks.clear();
        self.shader_textures.clear();
        self.clear_model();
        if let Some(path) = &assets.stage.model {
            self.load_model(
                path,
                assets.stage.model_anim.as_deref(),
                assets.stage.model_env.as_deref(),
                &assets.stage.model_lights,
            );
        }

        self.screen_fonts.clear();
        if let Some(screenpack) = &assets.screenpack {
            self.load_screen_fonts(screenpack);
        }
        log::info!(
            "sprite atlas: {} pages of {}px",
            self.atlas.pages.len(),
            self.atlas.page_size
        );
    }

    /// Builds the front-end atlas from the screenpack alone. Used before a
    /// match exists, when only title/select art is needed.
    pub fn load_frontend(
        &mut self,
        screenpack: Option<&manato_formats::Screenpack>,
        faces: &[manato_formats::SffFile],
    ) {
        let mut banks: Vec<(u8, &manato_formats::SffFile)> = Vec::new();
        if let Some(screenpack) = screenpack {
            banks.push((bank::SCREENPACK, &screenpack.sprites));
        }
        for (index, sff) in faces.iter().enumerate() {
            if index < 32 {
                banks.push((bank::CHARACTER_BASE + index as u8, sff));
            }
        }
        self.atlas = SpriteAtlas::build(&self.device, &self.queue, &self.sprite_bgl, &banks);
        self.screen_fonts.clear();
        if let Some(screenpack) = screenpack {
            self.load_screen_fonts(screenpack);
        }
    }

    fn load_screen_fonts(&mut self, screenpack: &manato_formats::Screenpack) {
        for (index, font) in &screenpack.loaded_fonts {
            let tint = screenpack
                .fonts
                .get(index)
                .map(|decl| decl.tint)
                .unwrap_or([255, 255, 255]);
            self.screen_fonts.insert(
                *index,
                ScreenFont::build(
                    &self.device,
                    &self.queue,
                    &self.sprite_bgl,
                    *index,
                    font,
                    tint,
                ),
            );
        }
    }

    /// Number of atlas pages currently allocated (diagnostics).
    pub fn atlas_pages(&self) -> usize {
        self.atlas.pages.len()
    }

    /// Recolors character sprite banks when `RemapPal` changes. Only the
    /// affected bank is re-uploaded, and clearing the remap restores the
    /// original pixels.
    fn apply_palette_remaps(&mut self, sim: &SimState) {
        use crate::atlas::bank;
        let mut desired: std::collections::HashMap<u8, AppliedPalette> =
            std::collections::HashMap::new();
        for fighter in &sim.fighters {
            let bank = bank::CHARACTER_BASE + fighter.char_id as u8;
            let entry = desired.entry(bank).or_insert((None, None));
            if let Some(palette) = &fighter.remap_palette {
                *entry = (fighter.pal_remap_source, Some(palette.clone()));
            }
        }
        for (bank, (source, palette)) in desired {
            if self.applied_palettes.get(&bank) == Some(&(source, palette.clone())) {
                continue;
            }
            let Some(char_id) = (bank.checked_sub(bank::CHARACTER_BASE)).map(|id| id as usize)
            else {
                continue;
            };
            let Some(character) = sim.assets.characters.get(char_id) else {
                continue;
            };
            let was_remapped = self.applied_palettes.contains_key(&bank);
            if was_remapped || palette.is_some() {
                // Restore originals first so a changed source filter cannot
                // leave stale recolored sprites behind.
                for sprite in &character.sprites.sprites {
                    if !sprite.indices.is_empty() {
                        self.atlas.update_sprite(
                            &self.queue,
                            bank,
                            sprite.group,
                            sprite.index,
                            sprite.width,
                            sprite.height,
                            &sprite.rgba,
                        );
                    }
                }
            }
            if let Some(palette) = &palette {
                for sprite in &character.sprites.sprites {
                    if let Some(source) = source {
                        if sprite.palette_key != Some(source) {
                            continue;
                        }
                    }
                    if let Some(rgba) = sprite.recolor_rgba(palette) {
                        self.atlas.update_sprite(
                            &self.queue,
                            bank,
                            sprite.group,
                            sprite.index,
                            sprite.width,
                            sprite.height,
                            &rgba,
                        );
                    }
                }
            }
            self.applied_palettes.insert(bank, (source, palette));
        }
    }

    /// Compiles a `ShaderSet` shader from a WGSL source string. The file must
    /// define `vs_main`/`fs_main` matching the engine's sprite vertex layout
    /// (position, uv, color, z) and binding 0/1 texture/sampler.
    pub fn register_shader(&mut self, name: &str, source: &str) -> bool {
        self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("manato custom shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let make = |blend: Blend| {
            self.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("manato custom shader pipeline"),
                    layout: Some(&self.custom_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs_main"),
                        buffers: &[vertex_layout()],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs_main"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: GAME_FORMAT,
                            blend: Some(blend_state(blend)),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: true,
                        depth_compare: wgpu::CompareFunction::LessEqual,
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                    cache: None,
                })
        };
        let pipelines = [make(Blend::Normal), make(Blend::Add), make(Blend::Sub)];
        self.custom_shaders.insert(name.to_string(), pipelines);
        true
    }

    /// Compiles a GLSL fragment shader against the sprite vertex module. The
    /// fragment must declare `layout(binding = 0) uniform texture2D tex`,
    /// `layout(binding = 1) uniform sampler samp`, inputs `uv` (location 0)
    /// and `color` (location 1), and a colour output at location 0.
    pub fn register_glsl_shader(&mut self, name: &str, source: &str) -> bool {
        let source = preprocess_implicit_glsl(source);
        self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let fragment = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("manato GLSL custom shader"),
                source: wgpu::ShaderSource::Glsl {
                    shader: source.into(),
                    stage: wgpu::naga::ShaderStage::Fragment,
                    defines: &[],
                },
            });
        let make = |blend: Blend| {
            self.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("manato GLSL custom pipeline"),
                    layout: Some(&self.custom_layout),
                    vertex: wgpu::VertexState {
                        module: &self.glsl_vertex_module,
                        entry_point: Some("main"),
                        buffers: &[vertex_layout()],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &fragment,
                        entry_point: Some("main"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: GAME_FORMAT,
                            blend: Some(blend_state(blend)),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: true,
                        depth_compare: wgpu::CompareFunction::LessEqual,
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                    cache: None,
                })
        };
        let pipelines = [make(Blend::Normal), make(Blend::Add), make(Blend::Sub)];
        if let Some(error) = pollster::block_on(self.device.pop_error_scope()) {
            log::warn!("GLSL shader {name} failed to compile: {error}");
            return false;
        }
        self.custom_shaders.insert(name.to_string(), pipelines);
        true
    }

    /// Loads and compiles one `ShaderSet` name on first use. Only WGSL files
    /// are executed; GLSL content shaders are reported once and fall back to
    /// the default pipeline.
    fn ensure_custom_shader(&mut self, sim: &SimState, name: &str) {
        if self.custom_shaders.contains_key(name) || self.failed_shaders.contains(name) {
            return;
        }
        let path = sim
            .assets
            .characters
            .iter()
            .find_map(|character| character.def.shaders.get(name).cloned());
        let Some(path) = path else {
            self.failed_shaders.insert(name.to_string());
            return;
        };
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                log::warn!("custom shader {}: {error}", path.display());
                self.failed_shaders.insert(name.to_string());
                return;
            }
        };
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let compiled = match extension.as_str() {
            "wgsl" => self.register_shader(name, &source),
            "glsl" | "frag" | "fs" => self.register_glsl_shader(name, &source),
            _ => {
                log::warn!("custom shader {} is not WGSL or GLSL", path.display());
                false
            }
        };
        if !compiled {
            self.failed_shaders.insert(name.to_string());
        }
    }

    /// Draws one frame into the internal 320x240 target.
    fn overlay_signature(sim: &SimState) -> Vec<(usize, String)> {
        sim.storyboards
            .iter()
            .map(|overlay| {
                (
                    std::sync::Arc::as_ptr(&overlay.storyboard) as usize,
                    overlay.storyboard.path.to_string_lossy().into_owned(),
                )
            })
            .collect()
    }

    /// Rebuilds the atlas with one bank per in-match storyboard overlay.
    fn rebuild_overlay_atlas(&mut self, sim: &SimState) {
        let assets = &sim.assets;
        let mut banks: Vec<(u8, &manato_formats::SffFile)> = vec![
            (bank::STAGE, &assets.stage.sprites),
            (bank::EFFECTS, &assets.effects.sprites),
        ];
        for (index, character) in assets.characters.iter().enumerate() {
            let id = bank::CHARACTER_BASE as usize + index;
            if id < bank::OVERLAY_BASE as usize {
                banks.push((id as u8, &character.sprites));
            }
        }
        if let Some(screenpack) = &assets.screenpack {
            banks.push((bank::SCREENPACK, &screenpack.sprites));
        }
        for (index, file) in &self.projectile_palette_files {
            banks.push((*index, file));
        }
        for (index, overlay) in sim.storyboards.iter().enumerate() {
            let id = bank::OVERLAY_BASE as usize + index;
            if id < bank::OVERLAY_END as usize {
                banks.push((id as u8, &overlay.storyboard.sprites));
            } else if index == (bank::OVERLAY_END - bank::OVERLAY_BASE) as usize {
                log::warn!(
                    "storyboard overlay sprite banks exhausted; extra overlays are not baked"
                );
            }
        }
        self.overlay_bg_banks.clear();
        let mut next_bg = bank::OVERLAY_BG_BASE as usize;
        for overlay in &sim.storyboards {
            let key = std::sync::Arc::as_ptr(&overlay.storyboard) as usize;
            for (scene_index, scene) in overlay.storyboard.scenes.iter().enumerate() {
                let Some(stage) = &scene.bg else {
                    continue;
                };
                if next_bg >= bank::OVERLAY_BG_END as usize {
                    log::warn!("storyboard overlay background banks exhausted");
                    break;
                }
                banks.push((next_bg as u8, &stage.sprites));
                self.overlay_bg_banks
                    .insert((key, scene_index), next_bg as u8);
                next_bg += 1;
            }
        }
        self.atlas = SpriteAtlas::build(&self.device, &self.queue, &self.sprite_bgl, &banks);
    }

    /// Sets a short status line drawn in the top-left corner, used by the
    /// spectator and host broadcast modes.
    pub fn set_status_line(&mut self, line: Option<String>) {
        self.status_line = line;
    }

    pub fn render(&mut self, sim: &SimState) {
        let signature = Self::overlay_signature(sim);
        if signature != self.overlay_signature {
            self.rebuild_overlay_atlas(sim);
            self.overlay_signature = signature;
        }
        self.apply_palette_remaps(sim);
        self.shader_time = sim.match_time as f32 / 60.0;
        self.update_model(sim.match_time as f32 / 60.0, &sim.model_nodes);
        for fighter in &sim.fighters {
            if fighter.shader.is_none() {
                continue;
            }
            let bank = bank::CHARACTER_BASE + fighter.char_id as u8;
            for (group, index) in [fighter.shader_tex1, fighter.shader_tex2]
                .into_iter()
                .flatten()
            {
                self.ensure_shader_texture(sim, (bank, group, index));
            }
        }
        let deps = SceneDeps {
            atlas: &self.atlas,
            font: &self.font,
            screen_fonts: &self.screen_fonts,
            overlay_bg_banks: &self.overlay_bg_banks,
            projectile_banks: &self.projectile_banks,
        };
        let hud_start = scene::build(&mut self.painter, sim, &deps);
        if let Some(line) = self.status_line.as_deref() {
            self.painter
                .text(&self.font, line, 4.0, 4.0, 1.0, [1.0, 1.0, 1.0, 0.9]);
        }
        // Compile any selected custom shaders before the render pass.
        let mut names: Vec<String> = self
            .painter
            .ranges
            .iter()
            .filter_map(|range| range.shader.clone())
            .collect();
        names.sort();
        names.dedup();
        for name in names {
            self.ensure_custom_shader(sim, &name);
        }
        self.present(hud_start);
    }

    /// Loads and bakes a static glTF model for the current scene/stage.
    pub fn load_model(
        &mut self,
        path: &Path,
        clip: Option<&str>,
        env: Option<&Path>,
        lights: &[manato_formats::stage::ModelLightDef],
    ) {
        match model::load(path) {
            Ok(mut asset) => {
                asset.set_lights(lights);
                if let Some(env) = env {
                    if let Err(error) = asset.set_environment(env) {
                        log::warn!("model env {}: {error:#}", env.display());
                    }
                }
                if let Some(clip) = clip {
                    if !asset.select_clip(clip) {
                        log::warn!("model {}: unknown animation clip {clip}", path.display());
                    }
                }
                self.model_draws.clear();
                let samples = asset.sample(0.0, &Default::default());
                for (draw, vertices) in asset.draws.iter().zip(samples.iter()) {
                    let view = match &draw.texture {
                        Some(texture) if texture.width > 0 && texture.height > 0 => {
                            let gpu_texture =
                                self.device.create_texture(&wgpu::TextureDescriptor {
                                    label: Some("manato model base color"),
                                    size: wgpu::Extent3d {
                                        width: texture.width,
                                        height: texture.height,
                                        depth_or_array_layers: 1,
                                    },
                                    mip_level_count: 1,
                                    sample_count: 1,
                                    dimension: wgpu::TextureDimension::D2,
                                    format: wgpu::TextureFormat::Rgba8Unorm,
                                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                                        | wgpu::TextureUsages::COPY_DST,
                                    view_formats: &[],
                                });
                            self.queue.write_texture(
                                wgpu::TexelCopyTextureInfo {
                                    texture: &gpu_texture,
                                    mip_level: 0,
                                    origin: wgpu::Origin3d::ZERO,
                                    aspect: wgpu::TextureAspect::All,
                                },
                                &texture.rgba,
                                wgpu::TexelCopyBufferLayout {
                                    offset: 0,
                                    bytes_per_row: Some(texture.width * 4),
                                    rows_per_image: Some(texture.height),
                                },
                                wgpu::Extent3d {
                                    width: texture.width,
                                    height: texture.height,
                                    depth_or_array_layers: 1,
                                },
                            );
                            gpu_texture.create_view(&Default::default())
                        }
                        _ => self.model_white_view.clone(),
                    };
                    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("manato model material"),
                        layout: &self.model_bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.model_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(&self.model_lut_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::Sampler(&self.model_lut_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::TextureView(&self.refraction_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 5,
                                resource: wgpu::BindingResource::Sampler(&self.grab_sampler),
                            },
                        ],
                    });
                    self.model_draws.push((
                        model::upload_model(&self.device, vertices),
                        vertices.len() as u32,
                        bind_group,
                    ));
                }
                self.model_asset = Some(asset);
                self.model_time = 0.0;
            }
            Err(error) => {
                log::warn!("model {}: {error:#}", path.display());
                self.clear_model();
            }
        }
    }

    fn update_model(
        &mut self,
        time: f32,
        node_overrides: &std::collections::HashMap<i32, (f32, f32, f32)>,
    ) {
        let Some(asset) = &self.model_asset else {
            return;
        };
        if (time - self.model_time).abs() < f32::EPSILON {
            return;
        }
        self.model_time = time;
        let samples = asset.sample(time, node_overrides);
        for ((buffer, count, _), vertices) in self.model_draws.iter().zip(samples.iter()) {
            if vertices.len() as u32 == *count {
                self.queue
                    .write_buffer(buffer, 0, bytemuck::cast_slice(vertices));
            }
        }
    }

    pub fn clear_model(&mut self) {
        self.model_draws.clear();
        self.model_asset = None;
    }

    /// Builds the atlas from a storyboard's sprite file.
    pub fn load_storyboard(&mut self, storyboard: &manato_formats::storyboard::Storyboard) {
        let mut banks: Vec<(u8, &manato_formats::SffFile)> =
            vec![(bank::STORYBOARD, &storyboard.sprites)];
        for (index, scene) in storyboard.scenes.iter().enumerate() {
            if let Some(stage) = &scene.bg {
                let id = bank::STORYBOARD_BG as usize + index;
                if id <= u8::MAX as usize {
                    banks.push((id as u8, &stage.sprites));
                }
            }
        }
        self.atlas = SpriteAtlas::build(&self.device, &self.queue, &self.sprite_bgl, &banks);
        self.clear_model();
        if let Some(path) = &storyboard.model {
            self.load_model(
                path,
                storyboard.model_anim.as_deref(),
                storyboard.model_env.as_deref(),
                &storyboard.model_lights,
            );
        }
        self.screen_fonts.clear();
        for (index, path) in &storyboard.font_files {
            match manato_formats::font::BitmapFont::load(path) {
                Ok(font) => {
                    self.screen_fonts.insert(
                        *index as u32,
                        ScreenFont::build(
                            &self.device,
                            &self.queue,
                            &self.sprite_bgl,
                            *index as u32,
                            &font,
                            [255, 255, 255],
                        ),
                    );
                }
                Err(error) => log::warn!("storyboard font {} failed: {error}", path.display()),
            }
        }
    }

    /// Draws one storyboard frame into the internal 320x240 target.
    pub fn render_storyboard(
        &mut self,
        storyboard: &manato_formats::storyboard::Storyboard,
        player: &manato_formats::storyboard::StoryboardPlayer,
    ) {
        self.update_model(player.time as f32 / 60.0, &Default::default());
        let deps = SceneDeps {
            atlas: &self.atlas,
            font: &self.font,
            screen_fonts: &self.screen_fonts,
            overlay_bg_banks: &self.overlay_bg_banks,
            projectile_banks: &self.projectile_banks,
        };
        scene::draw_storyboard(&mut self.painter, &deps, storyboard, player);
        self.present(usize::MAX);
    }

    /// Draws the high-score table.
    pub fn render_scores(&mut self, scores: &[u64]) {
        self.painter.clear();
        self.painter.set_depth(1.0);
        self.painter.rect(
            Blend::Normal,
            0.0,
            0.0,
            GAME_WIDTH as f32,
            GAME_HEIGHT as f32,
            [0.02, 0.02, 0.05, 1.0],
        );
        self.painter.text(
            &self.font,
            "HIGH SCORES",
            104.0,
            50.0,
            2.0,
            [1.0, 0.9, 0.35, 1.0],
        );
        if scores.is_empty() {
            self.painter.text(
                &self.font,
                "NO SCORES YET",
                110.0,
                120.0,
                1.0,
                [0.7, 0.7, 0.7, 1.0],
            );
        }
        for (index, score) in scores.iter().take(5).enumerate() {
            let y = 84.0 + index as f32 * 24.0;
            self.painter.text(
                &self.font,
                &format!("{}. {score}", index + 1),
                120.0,
                y,
                1.5,
                [1.0, 1.0, 1.0, 1.0],
            );
        }
        self.painter.text(
            &self.font,
            "PRESS START",
            108.0,
            216.0,
            1.0,
            [0.7, 0.7, 0.7, 1.0],
        );
        self.present(usize::MAX);
    }

    /// Draws the arcade continue screen (countdown, credits, score).
    pub fn render_continue(&mut self, credits: u32, seconds: i32, score: u64) {
        self.painter.clear();
        self.painter.set_depth(1.0);
        self.painter.rect(
            Blend::Normal,
            0.0,
            0.0,
            GAME_WIDTH as f32,
            GAME_HEIGHT as f32,
            [0.02, 0.02, 0.05, 1.0],
        );
        let white = [1.0, 1.0, 1.0, 1.0];
        let yellow = [1.0, 0.9, 0.35, 1.0];
        self.painter
            .text(&self.font, "CONTINUE?", 96.0, 86.0, 2.0, white);
        self.painter.text(
            &self.font,
            &format!("{seconds:02}"),
            148.0,
            120.0,
            3.0,
            yellow,
        );
        self.painter.text(
            &self.font,
            &format!("CREDITS {credits}"),
            96.0,
            176.0,
            1.0,
            white,
        );
        self.painter.text(
            &self.font,
            &format!("SCORE {score}"),
            96.0,
            192.0,
            1.0,
            white,
        );
        self.painter.text(
            &self.font,
            "PRESS START",
            108.0,
            216.0,
            1.0,
            [0.7, 0.7, 0.7, 1.0],
        );
        self.present(usize::MAX);
    }

    /// Draws a front-end title or select frame into the internal target.
    pub fn render_title(
        &mut self,
        screenpack: &manato_formats::Screenpack,
        states: &[manato_sim::StageLayerState],
        view: &TitleView,
    ) {
        let deps = FrontendDeps {
            atlas: &self.atlas,
            font: &self.font,
            screen_fonts: &self.screen_fonts,
        };
        self.painter.clear();
        frontend::draw_title(&mut self.painter, &deps, screenpack, states, view);
        self.present(usize::MAX);
    }

    /// Draws a front-end select frame into the internal target.
    pub fn render_select(
        &mut self,
        screenpack: &manato_formats::Screenpack,
        states: &[manato_sim::StageLayerState],
        view: &SelectView,
    ) {
        let deps = FrontendDeps {
            atlas: &self.atlas,
            font: &self.font,
            screen_fonts: &self.screen_fonts,
        };
        self.painter.clear();
        frontend::draw_select(&mut self.painter, &deps, screenpack, states, view);
        self.present(usize::MAX);
    }

    /// Draws the built-in options overlay on top of the title background.
    pub fn render_options(
        &mut self,
        screenpack: &manato_formats::Screenpack,
        states: &[manato_sim::StageLayerState],
        view: &OptionsView,
    ) {
        let deps = FrontendDeps {
            atlas: &self.atlas,
            font: &self.font,
            screen_fonts: &self.screen_fonts,
        };
        self.painter.clear();
        frontend::draw_options(&mut self.painter, &deps, screenpack, states, view);
        self.present(usize::MAX);
    }

    fn ensure_shader_texture(&mut self, sim: &SimState, key: (u8, i32, i32)) {
        if self.shader_textures.contains_key(&key) {
            return;
        }
        let (bank, group, index) = key;
        let Some(char_index) = (bank as usize).checked_sub(bank::CHARACTER_BASE as usize) else {
            return;
        };
        let Some(sprite) = sim
            .assets
            .characters
            .get(char_index)
            .and_then(|character| character.sprites.get(group, index))
        else {
            return;
        };
        if sprite.width == 0 || sprite.height == 0 {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("manato shader reference texture"),
            size: wgpu::Extent3d {
                width: sprite.width,
                height: sprite.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        if sprite.rgba.len() == (sprite.width * sprite.height * 4) as usize {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &sprite.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(sprite.width * 4),
                    rows_per_image: Some(sprite.height),
                },
                wgpu::Extent3d {
                    width: sprite.width,
                    height: sprite.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.shader_textures
            .insert(key, texture.create_view(&Default::default()));
    }

    /// Uploads the accumulated vertices and records the frame commands.
    fn present(&mut self, hud_start: usize) {
        let bytes = cast_slice(&self.painter.vertices);
        if bytes.len() as u64 > self.vertex_capacity {
            let size = (bytes.len() as u64).next_power_of_two().max(4096);
            self.vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("manato vertex buffer"),
                size,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.vertex_capacity = size;
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&self.vertex_buffer, 0, bytes);
        }

        let ranges = std::mem::take(&mut self.painter.ranges);
        // Build per-range custom bind groups (shaderparam + aliased textures).
        let mut custom_bindings: Vec<Option<(wgpu::Buffer, wgpu::BindGroup)>> =
            Vec::with_capacity(ranges.len());
        for range in &ranges {
            let binding = (|| {
                if range.mask {
                    return None;
                }
                let name = range.shader.as_ref()?;
                if !self.custom_shaders.contains_key(name) {
                    return None;
                }
                let TexRef::Atlas(page) = range.tex else {
                    return None;
                };
                let view = self.atlas.page_view(page)?;
                let palette_view = range
                    .palette
                    .and_then(|(bank, key)| self.atlas.palette_view(bank, key))
                    .unwrap_or(view);
                let tex1_view = range
                    .shader_tex1
                    .and_then(|key| self.shader_textures.get(&key))
                    .unwrap_or(view);
                let tex2_view = range
                    .shader_tex2
                    .and_then(|key| self.shader_textures.get(&key))
                    .unwrap_or(view);
                let mut data = [0f32; 20];
                if let Some(params) = range.shader_params {
                    data[..16].copy_from_slice(&params);
                }
                data[16] = self.shader_time;
                data[17] = GAME_WIDTH as f32;
                data[18] = GAME_HEIGHT as f32;
                data[19] = GAME_WIDTH as f32 / GAME_HEIGHT as f32;
                let buffer = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("manato shader params"),
                        contents: cast_slice(&data),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let entries = [
                    (0, wgpu::BindingResource::TextureView(view)),
                    (1, wgpu::BindingResource::Sampler(&self.shader_sampler)),
                    (2, buffer.as_entire_binding()),
                    (3, wgpu::BindingResource::TextureView(tex1_view)),
                    (4, wgpu::BindingResource::Sampler(&self.shader_sampler)),
                    (5, wgpu::BindingResource::TextureView(tex2_view)),
                    (6, wgpu::BindingResource::Sampler(&self.shader_sampler)),
                    (7, wgpu::BindingResource::TextureView(palette_view)),
                    (8, wgpu::BindingResource::Sampler(&self.shader_sampler)),
                    (9, wgpu::BindingResource::TextureView(&self.grab_view)),
                    (10, wgpu::BindingResource::Sampler(&self.shader_sampler)),
                ]
                .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource });
                let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("manato shader bind group"),
                    layout: &self.custom_bgl,
                    entries: &entries,
                });
                Some((buffer, bind_group))
            })();
            custom_bindings.push(binding);
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("manato frame encoder"),
            });
        // Grab pass: the previous frame is sampled as `bgl_RenderedTexture`.
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.game_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &self.grab_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: GAME_WIDTH,
                height: GAME_HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        let draw_range_set =
            |pass: &mut wgpu::RenderPass<'_>, ranges: &[draw::DrawRange], offset: usize| {
                if ranges.is_empty() {
                    return;
                }
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                for (local, range) in ranges.iter().enumerate() {
                    let range_index = offset + local;
                    let custom = if range.mask {
                        None
                    } else {
                        range
                            .shader
                            .as_ref()
                            .and_then(|name| self.custom_shaders.get(name))
                    };
                    let pipeline = if let Some(custom) = custom {
                        match range.blend {
                            Blend::Normal => &custom[0],
                            Blend::Add => &custom[1],
                            Blend::Sub => &custom[2],
                        }
                    } else {
                        let pipelines = if range.mask {
                            &self.pipelines_mask
                        } else {
                            &self.pipelines
                        };
                        match range.blend {
                            Blend::Normal => &pipelines[0],
                            Blend::Add => &pipelines[1],
                            Blend::Sub => &pipelines[2],
                        }
                    };
                    let bind_group = match range.tex {
                        TexRef::Atlas(page) => self.atlas.page_bind_group(page),
                        TexRef::Font => &self.font.bind_group,
                        TexRef::ScreenFont(font, page) => self
                            .screen_fonts
                            .get(&font)
                            .map(|font| font.page_bind_group(page))
                            .unwrap_or(&self.font.bind_group),
                        TexRef::Solid => &self.solid,
                        TexRef::Shadow => &self.shadow,
                    };
                    if let Some([x, y, w, h]) = range.scissor {
                        let x = x.min(GAME_WIDTH.saturating_sub(1));
                        let y = y.min(GAME_HEIGHT.saturating_sub(1));
                        let w = w.min(GAME_WIDTH.saturating_sub(x)).max(1);
                        let h = h.min(GAME_HEIGHT.saturating_sub(y)).max(1);
                        pass.set_scissor_rect(x, y, w, h);
                    } else {
                        pass.set_scissor_rect(0, 0, GAME_WIDTH, GAME_HEIGHT);
                    }
                    pass.set_pipeline(pipeline);
                    if let Some(Some((_, custom_bind_group))) = custom_bindings.get(range_index) {
                        pass.set_bind_group(0, custom_bind_group, &[]);
                    } else {
                        pass.set_bind_group(0, bind_group, &[]);
                    }
                    pass.draw(range.start..range.start + range.count, 0..1);
                }
            };
        let scene_end = hud_start.min(ranges.len());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("manato scene pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.game_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            draw_range_set(&mut pass, &ranges[..scene_end], 0);
        }
        // The current scene is now available for transmissive model sampling.
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.game_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &self.refraction_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: GAME_WIDTH,
                height: GAME_HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        if !self.model_draws.is_empty() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("manato model pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.game_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.model_pipeline);
            for (buffer, count, bind_group) in &self.model_draws {
                pass.set_bind_group(0, bind_group, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..*count, 0..1);
            }
        }
        if scene_end < ranges.len() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("manato hud pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.game_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            draw_range_set(&mut pass, &ranges[scene_end..], scene_end);
        }
        self.queue.submit([encoder.finish()]);
    }

    /// Blits the 320x240 target to a surface view, letterboxed on black.
    pub fn blit(&self, target: &wgpu::TextureView, width: u32, height: u32) {
        let width = width.max(1) as f32;
        let height = height.max(1) as f32;
        let scale = (width / GAME_WIDTH as f32).min(height / GAME_HEIGHT as f32);
        let half_w = GAME_WIDTH as f32 * scale / width;
        let half_h = GAME_HEIGHT as f32 * scale / height;
        let rect: [f32; 4] = [-half_w, half_h, half_w, -half_h];
        self.queue
            .write_buffer(&self.blit_uniform, 0, cast_slice(&rect));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("manato blit encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("manato blit pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
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
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &self.blit_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
        self.queue.submit([encoder.finish()]);
    }

    /// Reads the 320x240 target back and saves it as a PNG.
    pub fn screenshot(&self, path: &Path) -> anyhow::Result<()> {
        let bytes_per_pixel = 4u32;
        let unpadded = GAME_WIDTH * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = unpadded.div_ceil(align) * align;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("manato screenshot buffer"),
            size: (padded * GAME_HEIGHT) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("manato screenshot encoder"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.game_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(GAME_HEIGHT),
                },
            },
            wgpu::Extent3d {
                width: GAME_WIDTH,
                height: GAME_HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = self.device.poll(wgpu::PollType::Wait);
        rx.recv().context("screenshot map channel closed")??;

        let mapped = slice.get_mapped_range();
        let mut rgba = Vec::with_capacity((GAME_WIDTH * GAME_HEIGHT * 4) as usize);
        for row in 0..GAME_HEIGHT {
            let start = (row * padded) as usize;
            let end = start + unpadded as usize;
            rgba.extend_from_slice(&mapped[start..end]);
        }
        drop(mapped);
        buffer.unmap();

        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        image::save_buffer(
            path,
            &rgba,
            GAME_WIDTH,
            GAME_HEIGHT,
            image::ColorType::Rgba8,
        )
        .with_context(|| format!("failed to save screenshot {}", path.display()))?;
        Ok(())
    }
}

fn make_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    texture: &wgpu::Texture,
    label: &str,
) -> wgpu::BindGroup {
    let view = texture.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    })
}

fn create_shadow_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    const W: u32 = 64;
    const H: u32 = 32;
    let mut pixels = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let nx = (x as f32 + 0.5) / W as f32 * 2.0 - 1.0;
            let ny = (y as f32 + 0.5) / H as f32 * 2.0 - 1.0;
            let d = (nx * nx + ny * ny).sqrt();
            let a = ((1.0 - d).clamp(0.0, 1.0)).powf(1.6);
            let i = ((y * W + x) * 4) as usize;
            pixels[i] = 0;
            pixels[i + 1] = 0;
            pixels[i + 2] = 0;
            pixels[i + 3] = (a * 255.0) as u8;
        }
    }

    device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("manato shadow texture"),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: GAME_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &pixels,
    )
}

/// Rewrites Ikemen's OpenGL/GLES-style GLSL into the explicit Vulkan-style
/// declarations this renderer's bind group layout expects. Supports implicit
/// `in`/`out` locations, `sampler2D` texture/sampler pairs, `TextureSize`, and
/// the common PalFX uniforms (mapped to `shaderparam` slots).
fn preprocess_implicit_glsl(source: &str) -> String {
    let has_version = source
        .lines()
        .any(|line| line.trim_start().starts_with("#version"));
    let mut out = String::new();
    if !has_version {
        out.push_str("#version 450\n");
    }
    let mut prelude = Vec::new();
    let mut udb_needed = false;
    // Pick the GL branch of Ikemen's dual Vulkan/GL shader template.
    let mut skip_vulkan = false;
    let mut dual_branch = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#if") && trimmed.contains("__VERSION__") {
            skip_vulkan = true;
            dual_branch = true;
            continue;
        }
        if skip_vulkan {
            if trimmed.starts_with("#else") {
                skip_vulkan = false;
                continue;
            }
            if trimmed.starts_with("#endif") {
                skip_vulkan = false;
                dual_branch = false;
            }
            continue;
        }
        if dual_branch && trimmed.starts_with("#endif") {
            dual_branch = false;
            continue;
        }
        if let Some(name) = trimmed
            .strip_prefix("uniform sampler2D ")
            .and_then(|rest| rest.strip_suffix(';'))
            .map(str::trim)
        {
            let lower = name.to_ascii_lowercase();
            let binding = match lower.as_str() {
                "tex" | "texture" => 0,
                "bgl_renderedtexture" => 9,
                "tex1" => 3,
                "tex2" => 5,
                "pal" => 7,
                _ => {
                    // Unknown samplers keep their declaration and will be
                    // reported by the shader compiler.
                    out.push_str(line);
                    out.push('\n');
                    continue;
                }
            };
            prelude.push(format!(
                "layout(binding = {binding}) uniform texture2D {name}_tex;\n\
                 layout(binding = {}) uniform sampler {name}_samp;\n\
                 #define {name} sampler2D({name}_tex, {name}_samp)",
                binding + 1
            ));
            continue;
        }
        let trimmed_no_semi = trimmed.strip_suffix(';').unwrap_or(trimmed).trim();
        let mapped = match trimmed_no_semi {
            "uniform vec2 TextureSize" => {
                Some("#define TextureSize vec2(320.0, 240.0)".to_string())
            }
            "uniform vec4 add" => Some("#define add sp.param[0]".to_string()),
            "uniform vec4 mult" => Some("#define mult sp.param[1]".to_string()),
            "uniform float alpha" => Some("#define alpha sp.param[2].x".to_string()),
            "uniform float neg" => Some("#define neg (sp.param[2].y != 0.0)".to_string()),
            "uniform float gray" => Some("#define gray sp.param[2].z".to_string()),
            "uniform float hue" => Some("#define hue sp.param[2].w".to_string()),
            _ => None,
        };
        if let Some(mapped) = mapped {
            if mapped.contains("sp.param") {
                udb_needed = true;
            }
            prelude.push(mapped);
            continue;
        }
        let implicit = match trimmed {
            "in vec3 vTexCoord" | "in vec3 vTexCoord;" => {
                Some("layout(location = 0) in vec2 vTexCoord;")
            }
            "in vec2 vTexCoord" | "in vec2 vTexCoord;" | "in vec2 uv" | "in vec2 uv;" => {
                Some("layout(location = 0) in vec2 vTexCoord;")
            }
            "in vec4 color" | "in vec4 color;" => Some("layout(location = 1) in vec4 color;"),
            "out vec4 FragColor" | "out vec4 FragColor;" => {
                Some("layout(location = 0) out vec4 FragColor;")
            }
            _ => None,
        };
        if let Some(implicit) = implicit {
            out.push_str(implicit);
            out.push('\n');
            continue;
        }
        if trimmed.contains("gl_FragColor") {
            out.push_str(&line.replace("gl_FragColor", "fragColor"));
            out.push('\n');
            continue;
        }
        out.push_str(&line.replace("vTexCoord.z", "0.0"));
        out.push('\n');
    }
    if udb_needed {
        prelude.push(
            "layout(binding = 2) uniform ShaderParams { vec4 param[4]; vec4 misc; } sp;"
                .to_string(),
        );
    }
    if !prelude.is_empty() {
        // Insert after the version directive.
        if let Some(position) = out.find('\n') {
            out.insert_str(position + 1, &(prelude.join("\n") + "\n"));
        } else {
            out.push_str(&prelude.join("\n"));
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SHADER: &str = r#"
struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) z: f32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(
        in.pos.x / 320.0 * 2.0 - 1.0,
        1.0 - in.pos.y / 240.0 * 2.0,
        in.z,
        1.0,
    );
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv) * in.color;
    return vec4<f32>(c.g, c.b, c.r, c.a);
}
"#;

    #[test]
    fn score_table_renders_when_gpu_available() {
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        renderer.render_scores(&[9000, 5000, 300, 100]);
        let path = std::env::temp_dir().join(format!("manato-scores-{}.png", std::process::id()));
        renderer.screenshot(&path).expect("screenshot");
        let size = std::fs::metadata(&path).expect("file").len();
        assert!(size > 1000, "score table should not be blank");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn continue_overlay_renders_when_gpu_available() {
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        renderer.render_continue(3, 9, 1234);
        let path = std::env::temp_dir().join(format!("manato-continue-{}.png", std::process::id()));
        renderer.screenshot(&path).expect("screenshot");
        let size = std::fs::metadata(&path).expect("file").len();
        assert!(size > 1000, "continue screen should not be blank");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn custom_shader_grab_texture_renders_when_gpu_available() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = root.join("content/chars/kaen/kaen.def");
        if !source.exists() {
            return;
        }
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let dir = std::env::temp_dir().join(format!("manato-grab-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let shader_path = dir.join("grab.frag");
        std::fs::write(
            &shader_path,
            r#"
uniform sampler2D bgl_RenderedTexture;
in vec2 vTexCoord;
out vec4 FragColor;
void main() {
    vec4 c = texture(bgl_RenderedTexture, vTexCoord.xy);
    FragColor = vec4(c.rgb * 0.5 + 0.25, 1.0);
}
"#,
        )
        .expect("shader");
        let config = manato_sim::MatchConfig {
            mugen_root: root.clone(),
            p1: source,
            p2: std::path::PathBuf::from("content/chars/nival/nival.def"),
            stage: std::path::PathBuf::from("content/stages/training/stage1.def"),
            screenpack: None,
            rounds_to_win: 1,
            round_time: 99,
        };
        let mut sim = manato_sim::SimState::load(&config).expect("sim");
        {
            let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("assets");
            assets.characters[0]
                .def
                .shaders
                .insert("grab-test".into(), shader_path);
        }
        sim.fighters[0].shader = Some("grab-test".into());
        renderer.load_assets(&sim);
        renderer.render(&sim);
        renderer.render(&sim);
        let path = dir.join("grab.png");
        renderer.screenshot(&path).expect("screenshot");
        assert!(std::fs::metadata(&path).expect("file").len() > 1000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn projectile_palette_banks_materialize_when_gpu_available() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = root.join("content/chars/kaen/kaen.def");
        if !source.exists() {
            return;
        }
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let config = manato_sim::MatchConfig {
            mugen_root: root.clone(),
            p1: source,
            p2: std::path::PathBuf::from("content/chars/nival/nival.def"),
            stage: std::path::PathBuf::from("content/stages/training/stage1.def"),
            screenpack: None,
            rounds_to_win: 1,
            round_time: 99,
        };
        let mut sim = manato_sim::SimState::load(&config).expect("sim");
        {
            let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
            assets.characters[0]
                .palettes
                .insert((1, 2), vec![[255, 0, 0, 255]; 256]);
        }
        renderer.load_assets(&sim);
        assert!(
            !renderer.projectile_banks.is_empty(),
            "character palettes should materialize projectile banks"
        );
        assert!(renderer
            .projectile_banks
            .values()
            .all(|bank| (192..=255).contains(bank)));
    }

    #[test]
    fn projectile_palette_banks_exceed_32_combos_when_gpu_available() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = root.join("content/chars/kaen/kaen.def");
        if !source.exists() {
            return;
        }
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let config = manato_sim::MatchConfig {
            mugen_root: root,
            p1: source,
            p2: std::path::PathBuf::from("content/chars/nival/nival.def"),
            stage: std::path::PathBuf::from("content/stages/training/stage1.def"),
            screenpack: None,
            rounds_to_win: 1,
            round_time: 99,
        };
        let mut sim = manato_sim::SimState::load(&config).expect("sim");
        {
            let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
            for slot in 0..2.min(assets.characters.len()) {
                for index in 0..40 {
                    assets.characters[slot]
                        .palettes
                        .insert((10 + slot as i32, index), vec![[255, 0, 0, 255]; 256]);
                }
            }
        }
        renderer.load_assets(&sim);
        let palette_count = renderer
            .projectile_banks
            .keys()
            .filter(|(slot, key)| *slot < 2 && key.0 >= 10)
            .count();
        assert!(
            palette_count > 32,
            "expected more than 32 dedicated palette banks, got {palette_count}"
        );
        let mut banks: Vec<u8> = renderer.projectile_banks.values().copied().collect();
        let total = banks.len();
        banks.sort_unstable();
        banks.dedup();
        assert_eq!(banks.len(), total, "each palette combo needs a unique bank");
        assert!(banks.iter().all(|bank| (192..=255).contains(bank)));
        assert_eq!(
            renderer.projectile_palette_files.len(),
            total,
            "palette files must be retained for overlay atlas rebuilds"
        );
    }

    #[test]
    fn custom_shader_tex1_and_tex2_bind_referenced_sprites() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = root.join("content/chars/kaen/kaen.def");
        if !source.exists() {
            return;
        }
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let dir = std::env::temp_dir().join(format!("manato-tex1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let shader_path = dir.join("tex.frag");
        std::fs::write(
            &shader_path,
            r#"#version 450
layout(location = 0) in vec2 uv;
layout(location = 1) in vec4 color;
layout(binding = 0) uniform texture2D tex;
layout(binding = 1) uniform sampler samp;
layout(binding = 3) uniform texture2D tex1;
layout(binding = 4) uniform sampler tex1samp;
layout(binding = 5) uniform texture2D tex2;
layout(binding = 6) uniform sampler tex2samp;
layout(location = 0) out vec4 frag;
void main() {
    vec4 c = texture(sampler2D(tex, samp), uv);
    vec4 a = texture(sampler2D(tex1, tex1samp), uv);
    vec4 b = texture(sampler2D(tex2, tex2samp), uv);
    frag = vec4(c.rgb * 0.4 + a.rgb * 0.3 + b.rgb * 0.3, c.a) * color;
}
"#,
        )
        .expect("shader");
        let config = manato_sim::MatchConfig {
            mugen_root: root.clone(),
            p1: source,
            p2: std::path::PathBuf::from("content/chars/nival/nival.def"),
            stage: std::path::PathBuf::from("content/stages/training/stage1.def"),
            screenpack: None,
            rounds_to_win: 1,
            round_time: 99,
        };
        let mut sim = manato_sim::SimState::load(&config).expect("sim");
        {
            let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("assets");
            assets.characters[0]
                .def
                .shaders
                .insert("tex-test".into(), shader_path);
        }
        sim.fighters[0].shader = Some("tex-test".into());
        sim.fighters[0].shader_tex1 = Some((0, 0));
        sim.fighters[0].shader_tex2 = Some((0, 0));
        renderer.load_assets(&sim);
        renderer.render(&sim);
        let bank = atlas::bank::CHARACTER_BASE;
        assert!(renderer.shader_textures.contains_key(&(bank, 0, 0)));
        let path = dir.join("tex.png");
        renderer.screenshot(&path).expect("screenshot");
        assert!(std::fs::metadata(&path).expect("file").len() > 1000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn indexed_sprites_upload_real_palette_textures() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = root.join("content/chars/kaen/kaen.def");
        if !source.exists() {
            return;
        }
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let config = manato_sim::MatchConfig {
            mugen_root: root.clone(),
            p1: source,
            p2: std::path::PathBuf::from("content/chars/nival/nival.def"),
            stage: std::path::PathBuf::from("content/stages/training/stage1.def"),
            screenpack: None,
            rounds_to_win: 1,
            round_time: 99,
        };
        let sim = manato_sim::SimState::load(&config).expect("sim");
        renderer.load_assets(&sim);
        let bank = atlas::bank::CHARACTER_BASE;
        let sprite = sim.assets.characters[0]
            .sprites
            .sprites
            .iter()
            .find(|sprite| sprite.palette_key.is_some() && !sprite.palette.is_empty())
            .expect("indexed sprite");
        assert!(renderer
            .atlas
            .palette_view(bank, sprite.palette_key.expect("palette key"))
            .is_some());
    }

    #[test]
    fn custom_shader_pal_binding_renders_when_gpu_available() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = root.join("content/chars/kaen/kaen.def");
        if !source.exists() {
            return;
        }
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let dir = std::env::temp_dir().join(format!("manato-pal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let shader_path = dir.join("pal.frag");
        std::fs::write(
            &shader_path,
            r#"#version 450
layout(location = 0) in vec2 uv;
layout(location = 1) in vec4 color;
layout(binding = 0) uniform texture2D tex;
layout(binding = 1) uniform sampler samp;
layout(binding = 7) uniform texture2D pal;
layout(binding = 8) uniform sampler palsamp;
layout(location = 0) out vec4 frag;
void main() {
    vec4 c = texture(sampler2D(tex, samp), uv);
    vec3 p = texture(sampler2D(pal, palsamp), vec2(uv.x, 0.5)).rgb;
    frag = vec4(c.rgb * 0.5 + p * 0.5, c.a) * color;
}
"#,
        )
        .expect("shader");
        let config = manato_sim::MatchConfig {
            mugen_root: root.clone(),
            p1: source,
            p2: std::path::PathBuf::from("content/chars/nival/nival.def"),
            stage: std::path::PathBuf::from("content/stages/training/stage1.def"),
            screenpack: None,
            rounds_to_win: 1,
            round_time: 99,
        };
        let mut sim = manato_sim::SimState::load(&config).expect("sim");
        {
            let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("assets");
            assets.characters[0]
                .def
                .shaders
                .insert("pal-test".into(), shader_path);
        }
        sim.fighters[0].shader = Some("pal-test".into());
        renderer.load_assets(&sim);
        renderer.render(&sim);
        let path = dir.join("pal.png");
        renderer.screenshot(&path).expect("screenshot");
        assert!(std::fs::metadata(&path).expect("file").len() > 1000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn implicit_glsl_preprocessor_adds_bindings_and_locations() {
        let source = r#"
uniform vec2 TextureSize;
uniform sampler2D Texture;
in vec3 vTexCoord;
out vec4 FragColor;
void main() {
    vec4 c = texture(Texture, vTexCoord.xy);
    FragColor = c + vec4(vTexCoord.z) + vec4(TextureSize, 0.0, 0.0);
}
"#;
        let out = preprocess_implicit_glsl(source);
        assert!(out.starts_with("#version 450"));
        assert!(out.contains("layout(binding = 0) uniform texture2D Texture_tex;"));
        assert!(out.contains("layout(binding = 1) uniform sampler Texture_samp;"));
        assert!(out.contains("#define Texture sampler2D(Texture_tex, Texture_samp)"));
        let grab = preprocess_implicit_glsl("uniform sampler2D bgl_RenderedTexture;");
        assert!(grab.contains("layout(binding = 9) uniform texture2D bgl_RenderedTexture_tex;"));
        assert!(grab.contains("layout(binding = 10) uniform sampler bgl_RenderedTexture_samp;"));
        assert!(out.contains("layout(location = 0) in vec2 vTexCoord;"));
        assert!(out.contains("layout(location = 0) out vec4 FragColor;"));
        assert!(out.contains("#define TextureSize vec2(320.0, 240.0)"));
        assert!(!out.contains("vTexCoord.z"));
    }

    #[test]
    fn implicit_glsl_scanline_shader_compiles_when_gpu_available() {
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let source = r#"
#if __VERSION__ >= 450
layout(push_constant, std430) uniform u { vec2 TextureSize; };
layout(binding = 0) uniform sampler2D Texture;
layout(location = 0) in vec3 vTexCoord;
layout(location = 0) out vec4 FragColor;
#else
uniform vec2 TextureSize;
uniform sampler2D Texture;
in vec3 vTexCoord;
out vec4 FragColor;
#endif
void main() {
    vec4 rgb = texture(Texture, vTexCoord.xy);
    FragColor = rgb * 0.5 + vec4(TextureSize / 640.0, 0.0, 0.0);
}
"#;
        assert!(renderer.register_glsl_shader("implicit-scanline", source));
        assert!(renderer.custom_shaders.contains_key("implicit-scanline"));
    }

    #[test]
    fn overlay_scene_background_renders_when_gpu_available() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = root.join("content/chars/kaen/kaen.def");
        let stage = root.join("content/stages/training/stage1.def");
        if !source.exists() || !stage.exists() {
            return;
        }
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let dir = std::env::temp_dir().join(format!("manato-overlay-bg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let def = dir.join("story.def");
        std::fs::write(
            &def,
            format!(
                "[Info]\nname = Overlay Bg\n[SceneDef]\nstartscene = 0\n[Scene 0]\nend.time = 100\nbg.name = {}\nlayer0.text = \"BG\"\nlayer0.offset = 120,100\nlayer0.textdelay = 0\n",
                stage.display()
            ),
        )
        .expect("write");
        let config = manato_sim::MatchConfig {
            mugen_root: root.clone(),
            p1: source,
            p2: std::path::PathBuf::from("content/chars/nival/nival.def"),
            stage: std::path::PathBuf::from("content/stages/training/stage1.def"),
            screenpack: None,
            rounds_to_win: 1,
            round_time: 99,
        };
        let mut sim = manato_sim::SimState::load(&config).expect("sim");
        let assets = sim.assets.clone();
        let control = manato_formats::Controller {
            kind: "storyboard".into(),
            params: vec![
                ("path".into(), def.display().to_string()),
                ("space".into(), "screen".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(&mut sim, 0, &control, &assets);
        assert_eq!(sim.storyboards.len(), 1);
        renderer.load_assets(&sim);
        renderer.render(&sim);
        let key = (
            std::sync::Arc::as_ptr(&sim.storyboards[0].storyboard) as usize,
            0,
        );
        assert!(renderer.overlay_bg_banks.contains_key(&key));
        let path = dir.join("overlay.png");
        renderer.screenshot(&path).expect("screenshot");
        assert!(std::fs::metadata(&path).expect("file").len() > 1000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scene_reports_hud_boundary_for_model_pass() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = root.join("content/chars/kaen/kaen.def");
        if !source.exists() {
            return;
        }
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let config = manato_sim::MatchConfig {
            mugen_root: root,
            p1: source,
            p2: std::path::PathBuf::from("content/chars/nival/nival.def"),
            stage: std::path::PathBuf::from("content/stages/training/stage1.def"),
            screenpack: None,
            rounds_to_win: 1,
            round_time: 99,
        };
        let sim = manato_sim::SimState::load(&config).expect("sim");
        renderer.load_assets(&sim);
        let deps = SceneDeps {
            atlas: &renderer.atlas,
            font: &renderer.font,
            screen_fonts: &renderer.screen_fonts,
            overlay_bg_banks: &renderer.overlay_bg_banks,
            projectile_banks: &renderer.projectile_banks,
        };
        let mut painter = Painter::default();
        let hud_start = scene::build(&mut painter, &sim, &deps);
        assert!(hud_start > 0, "scene ranges should precede the HUD");
        assert!(
            hud_start <= painter.ranges.len(),
            "HUD boundary must be within the recorded ranges"
        );
    }

    #[test]
    fn gltf_model_loads_and_renders_when_gpu_available() {
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let dir = std::env::temp_dir().join(format!("manato-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let model = dir.join("triangle.gltf");
        std::fs::write(
            &model,
            r#"{
  "asset": {"version": "2.0"},
  "buffers": [{"uri": "data:application/octet-stream;base64,AACAvwAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAAEAAAAAA", "byteLength": 36}],
  "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962}],
  "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [-1,0,0], "max": [1,2,0]}],
  "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1,0,0,1]}}],
  "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "material": 0, "mode": 4}]}],
  "nodes": [{"mesh": 0}],
  "scenes": [{"nodes": [0]}],
  "scene": 0
}"#,
        )
        .expect("model");
        renderer.load_model(&model, None, None, &[]);
        assert_eq!(renderer.model_draws.len(), 1);
        assert_eq!(renderer.model_draws[0].1, 3);
        let storyboard = manato_formats::storyboard::Storyboard {
            path: dir.join("story.def"),
            name: "model".into(),
            author: String::new(),
            start_scene: 0,
            sprites: manato_formats::SffFile::default(),
            animations: HashMap::new(),
            scenes: vec![manato_formats::storyboard::StoryboardScene {
                name: "Scene 0".into(),
                end_time: 10,
                ..Default::default()
            }],
            font_files: std::collections::BTreeMap::new(),
            sounds: manato_formats::SoundBank::default(),
            lua_source: None,
            model: Some(model),
            model_anim: None,
            model_env: None,
            model_lights: Vec::new(),
        };
        let player = manato_formats::storyboard::StoryboardPlayer::new(&storyboard);
        renderer.load_storyboard(&storyboard);
        assert_eq!(renderer.model_draws.len(), 1);
        renderer.render_storyboard(&storyboard, &player);
        let path = dir.join("model.png");
        renderer.screenshot(&path).expect("screenshot");
        assert!(std::fs::metadata(&path).expect("file").len() > 1000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn storyboard_bg_ctrls_apply_to_layer_states() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join("content/stages/training/stage1.def");
        if !path.exists() {
            return;
        }
        let data = vec![root.join("data")];
        let mut stage = manato_formats::stage::Stage::load(&path, &data).expect("stage");
        assert!(!stage.layers.is_empty());
        stage.bg_ctrls = vec![
            manato_formats::BgCtrl {
                layer: 0,
                start: 0,
                end: 1000,
                loop_period: -1,
                controller: manato_formats::Controller {
                    kind: "visible".into(),
                    params: vec![("value".into(), "0".into())],
                    line: 0,
                },
            },
            manato_formats::BgCtrl {
                layer: 0,
                start: 0,
                end: 1000,
                loop_period: -1,
                controller: manato_formats::Controller {
                    kind: "posset".into(),
                    params: vec![("x".into(), "42".into())],
                    line: 0,
                },
            },
        ];
        let states = scene::storyboard_bg_states(&stage, 3);
        assert!(!states[0].visible, "BgCtrl visible override applied");
        assert_eq!(states[0].offset.x, 42.0, "BgCtrl posset applied");
    }

    #[test]
    fn storyboard_background_renders_when_gpu_available() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let stage = root.join("content/stages/training/stage1.def");
        if !stage.exists() {
            return;
        }
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let dir = std::env::temp_dir().join(format!("manato-sb-bg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let def = dir.join("story.def");
        std::fs::write(
            &def,
            format!(
                "[Info]\nname = Bg\n[SceneDef]\nstartscene = 0\n[Scene 0]\nend.time = 10\nbg.name = {}\n",
                stage.display()
            ),
        )
        .expect("write");
        let storyboard = manato_formats::storyboard::Storyboard::load(&def).expect("load");
        assert!(storyboard.scenes[0].bg.is_some());
        let player = manato_formats::storyboard::StoryboardPlayer::new(&storyboard);
        renderer.load_storyboard(&storyboard);
        renderer.render_storyboard(&storyboard, &player);
        let path = dir.join("bg.png");
        renderer.screenshot(&path).expect("screenshot");
        assert!(std::fs::metadata(&path).expect("file").len() > 1000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn glsl_fragment_compiles_when_gpu_available() {
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let source = r#"#version 450
layout(location = 0) smooth in vec2 uv;
layout(location = 1) smooth in vec4 color;
layout(binding = 0) uniform texture2D tex;
layout(binding = 1) uniform sampler samp;
layout(location = 0) out vec4 frag;
void main() {
    vec4 c = texture(sampler2D(tex, samp), uv) * color;
    frag = vec4(c.g, c.b, c.r, c.a);
}
"#;
        assert!(renderer.register_glsl_shader("swap-glsl", source));
        assert!(renderer.custom_shaders.contains_key("swap-glsl"));
    }

    #[test]
    fn wgsl_custom_shader_compiles_when_gpu_available() {
        let Ok(mut renderer) = Renderer::new_offscreen() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        assert!(renderer.register_shader("swap", TEST_SHADER));
        assert!(renderer.custom_shaders.contains_key("swap"));
        // Re-registering replaces the cached pipelines.
        assert!(renderer.register_shader("swap", TEST_SHADER));
    }
}
