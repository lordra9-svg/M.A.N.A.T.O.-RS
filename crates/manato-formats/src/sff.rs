//! SFF v1 and v2 sprite decoding.
//!
//! This implementation follows `docs/spec/sff.md`, which is derived from the
//! MIT-licensed Ikemen-GO source and public M.U.G.E.N format documentation.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{bail, Context};

const MAX_DIMENSION: u32 = 8192;
const MAX_SPRITES: usize = 65_535;
const MAX_PALETTES: usize = 4096;

/// Decoded sprite with straight RGBA8 pixels.
#[derive(Debug, Clone)]
pub struct Sprite {
    pub group: i32,
    pub index: i32,
    pub width: u32,
    pub height: u32,
    pub axis_x: i32,
    pub axis_y: i32,
    pub rgba: Vec<u8>,
    /// Palette indices for indexed sprites (`RemapPal`); empty for true-color.
    pub indices: Vec<u8>,
    /// Palette the indices were decoded with.
    pub palette: Vec<[u8; 4]>,
}

impl Sprite {
    /// Rebuilds RGBA pixels from the retained index buffer and a palette.
    /// Returns `None` for true-color sprites or palettes that cannot cover
    /// every index.
    pub fn recolor_rgba(&self, palette: &[[u8; 4]]) -> Option<Vec<u8>> {
        if self.indices.len() != (self.width * self.height) as usize || palette.is_empty() {
            return None;
        }
        let mut rgba = vec![0u8; self.indices.len() * 4];
        for (pixel, index) in self.indices.iter().enumerate() {
            let color = palette
                .get(*index as usize)
                .copied()
                .unwrap_or([0, 0, 0, 0]);
            rgba[pixel * 4..pixel * 4 + 4].copy_from_slice(&color);
        }
        Some(rgba)
    }

    pub fn empty(group: i32, index: i32) -> Self {
        Self {
            group,
            index,
            width: 0,
            height: 0,
            axis_x: 0,
            axis_y: 0,
            rgba: Vec::new(),
            indices: Vec::new(),
            palette: Vec::new(),
        }
    }

    fn blank(group: i32, index: i32, width: u32, height: u32, axis_x: i32, axis_y: i32) -> Self {
        Self {
            group,
            index,
            width,
            height,
            axis_x,
            axis_y,
            rgba: vec![0; (width * height * 4) as usize],
            indices: Vec::new(),
            palette: Vec::new(),
        }
    }
}

/// An SFF v1 or v2 sprite file.
#[derive(Debug, Clone, Default)]
pub struct SffFile {
    pub version: u8,
    pub sprites: Vec<Sprite>,
    /// SFF v2 palette nodes keyed by `(group, index)`.
    pub palettes: BTreeMap<(i32, i32), Vec<[u8; 4]>>,
    by_key: HashMap<(i32, i32), usize>,
}

impl SffFile {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let data = std::fs::read(path)
            .with_context(|| format!("failed to read sprite file {}", path.display()))?;
        Self::from_bytes(&data)
    }

    pub fn get(&self, group: i32, index: i32) -> Option<&Sprite> {
        self.by_key.get(&(group, index)).map(|&i| &self.sprites[i])
    }

    pub fn contains(&self, group: i32, index: i32) -> bool {
        self.by_key.contains_key(&(group, index))
    }

    /// Builds a sprite file from decoded sprites, used by the font reader.
    pub fn from_sprites(sprites: Vec<Sprite>) -> Self {
        let mut by_key = HashMap::new();
        for (i, sprite) in sprites.iter().enumerate() {
            by_key.entry((sprite.group, sprite.index)).or_insert(i);
        }
        Self {
            version: 2,
            sprites,
            palettes: BTreeMap::new(),
            by_key,
        }
    }

    pub fn from_bytes(data: &[u8]) -> anyhow::Result<Self> {
        if data.len() < 16 {
            bail!("sprite file is too small");
        }
        if &data[..12] != b"ElecbyteSpr\0" {
            bail!("not an Elecbyte sprite file");
        }
        let version = data[15];
        let mut out = Self {
            version,
            sprites: Vec::new(),
            palettes: BTreeMap::new(),
            by_key: HashMap::new(),
        };
        match version {
            1 => out.parse_v1(data)?,
            2 => out.parse_v2(data)?,
            other => bail!("unsupported SFF version {other}"),
        }
        out.rebuild_keys();
        Ok(out)
    }

    /// Sprite keys in a stable order (used by the atlas packer).
    pub fn keys(&self) -> Vec<(i32, i32)> {
        self.by_key.keys().copied().collect()
    }

    fn rebuild_keys(&mut self) {
        self.by_key.clear();
        for (i, sprite) in self.sprites.iter().enumerate() {
            self.by_key.entry((sprite.group, sprite.index)).or_insert(i);
        }
    }

    fn parse_v1(&mut self, data: &[u8]) -> anyhow::Result<()> {
        if data.len() < 32 {
            bail!("SFF v1 header truncated");
        }
        let count = u32_at(data, 20) as usize;
        let mut offset = u32_at(data, 24) as usize;
        let subheader = (u32_at(data, 28) as usize).max(32);
        let mut previous_palette: Option<Vec<[u8; 4]>> = None;

        for _ in 0..count.min(MAX_SPRITES) {
            let Some(header) = slice(data, offset, 32) else {
                break;
            };
            let next = u32_at(header, 0) as usize;
            let data_len = u32_at(header, 4) as usize;
            let axis_x = i16_at(header, 8) as i32;
            let axis_y = i16_at(header, 10) as i32;
            let group = u16_at(header, 12) as i32;
            let index = u16_at(header, 14) as i32;
            let link = u16_at(header, 16) as usize;
            let same_palette = header[18] != 0;

            // Zero-length entries are links and may sit at end-of-file, so they
            // must be handled before the payload bounds check.
            if data_len == 0 {
                self.push_link(group, index, axis_x, axis_y, link);
                if next == 0 || next <= offset {
                    break;
                }
                offset = next;
                continue;
            }

            let data_start = offset.saturating_add(subheader);
            if data_start >= data.len() {
                break;
            }
            // Payload length comes from `data_length`. Only trim when the
            // declared data overlaps the next header; alignment padding after
            // the data must not be treated as part of the PCX payload.
            let mut data_end = data_start.saturating_add(data_len).min(data.len());
            if next > data_start && next < data_end {
                data_end = next;
            }
            let payload = &data[data_start..data_end];

            if payload.is_empty() {
                self.push_link(group, index, axis_x, axis_y, link);
            } else {
                match decode_pcx(payload, same_palette.then_some(previous_palette.as_deref())) {
                    Ok((width, height, rgba, indices, palette)) => {
                        if let Some(palette) = palette {
                            previous_palette = Some(palette);
                        }
                        self.sprites.push(Sprite {
                            group,
                            index,
                            width,
                            height,
                            axis_x,
                            axis_y,
                            rgba,
                            indices,
                            palette: previous_palette.clone().unwrap_or_default(),
                        });
                    }
                    Err(error) => {
                        log::warn!("SFF v1: cannot decode {group},{index}: {error}");
                        self.sprites
                            .push(Sprite::blank(group, index, 0, 0, axis_x, axis_y));
                    }
                }
            }

            if next == 0 || next <= offset {
                break;
            }
            offset = next;
        }
        Ok(())
    }

    fn parse_v2(&mut self, data: &[u8]) -> anyhow::Result<()> {
        if data.len() < 64 {
            bail!("SFF v2 header truncated");
        }
        let sprite_offset = u32_at(data, 36) as usize;
        let sprite_count = u32_at(data, 40) as usize;
        let palette_offset = u32_at(data, 44) as usize;
        let palette_count = u32_at(data, 48) as usize;
        let ldata_offset = u32_at(data, 52) as usize;
        let tdata_offset = u32_at(data, 60) as usize;
        let minor2 = data[13];

        let (mut palettes, palette_map) =
            parse_v2_palettes(data, palette_offset, palette_count, ldata_offset, minor2);
        self.palettes = palette_map;

        for i in 0..sprite_count.min(MAX_SPRITES) {
            let Some(node) = slice(data, sprite_offset + i * 28, 28) else {
                break;
            };
            let group = u16_at(node, 0) as i32;
            let index = u16_at(node, 2) as i32;
            let width = u16_at(node, 4) as u32;
            let height = u16_at(node, 6) as u32;
            let axis_x = i16_at(node, 8) as i32;
            let axis_y = i16_at(node, 10) as i32;
            let link = u16_at(node, 12) as usize;
            let format = node[14];
            let depth = node[15];
            let data_offset = u32_at(node, 16) as usize;
            let data_len = u32_at(node, 20) as usize;
            let palette_header = u16_at(node, 24) as usize;
            let flags = u16_at(node, 26);

            if data_len == 0 {
                self.push_link(group, index, axis_x, axis_y, link);
                continue;
            }
            if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
                self.sprites.push(Sprite::empty(group, index));
                continue;
            }

            let base = if flags & 1 != 0 {
                tdata_offset
            } else {
                ldata_offset
            };
            let Some(payload) = slice(data, base.saturating_add(data_offset), data_len) else {
                log::warn!("SFF v2: sprite data out of bounds for {group},{index}");
                self.sprites
                    .push(Sprite::blank(group, index, width, height, axis_x, axis_y));
                continue;
            };
            // Missing or broken palette nodes fall back to palette 0, matching
            // Ikemen-GO's out-of-range palette clamping.
            let palette = palettes
                .get(palette_header)
                .and_then(|p| p.as_deref())
                .or_else(|| palettes.first().and_then(|p| p.as_deref()))
                .unwrap_or(&[]);

            let (out_w, out_h, rgba, indices) = match format {
                0 => {
                    let (w, h, rgba, indices) = decode_raw(payload, width, height, depth, palette);
                    (w, h, rgba, indices)
                }
                2 => {
                    let (w, h, rgba, indices) =
                        decode_indexed(payload, width, height, palette, Codec::Rle8);
                    (w, h, rgba, indices)
                }
                3 => {
                    let (w, h, rgba, indices) =
                        decode_indexed(payload, width, height, palette, Codec::Rle5);
                    (w, h, rgba, indices)
                }
                4 => {
                    let (w, h, rgba, indices) =
                        decode_indexed(payload, width, height, palette, Codec::Lz5);
                    (w, h, rgba, indices)
                }
                10..=12 => match decode_png(payload.get(4..).unwrap_or(&[]), palette) {
                    Ok((w, h, pixels, indices)) => (w, h, pixels, indices.unwrap_or_default()),
                    Err(error) => {
                        log::warn!("SFF v2: PNG decode failed for {group},{index}: {error}");
                        (
                            width,
                            height,
                            vec![0; (width * height * 4) as usize],
                            Vec::new(),
                        )
                    }
                },
                other => {
                    log::warn!("SFF v2: unsupported pixel format {other} for {group},{index}");
                    (
                        width,
                        height,
                        vec![0; (width * height * 4) as usize],
                        Vec::new(),
                    )
                }
            };

            self.sprites.push(Sprite {
                group,
                index,
                width: out_w,
                height: out_h,
                axis_x,
                axis_y,
                rgba,
                indices,
                palette: palette.to_vec(),
            });
        }

        // Palettes are parsed once; drop the temporary ownership.
        palettes.clear();
        Ok(())
    }

    fn push_link(&mut self, group: i32, index: i32, axis_x: i32, axis_y: i32, link: usize) {
        if let Some(source) = self.sprites.get(link).cloned() {
            self.sprites.push(Sprite {
                group,
                index,
                axis_x,
                axis_y,
                ..source
            });
        } else {
            self.sprites.push(Sprite::empty(group, index));
        }
    }
}

fn parse_v2_palettes(
    data: &[u8],
    palette_offset: usize,
    palette_count: usize,
    ldata_offset: usize,
    minor2: u8,
) -> V2Palettes {
    let mut palettes: Vec<Option<Vec<[u8; 4]>>> = vec![None; palette_count.min(MAX_PALETTES)];
    let mut by_key = BTreeMap::new();
    for i in 0..palettes.len() {
        let Some(node) = slice(data, palette_offset + i * 16, 16) else {
            break;
        };
        let group = u16_at(node, 0) as i32;
        let index = u16_at(node, 2) as i32;
        let colors = u16_at(node, 4) as usize;
        let link = u16_at(node, 6) as usize;
        let data_offset = u32_at(node, 8) as usize;
        let data_len = u32_at(node, 12) as usize;

        if data_len == 0 {
            if let Some(Some(source)) = palettes.get(link).cloned() {
                by_key.insert((group, index), source.clone());
                palettes[i] = Some(source);
            }
            continue;
        }
        let Some(raw) = slice(data, ldata_offset.saturating_add(data_offset), data_len) else {
            continue;
        };
        let count = colors.min(256).min(raw.len() / 4);
        let mut palette = Vec::with_capacity(count);
        for c in 0..count {
            let p = &raw[c * 4..c * 4 + 4];
            let mut alpha = p[3];
            if minor2 == 0 {
                alpha = if c == 0 { 0 } else { 255 };
            }
            palette.push([p[2], p[1], p[0], alpha]);
        }
        by_key.insert((group, index), palette.clone());
        palettes[i] = Some(palette);
    }
    (palettes, by_key)
}

enum Codec {
    Rle8,
    Rle5,
    Lz5,
}

fn decode_indexed(
    raw: &[u8],
    width: u32,
    height: u32,
    palette: &[[u8; 4]],
    codec: Codec,
) -> (u32, u32, Vec<u8>, Vec<u8>) {
    let count = (width * height) as usize;
    let mut indices = vec![0u8; count];
    if raw.len() < 4 {
        return (width, height, vec![0; count * 4], indices);
    }
    let stream = &raw[4..];
    let ok = match codec {
        Codec::Rle8 => rle8_decode(stream, &mut indices),
        Codec::Rle5 => rle5_decode(stream, &mut indices),
        Codec::Lz5 => lz5_decode(stream, &mut indices),
    };
    if !ok {
        log::warn!("SFF v2: indexed decode stopped early");
    }
    let mut rgba = vec![0u8; count * 4];
    for (i, &index) in indices.iter().enumerate() {
        let color = palette.get(index as usize).copied().unwrap_or([0, 0, 0, 0]);
        rgba[i * 4..i * 4 + 4].copy_from_slice(&color);
    }
    (width, height, rgba, indices)
}

fn decode_raw(
    raw: &[u8],
    width: u32,
    height: u32,
    depth: u8,
    palette: &[[u8; 4]],
) -> (u32, u32, Vec<u8>, Vec<u8>) {
    let count = (width * height) as usize;
    let mut rgba = vec![0u8; count * 4];
    let mut indices = Vec::new();
    match depth {
        8 => {
            indices = vec![0u8; count];
            for i in 0..count {
                let index = raw.get(i).copied().unwrap_or(0);
                indices[i] = index;
                let color = palette.get(index as usize).copied().unwrap_or([0, 0, 0, 0]);
                rgba[i * 4..i * 4 + 4].copy_from_slice(&color);
            }
        }
        24 | 32 => {
            let step = if depth == 32 { 4 } else { 3 };
            for i in 0..count {
                let at = i * step;
                if at + step > raw.len() {
                    break;
                }
                rgba[i * 4] = raw[at + 2];
                rgba[i * 4 + 1] = raw[at + 1];
                rgba[i * 4 + 2] = raw[at];
                rgba[i * 4 + 3] = if depth == 32 { raw[at + 3] } else { 255 };
            }
        }
        _ => {}
    }
    (width, height, rgba, indices)
}

type PaletteMap = BTreeMap<(i32, i32), Vec<[u8; 4]>>;
type V2Palettes = (Vec<Option<Vec<[u8; 4]>>>, PaletteMap);
type PngPixels = (u32, u32, Vec<u8>, Option<Vec<u8>>);

pub(crate) type DecodedPcx = (u32, u32, Vec<u8>, Vec<u8>, Option<Vec<[u8; 4]>>);

pub(crate) fn decode_pcx(
    data: &[u8],
    reuse_palette: Option<Option<&[[u8; 4]]>>,
) -> anyhow::Result<DecodedPcx> {
    if data.len() < 128 || data[0] != 0x0A {
        bail!("invalid PCX header");
    }
    if data[2] != 1 || data[3] != 8 || data[65] != 1 {
        bail!("unsupported PCX encoding");
    }
    let x_min = u16_at(data, 4) as i32;
    let y_min = u16_at(data, 6) as i32;
    let x_max = u16_at(data, 8) as i32;
    let y_max = u16_at(data, 10) as i32;
    let width = x_max - x_min + 1;
    let height = y_max - y_min + 1;
    if width <= 0 || height <= 0 || width > MAX_DIMENSION as i32 || height > MAX_DIMENSION as i32 {
        bail!("invalid PCX dimensions");
    }
    let bytes_per_line = (u16_at(data, 66) as usize).max(width as usize);

    let mut indices = vec![0u8; (width * height) as usize];
    let mut pos = 128usize;
    for y in 0..height as usize {
        let mut row = vec![0u8; bytes_per_line];
        let mut x = 0usize;
        while x < bytes_per_line {
            if pos >= data.len() {
                bail!("truncated PCX RLE stream");
            }
            let byte = data[pos];
            pos += 1;
            if byte & 0xC0 == 0xC0 {
                let run = (byte & 0x3F) as usize;
                if pos >= data.len() {
                    bail!("truncated PCX run");
                }
                let value = data[pos];
                pos += 1;
                for _ in 0..run {
                    if x < bytes_per_line {
                        row[x] = value;
                        x += 1;
                    }
                }
            } else {
                row[x] = byte;
                x += 1;
            }
        }
        let copy = width.min(bytes_per_line as i32) as usize;
        indices[y * width as usize..y * width as usize + copy].copy_from_slice(&row[..copy]);
    }

    let palette = if let Some(Some(existing)) = reuse_palette {
        Some(existing.to_vec())
    } else {
        extract_pcx_palette(data)
    };
    let palette = palette.unwrap_or_else(default_palette);
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for (i, &index) in indices.iter().enumerate() {
        let color = palette.get(index as usize).copied().unwrap_or([0, 0, 0, 0]);
        rgba[i * 4..i * 4 + 4].copy_from_slice(&color);
    }
    Ok((width as u32, height as u32, rgba, indices, Some(palette)))
}

fn extract_pcx_palette(data: &[u8]) -> Option<Vec<[u8; 4]>> {
    if data.len() < 769 {
        return None;
    }
    let start = data.len() - 769;
    let marker = (128..=start).rev().find(|&i| data[i] == 0x0C)?;
    let rgb = &data[marker + 1..];
    if rgb.len() < 768 {
        return None;
    }
    let mut palette = Vec::with_capacity(256);
    for i in 0..256 {
        let at = i * 3;
        palette.push([
            rgb[at],
            rgb[at + 1],
            rgb[at + 2],
            if i == 0 { 0 } else { 255 },
        ]);
    }
    Some(palette)
}

fn default_palette() -> Vec<[u8; 4]> {
    (0..256)
        .map(|i| {
            let value = i as u8;
            [value, value, value, if i == 0 { 0 } else { 255 }]
        })
        .collect()
}

fn decode_png(raw: &[u8], palette: &[[u8; 4]]) -> anyhow::Result<PngPixels> {
    use std::io::Cursor;

    let decoder = png::Decoder::new(Cursor::new(raw));
    let mut reader = decoder.read_info()?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer)?;
    let data = &buffer[..info.buffer_size()];
    let width = info.width;
    let height = info.height;
    let mut rgba = vec![0u8; (width * height * 4) as usize];

    let mut indices = None;
    match info.color_type {
        png::ColorType::Indexed => {
            if info.bit_depth != png::BitDepth::Eight {
                bail!("unsupported indexed PNG depth {:?}", info.bit_depth);
            }
            let mut decoded = vec![0u8; (width * height) as usize];
            for i in 0..(width * height) as usize {
                let index = data.get(i).copied().unwrap_or(0);
                decoded[i] = index;
                let color = palette.get(index as usize).copied().unwrap_or([0, 0, 0, 0]);
                rgba[i * 4..i * 4 + 4].copy_from_slice(&color);
            }
            indices = Some(decoded);
        }
        png::ColorType::Rgba => rgba.copy_from_slice(data),
        png::ColorType::Rgb => {
            for (i, pixel) in data.as_chunks::<3>().0.iter().enumerate() {
                rgba[i * 4..i * 4 + 3].copy_from_slice(pixel);
                rgba[i * 4 + 3] = 255;
            }
        }
        png::ColorType::Grayscale => {
            for (i, value) in data.iter().enumerate() {
                rgba[i * 4] = *value;
                rgba[i * 4 + 1] = *value;
                rgba[i * 4 + 2] = *value;
                rgba[i * 4 + 3] = 255;
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for (i, pixel) in data.as_chunks::<2>().0.iter().enumerate() {
                rgba[i * 4] = pixel[0];
                rgba[i * 4 + 1] = pixel[0];
                rgba[i * 4 + 2] = pixel[0];
                rgba[i * 4 + 3] = pixel[1];
            }
        }
    }
    Ok((width, height, rgba, indices))
}

pub fn rle8_decode(src: &[u8], dst: &mut [u8]) -> bool {
    let mut source = 0usize;
    let mut target = 0usize;
    while target < dst.len() {
        if source >= src.len() {
            return false;
        }
        let control = src[source];
        source += 1;
        let (value, run) = if control & 0xC0 == 0x40 {
            if source >= src.len() {
                return false;
            }
            let value = src[source];
            source += 1;
            (value, (control & 0x3F) as usize)
        } else {
            (control, 1)
        };
        for _ in 0..run {
            if target >= dst.len() {
                break;
            }
            dst[target] = value;
            target += 1;
        }
    }
    true
}

pub fn rle5_decode(src: &[u8], dst: &mut [u8]) -> bool {
    let mut source = 0usize;
    let mut target = 0usize;
    while target < dst.len() {
        if source >= src.len() {
            return false;
        }
        let mut run = src[source] as i32;
        source += 1;
        if source >= src.len() {
            return false;
        }
        let mut extra = (src[source] & 0x7F) as i32;
        let mut value = 0u8;
        if src[source] & 0x80 != 0 {
            if source + 1 >= src.len() {
                return false;
            }
            value = src[source + 1];
            source += 1;
        }
        source += 1;

        loop {
            if target < dst.len() {
                dst[target] = value;
                target += 1;
            }
            run -= 1;
            if run >= 0 {
                continue;
            }
            extra -= 1;
            if extra < 0 {
                break;
            }
            if source >= src.len() {
                return false;
            }
            value = src[source] & 0x1F;
            run = (src[source] >> 5) as i32;
            source += 1;
        }
    }
    true
}

pub fn lz5_decode(src: &[u8], dst: &mut [u8]) -> bool {
    if src.is_empty() {
        return false;
    }
    let mut source = 1usize;
    let mut target = 0usize;
    let mut control = src[0];
    let mut control_bit = 0u32;
    let mut reference = 0u32;
    let mut reference_bits = 0u32;

    while target < dst.len() {
        if source >= src.len() {
            return false;
        }
        let data = src[source] as i32;
        source += 1;

        if control & (1u8 << control_bit) != 0 {
            let (offset, run) = if data & 0x3F == 0 {
                if source + 1 >= src.len() {
                    return false;
                }
                let offset = ((data << 2) | src[source] as i32) + 1;
                source += 1;
                let run = src[source] as i32 + 2;
                source += 1;
                (offset, run)
            } else {
                reference |= ((data & 0xC0) as u32) >> reference_bits;
                reference_bits += 2;
                let run = data & 0x3F;
                let offset = if reference_bits < 8 {
                    if source >= src.len() {
                        return false;
                    }
                    let offset = src[source] as i32 + 1;
                    source += 1;
                    offset
                } else {
                    let offset = reference as i32 + 1;
                    reference = 0;
                    reference_bits = 0;
                    offset
                };
                (offset, run)
            };

            if offset <= 0 || offset as usize > target {
                return false;
            }
            for _ in 0..=run {
                if target >= dst.len() {
                    break;
                }
                dst[target] = dst[target - offset as usize];
                target += 1;
            }
        } else {
            let (value, run) = if data & 0xE0 == 0 {
                if source >= src.len() {
                    return false;
                }
                let run = src[source] as i32 + 8;
                source += 1;
                (data & 0x1F, run)
            } else {
                (data & 0x1F, data >> 5)
            };
            for _ in 0..run {
                if target >= dst.len() {
                    break;
                }
                dst[target] = value as u8;
                target += 1;
            }
        }

        control_bit += 1;
        if control_bit >= 8 {
            if source >= src.len() {
                break;
            }
            control = src[source];
            source += 1;
            control_bit = 0;
        }
    }
    target == dst.len()
}

fn slice(data: &[u8], offset: usize, length: usize) -> Option<&[u8]> {
    let end = offset.checked_add(length)?;
    data.get(offset..end)
}

fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn i16_at(data: &[u8], at: usize) -> i16 {
    u16_at(data, at) as i16
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle8_roundtrip() {
        let mut out = [0u8; 6];
        assert!(rle8_decode(&[0x40 | 6, 7], &mut out));
        assert_eq!(out, [7; 6]);
    }

    #[test]
    fn rle8_rejects_truncation() {
        let mut out = [0u8; 6];
        assert!(!rle8_decode(&[0x40 | 6], &mut out));
    }

    #[test]
    fn lz5_literal_run() {
        let mut out = [9u8; 1];
        assert!(lz5_decode(&[0, 0x20], &mut out));
        assert_eq!(out, [0]);
    }

    #[test]
    fn lz5_rejects_bad_reference() {
        let mut out = [0u8; 4];
        assert!(!lz5_decode(&[0x01, 0x02, 0x01], &mut out));
    }

    #[test]
    fn v2_indexed_sprites_skip_the_size_prefix() {
        let mut data = vec![0u8; 512];
        data[..12].copy_from_slice(b"ElecbyteSpr\0");
        data[13] = 1;
        data[15] = 2;

        let palette_offset = 68usize;
        let ldata_offset = palette_offset + 16;
        let palette_len = 32 * 4;
        let sprite_offset = ldata_offset + palette_len;
        let sprite_data_offset = sprite_offset + 28;

        let put_u16 = |data: &mut [u8], at: usize, v: u16| {
            data[at..at + 2].copy_from_slice(&v.to_le_bytes());
        };
        let put_u32 = |data: &mut [u8], at: usize, v: u32| {
            data[at..at + 4].copy_from_slice(&v.to_le_bytes());
        };

        let total = data.len() as u32;
        put_u32(&mut data, 36, sprite_offset as u32);
        put_u32(&mut data, 40, 1);
        put_u32(&mut data, 44, palette_offset as u32);
        put_u32(&mut data, 48, 1);
        put_u32(&mut data, 52, ldata_offset as u32);
        put_u32(&mut data, 60, total);

        put_u16(&mut data, palette_offset, 1);
        put_u16(&mut data, palette_offset + 2, 1);
        put_u16(&mut data, palette_offset + 4, 32);
        put_u32(&mut data, palette_offset + 8, 0);
        put_u32(&mut data, palette_offset + 12, palette_len as u32);
        for c in 0..32 {
            let at = ldata_offset + c * 4;
            data[at..at + 4].copy_from_slice(&[30, 20, 10, 255]);
        }

        put_u16(&mut data, sprite_offset, 0);
        put_u16(&mut data, sprite_offset + 2, 0);
        put_u16(&mut data, sprite_offset + 4, 2);
        put_u16(&mut data, sprite_offset + 6, 2);
        data[sprite_offset + 14] = 4;
        data[sprite_offset + 15] = 5;
        put_u32(
            &mut data,
            sprite_offset + 16,
            (sprite_data_offset - ldata_offset) as u32,
        );
        put_u32(&mut data, sprite_offset + 20, 6);
        data[sprite_data_offset..sprite_data_offset + 6].copy_from_slice(&[4, 0, 0, 0, 0x00, 0x80]);

        let sff = SffFile::from_bytes(&data).expect("parse sff");
        let sprite = sff.get(0, 0).expect("sprite");
        assert_eq!((sprite.width, sprite.height), (2, 2));
        assert_eq!(sprite.rgba, [10, 20, 30, 255].repeat(4));
    }

    #[test]
    fn v2_png32_sprite() {
        let mut data = vec![0u8; 512];
        data[..12].copy_from_slice(b"ElecbyteSpr\0");
        data[15] = 2;

        let sprite_offset = 512usize;
        let ldata_offset = sprite_offset + 28;
        let png = {
            let mut raw = vec![0u8; 2 * 2 * 4];
            for (i, pixel) in raw.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                pixel.copy_from_slice(&[(i * 40) as u8, 20, 30, 255]);
            }
            let mut encoded = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut encoded, 2, 2);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                writer.write_image_data(&raw).unwrap();
            }
            encoded
        };
        let payload = [&(2u32 * 2 * 4).to_le_bytes()[..], &png].concat();
        let total = ldata_offset + payload.len();
        data.resize(total, 0);
        data[36..40].copy_from_slice(&(sprite_offset as u32).to_le_bytes());
        data[40..44].copy_from_slice(&1u32.to_le_bytes());
        data[44..48].copy_from_slice(&0u32.to_le_bytes());
        data[48..52].copy_from_slice(&0u32.to_le_bytes());
        data[52..56].copy_from_slice(&(ldata_offset as u32).to_le_bytes());
        data[60..64].copy_from_slice(&(ldata_offset as u32).to_le_bytes());
        data[sprite_offset..sprite_offset + 2].copy_from_slice(&0u16.to_le_bytes());
        data[sprite_offset + 2..sprite_offset + 4].copy_from_slice(&0u16.to_le_bytes());
        data[sprite_offset + 4..sprite_offset + 6].copy_from_slice(&2u16.to_le_bytes());
        data[sprite_offset + 6..sprite_offset + 8].copy_from_slice(&2u16.to_le_bytes());
        data[sprite_offset + 14] = 12;
        data[sprite_offset + 15] = 32;
        data[sprite_offset + 16..sprite_offset + 20].copy_from_slice(&0u32.to_le_bytes());
        data[sprite_offset + 20..sprite_offset + 24]
            .copy_from_slice(&(payload.len() as u32).to_le_bytes());
        data[ldata_offset..ldata_offset + payload.len()].copy_from_slice(&payload);

        let sff = SffFile::from_bytes(&data).expect("parse sff");
        let sprite = sff.get(0, 0).expect("sprite");
        assert_eq!((sprite.width, sprite.height), (2, 2));
        assert_eq!(sprite.rgba.len(), 16);
        assert_eq!(&sprite.rgba[4..8], &[40, 20, 30, 255]);
    }

    #[test]
    fn v2_linked_sprite_overrides_axis() {
        let mut data = vec![0u8; 512];
        data[..12].copy_from_slice(b"ElecbyteSpr\0");
        data[15] = 2;

        let sprite_offset = 512usize;
        let ldata_offset = sprite_offset + 2 * 28;
        let payload = [&(1u32).to_le_bytes()[..], &[1u8]].concat();
        let total = ldata_offset + payload.len();
        data.resize(total, 0);
        data[36..40].copy_from_slice(&(sprite_offset as u32).to_le_bytes());
        data[40..44].copy_from_slice(&2u32.to_le_bytes());
        data[44..48].copy_from_slice(&0u32.to_le_bytes());
        data[48..52].copy_from_slice(&0u32.to_le_bytes());
        data[52..56].copy_from_slice(&(ldata_offset as u32).to_le_bytes());
        data[60..64].copy_from_slice(&(total as u32).to_le_bytes());

        let node = sprite_offset;
        data[node..node + 2].copy_from_slice(&0u16.to_le_bytes());
        data[node + 2..node + 4].copy_from_slice(&0u16.to_le_bytes());
        data[node + 4..node + 6].copy_from_slice(&1u16.to_le_bytes());
        data[node + 6..node + 8].copy_from_slice(&1u16.to_le_bytes());
        data[node + 8..node + 10].copy_from_slice(&3i16.to_le_bytes());
        data[node + 10..node + 12].copy_from_slice(&4i16.to_le_bytes());
        data[node + 14] = 0;
        data[node + 15] = 8;
        data[node + 16..node + 20].copy_from_slice(&0u32.to_le_bytes());
        data[node + 20..node + 24].copy_from_slice(&(payload.len() as u32).to_le_bytes());

        let linked = sprite_offset + 28;
        data[linked..linked + 2].copy_from_slice(&1u16.to_le_bytes());
        data[linked + 2..linked + 4].copy_from_slice(&2u16.to_le_bytes());
        data[linked + 4..linked + 6].copy_from_slice(&1u16.to_le_bytes());
        data[linked + 6..linked + 8].copy_from_slice(&1u16.to_le_bytes());
        data[linked + 8..linked + 10].copy_from_slice(&9i16.to_le_bytes());
        data[linked + 10..linked + 12].copy_from_slice(&10i16.to_le_bytes());
        data[linked + 12..linked + 14].copy_from_slice(&0u16.to_le_bytes());
        data[ldata_offset..ldata_offset + payload.len()].copy_from_slice(&payload);

        let sff = SffFile::from_bytes(&data).expect("parse sff");
        let sprite = sff.get(1, 2).expect("linked sprite");
        assert_eq!((sprite.width, sprite.height), (1, 1));
        assert_eq!((sprite.axis_x, sprite.axis_y), (9, 10));
    }
    fn pcx(indices: &[u8], width: u16, height: u16, palette: Option<&[[u8; 3]]>) -> Vec<u8> {
        let mut out = vec![0u8; 128];
        out[0] = 0x0A; // PCX magic
        out[2] = 1; // RLE
        out[3] = 8; // 8-bit
        out[4..6].copy_from_slice(&0u16.to_le_bytes());
        out[6..8].copy_from_slice(&0u16.to_le_bytes());
        out[8..10].copy_from_slice(&(width - 1).to_le_bytes());
        out[10..12].copy_from_slice(&(height - 1).to_le_bytes());
        out[65] = 1; // one color plane
        out[66..68].copy_from_slice(&width.to_le_bytes());

        // Rows are stored uncompressed when every index is < 0xC0.
        for &index in indices {
            out.push(index);
        }
        if let Some(palette) = palette {
            out.push(0x0C);
            for color in palette.iter().take(256) {
                out.extend_from_slice(color);
            }
        }
        out
    }

    fn v1_sprite(next: u32, group: u16, index: u16, same_palette: bool, payload: &[u8]) -> Vec<u8> {
        let mut header = vec![0u8; 32];
        header[0..4].copy_from_slice(&next.to_le_bytes());
        header[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        header[8..10].copy_from_slice(&0i16.to_le_bytes());
        header[10..12].copy_from_slice(&0i16.to_le_bytes());
        header[12..14].copy_from_slice(&group.to_le_bytes());
        header[14..16].copy_from_slice(&index.to_le_bytes());
        header[18] = u8::from(same_palette);
        let mut out = header;
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn v1_same_palette_sprites_reuse_the_previous_palette() {
        let mut palette = vec![[0u8, 0, 0]; 256];
        palette[0] = [0, 0, 0];
        palette[1] = [255, 0, 0];
        palette[2] = [0, 255, 0];
        palette[3] = [0, 0, 255];

        let first = pcx(&[1, 2], 2, 1, Some(&palette));
        let second = pcx(&[3], 1, 1, None);

        let mut data = vec![0u8; 32];
        data[..12].copy_from_slice(b"ElecbyteSpr\0");
        data[15] = 1;
        data[20..24].copy_from_slice(&2u32.to_le_bytes());
        data[24..28].copy_from_slice(&32u32.to_le_bytes());
        data[28..32].copy_from_slice(&32u32.to_le_bytes());

        let first_offset = data.len() as u32;
        let second_offset = first_offset + 32 + first.len() as u32;
        data.extend_from_slice(&v1_sprite(second_offset, 0, 0, false, &first));
        data.extend_from_slice(&v1_sprite(0, 0, 1, true, &second));

        let sff = SffFile::from_bytes(&data).expect("parse v1 palette reuse");
        let first_sprite = sff.get(0, 0).expect("first sprite");
        assert_eq!(&first_sprite.rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&first_sprite.rgba[4..8], &[0, 255, 0, 255]);
        // Index/palette data is retained for RemapPal.
        assert_eq!(first_sprite.indices, vec![1, 2]);
        assert_eq!(first_sprite.palette[1], [255, 0, 0, 255]);
        let mut swapped = first_sprite.palette.clone();
        swapped[1] = [10, 20, 30, 255];
        swapped[2] = [40, 50, 60, 255];
        assert_eq!(
            first_sprite.recolor_rgba(&swapped).expect("recolor"),
            vec![10, 20, 30, 255, 40, 50, 60, 255]
        );

        let second_sprite = sff.get(0, 1).expect("same-palette sprite");
        assert_eq!(&second_sprite.rgba[0..4], &[0, 0, 255, 255]);
    }
    #[test]
    fn v1_same_palette_without_previous_palette_uses_defaults() {
        let payload = pcx(&[1], 1, 1, None);
        let mut data = vec![0u8; 32];
        data[..12].copy_from_slice(b"ElecbyteSpr\0");
        data[15] = 1;
        data[20..24].copy_from_slice(&1u32.to_le_bytes());
        data[24..28].copy_from_slice(&32u32.to_le_bytes());
        data[28..32].copy_from_slice(&32u32.to_le_bytes());
        data.extend_from_slice(&v1_sprite(0, 1, 1, true, &payload));

        let sff = SffFile::from_bytes(&data).expect("parse v1 default palette");
        let sprite = sff.get(1, 1).expect("sprite");
        assert_eq!(&sprite.rgba[0..4], &[1, 1, 1, 255]);
        assert_eq!(sff.get(1, 1).map(|s| (s.width, s.height)), Some((1, 1)));
    }
    #[test]
    fn v1_palette_ignores_padding_outside_declared_data() {
        let mut palette = vec![[0u8, 0, 0]; 256];
        // 0x0C in the palette RGB would confuse a backwards marker scan if
        // the trailing padding were included in the payload.
        palette[0] = [12, 0, 0];
        palette[1] = [90, 80, 70];
        let payload = pcx(&[1], 1, 1, Some(&palette));

        let mut data = vec![0u8; 32];
        data[..12].copy_from_slice(b"ElecbyteSpr\0");
        data[15] = 1;
        data[20..24].copy_from_slice(&1u32.to_le_bytes());
        data[24..28].copy_from_slice(&32u32.to_le_bytes());
        data[28..32].copy_from_slice(&32u32.to_le_bytes());

        let padding = [0u8; 4];
        let data_start = 32u32 + 32;
        let next = data_start + payload.len() as u32 + padding.len() as u32;
        data.extend_from_slice(&v1_sprite(next, 1, 1, false, &payload));
        data.extend_from_slice(&padding);

        let sff = SffFile::from_bytes(&data).expect("parse v1 padded payload");
        let sprite = sff.get(1, 1).expect("sprite");
        assert_eq!(&sprite.rgba[0..4], &[90, 80, 70, 255]);
    }

    #[derive(Clone)]
    struct V2Node {
        group: u16,
        index: u16,
        width: u16,
        height: u16,
        axis_x: i16,
        axis_y: i16,
        link: u16,
        format: u8,
        depth: u8,
        palette: u16,
        tdata: bool,
        payload: Option<Vec<u8>>,
    }

    enum V2Palette {
        Colors(Vec<[u8; 4]>),
        Link(u16),
    }

    fn build_v2(nodes: &[V2Node], palettes: &[V2Palette]) -> Vec<u8> {
        let sprite_offset = 512usize;
        let palette_offset = sprite_offset + nodes.len() * 28;
        let palette_data: usize = palettes
            .iter()
            .map(|palette| match palette {
                V2Palette::Colors(colors) => colors.len() * 4,
                V2Palette::Link(_) => 0,
            })
            .sum();
        let ldata_offset = palette_offset + palettes.len() * 16 + palette_data;
        let ldata_payload: usize = nodes
            .iter()
            .filter(|node| !node.tdata)
            .map(|node| node.payload.as_ref().map(Vec::len).unwrap_or(0))
            .sum();
        let tdata_payload: usize = nodes
            .iter()
            .filter(|node| node.tdata)
            .map(|node| node.payload.as_ref().map(Vec::len).unwrap_or(0))
            .sum();
        let tdata_offset = ldata_offset + palette_data + ldata_payload;
        let total = tdata_offset + tdata_payload;
        let mut data = vec![0u8; total];
        data[..12].copy_from_slice(b"ElecbyteSpr\0");
        data[13] = 0;
        data[15] = 2;
        data[36..40].copy_from_slice(&(sprite_offset as u32).to_le_bytes());
        data[40..44].copy_from_slice(&(nodes.len() as u32).to_le_bytes());
        data[44..48].copy_from_slice(&(palette_offset as u32).to_le_bytes());
        data[48..52].copy_from_slice(&(palettes.len() as u32).to_le_bytes());
        data[52..56].copy_from_slice(&(ldata_offset as u32).to_le_bytes());
        data[60..64].copy_from_slice(&(tdata_offset as u32).to_le_bytes());

        let mut colors_at = ldata_offset;
        for (i, palette) in palettes.iter().enumerate() {
            let node = palette_offset + i * 16;
            data[node..node + 2].copy_from_slice(&3u16.to_le_bytes());
            data[node + 2..node + 4].copy_from_slice(&(7u16 + i as u16).to_le_bytes());
            match palette {
                V2Palette::Colors(colors) => {
                    data[node + 4..node + 6].copy_from_slice(&(colors.len() as u16).to_le_bytes());
                    data[node + 8..node + 12]
                        .copy_from_slice(&((colors_at - ldata_offset) as u32).to_le_bytes());
                    let bytes = (colors.len() * 4) as u32;
                    data[node + 12..node + 16].copy_from_slice(&bytes.to_le_bytes());
                    for color in colors {
                        data[colors_at..colors_at + 4]
                            .copy_from_slice(&[color[2], color[1], color[0], color[3]]);
                        colors_at += 4;
                    }
                }
                V2Palette::Link(link) => {
                    data[node + 6..node + 8].copy_from_slice(&link.to_le_bytes());
                }
            }
        }

        let mut ldata_at = ldata_offset + palette_data;
        let mut tdata_at = tdata_offset;
        for (i, node) in nodes.iter().enumerate() {
            let at = sprite_offset + i * 28;
            data[at..at + 2].copy_from_slice(&node.group.to_le_bytes());
            data[at + 2..at + 4].copy_from_slice(&node.index.to_le_bytes());
            data[at + 4..at + 6].copy_from_slice(&node.width.to_le_bytes());
            data[at + 6..at + 8].copy_from_slice(&node.height.to_le_bytes());
            data[at + 8..at + 10].copy_from_slice(&node.axis_x.to_le_bytes());
            data[at + 10..at + 12].copy_from_slice(&node.axis_y.to_le_bytes());
            data[at + 12..at + 14].copy_from_slice(&node.link.to_le_bytes());
            data[at + 14] = node.format;
            data[at + 15] = node.depth;
            data[at + 24..at + 26].copy_from_slice(&node.palette.to_le_bytes());
            if let Some(payload) = &node.payload {
                let (base, offset) = if node.tdata {
                    data[at + 26..at + 28].copy_from_slice(&1u16.to_le_bytes());
                    (tdata_at, tdata_offset)
                } else {
                    (ldata_at, ldata_offset)
                };
                data[at + 16..at + 20].copy_from_slice(&((base - offset) as u32).to_le_bytes());
                data[at + 20..at + 24].copy_from_slice(&(payload.len() as u32).to_le_bytes());
                data[base..base + payload.len()].copy_from_slice(payload);
                if node.tdata {
                    tdata_at += payload.len();
                } else {
                    ldata_at += payload.len();
                }
            }
        }
        data
    }

    fn palette_colors() -> Vec<[u8; 4]> {
        vec![[0, 0, 0, 0], [10, 20, 30, 255], [40, 50, 60, 255]]
    }

    fn node(group: u16, index: u16, link: u16, payload: Option<Vec<u8>>) -> V2Node {
        V2Node {
            group,
            index,
            width: if payload.is_some() { 1 } else { 0 },
            height: if payload.is_some() { 1 } else { 0 },
            axis_x: 0,
            axis_y: 0,
            link,
            format: 2,
            depth: 8,
            palette: 0,
            tdata: false,
            payload,
        }
    }

    fn rle8_payload(indices: &[u8]) -> Vec<u8> {
        let mut payload = (indices.len() as u32).to_le_bytes().to_vec();
        payload.push(0x40 | indices.len() as u8);
        payload.push(indices[0]);
        payload
    }

    #[test]
    fn v2_linked_sprite_chains_reuse_decoded_pixels() {
        let mut base = node(0, 0, 0, Some(rle8_payload(&[1, 1])));
        base.width = 2;
        base.axis_x = -3;
        let mut middle = node(0, 5, 0, None);
        middle.axis_x = 7;
        let mut chained = node(0, 6, 1, None);
        chained.axis_y = -9;
        let dangling = node(0, 7, 99, None);

        let sff = SffFile::from_bytes(&build_v2(
            &[base, middle, chained, dangling],
            &[V2Palette::Colors(palette_colors())],
        ))
        .expect("parse linked sprites");

        let first = sff.get(0, 0).expect("base sprite");
        assert_eq!(first.rgba, [10, 20, 30, 255].repeat(2));
        assert_eq!(first.axis_x, -3);

        let middle = sff.get(0, 5).expect("direct link");
        assert_eq!((middle.width, middle.height), (2, 1));
        assert_eq!(middle.axis_x, 7);
        assert_eq!(middle.rgba, first.rgba);

        let chained = sff.get(0, 6).expect("chained link");
        assert_eq!(chained.rgba, first.rgba);
        assert_eq!(chained.axis_y, -9);

        let dangling = sff.get(0, 7).expect("dangling link");
        assert_eq!((dangling.width, dangling.height), (0, 0));
    }

    #[test]
    fn v2_missing_palette_falls_back_to_first_palette() {
        let payload = [&(2u32).to_le_bytes()[..], &[1u8, 2]].concat();
        let mut sprite = node(0, 0, 0, Some(payload));
        sprite.width = 2;
        sprite.palette = 4; // out of range for a one-palette file
        let sff = SffFile::from_bytes(&build_v2(&[sprite], &[V2Palette::Colors(palette_colors())]))
            .expect("parse missing palette");
        let sprite = sff.get(0, 0).expect("sprite");
        assert_eq!(&sprite.rgba[0..4], &[10, 20, 30, 255]);
        assert_eq!(&sprite.rgba[4..8], &[40, 50, 60, 255]);
    }

    #[test]
    fn v2_rle5_and_lz5_payloads_skip_the_size_prefix() {
        let mut rle5 = node(0, 0, 0, Some(vec![1, 0, 0, 0, 0x00, 0x81, 0x01]));
        rle5.format = 3;
        let mut lz5 = node(0, 1, 0, Some(vec![1, 0, 0, 0, 0x00, 0x21]));
        lz5.format = 4;
        lz5.depth = 5;
        let sff = SffFile::from_bytes(&build_v2(
            &[rle5, lz5],
            &[V2Palette::Colors(palette_colors())],
        ))
        .expect("parse indexed codecs");
        assert_eq!(sff.get(0, 0).expect("rle5").rgba, [10, 20, 30, 255]);
        assert_eq!(sff.get(0, 1).expect("lz5").rgba, [10, 20, 30, 255]);
    }
    fn v1_link(next: u32, group: u16, index: u16, link: u16, axis_x: i16, axis_y: i16) -> Vec<u8> {
        let mut header = vec![0u8; 32];
        header[0..4].copy_from_slice(&next.to_le_bytes());
        header[8..10].copy_from_slice(&axis_x.to_le_bytes());
        header[10..12].copy_from_slice(&axis_y.to_le_bytes());
        header[12..14].copy_from_slice(&group.to_le_bytes());
        header[14..16].copy_from_slice(&index.to_le_bytes());
        header[16..18].copy_from_slice(&link.to_le_bytes());
        header
    }

    #[test]
    fn v1_zero_length_sprites_link_to_earlier_frames() {
        let mut palette = vec![[0u8, 0, 0]; 256];
        palette[1] = [90, 80, 70];
        let payload = pcx(&[1, 1], 2, 1, Some(&palette));

        let base_offset = 32u32;
        let direct_offset = base_offset + 32 + payload.len() as u32;
        let chain_offset = direct_offset + 32;
        let dangling_offset = chain_offset + 32;

        let mut data = vec![0u8; 32];
        data[..12].copy_from_slice(b"ElecbyteSpr\0");
        data[15] = 1;
        data[20..24].copy_from_slice(&4u32.to_le_bytes());
        data[24..28].copy_from_slice(&base_offset.to_le_bytes());
        data[28..32].copy_from_slice(&32u32.to_le_bytes());

        data.extend_from_slice(&v1_sprite(direct_offset, 0, 0, false, &payload));
        data.extend_from_slice(&v1_link(chain_offset, 0, 5, 0, 7, 0));
        data.extend_from_slice(&v1_link(dangling_offset, 0, 6, 1, 0, -9));
        data.extend_from_slice(&v1_link(0, 0, 7, 99, 0, 0));

        let sff = SffFile::from_bytes(&data).expect("parse v1 links");
        let base = sff.get(0, 0).expect("base");
        assert_eq!(base.rgba, [90, 80, 70, 255].repeat(2));

        let direct = sff.get(0, 5).expect("direct link");
        assert_eq!((direct.width, direct.height), (2, 1));
        assert_eq!(direct.axis_x, 7);
        assert_eq!(direct.rgba, base.rgba);

        let chained = sff.get(0, 6).expect("chained link");
        assert_eq!(chained.axis_y, -9);
        assert_eq!(chained.rgba, base.rgba);

        // A trailing dangling link still parses into an empty sprite.
        let dangling = sff.get(0, 7).expect("dangling link");
        assert_eq!((dangling.width, dangling.height), (0, 0));
    }
    #[test]
    fn v2_palette_links_chain_and_broken_links_fall_back() {
        let mut first = node(0, 0, 0, Some(rle8_payload(&[2, 2])));
        first.width = 2;
        first.palette = 2;
        let mut second = node(0, 1, 0, Some(rle8_payload(&[1])));
        second.palette = 1;
        let palettes = [
            V2Palette::Colors(palette_colors()),
            V2Palette::Link(0),
            V2Palette::Link(1),
        ];
        let sff = SffFile::from_bytes(&build_v2(&[first, second], &palettes))
            .expect("parse palette links");
        assert_eq!(
            sff.get(0, 0).expect("chained palette").rgba,
            [40, 50, 60, 255].repeat(2)
        );
        assert_eq!(
            sff.get(0, 1).expect("direct palette").rgba,
            [10, 20, 30, 255]
        );

        // A palette node pointing at a missing source stays absent.
        let mut sprite = node(0, 2, 0, Some(rle8_payload(&[1])));
        sprite.palette = 0;
        let broken = [V2Palette::Link(9)];
        let sff = SffFile::from_bytes(&build_v2(&[sprite.clone()], &broken))
            .expect("parse broken palette link");
        assert_eq!(sff.get(0, 2).expect("sprite").rgba, [0, 0, 0, 0]);

        // No palette section at all also yields transparent indexed pixels.
        let sff = SffFile::from_bytes(&build_v2(&[sprite], &[])).expect("parse no palettes");
        assert_eq!(sff.get(0, 2).expect("sprite").rgba, [0, 0, 0, 0]);
    }

    #[test]
    fn v2_minor2_keeps_stored_palette_alpha() {
        let colors = vec![[0u8, 0, 0, 0], [10, 20, 30, 128]];
        let mut sprite = node(0, 0, 0, Some(rle8_payload(&[1, 1])));
        sprite.width = 2;

        let mut data = build_v2(&[sprite.clone()], &[V2Palette::Colors(colors.clone())]);
        data[13] = 1; // verlo2 != 0 keeps the stored alpha
        let sff = SffFile::from_bytes(&data).expect("parse minor2 alpha");
        let sprite = sff.get(0, 0).expect("sprite");
        assert_eq!(&sprite.rgba[0..4], &[10, 20, 30, 128]);

        // verlo2 == 0 forces index 0 transparent and all others opaque.
        let mut second = node(0, 0, 0, Some(rle8_payload(&[1, 1])));
        second.width = 2;
        let mut data = build_v2(&[second], &[V2Palette::Colors(colors)]);
        data[13] = 0;
        let sff = SffFile::from_bytes(&data).expect("parse forced alpha");
        let sprite = sff.get(0, 0).expect("sprite");
        assert_eq!(&sprite.rgba[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn v2_png8_sprite_uses_sff_palette_not_png_plte() {
        let png = {
            let mut encoded = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut encoded, 2, 2);
                encoder.set_color(png::ColorType::Indexed);
                encoder.set_depth(png::BitDepth::Eight);
                // The PNG's own PLTE is green; the SFF palette must win.
                encoder.set_palette(vec![255, 0, 0, 0, 255, 0]);
                let mut writer = encoder.write_header().unwrap();
                writer.write_image_data(&[1, 1, 1, 1]).unwrap();
            }
            encoded
        };
        let payload = [&(2u32 * 2 * 4).to_le_bytes()[..], &png].concat();
        let mut sprite = node(0, 0, 0, Some(payload));
        sprite.width = 2;
        sprite.height = 2;
        sprite.format = 10;
        sprite.depth = 8;
        let sff = SffFile::from_bytes(&build_v2(&[sprite], &[V2Palette::Colors(palette_colors())]))
            .expect("parse png8");
        let sprite = sff.get(0, 0).expect("sprite");
        assert_eq!((sprite.width, sprite.height), (2, 2));
        assert_eq!(sprite.rgba, [10, 20, 30, 255].repeat(4));
    }

    #[test]
    fn v2_truncated_png_yields_a_blank_sprite() {
        let payload = [&(16u32).to_le_bytes()[..], b"not a png"].concat();
        let mut sprite = node(0, 0, 0, Some(payload));
        sprite.width = 2;
        sprite.height = 2;
        sprite.format = 12;
        sprite.depth = 32;
        let sff = SffFile::from_bytes(&build_v2(&[sprite], &[V2Palette::Colors(palette_colors())]))
            .expect("truncated PNG must not abort the file");
        let sprite = sff.get(0, 0).expect("sprite");
        assert_eq!((sprite.width, sprite.height), (2, 2));
        assert_eq!(sprite.rgba, vec![0; 2 * 2 * 4]);
    }

    #[test]
    fn v2_raw_16_bit_depth_is_unsupported_but_safe() {
        let mut sprite = node(0, 0, 0, Some(vec![0xFF, 0x7F]));
        sprite.format = 0;
        sprite.depth = 16;
        let sff = SffFile::from_bytes(&build_v2(&[sprite], &[])).expect("parse 16-bit raw");
        let sprite = sff.get(0, 0).expect("sprite");
        assert_eq!((sprite.width, sprite.height), (1, 1));
        assert_eq!(sprite.rgba, [0, 0, 0, 0]);
    }
    #[test]
    fn v2_tdata_flag_selects_the_temporary_data_region() {
        let mut sprite = node(0, 0, 0, Some(rle8_payload(&[1, 1])));
        sprite.width = 2;
        sprite.tdata = true;
        let data = build_v2(&[sprite], &[V2Palette::Colors(palette_colors())]);
        // Sprite node flags bit 0 selects `tdata_offset`.
        assert_eq!(data[512 + 26] & 1, 1);
        // Palette group/index fields are present but selection is by table index.
        let palette_node = 512 + 28;
        assert_eq!(
            u16::from_le_bytes(data[palette_node..palette_node + 2].try_into().unwrap()),
            3
        );
        assert_eq!(
            u16::from_le_bytes(data[palette_node + 2..palette_node + 4].try_into().unwrap()),
            7
        );

        let sff = SffFile::from_bytes(&data).expect("parse tdata sprite");
        let sprite = sff.get(0, 0).expect("sprite");
        assert_eq!((sprite.width, sprite.height), (2, 1));
        assert_eq!(sprite.rgba, [10, 20, 30, 255].repeat(2));
    }
}
