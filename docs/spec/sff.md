# SFF sprite format specification

**Status: APPROVED / FROZEN.** This spec is the implementation source
for `manato-formats::sff`. It is derived from the MIT-licensed Ikemen-GO source
(`src/image.go`), public M.U.G.E.N format documentation, and observed content.
It is not a copy of any implementation.

## 1. Scope

Support the sprite files used by M.U.G.E.N 1.1 and Ikemen-GO content:

- SFF v1 (PCX-based)
- SFF v2 (raw, RLE8, RLE5, LZ5, PNG8/24/32)
- Linked sprites
- Inline and node-based palettes

Out of scope for this spec: texture atlas placement, palette remapping at
runtime, ACT palette loading, and rendering.

## 2. Common header

All integers are little-endian.

```
0x00  char[12]  "ElecbyteSpr\0"
0x0C  u8[4]     version bytes: verhi, verlo1, verlo2, verlo3
```

`verhi` selects the parser:

- `1` → SFF v1
- `2` → SFF v2

Reject other versions with a clear error.

## 3. SFF v1

### 3.1 Header

```
0x10  u32  reserved
0x14  u32  sprite_count
0x18  u32  first_sprite_offset
0x1C  u32  subheader_size (usually 32)
```

`sprite_count` is capped at 65,535 entries. `first_sprite_offset` points to the
first subfile header.

### 3.2 Subfile header

At least 32 bytes:

```
0x00  u32  next_offset
0x04  u32  data_length
0x08  i16  axis_x
0x0A  i16  axis_y
0x0C  u16  group
0x0E  u16  index
0x10  u16  link_index
0x12  u8   same_palette
0x13  u8   reserved
0x14  u8[12] reserved
```

Rules:

- `next_offset == 0` or `next_offset <= current_offset` ends the chain.
- Sprite data starts at `current_offset + subheader_size` and runs for
  `data_length` bytes. Alignment padding between the declared data end and the
  next header is not part of the payload. If the declared data overlaps the
  next header, it is trimmed at that header.
- If `data_length == 0` or the data is empty, the sprite is a link:
  copy the sprite at `link_index`, then override group, index, axis.
- Otherwise decode the PCX payload.
- `same_palette != 0` means reuse the previous sprite's palette.

### 3.3 PCX payload

The payload must begin with `0x0A` and be at least 128 bytes.

```
0x00  u8   manufacturer (0x0A)
0x02  u8   encoding (1 = RLE)
0x03  u8   bits_per_pixel (8)
0x04  u16  x_min
0x06  u16  y_min
0x08  u16  x_max
0x0A  u16  y_max
0x41  u8   planes (must be 1)
0x42  u16  bytes_per_line
```

Width = `x_max - x_min + 1`, height = `y_max - y_min + 1`. Reject zero,
negative, or larger than 8192 dimensions.

RLE stream starts at byte 128. For each scanline of `bytes_per_line` bytes:

- `byte & 0xC0 == 0xC0` → run of `byte & 0x3F`, next byte is the value.
- Otherwise the byte is a literal pixel.
- A run longer than the remaining scanline bytes is clamped to the line end
  rather than rejected.
- `bytes_per_line` is raised to at least `width` when the header value is
  smaller.

Copy the first `width` bytes of each decoded scanline into the tight indexed
buffer; discard padding.

### 3.4 SFF v1 palette

A palette is stored as `0x0C` followed by 768 RGB bytes, normally at the end of
the subfile. Search backwards from the block end for `0x0C`; fall back to
`block_end - 769` if not found.

Palette entries are RGB; index 0 is transparent. All other entries are opaque.

## 4. SFF v2

### 4.1 Header

```
0x10  u32  reserved
0x14  u32  reserved
0x18  u32  reserved
0x1C  u32  reserved
0x20  u32  sprite_offset
0x24  u32  sprite_count
0x28  u32  palette_offset
0x2C  u32  palette_count
0x30  u32  ldata_offset
0x34  u32  ldata_length
0x38  u32  tdata_offset
0x3C  u32  reserved
```

`ldata` is uncompressed data; `tdata` is translated/compressed data. A sprite
node's `flags` bit 0 selects `tdata` (1) or `ldata` (0).

### 4.2 Palette node

16 bytes:

```
0x00  u16  group
0x02  u16  index
0x04  u16  color_count
0x06  u16  link_index
0x08  u32  data_offset
0x0C  u32  data_length
```

If `data_length == 0`, copy the palette at `link_index`.

Otherwise data is at `ldata_offset + data_offset`, `color_count` entries of BGRA
(4 bytes each), capped at 256 entries and bounded by `data_length / 4`.

Convert BGRA to RGBA. Alpha rules:

- `verlo2 == 0`: force index 0 alpha to 0, all others to 255.
- `verlo2 != 0`: keep the stored alpha.

### 4.3 Sprite node

28 bytes:

```
0x00  u16  group
0x02  u16  index
0x04  u16  width
0x06  u16  height
0x08  i16  axis_x
0x0A  i16  axis_y
0x0C  u16  link_index
0x0E  u8   format
0x0F  u8   depth
0x10  u32  data_offset
0x14  u32  data_length
0x18  u16  palette_header
0x1A  u16  flags
```

If `data_length == 0`, copy the sprite at `link_index`, then override group,
index, axis.

Data address = (`flags & 1 ? tdata_offset : ldata_offset`) + `data_offset`.

### 4.4 Pixel formats

| Format | Meaning | Payload |
| --- | --- | --- |
| 0 | Raw | 8-bit indices or 24/32-bit BGR(A) |
| 2 | RLE8 | 4-byte size prefix + RLE8 |
| 3 | RLE5 | 4-byte size prefix + RLE5 |
| 4 | LZ5 | 4-byte size prefix + LZ5 |
| 10 | PNG8 | 4-byte size prefix + PNG indexed |
| 11 | PNG24 | 4-byte size prefix + PNG RGB |
| 12 | PNG32 | 4-byte size prefix + PNG RGBA |

The 4-byte prefix is the uncompressed size in bytes; it is not part of the
compressed stream. Indexed formats map indices through the selected palette
node. Format 0 depth 8 also maps through the palette. Format 0 depth 24/32 is
BGR(A) and is converted directly to RGBA.

A missing, out-of-range, or broken palette node falls back to palette 0,
matching Ikemen-GO's palette clamping; an index outside the selected palette
reads as transparent.

### 4.5 RLE8

```
while output not full:
  control = next byte
  if control & 0xC0 == 0x40:
    run = control & 0x3F
    value = next byte
  else:
    run = 1
    value = control
  write value run times
```

### 4.6 RLE5

`depth` is 5 bits per pixel. Implement the standard RLE5 stream used by
M.U.G.E.N: read run/value pairs until the output is full, with extended runs
when the high bit is set. The decoder must stop safely on truncated input and
report failure.

### 4.7 LZ5

```
control = first byte
while output not full:
  data = next byte
  if control bit set:
    if data & 0x3F == 0:
      extended_offset = ((data << 2) | next) + 1
      run = next + 2
    else:
      offset = accumulated 2-bit reference bits + 1
      run = data & 0x3F
    copy `run` bytes from output[-offset]
  else:
    if data & 0xE0 == 0:
      run = next + 8
    else:
      run = data >> 5
      value = data & 0x1F
    write value run times
  advance control bit; load a new control byte every 8 steps
```

Validate every back-reference (`offset > 0`, `offset <= output_written`).
Return failure on truncation or invalid references.

### 4.8 PNG

PNG payloads begin with the 4-byte size prefix, then the PNG signature.

- Indexed PNG: read raw 8-bit indices, map each through the SFF palette node.
  Do not use the PNG's own `PLTE`, which may be a dummy.
- RGB: expand to RGBA with alpha 255.
- RGBA: use as-is.
- Grayscale/GrayscaleAlpha: expand to RGBA.
- Reject unsupported bit depths for indexed PNG.

Use the `png` crate for zlib/IDAT handling; do not reimplement PNG filtering.

## 4.5 Indexed pixels and palettes

Indexed sprites retain their decoded palette indices (`Sprite::indices`), the
palette they were decoded with (`Sprite::palette`), and the palette identity
(`Sprite::palette_key`): the SFF v2 palette-node group/index, or `(1, 1)` for
SFF v1 embedded palettes. True-color sprites leave all of these empty.
`Sprite::recolor_rgba(palette)` rebuilds RGBA for `RemapPal`, and the renderer
only recolors sprites whose key matches the `RemapPal` source (a `-1` source
force-remaps every indexed sprite).

SFF v2 palette nodes are exposed as a `(group, index)` map, and ACT files
declared in the character DEF (`pal1`..`palN`, raw 768-byte RGB or 1024-byte
RGBA) are registered as `(1, N)` with index 0 forced transparent.

## 4.6 Sprite substitution

`RemapSprite` stores `(group, index) → (group, index)` substitutions on the
fighter. Rendering (fighters and their afterimages) resolves through the map;
unmapped keys use the animation's original sprite. `reset = 1` clears the map,
which also happens at round start. `preset = "name"` loads a
`[RemapPreset name]` table from the character DEF (keys are
`srcGroup,srcIndex = dstGroup,dstIndex`). Per-frame collision-box substitution
is not modeled.

## 5. Error handling

- Reject truncated headers, invalid signatures, unsupported versions.
- Bounds-check every offset and length before slicing.
- Log and skip a single bad sprite/palette rather than aborting the whole file
  when the header is otherwise valid.
- A missing or broken palette node falls back to palette 0; a sprite index
  outside that palette reads as transparent.
- Preserve declaration order; `get(group, index)` returns the first sprite with
  that key.

## 6. Test plan

- Existing unit tests: RLE8 round-trip, LZ5 literal run, v2 indexed prefix.
- Synthetic v2 PNG8/PNG32 sprites and truncated PNG payloads.
- Linked sprite copy and axis override.
- Palette link, minor2 alpha behavior, and missing-palette fallback.
- Truncated RLE8/LZ5/PNG payloads.
- Bundled character/stage SFFs.
- Stock KFM and Ryu SFFs when available.

## 7. Sources

- Ikemen-GO `src/image.go` (MIT), especially `SffHeader.Read`,
  `Sprite.readHeaderV2`, `Sprite.read`, and the RLE decoders.
- Public M.U.G.E.N SFF format documentation.
- Observed SFF v1/v2 files in the project content.
