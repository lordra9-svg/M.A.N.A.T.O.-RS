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

## 1. Content formats

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| F-01 | Text/INI parsing | Required | Required | Verified | Unit tests |
| F-02 | DEF character/stage | Required | Required | Partial | Bundled + stock tests |
| F-03 | CNS states/controllers | Required | Required | Partial | CNS spec + parser; execution gaps remain |
| F-04 | CMD commands | Required | Required | Partial | CMD spec + parser/defaults/prefixes; oracle pending |
| F-05 | AIR animations/Clsn | Required | Required | Partial | AIR spec + parser tests; SubAdd/interpolation rendering pending |
| F-06 | SFF v1 | Required | Required | Partial | SFF spec + bundled/stock tests; oracle pending |
| F-07 | SFF v2 | Required | Required | Partial | SFF spec + bundled/stock tests; oracle pending |
| F-08 | SND | Required | Required | Partial | SND spec + parser tests; oracle pending |
| F-09 | Stage DEF/BG layers | Required | Required | Partial | BGCtrl/reflection/BGM/window/mask added; oracle pending |
| F-10 | `st1`..`st9` state files | Required | Required | Partial | Ryu option file |
| F-11 | Motifs/screenpack | Required | Required | Missing | TBD |
| F-12 | Storyboards | Required | Required | Missing | TBD |
| F-13 | Select/VS screens | Required | Required | Missing | TBD |

## 2. State machine

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| S-01 | Statedef defaults/type/physics | Required | Required | Partial | Bundled roster |
| S-02 | ChangeState/ChangeAnim | Required | Required | Partial | Bundled roster |
| S-03 | CtrlSet/StateTypeSet/MoveTypeSet | Required | Required | Partial | Bundled roster |
| S-04 | `persistent`/`ignorehitpause` | Required | Required | Partial | Projectiles |
| S-05 | Special states -1/-2/-3 | Required | Required | Partial | Bundled roster |
| S-06 | Command buffer/matching | Required | Required | Partial | AI/roster |
| S-07 | State transition loop cap | Required | Required | Verified | Unit tests |
| S-08 | Intro/KO control locks | Required | Required | Verified | Smoke tests |

## 3. Triggers and expressions

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| T-01 | Time/Anim/AnimElem/AnimTime | Required | Required | Partial | Unit tests |
| T-02 | Pos/Vel/StateType/MoveType | Required | Required | Partial | Unit tests |
| T-03 | Command/command timers | Required | Required | Partial | Roster |
| T-04 | GetHitVar/HitShakeOver/HitOver | Required | Required | Partial | Core hit triggers; oracle pending |
| T-05 | P2/redirect/helper/explod/proj vars | Required | Required | Partial | Projectiles |
| T-06 | Const/expressions/random | Required | Required | Partial | Unit tests |
| T-07 | Team/tag/partner triggers | Required | Ikemen | Missing | TBD |
| T-08 | Ikemen-only triggers | N/A | Ikemen | Missing | TBD |

## 4. Controllers

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| C-01 | Vel/Pos/Power/Life/Ctrl | Required | Required | Partial | LifeAdd/LifeSet/PlayerPush/Gravity added |
| C-02 | ChangeState/ChangeAnim/Turn | Required | Required | Partial | Roster |
| C-03 | HitDef/ReversalDef | Required | Required | Partial | HitDef/ReversalDef added |
| C-04 | Projectile | Required | Required | Partial | Projectile tests |
| C-05 | Explod/ModifyExplod/RemoveExplod | Required | Required | Partial | TBD |
| C-06 | Helper/Bind/Root/Target | Required | Required | Partial | Helper/BindTo/ParentVar/RootVar added; oracle pending |
| C-07 | AfterImage/PalFX/Trans | Required | Required | Partial | AfterImage/PalFX/Trans added; RemapPal pending |
| C-08 | SuperPause/Pause/EnvShake | Required | Required | Partial | Roster |
| C-09 | NotHitBy/HitBy/HitOverride | Required | Required | Partial | HitOverride added; oracle pending |
| C-10 | Width/ScreenBound/PlayerPush | Required | Required | Partial | ScreenBound/PlayerPush added |
| C-11 | AssertSpecial flags | Required | Required | Partial | Roster |
| C-12 | MakeDust/GameMakeAnim | Required | Required | Partial | TBD |
| C-13 | Ikemen-only controllers | N/A | Ikemen | Missing | TBD |

## 5. Collision and combat

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| K-01 | Clsn1/Clsn2 transforms | Required | Required | Partial | Box transforms + combat tests; oracle pending |
| K-02 | Hit/guard flags and priorities | Required | Required | Partial | Guard flags + guard damage tests |
| K-03 | Hit pause/super pause | Required | Required | Partial | Hit pause applied; oracle pending |
| K-04 | Juggle/air hit/fall states | Required | Required | Partial | Juggle point tests + fall states |
| K-05 | Throws/target bind | Required | Required | Partial | Throw states/target bind test |
| K-06 | Projectile collision | Required | Required | Partial | Projectile tests |
| K-07 | Helper collision | Required | Required | Partial | Helper HitDef test; oracle pending |
| K-08 | Corner push/pushboxes | Required | Required | Partial | Corner-push + PlayerPush tests |
| K-09 | Guard crush/dizzy/guard points | N/A | Ikemen | Missing | TBD |

## 6. Rounds and game flow

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| R-01 | Intro/Fight/KO/Over | Required | Required | Partial | Smoke tests |
| R-02 | Timer/KO/draw/best-of-three | Required | Required | Partial | Smoke tests |
| R-03 | Win poses/quotes | Required | Required | Missing | TBD |
| R-04 | Turns/team/tag modes | Required | Ikemen | Missing | TBD |
| R-05 | Continue/survival/arcade | Required | Required | Missing | TBD |
| R-06 | Match restart/round time controllers | Required | Ikemen | Missing | TBD |

## 7. Rendering

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| V-01 | Sprite/palette rendering | Required | Required | Verified | Screenshots |
| V-02 | Blend modes | Required | Required | Verified | Screenshots |
| V-03 | Stage layers/parallax/tiling | Required | Required | Partial | Training Grid |
| V-04 | HUD/lifebars/fonts | Required | Required | Partial | Screenshots |
| V-05 | Explods/projectiles | Required | Required | Partial | Projectile tests |
| V-06 | Afterimages | Required | Required | Partial | Ghost trail rendering added |
| V-07 | Screenpack/motifs | Required | Required | Missing | TBD |
| V-08 | Zoom/camera controls | N/A | Ikemen | Missing | TBD |
| V-09 | Custom shaders | N/A | Ikemen | Missing | TBD |

## 8. Audio

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| A-01 | SND playback | Required | Required | Partial | Audio smoke |
| A-02 | Character/effect sounds | Required | Required | Verified | Roster |
| A-03 | BGM/music | Required | Required | Partial | Symphonia decode + looping mixer; oracle pending |
| A-04 | Pan/volume/channels | Required | Required | Partial | Mixer tests |
| A-05 | Pause/loop/modify sound | Required | Ikemen | Missing | TBD |

## 9. Input, AI, and netplay

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| I-01 | Keyboard input | Required | Required | Verified | Window run |
| I-02 | Gamepad input | Required | Required | Missing | TBD |
| I-03 | Remapping | Required | Required | Missing | TBD |
| I-04 | Deterministic CPU AI | Required | Required | Partial | Smoke tests |
| N-01 | Rollback determinism | Required | Required | Partial | SyncTest |
| N-02 | P2P input exchange | Required | Required | Partial | TBD |
| N-03 | Spectators/lobby | N/A | Ikemen | Missing | TBD |

## 10. Acceptance content

| ID | Content | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| X-01 | Kaen, Nival, Ordan | N/A | N/A | Verified | Bundled tests |
| X-02 | Training Grid | N/A | N/A | Verified | Bundled tests |
| X-03 | Stock KFM | Required | Required | Partial | Stock tests |
| X-04 | Stock Ryu | Required | Required | Partial | Content load |
| X-05 | Stock stages | Required | Required | Partial | TBD |
| X-06 | Ikemen-GO test content | N/A | Required | Missing | TBD |
| X-07 | M.U.G.E.N 1.1 oracle assertions | Required | N/A | Missing | TBD |
| X-08 | Ikemen-GO oracle assertions | N/A | Required | Missing | TBD |

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
