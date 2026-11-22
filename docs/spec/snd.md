# SND sound bank specification

**Status: APPROVED / FROZEN.** Derived from the MIT-licensed Ikemen-GO
source (`src/sound.go`), public M.U.G.E.N format documentation, and observed
content. Not a copy of any implementation.

## 1. Scope

Decode M.U.G.E.N/Ikemen-GO `.snd` banks into playable PCM:

- 8/16/24/32-bit integer WAV
- 32-bit float WAV
- Mono and multi-channel WAV
- Multiple sounds keyed by `(group, index)`

Out of scope: BGM, looping, panning, and playback scheduling; sim-side channel state is specified in `audio.md`.

## 2. Header

All integers are little-endian.

```
0x00  char[12]  "ElecbyteSnd\0"
0x0C  u16       version
0x0E  u16       version2
0x10  u32       sound_count
0x14  u32       first_subheader_offset
```

- `sound_count` is capped at 65,535.
- `first_subheader_offset` must be within the file.
- Version fields are informational; they do not change the subheader layout.

## 3. Subheader

16 bytes:

```
0x00  u32  next_offset
0x04  u32  data_length
0x08  u16  group
0x0A  u16  index
0x0C  u32  reserved
```

Rules:

- `next_offset == 0` or `next_offset <= current_offset` ends the chain.
- WAV data starts at `current_offset + 16` and has `data_length` bytes.
- Empty or truncated entries are skipped with a warning.
- Duplicate `(group, index)` keys: the first successfully decoded entry wins;
  later duplicates are ignored.

## 4. WAV payload

The payload must be a RIFF/WAVE stream. Decode with a WAV parser; do not
implement chunk parsing by hand.

Accepted sample formats:

| Format | Conversion to signed 16-bit |
| --- | --- |
| Unsigned 8-bit | `(sample - 128) << 8` |
| Signed 16-bit | as-is |
| Signed 24-bit | `sample >> 8` |
| Signed 32-bit | `sample >> 16` |
| IEEE float 32 | `clamp(sample, -1, 1) * 32767` |

Retain `sample_rate` and `channels`. Samples are interleaved.

Reject payloads that are not RIFF/WAVE. A non-standard raw-PCM fallback is not
part of compatibility mode; if retained as a tolerant extension it must be
explicitly enabled and documented.

## 5. Sound keys

- Group and index are signed 32-bit values for lookup, stored as unsigned 16-bit
  in the file.
- `get(group, index)` returns the first decoded sound for that key.
- Negative group/index values are not valid keys in a file, but the engine may
  use negative values internally to mean “no sound.”

## 6. Error handling

- Reject invalid signatures, truncated headers, and out-of-bounds offsets.
  A declared first-subheader offset outside the file, or a truncated first
  subheader, fails the bank load.
- Later empty or truncated entries are skipped with a warning; the chain
  continues through `next_offset`.
- Do not panic on malformed WAV data.
- Log a warning for duplicate keys, truncated entries, and undecodable WAVs.

## 7. Test plan

- Load bundled character sound banks and `fight.snd`.
- 8-bit, 16-bit, 24-bit, 32-bit, and float WAV fixtures.
- Mono and stereo fixtures.
- Duplicate keys: first wins.
- Truncated subheader, truncated WAV, invalid magic.
- Empty bank.
- Non-RIFF payload rejection in compatibility mode.

## 8. Sources

- Ikemen-GO `src/sound.go` (MIT), especially `LoadSndFiltered` and `readSound`.
- Public M.U.G.E.N SND documentation.
- Observed `.snd` files in the project content.
