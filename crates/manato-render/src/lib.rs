//! wgpu renderer for M.A.N.A.T.O.-RS.
//!
//! The game is rendered into a fixed 320x240 texture and then blitted to the
//! window with nearest-neighbour scaling, which keeps the original low
//! resolution pixel look and makes offscreen screenshots trivial.

pub mod atlas;
pub mod draw;
pub mod fightscreen;
pub mod scene;
pub mod screenfont;
pub mod text;

pub use wgpu;

use std::path::Path;

use anyhow::Context as _;
use bytemuck::cast_slice;
use manato_sim::SimState;
use wgpu::util::DeviceExt as _;

use atlas::{bank, SpriteAtlas};
use draw::{
    blend_state, create_blit_shader, create_sprite_shader, vertex_layout, Blend, Painter, TexRef,
};
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

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,

    game_texture: wgpu::Texture,
    game_view: wgpu::TextureView,

    sprite_bgl: wgpu::BindGroupLayout,
    pipelines: [wgpu::RenderPipeline; 3],
    pipelines_mask: [wgpu::RenderPipeline; 3],

    atlas: SpriteAtlas,
    font: FontAtlas,
    screen_fonts: std::collections::BTreeMap<u32, ScreenFont>,
    solid: wgpu::BindGroup,
    shadow: wgpu::BindGroup,

    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group: wgpu::BindGroup,
    blit_uniform: wgpu::Buffer,

    vertex_buffer: wgpu::Buffer,
    vertex_capacity: u64,
    painter: Painter,
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
                depth_stencil: None,
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

        Ok(Self {
            device,
            queue,
            game_texture,
            game_view,
            sprite_bgl,
            pipelines,
            pipelines_mask,
            atlas,
            font,
            screen_fonts: std::collections::BTreeMap::new(),
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

        self.screen_fonts.clear();
        if let Some(screenpack) = &assets.screenpack {
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
        log::info!(
            "sprite atlas: {} pages of {}px",
            self.atlas.pages.len(),
            self.atlas.page_size
        );
    }

    /// Number of atlas pages currently allocated (diagnostics).
    pub fn atlas_pages(&self) -> usize {
        self.atlas.pages.len()
    }

    /// Draws one frame into the internal 320x240 target.
    pub fn render(&mut self, sim: &SimState) {
        let deps = SceneDeps {
            atlas: &self.atlas,
            font: &self.font,
            screen_fonts: &self.screen_fonts,
        };
        scene::build(&mut self.painter, sim, &deps);

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
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if !ranges.is_empty() {
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            }
            for range in &ranges {
                let pipelines = if range.mask {
                    &self.pipelines_mask
                } else {
                    &self.pipelines
                };
                let pipeline = match range.blend {
                    Blend::Normal => &pipelines[0],
                    Blend::Add => &pipelines[1],
                    Blend::Sub => &pipelines[2],
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
                pass.set_bind_group(0, bind_group, &[]);
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
    let texture = device.create_texture_with_data(
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
    );
    texture
}
