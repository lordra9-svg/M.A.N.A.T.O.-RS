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

## 2.0 GetHitVar depth values

`HitDef ground.velocity`/`air.velocity`/`down.velocity`/`guard.velocity`/
`airguard.velocity` accept a third z component, and `zaccel` is parsed.
A connecting hit stores the selected velocity z in `GetHitVar(zvel)`, the
HitDef's `zaccel` in `GetHitVar(zaccel)`, and `fall.zvelocity` in
`GetHitVar(fall.zvel)`. `GetHitVar(zoff)` remains zero because hit position
offsets are not modeled.

## 2.1 Z-axis hits

`HitDef depth = top, bottom` (parsed into `attack_depth`) defines the
attacker's z extents around its own `z`:

```
attack = [z - attack_depth[0], z + attack_depth[1]]
defender = [z - depth_player[0], z + depth_player[1]]
```

The bands must overlap for a hit to connect, using the defender's current
`Depth` `player` extents. The check only runs when the stage defines a depth
range (`[PlayerInfo] topbound != botbound`); otherwise hits ignore depth, as in
Ikemen. The same band test applies to projectile hits.

Projectiles also carry `z`/`vel_z`/`accel_z`:
`offset = x, y, z` initialises the projectile at the owner's `z` plus the
offset (P2 postypes use P2's `z`), and `velocity`/`accel`/`remvelocity` accept a
third component. `projdepthbound` removes the projectile when its `z` leaves
`[z_min - bound, z_max + bound]`, and only when the stage defines a depth
range.

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

### 3.1 Perspective

The stage `[Scaling]` section supplies `topz`, `botz`, `topscale`,
`botscale`, and `depthtoscreen`. `Stage::z_scale(z)` interpolates the sprite
scale across `topz..botz` (clamped at 0, and `1.0` when the range is empty or
the stage does not define it). `z_y_offset(z)` is `z * depthtoscreen`.

When a fighter's `z` is non-zero, its draw anchor is projected as:

```
x = pivot_x + (x - pivot_x) * z_scale
y = ground + (y - ground) * z_scale + z * depthtoscreen
```

where `pivot_x` is the camera centre and `ground` is the ground line. Sprite
scale is multiplied by `z_scale`, so a fighter at the far plane is smaller and
shifted toward the horizon. With `z = 0` and no `[Scaling]` range the
transform is an identity.

### 3.2 Stage-layer perspective

Stage layers use the same transform when a depth is assigned. The layer's
start position is projected about the camera centre and ground line, its scale
is multiplied by `z_scale`, and its painter depth becomes
`0.5 + z * 0.001` (clamped `0.05..0.95`) for that layer only; other layers keep
their region plane.

### 3.3 Effects, shadows, and texts

`Explod pos` and `Text pos` accept a third z component; explods default to the
owner's (or `postype = p2`'s) `z` and add the offset, and bound explods follow
their anchor's `z` each tick. Shadows and these objects use the same
`[Scaling]` projection as fighters (position about the camera centre and
ground line, sprite scale multiplied by `z_scale`), and each object writes its
own painter depth `0.5 + z * 0.001` (clamped `0.05..0.95`) while keeping the
surrounding region plane for `z = 0`. Texts use a third `pos` component as an
engine extension; Ikemen text positions are 2D.

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

- GLTF 3D nodes. `ModifyBGCtrl3D` is modeled as a layer controller:
  `ctrlid` selects BG layers by `id` (`-1` = all) and
  `value = z[, xscale[, yscale]]`. Its `time` window is accepted but not
  applied (values persist until changed or the round restarts). Ikemen
  targets 3D model nodes here, which this engine does not load.
- Perspective for reflections and projectile sprites (effects, shadows, and
  texts are covered).
- Fighter `p2depth`/`depthbound` clamping (the flag is stored but does not
  constrain movement).
- Projectile z rendering/perspective (depth affects collisions and bounds
  only).
- Hit `zoff` (hit position offset) is always zero; `zvel`/`zaccel` and
  `fall.zvel` report the values applied by the connecting HitDef.
- Custom shader execution, `shaderparam`, and texture inputs.
