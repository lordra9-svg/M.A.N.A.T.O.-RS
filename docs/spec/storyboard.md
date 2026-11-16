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
  - `clearcolor = r,g,b`, `clearalpha = a`,
  - `fadein.time`/`fadeout.time`, `fadein.color`, `fadeout.color`,
    `fadein.alpha = start,end`, `fadeout.alpha = start,end`,
  - `layerN.anim` or `layerN.spr = group,index`,
  - `layerN.offset`, `layerN.scale`, `layerN.facing`, `layerN.layerno`,
    `layerN.starttime`, `layerN.endtime` (`-1` = scene end),
  - `layerN.velocity`, `layerN.accel`, `layerN.friction` (friction is parsed
    but not integrated),
  - `layerN.text`, `layerN.textdelay`, `layerN.font`
    (`index,r,g,b,a,...`; the built-in bitmap font is used for rendering).
- `soundN.value = group,index`, `soundN.starttime`, `soundN.volume`
  (`volumescale` alias), `soundN.pan`.
- Inline AIR actions provide animation frames for `layerN.anim`.

## 3. Playback

`StoryboardPlayer` keeps the current scene index and scene time. Each tick
advances time; when `end.time` is reached the player follows `jump` (looked up
case-insensitively, spaces and underscores equivalent) or advances to the next
scene. Running past the last scene sets `done`.

Layer visibility uses `starttime`/`endtime`; `textdelay` reveals one character
per N ticks; `velocity`/`accel` integrate position from `starttime`. Fade-in
alpha decreases over `fadein.time`; fade-out alpha increases over the last
`fadeout.time` ticks before `end.time`.

Scene sounds fire once when `time == starttime` and are exposed through
`StoryboardPlayer::take_sounds()` as `(group, index, volume, pan)`; the app
plays them from the storyboard's `snd` bank.

## 4. Rendering

`Renderer::load_storyboard` builds a dedicated atlas bank from the storyboard
SFF; `render_storyboard` clears to `clearcolor`/`clearalpha`, draws layers
sorted by `layerno`, overlays fade alpha, and draws text with the storyboard
`fontN` screen font when `layerN.font` selects one (otherwise the built-in
bitmap font).

## 5. Not modeled

- Lua storyboard scripting, dynamic backgrounds, 3D models, BGM events,
  `layerall.pos`, interpolation, palFX, and the in-match
  `Storyboard`/`ModifyStoryboard` state controllers.
