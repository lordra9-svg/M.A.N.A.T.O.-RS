# AIR animation specification

**Status: DRAFT — awaiting approval.** Derived from the MIT-licensed Ikemen-GO
source (`src/anim.go`), public M.U.G.E.N AIR documentation, and observed
content. Not a copy of any implementation.

## 1. Scope

Parse M.U.G.E.N/Ikemen-GO `.air` animation files into frame lists used by the
simulation and renderer:

- Action blocks
- Frame sprites, offsets, durations, flips
- Blend/transparency parameters
- Scale and angle parameters
- Collision box defaults and per-frame overrides
- Loop points and interpolation markers

Out of scope: sprite decoding and texture upload.

## 2. Text format

- Lines are decoded with the shared tolerant text decoder (BOM/UTF-16/UTF-8).
- `;` starts a comment.
- Keywords are case-insensitive.
- Blank lines are ignored.
- A new `[Begin Action N]` block ends the previous action.
- Duplicate action numbers: the last block wins.
- Unknown keywords are ignored with a warning.

## 3. Action block

```
[Begin Action N]
```

`N` is a signed 32-bit action number.

Optional extensions:

```
copy action M
```

Copies the frames of action `M` into the current action.

Markers:

```
loopstart
interpolate offset
interpolate scale
interpolate angle
interpolate blend
```

## 4. Frame line

```
group, index, xoffset, yoffset, time[, flips][, transparency][, xscale][, yscale][, angle]
```

Required fields:

| Field | Meaning |
| --- | --- |
| group | sprite group |
| index | sprite index |
| xoffset | horizontal offset in pixels |
| yoffset | vertical offset in pixels |
| time | duration in ticks |

### 4.1 Flips

The sixth field is a string containing `H` and/or `V`:

- `H`: flip horizontally and negate `xoffset`.
- `V`: flip vertically and negate `yoffset`.
- `HV` / `VH`: both.

### 4.2 Transparency

The seventh field selects blend mode and optional source/destination alpha:

| Value | Meaning | Defaults |
| --- | --- | --- |
| `A` | additive | src 255, dst 255 |
| `A1` | additive, M.U.G.E.N shorthand | src 255, dst 128 |
| `AS<src>d<dst>` | additive with alpha | src 255, dst 0 |
| `S` | subtractive | src 255, dst 255 |
| `SS<src>d<dst>` | subtractive with alpha | src 255, dst 255 |
| `SA` | sub-additive | src 255, dst 255 |
| `SAS<src>d<dst>` | sub-additive with alpha | src 255, dst 255 |

Alpha digits are decimal, clamped to 0–255. `d` separates source and
destination alpha.

### 4.3 Scale and angle

- Eighth field: horizontal scale (float).
- Ninth field: vertical scale (float).
- Tenth field: angle in degrees (float).
- Empty fields mean “no change” for Ikemen-GO compatibility.

## 5. Collision boxes

### 5.1 Defaults

```
Clsn1Default: n
Clsn2Default: n
```

`n` is the expected number of boxes. The following `n` lines are the default
boxes:

```
Clsn1[i] = x1, y1, x2, y2
Clsn2[i] = x1, y1, x2, y2
```

Boxes are normalized so `x1 <= x2` and `y1 <= y2`.

### 5.2 Per-frame overrides

```
Clsn1: n
Clsn2: n
```

These apply to the **next frame only**. After that frame, the default boxes
resume.

A frame with no explicit boxes uses the current defaults.

If a declared box count does not match the number of parsed boxes, log an error
and keep the boxes that were parsed. Duplicate or out-of-range indices are
placed in the first free slot.

## 6. Timing

Target semantics (M.U.G.E.N 1.1 / Ikemen-GO):

- `time > 0`: frame lasts that many ticks.
- `time == 0`: frame is skipped during playback.
- `time == -1`: final frame is displayed forever; it is not a normal loop.
- `loopstart` is the frame index playback returns to after the last frame.
  If it is out of range, keep the previous loop start.
- `AnimTime` is the animation clock relative to the total duration; exact
  behavior must be frozen in the timing spec before implementation.

The current manato implementation uses a simplified `total_time - anim_time`
clock. The rewrite must align it with the approved M.U.G.E.N/Ikemen-GO
semantics and update the tests.

## 7. Interpolation

`interpolate offset|scale|angle|blend` marks the frame where interpolation
begins. Rendering interpolation is a separate renderer concern; the parser must
preserve the marker positions.

## 8. Error handling

- A malformed frame line is skipped with a warning.
- A block with no frames is kept as an empty animation.
- Out-of-range action numbers are rejected.
- Invalid box lines are skipped.
- Do not panic on truncated or malformed text.

## 9. Test plan

- Existing `parses_clsn_defaults_and_overrides` test.
- Flips and offset negation.
- Default boxes and one-frame overrides.
- `loopstart`, zero-duration frames, infinite last frame.
- Transparency variants and alpha clamping.
- Scale/angle parsing.
- `copy action`.
- Duplicate actions: last wins.
- Malformed lines and missing boxes.
- Bundled character animations and stock KFM/Ryu AIR.

## 10. Sources

- Ikemen-GO `src/anim.go` (MIT), especially `ReadAnimFrame`, `readAnimation`,
  `readClsn`, `Animation.Action`, `AnimTime`, and `AnimElemTime`.
- Public M.U.G.E.N AIR documentation.
- Observed `.air` files in the project content.
