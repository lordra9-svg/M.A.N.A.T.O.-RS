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

## 3. App integration

Selecting **Arcade** or **Survival** builds a ladder from the roster (every
entry except the chosen player), and `start_match` uses `current_opponent()`.
Survival rotates to the next ladder opponent after each win and keeps the
accumulated score/streak; arcade advances until the ladder is complete. When a match ends
(after the result delay) the app records `win`/`lose`:

- Win: the next ladder opponent starts; completing the ladder returns to the
  title.
- Loss with credits: the continue screen shows the countdown, credits, and
  score. START (Enter/Space/J) spends a credit and restarts the same opponent;
  the countdown expiring or Esc returns to the title.
- Loss without credits: returns to the title.

The best score is read from `<mugen root>/manato_highscore.dat` when a session
starts and written when the run returns to the title. The continue screen and
session track the current score; the file persists it across runs.

## 4. Not modeled

- Per-opponent stage/order configuration beyond roster order, and a dedicated
  high-score table. Survival now has a menu entry and rotates opponents.
