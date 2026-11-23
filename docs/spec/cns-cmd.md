# CNS/CMD specification

**Status: APPROVED / FROZEN.** Derived from the MIT-licensed Ikemen-GO
source (`src/compiler.go`, `src/compiler_functions.go`, `src/input.go`,
`src/char.go`), public M.U.G.E.N documentation, and observed content.

## 1. Scope

Parse and execute character state files and command definitions:

- `[Statedef]` headers and `[State]` controllers
- Trigger expressions and redirects
- Command definitions and command matching
- State-file merge order
- Command-driven state controllers (`[Statedef -1]`)

Out of scope for this spec: rendering, stage state files, and Ikemen-only
controllers/triggers not listed in `COMPATIBILITY.md`.

## 2. File model

CNS/CMD files are INI-like text:

- `;` starts a comment.
- Section headers are case-insensitive.
- Keys are case-insensitive and trimmed.
- Values are trimmed; quoted values keep their contents.
- Duplicate sections are preserved in file order.
- Duplicate `[Statedef N]` definitions: the later file wins during merge.
- Unknown sections are retained but ignored by the state machine.

### 2.1 Merge order

Character state files are merged in this order:

1. `stcommon`
2. `cns`
3. `st`, then `st0`, `st1`, … `stN` in natural numeric order (`st10` follows
   `st9`)
4. CMD file

Later files override earlier definitions for the same state number. Controllers
from the CMD file attach to its `[Statedef -1]` (or create a synthetic `-1`).
DEF `[Files]` paths accept Windows backslashes as separators.

## 3. Statedef

```
[Statedef N]
type = S|C|A|L|U
movetype = I|A|H|U
physics = S|C|A|N|U
anim = N
ctrl = 0|1
velset = x, y
poweradd = N
juggle = N
sprpriority = N
facep2 = 0|1
hitdefpersist = 0|1
movehitpersist = 0|1
hitcountpersist = 0|1
```

Defaults:

- `type = S`
- `movetype = I`
- `physics = N`
- `ctrl` unchanged
- `anim` unchanged
- `velset` unchanged
- `poweradd = 0`
- `juggle`: unchanged when omitted; an attack state without the parameter
  reuses the previous value (set `juggle = 0` explicitly to clear it)
- `sprpriority = 0`
- `facep2 = 0`
- `hitdefpersist = 0`: the active HitDef and its per-activation hit
  targets/counts are cleared on state entry
- `movehitpersist = 0`: `MoveContact`/`MoveHit`/`MoveGuarded`/`MoveReversed`
  are cleared on state entry
- `hitcountpersist = 0`: `HitCount`, `UniqHitCount`, and the guarded count
  are cleared on state entry

`U` means “keep the previous value.” Entering a state applies the header before
the first controller runs.

## 4. Controllers

A controller is a `[State N, name]` section attached to the enclosing
`[Statedef]`. The number in `[State N]` is a label, not a lookup key.

Required controller families for the approved matrix:

### 4.1 Movement and state

- `VelSet`, `VelAdd`, `VelMul`
- `PosSet`, `PosAdd`, `PosFreeze`
- `ChangeState`, `SelfState`, `ChangeAnim`, `ChangeAnim2`
  `ChangeState`/`SelfState` accept `continue`; with `continue = 1` the
  remaining controllers in the current block still run after the transition.
  `ChangeAnim`/`ChangeAnim2` accept `value`, the 1-based `elem`, and
  `elemtime` (ticks into that element). An invalid element resets to the
  first frame with `elemtime = 0`; a negative or out-of-duration `elemtime`
  resets to 0.
- `CtrlSet`, `StateTypeSet`, `Turn`
  `StateTypeSet` accepts single letters, quoted values, or full-word names by
  their first letter (`A`, `"Air"`, `Crouch`); `U` leaves a field unchanged.
  Ikemen-GO and M.U.G.E.N 1.1 have no separate `MoveTypeSet` controller.
- `VarSet`, `VarAdd`, `VarRangeSet`, `VarRandom`
- `PowerAdd`, `PowerSet`, `LifeAdd`, `LifeSet`
  `LifeAdd` scales negative values by final defence unless `absolute = 1`,
  always deals at least 1 point of damage, leaves healing unscaled, and adds
  the applied damage to `ReceivedDamage`.
- `DefenceMulSet`, `AttackMulSet`
- `SprPriority`, `Width`, `ScreenBound`, `PlayerPush`
  `ScreenBound value` clamps the fighter inside the stage bounds per axis and
  accepts one value for both axes or an `x,y` pair. `movecamera` (also
  `movecameraX`/`movecameraY`) toggles whether the fighter drives fighting
  camera tracking on each axis; omitted flags default to enabled for
  M.U.G.E.N-only content. An axis with no tracking fighters falls back to the
  active roster.
- `AssertSpecial` (render/round flags plus `nohitdamage`, `noguarddamage`,
  `nocornerpush`, `nofallcount`, `nodizzypointsdamage`, and
  `noguardpointsdamage`, which suppress the matching combat effects while
  the flag is asserted; `noinput` ignores the player's frame but keeps
  engine-asserted input; `nolifebardisplay`/`nopowerbardisplay`/
  `noguardbardisplay` hide the corresponding per-side bars; `nojump`,
  `nocrouch`, `nostand`, and `noairjump` block the built-in jump, crouch,
  crouch-to-stand, and air-jump transitions)

### 4.2 Combat

- `HitDef`, `ReversalDef`
- `Projectile`, `ModifyProjectile`
- `NotHitBy`, `HitBy`, `HitOverride`
- `HitVelSet`, `HitFallSet`, `HitFallVel`, `HitFallDamage`
- `TargetBind`, `TargetState`, `TargetFacing`, `TargetLifeAdd`,
  `TargetPowerAdd`, `TargetVelSet`, `TargetVelAdd`
- `Pause`, `SuperPause`
  `Pause` models `time` and `movetime`. `SuperPause` models `time` (default
  30), `movetime`, `poweradd`, and `sound`; `darken`/`brightness`, the `anim`
  effect, `pausebg`, `endcmbuftime`, `p2defmul`, and `unhittable` are not
  modeled (hit resolution is already frozen for the pause duration).
- `EnvShake`, `FallEnvShake`
- `GuardPointsAdd`, `GuardPointsSet`, `GuardBreakSet`
- `DizzyPointsAdd`, `DizzyPointsSet`, `DizzySet`
- `ModifyPlayer` (`lifemax`, `guardpointsmax`, `dizzypointsmax`, `attack`,
  `defence`, `ailevel`)
- `TargetGuardPointsAdd`, `TargetDizzyPointsAdd`
- `TargetGuardPointsSet`, `TargetDizzyPointsSet`
- `GetHitVarSet` (modeled GetHitVar fields)
- `MapSet`, `MapAdd`, `MapReset` (per-fighter string-keyed maps)
- `Height` (per-tick top/bottom size offset)
- `AngleSet`, `AngleAdd`, `AngleMul`, `AngleDraw` (2D draw angle/scale)
- `Camera` (`view = fighting|follow|free`, `followid`, `pos`)
- `ModifyHitDef` (overrides modeled fields of the active HitDef)
- `AttackDist` (updates the active HitDef `guard.dist`)
- `ModifyReversalDef` (reversal attr/guardflag overrides)
- `ModifyShadow`, `ModifyReflection`
  (intensity/color/offset/xscale/yscale overrides)
- `ExplodBindTime` (rebind matching explods by id)
- `ModifyStageVar` (camera bounds/framing/zoom, `ztension`/`ztensionvel`/
  `zautocenter`/`zboundtop`/`zboundbot`, and player-info bounds subset)
- `ModifyStageBG` (selects layers by `id`/`index`, then overrides `actionno`,
  `velocity.x/y`, `delta.x/y`, `start.x/y`, `scalestart`, `pos.x/y`,
  `layerno`, `spriteno`, `trans` (numeric add/sub/subadd type), the sprite
  `angle`/`xangle`/`yangle`/`xshear`, `projection`/`focallength`, and
  `shader`/`shaderparam`/`tex1anim`/`tex1spr`/`tex2anim`/`tex2spr`.
  Nonzero rotation/shear or a non-orthographic projection renders through the
  projected quad path)
- `ModifyBGCtrl` (matches stage controllers by `sctrlid` and updates the
  `time = start[, end[, loop]]` window, `value`, and `x`/`y`, plus palette
  keys `color` (0–256 saturation), `hue` (-256..256, 512 units per turn),
  `invertall`, `invertblend` (≥2 enables inversion), `add`/`mul`
  (0–255 / 0–256 units), and `sincolor`/`sinhue`/`sinadd`/`sinmul` with
  their periods and negative-period direction. `shader` selects a custom
  shader (`none` clears it), `shaderparam` sets indexed parameters, and
  `tex1anim`/`tex1spr`/`tex2anim`/`tex2spr` select the shader's referenced
  textures by animation or sprite pair. All keys apply directly to the
  targeted layer states and persist until changed)
- `TransformSprite` (xshear/focallength/projection subset)
- `TransformClsn` (per-tick Clsn1/Clsn2 scale, angle, and pivot; rotations
  produce axis-aligned bounds, scale-only keeps the source box corner order)
- `OverrideClsn` (per-tick box replacement: `group` 0 clears all, -1 targets
  every group, 1–4 selects Clsn1/Clsn2/size/dummy; an all-zero `rect` deletes
  boxes, `index < 0` replaces every box, and an index past the end appends a
  new box; `ClsnVar` and hit detection see the result)
- `Depth`, `DepthBound`, `ShaderSet` (per-tick depth extents/flags and a
  named `[Shaders]` selection; see `docs/spec/depth.md`; GLSL shader
  execution is not modeled)
- `GroundLevelOffset` (per-tick ground plane offset: the next physics step
  lands on `value` instead of 0; positive values lower the floor, negative
  values raise it, and the screen-bound clamp follows the offset)
- `SaveState`, `LoadState`, `MatchRestart` (snapshot/restore the live
  simulation and reset to round 1; see `docs/spec/save.md`; disk
  `SaveFile`/`LoadFile` and DEF/PAL overrides are not modeled)
- `ChangeMovelist` (selects one of the character's `[Files] movelist`,
  `movelist0`, `movelistN` texts by `value`; an unknown id leaves the
  selection unchanged. Ikemen exposes this list to Lua AI scripts; the
  selection is stored here even though no Lua runtime is embedded)
- `DisplayToClipboard`/`AppendToClipboard`/`ClearClipboard` (debug output:
  formatted text is logged and stored in `SimState.debug_output`, which is
  cleared every tick and excluded from rollback; the OS clipboard is not
  touched)
- `Text`, `ModifyText`, `RemoveText` (screen-space text objects; see
  `docs/spec/text.md`; custom shaders/TTF/windows are not modeled)
- `AssertCommand` (keeps a command name active for `buffertime` ticks; an
  empty name selects the character's first command; `time` is an alias)
- `AssertInput` (merges `flag`/`flag2..8` bits — `U`/`D`/`L`/`R`,
  `a`–`c`, `x`–`z`, `s`, and facing-relative `B`/`F` — into the next input
  frame)
- `AssertAnalogVector` (sets `leftx`/`lefty`/`rightx`/`righty`/
  `lefttrigger`/`righttrigger` for `Analog(...)`; sticks clamp to -1..1 and
  triggers to 0..1)
- `ShiftInput` (per-tick key remap, applied to the next input frame:
  `input`/`output` accept `U`/`D`/`L`/`R`, `a`–`c`, `x`–`z`, `s`, and the
  Ikemen-only `d`/`w`/`m` buttons; `output = none` disables the source,
  `input = none, output = none` clears all mappings. `B`/`F` direction
  aliases are not accepted)
- `ParentMapSet`, `ParentMapAdd`, `RootMapSet`, `RootMapAdd`,
  `TeamMapSet`, `TeamMapAdd`
- `RoundTimeAdd`, `RoundTimeSet`, `VictoryQuote`, `ScoreAdd`,
  `TargetScoreAdd`, `RedLifeAdd`, `RedLifeSet`, `TargetRedLifeAdd`

### 4.3 Presentation and audio

- `PlaySnd`, `StopSnd`, `SndPan`
- `Explod`, `ModifyExplod`, `RemoveExplod`
- `MakeDust`, `GameMakeAnim`
- `AfterImage`, `AfterImageTime`
  `AfterImage` ghosts record the fighter's sprite frames and fade by `length`
  and `timegap`. The palette options are modeled: `palbright`/`palcontrast`
  set each ghost's base add/multiply, `paladd`/`palmul` step older ghosts,
  `palpostbright` applies after the first step, and `palcolor`, `palhue`,
  `palinvertall`, and `palinvertblend` drive the shader palette filter.
  Defaults are identity so omitted parameters keep the white ghost look.
- `PalFX`, `RemapPal`, `Trans`
  `RemapPal` selects a destination palette by `(group, index)` — SFF v2
  palette nodes or DEF `palN` files (`(1, N)`). Unknown destinations are
  ignored; `source = dest = -1, 0` clears the remap. Only sprites whose
  palette identity matches `source` are recolored; a `-1` source force-remaps
  every indexed sprite. The renderer restores originals when the remap
  changes or clears.
  `RemapSprite` substitutes `(group, index)` lookups for the fighter and its
  afterimages; `reset = 1` clears the map and `preset = "name"` applies a
  `[RemapPreset name]` table from the character DEF.
  `PalFX` models `add`/`sinadd`, `mul`/`sinmul` (0–256 units, 256 =
  identity), `alpha` (0–1), `color` (0–256 saturation), `hue` (−256..256,
  512 units per turn), `invertall`, `invertblend` (≥2 enables inversion),
  and the `sincolor`/`sinhue` oscillations with their negative-period
  direction and cycle lengths. Sine phases use the same tick cadence as
  `sinadd`/`sinmul`.
- `AllPalFX`, `BgPalFX` (global palette effects for character/effect sprites
  and stage layers; model `add`, `mul`, `alpha`, and `time` with the same
  units as `PalFX`, plus `color`, `hue`, `invertall`, `invertblend`, and the
  `sinadd`/`sinmul`/`sincolor`/`sinhue` oscillations with their negative-period
  direction and cycle lengths. Character/effect sprites receive `AllPalFX`;
  stage layers receive `BgPalFX`, and HUD/front-end quads are unaffected.)
- `ForceFeedback`

### 4.4 Helpers

- `Helper`, `BindToRoot`, `BindToTarget`, `BindToParent`
- `ParentVarSet`, `ParentVarAdd`, `RootVarSet`, `RootVarAdd`

Controllers not listed here are either deferred (see `COMPATIBILITY.md`) or
Ikemen-only extensions.

### 4.5 Controller parameters

- `triggerall`, `trigger1..triggerN`, `persistent`, `ignorehitpause` are common
  controller keys.
- `persistent = 1` runs every tick the trigger is true.
- `persistent = 0` runs once per state entry.
- `persistent = N > 1` runs once every N matching ticks.
- Negative `persistent` values run on every matching tick (Ikemen-GO).
- `ignorehitpause = 1` allows the controller to run during hit pause.
- Missing or malformed parameters use the documented defaults; unknown
  parameters are logged and ignored.

## 5. Triggers and expressions

### 5.1 Grammar

Expressions support:

- Integer/float literals, quoted strings
- Unary `!`, `-`, `+`, `~`
- Arithmetic `+ - * / %`
- Comparisons `= != < <= > >=`
- Logical `&& ||`
- Bitwise `& | ^ << >>`
- Ternary `cond ? a : b`
- Function calls and trigger names
- Parenthesized expressions

The deprecated old-style `TimeMod [op] divisor, value` trigger is rewritten to
`(Time % divisor) [op] value`; a zero divisor never matches. The modern
`Time % divisor` form is preferred.

### 5.2 Redirects

Supported redirect prefixes:

- `p2`, `p3`, `p4`
- `parent`, `root`
- `helper`, `target`, `partner`, `enemy`, `enemynear`
- `playerid`, `playerindex`, `helperindex`
- `player(n)` (1-based player slot: `player(1)` is P1, `player(2)` is P2;
  out-of-range slots fall back to the current fighter)
- `stateowner`

A redirect applies to the following trigger or expression term. Invalid or
missing redirect targets make the trigger false.

### 5.3 Required trigger families

- Time/animation: `Time`, `Anim`, `PrevAnim`, `AnimElem`, `AnimElemNo`,
  `AnimElemTime`, `AnimTime`, `AnimExist`, `SelfAnimExist`, `AnimLength`,
  `AnimLoopCount`, `AnimPlayerNo`, `SpritePlayerNo` (1-based owner of the
  animation/sprite bank; sprites and animations share one bank here),
  `InCustomAnim`
  `AnimExist` follows the animation bank owner (`AnimPlayerNo`), while
  `SelfAnimExist` always checks the fighter's own character animations.
- Palette: `DrawPal(group|index)` reports the active RemapPal destination or
  the current sprite's SFF palette key.
- Position/velocity: `Pos`, `Vel`, `Facing`, `ScreenPos`, `ScreenVel`,
  `SprPriority`. `Pos Z`/`Vel Z` report depth position and velocity; `Vel X`
  is facing-relative while `Vel Z` is not.
- State: `StateNo`, `PrevStateNo`, `StateType`, `PrevStateType`, `MoveType`,
  `PrevMoveType`, `Ctrl`, `Physics`, `SelfStateNoExist(n)`,
  `InCustomState`, `InRunState` (always false: the engine has no `RunState`
  trigger, so no code ever runs under it)
- Helpers: `IsHelper`, `HelperName` (helper display name; players read
  empty)
- Combat: `Alive`, `Life`, `LifeMax`, `Power`, `GetHitVar`, `HitOver`,
  `HitShakeOver`, `HitPauseTime`, `CanRecover`, `InGuardDist`, `MoveContact`,
  `MoveHit`, `MoveGuarded`, `MoveCountered`, `MoveHitVar(field)`, `HitCount`,
  `UniqHitCount`, `HitDefAttr`,
  `GuardBreak`, `GuardPoints`, `GuardPointsMax`, `Dizzy`, `DizzyPoints`,
  `DizzyPointsMax`, `RedLife`, `ComboCount`, `ReceivedHits`, `ReceivedDamage`,
  `PowerMax`, `Attack`, `AttackMul`, `Defence`, `DefenceMul`, `GuardCount`,
  `Angle`, `HitOverridden`, `ClsnOverlap(box, playerID, box)` (Clsn1/Clsn2),
  `ClsnVar(group, index, field)` (local Clsn1/Clsn2 box edge: `back`, `top`,
  `front`, `bottom`; affected by `TransformClsn`; the `size`/`dummy` groups
  and missing boxes read 0),
  `ReversalDefAttr = attr` (matches the active `ReversalDef` attr).
  `MoveCountered` is set when the current hit connected while the defender
  was attacking (`MoveType = A`) and is cleared by `MoveHitReset`.
  `MoveHitVar` reports the last unique target (`playerid`/`id`, 1-based),
  `playerno`, `uniqhit`, the applied `cornerpush.veloff`, the current-frame
  `frame` flag, gained `power`, and `sparkx`/`sparky` from the last connected
  hit. `overridden` is true on the tick the defender's `HitOverride`
  intercepted the hit.
- Commands: `Command` (reads the active state bank's command list),
  `SelfCommand` (always reads the fighter's own command list even when a
  borrowed state bank is active)
- Opponent: `P2Dist`, `P2BodyDist`, `P2Life`, `P2StateNo`, `P2StateType`,
  `P2MoveType`, `NumEnemy`, `NumTarget`, `NumPartner`, `NumPlayer`,
  `PlayerNo`, `PlayerIDExist(n)`, `PlayerIndexExist(n)`, `PlayerNoExist(n)`,
  `LastPlayerID`, `ID` (and the `PlayerID` alias). Unique IDs are 1-based
  and assigned in fighter creation order (P1, P2, then each helper), so
  `PlayerID(1)` addresses P1 and `ID` reports the current fighter's own ID.
  `P5Name`–`P8Name` read empty because versus play has at most two slots.
  For M.U.G.E.N-version characters `P2BodyDist Y` equals the signed axis
  difference reported by `P2Dist Y`; Ikemen-version characters use a body-gap
  approximation.
- Run order: `RunOrder` (1-based fighter vector position; P1, P2, then
  helpers in spawn order), `GroundAngle` (0 for the engine's flat stages),
  and `AirJumpCount` (built-in air jumps consumed since leaving the ground).
  Built-in air jumps use `movement.airjump.num`, `movement.airjump.height`,
  and `movement.airjump.neu.y` and enter state 45 on a fresh up press while
  airborne below the height. `XAngle`/`YAngle` are 0
  and `Scale.Z` is 1 because the renderer is 2D.
- Round: `RoundNo`, `RoundsExisted`, `RoundState`, `IntroState`, `OutroState`,
  `FightScreenState(field)`, `FightScreenVar(field)` (fight-screen motif
  `info.*` fields and the `round.*`/`time.framespercount` timings from
  `fight.def`, with the built-in defaults when no screenpack is configured),
  `GameVar(field)`, `MatchOver`, `Winner`,
  `Loser`, `Win`, `WinKO`, `WinTime`, `WinSpecial`, `WinHyper`, `WinPerfect`,
  `WinClutch`,
  `DecisiveRound`, `ConsecutiveWins`, `FightTime`, `FirstAttack`, `TeamSide`,
  `TeamMode`, `IsHomeTeam`, `TeamSize`, `TeamLeader`,
  `MemberNo`, `Standby`, `TimeRemaining`, `TimeElapsed`, `TimeTotal`,
  `StageTime`, `Score`, `ScoreTotal`
  `GameVar(hidebars)` mirrors the active `NoBarDisplay`/bar-hiding flags;
  the persist-* options remain false.
  `FirstAttack` is true for the team side that landed the first unguarded
  hit of the current round. Guarded contacts do not claim it, later hits
  from the other side do not steal it, and it resets at the start of each
  round (and on `restart_match`); it is part of the rollback checksum.
- Identity: `IsHost` is true for side 0's characters in a network session and
  false for local play and side 1, matching Ikemen's first-human-player rule;
  `GameMode` is `"versus"`, the engine's match type. `WinSpecial`/`WinHyper`
  are always false because special/hyper finishes are not classified.
  `GameOption(key)` exposes `Options.Time`, `Options.Difficulty`,
  `Options.Match.Wins`, `Options.Life`, `Config.RoundTime`, and
  `Config.RoundsWinToMatch` from the active match options; there is no
  `mugen.cfg`, so unknown keys read false like Ikemen's `GetValue` error path.
  `SoundVar(channel, field)`
  reads the sim-side sound-channel registry populated by `PlaySnd channel`:
  `group`, `number`, `freqmul`, `isplaying`, `length` (sample frames),
  `pan`, `position`, `priority`, `samplerate`, and `volumescale`;
  `loopcount`/`loopstart`/`loopend`/`startposition` read 0 because loop
  metadata is not parsed. A negative channel selects the first playing
  channel, and an unregistered channel reports `group`/`number` as -1,
  `freqmul` as 1, and everything else as 0/false (Ikemen semantics).
- Screenpack: `MotifVar(info.name|info.author|info.localcoord.x|info.localcoord.y)`
  reads `system.def`; other keys read `0`. `MotifState(field)` is false for
  every field because no front-end motif screen is active during a fight.
- Utility: `MugenVersion(major|minor)`,
  `IkemenVersion(major|minor|patch)`, `IsAsserted(flag)`, `Map(key)`,
  `HitDefVar(key)` (damage/flags/sounds/timers/facing/statenos plus
  `yaccel`/`zaccel`, ground/air/down/guard/airguard velocity `.x`/`.y`/`.z`,
  fall x/y/z velocities, and cornerpush offsets), `SpriteVar(group|image|width|height|xoffset|yoffset)`,
  `AnimElemVar(field)` (current animation-frame fields: `alphadest`,
  `alphasource`, `angle`, `group`, `hflip`, `vflip`, `image`, `time`,
  `xoffset`, `yoffset`, `xscale`, `yscale`, `numclsn1`, `numclsn2`),
  `EnvShakeVar(time|freq|ampl|dir|phase|diradd|decay)`,
  `PalfxVar(time|add.r|add.g|add.b|mul.r|mul.g|mul.b)` (reads the effective
  local `PalFX`; `add` is 0–255, `mul` is 0–256 with 256 as identity, and
  `color`/`hue`/`invert*` plus the global `bg`/`all` variants are
  unmodeled),
  `Scale.X`, `Scale.Y` (1 unless `AngleDraw` is active), `Offset.X`,
  `Offset.Y` (current draw offset), `XShear` (current `TransformSprite`
  shear), `Alpha.Source`, `Alpha.Dest` (raw trans alpha bytes, 0–255),
  `InputTime(key)` (consecutive held ticks, or negative consecutive
  released ticks; `B`/`F` are facing-relative while `U`/`D`/`L`/`R` are
  absolute; buttons use `a`–`c`, `x`–`z`, `s`; the Ikemen-only
  `d`/`w`/`m` buttons are unmodeled and read 0),
  `TopBoundDist`, `TopBoundBodyDist`, `BotBoundDist`, `BotBoundBodyDist`
  (0: the 2D renderer has no stage depth bounds),
  `StageBGVar(id, index, field)`, `StageVar(key)` (stage info strings,
  live camera bounds/follow/tension/cut/zoom values, and
  `playerinfo.leftbound`/`rightbound`; other keys are unmodeled and read
  as `0`), `Name`, `DisplayName`, `AuthorName`,
  `NumStageBG(id)` (`id < 0` counts every layer), `Const`,
  `StageConst(key)` (stage DEF `[Constants]` values; numeric values read as
  numbers, other values as strings, missing keys as `0`),
  `BgmVar(field)` (live `PlayBGM`/`ModifyBGM` state when active, otherwise
  stage BGM: `filename`, `volume`, `loop`, `loopstart`, `loopend`, `freqmul`,
  `loopcount`, `startposition`, and live `position`/`length`/`samplerate`
  fed by the audio engine),
  `Const240p`, `Const480p`,
  `Const720p`, `Const1080p`, `Random`, `IfElse`, `Cond`, `Clamp`, `Lerp`, `Sign`,
  `Abs`, `Ceil`, `Floor`, `Min`, `Max`, `Sin`, `Cos`, `Tan`, `Asin`,
  `Acos`, `Atan`, `Atan2`, `Rad`, `Deg`, `Round`, `RandomRange`, `Exp`,
  `Ln`, `Log(base, value)`, `Sqrt`, `Pow`,
  `AILevel`, `AILevelF`, `LocalCoordX`, `LocalCoordY`, `LocalCoord(x|y)`,
  `Analog(axis)` (reads the axes set by `AssertAnalogVector`; 0 when not
  asserted),
  `Pi`, `E`, `Float(x)`
  Trigonometric functions take and return radians, as in the M.U.G.E.N
  expression reference.
- Bounds/edges: `LeftEdge`, `RightEdge`, `TopEdge`, `BottomEdge`,
  `BackEdge`, `FrontEdge`, `BackEdgeBodyDist`, `FrontEdgeBodyDist`,
  `BackEdgeDist`, `FrontEdgeDist`, `StageFrontEdgeDist`,
  `StageBackEdgeDist` (`StageBackEdge` deprecated alias)
- Projectiles/explods/helpers: `NumProj`, `NumProjID`, `ProjVar(id, index, field)`,
  `ProjClsnOverlap(index, targetID, box)` (0-based index among the fighter's
  projectiles after the first matching projectile ID; 1-based unique target
  ID; `clsn1`/`clsn2`; a projectile's Clsn1 or Clsn2 group may overlap, and
  the unmodeled `size` group is false),
  `HelperVar(id|helpertype|keyctrl)`, `HelperIndexExist(n)`, `Index`,
  `ExplodVar(id, index, field)` (`pos.z`, `vel.z`, and `accel.z` report
  depth position, velocity, and acceleration), `ProjCancelTime(id)`,
  `NumText(id)`, `Shader()`, `ProjHit`, `ProjContact`, `ProjGuarded`,
  `ProjMiss`/`ProjMissTime` (ticked after a projectile expires without
  hitting),
  `NumExplod`, `NumHelper`, `IsHelper`, `ParentDist`, `RootDist`,
  `ParentExist`. `ProjVar` `pos.z`/`vel.z`/`accel.z` report projectile depth
  position, velocity, and acceleration.
- Variables: `Var`, `FVar`, `SysVar`, `SysFVar`

### 5.4 Constants

`Const(name)` resolves the `[Data]`, `[Size]`, `[Velocity]`, and `[Movement]`
constants. Comma-separated values expand to `.x`/`.y`; scalar velocity values
also expand. Unknown constants evaluate to 0 and log once.

### 5.5 Helper relation triggers

Public M.U.G.E.N trigger documentation defines the helper relation family as
follows; Ikemen-GO content uses the same forms.

- `IsHelper` is 1 for a helper and 0 otherwise. `IsHelper(n)` additionally
  requires the helper's `id` to equal `n`.
- `ParentDist X|Y` is the distance from a helper to its parent. `X` is
  `(parent.x - self.x) * self.facing`, so a parent in front of the helper is
  positive; `Y` is `parent.y - self.y`, so a parent above the helper is
  negative.
- `RootDist X|Y` has the same axes relative to the helper's root (the owning
  player, possibly several helper generations up).
- A player with no parent/root, a destroyed or KO'd relation, or a missing
  component evaluates to 0. (M.U.G.E.N specifies "bottom" for the missing
  relation case; 0 keeps the common `!= 0` comparisons inert and matches the
  existing engine scalar model.)

### 5.6 Team triggers

Public M.U.G.E.N trigger documentation defines the team family as follows:

- `TeamMode = single|simul|turns` compares the player's team mode. The engine
  simulates single play only, so `single` is true and `simul`/`turns` are
  false for both sides. `TeamMode` without a comparison yields the string
  `"single"`; `!=` inverts the comparison.
- `IsHomeTeam` is 1 for the home team. In versus, P1's side (left) is home and
  P2's side is not. Helpers inherit their owner's side and therefore its
  home-team status.
- Arcade mode's rule that the CPU is always the home team is not
  representable: the simulator does not track which slot is CPU-controlled.
  The engine applies the versus rule in every match and documents the
  limitation rather than guessing.
- Unknown team-mode strings compare false.

### 5.7 AI difficulty and parent presence

- `AILevel` returns the difficulty of the AI driving the evaluating
  character's slot, and 0 when AI is disabled for that slot. The level is
  stored per fighter: a versus or arcade match can have one human slot at 0
  and one CPU slot in the 1..8 range. Helpers inherit their owner's level.
- `ParentExist` returns 1 when the evaluating character is a helper with a
  parent, and 0 for root players or when no parent relation is recorded.
  M.U.G.E.N 1.1's trigger reference does not list `ParentExist`; this engine
  defines it consistently with the helper relation family and its helper
  model.
- Opponent lookup uses the fighter's team side rather than the raw owner
  index, so a nested helper (a helper owned by another helper) still resolves
  the opposing player. Using the owner index directly underflows for nested
  helpers.

## 6. Commands

### 6.1 Command section

```
[Command]
name = "QCF_x"
command = ~D, DF, F, x
time = 15
buffer.time = 1
```

- `time` is the command window in ticks; default 15.
- `buffer.time` is how long a matched command remains active; default 1.
- `[Defaults]` entries `command.time`, `command.steptime`, and
  `command.buffer.time` override the defaults for all commands.

M.U.G.E.N-version characters (no `ikemenversion` above zero) get Ikemen-GO's
backward-compatibility quirks (`ApplyBackwardCompatibility`):

- In a non-OR step, a `/` on any key propagates to every direction and button
  in that step (`/B+a` behaves like `/B+/a`).
- A command whose every step is hold-only (`/`) keeps `buffer.time = 1`
  regardless of the declared value. This stops held commands from remaining
  active for the full buffer after release.

Ikemen-version characters keep the strict per-key prefix parsing and their
declared buffers.

### 6.2 Command symbols

Directions: `U`, `D`, `B`, `F`, `L`, `R`, `N`, `UB`, `UF`, `DB`, `DF`,
`UL`, `UR`, `DL`, `DR`.
Buttons: `a`, `b`, `c`, `x`, `y`, `z`, `s`.
Combinations: `x+y` (all required) and `a|b` (either).
Prefixes:

- `$`: pooled four-way direction tap
- `*`: exact-direction tap
- `/`: hold (optional charge digits, e.g. `/12D`); applies to a direction or
  button (`/x` matches while held)
- `~`: release (optional charge digits), for directions or buttons
- `>`: greater-direction expansion

Commands are matched facing-relative. Absolute input directions are converted
to forward/back using the character's facing.

### 6.3 Matching

- The most recent input is checked first.
- A command matches when every step is satisfied in order within `time`.
- A direction step without a prefix matches a press or hold.
- `$` requires a tap; `/` requires a sustained hold; `~` requires release.
- Button steps match the press edge by default; `/x` matches while held and
  `~x` matches the release edge.
- A matched command sets its buffer timer to `buffer.time`.
- `Command = "name"` is true while the buffer timer is active.
- AI input uses the same matcher; no special-case command table.

### 6.4 Built-in hold commands

`holdfwd`, `holdback`, `holdup`, and `holddown` are built-in pseudo-commands
used by the stock common state file. They never require a matching `[Command]`
definition:

- `Command = "holdfwd"` is true while the latest held direction includes the
  facing-relative forward component.
- `Command = "holdback"` uses the facing-relative back component.
- `Command = "holdup"`/`"holddown"` use the vertical component.
- Diagonals satisfy both component holds (`UF` sets forward and up; `DB` sets
  back and down).
- Facing -1 swaps forward and back; up and down are unchanged.
- A neutral direction makes all four false; the negated form
  `Command != "holdfwd"` is true whenever forward is not held.

## 7. State execution

Each tick, for each fighter:

1. Run special states `-3`, `-2`, `-1` in order.
2. Run built-in movement transitions when `ctrl` is true.
3. Integrate physics and position.
4. Run the current state's controllers in file order.
5. A `ChangeState` continues from the new state in the same tick, capped at 8
   transitions.
6. Advance animation after controllers.

`[Statedef -1]` is the command-driven state: its controllers are the character's
move list and run before built-in movement. Special states always come from
the fighter's own character, even while a positive custom state borrows another
player's bank. While borrowed state data is active (throw/custom state), `-3`
and `-1` are skipped but `-2` still runs.

## 8. Error handling

- Malformed sections are skipped with a warning.
- Missing state references log once and leave the fighter in place.
- Invalid trigger expressions evaluate to false.
- Unknown controllers are logged and ignored.
- Out-of-range variable indices are clamped.
- Do not panic on malformed text or recursive state changes.

## 9. Test plan

- Parse/merge `stcommon`, `cns`, `st`/`st0`..`stN`, CMD.
- Statedef defaults and `U` semantics.
- Controller execution order, persistent, ignorehitpause.
- ChangeState same-tick continuation and 8-transition cap.
- Command parsing for QCF/DP, button combos, `$`/`/`/`~`.
- Facing-relative command matching.
- Trigger expression precedence, redirects, constants.
- Command buffer and AI input.
- Bundled roster and stock KFM/Ryu move lists.
- Malformed/truncated files and missing states.

## 10. Sources

- Ikemen-GO `src/compiler.go`, `src/compiler_functions.go`, `src/input.go`,
  `src/char.go`, `src/bytecode.go` (MIT).
- Public M.U.G.E.N CNS/CMD documentation.
- Observed character state/command files.
