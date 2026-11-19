# Stage DEF/BG specification

**Status: APPROVED / FROZEN.** Derived from the MIT-licensed Ikemen-GO
source (`src/stage.go`), public M.U.G.E.N stage documentation, and observed
stages.

## 1. Scope

Parse M.U.G.E.N/Ikemen-GO stage definitions and render their background layers:

- `[Info]`, `[Camera]`, `[PlayerInfo]`, `[Bound]`, `[StageInfo]`
- `[Shadow]`, `[Reflection]`, `[Music]`
- `[BGdef]` and `[BG ...]` layers
- Stage-local `[Begin Action]` animations
- `[BGCtrl ...]` background controllers

Out of scope: stage shaders, video layers, 3D stages, and screenpack motifs.

## 2. File model

- Text is decoded with the shared tolerant decoder.
- `;` starts a comment.
- Section and key names are case-insensitive.
- Duplicate sections are preserved in order.
- Unknown sections/keys are ignored with a warning.
- `spr` resolves relative to the stage DEF, then through the data directories.
- Paths in `spr` and `[Music] bgmusic` accept Windows backslashes; they are
  normalized to `/` before resolution.

## 3. Info

```
[Info]
name = "..."
displayname = "..."
author = "..."
mugenversion = 1.1
localcoord = 320, 240
```

Defaults: `name` = file stem, `displayname` = `name`, `localcoord` = 320x240.

## 4. Camera

```
[Camera]
startx = 0
starty = 0
boundleft = -1000
boundright = 1000
boundhigh = -100
boundlow = 0
verticalfollow = .2
floortension = 0
tension = 50
overdrawhigh = 0
overdrawlow = 0
cuthigh = 0
cutlow = 0
ztension = 0
ztensionvel = 1
zautocenter = 0
zboundtop = 0
zboundbot = 0
```

- `startx/starty`: initial camera position.
- `boundleft/right`: horizontal camera clamp.
- `boundhigh/low`: vertical camera clamp.
- `verticalfollow`: 0..1 vertical follow strength.
- `floortension`: vertical follow dead zone.
- `tension`: horizontal camera dead zone.
- `ztension`: depth margin excluded at each end of `[PlayerInfo]
  topbound`/`botbound` when tracking the Z camera.
- `ztensionvel`: Z camera approach speed (clamped 0–20); the camera blends at
  `0.05 × ztensionvel` up to three times per tick and snaps within 0.1 depth
  units.
- `zautocenter`: centre the Z camera on the tracked depth range instead of the
  stage origin.
- `zboundtop/zboundbot`: clamp the resulting Z offset. Both zero means
  unbounded (engine decision; Ikemen clamps to zero by default so stages must
  opt in).
- Overdraw/cut values are parsed and preserved; clipping support is deferred.

## 5. PlayerInfo and Bound

```
[PlayerInfo]
p1startx = -70
p1starty = 0
p1facing = 1
p2startx = 70
p2starty = 0
p2facing = -1
leftbound = -1000
rightbound = 1000

[Bound]
screenleft = 15
screenright = 15
```

- Start positions and facing are used at round start.
- `leftbound/rightbound` clamp fighters.
- `screenleft/screenright` define the visible screen margin used by camera and
  edge triggers.

## 6. StageInfo

```
[StageInfo]
zoffset = 204
autoturn = 1
resetbg = 1
localcoord = 320, 240
xscale = 1
yscale = 1
portraitScale = 0.41666
```

- `zoffset`: screen Y of the ground line.
- `autoturn`: fighters turn toward each other when idle.
- `resetbg`: reset background animation/controllers between rounds.
- `localcoord`: stage-local coordinate space.
- `xscale/yscale`: stage scaling.

## 7. Shadow and Reflection

```
[Shadow]
intensity = 128
color = 0, 0, 0
yscale = .3
fade.range = 0, 0
xscale = 1
ydelta = 1

[Reflection]
intensity = 0
color = 255, 255, 255
yscale = 1
```

- `intensity`: shadow/reflection strength.
- `color`: RGB tint.
- `yscale`: vertical scale.
- `fade.range`: fade start/end heights.
- `xscale`, `ydelta`: additional scaling/position factors.

Reflection rendering is optional in the first implementation pass; values must
be parsed and preserved.

## 8. Music

```
[Music]
bgmusic = sound/stage.mp3
bgmvolume = 100
bgmloopstart = 0
bgmloopend = 0
bgmratio = .3
```

- `bgmusic`: path relative to the stage DEF.
- `bgmvolume`: 0..100.
- Loop points are optional.
- BGM playback is a separate audio subsystem; the stage parser preserves the
  values.

## 9. BGdef

```
[BGdef]
spr = stage.sff
debugbg = 0
bgclearcolor = 0, 0, 0
```

- `spr` selects the stage SFF.
- `debugbg` is a debug flag.
- `bgclearcolor` is the clear color.

## 10. BG layers

```
[BG 0]
type = normal
spriteno = 0, 0
actionno = -1
start = 0, 0
delta = 1, 1
trans = none
mask = 0
tile = 0, 0
tilespacing = 0, 0
layerno = 0
velocity = 0, 0
sin.x = 0, 0, 0
sin.y = 0, 0, 0
cos.x = 0, 0, 0
cos.y = 0, 0, 0
id = 0
positionlink = 0
window = 0, 0, 0, 0
startrect = 0, 0, 0, 0
windowdelta = 0, 0
```

### 10.1 Types

- `normal`: static sprite.
- `anim`: animated action via `actionno`.
- `parallax`: layer uses parallax width/scale.
- `dummy`: no drawing, used by controllers.
- `top` / `bottom`: aliases for layer numbers if content uses them.

### 10.2 Drawing

- Screen position:
  `x = start.x - camera.x * delta.x + controller offsets`
  `y = start.y - camera.y * delta.y + controller offsets`
- `spriteno` selects the sprite for `normal`.
- `actionno` selects a stage animation for `anim`.
- `tile = x, y`: repeat along each axis.
- `tilespacing` adds spacing between tiles.
- `layerno < 0` draws behind fighters; `layerno > 0` draws in front.
- `trans = none|add|add1|sub` selects blend mode.
- `mask` is parsed; masking is deferred.

### 10.3 Motion

- `velocity = x, y`: constant movement.
- `sin.x = amplitude, period, phase`; `sin.y` similarly.
- `cos.x/cos.y` are supported equivalents.
- `positionlink = 1`: position is linked to the previous layer’s motion.
- Window/startrect/windowdelta are parsed and preserved.

## 11. BG animations

Stage-local `[Begin Action N]` blocks use the AIR parser. They can be referenced
by `actionno` or by `[BGCtrl]` controllers.

## 12. BGCtrl controllers

```
[BGCtrl 1]
type = anim
trigger1 = time = 0
value = 1

[BGCtrl 1]
type = velset
trigger1 = time = 0
x = 0
y = -0.5
```

Required controller types:

- `anim`: change layer action
- `visible`: toggle drawing
- `enable`: toggle updates
- `posset`, `posadd`
- `velset`, `veladd`
- `sinx`, `siny`
- `palfx`
- `remappal`
- `shadercolor` (parsed, deferred)

Triggers use the same expression engine as characters. Controllers run once per
tick per matching layer.

`time = start[, end[, loop]]` controls the run window (Ikemen's
`bgCtrlAction`): a controller is active when the stage tick is at least
`start` and either at most `end` (no loop) or within `end - start` of each
`loop`-tick period. `loop` defaults to `-1`; a negative `start` or
`start >= loop` disables the controller. `ModifyBGCtrl` can retime and
re-parameterize controllers at runtime by `sctrlid`, including layer palette
saturation/hue/inversion, additive/multiplicative tint, their
`sincolor`/`sinhue`/`sinadd`/`sinmul` oscillations, and per-layer custom
shader/texture references.

## 13. Error handling

- Missing SFF: log and render a solid background.
- Missing sprite/action: skip the layer.
- Invalid numbers: use defaults and log.
- Unknown BG types: treat as `normal`.
- Empty BG list: fall back to a solid background.

## 14. Test plan

- Parse all bundled and stock stage DEFs.
- Camera bounds, player starts, zoffset, localcoord.
- Static, animated, tiled, and parallax layers.
- BG controllers: anim, pos, vel, sin/cos.
- Shadow/reflection/music values.
- Missing SFF/sprite/action.
- Unknown sections/keys.
- Render parity with the bundled Training Grid.

## 15. Sources

- Ikemen-GO `src/stage.go` (MIT).
- Public M.U.G.E.N stage documentation.
- Observed `stage1.def` and stock stages.
