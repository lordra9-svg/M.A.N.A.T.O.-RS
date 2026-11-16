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
`Turns` auto-advances to the next living standby member when the active member
is KO'd, and the round only ends when no living member remains. `Tag` keeps
manual `TagIn`/`TagOut` control. `Simul` is declared for content compatibility
but not simulated (one active fighter per side).

The active member's life/red-life/power are synchronized into the roster each
tick, so standby lifebars and turn selection see current values.

`assign_team(side, &[char_id, ...])` replaces a roster from already-loaded
characters. `start_round` resets every side to its leader and restores the
leader's default life.

## 2. Controllers

- `TagOut` / `TagIn`: save the active member's life/red-life/power, rotate
  `active` to the next member, rebuild the fighter slot from that member's
  character at the same position/facing, and restore its stored life/power.
  `stateno`/`partnerstateno` selects the incoming state (defaults to state 0);
  `ctrl`/`partnerctrl` sets the incoming control flag.
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

- Simultaneous (simul) teams: two or more fighters active per side.
- Alternate-character loading from `select.def` team configuration; rosters are
  assigned through the API (tests and hosts).
- Partner tag animations, tag-in invulnerability windows, and team-specific
  round/score rules.
