# Oracle-derived assertions

Each row is a behavior observed from Ikemen-GO through the harness in
[`README.md`](README.md) and reduced to a testable statement. Raw traces are
not committed. The harness command and the exact life/time inputs are recorded
so a future run can reproduce the observation.

Scenarios (`tools/oracle/run_round_oracle.py`):

| Name | P1 life | P2 life | Round time | Rounds |
| --- | --- | --- | --- | --- |
| `ko_full` | 1000 | 0 | 5 s | 1 |
| `ko_clutch_10` | 100 | 0 | 5 s | 1 |
| `ko_clutch_over` | 101 | 0 | 5 s | 1 |
| `time_over_full` | 1000 | 500 | 1 s | 1 |
| `time_over_draw` | 500 | 500 | 1 s | 1 |

All use `kfm` versus `kfm`, AI level 0, `-rounds 1`, `-nosound -nomusic
-nojoy`, and the pinned runtime in the README.

| # | Assertion | Observation |
| --- | --- | --- |
| O-01 | A round timer lasts 60 ticks per configured second. | `-time 5` reports `Timer = 300`; `-time 1` reports `Timer = 60`. |
| O-02 | A KO win at full life is a perfect KO, not a time-over win. | `ko_full`: `WinKO = true`, `WinTime = false`, `WinPerfect = true`, `WinClutch = false`. |
| O-03 | A time-over win at full life is both time-over and perfect. | `time_over_full`: `WinKO = false`, `WinTime = true`, `WinPerfect = true`, `WinClutch = false`. |
| O-04 | A win at exactly 10% of max life is a clutch win; 10.1% is not. | `ko_clutch_10` (100/1000): `WinClutch = true`; `ko_clutch_over` (101/1000): `WinClutch = false`. The threshold is inclusive `life <= lifeMax * 0.10`. |
| O-05 | Perfect and clutch wins are mutually exclusive. | Every scenario with `WinPerfect = true` reports `WinClutch = false`. |
| O-06 | `WinSide` is a zero-based team index, while the `Wins` table is one-based. | Every scenario reports `WinSide = 0` and `Wins[1] = 1` for a P1 win. |
| O-07 | The round `Timer` in the match dump is elapsed ticks at round end, not remaining ticks. | `ko_full` ends after one tick and reports `Timer = 1`; `time_over_full` runs the full 60-tick round and reports `Timer = 60`. |
| O-08 | Equal life at time over is a draw: no fighter has `Win = true`, `WinSide = -1`, and `Draws = 1`. | `time_over_draw` (500/500, 1 s). The oracle then replays another round because the first was a draw; both rounds report no winner. |
| O-09 | KFM state 200 (light punch) hits an idle opponent 4 ticks after state entry, deals 23 damage, and puts the defender in state 5000. | Per-frame capture: injection frame 209, contact frame 213, `p2_life` 1000 -> 977, `p2_state` 5000. |
| O-10 | One KFM state 200 activation hits only once; the attack does not re-hit after its 8-tick hit pause. | Same capture: `p2_life` has one 23-point drop over the following 25+ ticks while the attacker remains in state 200. |
| O-11 | Ground hit velocity is applied away from the attacker. With P1 facing right and `ground.velocity = -4`, the defender moves right from x=16 to ~31.9. | Same capture: `p2_x` increases from 16.0 to ~31.9 over 20 ticks. |
| O-12 | The defender leaves state 5000 for 5001 on the first tick after the 8-tick hit-shake window, 9 ticks after contact. | Same capture: contact frame 213, `p2_state` 5001 first at frame 222. |
| O-13 | A forced guard contact puts the defender in state 150 with guard animation 140, deals 0 guard damage for KFM's `damage = 23, 0`, and reports `MoveGuarded = 1`, `MoveHit = 0`. | `kfm_guard` capture: contact frame 213, `p2_state` 150, `p2_life` unchanged at 1000. |
| O-14 | The guard slide begins 9 ticks after contact (state 151) and the defender is pushed away ~14.8 pixels over the 14-tick guard window. | Same capture: `p2_state` 151 at frame 222, `p2_x` 16.0 -> 30.8345 by frame 226. |
| O-15 | KFM state 440 (trip) hits an idle opponent around 10 ticks after state entry, deals 72 damage, and enters trip/fall state 5070. | `kfm_launch` capture: injection frame 209, contact frame 219, `p2_life` 1000 -> 928, `p2_state` 5070. |
| O-16 | Trip state 5070 advances to 5071 on contact + 13, then the defender becomes airborne with upward velocity and gravity. | Same capture: state 5071 first at frame 232, `p2_y` -2.0, `p2_vy` -2.0. |
| O-17 | KFM state 440 consumes its Statedef `juggle = 7` from the defender even though the first hit is grounded, leaving 8 of 15 points. | `kfm_juggle` capture: `p2_juggle` 15 -> 8 after the 72-damage trip hit. |
| O-18 | A second airborne hit with KFM state 210 (Statedef `juggle = 4`) consumes 4 more points, enters air-hit state 5020, and deals 38 damage because fall defence multiplies by 1.5. | Same capture: `p2_juggle` 8 -> 4, life 928 -> 890, state 5020. |
| O-19 | A fresh airborne activation is not blocked by the points gate even when its cost exceeds the remaining points; M.U.G.E.N lets an activation juggle an enemy it has not targeted yet. The gate blocks only repeat hits within one activation. | `kfm_juggle_exhaust` capture: state 240 hits while airborne and consumes 5 (`p2_juggle` 8 -> 3). The insufficient-point same-activation case is covered by `juggle_points_gate_air_hits`. |
| O-21 | Kaen state 1000 fires a projectile that contacts an idle KFM 6 ticks after state entry, deals 55 damage, and puts the defender in state 5000. The owner reports projectile contact/hit timers with no guard timer. | `kaen_projectile` capture: injection frame 209, contact frame 215, `p2_life` 1000 -> 945, `p2_state` 5000, `p1_projContact`/`p1_projHit` 0, `p1_projGuarded` -1. |
| O-22 | Guarding the projectile (P2 in guard state 150) contacts 7 ticks after entry, deals 12 guard damage, keeps the defender in guard state 150, and reports the projectile guard timer with no hit timer. | `kaen_projectile_guard` capture: contact frame 216, `p2_life` 1000 -> 988, `p2_state` 150, `p1_projGuarded` 0, `p1_projHit` -1. |
| O-23 | Nival's projectile hits an idle opponent 6 ticks after entry for 45 damage (state 5000), and a guarded contact 7 ticks after entry costs 10 guard damage in state 150. | `nival_projectile[_guard]` captures: 45/state 5000/`projHit` 0; 10/state 150/`projGuarded` 0. |
| O-24 | Ordan's projectile hits an idle opponent 6 ticks after entry for 60 damage (state 5000), and a guarded contact 7 ticks after entry costs 14 guard damage in state 150. | `ordan_projectile[_guard]` captures: 60/state 5000/`projHit` 0; 14/state 150/`projGuarded` 0. |
| O-25 | Kaen state 3000 fires two offset blasts; the low blast hits for 65 and puts the defender in state 5000. The upper blast (`offset = 0,-58`, `velocity = 6,-2`) passes above the idle hurt box and does not connect. | Revalidated `kaen_super` capture: contact +37, `p2_life` 1000 -> 935, `p2_state` 5000, `p1_projHit` 0. The earlier 120-damage observation was an artifact: Kaen's content used non-Projectile parameter names (`pos`, `removetime`), which Ikemen warned about and ignored, so both blasts spawned at y=0. The content now uses `offset`/`projremovetime`, and Ikemen and our engine both report 65. |
| O-26 | Guarding state 3000's blasts costs 15 guard damage; the defender remains in guard state 150 and reports the projectile guard timer with no hit timer. The upper blast passes above the guard hurt box. | `kaen_super_guard` capture: contact frame 246, `p2_life` 1000 -> 985, `p2_state` 150, `p1_projGuarded` 0, `p1_projHit` -1. |
| O-27 | Nival state 3000 opens with a 30-tick SuperPause and then its single blast hits for 90 damage (state 5000). Contact lands 38 ticks after entry, versus ~6 for the plain projectile. | `nival_super` capture: contact frame 247, `p2_life` 1000 -> 910, `p2_state` 5000, `p1_projHit` 0. |
| O-28 | Guarding Nival's super blast costs 20 guard damage and leaves the defender in guard state 150 with the projectile guard timer set. | `nival_super_guard` capture: contact frame 247, `p2_life` 1000 -> 980, `p2_state` 150, `p1_projGuarded` 0, `p1_projHit` -1. |
| O-33 | With `Options.GuardBreak = 1`, a fixture defender with 20 guard points loses 15 per guarded hit whose raw life damage is 10 (`default.lifetoguardpointsmul = 1.5`): 20 -> 5 -> 0. Reaching 0 starts the 20-tick guard-crush window; after its 60-tick cooldown the meter regains +2 on the first regen tick. | `guard_crush_probe` capture: first guard frame 214, `p2_guardPoints` 20 -> 5; second guard frame 234 -> 0; regen tick frame 294 -> 2. Encoded by `guard_and_dizzy_point_rates_match_oracle` and `guard_hits_drain_guard_points`. |
| O-34 | With `Options.Dizzy = 1`, a fixture defender with 100 dizzy points loses 36 per unguarded 20-damage hit (`default.lifetodizzypointsmul = 1.8`): 100 -> 64 -> 28 -> 0, then the 60-tick dizzy stun resets the meter. | `dizzy_probe` capture: hits at frames 214/234/254, `p2_dizzyPoints` 100 -> 64 -> 28 -> 0; meter returns to 100 after the stun. Encoded by `guard_and_dizzy_point_rates_match_oracle` and `dizzy_stun_lasts_sixty_ticks_and_resets`. |
| O-32 | KFM state 200's PowerAdd gives P1 10 power before contact; the hit then adds 16 to the attacker and 13 to the defender using Ikemen's omitted-getpower/givepower defaults. | `kfm_attack` capture: frame 210 `p1_power` 0 -> 10, contact frame 214 `p1_power` 10 -> 26 and `p2_power` 0 -> 13. Encoded by `oracle_kfm_state_200_hits_idle`; no content-defined points involved. |
| O-31 | Kaen state 2100 spawns helper 2100 in state 2101; the helper's normal HitDef contacts 4 ticks after injection for 30 damage and puts the defender in state 5000. The helper self-destroys at time 20. | `kaen_helper` capture: injection frame 209, contact frame 213 (+4), `p2_life` 1000 -> 970, `p2_state` 5000. Encoded by `kaen_helper_hit_matches_oracle`; helper ownership is covered by the existing helper HitDef/projectile tests. |
| O-30 | Kaen state 800 grabs an idle KFM 5 ticks after entry: P1 enters 810, P2 enters 820, and the target is bound at `p1.x + 28`, `p1.y - 40`. At 25 ticks after the grab `TargetLifeAdd` deals 80 damage, and at +27 the target is released into state 5050 while still at the bound position. | `kaen_throw` capture: injection frame 209, grab frame 214, P1 life 1000 / P2 1000 -> 920 at frame 239, P2 state 5050 at frame 241, P2 position (12, -40) with P1 at (-16, 0). Encoded by `kaen_state_800_throw_matches_oracle_flow`. |
| O-29 | `HitPauseTime` reports the attacker's remaining self-pause. KFM state 400 (`pausetime = 10, 11`) reports 10 on the contact frame, then 9, 8, ... 0 on successive frozen ticks. The defender, frozen by its hit shake for the second value, reports 0 throughout. | `kfm_crouch_attack` capture: injection frame 209, contact frame 213, `p1_hitPause` 10 at contact and one lower each frame through 0 at frame 223; `p2_hitPause` 0 for the whole hit. State 200 (`pausetime = 8, 8`) shows the same pattern starting at 8. |
| O-20 | **Negative result:** a persistent `numhits = 2` HitDef does **not** stop repeat hits after the juggle budget reaches 0. The first fall hit consumes all 15 points, then repeat contacts continue until KO (100 hits in the capture). The expected same-activation rejection was not observed with this construct. | `kfm_multihit` capture: `p2_juggle` 15 -> 0 on the first fall hit, then life continues dropping from 990 to 0 with `hits = 100`. Fixture: `tools/oracle/oracle-char/oracle.cns`. |

| X-05a | The stock KFM stage renders as a warm wooden interior: the mean RGB of the stage band (y=45..225, HUD excluded) is (113, 92, 65). | Ikemen-GO v1.0.0 runtime, `955a2f2` build, 320x240, KFM vs KFM AI 0, screenshot 120 ticks after the first Fight tick. Our renderer: (105, 82, 52), same family. |
| X-05b | The stock stage0 renders blue: stage-band mean (16, 44, 152). | Same run. Our renderer: (17, 44, 153). |
| X-05c | The stock stage0-720 renders blue: stage-band mean (34, 61, 159). | Same run. Our renderer: (8, 34, 134). The difference is layer/camera composition, not palette. |
| X-05d | The stock stage1 renders blue-grey: stage-band mean (93, 125, 169). | Same run. Our renderer: (93, 125, 168). |
| X-05e | SFF v2 palette nodes store RGBA bytes; swapping them to BGRA tints stock stages incorrectly. | Before the fix our stage0-720 rendered red-brown and the KFM wall blue, while Ikemen rendered stage0-720 blue and the wall brown. `parse_v2_palettes` now keeps RGBA; the render oracle above fails under the old order. |

## Revalidation (2026-11-22)

The same pinned Ikemen-GO `v1.0.0` runtime (`955a2f2`) was rerun through the
harness on Wayland (the scripts now honor an existing `SDL_VIDEODRIVER`
instead of forcing `offscreen`). Results reproduced the recorded values:

- Round scenarios O-01..O-08: `ko_full` and `ko_clutch_10` are KO/perfect or
  clutch, `ko_clutch_over` is not clutch, `time_over_full` is time-over/perfect,
  `time_over_draw` has `win_side=-1`, and elapsed timers are 1 or 60 ticks.
- `kfm_attack`: contact frame 213 (+4), 23 damage, state 5000 → 5001 after 9
  ticks, push 15.899, power 10 → 26 on the attacker and 13 on the defender.
- `kfm_guard`: guard contact 213, guard state 150 / anim 140, 0 guard damage,
  push 14.834, 14 guard ticks.
- `kfm_launch`: contact +10, 72 damage, state 5070 → 5071 after 13 ticks.
- `kfm_juggle`: second hit 38 damage into state 5020, juggle 15 → 8 → 4.
- `kfm_juggle_exhaust`: third hit 63 damage into state 5000, juggle 8 → 3.
- `kfm_crouch_attack`: attacker `HitPauseTime` 10 at contact, defending side 0.
- `kaen_helper` (O-31): helper HitDef contacts +4 for 30 damage into state 5000.
- `guard_crush_probe` (O-33): guard points 20 -> 5 -> 0 at 15 per 10-damage hit, then +2 after cooldown.
- `dizzy_probe` (O-34): dizzy points 100 -> 64 -> 28 -> 0 at 36 per 20-damage hit.
- `kaen_throw` (O-30): grab at +5, state 820, bind (12,-40), 80 damage at +25, release to 5050 at +27.
- Projectile scenarios O-21..O-28: Kaen 55 damage / 12 guard, Nival 45 / 10,
  Ordan 60 / 14, all at +6 or +7 ticks with state 5000 or 150 and the matching
  projectile contact/hit/guard timers; Kaen super's low blast hits for 65 at
  +37 ticks (the upper offset blast passes overhead; see O-25), with 15 guard
  damage; Nival super hits for 90 / 20 at +38 ticks.

The capture summaries are deterministic across repeated `kfm_guard` runs.
Raw CSVs and screenshots remain in temporary directories.

## Mapping to M.A.N.A.T.O.-RS

| Oracle | Engine |
| --- | --- |
| O-01 | `RoundState::new` uses `round_time * TICKS_PER_SECOND`; covered by `oracle_round_timer_is_sixty_ticks_per_second`. |
| O-02..O-05 | `SimState::record_win` and `WinRecord`; covered by `oracle_round_win_classification`. |
| O-06 | Presentation only; the engine stores wins per side and does not use `WinSide`. |
| O-07 | Not represented; the engine tracks remaining time and does not expose elapsed round ticks. |
| O-08 | `RoundState::winner = None`, `time_over = true`, and no `WinRecord`; covered by `oracle_round_win_classification`. |
| O-09..O-12 | KFM hit flow is encoded by `oracle_kfm_state_200_hits_idle`; the harness is `tools/oracle/run_frame_capture.py` / `.sh`. |
| O-13..O-14 | KFM guard flow is encoded by `oracle_kfm_guard_flow`; run `IKEMEN_ORACLE_SCENARIO=kfm_guard`. |
| O-15..O-16 | KFM trip/launch flow is encoded by `oracle_kfm_launch_flow`; run `IKEMEN_ORACLE_SCENARIO=kfm_launch`. |
| O-17..O-18 | KFM juggle consumption is encoded by `oracle_kfm_juggle_flow`; run `IKEMEN_ORACLE_SCENARIO=kfm_juggle`. |
| O-19 | Fresh-activation bypass and the same-activation gate are encoded in `juggle_allows`; unit test `juggle_points_gate_air_hits`. |
| O-21..O-28 | Encoded by `bundled_projectile_hits`/`bundled_projectile_can_be_guarded`, `nival_projectile_hit_and_guard`, `ordan_projectile_hit_and_guard`, `state_3000_low_blast_hits_for_65`, `state_3000_blasts_are_guarded_for_15`, `nival_super_pauses_then_hits_for_90`, and `nival_super_guard_takes_20`. |
| O-29 | Encoded by `hit_pause_time_reports_self_pause_only`; run `IKEMEN_ORACLE_SCENARIO=kfm_crouch_attack`. |
| O-30 | Encoded by `kaen_state_800_throw_matches_oracle_flow`; run `IKEMEN_ORACLE_SCENARIO=kaen_throw --p1 kaen`. |
| O-31 | Encoded by `kaen_helper_hit_matches_oracle`; run `IKEMEN_ORACLE_SCENARIO=kaen_helper --p1 kaen`. |
| O-32 | Encoded by `oracle_kfm_state_200_hits_idle`; run `IKEMEN_ORACLE_SCENARIO=kfm_attack`. |
| O-33 | Encoded by `guard_and_dizzy_point_rates_match_oracle` / `guard_hits_drain_guard_points`; run `IKEMEN_ORACLE_SCENARIO=guard_crush_probe --p1 oracle --p2 oracle` with `Options.GuardBreak = 1`. |
| O-34 | Encoded by `guard_and_dizzy_point_rates_match_oracle` / `dizzy_stun_lasts_sixty_ticks_and_resets`; run `IKEMEN_ORACLE_SCENARIO=dizzy_probe --p1 oracle --p2 oracle` with `Options.Dizzy = 1`. |
| O-20 | Kept as a documented negative oracle finding; the engine gate remains unit-tested and is not changed to match this anomalous persistent-HitDef construct. |

These assertions remain subject to revision if a later oracle run contradicts
them. The tests that encode them live in `crates/manato-sim/tests/smoke.rs`.
