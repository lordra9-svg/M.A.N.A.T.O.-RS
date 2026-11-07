use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context};

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
}

impl Sprite {
    pub fn empty(group: i32, index: i32) -> Self {
        Self {
            group,
            index,
            width: 0,
            height: 0,
            axis_x: 0,
            axis_y: 0,
            rgba: Vec::new(),
        }
    }
}

/// An SFF v1 or v2 sprite file.
#[derive(Debug, Clone, Default)]
pub struct SffFile {
    pub version: u8,
    pub sprites: Vec<Sprite>,
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

    pub fn from_bytes(data: &[u8]) -> anyhow::Result<Self> {
        if data.len() < 512 {
            bail!("sprite file is too small");
        }
        if &data[..12] != b"ElecbyteSpr\0" {
            bail!("not an Elecbyte sprite file");
        }
        let version = data[15];
        let mut out = Self {
            version,
            sprites: Vec::new(),
            by_key: HashMap::new(),
        };
        match version {
            1 => out.load_v1(data)?,
            2 => out.load_v2(data)?,
            other => bail!("unsupported SFF version {other}"),
        }
        for (i, sprite) in out.sprites.iter().enumerate() {
            out.by_key.entry((sprite.group, sprite.index)).or_insert(i);
        }
        Ok(out)
    }

    /// Sprite keys in a stable order (used by the atlas packer).
    pub fn keys(&self) -> Vec<(i32, i32)> {
        self.by_key.keys().copied().collect()
    }

    fn load_v1(&mut self, data: &[u8]) -> anyhow::Result<()> {
        if data.len() < 512 {
            bail!("SFF v1 header truncated");
        }
        let count = read_u32(data, 20) as usize;
        let mut offset = read_u32(data, 24) as usize;
        let subheader = read_u32(data, 28).max(32) as usize;
        let mut previous_palette: Option<Vec<[u8; 4]>> = None;

        for _ in 0..count.min(65_535) {
            if offset + 32 > data.len() {
                break;
            }
            let h = &data[offset..offset + 32];
            let next = read_u32(h, 0) as usize;
            let data_len = read_u32(h, 4) as usize;
            let axis_x = read_u16(h, 8) as i16 as i32;
            let axis_y = read_u16(h, 10) as i16 as i32;
            let group = read_u16(h, 12) as i32;
            let index = read_u16(h, 14) as i32;
            let link = read_u16(h, 16) as usize;
            let same_palette = h[18] != 0;

            let data_start = offset + subheader;
            if data_start > data.len() {
                break;
            }
            let mut data_end = data_start.saturating_add(data_len).min(data.len());
            if next > data_start && next <= data.len() {
                data_end = next;
            }
            let pixels = &data[data_start..data_end];

            if data_len == 0 || pixels.is_empty() {
                if link < self.sprites.len() {
                    let mut sprite = self.sprites[link].clone();
                    sprite.group = group;
                    sprite.index = index;
                    sprite.axis_x = axis_x;
                    sprite.axis_y = axis_y;
                    self.sprites.push(sprite);
                } else {
                    self.sprites.push(Sprite {
                        group,
                        index,
                        width: 0,
                        height: 0,
                        axis_x,
                        axis_y,
                        rgba: Vec::new(),
                    });
                }
            } else {
                let decoded = decode_pcx(pixels);
                match decoded {
                    Some((w, hgt, rgba, palette)) => {
                        if !same_palette {
                            if let Some(p) = palette {
                                previous_palette = Some(p);
                            }
                        }
                        let _ = &previous_palette;
                        self.sprites.push(Sprite {
                            group,
                            index,
                            width: w,
                            height: hgt,
                            axis_x,
                            axis_y,
                            rgba,
                        });
                    }
                    None => {
                        log::warn!("SFF v1: could not decode PCX for {group},{index}");
                        self.sprites.push(Sprite {
                            group,
                            index,
                            width: 0,
                            height: 0,
                            axis_x,
                            axis_y,
                            rgba: Vec::new(),
                        });
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

    fn load_v2(&mut self, data: &[u8]) -> anyhow::Result<()> {
        if data.len() < 68 {
            bail!("SFF v2 header truncated");
        }
        let sprite_ofs = read_u32(data, 36) as usize;
        let sprite_count = read_u32(data, 40) as usize;
        let palette_ofs = read_u32(data, 44) as usize;
        let palette_count = read_u32(data, 48) as usize;
        let ldata_ofs = read_u32(data, 52) as usize;
        let tdata_ofs = read_u32(data, 60) as usize;
        let minor2 = data[13];

        let mut palettes: Vec<Option<Vec<[u8; 4]>>> = vec![None; palette_count.min(4096)];
        for i in 0..palettes.len() {
            let off = palette_ofs + i * 16;
            if off + 16 > data.len() {
                break;
            }
            let n = &data[off..off + 16];
            let _group = read_u16(n, 0);
            let _index = read_u16(n, 2);
            let colors = read_u16(n, 4) as usize;
            let link = read_u16(n, 6) as usize;
            let data_ofs = read_u32(n, 8) as usize;
            let data_len = read_u32(n, 12) as usize;

            if data_len == 0 {
                if link < palettes.len() {
                    palettes[i] = palettes[link].clone();
                }
                continue;
            }
            let abs = ldata_ofs.saturating_add(data_ofs);
            if abs + data_len.min(colors * 4) > data.len() || colors == 0 {
                continue;
            }
            let count = colors.min(256).min(data_len / 4);
            let mut colors_out = Vec::with_capacity(count);
            for c in 0..count {
                let p = &data[abs + c * 4..abs + c * 4 + 4];
                // SFF v2 palette entries are stored BGRA.
                let mut a = p[3];
                if minor2 == 0 {
                    a = if c == 0 { 0 } else { 255 };
                }
                colors_out.push([p[2], p[1], p[0], a]);
            }
            palettes[i] = Some(colors_out);
        }

        for i in 0..sprite_count.min(65_535) {
            let off = sprite_ofs + i * 28;
            if off + 28 > data.len() {
                break;
            }
            let n = &data[off..off + 28];
            let group = read_u16(n, 0) as i32;
            let index = read_u16(n, 2) as i32;
            let w = read_u16(n, 4) as u32;
            let hgt = read_u16(n, 6) as u32;
            let axis_x = read_u16(n, 8) as i16 as i32;
            let axis_y = read_u16(n, 10) as i16 as i32;
            let link = read_u16(n, 12) as usize;
            let format = n[14];
            let depth = n[15];
            let data_ofs = read_u32(n, 16) as usize;
            let data_len = read_u32(n, 20) as usize;
            let palette_header = read_u16(n, 24) as usize;
            let flags = read_u16(n, 26);

            if data_len == 0 {
                if link < self.sprites.len() {
                    let mut sprite = self.sprites[link].clone();
                    sprite.group = group;
                    sprite.index = index;
                    sprite.axis_x = axis_x;
                    sprite.axis_y = axis_y;
                    self.sprites.push(sprite);
                } else {
                    self.sprites.push(Sprite {
                        group,
                        index,
                        width: w,
                        height: hgt,
                        axis_x,
                        axis_y,
                        rgba: Vec::new(),
                    });
                }
                continue;
            }

            let base = if flags & 1 != 0 { tdata_ofs } else { ldata_ofs };
            let abs = base.saturating_add(data_ofs);
            if abs + data_len > data.len() {
                log::warn!("SFF v2: sprite {group},{index} data out of bounds");
                continue;
            }
            let raw = &data[abs..abs + data_len];
            let palette = palettes
                .get(palette_header)
                .and_then(|p| p.as_deref())
                .unwrap_or(&[]);

            let rgba = match format {
                0 => decode_raw(raw, w, hgt, depth, palette),
                2 => decode_indexed(raw.get(4..).unwrap_or(raw), w, hgt, palette, Codec::Rle8),
                3 => decode_indexed(raw.get(4..).unwrap_or(raw), w, hgt, palette, Codec::Rle5),
                4 => decode_indexed(raw.get(4..).unwrap_or(raw), w, hgt, palette, Codec::Lz5),
                10..=12 => {
                    // SFF v2 PNG sprites are prefixed with a four-byte
                    // uncompressed-size field.
                    let png = raw.get(4..).unwrap_or(raw);
                    match decode_png(png, w, hgt, palette) {
                        Ok(rgba) => rgba,
                        Err(e) => {
                            log::warn!("SFF v2: PNG decode failed for {group},{index}: {e}");
                            (w, hgt, vec![0; (w * hgt * 4) as usize])
                        }
                    }
                }
                other => {
                    log::warn!("SFF v2: unsupported pixel format {other} for {group},{index}");
                    (w, hgt, vec![0; (w * hgt * 4) as usize])
                }
            };
            self.sprites.push(Sprite {
                group,
                index,
                width: rgba.0,
                height: rgba.1,
                axis_x,
                axis_y,
                rgba: rgba.2,
            });
        }
        Ok(())
    }
}

/// Decodes an SFF v2 PNG sprite.
///
/// PNG8 sprites in SFF files are indexed images whose own `PLTE` chunk is often
/// a dummy black palette; the real colours live in the SFF palette node. We
/// therefore read the raw indices and map them through `palette` when present.
fn decode_png(
    raw: &[u8],
    _w: u32,
    _h: u32,
    palette: &[[u8; 4]],
) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    use std::io::Cursor;

    let decoder = png::Decoder::new(Cursor::new(raw));
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let data = &buf[..info.buffer_size()];
    let width = info.width;
    let height = info.height;

    let mut out = vec![0u8; (width * height * 4) as usize];
    match info.color_type {
        png::ColorType::Indexed => {
            if info.bit_depth != png::BitDepth::Eight {
                anyhow::bail!("unsupported indexed PNG bit depth {:?}", info.bit_depth);
            }
            for i in 0..(width * height) as usize {
                let idx = data.get(i).copied().unwrap_or(0) as usize;
                let color = palette.get(idx).copied().unwrap_or([0, 0, 0, 0]);
                out[i * 4..i * 4 + 4].copy_from_slice(&color);
            }
        }
        png::ColorType::Rgba => out.copy_from_slice(data),
        png::ColorType::Rgb => {
            for (i, px) in data.chunks_exact(3).enumerate() {
                out[i * 4..i * 4 + 3].copy_from_slice(px);
                out[i * 4 + 3] = 255;
            }
        }
        png::ColorType::Grayscale => {
            for (i, v) in data.iter().enumerate() {
                out[i * 4] = *v;
                out[i * 4 + 1] = *v;
                out[i * 4 + 2] = *v;
                out[i * 4 + 3] = 255;
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for (i, px) in data.chunks_exact(2).enumerate() {
                out[i * 4] = px[0];
                out[i * 4 + 1] = px[0];
                out[i * 4 + 2] = px[0];
                out[i * 4 + 3] = px[1];
            }
        }
    }
    Ok((width, height, out))
}

fn decode_raw(raw: &[u8], w: u32, h: u32, depth: u8, palette: &[[u8; 4]]) -> (u32, u32, Vec<u8>) {
    let count = (w * h) as usize;
    let mut out = vec![0u8; count * 4];
    match depth {
        8 => {
            for i in 0..count {
                let idx = raw.get(i).copied().unwrap_or(0) as usize;
                let c = palette.get(idx).copied().unwrap_or([0, 0, 0, 0]);
                out[i * 4..i * 4 + 4].copy_from_slice(&c);
            }
        }
        24 | 32 => {
            let step = if depth == 32 { 4 } else { 3 };
            for i in 0..count {
                let p = i * step;
                if p + step > raw.len() {
                    break;
                }
                // SFF stores these as BGR(A).
                out[i * 4] = raw[p + 2];
                out[i * 4 + 1] = raw[p + 1];
                out[i * 4 + 2] = raw[p];
                out[i * 4 + 3] = if depth == 32 { raw[p + 3] } else { 255 };
            }
        }
        _ => {}
    }
    (w, h, out)
}

enum Codec {
    Rle8,
    Rle5,
    Lz5,
}

fn decode_indexed(
    raw: &[u8],
    w: u32,
    h: u32,
    palette: &[[u8; 4]],
    codec: Codec,
) -> (u32, u32, Vec<u8>) {
    let count = (w * h) as usize;
    let mut indices = vec![0u8; count];
    let ok = match codec {
        Codec::Rle8 => rle8_decode(raw, &mut indices),
        Codec::Rle5 => rle5_decode(raw, &mut indices),
        Codec::Lz5 => lz5_decode(raw, &mut indices),
    };
    if !ok {
        log::warn!("SFF v2: indexed decode stopped early");
    }
    let mut out = vec![0u8; count * 4];
    for i in 0..count {
        let c = palette
            .get(indices[i] as usize)
            .copied()
            .unwrap_or([0, 0, 0, 0]);
        out[i * 4..i * 4 + 4].copy_from_slice(&c);
    }
    (w, h, out)
}

pub fn rle8_decode(src: &[u8], dst: &mut [u8]) -> bool {
    let mut i = 0;
    let mut j = 0;
    while j < dst.len() {
        if i >= src.len() {
            return false;
        }
        let mut byte = src[i];
        i += 1;
        let mut run = 1;
        if byte & 0xC0 == 0x40 {
            run = (byte & 0x3F) as usize;
            if i >= src.len() {
                return false;
            }
            byte = src[i];
            i += 1;
        }
        for _ in 0..run {
            if j < dst.len() {
                dst[j] = byte;
                j += 1;
            }
        }
    }
    true
}

pub fn rle5_decode(src: &[u8], dst: &mut [u8]) -> bool {
    if src.is_empty() {
        return false;
    }
    let mut i = 0;
    let mut j = 0;
    while j < dst.len() {
        if i >= src.len() {
            return false;
        }
        let mut run = src[i] as i32;
        i += 1;

        if i >= src.len() {
            return false;
        }
        let mut extra = (src[i] & 0x7F) as i32;
        let mut value = 0u8;
        if src[i] & 0x80 != 0 {
            if i + 1 >= src.len() {
                return false;
            }
            value = src[i + 1];
            i += 1;
        }
        i += 1;

        loop {
            if j < dst.len() {
                dst[j] = value;
                j += 1;
            }
            run -= 1;
            if run < 0 {
                extra -= 1;
                if extra < 0 {
                    break;
                }
                if i >= src.len() {
                    return false;
                }
                value = src[i] & 0x1F;
                run = (src[i] >> 5) as i32;
                i += 1;
            }
        }
    }
    true
}

pub fn lz5_decode(src: &[u8], dst: &mut [u8]) -> bool {
    if src.is_empty() {
        return false;
    }
    let mut i = 1;
    let mut j = 0;
    let mut run: i32;
    let mut control = src[0];
    let mut control_shift = 0u32;
    let mut ref_bits = 0u32;
    let mut ref_shift = 0u32;

    while j < dst.len() {
        if i >= src.len() {
            return false;
        }
        let mut data = src[i] as i32;
        i += 1;

        if control & (1u8 << control_shift) != 0 {
            if data & 0x3F == 0 {
                if i >= src.len() {
                    return false;
                }
                data = ((data << 2) | src[i] as i32) + 1;
                i += 1;
                if i >= src.len() {
                    return false;
                }
                run = src[i] as i32 + 2;
                i += 1;
            } else {
                ref_bits |= ((data & 0xC0) as u32) >> ref_shift;
                ref_shift += 2;
                run = data & 0x3F;
                if ref_shift < 8 {
                    if i >= src.len() {
                        return false;
                    }
                    data = src[i] as i32 + 1;
                    i += 1;
                } else {
                    data = ref_bits as i32 + 1;
                    ref_bits = 0;
                    ref_shift = 0;
                }
            }

            loop {
                if data <= 0 || data as usize > j {
                    return false;
                }
                if j < dst.len() {
                    dst[j] = dst[j - data as usize];
                }
                j += 1;
                run -= 1;
                if run < 0 {
                    break;
                }
            }
        } else {
            if data & 0xE0 == 0 {
                if i >= src.len() {
                    return false;
                }
                run = src[i] as i32 + 8;
                i += 1;
            } else {
                run = data >> 5;
                data &= 0x1F;
            }
            while run > 0 && j < dst.len() {
                dst[j] = data as u8;
                j += 1;
                run -= 1;
            }
        }

        control_shift += 1;
        if control_shift >= 8 {
            if i >= src.len() {
                break;
            }
            control = src[i];
            i += 1;
            control_shift = 0;
        }
    }
    j == dst.len()
}

/// Decodes an 8-bit PCX image embedded in an SFF v1 subfile.
fn decode_pcx(data: &[u8]) -> Option<(u32, u32, Vec<u8>, Option<Vec<[u8; 4]>>)> {
    if data.len() < 128 || data[0] != 0x0A {
        return None;
    }
    let encoding = data[2];
    let bpp = data[3];
    if encoding != 1 || bpp != 8 {
        return None;
    }
    let xmin = read_u16(data, 4) as i32;
    let ymin = read_u16(data, 6) as i32;
    let xmax = read_u16(data, 8) as i32;
    let ymax = read_u16(data, 10) as i32;
    let width = (xmax - xmin + 1).max(0) as usize;
    let height = (ymax - ymin + 1).max(0) as usize;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return None;
    }
    let planes = data[65].max(1) as usize;
    let bytes_per_line = read_u16(data, 66).max(1) as usize;
    if planes != 1 {
        return None;
    }

    let mut palette = None;
    if data.len() >= 769 && data[data.len() - 769] == 0x0C {
        let p = &data[data.len() - 768..];
        let mut colors = Vec::with_capacity(256);
        for i in 0..256 {
            colors.push([p[i * 3], p[i * 3 + 1], p[i * 3 + 2], 255]);
        }
        colors[0][3] = 0;
        palette = Some(colors);
    }

    let mut indices = vec![0u8; width * height];
    let mut pos = 128usize;
    for y in 0..height {
        let mut row = vec![0u8; bytes_per_line];
        let mut x = 0;
        while x < bytes_per_line {
            if pos >= data.len() {
                return None;
            }
            let byte = data[pos];
            pos += 1;
            if byte & 0xC0 == 0xC0 {
                let run = (byte & 0x3F) as usize;
                if pos >= data.len() {
                    return None;
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
        let n = width.min(bytes_per_line);
        indices[y * width..y * width + n].copy_from_slice(&row[..n]);
    }

    let pal = palette.clone().unwrap_or_else(default_palette);
    let mut rgba = vec![0u8; width * height * 4];
    for i in 0..width * height {
        let c = pal
            .get(indices[i] as usize)
            .copied()
            .unwrap_or([0, 0, 0, 0]);
        rgba[i * 4..i * 4 + 4].copy_from_slice(&c);
    }
    Some((width as u32, height as u32, rgba, palette))
}

fn default_palette() -> Vec<[u8; 4]> {
    (0..256)
        .map(|i| {
            let v = i as u8;
            [v, v, v, if i == 0 { 0 } else { 255 }]
        })
        .collect()
}

fn read_u16(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn read_u32(data: &[u8], at: usize) -> u32 {
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
    fn lz5_literal_run() {
        // Control byte 0: literal. data=0x20 => run=1, value=0.
        let mut out = [9u8; 1];
        assert!(lz5_decode(&[0, 0x20], &mut out));
        assert_eq!(out, [0]);
    }

    #[test]
    fn v2_indexed_sprites_skip_the_size_prefix() {
        // Minimal SFF v2 with one 2x2 LZ5 sprite. The compressed payload is
        // preceded by a little-endian uncompressed size, and the palette is
        // stored BGRA in the linked-data block.
        let mut data = vec![0u8; 512];
        data[..12].copy_from_slice(b"ElecbyteSpr\0");
        data[13] = 1; // minor2: keep palette alpha
        data[15] = 2;

        let palette_ofs = 68usize;
        let palette_count = 1u32;
        let ldata_ofs = palette_ofs + 16;
        let palette_len = 32 * 4;
        let sprite_ofs = ldata_ofs + palette_len;
        let sprite_data_ofs = sprite_ofs + 28;

        let put_u16 = |data: &mut [u8], at: usize, v: u16| {
            data[at..at + 2].copy_from_slice(&v.to_le_bytes());
        };
        let put_u32 = |data: &mut [u8], at: usize, v: u32| {
            data[at..at + 4].copy_from_slice(&v.to_le_bytes());
        };

        let total = data.len() as u32;
        put_u32(&mut data, 36, sprite_ofs as u32);
        put_u32(&mut data, 40, 1);
        put_u32(&mut data, 44, palette_ofs as u32);
        put_u32(&mut data, 48, palette_count);
        put_u32(&mut data, 52, ldata_ofs as u32);
        put_u32(&mut data, 60, total);

        // Palette node: group 1, index 1, 32 colours, data at ldata + 0.
        put_u16(&mut data, palette_ofs, 1);
        put_u16(&mut data, palette_ofs + 2, 1);
        put_u16(&mut data, palette_ofs + 4, 32);
        put_u32(&mut data, palette_ofs + 8, 0);
        put_u32(&mut data, palette_ofs + 12, palette_len as u32);
        for c in 0..32 {
            let at = ldata_ofs + c * 4;
            data[at..at + 4].copy_from_slice(&[30, 20, 10, 255]);
        }

        // Sprite node: 2x2, format 4 (LZ5), depth 5, palette node 0.
        put_u16(&mut data, sprite_ofs, 0);
        put_u16(&mut data, sprite_ofs + 2, 0);
        put_u16(&mut data, sprite_ofs + 4, 2);
        put_u16(&mut data, sprite_ofs + 6, 2);
        data[sprite_ofs + 14] = 4;
        data[sprite_ofs + 15] = 5;
        put_u32(
            &mut data,
            sprite_ofs + 16,
            (sprite_data_ofs - ldata_ofs) as u32,
        );
        put_u32(&mut data, sprite_ofs + 20, 6);

        // Size prefix (4 pixels) then control 0 + 0x80 => run of 4, value 0.
        data[sprite_data_ofs..sprite_data_ofs + 6].copy_from_slice(&[4, 0, 0, 0, 0x00, 0x80]);

        let sff = SffFile::from_bytes(&data).expect("parse sff");
        let sprite = sff.get(0, 0).expect("sprite");
        assert_eq!((sprite.width, sprite.height), (2, 2));
        assert_eq!(sprite.rgba, [10, 20, 30, 255].repeat(4));
    }
}
