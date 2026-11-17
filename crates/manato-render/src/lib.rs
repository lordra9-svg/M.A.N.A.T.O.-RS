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
    /// Sandboxed Lua runtimes for in-match storyboard overlays.
    storyboard_scripts:
        std::cell::RefCell<HashMap<usize, Option<manato_formats::storyboard_lua::StoryboardLua>>>,
    /// Signature of in-match storyboard overlays baked into the atlas.
    overlay_signature: Vec<(usize, String)>,
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
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
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
            ],
        });
        let custom_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("manato custom shader layout"),
            bind_group_layouts: &[&custom_bgl],
            push_constant_ranges: &[],
        });
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
            storyboard_scripts: std::cell::RefCell::new(HashMap::new()),
            overlay_signature: Vec::new(),
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
        let mut banks: Vec<(u8, &manato_formats::SffFile)> =
            vec![(bank::STAGE, &assets.stage.sprites)];
        for (i, character) in assets.characters.iter().enumerate() {
            banks.push((bank::CHARACTER_BASE + i as u8, &character.sprites));
        }
        banks.push((bank::EFFECTS, &assets.effects.sprites));
        if let Some(screenpack) = &assets.screenpack {
            banks.push((bank::SCREENPACK, &screenpack.sprites));
        }
        self.atlas = SpriteAtlas::build(&self.device, &self.queue, &self.sprite_bgl, &banks);
        self.overlay_signature.clear();

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
        for (index, overlay) in sim.storyboards.iter().enumerate() {
            let id = bank::OVERLAY_BASE as usize + index;
            if id <= u8::MAX as usize {
                banks.push((id as u8, &overlay.storyboard.sprites));
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
        let deps = SceneDeps {
            atlas: &self.atlas,
            font: &self.font,
            screen_fonts: &self.screen_fonts,
            storyboard_scripts: &self.storyboard_scripts,
        };
        scene::build(&mut self.painter, sim, &deps);
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
        self.present();
    }

    /// Builds the atlas from a storyboard's sprite file.
    pub fn load_storyboard(&mut self, storyboard: &manato_formats::storyboard::Storyboard) {
        self.atlas = SpriteAtlas::build(
            &self.device,
            &self.queue,
            &self.sprite_bgl,
            &[(bank::STORYBOARD, &storyboard.sprites)],
        );
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
        let deps = SceneDeps {
            atlas: &self.atlas,
            font: &self.font,
            screen_fonts: &self.screen_fonts,
            storyboard_scripts: &self.storyboard_scripts,
        };
        scene::draw_storyboard(&mut self.painter, &deps, storyboard, player);
        self.present();
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
        self.present();
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
        self.present();
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
        self.present();
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
        self.present();
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
        self.present();
    }

    /// Uploads the accumulated vertices and records the frame commands.
    fn present(&mut self) {
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
                    (3, wgpu::BindingResource::TextureView(view)),
                    (4, wgpu::BindingResource::Sampler(&self.shader_sampler)),
                    (5, wgpu::BindingResource::TextureView(view)),
                    (6, wgpu::BindingResource::Sampler(&self.shader_sampler)),
                    (7, wgpu::BindingResource::TextureView(view)),
                    (8, wgpu::BindingResource::Sampler(&self.shader_sampler)),
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
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("manato game pass"),
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
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if !ranges.is_empty() {
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            }
            for (range_index, range) in ranges.iter().enumerate() {
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
