# Rounds and game-flow specification

**Status: APPROVED / FROZEN.** Derived from the MIT-licensed Ikemen-GO
source (`src/system.go`, `src/char.go`), public M.U.G.E.N documentation, and
observed content.

## 1. Scope

Define the match lifecycle around the simulation:

- Round phases and transitions
- Intro, Fight, KO, and Over timing
- Round timer, KO, time-over, and draw handling
- Best-of-N rounds and match completion
- Win poses and match-end display
- Round-time and match-state triggers
- Round reset and restart
- KO slowdown, pause, and control locks

Out of scope for this spec: arcade/continue menus, survival, team/tag modes,
and screenpack storyboards.

## 2. Round phases

```
Intro -> Fight -> Ko -> (next round) Intro
                     \-> Over (match end)
```

| Phase | `RoundState` | Meaning |
| --- | --- | --- |
| `Intro` | 0 | Round announcement and player intros |
| `Fight` | 2 | Normal gameplay and round timer |
| `Ko` | 3 | KO/time-over reaction and win poses |
| `Over` | 4 | Match complete |

`RoundState` 1 (player intros) is folded into `Intro` for the first pass and is
reserved for a later extension. `RoundState` 4 is reported only once the match
is decided and the outro has finished.

## 3. Intro

- `phase_timer` starts at 40 ticks.
- Players are placed at the stage start positions with start facing.
- Player `state 0` and `ctrl` are applied, but control is locked.
- The HUD shows `Round N` while the intro runs.
- When `phase_timer` reaches 0, the phase becomes `Fight`, the display clears,
  and control unlocks.

## 4. Fight

Each tick:

1. Decrement the round timer if it is positive.
2. Detect KO when any player life is 0.
3. If both are dead, the round is a draw.
4. If exactly one is dead, the other wins the round.
5. If the timer reaches 0 with no KO, compare remaining life.
6. Equal life is a draw; otherwise the higher life wins.

On round end:

- Set `winner`.
- Enter `Ko` with a 120-tick `phase_timer`.
- Display `<Name> Wins`, `Time Over - <Name> Wins`, or `Draw Game`.
- Award one round win to the winner.
- Award the winner 200 power (capped).
- KO'd players enter their KO/fall states through normal combat rules.

### 4.1 Draw rounds

- A draw awards no round win.
- The round number still advances.
- `MatchOver` stays false unless the match win target or round cap is reached.

## 5. KO and outro

- During `Ko`, gameplay input is locked and the phase timer runs.
- Hit pause, super pause, and effects continue to update.
- Win poses: if the winning character has state `180` (or the configured win
  state), it is entered when the KO phase begins.
- `RoundNotOver` may delay the transition to the next round or match end.
- When the phase timer expires:
  - If a player has reached `rounds_to_win`, or the round cap is reached, the
    match becomes `Over`.
  - Otherwise increment the round number and start the next round.

## 6. Match completion

- Match winner: the player with more round wins.
- Equal wins after the round cap is a draw.
- `Over` sets `finished = true`.
- The display reads `<Name> Wins the Match` or `Draw Game`.
- The match does not restart automatically; the host may request a restart.

## 7. Round reset

`start_round` resets per-round state for both players:

- Position, velocity, facing, life, and animation to state 0.
- Hit, attack, hitdef, target, guard, afterimage, palette, and helper state.
- Helpers are removed.
- Power is carried across rounds, capped at the maximum.
- `round.phase = Intro`, `phase_timer = 40`, `timer = round_time * 60`.
- Stage background state is reset when `[StageInfo] resetbg = 1`.
- Camera returns to the stage start.

Round wins and the round number persist across rounds.

## 8. Triggers

Round/match triggers must reflect this state:

| Trigger | Meaning |
| --- | --- |
| `RoundNo` | Current round number, starting at 1 |
| `RoundsExisted` | Completed rounds |
| `RoundState` | Phase code above |
| `MatchOver` | Match is decided |
| `RoundTime` | Remaining round seconds |
| `Time` | Ticks since the current state was entered |
| `Winner` | 1 if this player won the round |
| `Loser` | 1 if this player lost the round |
| `Win` | This player won the round (false for helpers) |
| `WinKO` | Won by KO |
| `WinTime` | Won on time |
| `WinPerfect` | Won with full life |
| `WinClutch` | Won with life at or below the clutch threshold |
| `DecisiveRound` | This side is one round win from the match |
| `ConsecutiveWins` | Consecutive match sweeps (Ikemen-GO); 0 for helpers |
| `FightTime` | Ticks since the match started (Ikemen-GO); not reset between rounds |
| `TeamSide` | 1 for left, 2 for right |
| `DrawGame` | The current round is a draw |
| `Lose` | This player lost the round |
| `LoseKO` | This player lost by KO |
| `LoseTime` | This player lost on time |
| `RoundNo`-based AI/level triggers | Use the current round |

`Winner`/`Loser` are only meaningful after a round result is set.

## 9. Round-time and match-state controllers

- `RoundTimeAdd` and `RoundTimeSet` change the internal tick timer, clamped to
  `0..round_time * TICKS_PER_SECOND`. Values are in game ticks, matching
  Ikemen-GO's equivalent controllers; `RoundTime`/`TimeRemaining` still report
  whole seconds.
- `AssertSpecial` flags `NoKOSlow`, `NoKOSnd`, `NoKo`, `NoMusic`, and
  `NoBarDisplay` are honored by the round flow and renderer where applicable.
- `RoundNotOver` delays the next phase while asserted.
- `RoundState` changes are observable by `[Statedef -2]` and `[Statedef -3]`.

## 10. Control locks and slowdown

- `Intro` and `Ko` lock normal input and state `-1` command transitions.
- Special states `-3`, `-2`, and `-1` still run.
- `NoKOSlow` disables the KO slowdown; otherwise KO runs at a reduced update
  cadence for its first frames.
- Super pause and hit pause do not stop the round phase timer.

### 10.1 Intro/Outro phase triggers

The fight-screen motif's separate timing windows are not modeled. The
Ikemen-only `IntroState`/`OutroState`, `FightScreenState(...)`, and
`GameVar(...)` triggers therefore derive from the round phase:

- `IntroState`: `0` outside `Intro`; inside `Intro`, the remaining phase
  timer splits into `1` (pre-intro, > 30 ticks), `2` (player intros, > 10),
  `3` (round announcement, > 5), and `4` (fight called, > 0).
- `OutroState`: `0` in `Intro`/`Fight`; `4` in `Ko` when a winner exists
  (win states), `3` in `Ko` with no winner (round lost, no win state), and
  `5` in `Over` (match over).
- `FightScreenState(field)`: `rounddisplay` is true in `Intro`,
  `fightdisplay` in `Fight`, `kodisplay` in `Ko`, and `windisplay` in
  `Over`; other fields are false.
- `GameVar(field)`: `introtime` is the remaining intro ticks, `outrotime`
  the elapsed KO ticks (`KO_PHASE_TICKS` once the match is over),
  `pausetime`/`superpausetime` the shared pause timer (`Pause` and
  `SuperPause` use one timer here), and `slowtime` the remaining KO
  slowdown. `persistlife`, `persistmusic`, `persistrounds`, and `hidebars`
  read false because persisted-match options and bar hiding are not
  implemented; unknown fields read `0`.

## 11. Error handling

- Missing win/KO states fall back to leaving the fighter in place.
- Invalid round/win counts clamp to at least 1.
- A round that cannot resolve (both sides inactive) is a draw.
- No panics on missing characters, stages, or helpers.

## 12. Test plan

- Intro duration and control lock.
- Fight timer countdown and time-over winner selection.
- KO win, double KO draw, and time-over draw.
- Best-of-three match completion and draw match.
- Power carry-over and round reset.
- `RoundNo`, `RoundState`, `MatchOver`, `Winner`, `Loser`, `RoundTime` triggers.
- Win pose entry when present.
- `RoundNotOver` delay.
- Clean rollback determinism across round transitions.

## 13. Sources

- Ikemen-GO `src/system.go` (MIT), especially `roundState`, `introState`,
  `outroState`, and `stepRoundState`.
- Public M.U.G.E.N round and `RoundState` documentation.
- Observed match flow with the bundled roster and stock characters.
