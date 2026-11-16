# Arcade, continue, and survival flow

**Status: APPROVED (partial).** Derived from the MIT-licensed Ikemen-GO source
and M.U.G.E.N single-player modes.

## 1. Session model

`manato_sim::arcade::ArcadeSession` tracks a single-player run:

- `ladder: Vec<usize>` — roster indices of the opponents in order.
- `current` — index into `ladder`.
- `phase` — `Fight`, `Continue`, `Complete`, or `GameOver`.
- `credits` — remaining continues.
- `continue_timer` — ticks left on the continue countdown.
- `score`, `streak`, `wins`, `losses`, `continues`.
- `survival` — switches scoring and ladder behavior.

`ArcadeConfig { credits, continue_seconds, survival }` configures the run
(default 3 credits, 10 second countdown, arcade mode).

## 2. Flow

- `win(time_left, perfect, life_left)`:
  - Arcade score: `1000 + time_left * 10`, plus `1000` for a perfect.
  - Survival score: `300 + streak * 100 + life_left * 5`; the session stays on
    the same opponent.
  - Arcade advances `current`; passing the last entry sets `Complete`.
- `lose()`: increments losses and resets the streak. With credits left the
  phase becomes `Continue` and the countdown starts; otherwise `GameOver`.
- `tick()` decrements the continue countdown (60 ticks/second) and ends the
  run when it reaches zero.
- `use_credit()` spends one credit, records the continue, and resumes the same
  match.

## 3. Not modeled

- Front-end screens/dialogue for continue, credits, and score tallies (the
  state machine is available to the host; the window UI currently starts and
  returns from single matches).
- Per-opponent stage/order configuration from `select.def`, survival opponent
  rotation, and high-score persistence.
