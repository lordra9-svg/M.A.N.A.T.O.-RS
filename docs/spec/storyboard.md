# Storyboards

**Status: APPROVED (partial).** Derived from the MIT-licensed Ikemen-GO source
(`src/storyboard.go`) and the M.U.G.E.N storyboard scene format.

## 1. Scope

A storyboard is a DEF file with `[Info]`, `[SceneDef]`, `[Scene ...]` sections
and inline `[Begin Action N]` animations. The player renders full-screen
scenes with sprite/animation layers, text, fades, and scene jumps.

## 2. Format

- `[Info] name`, `author`.
- `[SceneDef] spr` (SFF), `snd` (SND), `fontN` (`.fnt` files), `startscene`
  (index).
- `[Scene <name>]`:
  - `end.time` (ticks; `-1`/`0` never ends on its own),
  - `jump` (target scene name, else the next scene),
  - `layerall.pos = x,y` (offset added to every sprite and text layer),
  - `clearcolor = r,g,b`, `clearalpha = a`,
  - `fadein.time`/`fadeout.time`, `fadein.color`, `fadeout.color`,
    `fadein.alpha = start,end`, `fadeout.alpha = start,end`,
  - `layerN.anim` or `layerN.spr = group,index`,
  - `layerN.offset`, `layerN.scale`, `layerN.facing`, `layerN.layerno`,
    `layerN.starttime`, `layerN.endtime` (`-1` = scene end),
  - `layerN.velocity`, `layerN.accel`, `layerN.friction` (friction is parsed
    but not integrated),
  - `layerN.text`, `layerN.textdelay`, `layerN.font`
    (`index,r,g,b,a,...`; the built-in bitmap font is used for rendering),
  - `layerN.palfx.*`: `add = r,g,b`, `mul = r,g,b` (0..256; mul defaults to
    `256,256,256`), `time`, `color`, `hue`, `sinadd = r,g,b,time`,
    `sinmul = r,g,b,time`, `sincolor = amount,time`, `sinhue = amount,time`,
    `invertall`, and `invertblend`. `time = -1` means infinite and `0` disables
    the effect; positive values expire after that many ticks. Sine phases start
    at scene time 0 and follow Ikemen's `sinAdd`/`sinMul`/`sinColor`/`sinHue`
    formulas.
- `soundN.value = group,index`, `soundN.starttime`, `soundN.volume`
  (`volumescale` alias), `soundN.pan`.
- `bgm` (or `bgmusic`) with `bgm.loop`, `bgm.volume`, `bgm.loopstart`,
  `bgm.loopend`, `bgm.freqmul`, `bgm.loopcount`.
- Inline AIR actions provide animation frames for `layerN.anim`.

## 3. Playback

`StoryboardPlayer` keeps the current scene index and scene time. Each tick
advances time; when `end.time` is reached the player follows `jump` (looked up
case-insensitively, spaces and underscores equivalent) or advances to the next
scene. Running past the last scene sets `done`.

Layer visibility uses `starttime`/`endtime`; `textdelay` reveals one character
per N ticks; `velocity`/`accel` integrate position from `starttime`.
`layerall.pos` is added after the per-layer position. Animation frames come
from the inline AIR actions, including their `Interpolate Offset/Scale/Angle`
markers, so poses between frames are blended exactly like fighter animations. Fade-in
alpha decreases over `fadein.time`; fade-out alpha increases over the last
`fadeout.time` ticks before `end.time`.

Scene sounds fire once when `time == starttime` and are exposed through
`StoryboardPlayer::take_sounds()` as `(group, index, volume, pan)`; the app
plays them from the storyboard's `snd` bank. The app starts/stops scene BGM
when the current scene changes, using the parsed loop/volume/frequency options.

## 4. Rendering

`Renderer::load_storyboard` builds a dedicated atlas bank from the storyboard
SFF; `render_storyboard` clears to `clearcolor`/`clearalpha`, draws layers
sorted by `layerno`, overlays fade alpha, and draws text with the storyboard
`fontN` screen font when `layerN.font` selects one (otherwise the built-in
bitmap font).

## 5. In-match overlays

The `Storyboard` controller (`path`, `id`, `space`, `pos`, `scale`, `facing`,
`bindtime`) loads a storyboard and adds a `StoryboardOverlay` to the match.
Overlays tick their own `StoryboardPlayer`, advance scene time, and are removed
when the scene set completes. `ModifyStoryboard` matches overlays by `id`
(`-1` = all owned) and updates position, scale, facing, and space.

Layer PalFX is applied per sprite/text quad by the sprite shader in the
Ikemen order: hue rotation, invert, saturation (`color`), add, then multiply.
Because it runs on sampled texels it affects the source sprite and text glyphs
rather than only the tint.

`palfx.invertblend` follows Ikemen's blend-pass mapping: negative values
(`<= -2`) become `3` while `invertall` is active. On additive/subtractive
sprite passes, values `>= 1` swap Add and Sub, negate the additive color, and
`3` also disables the shader invert because reverse-subtract blending already
inverts the result. Normal-blended sprites and text are unaffected by the
blend swap.

In-match overlays draw both **sprite/animation layers** and text layers at the
overlay position/scale/facing in screen or world space. The renderer assigns
one atlas bank per live overlay (`bank::OVERLAY_BASE + index`) and rebuilds the
sprite atlas when the overlay set changes, so each overlay uses its own
`[SceneDef] spr` SFF. Scene `clearcolor`/`clearalpha` fill the screen before
the overlay's layers, and `fadein`/`fadeout` alpha is drawn over them.

## 6. Lua scripting

`[SceneDef] lua = file.lua` (alias `script`) loads a sandboxed script. The
runtime exposes only `table`, `string`, and `math`, rejects `io`, `os`, and
`require`, and caps Lua heap allocations at 4 MiB. The script may define:

```lua
function update(ctx)
    ctx.x = ctx.x + math.sin(ctx.time / 8) * 4
end
```

`ctx` fields: `scene`, `time`, `x`, `y`, `scale_x`, `scale_y`, `facing`,
`alpha`, `done`, and `jump`. Numeric transforms are read back every rendered
frame for in-match overlays; `done = true` hides the overlay for that frame.
`jump` is accepted by the context but scene switching from Lua is not modeled
yet. Scripts are transient render-side state and are not included in rollback
snapshots (storyboard overlays are already excluded).

## 7. Not modeled

- Dynamic backgrounds and 3D models.
- Lua-driven scene jumps (the `jump` context field is parsed but unused).
