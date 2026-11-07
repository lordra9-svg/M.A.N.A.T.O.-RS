//! Tiny bitmap font used for the HUD and message overlays.
//!
//! Glyphs come from the public-domain `font8x8` basic set. They are uploaded
//! once into a 128x64 alpha atlas and drawn as quads, which keeps text pixel
//! aligned at any integer scale.

use font8x8::UnicodeFonts as _;
use wgpu::util::DeviceExt as _;

pub const GLYPH_W: f32 = 8.0;
pub const GLYPH_H: f32 = 8.0;

pub struct FontAtlas {
    pub bind_group: wgpu::BindGroup,
    _texture: wgpu::Texture,
    uvs: Vec<[f32; 4]>,
    pub sampler: wgpu::Sampler,
}

impl FontAtlas {
    pub fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) -> Self {
        const COLS: u32 = 16;
        const ROWS: u32 = 8;
        const CELL: u32 = 8;
        let width = COLS * CELL;
        let height = ROWS * CELL;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for code in 0u32..128 {
            let ch = char::from_u32(code).unwrap_or('?');
            let glyph = font8x8::BASIC_FONTS.get(ch).unwrap_or([0; 8]);
            let cx = (code % COLS) * CELL;
            let cy = (code / COLS) * CELL;
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..8u32 {
                    if bits & (1 << col) != 0 {
                        let x = cx + col;
                        let y = cy + row as u32;
                        let i = ((y * width + x) * 4) as usize;
                        pixels[i] = 255;
                        pixels[i + 1] = 255;
                        pixels[i + 2] = 255;
                        pixels[i + 3] = 255;
                    }
                }
            }
        }

        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("manato font atlas"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &pixels,
        );
        let view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("manato font sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("manato font bind group"),
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
        });

        let mut uvs = vec![[0.0; 4]; 128];
        for code in 0u32..128 {
            let cx = (code % COLS) * CELL;
            let cy = (code / COLS) * CELL;
            uvs[code as usize] = [
                (cx as f32 + 0.5) / width as f32,
                (cy as f32 + 0.5) / height as f32,
                (cx as f32 + CELL as f32 - 0.5) / width as f32,
                (cy as f32 + CELL as f32 - 0.5) / height as f32,
            ];
        }

        Self {
            bind_group,
            _texture: texture,
            uvs,
            sampler,
        }
    }

    pub fn glyph(&self, ch: char) -> &[f32; 4] {
        let code = ch as u32;
        let code = if code < 128 { code } else { b'?' as u32 };
        &self.uvs[code as usize]
    }
}

/// Maximum number of glyphs that fit in `width` pixels.
pub fn fit_chars(width: f32, scale: f32) -> usize {
    (width / (GLYPH_W * scale)).floor().max(1.0) as usize
}
