# Save/load and match restart

**Status: APPROVED.** Derived from the MIT-licensed Ikemen-GO source
(`src/state.go`, `src/bytecode.go`).

## 1. Scope

- `SaveState` — snapshot the live simulation.
- `LoadState` — restore the most recent snapshot.
- `MatchRestart` — reset the match to round 1 without reloading assets.
- `SimState::save_snapshot()` / `load_snapshot()` as the host-facing API.

Out of scope for now: `SaveFile`/`LoadFile` disk serialization, `MatchRestart`
DEF/PAL overrides, and asset reloads.

## 2. Semantics

`SaveState` and `LoadState` set a one-tick request flag. Requests are serviced
at the start of the next `tick`, before the first frame of gameplay, matching
Ikemen's frame-loop ordering:

1. Read and clear the request flags.
2. A restart calls `restart_match()` (round 1, wins/history/score cleared,
   bounds restored from the stage, fighters reset via the normal round start).
3. A load replaces the whole simulation with the stored snapshot.
4. A save clones the simulation into the snapshot slot.

A snapshot includes fighters (state, animation, variables, hit state, binds),
projectiles, effects, texts, round and camera state, RNG, stage overrides, and
all other `SimState` fields. Assets are shared through `Arc`, so the snapshot
does not copy sprites or parsed scripts. `LoadState` keeps the snapshot for
repeated loads; loading is deterministic and reproduces the same checksum
trajectory as the original run. Snapshots are excluded from the rollback
checksum.

## 3. Limitations

`MatchRestart` does not accept `stagedef`/`p1def`/`palN` overrides because
assets are loaded once per process. `SaveFile`/`LoadFile` are not modeled; they
need a stable on-disk serialization format.

## 4. Test plan

- Save, advance, load: checksum matches, and the next N ticks reproduce the
  original checksums.
- `SaveState`/`LoadState` controllers applied at the tick boundary.
- `MatchRestart` resets rounds, wins, winner, score, and life.
