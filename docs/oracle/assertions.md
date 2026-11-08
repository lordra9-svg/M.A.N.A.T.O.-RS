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

## Mapping to M.A.N.A.T.O.-RS

| Oracle | Engine |
| --- | --- |
| O-01 | `RoundState::new` uses `round_time * TICKS_PER_SECOND`; covered by `oracle_round_timer_is_sixty_ticks_per_second`. |
| O-02..O-05 | `SimState::record_win` and `WinRecord`; covered by `oracle_round_win_classification`. |
| O-06 | Presentation only; the engine stores wins per side and does not use `WinSide`. |
| O-07 | Not represented; the engine tracks remaining time and does not expose elapsed round ticks. |
| O-08 | `RoundState::winner = None`, `time_over = true`, and no `WinRecord`; covered by `oracle_round_win_classification`. |

These assertions remain subject to revision if a later oracle run contradicts
them. The tests that encode them live in `crates/manato-sim/tests/smoke.rs`.
