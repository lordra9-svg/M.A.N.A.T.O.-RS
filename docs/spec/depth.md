# Depth and custom shaders

**Status: APPROVED (partial).** Derived from the MIT-licensed Ikemen-GO source
(`src/bytecode.go`, `src/char.go`).

## 1. Scope

- `Depth` — per-tick depth extents (`player`, `edge`, `value`, each
  `top, bottom`; a single value sets both).
- `DepthBound` — per-tick depth-bound flag.
- `z` components on `PosSet`, `PosAdd`, `VelSet`, `VelAdd`.
- `ShaderSet` — selects a `[Shaders]` entry by name for `time` ticks.
- `Shader()` — returns the selected shader name (`""` when none).
- A wgpu depth buffer with per-quad depth writes.

## 2. Position and extents

`Fighter.z`/`vel_z` carry the depth position; `z` advances by `vel_z` during
physics integration. `Depth` extents and `DepthBound`, like most per-tick
assertions, reset at the start of every tick, so content re-asserts them.

## 3. Rendering

The renderer owns a `Depth32Float` buffer cleared to `1.0` each frame. Sprites
write depth with `LessEqual`, so equal-depth draws keep their normal painter
order. Scene regions use fixed planes:

| Region | Depth |
| --- | --- |
| Stage background layers, reflections, shadows | 1.0 – 0.9 |
| Behind effects and negative-layerno text | 0.9 – 0.89 |
| Fighters | `0.5 + z * 0.001` clamped to `0.05..0.95` |
| Projectiles, foreground layers, front effects/text | 0.45 – 0.05 |
| Fight-screen HUD | 0.02 – 0.0 |

A positive `z` (farther) therefore sorts behind a `z = 0` fighter, and a
negative `z` sorts in front.

## 4. Shaders

`CharacterDef.shaders` parses the DEF `[Shaders]` section (`name = file`) with
lowercased names. `ShaderSet` accepts only names present in that table; unknown
names log and leave the selection unchanged. `time = 0` (or an empty name)
clears the selection; negative time never expires.

Shader *execution* is not modeled: content shader files are GLSL, and the
engine has no GLSL→WGSL translation or uniform/parameter binding layer yet.
The selection is exposed to content through `Shader()` and is carried in the
rollback checksum.

## 5. Not modeled

- Perspective scaling (`zscale`, `zoffset` projection) and `ModifyBGCtrl3D`.
- Z-axis attack/defence collision (`attack_depth`), `p2depth`, and depth bounds
  affecting hit detection.
- Custom shader execution, `shaderparam`, and texture inputs.
