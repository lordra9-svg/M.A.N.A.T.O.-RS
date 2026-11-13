# Compatibility matrix

**Status: DRAFT — awaiting approval.** This matrix defines what “compatible”
means for M.A.N.A.T.O.-RS and drives the clean-room rewrite order.

## Status legend

| Status | Meaning |
| --- | --- |
| `Verified` | Implemented and covered by an approved test |
| `Partial` | Implemented but incomplete or not yet oracle-verified |
| `Missing` | Not implemented |
| `TBD` | Scope/priority needs approval |
| `N/A` | Deliberately out of scope |

### Frozen scope decisions

Approved scope is the fight screen plus title/select, `.fnt` v1/v2, explod
perspective, and Ikemen-only zoom. The following rows are therefore `N/A`
for manato and must not be implemented without a new scope approval:
F-12 (storyboards), R-04 (turns/team/tag), R-05 (continue/survival/arcade),
V-09 (custom shaders), and N-03 (spectators/lobby). X-06 and X-07 are also
`N/A`: no Ikemen test content is shipped, and the clean-room policy uses
Ikemen-GO alone as the runtime oracle.

## 1. Content formats

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| F-01 | Text/INI parsing | Required | Required | Verified | Unit tests |
| F-02 | DEF character/stage | Required | Required | Partial | Bundled/stock + localcoord, file-list, palette-map, and stage metadata tests |
| F-03 | CNS states/controllers | Required | Required | Partial | Execution tests incl. fall controllers, EnvColor, Gravity, HitAdd, MoveHitReset |
| F-04 | CMD commands | Required | Required | Partial | Defaults/prefixes/charge/steptime + `buffer.hitpause`/`buffer.pauseend` tests |
| F-05 | AIR animations/Clsn | Required | Required | Partial | AIR parser: Clsn, SubAdd/AS/SS, scale/angle, interpolate markers; interpolation rendering pending |
| F-06 | SFF v1 | Required | Required | Partial | PCX palette/reuse + EOF zero-length link chain tests |
| F-07 | SFF v2 | Required | Required | Partial | Link chains, palette links/group-index, tdata flag, depth fallback, RLE8/5/LZ5 |
| F-08 | SND | Required | Required | Partial | Parser tests + unsigned 8-bit mono/stereo and signed 24-bit conversion |
| F-09 | Stage DEF/BG layers | Required | Required | Partial | BGCtrl + delta/tile/window/mask/scale parser tests; oracle pending |
| F-10 | `st1`..`st9` state files | Required | Required | Partial | Load-order, override, and execution integration test |
| F-11 | Motifs/screenpack | Required | Required | Partial | `system.def`/`fight.def`/`select.def`, `.fnt` v1/v2, title/select/fight rendering, BGCtrl, `[Action]` banners incl. per-side/cap/expiry; storyboards are out of scope (F-12) |
| F-12 | Storyboards | Required | Required | N/A | Out of the approved frozen rendering scope (fight screen + title/select only) |
| F-13 | Select/VS screens | Required | Required | Partial | Roster grid, cursors, portraits, names, timer, stage preview/selection; VS animation is out of the frozen scope |

## 2. State machine

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| S-01 | Statedef defaults/type/physics | Required | Required | Partial | Statedef type/physics/anim/ctrl/sprpriority defaults + hitdefpersist/movehitpersist/hitcountpersist tests |
| S-02 | ChangeState/ChangeAnim | Required | Required | Partial | State round-trips + ctrl/anim overrides + ChangeAnim elem + ChangeAnim2 bank |
| S-03 | CtrlSet/StateTypeSet/MoveTypeSet | Required | Required | Partial | Ctrl/StateType/MoveType/Physics controller and trigger test |
| S-04 | `persistent`/`ignorehitpause` | Required | Required | Partial | Persistent 0/1/N first-tick + every-N timing + ignorehitpause |
| S-05 | Special states -1/-2/-3 | Required | Required | Partial | Per-tick + KO lock + helper keyctrl/helpertype gating |
| S-06 | Command buffer/matching | Required | Required | Partial | Motion/two-button/buffer + charge/greater/tap/steptime + pause-buffer + hold pseudo-commands (diagonals/facing) tests |
| S-07 | State transition loop cap | Required | Required | Verified | Unit tests |
| S-08 | Intro/KO control locks | Required | Required | Verified | Smoke tests |

## 3. Triggers and expressions

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| T-01 | Time/Anim/AnimElem/AnimTime | Required | Required | Partial | Anim triggers + loop boundary and invalid AnimElemTime sentinel tests |
| T-02 | Pos/Vel/StateType/MoveType | Required | Required | Partial | Pos/Vel/Facing/Physics + StateNo/PrevStateNo/Time + P2/distances |
| T-03 | Command/command timers | Required | Required | Partial | Command trigger + time/buffer expiry test |
| T-04 | GetHitVar/HitShakeOver/HitOver | Required | Required | Partial | GetHitVar vel/xoff/yoff/hittime countdown/HitOver equivalence/redlife/hitid/priority/fall/hitshaketime/fallcount/recovertime/fall.recovertime/fall.kill/fall.envshake.*/fall.xvel fall.yvel + HitVel + HitShakeOver/HitOver threshold + HitPauseTime attacker/defender split (O-29) |
| T-05 | P2/redirect/helper/explod/proj vars | Required | Required | Partial | Redirects + NumHelper/NumProj(ID active-only)/NumExplod + SysVar + IsHelper(id)/ParentDist/RootDist axis-facing + ParentExist ancestry tests |
| T-06 | Const/expressions/random | Required | Required | Partial | Const incl. 1080p + radian trig/asin/acos/atan2 + Clamp/Lerp/Sign + ScreenWidth/Height, TicksPerSecond, GameTime + per-slot AILevel |
| T-07 | Team/tag/partner triggers | Required | Ikemen | Partial | `TeamMode` single/simul/turns + `IsHomeTeam` versus side semantics; tag/partner/simul teams out of scope |
| T-08 | Ikemen-only triggers | N/A | Ikemen | Partial | `AnimLength`, `AnimLoopCount`, `PrevAnim`, `SprPriority`, `PrevStateType`, `PrevMoveType`, `StageFrontEdgeDist`, `StageBackEdgeDist`, `ComboCount`, `HitDefVar`, `StageBGVar`, `IsAsserted(flag)`, `LocalCoordX/Y`, `Map`, `Name`, `DisplayName`, `AuthorName`, `NumStageBG`, `NumPlayer`, `PlayerNo`, `PlayerIDExist`, `PlayerIndexExist`, `PlayerNoExist`, `TeamSize`, `TeamLeader`, `MemberNo`, `Standby`, `Score`, `ScoreTotal`, `RedLife`, `ReceivedHits`, `ReceivedDamage`, `SelfStateNoExist`, `StageTime`, `TimeRemaining`, `TimeElapsed`, `TimeTotal` implemented (single-member teams); remaining Ikemen-only trigger families pending |

## 4. Controllers

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| C-01 | Vel/Pos/Power/Life/Ctrl | Required | Required | Partial | Life/Power clamps + Vel/Pos edges + EnvColor time/under |
| C-02 | ChangeState/ChangeAnim/Turn | Required | Required | Partial | State/anim/Turn + ChangeAnim2 anim bank + auto-turn + presentation + AngleSet/Add/Mul/AngleDraw |
| C-03 | HitDef/ReversalDef | Required | Required | Partial | HitDef + AttackDist guard-distance update + affectteam + id/chainid/nochainid gating + prio/numhits/hitonce throw default + p1sprpriority default 1/p2sprpriority + getpower/givepower + HitCount/Uniq + MoveReversed/reversal/sounds/sparks + animtype/air.animtype/fall.animtype selection + ground/air type inheritance and timing defaults + yaccel/fall.yvelocity localcoord defaults/physics + None pause-only path + guard.damage/hittime/slidetime/ctrltime/dist/pausetime + mindist/maxdist/snap + xoff/yoff offsets + p1facing/p1getp2facing/p2facing + p2getp1state state/controller/animation bank + SelfState/get-hit clearing + airguard.velocity default + airguard.ctrltime + fall.recovertime/fall.kill/fall.envshake/envshake/palfx/air.fall/forcestand/forcecrouch/forcenofall + down.velocity/down.hittime/down.recover/down.recovertime/5110-getup/down.bounce/D-hitflag/5080/down.cornerpush + kill/guard.kill + direct guard velocity + ground/air/guard/airguard cornerpush parsing/defaults |
| C-04 | Projectile | Required | Required | Partial | Spawn/lifetime/hits + proj* aliases + bound removal + hit/removal animations + remvelocity |
| C-05 | Explod/ModifyExplod/RemoveExplod | Required | Required | Partial | animelem/freeze + bind + Modify trans/ownpal/shadow/bind/removetime |
| C-06 | Helper/Bind/Root/Target | Required | Required | Partial | Helper postype anchors + BindTo* + Target* + ownership/DestroySelf |
| C-07 | AfterImage/PalFX/Trans | Required | Required | Partial | AfterImage/PalFX time + sinadd/mul + round reset + Trans alpha; RemapPal deferred |
| C-08 | SuperPause/Pause/EnvShake | Required | Required | Partial | Pause/SuperPause movetime + EnvShake phase/dir/diradd/mul/decay + Nival oracle |
| C-09 | NotHitBy/HitBy/HitOverride | Required | Required | Partial | Filters incl. wildcard attrs/timer expiry + override slots/keepstate/forceguard |
| C-10 | Width/ScreenBound/PlayerPush | Required | Required | Partial | Width forms + ScreenBound X/Y axis clamps + PlayerPush separation |
| C-11 | AssertSpecial flags | Required | Required | Partial | Full mapping + per-tick render flags + NoHardcodedKeys + GlobalNoShadow |
| C-12 | MakeDust/GameMakeAnim | Required | Required | Partial | MakeDust spacing/pos2 + GameMakeAnim placement tests; oracle pending |
| C-13 | Ikemen-only controllers | N/A | Ikemen | Partial | GuardPointsAdd/GuardPointsSet/GuardBreakSet/DizzyPointsAdd/DizzyPointsSet/DizzySet/ModifyPlayer maxima/attack/defence/ailevel/TargetGuardPointsAdd/TargetDizzyPointsAdd/TargetGuardPointsSet/TargetDizzyPointsSet/GetHitVarSet/RoundTimeAdd/RoundTimeSet/VictoryQuote/ScoreAdd/TargetScoreAdd/RedLifeAdd/RedLifeSet/TargetRedLifeAdd/MapSet/MapAdd/MapReset/ParentMapSet/Add/RootMapSet/Add/TeamMapSet/Add/Height/Camera/ModifyHitDef/ModifyReversalDef/ModifyShadow/ModifyReflection subsets implemented; other Ikemen-only controllers pending |

## 5. Collision and combat

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| K-01 | Clsn1/Clsn2 transforms | Required | Required | Partial | Mirror/normalize/translate box test + KFM 200 hit flow |
| K-02 | Hit/guard flags and priorities | Required | Required | Partial | Guard flags/M + p1/p2 sprpriority + guard.velocity + F gate; KFM guard oracle |
| K-03 | Hit pause/super pause | Required | Required | Partial | Freeze + air hit + reaction-state selection (5010/5020/5030/5070); attacker/defender pausetime split (O-29); KFM oracle |
| K-04 | Juggle/air hit/fall states | Required | Required | Partial | air.juggle gate + HitFallSet semantics + fall velocity/landing + lethal fall KO |
| K-05 | Throws/target bind | Required | Required | Partial | Throw/bind/drop + Ordan slam + p1/p2 facing/sprpriority application |
| K-06 | Projectile collision | Required | Required | Partial | Hit/guard oracle O-21..O-28 + hits-limit removal; state 3000 upper blast 1px box margin |
| K-07 | Helper collision | Required | Required | Partial | Helper HitDef + helper-owned projectile owner/damage tests; oracle pending |
| K-08 | Corner push/pushboxes | Required | Required | Partial | Ground/air corner-push offsets, threshold, guard + PlayerPush separation |
| K-09 | Guard crush/dizzy/guard points | N/A | Ikemen | Partial | Guard points + GuardPointsAdd/Set + GuardBreakSet + TargetGuardPointsAdd/Set + GetHitVar(guardpoints) + 60-tick regen cooldown/+2 regen + 20-tick guard-crush no-guard window + dizzy points/1.8x hit loss/60-tick stun + DizzyPointsAdd/Set + TargetDizzyPointsAdd/Set + DizzySet + GetHitVar(dizzypoints) + ModifyPlayer maxima + Dizzy triggers; custom guard-break/dizzy states and effects pending |

## 6. Rounds and game flow

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| R-01 | Intro/Fight/KO/Over | Required | Required | Partial | Phase sequence/control locks + round reset power carry/helper removal |
| R-02 | Timer/KO/draw/best-of-three | Required | Required | Partial | Best-of-N/draw + RoundsWon/LoseKO/LoseTime + timer freeze/clamp |
| R-03 | Win poses/quotes | Required | Required | Partial | Win pose (state 180) entry + RoundNotOver delay + `[Quotes]` parsing + VictoryQuote selection + `[WinQuote]` display tests |
| R-04 | Turns/team/tag modes | Required | Ikemen | N/A | Out of the approved frozen scope; single/simul round structure only |
| R-05 | Continue/survival/arcade | Required | Required | N/A | Out of the approved frozen scope; single-match flow only |
| R-06 | Match restart/round time controllers | Required | Ikemen | Partial | RoundTime/NoKOSlow/NoKO flags + RoundTimeAdd/RoundTimeSet tick controllers + `restart_match` API and R-key reset |

## 7. Rendering

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| V-01 | Sprite/palette rendering | Required | Required | Verified | Screenshots |
| V-02 | Blend modes | Required | Required | Verified | Screenshots |
| V-03 | Stage layers/parallax/tiling | Required | Required | Partial | Layer geometry + BGCtrl vel/pos/visible/anim/sin/cos update tests |
| V-04 | HUD/lifebars/fonts | Required | Required | Partial | Win-icon kind/perfect/cap + combo refresh + delayed fills + guard points/bar rendering with hide-at-zero + red-life bar layer from red_life/life_max + guard-crush drain/window; custom guard-break effects pending |
| V-05 | Explods/projectiles | Required | Required | Partial | Lifetime/removal + edge/window/perspective edges + projectile shear/perspective rendering + projectile hit/guard oracle |
| V-06 | Afterimages | Required | Required | Partial | Ghost sampling cadence/decay + controller-time/alpha tests |
| V-07 | Screenpack/motifs | Required | Required | Partial | Manato Grid title/select/fight, fonts, BGCtrl, portraits, select timer, round sounds, action banner displaytime/lifecycle; VS/storyboards are out of scope |
| V-08 | Zoom/camera controls | N/A | Ikemen | Partial | Round reset startx/zoom + zoom-anchor + CameraPos/CameraZoom triggers + tension/bounds + Zoom stagebound/camerabound clamping |
| V-09 | Custom shaders | N/A | Ikemen | N/A | Out of the approved frozen rendering scope |

## 8. Audio

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| A-01 | SND playback | Required | Required | Partial | SND + channels/priority; oracle pending |
| A-02 | Character/effect sounds | Required | Required | Verified | Roster + hit/guard sound events and silent group -1 |
| A-03 | BGM/music | Required | Required | Partial | Symphonia decode + looping mixer; oracle pending |
| A-04 | Pan/volume/channels | Required | Required | Partial | Channel priority, pan, master/music volume, tests |
| A-05 | Pause/loop/modify sound | Required | Ikemen | Partial | PlaySnd loop, StopSnd/SndPan/ModifySnd, BGM pause/resume test |

## 9. Input, AI, and netplay

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| I-01 | Keyboard input | Required | Required | Verified | Window run |
| I-02 | Gamepad input | Required | Required | Partial | gilrs backend, d-pad/stick/buttons mapped; hardware matrix pending |
| I-03 | Remapping | Required | Required | Partial | TOML keyboard bindings; gamepad remap pending |
| I-04 | Deterministic CPU AI | Required | Required | Partial | Smoke tests |
| N-01 | Rollback determinism | Required | Required | Partial | SyncTest |
| N-02 | P2P input exchange | Required | Required | Partial | Localhost GGRS loopback test; internet/latency matrix pending |
| N-03 | Spectators/lobby | N/A | Ikemen | N/A | Out of the approved frozen scope; two-player rollback only |

## 10. Acceptance content

| ID | Content | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| X-01 | Kaen, Nival, Ordan | N/A | N/A | Verified | Bundled tests |
| X-02 | Training Grid | N/A | N/A | Verified | Bundled tests |
| X-03 | Stock KFM | Required | Required | Partial | Stock tests |
| X-04 | Stock Ryu | Required | Required | Partial | Content load |
| X-05 | Stock stages | Required | Required | Partial | kfm/stage0/stage0-720/stage1 load tests; render oracle pending |
| X-06 | Ikemen-GO test content | N/A | Required | N/A | Not shipped; Ikemen-GO is used only as the runtime oracle |
| X-09 | Manato Grid screenpack | N/A | N/A | Verified | Bundled generated art; title/select/fight screenshots |
| X-07 | M.U.G.E.N 1.1 oracle assertions | Required | N/A | N/A | Clean-room policy forbids M.U.G.E.N binary analysis; Ikemen-GO is the only oracle |
| X-08 | Ikemen-GO oracle assertions | N/A | Required | Partial | Round classification oracle in `docs/oracle/`; other subsystems pending |

## 11. Proposed rewrite order

1. Formats: SFF v1/v2, SND, AIR, CNS/CMD, stage parsing.
2. State machine and trigger/controller semantics.
3. Collision, HitDef, guard, throws, juggle.
4. Rounds and game flow.
5. Rendering gaps: explods, afterimages, screenpack, zoom.
6. Audio gaps: BGM, channels, pause.
7. Input/AI/netplay gaps.
8. Ikemen-only extensions from the approved matrix.

## 12. Approval checklist

- [ ] Rows and statuses accepted.
- [ ] Priority/order accepted.
- [ ] Acceptance content accepted.
- [ ] Oracle assertion scope accepted.
- [ ] Any rows to mark `N/A` removed or confirmed.
