# Text object specification

**Status: APPROVED.** Derived from the MIT-licensed Ikemen-GO source
(`src/font.go`, `src/bytecode.go`) and observed content.

## 1. Scope

`Text`, `ModifyText`, `RemoveText`, and the `NumText(id)` trigger. Text is
screen-space in the 320x240 game coordinate system; it is not camera-relative.

Out of scope: custom text shaders, TTF fonts, `hidewithbars`, windows, and
palette effects on text.

## 2. Model

Each `Text` creates a `TextObject` owned by the executing fighter:

| Field | Default | Notes |
| --- | --- | --- |
| `text` | empty | String expression; `%d`/`%i`/`%f`/`%s` are filled from `params` |
| `id` | `0` | Matched by `ModifyText`, `RemoveText`, `NumText` |
| `pos` | `0, 0` | Screen coordinates; the third component sets `z` |
| `velocity` / `accel` | `0, 0` | Screen-space motion; a third component drives depth motion |
| `friction` | `1, 1` | Multiplied into velocity each tick |
| `removetime` | `1` | Countdown; `0` removes the text, negative lives forever |
| `layerno` | `0` | Negative draws behind fighters |
| `font` | `-1` | `bank, index`; `f` = fight.def, `m` = system.def; missing fonts fall back to the built-in font |
| `align` | `1` | `0` left, `1` center, `2` right |
| `scale` | `1, 1` | Glyph scale |
| `color` | `255,255,255,255` | RGBA in 0–255 units |
| `textspacing` | `0, 0` | Extra per-character spacing |
| `textdelay` | `0` | Ticks per revealed character; 0 shows the whole string |
| `angle` / `xshear` | `0` | Parsed and stored (the built-in font path is axis-aligned) |

Every tick the object advances `pos` by its velocity, applies friction and
acceleration, integrates depth velocity/acceleration into `z`, increments
`time`, and decrements a positive `removetime`.

## 3. Controllers

- `Text` spawns a new object. An empty `text` after formatting shows `params`.
- `ModifyText` matches by `id` (`-1` = every id) and `index` (`-1` = every
  match, otherwise the Nth match in creation order) and updates the text and
  visual fields. `id` selects targets but never rewrites the stored id.
- `RemoveText` uses the same matching; an omitted or negative `id` removes
  every text owned by the fighter.

## 4. Trigger

`NumText(id)` counts the fighter's texts with that id; `NumText(-1)` counts
all of them.

## 5. Rendering

Texts with negative `layerno` draw before the fighters; the others draw after
the effects and foreground projectiles, before the fight-screen HUD. Screenpack
fonts are used when available, otherwise the engine's built-in bitmap font.

## 6. Test plan

- Spawn, `%d` substitution, position, and `NumText` id filtering.
- Lifetime countdown and removal.
- `textdelay` typing.
- `ModifyText` by id/index, `RemoveText` by id/index and omitted id.
- Rollback checksum coverage.
