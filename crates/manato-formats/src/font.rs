//! M.U.G.E.N bitmap font (`.fnt`) reader.
//!
//! Two on-disk layouts are supported, matching `docs/spec/render.md` §3:
//!
//! - **v1 binary**: starts with `ElecbyteFnt\0`, embeds a PCX atlas and an INI
//!   text blob holding `[Def]` and a proportional `[Map]`.
//! - **v2 INI**: a text file with `[FNT v2]`, `[Def]`, and an SFF atlas whose
//!   sprites are indexed as `(group = bank, index = character code)`.
//!
//! Both layouts produce the same [`BitmapFont`], which the renderer uploads
//! into a font atlas bank.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use crate::sff::{decode_pcx, SffFile, Sprite};
use crate::text::{parse_pair_i32, split_params, TextFile};

const FNT_MAGIC: &[u8] = b"ElecbyteFnt\0";
const MAX_GLYPHS: usize = 65_536;

/// One glyph cell inside the font atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    /// Atlas sprite group (palette bank for v2 SFF fonts).
    pub group: i32,
    /// Atlas sprite index.
    pub index: i32,
    /// Horizontal offset added before drawing, in font pixels.
    pub offset: i32,
    /// Advance width after the glyph, before `spacing.x`.
    pub width: u16,
}

/// A decoded bitmap font.
#[derive(Debug, Clone, Default)]
pub struct BitmapFont {
    pub size: [u16; 2],
    pub spacing: [i32; 2],
    pub offset: [i32; 2],
    pub color: [u8; 3],
    /// `[Def] Type = 1`: draw solid cells instead of atlas sprites.
    pub solid: bool,
    pub sprites: SffFile,
    pub glyphs: BTreeMap<char, Glyph>,
    /// File the font was loaded from, for diagnostics.
    pub source: Option<PathBuf>,
}

impl BitmapFont {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let data = std::fs::read(path)
            .with_context(|| format!("failed to read font {}", path.display()))?;
        let mut font = Self::from_bytes(&data, path)?;
        font.source = Some(path.to_path_buf());
        Ok(font)
    }

    /// Parses a font from raw bytes. `path` is used to resolve the v2 atlas.
    pub fn from_bytes(data: &[u8], path: &Path) -> anyhow::Result<Self> {
        if data.starts_with(FNT_MAGIC) {
            Self::from_v1(data)
        } else {
            Self::from_v2(data, path)
        }
    }

    pub fn glyph(&self, ch: char) -> Option<&Glyph> {
        self.glyphs.get(&ch)
    }

    /// Width of one glyph cell, used by the renderer when the atlas sprite is
    /// unavailable.
    pub fn cell_width(&self) -> f32 {
        self.size[0] as f32
    }

    pub fn has_glyphs(&self) -> bool {
        !self.glyphs.is_empty() || self.solid
    }

    fn from_v1(data: &[u8]) -> anyhow::Result<Self> {
        if data.len() < 32 {
            bail!("font file is too small for an ElecbyteFnt header");
        }
        let pcx_offset = u32_at(data, 16) as usize;
        let pcx_length = u32_at(data, 20) as usize;
        let text_offset = u32_at(data, 24) as usize;
        let text_length = u32_at(data, 28) as usize;
        if pcx_offset >= data.len() {
            bail!("font PCX offset is outside the file");
        }
        let pcx_end = pcx_offset.saturating_add(pcx_length).min(data.len());
        let pcx = &data[pcx_offset..pcx_end];
        let (atlas_width, atlas_height, atlas_rgba, _, _) =
            decode_pcx(pcx, None).context("failed to decode the font atlas PCX")?;

        let text_end = text_offset.saturating_add(text_length).min(data.len());
        let text = if text_offset < text_end {
            crate::text::decode_text(&data[text_offset..text_end])
        } else {
            String::new()
        };
        let ini = TextFile::parse_str(&text);

        let mut font = Self {
            size: [8, 8],
            ..Self::default()
        };
        if let Some(def) = ini.find_header("def") {
            read_def(&mut font, def);
        }
        if font.size[0] == 0 {
            font.size[0] = 8;
        }
        if font.size[1] == 0 {
            font.size[1] = 8;
        }

        let cell_w = font.size[0] as u32;
        let cell_h = font.size[1] as u32;
        let mut sprites = Vec::new();
        let mut glyphs = BTreeMap::new();

        if let Some(map) = ini.find_header("map") {
            let mut cursor = 0i32;
            for entry in &map.entries {
                if glyphs.len() >= MAX_GLYPHS {
                    break;
                }
                let Some(ch) = parse_map_key(&entry.key) else {
                    continue;
                };
                let parts = split_params(&entry.value);
                let mut offset = parts
                    .first()
                    .and_then(|s| s.trim().parse::<i32>().ok())
                    .unwrap_or(cursor);
                let width = match parts.get(1).and_then(|s| s.trim().parse::<i32>().ok()) {
                    Some(w) if w != 0 => {
                        if w < 0 {
                            offset += w;
                            (-w) as u16
                        } else {
                            w as u16
                        }
                    }
                    _ => cell_w.min(u16::MAX as u32) as u16,
                };
                cursor = offset + width as i32;
                glyphs.insert(
                    ch,
                    Glyph {
                        group: 0,
                        index: ch as i32,
                        offset: 0,
                        width,
                    },
                );
                sprites.push(crop_sprite(
                    &atlas_rgba,
                    atlas_width,
                    atlas_height,
                    offset.max(0) as u32,
                    width as u32,
                    cell_h,
                    ch as i32,
                ));
            }
        }

        if glyphs.is_empty() {
            // No map: assume a 16x16 grid of ASCII cells.
            let cols = (atlas_width / cell_w.max(1)).max(1);
            let rows = (atlas_height / cell_h.max(1)).max(1);
            for code in 0..(cols * rows).min(256) {
                let ch = char::from(code as u8);
                let col = code % cols;
                let row = code / cols;
                if row >= rows {
                    break;
                }
                glyphs.insert(
                    ch,
                    Glyph {
                        group: 0,
                        index: ch as i32,
                        offset: 0,
                        width: cell_w.min(u16::MAX as u32) as u16,
                    },
                );
                sprites.push(crop_sprite(
                    &atlas_rgba,
                    atlas_width,
                    atlas_height,
                    col * cell_w,
                    cell_w,
                    cell_h,
                    ch as i32,
                ));
            }
        }

        font.sprites = SffFile::from_sprites(sprites);
        font.glyphs = glyphs;
        Ok(font)
    }

    fn from_v2(data: &[u8], path: &Path) -> anyhow::Result<Self> {
        let ini = TextFile::parse_bytes(data);
        let mut font = Self {
            size: [8, 8],
            ..Self::default()
        };

        let Some(def) = ini.find_header("def") else {
            bail!("font {} has no [Def] section", path.display());
        };
        read_def(&mut font, def);
        if font.size[0] == 0 {
            font.size[0] = 8;
        }
        if font.size[1] == 0 {
            font.size[1] = 8;
        }
        if font.solid {
            return Ok(font);
        }

        let Some(file) = def.get("file").map(str::trim).filter(|s| !s.is_empty()) else {
            bail!("font {} has no atlas file", path.display());
        };
        let atlas = resolve_font_asset(path, file)
            .with_context(|| format!("font {} references missing atlas {file}", path.display()))?;
        let sff = SffFile::load(&atlas).with_context(|| {
            format!(
                "font {} failed to load atlas {}",
                path.display(),
                atlas.display()
            )
        })?;

        let bank_type = def.get("banktype").unwrap_or("palette").trim();
        let default_bank = def
            .get("bank")
            .and_then(|v| v.trim().parse::<i32>().ok())
            .unwrap_or(0);
        let mut glyphs = BTreeMap::new();
        for sprite in &sff.sprites {
            if glyphs.len() >= MAX_GLYPHS {
                break;
            }
            if sprite.group != default_bank && !bank_type.eq_ignore_ascii_case("sprite") {
                continue;
            }
            let ch = match char::from_u32(sprite.index.max(0) as u32) {
                Some(ch) => ch,
                None => continue,
            };
            if ch.is_control() && ch != ' ' {
                continue;
            }
            glyphs.entry(ch).or_insert(Glyph {
                group: sprite.group,
                index: sprite.index,
                offset: 0,
                width: sprite.width.min(u16::MAX as u32) as u16,
            });
        }

        font.sprites = sff;
        font.glyphs = glyphs;
        Ok(font)
    }
}

fn read_def(font: &mut BitmapFont, def: &crate::text::Section) {
    if let Some(size) = def.get("size") {
        let (w, h) = parse_pair_i32(size);
        font.size = [w.max(0) as u16, h.max(0) as u16];
    }
    if let Some(spacing) = def.get("spacing") {
        let (x, y) = parse_pair_i32(spacing);
        font.spacing = [x, y];
    }
    if let Some(offset) = def.get("offset") {
        let (x, y) = parse_pair_i32(offset);
        font.offset = [x, y];
    }
    if let Some(color) = def.get("color") {
        let parts = split_params(color);
        for (slot, part) in font.color.iter_mut().zip(parts.iter()) {
            *slot = part.trim().parse::<u8>().unwrap_or(*slot);
        }
    }
    if let Some(kind) = def.get("type") {
        font.solid = kind.trim().eq_ignore_ascii_case("1");
    }
}

fn resolve_font_asset(font_path: &Path, file: &str) -> Option<PathBuf> {
    let file = file.trim().trim_matches('"');
    let direct = font_path.parent()?.join(file);
    if direct.is_file() {
        return Some(direct);
    }
    let data = font_path.parent()?.join("data").join(file);
    if data.is_file() {
        return Some(data);
    }
    None
}

fn parse_map_key(key: &str) -> Option<char> {
    let key = key.trim();
    if let Some(hex) = key.strip_prefix("0x").or_else(|| key.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    if let Some(dec) = key.strip_prefix('#') {
        return dec.parse::<u32>().ok().and_then(char::from_u32);
    }
    key.chars().next()
}

fn crop_sprite(
    rgba: &[u8],
    atlas_w: u32,
    atlas_h: u32,
    x: u32,
    width: u32,
    height: u32,
    index: i32,
) -> Sprite {
    let w = width.min(atlas_w.saturating_sub(x)).min(atlas_w);
    let h = height.min(atlas_h);
    let mut out = vec![0u8; (w * h * 4) as usize];
    for row in 0..h {
        for col in 0..w {
            let src = ((row * atlas_w + x + col) * 4) as usize;
            let dst = ((row * w + col) * 4) as usize;
            if src + 3 < rgba.len() {
                out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
            }
        }
    }
    Sprite {
        group: 0,
        index,
        width: w,
        height: h,
        axis_x: 0,
        axis_y: 0,
        rgba: out,
        indices: Vec::new(),
        palette: Vec::new(),
    }
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcx(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; 128];
        out[0] = 0x0A;
        out[1] = 5;
        out[2] = 1;
        out[3] = 8;
        out[8..10].copy_from_slice(&((width - 1) as u16).to_le_bytes());
        out[10..12].copy_from_slice(&((height - 1) as u16).to_le_bytes());
        out[65] = 1;
        out[66..68].copy_from_slice(&(width as u16).to_le_bytes());
        out.extend_from_slice(pixels);
        out.push(0x0C);
        for i in 0..256u32 {
            out.push((i * 7) as u8);
            out.push((i * 3) as u8);
            out.push(i as u8);
        }
        out
    }

    fn v1_font(atlas: &[u8], text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(FNT_MAGIC);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        let header_len = 32usize;
        let pcx_offset = header_len;
        let text_offset = pcx_offset + atlas.len();
        out.extend_from_slice(&(pcx_offset as u32).to_le_bytes());
        out.extend_from_slice(&(atlas.len() as u32).to_le_bytes());
        out.extend_from_slice(&(text_offset as u32).to_le_bytes());
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(atlas);
        out.extend_from_slice(text.as_bytes());
        out
    }

    #[test]
    fn reads_v1_font_with_proportional_map() {
        let pixels: Vec<u8> = (0..(16 * 8)).map(|i| (i % 251) as u8).collect();
        let atlas = pcx(16, 8, &pixels);
        let text = "[Def]\nSize = 8, 8\nSpacing = 1, 1\nColors = 16\n\
                    [Map]\nA = 0, 8\nB = 8, 8\n";
        let bytes = v1_font(&atlas, text);
        let font = BitmapFont::from_bytes(&bytes, Path::new("test.fnt")).unwrap();
        assert_eq!(font.size, [8, 8]);
        assert_eq!(font.spacing, [1, 1]);
        assert_eq!(font.glyph('A').unwrap().width, 8);
        assert_eq!(font.glyph('B').unwrap().index, 'B' as i32);
        assert_eq!(font.sprites.get(0, 'A' as i32).unwrap().width, 8);
        assert_eq!(font.sprites.get(0, 'A' as i32).unwrap().height, 8);
    }

    #[test]
    fn v1_without_map_uses_grid() {
        let pixels: Vec<u8> = vec![1; 16 * 8];
        let atlas = pcx(16, 8, &pixels);
        let bytes = v1_font(&atlas, "[Def]\nSize = 8, 8\n");
        let font = BitmapFont::from_bytes(&bytes, Path::new("test.fnt")).unwrap();
        assert!(font.glyph('\u{1}').is_some());
        assert!(font.glyph('\u{2}').is_none());
        assert_eq!(font.glyph('\u{1}').unwrap().width, 8);
    }

    #[test]
    fn rejects_short_font() {
        assert!(BitmapFont::from_bytes(b"ElecbyteFnt\0", Path::new("x.fnt")).is_err());
    }

    #[test]
    fn parses_v2_header_shape() {
        let text = "[FNT v2]\nversion = 2\n\n[Def]\nType = 1\nSize = 6, 9\n\
                    Spacing = 0, 2\nColor = 255, 128, 0\n";
        let font = BitmapFont::from_bytes(text.as_bytes(), Path::new("font.def")).unwrap();
        assert!(font.solid);
        assert_eq!(font.size, [6, 9]);
        assert_eq!(font.spacing, [0, 2]);
        assert_eq!(font.color, [255, 128, 0]);
    }

    #[test]
    fn map_key_accepts_hex() {
        assert_eq!(parse_map_key("0x41"), Some('A'));
        assert_eq!(parse_map_key("A"), Some('A'));
        assert_eq!(parse_map_key("#66"), Some('B'));
    }
}
