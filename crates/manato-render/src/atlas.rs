//! Sprite atlas packing and texturing.
//!
//! Every sprite from every content bank (stage, characters, fightfx) is packed
//! into a small number of RGBA pages. Drawing a sprite is then a single quad
//! referencing a page and a UV rectangle.

use std::collections::HashMap;

use manato_formats::SffFile;

/// Identifies the sprite bank a `(group, index)` pair belongs to.
pub mod bank {
    /// Stage sprite file.
    pub const STAGE: u8 = 2;
    /// Fight effects (`fightfx.sff`).
    pub const EFFECTS: u8 = 3;
    /// Screenpack sprites (`system.def` `[Files] spr`).
    pub const SCREENPACK: u8 = 4;
    /// Storyboard sprites (`[SceneDef] spr`).
    pub const STORYBOARD: u8 = 5;
    /// Character sprites are stored at `CHARACTER_BASE + char_id`.
    pub const CHARACTER_BASE: u8 = 16;
}

#[derive(Debug, Clone, Copy)]
pub struct AtlasSprite {
    pub page: u32,
    /// UV rectangle `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
    pub width: f32,
    pub height: f32,
}

pub struct AtlasPage {
    pub bind_group: wgpu::BindGroup,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

pub struct SpriteAtlas {
    pub pages: Vec<AtlasPage>,
    pub page_size: u32,
    sprites: HashMap<(u8, i32, i32), AtlasSprite>,
    placements: HashMap<(u8, i32, i32), (u32, u32, u32)>,
    sampler: wgpu::Sampler,
}

struct Placement {
    bank: u8,
    group: i32,
    index: i32,
    width: u32,
    height: u32,
    page: u32,
    x: u32,
    y: u32,
}

impl SpriteAtlas {
    pub fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        banks: &[(u8, &SffFile)],
    ) -> Self {
        const PAGE_SIZE: u32 = 1024;
        let page_size = PAGE_SIZE.max(
            banks
                .iter()
                .flat_map(|(_, sff)| sff.sprites.iter())
                .map(|s| s.width.max(s.height))
                .max()
                .unwrap_or(1)
                .next_power_of_two(),
        );

        // Sort by height so the shelf packer wastes less space.
        let mut entries: Vec<(u8, i32, i32, u32, u32)> = Vec::new();
        for (id, sff) in banks {
            for sprite in &sff.sprites {
                if sprite.width == 0 || sprite.height == 0 || sprite.rgba.is_empty() {
                    continue;
                }
                entries.push((*id, sprite.group, sprite.index, sprite.width, sprite.height));
            }
        }
        entries.sort_by(|a, b| {
            b.4.cmp(&a.4)
                .then_with(|| a.0.cmp(&b.0))
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });

        let mut placements = Vec::with_capacity(entries.len());
        let mut page_count = 1u32;
        let mut shelf_x = 0u32;
        let mut shelf_y = 0u32;
        let mut shelf_h = 0u32;
        for (bank, group, index, width, height) in entries {
            if width > page_size || height > page_size {
                log::warn!("sprite {group},{index} is larger than the atlas page; skipping");
                continue;
            }
            if shelf_x + width > page_size {
                shelf_x = 0;
                shelf_y += shelf_h;
                shelf_h = 0;
            }
            if shelf_y + height > page_size {
                page_count += 1;
                shelf_y = 0;
                shelf_x = 0;
                shelf_h = 0;
            }
            placements.push(Placement {
                bank,
                group,
                index,
                width,
                height,
                page: page_count - 1,
                x: shelf_x,
                y: shelf_y,
            });
            shelf_x += width;
            shelf_h = shelf_h.max(height);
        }

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("manato sprite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let page_size_f = page_size as f32;
        let mut placement_map = HashMap::new();
        for p in &placements {
            placement_map.insert((p.bank, p.group, p.index), (p.page, p.x, p.y));
        }
        let mut sprites = HashMap::new();
        for p in &placements {
            sprites.insert(
                (p.bank, p.group, p.index),
                AtlasSprite {
                    page: p.page,
                    uv: [
                        (p.x as f32 + 0.5) / page_size_f,
                        (p.y as f32 + 0.5) / page_size_f,
                        (p.x as f32 + p.width as f32 - 0.5) / page_size_f,
                        (p.y as f32 + p.height as f32 - 0.5) / page_size_f,
                    ],
                    width: p.width as f32,
                    height: p.height as f32,
                },
            );
        }

        let mut pages = Vec::new();
        for _ in 0..page_count {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("manato sprite atlas page"),
                size: wgpu::Extent3d {
                    width: page_size,
                    height: page_size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("manato sprite atlas bind group"),
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
            pages.push(AtlasPage {
                bind_group,
                texture,
                view,
            });
        }

        // Upload the pixels.
        for p in &placements {
            let Some(sprite) = banks
                .iter()
                .find(|(id, _)| *id == p.bank)
                .and_then(|(_, sff)| sff.get(p.group, p.index))
            else {
                continue;
            };
            let expected = (p.width * p.height * 4) as usize;
            if sprite.rgba.len() != expected {
                continue;
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &pages[p.page as usize].texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: p.x,
                        y: p.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &sprite.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(p.width * 4),
                    rows_per_image: Some(p.height),
                },
                wgpu::Extent3d {
                    width: p.width,
                    height: p.height,
                    depth_or_array_layers: 1,
                },
            );
        }

        Self {
            pages,
            page_size,
            sprites,
            placements: placement_map,
            sampler,
        }
    }

    /// Texture view of an atlas page (custom shader bind groups).
    pub fn page_view(&self, page: u32) -> Option<&wgpu::TextureView> {
        self.pages.get(page as usize).map(|page| &page.view)
    }

    pub fn get(&self, bank: u8, group: i32, index: i32) -> Option<AtlasSprite> {
        self.sprites.get(&(bank, group, index)).copied()
    }

    /// Re-uploads one sprite's pixels in place. The atlas layout, UVs, and
    /// bind groups are unchanged.
    pub fn update_sprite(
        &self,
        queue: &wgpu::Queue,
        bank: u8,
        group: i32,
        index: i32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) {
        let Some(&(page, x, y)) = self.placements.get(&(bank, group, index)) else {
            return;
        };
        let Some(page) = self.pages.get(page as usize) else {
            return;
        };
        if rgba.len() != (width * height * 4) as usize {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &page.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn page_bind_group(&self, page: u32) -> &wgpu::BindGroup {
        &self.pages[page as usize].bind_group
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }
}
