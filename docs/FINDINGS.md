# Consolidated engine findings

This document collects the behavior learned while building M.A.N.A.T.O.-RS.
It is the narrative companion to the machine-checkable table in
[`oracle/assertions.md`](oracle/assertions.md) and the status matrix in
[`../COMPATIBILITY.md`](../COMPATIBILITY.md). Read this document for the
"what did we learn and why does it matter" view; read `assertions.md` for the
exact inputs and observed values.

## 1. How these findings were obtained

| Source | Used for | Confidence |
| --- | --- | --- |
| Running Ikemen-GO (pinned `955a2f2`) with scripted input and Lua debug output | Combat, round, projectile, pause, stage-render observations | High; re-runnable |
| MIT-licensed Ikemen-GO source and documentation | Format semantics, trigger/controller contracts, parser rules | High; source-cited |
| Public M.U.G.E.N format documentation and stock content as input data | DEF/CNS/AIR/SFF/SND structure, stock compatibility | Medium; content-driven |
| Engine unit/integration tests | Determinism, serialization, edge cases, hardware-independent behavior | High for the tested contract |

There is **no M.U.G.E.N binary oracle** in this project. X-07 forbids reading
or disassembling M.U.G.E.N, so M.U.G.E.N-specific claims come from documented
formats, stock content behavior, and compatibility tests, not from observing
the binary. Ikemen-GO is the only runtime oracle.

Confidence labels used below:

- **Oracle** — observed in Ikemen-GO and encoded in a Rust test (`O-xx`).
- **Source** — read from MIT-licensed Ikemen-GO source/docs.
- **Content** — required by stock content or the file format.
- **Residual** — a known small divergence or an unexecuted check.

## 2. Content formats

### Text and DEF parsing

- Files are decoded tolerantly: BOM, UTF-16, comments, quoted values, and
  Windows `\` separators are accepted. Parsing is case-insensitive.
- Duplicate sections are preserved in order. This is required for repeated
  `[BG ...]` blocks and repeated state sections; a map keyed by section name
  would silently lose stage layers.
- Character state files merge in M.U.G.E.N order: `stcommon`, `cns`, `st`,
  `st0`, `st1`, … `stN` in **natural numeric order** (`st10` after `st9`), then
  the CMD. Later files override earlier constants/states. Characters that put
  configuration in `st1` depend on this.
- `[Files]` paths are relative to the DEF's directory unless the engine has an
  explicit root override. Palette maps, `st`/`st0`..`stN`, and stage metadata
  all follow the same resolution rule.

### SFF sprites

- **SFF v1** embeds PCX subfiles with inline palettes. Palette reuse and
  zero-length EOF link chains occur in stock files; declared payload lengths
  exclude alignment padding.
- **SFF v2** uses link chains, palette links, optional tdata, and raw/RLE8/
  RLE5/LZ5/PNG sprite encodings. Indexed formats and PNG sprites carry a
  4-byte uncompressed-size prefix.
- PNG8 sprites frequently contain a dummy `PLTE` chunk. The real colors come
  from the SFF palette node, not the PNG's own palette; using the embedded
  `PLTE` produces wrong colors.
- **Correction (X-05e):** SFF v2 palette nodes store **RGBA** bytes, not BGRA.
  Reading them as BGRA tinted the stock stage red-brown and the KFM wall blue,
  the opposite of Ikemen. `parse_v2_palettes` now keeps RGBA; the stage-render
  oracle fails under the old order.
- Masked sprites must discard transparent texels when building the atlas; if
  they are written as opaque black they bleed into neighboring frames.
- Missing palettes and truncated PNGs have defined fallbacks rather than
  panics, because user content is often malformed.

### Animation (AIR)

- Frames carry sprite group/index, offsets, duration, flip, blend mode, and
  collision boxes.
- Interpolation markers and `SubAdd`/`AS`/`SS` forms are parsed. Offset, scale,
  angle, and blend-alpha interpolation is applied to fighters, projectiles,
  explods, and stage background animations.
- Finite animations get an extra tick at `total_time` so `AnimTime = 0` fires
  before the animation wraps. Duration sums use saturating arithmetic because
  stock animations can contain `-1` ("forever") frames.

### Sound (SND)

- SND entries are WAV payloads; the parser converts unsigned 8-bit and signed
  16/24/32-bit PCM plus float samples into a common PCM representation with
  sample rate and channel count.
- Header offsets are validated; truncated entries are skipped instead of
  failing the whole bank.
- Group `-1` means "silent" and must not play. `F`-prefixed common sounds come
  from the shared `fight.snd` bank, character sounds from the character's own
  bank.

### Stages and screenpacks

- Stage layers honor `start`, `delta` (parallax), `tile`, `tileSpacing`, and
  negative/positive layering behind or in front of fighters.
- `BGCtrl` supports anim, visible, enable, position, velocity, sine/cosine,
  palette effects, and shader sets, with `time` windows and loops.
  `ModifyBGCtrl` retimes or overrides existing controllers.
- Screenpack `.fnt` v1/v2 fonts, `system.def`, `fight.def`, and `select.def`
  drive the title/select/fight screens. Screenpack life/power bars, faces,
  names, timer, win icons, combo counters, round announcements, and per-action
  banners replace the built-in HUD.
- Storyboards and static glTF models are implemented beyond the original
  M.U.G.E.N format; see `docs/spec/storyboard.md` and `docs/spec/model.md`.

## 3. State machine and triggers

- A new state applies defaults `type = S`, `movetype = I`, `physics = N`
  unless the Statedef overrides them. An explicit `U` means "keep the previous
  value". This matters after hitstun: without the idle default the CPU keeps
  believing it is attacking.
- `ChangeState` continues from the new state in the same tick, capped at 8
  transitions to prevent infinite loops.
- Special states `-3`, `-2`, `-1` run in that order before normal gameplay.
  Borrowed states follow own-bank lookup rules; helper `keyctrl`/`helpertype`
  gate input.
- `persistent` timing: `0`/`1` runs on the first tick, `N` every N ticks, and
  a negative value every tick. `ignorehitpause` lets a controller run during
  hit pause.
- `anim_time` is total elapsed ticks; `AnimTime = total_time - anim_time`, and
  the extra tick at `total_time` is intentional (see AIR above).
- `HitPauseTime` reports the **attacker's own** remaining self-pause. KFM state
  400 (`pausetime = 10, 11`) reports 10 on the contact frame, then 9…0 on
  successive frozen ticks, while the defender reports 0 throughout (O-29).
- State/trigger semantics for M.U.G.E.N and Ikemen differ in a few places.
  For example, `P2BodyDist Y` uses the M.U.G.E.N signed-axis convention, while
  Ikemen-only triggers expose additional state that the 2D engine reports as
  fixed values (`XAngle`/`YAngle` = 0, `Scale.Z` = 1, bare `TimeMod = 0`).

## 4. Combat

### Basic hit flow

Using KFM state 200 (light punch) against an idle opponent (O-09..O-12,
O-32, O-35):

| Observation | Value |
| --- | --- |
| Contact after state entry | +4 ticks |
| Damage | 23 |
| Defender reaction | state 5000 |
| Reaction exit | 5001 on the first tick after the 8-tick hit shake (+9 from contact) |
| Push | 15.899 px (our engine 14.332, within the documented ~2 px residual) |
| Power | `PowerAdd` 10 before contact; +16 attacker / +13 defender on hit |

One activation hits once; it does not re-hit after its hit pause.

Guarding (O-13, O-14):

- Forced guard puts the defender in state 150 with animation 140.
- KFM's `damage = 23, 0` deals **0 guard damage** and reports
  `MoveGuarded = 1`, `MoveHit = 0`.
- The guard slide starts 9 ticks after contact (state 151) and pushes ~14.8 px
  over the 14-tick guard window (our engine 16.227, same residual).

Trips and launches (O-15, O-16):

- KFM state 440 contacts around +10, deals 72, and enters trip/fall state 5070.
- 5070 advances to 5071 at contact +13, then the defender becomes airborne with
  upward velocity and gravity.

### Juggle system

- The juggle budget is **15 points**.
- KFM state 440 has `juggle = 7`; it consumes 7 even though the first hit is
  grounded, leaving 8.
- KFM state 210 has `juggle = 4`; an airborne follow-up consumes 4 and enters
  air-hit state 5020.
- Fall defence multiplies damage by **1.5** for that airborne hit (72 → 38 in
  the capture).
- A fresh activation is not blocked by the points gate even when its cost
  exceeds the remaining points. The gate blocks **repeat hits within one
  activation**, which is why M.U.G.E.N lets a new move juggle an enemy it has
  not targeted yet.
- **Negative result (O-20):** a purpose-built persistent `numhits = 2` HitDef
  did **not** stop repeat hits after the budget reached 0. The first fall hit
  consumed all 15 points, then repeat contacts continued until KO (100 hits in
  the capture). The engine keeps the same-activation gate unit-tested but does
  not change behavior to match this anomalous construct.

### Pause and super pause

- `Pause time = 10, movetime = 0` freezes the opponent's animation for 10
  ticks, then animation resumes (O-38).
- `SuperPause time = 15, movetime = 0` freezes through the pause window (O-39).
- Ikemen recomputes the environment-shake offset from elapsed time each frame,
  so `mul` applies once per full `freq` cycle, not once per tick.

### Throws, helpers, and projectiles

- **Throw (O-30):** Kaen state 800 grabs an idle KFM at +5 ticks; P1 enters
  810, P2 enters 820, and the target binds at `p1.x + 28`, `p1.y - 40`. At +25
  `TargetLifeAdd` deals 80; at +27 the target is released into state 5050
  while still at the bound position.
- **Helper (O-31):** Kaen state 2100 spawns helper 2100 in state 2101. Its
  HitDef contacts +4 ticks after injection for 30 damage into state 5000. The
  helper self-destroys at time 20.
- **Projectiles (O-21..O-28):** a plain projectile contacts an idle target at
  +6 and a guarding target at +7. Damage/guard damage by character:

  | Source | Hit | Guard |
  | --- | --- | --- |
  | Kaen state 1000 | 55 | 12 |
  | Nival projectile | 45 | 10 |
  | Ordan projectile | 60 | 14 |
  | Kaen state 3000 (low blast) | 65 | 15 |
  | Nival state 3000 (after 30-tick pause) | 90 | 20 |

- **Correction (O-25):** Kaen's state 3000 originally used `pos` and
  `removetime` as Projectile parameters. Ikemen warns about and ignores invalid
  Projectile parameter names, so both blasts spawned at y=0 and the capture
  showed 120 combined damage. The content now uses `offset` and
  `projremovetime`; Ikemen and our engine both report 65, and the upper blast
  (`offset = 0,-58`, `velocity = 6,-2`) passes above the idle hurt box.
- Projectile lifetime/collision findings: `NumProj(ID)` counts only active
  projectiles; projectile clashes resolve through `projcancelanim`/
  `ProjCancelTime`; `projpriority` defaults to 1 and `projanim` to 0.
- `NumExplod` counts explicit `Explod`/`GameMakeAnim`, **not** `MakeDust` or
  hit sparks. GameMakeAnim first appears one tick after the controller state is
  entered (O-37). Our engine spawns the effect immediately, a documented
  one-tick residual.

### Guard points, guard crush, dizzy

- Guard damage for the guard-point meter is **1.5 × raw life damage**
  (`default.lifetoguardpointsmul`), not the HitDef's `guard.damage` value.
  With 20 guard points and 10 raw damage per hit: 20 → 5 → 0 (O-33).
- Reaching 0 starts a 20-tick guard-crush window with no guarding. After a
  60-tick cooldown the meter regains +2 on the first regen tick.
- Dizzy points use **1.8 × raw life damage** (`default.lifetodizzypointsmul`).
  With 100 dizzy points and 20 damage per hit: 100 → 64 → 28 → 0 (O-34).
- A 60-tick dizzy stun then resets the meter to its maximum.
- Custom guard-break/dizzy states come from `StateGuardBreakHit`,
  `StateGuardBreakRecover`, and `StateDizzy`.

## 5. Round flow

- A round timer lasts **60 ticks per configured second** (`-time 5` → 300).
- The match dump's `Timer` is **elapsed ticks at round end**, not remaining
  ticks. A one-tick KO reports 1; a full 60-tick time-over reports 60.
- A KO win at full life is perfect, not time-over. A time-over win at full life
  is both time-over and perfect.
- A clutch win is `life <= lifeMax * 0.10`, inclusive. 100/1000 is clutch;
  101/1000 is not. Perfect and clutch are mutually exclusive.
- `WinSide` is a **zero-based team index**; the `Wins` table is one-based.
- Equal life at time over is a draw: no fighter has `Win = true`,
  `WinSide = -1`, and `Draws = 1`. The oracle then replays another round even
  when `-rounds 1` was requested.
- The harness flag `-rounds` means **total rounds to play**, not "rounds needed
  to win". Use `-rounds 1` and inspect the per-round table.

## 6. Rendering

- The classic target is 320x240; scene rendering is nearest-neighbour and then
  letterboxed to the window. Headless screenshots read the same texture, so
  headless and windowed output match.
- The close oracle stage starts with camera `x = 0`, `y = 0`, `zoom = 1`
  (O-36). Camera tracking, tension, bounds, and zoom are covered by unit tests;
  the oracle only establishes the baseline.
- Stock-stage render oracle (X-05a..X-05d), stage-band mean RGB at 320x240,
  AI 0, 120 fight ticks:

  | Stage | Ikemen | Ours |
  | --- | --- | --- |
  | stock KFM stage | (113, 92, 65) | (105, 82, 52) |
  | stage0 | (16, 44, 152) | (17, 44, 153) |
  | stage0-720 | (34, 61, 159) | (8, 34, 134) |
  | stage1 | (93, 125, 169) | (93, 125, 168) |

  The stage0-720 difference is layer/camera composition, not palette. Only
  derived means are committed; raw PNGs stay in a temp directory.
- Afterimages honor `timegap`, Ikemen's `framegap` (animation-frame delta),
  `length` decay, `time = 0` disable / negative forever, alpha/blend, palette
  add/mul/step/post/color/hue/invert, and `RemapSprite` interaction.
- Ikemen-only extensions implemented here include custom shaders, zoom/camera
  controls, depth/Z, glTF scene models, and storyboard Lua.

## 7. Audio

- Audio is event-driven: the simulation emits `SoundEvent { source, group,
  index, volume, pan }`; the audio crate resolves the bank, resamples to the
  device rate, and mixes with equal-power panning.
- Character `PlaySnd` and HitDef `hitsound`/`guardsound` resolve against the
  attacker's bank; `F`-prefixed common sounds use the shared `fight.snd` bank.
  Sounds are panned by on-screen position.
- **No hardware playback oracle exists.** A-01 is verified with `OfflineMixer`
  tests that render decoded samples and cover playback, pan, volume,
  resampling, priority/lowpriority, looping, and stop without a device.
  Real-device latency and device-loss behavior remain hardware checks.

## 8. Netplay and determinism

- The simulation is fixed at 60 Hz and uses a seeded RNG inside `SimState`.
  Assets are immutable and shared through `Arc`; snapshots copy only mutable
  state, and snapshots are excluded from the rollback checksum.
- GGRS 0.13 is the rollback session manager (`ggrs::Config` with
  `Input = PlayerInput`, `State = SimState`), using two frames of input delay.
  The engine supplies the deterministic simulation and snapshot boundary; GGRS
  supplies rollback scheduling, prediction, and the transport trait.
- GGRS `SyncTest` runs the real game through save/load/rollback cycles. The
  recorded check is `--synctest 300`: checksum `cabb7db7f2d864f1`, life
  1000/940.
- A deterministic UDP proxy test inserts **12–30 ms delay/jitter and 1-in-17
  packet loss** around a two-peer session. Both peers converge at **90
  confirmed frames** with matching checksums.
- That test is not an internet matrix. Real router/NAT/firewall behavior,
  Wi-Fi loss patterns, and long-haul latency remain unexecuted.
- Spectator snapshots are chunked at 1 KiB with a 256 KiB cap and are rejected
  on inconsistent totals, duplicate indices, stale frames, oversized chunks,
  or a checksum mismatch after restore.

## 9. Corrections and negative findings

These are the places where the first assumption was wrong. They are the most
valuable findings because they prevent repeating the mistake.

| Finding | What was wrong | Correction |
| --- | --- | --- |
| SFF v2 palette order (X-05e) | Palettes were read as BGRA | Palettes are RGBA; the stage oracle fails under BGRA |
| Kaen super damage (O-25) | A 120-damage capture looked like real behavior | The content used invalid `pos`/`removetime` Projectile parameters; with `offset`/`projremovetime` the low blast hits for 65 and the upper one passes overhead |
| Juggle gate (O-19, O-20) | Assumed the points gate blocks any activation with insufficient points | A fresh activation bypasses the gate; only repeat hits in one activation are blocked. A persistent `numhits = 2` construct still repeats until KO — kept as a negative finding |
| `NumExplod` (O-37) | Assumed dust and hit sparks count as explods | Only explicit `Explod`/`GameMakeAnim` count; MakeDust does not |
| Guard points (O-33) | Assumed HitDef `guard.damage` feeds the guard meter | The meter uses 1.5 × raw life damage |
| Push distances (O-35) | Assumed exact pixel agreement | Our engine differs by up to ~2 px from Ikemen's float/tick ordering; documented as residual K-08 |
| MakeDust timing (O-37) | Assumed same-tick spawn | Ikemen exposes the effect one tick later; our engine spawns immediately (one-tick residual) |
| `-rounds` flag | Assumed "rounds needed to win" | It means total rounds to play; a draw forces another round |

## 10. Coverage, guarantees, and gaps

`COMPATIBILITY.md` is the source of truth. At the time of writing, all
Required/Required engine rows are `Verified` except:

| Row | Status | Meaning |
| --- | --- | --- |
| X-04 Stock Ryu | **Partial / blocked** | No stock Ryu DEF/SFF/SND exists in either content root. Only shared loaders can be exercised. A locally supplied Ryu asset is required. |
| C-13 Ikemen-only controllers | **Partial / out of scope** | Dialogue controllers and `RunState` are explicitly out of scope by owner decision. |

X-08 is `Verified` against stated completion criteria: every live
Required/Required row has deterministic tests, and the oracle harness covers
round flow, KFM state/combat, projectiles, throws/helpers, guard/dizzy/power/
push, camera/pause/superpause, dust/animation, and stock-stage rendering.
The criteria exclude documented limitations and hardware-only matrices, so
`Verified` here means "the stated review surface is covered", not "every
possible content file behaves identically to every engine".

Known residual gaps, recorded rather than hidden:

- **No M.U.G.E.N binary oracle** (X-07). M.U.G.E.N claims rest on formats,
  stock content, and tests.
- **Hardware matrices:** real internet/router/NAT netplay, additional gamepads,
  and real audio-device playback are not automated.
- **Float/tick residuals:** push distance and MakeDust spawn timing (above).
- **Content-blocked rows:** X-04.
- **Deliberate scope exclusions:** dialogue controllers and `RunState` (C-13);
  consult the matrix for the current freeze list.

## 11. Where each finding is enforced

| Area | Assertions | Tests |
| --- | --- | --- |
| Round flow | O-01..O-08 | `oracle_round_timer_is_sixty_ticks_per_second`, `oracle_round_win_classification` |
| KFM hit/guard/launch/juggle | O-09..O-19, O-29, O-32, O-35 | `oracle_kfm_state_200_hits_idle`, `oracle_kfm_guard_flow`, `oracle_kfm_launch_flow`, `oracle_kfm_juggle_flow`, `hit_pause_time_reports_self_pause_only` |
| Projectiles | O-21..O-28 | `bundled_projectile_hits`, `bundled_projectile_can_be_guarded`, `nival_projectile_hit_and_guard`, `ordan_projectile_hit_and_guard`, `state_3000_low_blast_hits_for_65`, `state_3000_blasts_are_guarded_for_15`, `nival_super_pauses_then_hits_for_90`, `nival_super_guard_takes_20` |
| Throw/helper | O-30, O-31 | `kaen_state_800_throw_matches_oracle_flow`, `kaen_helper_hit_matches_oracle` |
| Guard/dizzy | O-33, O-34 | `guard_and_dizzy_point_rates_match_oracle`, `guard_hits_drain_guard_points`, `dizzy_stun_lasts_sixty_ticks_and_resets` |
| Camera/pause/superpause | O-36, O-38, O-39 | `camera_start_matches_oracle`, `pause_controller_freezes_opponent_animation`, `superpause_freezes_opponent_animation` |
| Dust/explods | O-37 | GameMakeAnim/MakeDust tests |
| Stock stages | X-05a..X-05e | `stock_stage_render_matches_ikemen_oracle` |

The full assertion text, exact scenario inputs, and the oracle-to-engine
mapping live in [`oracle/assertions.md`](oracle/assertions.md). The harness
procedure and pinned runtime live in [`oracle/README.md`](oracle/README.md).
