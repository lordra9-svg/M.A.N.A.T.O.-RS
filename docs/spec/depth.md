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

With `DepthBound value = 1` and a stage depth range, the integrated `z` is
clamped to `[depth_edge.top + z_min, z_max - depth_edge.bottom]`, matching
Ikemen's `zDepthBound`. Without a `[PlayerInfo] topbound`/`botbound` range the
clamp is disabled. `movecamera` is accepted but not modeled.

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
their region plane. Projectile sprites and fighter reflections use the same
perspective and per-object painter depth.

### 3.3 Effects, shadows, texts, projectiles, and reflections

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

Shader selection is exposed through `Shader()` and carried in the rollback
checksum. The renderer compiles a selected shader on first use when the
`[Shaders]` file is **WGSL** (`.wgsl`). The file must define both `vs_main` and
`fs_main` using the engine sprite interface:

- vertex inputs: `@location(0) pos: vec2<f32>`, `(1) uv: vec2<f32>`,
  `(2) color: vec4<f32>`, `(3) z: f32`;
- bindings: `@group(0) @binding(0)` texture2d, `(1)` sampler.

GLSL files are also executed as fragment shaders (`.glsl`, `.frag`, `.fs`). The
renderer pairs them with a built-in GLSL vertex shader. Both explicit
Vulkan-style bindings/locations and Ikemen's implicit OpenGL/GLES style are
accepted. Implicit shaders are rewritten before compilation:

- missing `#version` gets `#version 450`,
- `uniform sampler2D NAME` maps to explicit texture/sampler pairs for
  `tex`/`Texture` (0/1), `tex1` (3/4), `tex2` (5/6), and `pal` (7/8),
- implicit `in vec3 vTexCoord`/`in vec2 uv`/`in vec4 color` and
  `out vec4 FragColor` get locations 0/1/0, with `vTexCoord.z` reading `0.0`,
- `TextureSize` maps to 320x240, and `add`/`mult`/`alpha`/`neg`/`gray`/`hue`
  map to `shaderparam` slots `param[0]`, `param[1]`, `param[2].x/.y/.z/.w`,
- Ikemen's dual `#if __VERSION__ >= 450 ... #else ... #endif` template keeps the
  GL branch.

Explicit shaders can still declare the full binding set:

```glsl
#version 450
layout(location = 0) in vec2 uv;
layout(location = 1) in vec4 color;
layout(binding = 0) uniform texture2D tex;
layout(binding = 1) uniform sampler samp;
layout(binding = 2) uniform ShaderParams { vec4 param[4]; vec4 misc; } sp;
layout(binding = 3) uniform texture2D pal;
layout(binding = 4) uniform sampler palSamp;
layout(binding = 5) uniform texture2D tex1;
layout(binding = 6) uniform sampler tex1Samp;
layout(binding = 7) uniform texture2D tex2;
layout(binding = 8) uniform sampler tex2Samp;
layout(binding = 9) uniform texture2D bgl_RenderedTexture;
layout(binding = 10) uniform sampler grabSamp;
layout(location = 0) out vec4 frag;
void main() {
    frag = texture(sampler2D(tex, samp), uv) * color;
}
```

`ShaderSet shaderparam = count, index, value[, index, value...]` fills the
`param` array (16 slots). `misc` is `(time_seconds, 320, 240, aspect)`.
`pal` is a real 256x1 RGBA palette strip uploaded from the sprite's SFF
palette (per bank and palette key), so palette-index shaders sample the actual
colors. `ShaderSet tex1 = group,index` / `tex2 = group,index` select SFF
sprites from the executing character; the renderer uploads those sprites as
standalone RGBA textures (cached per bank/group/index) and binds them to
`tex1`/`tex2` bindings 3/5. Without references, those slots fall back to the
sprite's atlas page.

Compiled shaders are cached per name and usable with normal/add/sub blends and
the depth buffer; selecting a shader splits the painter's draw ranges so only
that fighter's quads use it. Compilation failures fall back to the default
sprite pipeline.

Implicit-location GLSL is normalized by the preprocessor above. `param`
(`shaderparam`), a real `pal` palette texture, real `tex1`/`tex2` sprite
references, and a grab texture (`bgl_RenderedTexture` at bindings 9/10) are
provided. The grab pass copies the previous finished game target to a
separate texture before the next render pass, so grab shaders see the frame
before the current one (one frame of latency).

## 5. Not modeled


- GLTF 3D nodes. `ModifyBGCtrl3D` is modeled as a layer controller:
  `ctrlid` selects BG layers by `id` (`-1` = all) and
  `value = z[, xscale[, yscale]]`. Its `time` window is accepted but not
  applied (values persist until changed or the round restarts). Ikemen
  targets 3D model nodes here, which this engine does not load.
- Hit `zoff` (hit position offset) is always zero; `zvel`/`zaccel` and
  `fall.zvel` report the values applied by the connecting HitDef.
- Grab passes and non-placeholder shader texture inputs.
