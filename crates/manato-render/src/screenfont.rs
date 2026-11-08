//! GPU font banks for screenpack (`.fnt`) fonts.
//!
//! Each loaded [`BitmapFont`] is packed into its own sprite atlas. Drawing
//! text then emits one quad per glyph referencing that atlas plus the
//! built-in fallback when a font is missing (`docs/spec/render.md` §3.3).

use std::collections::BTreeMap;

use manato_core::Vec2;
use manato_formats::font::{BitmapFont, Glyph};

use crate::atlas::SpriteAtlas;
use crate::draw::{Blend, Painter, TexRef};

pub struct ScreenFont {
    /// Index into the screenpack's `[Files] fontN` table.
    pub index: u32,
    atlas: SpriteAtlas,
    glyphs: BTreeMap<char, Glyph>,
    pub solid: bool,
    pub size: [f32; 2],
    pub spacing: [f32; 2],
    pub offset: [f32; 2],
    pub tint: [f32; 3],
}

impl ScreenFont {
    pub fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        index: u32,
        font: &BitmapFont,
        tint: [u8; 3],
    ) -> Self {
        let atlas = SpriteAtlas::build(device, queue, layout, &[(0, &font.sprites)]);
        Self {
            index,
            atlas,
            glyphs: font.glyphs.clone(),
            solid: font.solid,
            size: [font.size[0] as f32, font.size[1] as f32],
            spacing: [font.spacing[0] as f32, font.spacing[1] as f32],
            offset: [font.offset[0] as f32, font.offset[1] as f32],
            tint: [
                tint[0] as f32 / 255.0,
                tint[1] as f32 / 255.0,
                tint[2] as f32 / 255.0,
            ],
        }
    }

    pub fn page_bind_group(&self, page: u32) -> &wgpu::BindGroup {
        self.atlas.page_bind_group(page)
    }

    pub fn glyph_advance(&self, ch: char) -> f32 {
        if self.solid {
            return self.size[0] + self.spacing[0];
        }
        self.glyphs
            .get(&ch)
            .map(|glyph| glyph.width as f32 + self.spacing[0])
            .unwrap_or(0.0)
    }

    pub fn text_width(&self, text: &str, scale: Vec2) -> f32 {
        text.chars()
            .map(|ch| {
                if ch == '\n' {
                    0.0
                } else {
                    self.glyph_advance(ch) * scale.x
                }
            })
            .sum()
    }

    /// Draws `text` with its anchor at `pos` (top-left for `align = 0`,
    /// centre for `1`, right edge for `2`). `color` is the element colour,
    /// multiplied by the font tint.
    pub fn draw(
        &self,
        painter: &mut Painter,
        text: &str,
        pos: Vec2,
        scale: Vec2,
        align: i32,
        color: [f32; 4],
    ) {
        let color = [
            color[0] * self.tint[0],
            color[1] * self.tint[1],
            color[2] * self.tint[2],
            color[3],
        ];
        let width = self.text_width(text, scale);
        let mut pen = Vec2::new(
            pos.x + self.offset[0] * scale.x
                - match align {
                    1 => width * 0.5,
                    2 => width,
                    _ => 0.0,
                },
            pos.y + self.offset[1] * scale.y,
        );
        let line_height = (self.size[1] + self.spacing[1]) * scale.y;
        for ch in text.chars() {
            if ch == '\n' {
                pen.x = pos.x + self.offset[0] * scale.x
                    - match align {
                        1 => width * 0.5,
                        2 => width,
                        _ => 0.0,
                    };
                pen.y += line_height;
                continue;
            }
            if self.solid {
                if !ch.is_whitespace() {
                    painter.rect(
                        Blend::Normal,
                        pen.x,
                        pen.y,
                        self.size[0] * scale.x,
                        self.size[1] * scale.y,
                        color,
                    );
                }
                pen.x += self.glyph_advance(ch) * scale.x;
                continue;
            }
            let Some(glyph) = self.glyphs.get(&ch) else {
                continue;
            };
            if let Some(sprite) = self.atlas.get(0, glyph.group, glyph.index) {
                let x = pen.x + glyph.offset as f32 * scale.x;
                let w = sprite.width * scale.x;
                let h = sprite.height * scale.y;
                painter.quad(
                    TexRef::ScreenFont(self.index, sprite.page),
                    Blend::Normal,
                    [x, pen.y, x + w, pen.y + h],
                    sprite.uv,
                    false,
                    false,
                    color,
                );
            }
            pen.x += self.glyph_advance(ch) * scale.x;
        }
    }
}

/// Looks a font up by index, or returns `None` so callers fall back to the
/// built-in engine font.
pub fn font<'a>(banks: &'a BTreeMap<u32, ScreenFont>, index: u32) -> Option<&'a ScreenFont> {
    banks.get(&index)
}
