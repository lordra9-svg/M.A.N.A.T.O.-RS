# CNS/CMD specification

**Status: DRAFT — awaiting approval.** Derived from the MIT-licensed Ikemen-GO
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
3. `st`
4. `st1` … `st9`
5. CMD file

Later files override earlier definitions for the same state number. Controllers
from the CMD file attach to its `[Statedef -1]` (or create a synthetic `-1`).

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
```

Defaults:

- `type = S`
- `movetype = I`
- `physics = N`
- `ctrl` unchanged
- `anim` unchanged
- `velset` unchanged
- `poweradd = 0`
- `juggle = 0`
- `sprpriority = 0`
- `facep2 = 0`

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
- `CtrlSet`, `StateTypeSet`, `MoveTypeSet`, `Turn`
- `VarSet`, `VarAdd`, `VarRangeSet`, `VarRandom`
- `PowerAdd`, `PowerSet`, `LifeAdd`, `LifeSet`
- `DefenceMulSet`, `AttackMulSet`
- `SprPriority`, `Width`, `ScreenBound`, `PlayerPush`
- `AssertSpecial`

### 4.2 Combat

- `HitDef`, `ReversalDef`
- `Projectile`, `ModifyProjectile`
- `NotHitBy`, `HitBy`, `HitOverride`
- `HitVelSet`, `HitFallSet`, `HitFallVel`, `HitFallDamage`
- `TargetBind`, `TargetState`, `TargetFacing`, `TargetLifeAdd`,
  `TargetPowerAdd`, `TargetVelSet`, `TargetVelAdd`
- `Pause`, `SuperPause`
- `EnvShake`, `FallEnvShake`

### 4.3 Presentation and audio

- `PlaySnd`, `StopSnd`, `SndPan`
- `Explod`, `ModifyExplod`, `RemoveExplod`
- `MakeDust`, `GameMakeAnim`
- `AfterImage`, `AfterImageTime`
- `PalFX`, `RemapPal`, `Trans`
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

### 5.2 Redirects

Supported redirect prefixes:

- `p2`, `p3`, `p4`
- `parent`, `root`
- `helper`, `target`, `partner`, `enemy`, `enemynear`
- `playerid`, `playerindex`, `helperindex`
- `stateowner`

A redirect applies to the following trigger or expression term. Invalid or
missing redirect targets make the trigger false.

### 5.3 Required trigger families

- Time/animation: `Time`, `Anim`, `AnimElem`, `AnimElemTime`, `AnimTime`,
  `AnimExist`, `SelfAnimExist`
- Position/velocity: `Pos`, `Vel`, `Facing`, `ScreenPos`, `ScreenVel`
- State: `StateNo`, `PrevStateNo`, `StateType`, `MoveType`, `Ctrl`, `Physics`
- Combat: `Alive`, `Life`, `LifeMax`, `Power`, `GetHitVar`, `HitOver`,
  `HitShakeOver`, `CanRecover`, `InGuardDist`, `MoveContact`, `MoveHit`,
  `MoveGuarded`, `HitCount`, `UniqHitCount`, `HitDefAttr`
- Commands: `Command`
- Opponent: `P2Dist`, `P2BodyDist`, `P2Life`, `P2StateNo`, `P2StateType`,
  `P2MoveType`, `NumEnemy`, `NumTarget`, `NumPartner`
- Round: `RoundNo`, `RoundsExisted`, `RoundState`, `MatchOver`, `Winner`,
  `Loser`, `TeamSide`
- Utility: `Const`, `Const240p`, `Const480p`, `Const720p`, `Random`, `IfElse`,
  `Abs`, `Ceil`, `Floor`, `Min`, `Max`, `Sin`, `Cos`, `Tan`, `Exp`, `Log`,
  `Sqrt`, `Pow`
- Bounds/edges: `LeftEdge`, `RightEdge`, `TopEdge`, `BottomEdge`,
  `BackEdge`, `FrontEdge`, `BackEdgeBodyDist`, `FrontEdgeBodyDist`,
  `BackEdgeDist`, `FrontEdgeDist`
- Projectiles/explods/helpers: `NumProj`, `NumProjID`, `ProjHit`,
  `ProjContact`, `ProjGuarded`, `NumExplod`, `NumHelper`
- Variables: `Var`, `FVar`, `SysVar`, `SysFVar`

### 5.4 Constants

`Const(name)` resolves the `[Data]`, `[Size]`, `[Velocity]`, and `[Movement]`
constants. Comma-separated values expand to `.x`/`.y`; scalar velocity values
also expand. Unknown constants evaluate to 0 and log once.

## 6. Commands

### 6.1 Command section

```
[Command]
name = "QCF_x"
command = ~D, DF, F, x
time = 15
buffer.time = 1
```

- `time` is the command window in ticks.
- `buffer.time` is how long a matched command remains active.
- Missing `time` defaults to 1; missing `buffer.time` defaults to 1.

### 6.2 Command symbols

Directions: `U`, `D`, `B`, `F`, `UB`, `UF`, `DB`, `DF`.
Buttons: `a`, `b`, `c`, `x`, `y`, `z`, `s`.
Combinations: `x+y`, `a+b+c`.

Prefixes:

- `$`: direction tap (pressed and released within the window)
- `/`: direction hold (charge)
- `~`: release
- `>` and `^`: Ikemen-only variants; preserve if parsed, otherwise defer

Commands are matched facing-relative. Absolute input directions are converted
to forward/back using the character's facing.

### 6.3 Matching

- The most recent input is checked first.
- A command matches when every step is satisfied in order within `time`.
- A direction step without a prefix matches a press or hold.
- `$` requires a tap; `/` requires a sustained hold; `~` requires release.
- Button steps match the press edge, not the held state.
- A matched command sets its buffer timer to `buffer.time`.
- `Command = "name"` is true while the buffer timer is active.
- AI input uses the same matcher; no special-case command table.

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
move list and run before built-in movement.

## 8. Error handling

- Malformed sections are skipped with a warning.
- Missing state references log once and leave the fighter in place.
- Invalid trigger expressions evaluate to false.
- Unknown controllers are logged and ignored.
- Out-of-range variable indices are clamped.
- Do not panic on malformed text or recursive state changes.

## 9. Test plan

- Parse/merge `stcommon`, `cns`, `st`, `st1`..`st9`, CMD.
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
