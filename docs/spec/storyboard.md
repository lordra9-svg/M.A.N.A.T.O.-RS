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
  - `layerN.palfx.add = r,g,b` and `layerN.palfx.mul = r,g,b` (0..256 scale;
    mul defaults to `256,256,256`). The tint is `color * mul / 256 + add / 256`,
    clamped to 0..1, and applies to sprite and text layers in both front-end
    scenes and in-match overlays.
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

In-match overlays draw both **sprite/animation layers** and text layers at the
overlay position/scale/facing in screen or world space. The renderer assigns
one atlas bank per live overlay (`bank::OVERLAY_BASE + index`) and rebuilds the
sprite atlas when the overlay set changes, so each overlay uses its own
`[SceneDef] spr` SFF. Scene `clearcolor`/`clearalpha` fill the screen before
the overlay's layers, and `fadein`/`fadeout` alpha is drawn over them.

## 6. Not modeled

- Lua storyboard scripting, dynamic backgrounds, and 3D models.
- Time-varying PalFX parameters (`time`, `color`, `hue`, `sinadd`, `sinmul`,
  `invertall`); only static `add`/`mul` are applied.
