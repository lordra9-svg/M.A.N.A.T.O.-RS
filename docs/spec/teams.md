# Team and tag flow

**Status: APPROVED (partial).** Derived from the MIT-licensed Ikemen-GO source
(`src/char.go`, `src/bytecode.go`).

## 1. Model

`SimState.teams[side]` is a `TeamRoster`:

- `members: Vec<TeamMember { char_id, life, red_life, power }>`
- `active: usize` — index of the member currently occupying the side's fighter
  slot.

Only one member per side is simulated at a time. The leader (member 0) is the
initial active fighter; standby members are stat-only roster entries and are
not ticked, drawn, or collided.

`SimState.team_modes[side]` selects `Single`, `Turns`, `Tag`, or `Simul`.

- `Turns` auto-advances to the next living standby member when the active
  member is KO'd; the round only ends when no living member remains.
- `Tag` keeps manual `TagIn`/`TagOut` control.
- `Simul` spawns every roster member after the leader as an active CPU
  fighter at round start, staggered behind the leader. Every member ticks,
  collides, is targetable, renders, and drives the camera. The round ends only
  when every active member of a side is KO'd; time-over compares total team
  life across all roster members. Partners use the configured AI level (at
  least 1); `ai_sides` does not disable them. `assign_team` accepts any roster
  length, so simul sizes beyond the front-end's single partner selection are
  available programmatically. Manual tag control during simul is not modeled.

The title Options screen exposes `P1 TEAM`, `P2 TEAM`, and three partner
slots per side (`P1 PARTNER`, `P1 PARTNER 2`, `P1 PARTNER 3`, and the same for
P2): team mode cycles Single → Turns → Tag → Simul, and each partner slot
cycles through the select.def roster, skipping entries marked `single = 1`. On
match start every selected partner character is loaded into `Assets`, the side
roster becomes `[leader, partner, partner2, partner3]`, and
`SimState.team_modes` receives the chosen modes. Slots whose entry is
single-only or fails to load are skipped. A leader marked `single = 1` in `select.def` forces that side to Single
mode regardless of the options screen, and such entries are never loaded as
partners. Single-mode sides ignore partner selection.

The active member's life/red-life/power are synchronized into the roster each
tick, so standby lifebars and turn selection see current values.

`assign_team(side, &[char_id, ...])` replaces a roster from already-loaded
characters. `start_round` resets every side to its leader and restores the
leader's default life.

## 2. Controllers

- `TagIn`: save the active member's life/red-life/power, rotate `active` to the
  next member, rebuild the fighter slot from that member's character at the
  same position/facing, and restore its stored life/power. `stateno`
  (or `partnerstateno`) selects the incoming state; `ctrl`/`partnerctrl` sets
  the incoming control flag. A provided incoming state also grants tag-in
  invulnerability for that state's first `ChangeAnim` animation (fallback 20
  ticks), so the incoming member cannot be hit while its tag animation plays.
- `TagOut`: `partnerstateno` selects the incoming state as above. When
  `stateno` is also given, the outgoing fighter first enters that state with
  control disabled and the roster swap is delayed until the state's first
  `ChangeAnim` animation completes (falling back to 20 ticks when no readable
  animation exists). This lets the character play a tag-out animation before
  the partner replaces it. Without `stateno`, the swap is immediate. A tag-out
  that completes into an incoming state grants the same tag-in invulnerability
  window.
- Pending tag transitions are part of rollback state and the checksum, and are
  cleared when a round starts or another tag action supersedes them.
- `Turn`: flips facing (unchanged 2D behavior).

## 3. Triggers

- `TeamSize` — roster length (minimum 1).
- `MemberNo` — active member number (1-based).
- `TeamLeader` — player number for single-member teams, otherwise the team
  leader's member number (1).
- `Standby` — always false because standby members are not simulated.

## 4. Rendering

The fallback HUD draws a thin lifebar for every standby member next to the
active fighter's bar, using the synced roster life/life-max values.

## 5. Not modeled

- Front-end selection of more than three partners per side (the roster API and
  simul spawning have no such limit).
- Alternate-character loading beyond the explicit leader/partner slots; rosters are
  assigned through the API (tests and hosts).
- Team-specific round/score rules.
- Tag-out animations longer than the state's first `ChangeAnim` (the swap is
  timed from that animation, not from later state changes).
