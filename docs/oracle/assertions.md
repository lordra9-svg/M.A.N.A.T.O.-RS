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
| O-20 | **Negative result:** a persistent `numhits = 2` HitDef does **not** stop repeat hits after the juggle budget reaches 0. The first fall hit consumes all 15 points, then repeat contacts continue until KO (100 hits in the capture). The expected same-activation rejection was not observed with this construct. | `kfm_multihit` capture: `p2_juggle` 15 -> 0 on the first fall hit, then life continues dropping from 990 to 0 with `hits = 100`. Fixture: `tools/oracle/oracle-char/oracle.cns`. |

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
| O-21..O-24 | Encoded by `bundled_projectile_hits`/`bundled_projectile_can_be_guarded` and `nival_projectile_hit_and_guard`/`ordan_projectile_hit_and_guard`; run `IKEMEN_ORACLE_SCENARIO=<char>_projectile[_guard]`. |
| O-20 | Kept as a documented negative oracle finding; the engine gate remains unit-tested and is not changed to match this anomalous persistent-HitDef construct. |

These assertions remain subject to revision if a later oracle run contradicts
them. The tests that encode them live in `crates/manato-sim/tests/smoke.rs`.
