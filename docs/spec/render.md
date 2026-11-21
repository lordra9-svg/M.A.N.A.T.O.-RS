# Rendering, screenpack, and camera specification

**Status: APPROVED / FROZEN.** Approved on 2026-11-07 with the expanded scope
recorded in §1.1. No implementation lands against this document until it is
frozen; changes require a new approval.

Derived from the MIT-licensed Ikemen-GO source (`src/camera.go`,
`src/fightscreen.go`, `src/motif.go`, `src/bytecode.go`, `src/iniutils.go`,
`src/render.go`, `src/anim.go`, `src/common.go`), public M.U.G.E.N 1.1
documentation, and observed bundled content. No M.U.G.E.N source or binary is
consulted.

## 1. Scope

This spec closes compatibility rows V-03, V-04, V-05, V-06, V-07, and V-08.

In scope:

1. A normative paint-order contract for the whole frame.
2. Explod/effect parameter and rendering completion, including perspective
   projection (`xangle`, `yangle`, `xshear`, `projection`, `focallength`).
3. A M.U.G.E.N bitmap-font (`.fnt`) reader, required by screenpacks.
4. A fight-screen motif subset: `system.def` discovery plus `fight.def`
   life bars, power bars, guard bars, faces, names, timer, win icons, combo
   counter, round/announcement text, and action text.
5. Title screen and character-select screen from `system.def`, with
   `titlebgdef`/`selectbgdef` backgrounds and `select.def` roster parsing.
6. A front-end state machine (title → select → fight → result → title).
7. Camera zoom: stage `[Camera]` zoom keys, the `Zoom` state controller, the
   `ZoomVar` trigger, and zoom-aware screen mapping (Ikemen-GO only;
   M.U.G.E.N row stays `N/A`).
8. A bundled MIT screenpack ("Manato Grid") implementing the subset above.

### 1.1 Approval record

- Fight screen **plus title and select screens** approved.
- `.fnt` v1/v2 parsing approved as part of this pass.
- Explod perspective projection **included** rather than deferred.
- Bundled "Manato Grid" screenpack approved.
- Zoom remains Ikemen-only and stays `N/A` for M.U.G.E.N.

Still out of scope (documented as `Missing`/`Partial`, not silently skipped):

- VS screen, victory/win/results screens, game-over, continue, options,
  pause, hiscore, replay, dialogue, attract mode, demo mode.
- Storyboards and the motif Lua API.
- Team/tag layouts: simul/turns bar stacks, teammate face/name rows,
  tag-team select.
- Palette selection on the select screen; players use palette 1.
- Custom shaders (V-09), explod interpolation/tweening, stage shadow and
  reflection perspective.
- Language/localization variants.

If an out-of-scope key or section appears in content, it parses and is
ignored without error.

## 2. Paint order

One frame is painted in exactly this order. "Screen space" means the viewport
is already transformed by the camera (see §11); the fight screen, title
screen, select screen, and environment overlay are painted in raw viewport
space.

| # | Pass | Notes |
| --- | --- | --- |
| 1 | Stage layers with `layerno < 0` | Declared order; per-layer scissor and mask |
| 2 | Stage reflection | Skipped when `reflection.intensity <= 0` |
| 3 | Fighter shadows | Skipped for helpers, `invisible`, `noshadow` |
| 4 | Effects with `ontop = 0` | Stable sort by `sprpriority`, then insertion order |
| 5 | Fighters and helpers | Stable sort by `sprpriority`, then entity index |
| 6 | Projectiles with `projlayerno <= 0` | Stable sort by `projsprpriority`, then insertion order |
| 7 | Stage layers with `layerno == 0` | Declared order |
| 8 | Stage layers with `layerno > 0` | Declared order |
| 8b | Projectiles with `projlayerno > 0` | Same ordering as pass 6 |
| 9 | Effects with `ontop = 1` | Same ordering as pass 4 |
| 10 | Fight screen background elements | `bg*`, `top`, `mid`, `red`, `front`, bar `shift` |
| 11 | `envcolor` overlay | Full viewport, after the scene, before fight-screen text |
| 12 | Fight screen foreground elements | Text, faces, win icons, announcements |

Rules:

- Sorting is stable; ties resolve by entity index (fighters) or by insertion
  order (effects, projectiles).
- Scissor and sprite-mask state is pushed per stage layer and reset to
  disabled before the next pass.
- `assertspecial = invisible` hides the owning fighter and its shadows but
  not its explods.
- Projectiles honor `projscale`, `projangle`, `projxangle`/`projyangle`,
  `projxshear`, and `projprojection`/`projfocallength` in the same way as
  explods; an orthographic, unsheared, unrotated projectile uses the fast
  sprite path. `projwindow` clips drawing to a screen-space rect, and
  `projclsnscale`/`projclsnangle` scale/rotate collision boxes (rotated boxes
  use axis-aligned bounds). `velmul` multiplies each velocity component after
  acceleration every tick (default `1,1,1`). `projreflection > 0` draws a
  stage-styled reflection; negative values draw one only when the projectile
  also has a shadow color. `shader.name`/`shader.param0..15` and
  `shader.tex1.*`/`shader.tex2.*` bind a custom shader, its indexed params,
  and referenced textures for the projectile. `shader.time = 0` clears the
  shader; `shader.reflection` applies it to the projectile reflection.
  `shader.shadow` applies the custom shader when drawing the projectile's
  shadow. `afterimage.time/length/timegap/trans` emit a fading ghost trail
  drawn with the projectile's selected palette bank. `shader.afterimage`
  applies the projectile's custom shader (with its params and `tex1`/`tex2`
  bindings) to those ghosts; without the flag ghosts use the plain sprite
  pipeline. `supermovetime` lets a projectile keep moving
  during a global super pause until the timer expires (`0` or `< -1` freezes,
  `-1` moves forever); `pausemovetime` is tracked alongside it. `ownpal` and
  `remappal` are parsed and reported through `ProjVar(drawpal.group/index)`.
  Characters with palettes get dedicated recolored atlas banks (192–255, 64
  combos shared across all characters) for projectile drawing: `remappal`
  selects its destination palette, `ownpal` selects the sprite's own palette,
  otherwise the owner bank is used. Banks are only exhausted after 64
  character/palette combinations; the reassigned sprite files are retained so
  in-match overlay atlas rebuilds keep the palette banks resident. Overlay
  sprite and overlay-background banks are capped below the palette region.
  Non-zero `projshadow` draws a grounded ellipse whose color is the shadow
  RGB and whose alpha fades with height.
- Fighters honor the per-tick `TransformSprite` subset (`xshear`,
  `focallength`, `projection`) through the projected-quad path; `window` is
  parsed but not modeled as a per-fighter scissor.
- Effects owned by a hidden fighter still draw.
- Title and select screens replace passes 1–12 entirely; they never composite
  over a live match, except for the optional background used behind the
  pause menu (out of scope).

## 3. Bitmap fonts (`.fnt`)

M.U.G.E.N fonts are an INI file referencing a PCX/BMP atlas. Implement v1 and
v2.

### 3.1 v1 binary (`.fnt`)

A binary container: 12-byte `ElecbyteFnt\0` magic, `ver`/`ver2` (u16 LE), then
offsets and lengths for an embedded PCX atlas and an embedded INI text blob.

The text blob holds `[Def]` and a proportional `[Map]`.

| `[Def]` key | Meaning |
| --- | --- |
| `Type` | `0` = image atlas, `1` = RGB-drawn glyphs |
| `Bank` | Palette bank for the atlas |
| `Size` | `w, h` glyph cell size |
| `Spacing` | `x, y` advance between cells |
| `Offset` | Base draw offset |
| `Colors` | Palette rotation colours |

`[Map]` lines are `<char> = <offset>[, <width>]`:

- `<char>` is a literal character, `0xNN` hex, or `#NN` decimal.
- A missing width uses `Size.x`.
- A negative width shifts the offset left and uses its magnitude.
- The cursor advances by width, so omitting offsets gives a contiguous strip.

With no `[Map]`, the atlas is read as a 16-column grid and cell `n` maps to
character code `n`.

### 3.2 v2 (`.def`, `[FNT v2]`)

| Section | Keys |
| --- | --- |
| `[FNT v2]` | `version` (must be `2`) |
| `[Def]` | `Type`, `Size`, `Spacing`, `Offset`, `Color`, `File`, `Bank`, `BankType` |

- `Type = 1` (or `truetype`) is a solid-colour font; no atlas is loaded.
- Otherwise `File` names an SFF atlas. Sprites map as
  `group = palette bank`, `index = character code`; the default bank is
  `[Def] Bank` (default `0`). `BankType = sprite` accepts every group.
- `File` resolves relative to the font file, then `<font dir>/data/`.

### 3.3 Rendering

- Glyph advance is the glyph width plus `Spacing.x`; `Spacing.y` is line
  advance for multi-line text.
- A missing glyph draws nothing and does not advance.
- Type 1 draws a solid `Size` rectangle per glyph.
- Font `params` in `fight.def` (`fontN = path, bank, r, g, b`) override the
  palette bank and tint. Tint clamps to `0..255`.
- Scale is uniform per draw call; negative scale is not supported.
- `%` escapes are not interpreted by the font layer; the element formats the
  string (§6).
- If a referenced font fails to load, the element falls back to the built-in
  engine font and logs once per path. A bad glyph map is not fatal.
- `.fnt` parsing lives in `manato-formats`; it produces `BitmapFont`
  (glyph cells, spacing, atlas sprites) and never touches the GPU.

## 4. Explod and effect completion

`Explod` gains the parameters below. Existing parameters keep their meaning.

| Parameter | Default | Behaviour |
| --- | --- | --- |
| `anim` | required | Animation number; `f<group>`/`s` prefix selects fightfx |
| `id` | 0 | Matched by `ModifyExplod`, `RemoveExplod`, `NumExplod` |
| `pos` | `0, 0` | Offset from the anchor, mirrored by `facing` on X |
| `postype` | `p1` | `p1`, `p2`, `front`, `back`, `left`, `right` |
| `facing` | owner facing | `-1`, `1`, or `p1`/`p2` resolve to that player's facing |
| `vfacing` | `1` | Vertical flip of the sprite |
| `bindtime` | `0` | Ticks to stay attached to the anchor; `-1` = forever |
| `vel` | `0, 0` | Applied after the bind expires |
| `accel` | `0, 0` | Added to velocity each tick after the bind expires |
| `removetime` | `-1` | Lifetime in ticks; `-1` = animation length |
| `removeongethit` | `0` | Remove when the owner is hit |
| `removeonchangestate` | `1` | Remove when the owner changes state |
| `ignorehitpause` | `1` | Continue updating during hit/super pause |
| `supermovetime` | `0` | Ticks to update during super pause |
| `pausemovetime` | `0` | Ticks to update during hit pause |
| `scale` | `1, 1` | Multiplied with the animation frame scale |
| `sprpriority` | `0` | Sort key; `ontop = 0` moves it behind entities |
| `ontop` | `1` | `0` = behind fighters, `1` = in front |
| `shadow` | `0` | Draw the stage shadow under the explod anchor |
| `ownpal` | `0` | Use the owner palette instead of the sprite default |
| `trans` | `default` | `default`, `none`, `add`, `add1`, `sub`, `sub1`, `addalpha`, `alpha` |
| `alpha` | `256, 0` | Pair for `addalpha`/`alpha`, 0..256: **source**, **dest** |
| `angle` | `0` | Clockwise 2D rotation about the sprite axis |
| `xangle` | `0` | Rotation about the X axis, degrees |
| `yangle` | `0` | Rotation about the Y axis, degrees |
| `xshear` | `0` | Horizontal shear factor |
| `projection` | `orthographic` | `orthographic`, `perspective`, `perspective2` |
| `focallength` | `2048` | Perspective camera distance, localcoord units |
| `window` | full | `x1, y1, x2, y2` clip rect in screen space |
| `animelem` | `1` | Starting animation element |
| `animfreeze` | `0` | Hold the current frame while still counting time |
| `space` | `stage` | `stage` or `screen`; `screen` ignores the camera |

Alpha semantics (derived from `src/bytecode.go`, explod `trans`/`alpha`):

- `default` = normal blend with the animation frame alpha.
- `none` = normal blend, alpha 1.
- `add`/`add1` = additive blend, alpha 1.
- `sub`/`sub1` = subtractive blend, alpha 1.
- `addalpha` = additive blend with `alpha[1] / 256` as opacity.
- `alpha` = normal blend with `alpha[1] / 256` as opacity.
- When `trans = addalpha` or `alpha` is given without `alpha`, the pair
  defaults to `256, 0`; supply the pair to get a visible result.

Bind and lifetime rules:

- While bound, position follows the anchor plus the mirrored `pos` offset;
  `vel`/`accel` are not applied.
- When `bindtime` reaches 0, the explod is released at its current position
  and `vel`/`accel` start applying.
- `removetime = -1` expires with the animation; `0` means the same as `-1`.
- `removeongethit` removes the explod on the tick the owner enters `gethit`.
- `removeonchangestate = 1` removes the explod on any state change.

### 4.1 Perspective projection

Given a sprite quad with local corners `(x, y, 0)` relative to the pivot
(the sprite axis point plus the draw offset):

1. Apply shear: `x -= xshear * (y - pivot_y)`.
2. Rotate by `Rz(angle)`, then `Rx(xangle)`, then `Ry(yangle)`; angles are
   degrees. `Rz` is the existing 2D rotation.
3. Let `z` be the rotated corner's Z.
4. `orthographic`: `screen = pivot_screen + rotated.xy`.
5. `perspective`: `k = f / max(f - z, 1)`;
   `screen = pivot_screen + rotated.xy * k`.
6. `perspective2`: same `k`, but projected about the viewport centre:
   `screen = view_centre + (pivot_screen + rotated.xy - view_centre) * k`.

`f` is `focallength` scaled from localcoord units to viewport pixels:
`f_pixels = focallength * (view_h / localcoord_h)`. `perspective` places the
camera in front of the sprite and `perspective2` treats `xangle`/`yangle` as
a camera orbit around the screen centre; both use the same divide.

Corners are rasterised as two triangles with interpolated UVs. When all
three rotation angles are zero and `projection` is orthographic, the existing
`quad_rotated` path is used to keep output identical.

`ModifyExplod` accepts the same visual keys plus `id`; it updates the newest
matching explod of the owner. `facing` and `vfacing` accept any number and
clamp to forward/up (`>= 0`) or reversed/down (`< 0`). `RemoveExplod` with an explicit non-negative
`id` removes matching explods of the owner; an omitted `id` (or `-1`) removes
every explod owned by that fighter. `NumExplod` counts them.

Stage shadows and reflections gain `xangle`/`yangle`/`xshear`/`projection`
for parity with the stage format, but the sim only applies them to explods in
this pass; stage shadow/reflection perspective stays out of scope.

## 5. Screenpack discovery

A project may declare `screenpack = <path>` in `manato.toml` or
`--screenpack <path>` on the command line. The engine then loads:

1. `system.def` from that directory.
2. `[Files] fight` from `system.def`, resolved relative to the `system.def`
   directory, then `data/`, then the content root.
3. `[Files] spr`, `snd`, `fontN`, `select` (`select.def` path) from
   `system.def` and, when missing, from `fight.def`.

When no screenpack is configured, or `system.def` is unreadable, the engine
falls back to the built-in HUD (§13) and logs once.

### 5.1 `system.def` subset

| Section | Keys used |
| --- | --- |
| `[Info]` | `name`, `author`, `localcoord`, `mugenversion` |
| `[Files]` | `fight`, `spr`, `snd`, `fontN`, `select`, `selectbgdef` |
| `[Music]` | existing stage/global BGM keys |
| `[Title Info]` | `title.*`, `menu.*`, `menu.itemname.*`, `menu.valuename.*` |
| `[TitleBGdef]` | `spr`, `bgclearcolor`, `bgclearalpha`, `bgclearlayerno` |
| `[Select Info]` | §8 |
| `[SelectBGdef]` | `spr`, `bgclearcolor`, `bgclearalpha`, `bgclearlayerno` |

All other sections parse without error and are ignored.

`localcoord` defaults to `320, 240`. Screenpack coordinates are in localcoord
units and scale to the viewport by `viewport / localcoord` (uniform,
letterboxed; §10).

Background definitions (`[TitleBGdef]`, `[SelectBGdef]`) use the existing
stage BG parser: `spr`, `bgclearcolor`, `bgclearalpha`, `bgclearlayerno`,
`[BGDef]` `spr`, and numbered `[BG n]` layers with `type`, `spriteno`,
`start`, `delta`, `trans`, `tile`, `tilespacing`, `layerno`, `window`,
`mask`, and BGCtrl support. Animation IDs referenced by BGCtrl resolve
against the screenpack SFF.

## 6. `fight.def` subset

Sections, keys, and drawing rules are normative for the subset. Unknown keys
are ignored. Character and team prefixes (`p1.`, `p2.`, `team1.`, `team2.`)
are accepted; `pN.` applies to one player and overrides the unprefixed value.

### 6.1 `[LifeBar]` and `[LifeBarP1]`/`[LifeBarP2]`

| Key | Meaning |
| --- | --- |
| `pos` | Top-left anchor in localcoord |
| `range.x`, `range.y` | Visible width/height of the fill area |
| `bg0.*`, `bg1.*`, `bg2.*` | Background layers; `bg1` = mid-life, `bg2` = low-life |
| `top.*` | Foreground overlay drawn last |
| `front.*` | Bar fill animation, cropped to the life ratio |
| `mid.*` | Delayed "damage" fill behind `front` |
| `red.*` | Recoverable-life fill (from `life` to `red_life`) |
| `danger.*` | M.U.G.E.N 1.1 alias for `red` |
| `shift.*` | Fill offset, `shift.x` in pixels |
| `warn.*` | Warning overlay, shown in `warn.range` |
| `value.*` | Text element for the numeric life value |
| `mid.shift`, `mid.freeze`, `mid.delay`, `mid.mult`, `mid.steps` | Delayed-fill timing |

Rendering: the fill animation is cropped horizontally to
`ratio = life / life_max`, anchored to the name's side (P1 left, P2 right);
`top` is not cropped. `mid` lags behind `front` by `mid.delay` ticks and
drains at `mid.steps` pixels per tick times `mid.mult`, clamped to
`midlifeMin`. When `mid.freeze` is false the mid fill drains during hit
pause.

The `red` layer is cropped to `red_life / life_max` and drawn behind `front`,
so the visible red band spans current life to recoverable life. It is skipped
when `red_life <= life`, `life_max <= 0`, or the layer is empty. Critical-life
feedback remains the `bg1`/`bg2`/`warn` layers; `danger.*` remains an alias
for the same `red` layer.

Text: `%d` and `%i` format the integer life value; `%f` formats the ratio as
a percentage with no decimals.

### 6.2 `[PowerBar]`

Same layout keys as `[LifeBar]`, plus:

| Key | Meaning |
| --- | --- |
| `counter.*` | Text element for the numeric power value |
| `level1.*`, `level2.*`, `level3.*` | Level markers, drawn when power reaches `levelN.value` |
| `mid.*` | Delayed fill |
| `front.*` | Fill cropped to `power / power_max` |

Power is rendered in `max_power / 1000` level units; `counter` shows
`power / 1000` rounded down.

### 6.3 `[GuardBar]`

Same layout keys as `[LifeBar]`, driven by `guard` points:
`front` cropped to `guard / guard_max`, `mid` for the lagging fill, `value`
for the numeric text. Guard points come from the character’s `guardpoints`
data value; a missing or zero maximum hides the bar. Guarded hits reduce the
points by the applied guard damage, and the bar resets to full each round.
Guard crush is not modeled.

### 6.4 `[Face]`

| Key | Meaning |
| --- | --- |
| `pos` | Top-left anchor |
| `bg.*`, `bg0.*`, `bg1.*`, `bg2.*` | Background layers |
| `top.*` | Overlay |
| `face.spr` | `group, index` of the portrait in the character SFF |
| `face.palshare` | Use the character palette |
| `ko.*` | Overlay drawn when the character is KO'd |

Portrait sprites come from the character's own SFF.

### 6.5 `[Name]`

| Key | Meaning |
| --- | --- |
| `pos` | Anchor |
| `bg.*`, `top.*` | Optional layers |
| `name.font` | Font index into `[Files] fontN` |
| `name.text` | Static text (empty = character display name) |
| `name.align` | `0` left, `1` center, `2` right; default `0` |

Name text clips to `range.x` when set, otherwise to the viewport edge.

### 6.6 `[Time]`

| Key | Meaning |
| --- | --- |
| `pos` | Anchor |
| `bg.*`, `top.*` | Layers |
| `counter.font` | Font index |
| `counter.text` | Format string; `%d` = whole seconds |
| `counter.displaytime` | Show the counter for this many ticks after each change |
| `framespercount` | Ticks per displayed second, from `[Round]`/engine config |

The timer shows while a round is running, not during `Over`. The displayed
value is `ceil(ticks_remaining / frames_per_count)`.

### 6.7 `[WinIcon]`

| Key | Meaning |
| --- | --- |
| `pos`, `iconoffset` | Anchor and per-icon step |
| `n.*`, `s.*`, `h.*`, `c.*`, `t.*` | Win-type icons (normal, special, hyper, cheese, time) |
| `perfect.*`, `clutch.*` | Overlays for perfect/clutch wins |
| `bg0.*`, `top.*` | Layers |
| `useiconupto` | Maximum icons drawn; `0` = unlimited |
| `counter.*` | Optional numeric counter text |

Match wins are tracked as a list of win types. This pass distinguishes `n`
and `t`, maps every other win to `n`, and records perfect/clutch flags.

### 6.8 `[Combo]`

| Key | Meaning |
| --- | --- |
| `pos` | Anchor |
| `start.x` | Slide-in start X; the counter slides to `pos.x` |
| `counter.font`, `counter.text` | Hit count, `%d` |
| `text.font`, `text.text` | Static label, e.g. `"%d Hits"` |
| `displaytime` | Ticks after the last hit before hiding |
| `showspeed`, `hidespeed` | Slide speed in localcoord pixels per tick |
| `bg.*`, `top.*` | Layers |

Hidden when the hit count is 0 or the display timer expires. Drawn on the
opposite side of the attacking player, mirrored by `start.x` sign.

### 6.9 `[Round]`

| Key | Meaning |
| --- | --- |
| `pos` | Anchor |
| `start.waittime` | Ticks before the round announcement |
| `round.time` | Ticks `round1..N` is shown |
| `round.default.*`, `round.single.*`, `round.final.*` | Anim/text/sound elements |
| `fight.time`, `fight.*` | "Fight" announcement |
| `ctrl.time` | Tick at which control unlocks |
| `ko.time`, `ko.*` | KO announcement |
| `dko.time`, `dko.*`, `dko.showdraw` | Double KO |
| `to.time`, `to.*` | Time over |
| `win.time`, `win.*` | Win text |
| `win2.time`, `win2.*` | Alternate (perfect) win text |

`*` denotes `<name>.anim`, `<name>.spr`, `<name>.font`, `<name>.text`,
`<name>.snd`, `<name>.pos`, `<name>.layerno`. Only `anim`/`spr` or `text`
are required; missing parts are skipped independently.

Announcements emit `SoundEvent` entries on the `Common` channel with
group/index from `<name>.snd`. Announcement timing follows the rounds spec and
never changes round outcomes.

### 6.10 `[Action]` (subset)

`[Action]` elements are text banners driven by state controllers. Each
element supports `pos`, `spacing`, `start.x`, `text.*`, `displaytime`,
`showspeed`, `hidespeed`, `bg`, and `front`. At most 4 messages are visible;
older messages are dropped.

### 6.11 Font declarations

`fight.def` and `system.def` both accept
`fontN = <path>, <bank>, <r>, <g>, <b>`. Missing tint components default to
`255`. Paths resolve relative to the defining file, then `data/`, then the
content root.

### 6.12 `[WinQuote]` (`system.def`)

The victory quote is declared in `system.def` (`text`, `offset`, `font`,
`align`, `color`, `scale`). Once the match reaches `Over` with a winner, the
renderer draws the winning character's quote, selected by the `VictoryQuote`
controller index when that index exists, otherwise the first available
`[Quotes]` entry. If the character has no quotes, the motif's `text` is used.
Characters without quotes and an empty motif text draw nothing.

## 7. Title screen

Driven by `[Title Info]`, drawn over `[TitleBGdef]`.

| Key | Meaning |
| --- | --- |
| `title.font`, `title.text`, `title.offset`, `title.scale` | Optional title text |
| `title.anim`, `title.spr` | Optional title animation or sprite |
| `menu.pos` | Menu anchor |
| `menu.item.font` | Item font index |
| `menu.item.offset`, `menu.item.spacing` | Per-item layout |
| `menu.item.active.*` | Highlighted item override (font/text/offset/scale/color) |
| `menu.itemname.<key>` | Menu item labels |
| `menu.bg.<key>.*` | Optional per-item background |
| `menu.p1.cursor.*` | P1 cursor animation and position |
| `menu.p2.cursor.*` | P2 cursor animation and position |
| `menu.valuename.<key>` | Item value labels (unused in this pass) |
| `menu.background` | Optional static menu background |

Menu keys and order: `arcade`, `versus`, `training`, `options`, `quit`.
Only items with a non-empty `menu.itemname.<key>` are shown; an item whose
label is missing is skipped. `options` opens the built-in options list
(§9.2); `quit` exits the application.

Controls:

- P1 up/down moves the cursor; P1 `start`/`x` confirms.
- The menu wraps at both ends.
- A cursor step produces the `move.snd` sound when present.
- Confirm produces `done.snd` when present.

Drawing order: background layers by `layerno`; title animation/text; item
backgrounds; item labels with the active item overridden; cursors; any
footer text defined in content.

Timing: the title screen has no simulation tick budget and is not part of
rollback. Its animation clock is the app frame counter, capped at 60 Hz.

Fade: `fadein`/`fadeout` blocks are parsed but treated as an instant cut in
this pass; the keys do not change the screen flow.

## 8. Select screen

Driven by `[Select Info]`, drawn over `[SelectBGdef]`.

### 8.1 Roster (`select.def`)

`[Characters]` lines: `<def>[, <stage>][, <music>][, <key> = <value>...]`.

- Blank lines and `;` comments are ignored.
- The special token `randomselect` on its own line inserts one random slot.
- Positional fields after the def are stage then music; later `key = value`
  fields are character parameters.
- `single = 1` marks a single-mode-only character: it cannot be used as a team
  partner and its selection forces Single mode for that side. `order` is
  ignored; the roster keeps file order.
- A missing character def at match start is a load error, reported once.

Stage resolution: the stage named on the selected roster line, otherwise the
stage chosen by the stage list, otherwise the configured default stage.

### 8.2 Layout

| Key | Default | Meaning |
| --- | --- | --- |
| `rows` | `2` | Visible grid rows |
| `columns` | `5` | Visible grid columns |
| `wrapping` | `0` | Cursor wraps at the grid edges |
| `pos` | `0, 0` | Grid anchor |
| `showemptyboxes` | `0` | Draw empty slots up to `rows * columns` |
| `moveoveremptyboxes` | `0` | Skip empty slots while moving |
| `cell.size` | `0, 0` | Cell size; `0` = use the slot art |
| `cell.spacing` | `0, 0` | Gap between cells |
| `cell.bg.*` | | Empty cell background |
| `cell.random.*` | | Random-slot art |
| `cell.face.*` | | Character portrait inside the cell |
| `cell.name.*` | | Character name under the cell |

The grid scrolls by whole pages when the cursor leaves the visible page.
Page movement keeps the same in-cell position.

### 8.3 Cursors and selection

| Key | Meaning |
| --- | --- |
| `p1.cursor.startcell` | Initial cell `x, y` |
| `p1.cursor.active.*` | Cursor animation by cell state |
| `p1.cursor.done.*` | Confirmed cursor animation |
| `p1.cursor.move.snd` | Cursor movement sound |
| `p1.cursor.done.snd` | Confirm sound |
| `p1.face.*` | Large portrait beside the grid |
| `p1.name.*` | Selected character name |
| `p2.*` | Same for player 2 |
| `done.key` | Key that confirms the selection (default `start`) |
| `cancel.key` | Key that cancels back to the title (default `esc`) |
| `timer.*` | Optional select countdown; `0` = disabled |
| `stage.*` | Stage preview and stage list |

Behaviour:

- P1 moves with movement keys, P2 with its own movement keys.
- In versus mode both players select independently; P1 confirms with
  `start`/`x`, P2 with `start`/`x` on its own device.
- In arcade/training mode only P1 selects; the opponent is the CPU or the
  roster's next entry.
- `randomselect` slots choose a random roster entry deterministically from
  the app's seeded RNG (not rollback state).
- `cancel` returns to the title screen without starting a match.
- `timer` reaching zero selects the current cell and continues.
- Palette selection is out of scope; palette 1 is used.

### 8.4 Drawing

1. Background layers by `layerno`.
2. Grid cell backgrounds, then cell portraits, then cell names.
3. Large portraits and names for each player.
4. Cursors.
5. Stage preview, then timer.
6. Footer/help text from content.

## 9. Front-end flow

```
Title --arcade/versus/training--> Select --> Fight --> Result --> Title
  |                                  |
  +--options--> built-in options <---+
  +--quit--> exit
```

- `--menu` (or `--frontend`) starts at the title screen. Without it, the app
  keeps the current direct-to-match behaviour so headless tests and
  screenshots stay stable.
- `arcade` and `training` start a single match and return to the title on
  match over (arcade) or on cancel (training).
- `versus` starts a two-human match.
- `options` shows the built-in options list (§9.2) as an overlay over the
  title; changes apply to the next match.
- `Result` is the existing round-over banner; no separate screen.
- The front end never allocates a `SimState`; it only holds match settings
  and the roster. A match is built when the select screen confirms.

### 9.1 Match settings

From the front end: mode, P1 def, P2 def, stage, AI level, rounds, round
time. Command-line `--p1`, `--p2`, `--stage`, `--ai`, `--rounds`, and
`--round-time` remain the defaults for the select screen and are overridden
by it.

### 9.2 Built-in options

The options list is engine-owned (not screenpack-driven) and contains:
`Rounds`, `Round time`, `CPU level`, `Sound volume`, `Music volume`,
`Fullscreen`, `Back`. Navigation reuses the title controls. Values are
stored in memory only; no config file is written in this pass.

## 10. Screenpack layout and scaling

- Screen coordinates are localcoord units. The whole screen is transformed
  by a single uniform scale
  `min(viewport_w / localcoord_w, viewport_h / localcoord_h)` and centred.
- The game view is composed behind the fight screen with the same aspect
  logic; the fight screen never scales with camera zoom.
- Text elements honour `align` where present; bars do not.
- Elements with missing required keys are skipped individually; one bad
  element must not disable the screenpack.
- `layerno` on any element, when present, is clamped to `0..1`; `0` draws
  behind `1`.

## 11. Camera zoom (Ikemen-GO only)

### 11.1 Stage keys (`[Camera]`)

| Key | Default | Meaning |
| --- | --- | --- |
| `startzoom` | `1.0` | Initial scale at round start |
| `zoomout` | `1.0` | Minimum (most zoomed-out) scale |
| `zoomin` | `1.0` | Maximum (most zoomed-in) scale |
| `zoomindelay` | `0` | Ticks to wait before zooming back in |
| `zoominspeed` | `1.0` | Fraction of the remaining distance closed per tick when zooming in |
| `zoomoutspeed` | `1.0` | Same, when zooming out |
| `zoomanchor` | `0` | Anchor zoom at the stage floor instead of the screen centre |

Constraints, validated at load:

- `0 < zoomout <= zoomin`.
- `startzoom` is clamped to `[zoomout, zoomin]`.
- Values <= 0 are replaced by defaults and logged.

### 11.2 Zoom state

Per tick, after fighter positions are final:

1. Compute the visible extents of all non-helper fighters, including
   `size.` boxes and `width` overrides, plus the stage tension.
2. `target = clamp(view_w / extent_w, view_h / extent_h, zoomout, zoomin)`,
   also clamped so the view stays inside the playable area.
3. If `target < current`: `current += (target - current) * zoomoutspeed`,
   immediately, and reset the in-delay counter to `zoomindelay`.
4. If `target > current`: decrement the in-delay counter; while positive,
   hold. Otherwise `current += (target - current) * zoominspeed`.
5. Clamp `current` to `[zoomout, zoomin]`.
6. Clamp the camera position so the visible rect stays inside the camera
   bounds, adjusted by the current scale.

`startzoom` is applied on round reset; `current` is not smoothed across a
round reset.

### 11.3 `Zoom` state controller

| Parameter | Default | Behaviour |
| --- | --- | --- |
| `pos` | current | Target camera position `x, y` |
| `scale` | current | Target scale |
| `lag` | `0` | Ticks per step toward the target |
| `endlag` | `0` | Ticks to hold at the end before releasing |
| `time` | `0` | Total ticks of the override; `0` = instant |
| `camerabound` | `0` | Keep the override inside the camera bounds |
| `stagebound` | `0` | Keep the override inside the playable area |

While an override is active, automatic tracking is suspended; the override
steers `current` toward `scale`/`pos` over `time` ticks with `lag` smoothing
and releases after `endlag`. A second `Zoom` replaces the active override.

`camerabound` clamps the target and every smoothed step to the stage camera
bounds (`boundleft`/`boundright` inset by the half-visible width,
`boundhigh`/`boundlow`). `stagebound` does the same against the playable
player-info bounds (`leftbound`/`rightbound`). Both are applied after zoom
smoothing, so a shrink that would expose the outside of the stage cannot
push the camera past its bounds.

### 11.4 `ZoomVar` trigger

`ZoomVar(scale)`, `ZoomVar(pos.x)`, `ZoomVar(pos.y)`, `ZoomVar(lag)`,
`ZoomVar(endlag)`, and `ZoomVar(time)` read the active override, or the
current camera values when no override is active. `camerazoom` reads
`current` in all cases.

### 11.5 Screen mapping

With zoom `s` and camera position `c`:

```
screen_x = view_w / 2 + (world_x - c.x) * s
screen_y = ground_y + (world_y - c.y) * s
```

`ground_y` is the stage `zoffset` projected through the same transform, so
the floor stays fixed when zooming. `zoomanchor = 1` anchors at `ground_y`;
otherwise it anchors at the viewport centre. Effects with `space = screen`
bypass both camera translation and zoom.

## 12. Determinism and rollback

- Zoom state (`current`, target, in-delay counter, override) is part of the
  simulation snapshot, stepped in `SimState::tick`, and included in the
  rollback checksum.
- Camera position and zoom are computed from simulation state only; they read
  no wall-clock time, viewport size, or window state. Viewport size is a
  presentation-only scale applied at draw time.
- Fight-screen animation state (combo timers, announcement timers, mid-fill
  values, win icon lists) lives in `SimState` and is checksummed.
- The front end (title, select, options) is outside the simulation and is
  never checksummed.
- Font loading, screenpack parsing, and texture upload are outside the
  simulation and must not affect the checksum.

## 13. Fallback

When no screenpack is configured or parsing fails:

- The current built-in HUD remains the renderer: two life bars, a timer, a
  power bar per player, win pips, names, and the round banner.
- The built-in front end is used instead of the screenpack title/select
  screens: a plain text title list and a plain text roster grid using the
  built-in font.
- The built-in HUD is documented as a fallback, not as a compatibility row.
- The fallback must remain screenshot-stable so existing visual tests keep
  working.

## 14. Acceptance

- Unit tests for `.fnt` v1 and v2 parsing, proportional maps, and missing
  glyphs.
- Unit tests for explod `trans`/`alpha`, `bindtime` release, `space`,
  `removeongethit`, `window` clipping, and perspective corner projection.
- Unit tests for life/power/guard ratio cropping and mid-fill drain.
- Unit tests for `system.def`/`fight.def`/`select.def` parsing, including
  prefixed keys and missing sections.
- Front-end tests for menu navigation, roster pages, confirm/cancel, and
  versus selection.
- Zoom tests: `startzoom` at round start; zoom-out reaches `zoomout`;
  zoom-in respects `zoomindelay`; `Zoom` override and `ZoomVar` round-trip;
  checksums are deterministic.
- Integration: bundled `Manato Grid` drives title, select, and fight; a
  screenshot of each differs from the built-in fallback.
- `cargo test --workspace` green; screenshot run with `--frames 120` green.
- `COMPATIBILITY.md` rows V-03..V-08 updated honestly; anything still
  missing stays `Partial`/`Missing`.

## 15. Derived assertions

| # | Assertion | Source |
| --- | --- | --- |
| D-01 | Explod `trans = addalpha` with no `alpha` defaults to `256, 0` | `src/bytecode.go` explod alpha handling |
| D-02 | `alpha[1] / 256` is the opacity for `addalpha` and `alpha` | `src/bytecode.go` trans/alpha |
| D-03 | Effects sort by `sprpriority`; `ontop = 0` draws behind fighters | `src/bytecode.go` explod `ontop` |
| D-04 | Life bar `mid` drains `mid.steps` pixels per tick times `mid.mult`, after `mid.delay` | `src/fightscreen.go` `LifeBar.step` |
| D-05 | Timer value is `ceil(ticks / frames_per_count)` unless legacy time is enabled | `src/fightscreen.go` `FightScreenTime.draw` |
| D-06 | `zoomout <= zoomin`; `startzoom` is clamped into that range | `src/camera.go` `Reset` |
| D-07 | Zoom-out is immediate; zoom-in waits `zoomindelay` ticks | `src/camera.go` `Update` |
| D-08 | `Zoom` override suspends automatic tracking until `time`/`endlag` expire | `src/bytecode.go` `zoom.Run` |
| D-09 | `ZoomVar` reads the active override, otherwise the camera | `src/bytecode.go` `OC_ex2_zoomvar_*` |
| D-10 | Default `localcoord` is `320, 240`; `projection` defaults to `orthographic`; `focallength` defaults to `2048` | `src/motif.go` `Info`, `src/common.go` `ReadLayout` |
| D-11 | Perspective divides by `f / (f - z)`; `perspective2` projects about the screen centre | `src/render.go` `applyProjection` |
| D-12 | `xangle` and `yangle` are applied before the 2D `angle` | `src/render.go` `applyRotation` |
| D-13 | Select cell spacing mirrors a single value into both axes | `src/motif.go` `applyCustomDefaults` |
| D-14 | `[Characters]` tokens are `<def>[, <stage>][, <order>]` and `randomselect` inserts a random slot | `src/select.go` |
| D-15 | Title and select backgrounds are stage-style BG defs with their own SFF and BGCtrl | `src/motif.go` `TitleBgDef`, `SelectBgDef` |

These assertions stay provisional until the oracle harness exists. They are
committed as text, not as raw traces.
