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
perspective, and Ikemen-only zoom. The project owner has since **expanded the
scope** to include F-12 (storyboards), R-04 (turns/team/tag), R-05
(continue/survival/arcade), V-09 (custom shaders), and N-03
(spectators/lobby); those rows now carry `Partial` and are pending
implementation. X-06 and X-07 remain `N/A`: no Ikemen test content is
shipped, and the clean-room policy uses Ikemen-GO alone as the runtime oracle.

## Remaining work

No row is `Missing` or `TBD`. Every remaining `Partial` is either implemented
with residual edge cases or blocked on a missing subsystem. The blockers are
deliberate, not scheduling gaps:

| Blocker | Rows | Reason |
| --- | --- | --- |
| After-image remap | C-07, C-13 | `RemapSprite` group/index substitution and named presets are implemented; `AfterImage` palette options (`palbright`, `palcontrast`, `paladd`, `palmul`, `palpostbright`, `palcolor`, `palhue`, `palinvertall`, `palinvertblend`) are modeled. |
| 3D / depth | T-04, V-03 | `Depth`/`DepthBound`/z position, a wgpu depth buffer, stage `[Scaling]` perspective for fighters and layers, layer `ModifyBGCtrl3D` (id + z/scale approximation), and static glTF triangle models from stage `[Model]`/storyboard `[SceneDef] model` (base-color textures + baseColorFactor and first-clip node translation/rotation/scale animation, 4-weight skinning, morph-target position playback, and simple normal/metallic/roughness directional+ambient lighting, baked placement) are implemented; glTF multi-clip animation and split-sum environment lighting with a GPU BRDF LUT, content-configurable model lights, and clearcoat/transmission/anisotropy/sheen/iridescence/volume/refraction/dispersion material extensions are implemented; `ModifyBGCtrl3D` targets model nodes by index; effect/shadow/text/projectile/reflection perspective is implemented; fighter/projectile z-axis hits use `HitDef depth`/`Depth` extents and projectile z plus `projdepthbound`; fighter `DepthBound` clamps z to the stage range. `ScreenBound movecamera` gates fighting-camera X/Y tracking and `DepthBound movecamera` gates Z tracking with `[Camera] ztension`, `ztensionvel`, `zautocenter`, `zboundtop`, and `zboundbot` bounds. |
| Storyboards | F-12, C-13 | Front-end scenes, fonts, sounds, scene BGM, `layerall.pos`, full layer PalFX (time-sensitive add/mul, color, hue, sinadd, sinmul, sincolor, sinhue, invertall, invertblend), AIR interpolation, and in-match sprite/text overlays with clears/fades render/play; a sandboxed `[SceneDef] lua` runtime drives overlay transforms, alpha/done, and named scene jumps, `bg.name` stage backgrounds render under front-end scenes and in-match overlays with `BgCtrl` overrides, and static glTF scene models render behind them (skins/animations/textures remain unmodeled). |

| Team/tag flow | R-04 | Active/standby rosters, TagIn/TagOut swaps with tag-out state animation timing and tag-in invulnerability windows, Turns KO advancement, simultaneous teams of arbitrary roster size (staggered CPU members, team-aware targeting/inputs/camera/KO/time-over), front-end mode and three-partner-per-side selection, select.def single-only rules, and partner lifebars work; front-end selection of more than three partners remains unmodeled. |
| Arcade flow | R-05 | Ladder/continue/survival state, app progression, roster-order/random opponents, `[Arcade]` special + final-boss placement, per-opponent stages and music, survival rotation, a persisted top-five score table with SCORES screen, and the continue overlay exist; per-character `arcadepath` and mid-run challengers remain unmodeled. |
| GLSL uniform layer | V-09 | `.glsl`/`.frag` fragments execute with explicit Vulkan-style bindings or Ikemen-style implicit uniforms (sampler pairs, locations, `TextureSize`, PalFX slots, dual GL/Vulkan template), a built-in vertex stage, a `shaderparam` uniform block, and real `pal` palette plus `tex1`/`tex2` sprite-reference textures and a previous-frame `bgl_RenderedTexture` grab pass. |
| Spectators/lobby | N-03 | Lobby state/protocol, server routing, UDP relay, spectator frame sync, chunked state streaming with host serialization and spectator restore/checksum verification, plus a direct continuous broadcaster and app `--broadcast`/`--spectate` mode with an on-screen status line exist; LAN discovery, STUN public-address discovery, app `--discover`/`--stun` modes, ranked `--matchmake` spectator join, and `LobbyClient`/`--matchmake-play` player join/ready/start coordination, announced GGRS ports, `Peer` handoff messages, `MatchHandoff`, `Netplay::from_handoff`, headless `--matchmake-play` auto-entry, windowed `--netplay` input wiring, `--matchmake-window` lobby-to-window auto-entry, best-effort `NatPuncher` hole punching, a two-peer `UdpForwarder`/`--relay`, a framed eight-peer `MultiRelay`/`--relay-multi`, shared-secret HMAC and realm/user long-term credential join authentication with nonce replay protection, `RelayGgrsSocket`/`Netplay::from_relay` GGRS integration, `--relay-fallback` automatic punch-to-relay selection, and UDP, TCP, and TLS (rustls) STUN/TURN servers (Allocate/Refresh/CreatePermission/ChannelBind/ChannelData with long-term MESSAGE-INTEGRITY) are implemented; Screen-space refraction and physically split dispersion rays remain unmodeled. |

Oracle-only rows (`F-09`, `K-01`..`K-09`, `V-05`, `V-08`, `A-01`, `A-03`,
`A-04`, `R-01`..`R-06`, and the remaining `S-`/`T-` rows) are implemented and
covered by engine tests; they stay `Partial` because X-07 forbids asserting a
M.U.G.E.N binary oracle. Closing them requires either a documented engine
decision or an approved Ikemen-GO runtime comparison.

Smaller open edge cases are tracked in the row's Verification column. The
next candidates are the oracle/hardware comparison rows and remaining
`Partial` edge cases.

## 1. Content formats

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| F-01 | Text/INI parsing | Required | Required | Verified | Unit tests |
| F-02 | DEF character/stage | Required | Required | Partial | Bundled/stock + localcoord, file-list, palette-map, stage metadata tests; `st`/`st0`..`stN` natural-order merging; Windows `\` path normalization |
| F-03 | CNS states/controllers | Required | Required | Partial | Execution tests incl. fall controllers, EnvColor, Gravity, HitAdd, MoveHitReset |
| F-04 | CMD commands | Required | Required | Partial | Defaults/prefixes/charge/steptime + `buffer.hitpause`/`buffer.pauseend` tests |
| F-05 | AIR animations/Clsn | Required | Required | Partial | AIR parser: Clsn, SubAdd/AS/SS, scale/angle, interpolate markers; offset/scale/angle/blend-alpha interpolation rendered for fighters, projectiles, explods, and stage BG animations |
| F-06 | SFF v1 | Required | Required | Partial | PCX palette/reuse + EOF zero-length link chain + declared data-length payload bounds (alignment padding excluded) tests |
| F-07 | SFF v2 | Required | Required | Partial | Link chains, palette links/group-index, tdata flag, depth fallback, RLE8/5/LZ5, PNG8/24/32, minor2 palette alpha, missing-palette fallback, truncated-PNG safety |
| F-08 | SND | Required | Required | Partial | Parser tests + unsigned 8-bit mono/stereo, signed 16/24/32-bit, and float conversion + header-offset validation + truncated-entry skip |
| F-09 | Stage DEF/BG layers | Required | Required | Partial | BGCtrl types + `time` windows/loops + ModifyBGCtrl retiming + delta/tile/window/mask/scale parser tests; oracle pending |
| F-10 | `st1`..`st9` state files | Required | Required | Partial | Load-order, override, and execution integration test |
| F-11 | Motifs/screenpack | Required | Required | Partial | `system.def`/`fight.def`/`select.def`, `.fnt` v1/v2, title/select/fight rendering, BGCtrl, `[Action]` banners incl. per-side/cap/expiry; storyboards tracked by F-12 |
| F-12 | Storyboards | Required | Required | Partial | DEF `[SceneDef]`/`[Scene]` + inline AIR, sprite/anim layers, text with typing delay and `fontN` .fnt fonts, soundN events through the audio engine, scene BGM, fades, clear colour, scene end/jump player, a storyboard renderer/`--storyboard` player, and in-match `Storyboard`/`ModifyStoryboard` sprite and text overlays with per-overlay atlas banks, clears, and fades; sandboxed `[SceneDef] lua` overlay transforms and named scene jumps; `bg.name` front-end and in-match overlay scene backgrounds with ticked `BgCtrl` (visible/enable/anim/pos/velocity/scale/sprite); static textured glTF scene models with multi-clip node animation, skinning, morph targets, normal/emissive/ORM maps, a metallic-roughness BRDF, split-sum envmap IBL, configurable lights, and the supported PBR material extensions |
| F-13 | Select/VS screens | Required | Required | Partial | Roster grid, cursors, portraits, names, timer, stage preview/selection; VS animation is out of the frozen scope |

## 2. State machine

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| S-01 | Statedef defaults/type/physics | Required | Required | Partial | Statedef type/physics/anim/ctrl/sprpriority defaults + juggle persistence + hitdefpersist/movehitpersist/hitcountpersist tests |
| S-02 | ChangeState/ChangeAnim | Required | Required | Partial | State round-trips + ctrl/anim overrides + ChangeState continue + ChangeAnim elem/elemtime + ChangeAnim2 bank |
| S-03 | CtrlSet/StateTypeSet/MoveTypeSet | Required | Required | Partial | Ctrl/StateType/Physics controller tests + first-letter/quoted type names + `U` unchanged (no separate MoveTypeSet controller in either target) |
| S-04 | `persistent`/`ignorehitpause` | Required | Required | Partial | Persistent 0/1/N first-tick + every-N timing + negative persistent every tick + ignorehitpause |
| S-05 | Special states -1/-2/-3 | Required | Required | Partial | Per-tick + KO lock + own-bank lookup + borrowed-state -3/-1 skip + helper keyctrl/helpertype gating |
| S-06 | Command buffer/matching | Required | Required | Partial | Motion/two-button/buffer + charge/greater/tap/steptime + pause-buffer + button `/` hold and `~` release + hold pseudo-commands (diagonals/facing) tests |
| S-07 | State transition loop cap | Required | Required | Verified | Unit tests |
| S-08 | Intro/KO control locks | Required | Required | Verified | Smoke tests |

## 3. Triggers and expressions

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| T-01 | Time/Anim/AnimElem/AnimTime | Required | Required | Partial | Anim triggers + AnimElemNo look-ahead/wrap + loop boundary and invalid AnimElemTime sentinel + AnimExist animation-bank vs SelfAnimExist own-bank tests |
| T-02 | Pos/Vel/StateType/MoveType | Required | Required | Partial | Pos/Vel/Facing/Physics (incl. Pos Z/Vel Z) + StateNo/PrevStateNo/Time + P2 distances incl. M.U.G.E.N-version P2BodyDist Y signed axis difference |
| T-03 | Command/command timers | Required | Required | Partial | Command trigger + time/buffer expiry + M.U.G.E.N `/` step propagation and hold-only 1-tick buffer quirks |
| T-04 | GetHitVar/HitShakeOver/HitOver | Required | Required | Partial | HitDef depth + z-axis hit gating + DepthBound z clamping + GetHitVar vel/xoff/yoff/hittime countdown/HitOver equivalence/redlife/hitid/priority/fall/hitshaketime/fallcount/recovertime/fall.recovertime/fall.kill/fall.envshake.*/fall.xvel fall.yvel + ground/air/fall.animtype banks + hitdamage/guarddamage/kill/guardcount/attr + zoff (always zero) + zvel/zaccel/fall.zvel + xveladd/yveladd (always zero) + HitVel + HitShakeOver/HitOver threshold + HitPauseTime attacker/defender split (O-29) + PowerMax/Attack/AttackMul/Defence/DefenceMul/GuardCount/Angle/HitOverridden/ClsnOverlap |
| T-05 | P2/redirect/helper/explod/proj vars | Required | Required | Partial | Redirects + NumHelper/NumProj(ID active-only)/NumExplod + ExplodVar(id, index, field) incl. animplayerno/bindid/loopcount/teamside/pos.z/vel.z/accel.z + ProjVar(id, index, field) incl. projhitsmax/remvelocity(.x/.y/.z)/teamside/pos.z/vel.z/accel.z + HelperVar + Index + SysVar + ID/PlayerID 1-based unique IDs + IsHelper(id)/ParentDist/RootDist axis-facing + ParentExist ancestry tests |
| T-06 | Const/expressions/random | Required | Required | Partial | Const incl. 1080p + radian trig/asin/acos/atan2 + Clamp/Lerp/Sign/Round/RandomRange + Cond/E/Ln/Log(base,value) + ScreenWidth/Height, TicksPerSecond, GameTime + per-slot AILevel + old-style TimeMod |
| T-07 | Team/tag/partner triggers | Required | Ikemen | Partial | `TeamMode` reports the simulated mode, `TeamSize`/`TeamLeader`/`MemberNo`/`Standby`/`NumPartner` cover rosters and simultaneous partners, and `IsHomeTeam` follows versus side semantics |
| T-08 | Ikemen-only triggers | N/A | Ikemen | Partial | `AnimLength`, `AnimLoopCount`, `PrevAnim`, `SprPriority`, `PrevStateType`, `PrevMoveType`, `StageFrontEdgeDist`, `StageBackEdgeDist`, `ComboCount`, `ConsecutiveWins`, `EnvShakeVar`, `FightTime`, `HelperIndexExist`, `IkemenVersion`, `MugenVersion`, `LastPlayerID`, `HitDefVar`, `SpriteVar`, `StageBGVar`, `StageVar`, `NumStageBG(id)`, `IsAsserted(flag)`, `LocalCoordX/Y`, `Map`, `Name`, `DisplayName`, `AuthorName`, `NumStageBG`, `NumPlayer`, `PlayerNo`, `PlayerIDExist`, `PlayerIndexExist`, `PlayerNoExist`, `PalfxVar`, `Scale.X/Y`, `Offset.X/Y`, `XShear`, `Alpha.Source/Dest`, `Pi`, `Float`, `AILevelF`, `AnimPlayerNo`, `SpritePlayerNo`, `HelperName`, `InCustomState`, `InCustomAnim`, `SelfCommand`, `ReversalDefAttr`, `P5Name`–`P8Name`, `RunOrder`, `GroundAngle`, `DrawPal(group|index)`, `LocalCoord(x|y)`, `FirstAttack`, `IsHost`, `GameMode`, `InputTime(key)`, `MoveCountered`, `MoveHitVar(playerid|id|playerno|uniqhit/cornerpush.veloff/frame/power/sparkx/sparky/overridden)`, `InRunState`, `ClsnVar(group, index, field) with TransformClsn rotation`, `AnimElemVar(field)`, `StageConst(key)`, `Analog(axis)`, `IntroState`, `OutroState`, `FightScreenState(field)`, `FightScreenVar(field)`, `MotifVar(field)`, `MotifState(field)`, `GameVar(field)`, `ProjClsnOverlap(index, targetID, box)`, `BgmVar(field)`, `GameOption(key)`, `SoundVar(channel, field)`, `WinSpecial`, `WinHyper`, `AirJumpCount`, `XAngle`, `YAngle`, `Scale.Z`, `Player(n)` redirect, `TopBoundDist`, `TopBoundBodyDist`, `BotBoundDist`, `BotBoundBodyDist`, `TeamSize`, `TeamLeader`, `MemberNo`, `Standby`, `Score`, `ScoreTotal`, `RedLife`, `ReceivedHits`, `ReceivedDamage`, `SelfStateNoExist`, `StageTime`, `TimeRemaining`, `TimeElapsed`, `TimeTotal` implemented (single-member teams); `NumText`/`Text`/`ModifyText`/`RemoveText` (including z position/velocity/acceleration) are implemented, `ProjCancelTime` is implemented via projectile clash resolution, and `Shader()` reports the active `ShaderSet` name |

## 4. Controllers

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| C-01 | Vel/Pos/Power/Life/Ctrl | Required | Required | Partial | Life/Power clamps + LifeAdd absolute/defence scaling + Vel/Pos edges + EnvColor time/under |
| C-02 | ChangeState/ChangeAnim/Turn | Required | Required | Partial | State/anim/Turn + ChangeAnim2 anim bank + ChangeAnim elemtime/invalid-elem rules + auto-turn + presentation + AngleSet/Add/Mul/AngleDraw |
| C-03 | HitDef/ReversalDef | Required | Required | Partial | HitDef + AttackDist guard-distance update + affectteam + id/chainid/nochainid gating + prio/numhits/hitonce throw default + p1sprpriority default 1/p2sprpriority + getpower/givepower + HitCount/Uniq + MoveReversed/reversal/sounds/sparks + palfx/envshake only on unguarded hits + animtype/air.animtype/fall.animtype selection + ground/air type inheritance and timing defaults + yaccel/fall.yvelocity localcoord defaults/physics + None pause-only path + guard.damage/hittime/slidetime/ctrltime/dist/pausetime + mindist/maxdist/snap + xoff/yoff offsets + p1facing/p1getp2facing/p2facing + p2getp1state state/controller/animation bank + SelfState/get-hit clearing + airguard.velocity default + airguard.ctrltime + fall.recovertime/fall.kill/fall.envshake/envshake/palfx/air.fall/forcestand/forcecrouch/forcenofall + down.velocity/down.hittime/down.recover/down.recovertime/5110-getup/down.bounce/D-hitflag/5080/down.cornerpush + kill/guard.kill + direct guard velocity + ground/air/guard/airguard cornerpush parsing/defaults |
| C-04 | Projectile | Required | Required | Partial | Spawn/lifetime/hits + proj* aliases + bound removal + hit/removal animations + remvelocity + projpriority default 1 + projanim 0 + Clsn2 projectile clashes/projcancelanim/ProjCancelTime + z offset/velocity/accel + projdepthbound + z-gated hits + projxangle/projyangle rendering, ProjVar angle.x/y, projwindow clipping, projclsnscale/projclsnangle collision transforms, velmul per-tick velocity scaling, projreflection rendering, projectile shader/param/tex1/tex2 bindings (including shader.time/reflection flags, supermovetime/pausemovetime, ownpal/remappal with dedicated recolored projectile banks, projshadow rendering, and projectile afterimage trails) |
| C-05 | Explod/ModifyExplod/RemoveExplod | Required | Required | Partial | animelem/freeze + bind + Modify trans/ownpal/shadow/bind/removetime/facing/vfacing + RemoveExplod omitted-id/all-owned semantics |
| C-06 | Helper/Bind/Root/Target | Required | Required | Partial | Helper postype anchors + persistent BindToRoot/Parent/Target with time/facing + Target* + ownership/DestroySelf |
| C-07 | AfterImage/PalFX/Trans | Required | Required | Partial | AfterImage palette options + PalFX time + sinadd/sinmul + PalFX color/hue/invertall/invertblend/sincolor/sinhue + AllPalFX/BgPalFX add/mul/alpha/color/hue/invertall/invertblend + sinadd/sinmul/sincolor/sinhue + round reset + Trans alpha + SFF index/palette retention + ACT palettes + RemapPal bank recolor with source-palette filtering + RemapSprite group/index substitution and named presets + collision-box substitution from the destination frame in the same animation |
| C-08 | SuperPause/Pause/EnvShake | Required | Required | Partial | Pause/SuperPause movetime + SuperPause default time/poweradd + EnvShake phase/dir/diradd/mul/decay + Nival oracle |
| C-09 | NotHitBy/HitBy/HitOverride | Required | Required | Partial | Filters incl. wildcard attrs/timer expiry + override slots/keepstate/forceguard |
| C-10 | Width/ScreenBound/PlayerPush | Required | Required | Partial | Width forms + ScreenBound X/Y axis clamps + movecamera X/Y camera tracking + PlayerPush separation |
| C-11 | AssertSpecial flags | Required | Required | Partial | Full mapping + per-tick render flags + NoHardcodedKeys + GlobalNoShadow + TimerFreeze round-clock freeze + NoHitDamage/NoGuardDamage/NoCornerPush/NoFallCount/NoDizzyPointsDamage/NoGuardPointsDamage combat suppression + NoInput + NoLifeBarDisplay/NoPowerBarDisplay/NoGuardBarDisplay + NoJump/NoCrouch/NoStand/NoAirJump |
| C-12 | MakeDust/GameMakeAnim | Required | Required | Partial | MakeDust spacing/pos2 + GameMakeAnim placement tests; oracle pending |
| C-13 | Ikemen-only controllers | N/A | Ikemen | Partial | GuardPointsAdd/GuardPointsSet/GuardBreakSet/DizzyPointsAdd/DizzyPointsSet/DizzySet/ModifyPlayer maxima/attack/defence/ailevel/TargetGuardPointsAdd/TargetDizzyPointsAdd/TargetGuardPointsSet/TargetDizzyPointsSet/GetHitVarSet/RoundTimeAdd/RoundTimeSet/VictoryQuote/ScoreAdd/TargetScoreAdd/RedLifeAdd/RedLifeSet/TargetRedLifeAdd/MapSet/MapAdd/MapReset/ParentMapSet/Add/RootMapSet/Add/TeamMapSet/Add/Height/Camera/ModifyHitDef/ModifyReversalDef/ModifyShadow/ModifyReflection/ExplodBindTime/ModifyStageVar (camera/bounds/zoom + Z camera keys)/ModifyStageBG (action/velocity/delta/start/scale/pos/layerno/spriteno/trans, angle/xangle/yangle/xshear, projection/focallength, and shader/shaderparam/tex1/tex2 references)/ModifyBGCtrl (id/time/value/x/y + color/hue/invert/add/mul, sine variants, and shader/shaderparam/tex1/tex2 references)/TransformSprite (xshear/focallength/projection subset)/TransformClsn (scale/angle/pivot)/OverrideClsn (box replace/add/delete subset)/ShiftInput (U/D/L/R + a-c/x-z/s/d/w/m)/AllPalFX/BgPalFX (add/mul/alpha/color/hue/invertall/invertblend and sine variants)/AssertCommand/AssertInput/AssertAnalogVector/ChangeMovelist/RemapPal/SaveState/LoadState/MatchRestart/SaveFile/LoadFile/PlayBGM/ModifyBGM/RemapSprite/GroundLevelOffset/Depth/DepthBound (z clamping + movecamera Z tracking)/ShaderSet/DisplayToClipboard/AppendToClipboard/ClearClipboard implemented; remaining Ikemen-only controllers are blocked or out of scope: dialogue controllers, and RunState |

## 5. Collision and combat

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| K-01 | Clsn1/Clsn2 transforms | Required | Required | Partial | Mirror/normalize/translate box test + KFM 200 hit flow |
| K-02 | Hit/guard flags and priorities | Required | Required | Partial | Guard flags/M + p1/p2 sprpriority + guard.velocity + F gate; KFM guard oracle |
| K-03 | Hit pause/super pause | Required | Required | Partial | Freeze + air hit + reaction-state selection (5010/5020/5030/5070); attacker/defender pausetime split (O-29); KFM oracle |
| K-04 | Juggle/air hit/fall states | Required | Required | Partial | air.juggle gate + HitFallSet semantics + fall velocity/landing + lethal fall KO + GroundLevelOffset plane/landing/clamp tests |
| K-05 | Throws/target bind | Required | Required | Partial | Throw/bind/drop + Ordan slam + p1/p2 facing/sprpriority application |
| K-06 | Projectile collision | Required | Required | Partial | Hit/guard oracle O-21..O-28 + hits-limit removal; state 3000 upper blast 1px box margin |
| K-07 | Helper collision | Required | Required | Partial | Helper HitDef + helper-owned projectile owner/damage tests; oracle pending |
| K-08 | Corner push/pushboxes | Required | Required | Partial | Ground/air corner-push offsets, threshold, guard + PlayerPush separation |
| K-09 | Guard crush/dizzy/guard points | N/A | Ikemen | Partial | Guard points + GuardPointsAdd/Set + GuardBreakSet + TargetGuardPointsAdd/Set + GetHitVar(guardpoints) + 60-tick regen cooldown/+2 regen + 20-tick guard-crush no-guard window + dizzy points/1.8x hit loss/60-tick stun + DizzyPointsAdd/Set + TargetDizzyPointsAdd/Set + DizzySet + GetHitVar(dizzypoints) + ModifyPlayer maxima + Dizzy triggers; custom guard-break/dizzy states entered from `StateGuardBreakHit`/`StateGuardBreakRecover`/`StateDizzy` constants, with content-driven effects |

## 6. Rounds and game flow

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| R-01 | Intro/Fight/KO/Over | Required | Required | Partial | Phase sequence/control locks + round reset power carry/helper removal |
| R-02 | Timer/KO/draw/best-of-three | Required | Required | Partial | Best-of-N/draw + RoundsWon/LoseKO/LoseTime + timer freeze/clamp |
| R-03 | Win poses/quotes | Required | Required | Partial | Win pose (state 180) entry + RoundNotOver delay + Win/WinKO/WinTime/WinPerfect/WinClutch/DecisiveRound triggers + `[Quotes]` parsing + VictoryQuote selection + `[WinQuote]` display tests |
| R-04 | Turns/team/tag modes | Required | Ikemen | Partial | Team rosters with active/standby members, `TagIn`/`TagOut` swaps with life/power snapshots, `Turns` auto-advance on KO, `Simul` arbitrary roster members with team-aware targeting/inputs/camera/KO/time-over, front-end team-mode/partner selection wired into `Assets`/`SimState`, select.def `single = 1` leader/partner rules, synced roster life, partner lifebars, `assign_team`, and `TeamSize`/`TeamLeader`/`MemberNo`/`Standby` triggers; >2 simultaneous members, select.def-driven default team rules, and tag animations not modeled |
| R-05 | Continue/survival/arcade | Required | Required | Partial | `ArcadeSession` ladder/complete/game-over flow, credit-based continue countdown, arcade/survival scoring with streaks, app ladder progression, a rendered continue/credits/score screen, roster-order opponents, per-opponent select.def stages, survival rotation/menu entry, and a persisted top-five score table with a SCORES screen; random/special ordering and per-opponent music not modeled |
| R-06 | Match restart/round time controllers | Required | Ikemen | Partial | RoundTime/NoKOSlow/NoKO flags + RoundTimeAdd/RoundTimeSet tick controllers + MatchRestart controller + SaveFile/LoadFile + `restart_match` API and R-key reset |

## 7. Rendering

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| V-01 | Sprite/palette rendering | Required | Required | Verified | Screenshots |
| V-02 | Blend modes | Required | Required | Verified | Screenshots |
| V-03 | Stage layers/parallax/tiling | Required | Required | Partial | Layer geometry + BGCtrl vel/pos/visible/anim/sin/cos update tests + `[Scaling]` perspective for fighters + layered depth/scale via `ModifyBGCtrl3D` |
| V-04 | HUD/lifebars/fonts | Required | Required | Partial | Win-icon kind/perfect/cap + combo refresh + delayed fills + guard points/bar rendering with hide-at-zero + red-life bar layer from red_life/life_max + guard-crush drain/window; custom guard-break effects pending |
| V-05 | Explods/projectiles | Required | Required | Partial | Lifetime/removal + edge/window/perspective edges + projectile shear/perspective rendering + projectile hit/guard oracle |
| V-06 | Afterimages | Required | Required | Partial | Ghost sampling cadence/decay + controller-time/alpha tests |
| V-07 | Screenpack/motifs | Required | Required | Partial | Manato Grid title/select/fight, fonts, BGCtrl, portraits, select timer, round sounds, action banner displaytime/lifecycle; VS/storyboards tracked by V-07/F-12 |
| V-08 | Zoom/camera controls | N/A | Ikemen | Partial | Round reset startx/zoom + zoom-anchor + CameraPos/CameraZoom triggers + tension/bounds + Zoom stagebound/camerabound clamping |
| V-09 | Custom shaders | N/A | Ikemen | Partial | `[Shaders]` parsing, `ShaderSet` selection, `Shader()` trigger, WGSL pipeline registry/execution + GLSL fragment execution (explicit or implicitly-declared bindings via GLSL preprocessing, built-in GLSL vertex stage, blend/depth, cached per name, `shaderparam` uniform block, real `pal` palette textures, `tex1`/`tex2` sprite-reference textures, and previous-frame grab textures), wgpu depth buffer, stage `[Scaling]` perspective for fighters/layers/effects/shadows/texts/projectiles/reflections, z-axis collision, and static glTF scene models implemented; clearcoat/transmission/anisotropy/sheen/iridescence/volume/refraction-IOR/dispersion supported; previous-frame screen-space refraction implemented; per-pixel ray-traced refraction and split dispersion rays not modeled; normal/emissive/ORM maps, envmap ambient, multi-clip models, and model-node `ModifyBGCtrl3D` supported |

## 8. Audio

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| A-01 | SND playback | Required | Required | Partial | SND + channels/priority; oracle pending |
| A-02 | Character/effect sounds | Required | Required | Verified | Roster + hit/guard sound events and silent group -1 |
| A-03 | BGM/music | Required | Required | Partial | Symphonia decode + looping mixer + PlayBGM/ModifyBGM controllers + live BgmVar position/length/samplerate feedback + non-loop playback/startposition/freqmul/loopcount and live loop points |
| A-04 | Pan/volume/channels | Required | Required | Partial | Channel priority, pan, master/music volume, tests |
| A-05 | Pause/loop/modify sound | Required | Ikemen | Partial | PlaySnd loop, StopSnd/SndPan/ModifySnd, BGM pause/resume test |

## 9. Input, AI, and netplay

| ID | Area | M.U.G.E.N 1.1 | Ikemen-GO | manato | Verification |
| --- | --- | --- | --- | --- | --- |
| I-01 | Keyboard input | Required | Required | Verified | Window run |
| I-02 | Gamepad input | Required | Required | Partial | gilrs backend, d-pad/stick/buttons mapped, `--backend auto|vulkan|gl|metal|dx12` with adapter logging; hardware matrix pending |
| I-03 | Remapping | Required | Required | Partial | TOML keyboard bindings; gamepad remap pending |
| I-04 | Deterministic CPU AI | Required | Required | Partial | Smoke tests |
| N-01 | Rollback determinism | Required | Required | Partial | SyncTest |
| N-02 | P2P input exchange | Required | Required | Partial | Localhost GGRS loopback test; internet/latency matrix pending |
| N-03 | Spectators/lobby | N/A | Ikemen | Partial | Lobby state (roles, capacity, ready/start, host transfer), bincode protocol incl. Welcome/State snapshots and chunked Snapshot streaming (1 KiB chunks, 256 KiB cap, stale/duplicate/oversize rejection) with host `snapshot_messages` serialization and `SpectatorSnapshotReceiver` restore + checksum verification, LobbyServer routing, a non-blocking UDP relay with localhost tests, spectator frame/lag sync, and a direct continuous `SnapshotBroadcaster`/`SpectatorClient` with app --broadcast/--spectate and a status-line UI; LAN discovery, STUN address discovery, ranked spectator matchmaking, and player-slot join/ready/start coordination plus Starting-lobby GGRS handoff, headless auto-entry, windowed netplay input, lobby-to-window auto-entry, best-effort NAT punching, two-peer and framed multi-peer UDP relays, shared-secret and long-term credential relay auth, GGRS relay-tunneled netplay, and automatic punch-to-relay selection implemented; full STUN/TURN wire protocol not modeled |

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
