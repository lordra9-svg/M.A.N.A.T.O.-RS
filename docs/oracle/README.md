# Ikemen-GO oracle harness

M.A.N.A.T.O.-RS uses Ikemen-GO as its only engine oracle. This directory holds
the **derivation** side of that process: scripts that run Ikemen-GO
black-box and turn its output into committed assertions. Raw logs, traces, and
screenshots are never committed.

## Pinned oracle

| Item | Value |
| --- | --- |
| Engine source | `ikemen-engine/Ikemen-GO` at commit `955a2f2` |
| Runtime data | official `v1.0.0` Linux release (`data/`, `chars/`, `font/`, `external/`) |
| Observed on | 2026-11-08, Linux x86_64 |
| Renderer | OpenGL 4.6 via Mesa, SDL video driver `offscreen` |

The official release binary links a bundled SDL2 without the `offscreen` video
driver, so the harness runs a locally built binary against the release runtime
data. Build it from the pinned source:

```sh
# Go 1.27+ and libxmp headers are required (brew install go libxmp).
cd /path/to/ikemen-go
CGO_ENABLED=1 \
CGO_CFLAGS="-I/home/linuxbrew/.linuxbrew/include" \
CGO_LDFLAGS="-L/home/linuxbrew/.linuxbrew/lib" \
go build -trimpath -o /path/to/ikemen-release/ikemen-local ./src
```

## Running

```sh
export IKEMEN_DIR=/path/to/ikemen-release
python3 tools/oracle/run_round_oracle.py            # all scenarios
python3 tools/oracle/run_round_oracle.py --json     # machine-readable
```

`run_round_oracle.py` starts one headless match per scenario, reads Ikemen's
`-log` table dump from a temporary directory, prints a compact summary, and
deletes the raw log. The harness defaults `SDL_VIDEODRIVER` to `offscreen`,
but respects an inherited value; on a desktop session use
`SDL_VIDEODRIVER=wayland` (or `x11`) when the offscreen GL context is
unavailable. Useful environment:

- `IKEMEN_BIN` overrides the binary path.
- `--keep` keeps raw logs in a temp directory for debugging.
- `--from-logs DIR` parses pre-captured `<scenario>.log` files without running
  the engine (used when no offscreen GL session is available).

If execution is unavailable, run the game manually from a desktop session:

```sh
cd "$IKEMEN_DIR"
SDL_VIDEODRIVER=offscreen LD_LIBRARY_PATH="$PWD/lib:$LD_LIBRARY_PATH" \
  ./ikemen-local -windowed -nosound -nomusic -nojoy \
  -p1 kfm -p2 kfm -p1.ai 0 -p2.ai 0 \
  -p1.life 100 -p2.life 0 -rounds 1 -time 5 -log ko_clutch_10.log
```

`parse_match_log.py MATCH.log --json` converts one dump into JSON.
`run_round_oracle.py --from-logs .` reads a directory of such dumps.

## Per-frame capture

`run_frame_capture.py` installs a Lua debug-info module, runs a scripted
close-range match, and reduces a CSV capture to a compact summary. It uses
Ikemen's public Lua debug API (`loadDebugInfo`, `player`, `changeState`,
`life`, `stateNo`, `posX`, ...), not engine internals.

```sh
export IKEMEN_DIR=/path/to/ikemen-runtime
python3 tools/oracle/run_frame_capture.py --scenario kfm_attack --json
```

The runtime must contain a close-range `stages/oracle.def`; the script copies
`tools/oracle/oracle-stage.def` and `content/stages/training/stage1.sff` into
place automatically. The script temporarily patches the throwaway runtime's
`external/script/main.lua` to load the capture module before the quick-match
path, then restores it.

If the engine cannot be launched from the Python process (for example, no
offscreen GL session), use the shell-native wrapper. It prepares the runtime,
launches the game from the runtime directory, and reduces the capture:

```sh
IKEMEN_DIR=/path/to/ikemen-runtime tools/oracle/run_frame_capture.sh
```

Set `IKEMEN_ORACLE_KEEP=1` to keep the raw CSV and `IKEMEN_ORACLE_TIMEOUT` to
limit the run. Library paths must already include the runtime's SDL/FFmpeg/xmp
dependencies; the wrapper prepends the existing `LD_LIBRARY_PATH` to the
runtime `lib/` directory.

To reduce an existing capture:

```sh
python3 tools/oracle/run_frame_capture.py --from-csv frames.csv --json
```

## Projectile scenarios

`kaen_projectile[_guard]`, `nival_projectile[_guard]`, and
`ordan_projectile[_guard]` fire the character's state-1000 projectile at an
idle or guarding KFM. `kaen_super[_guard]` fires Kaen state 3000's dual blast; `nival_super[_guard]`
covers Nival's paused super blast. Ordan state 3000 is a command grab rather
than a projectile. Assertions O-21..O-28 cover damage, guard damage, get-hit vs guard state, dual-blast
combined damage, and the owner's projectile contact/hit/guard timers.
`kfm_crouch_attack` injects KFM state 400 (`pausetime = 10, 11`) to capture
`HitPauseTime` for an asymmetric pause; O-29 covers the attacker/defender
distinction. `kaen_throw` injects Kaen state 800 to capture target-bind
timing, position, damage, and release (O-30).

## O-20 negative finding

A purpose-built fixture (`tools/oracle/oracle-char/oracle.cns`) forces a
persistent `numhits = 2` HitDef onto an airborne target. Ikemen-GO consumes
the full 15-point juggle budget on the first fall hit but then continues repeat
contacts until KO; it does not reject the same-activation repeats. This is
recorded as negative O-20 in `assertions.md`, not as a rejection assertion.

## Current coverage

Round-end classification (`docs/oracle/assertions.md`, O-01..O-08) and the
KFM light-punch hit flow (O-09..O-12), forced-guard flow (O-13..O-14),
trip/launch flow (O-15..O-16), juggle consumption (O-17..O-19), and the
negative O-20 persistent-HitDef finding:
timer rate, KO vs time-over, perfect, clutch, time-over draws, contact timing,
damage, get-hit state, hit-shake transition, single-hit activation, push
direction, guard state/animation, zero guard damage, guard slide, trip damage, and the
airborne fall transition. Run the guard and launch scenarios with:

```sh
IKEMEN_ORACLE_SCENARIO=kfm_guard tools/oracle/run_frame_capture.sh
IKEMEN_ORACLE_SCENARIO=kfm_launch tools/oracle/run_frame_capture.sh
IKEMEN_ORACLE_SCENARIO=kfm_juggle tools/oracle/run_frame_capture.sh
IKEMEN_ORACLE_SCENARIO=kfm_juggle_exhaust tools/oracle/run_frame_capture.sh
IKEMEN_ORACLE_SCENARIO=kaen_projectile tools/oracle/run_frame_capture.sh
IKEMEN_ORACLE_SCENARIO=nival_projectile tools/oracle/run_frame_capture.sh
IKEMEN_ORACLE_SCENARIO=ordan_projectile_guard tools/oracle/run_frame_capture.sh
IKEMEN_ORACLE_SCENARIO=kaen_super_guard tools/oracle/run_frame_capture.sh
IKEMEN_ORACLE_SCENARIO=nival_super tools/oracle/run_frame_capture.sh
IKEMEN_ORACLE_SCENARIO=nival_super_guard tools/oracle/run_frame_capture.sh
```

The guard scenario uses the public Lua `changeState(150)` debug API to force a
block, so it does not depend on AI decision-making.

Stock-stage rendering (X-05) uses a throwaway runtime copy with the pinned
`Ikemen_GO-v1.0.0-linux.zip` data and stock `mugen-1.1b1` stage files under
unique names. `save/config.ini` sets `GameWidth`/`GameHeight` and
`WindowWidth`/`WindowHeight` to 320x240, points `ScreenshotFolder` at a temp
directory, and adds a `Common.Lua1` hook that calls `screenshot()` 120 ticks
after `roundState() == 2`. Our engine renders the same stage after 165 ticks
with AI 0. Only derived stage-band mean colours are committed; raw PNGs stay
in the temp directory. The render test is
`stock_stage_render_matches_ikemen_oracle` in `manato-render`.

Note: Ikemen's `-rounds` command-line flag means "total rounds to play", not
"rounds needed to win". Use `-rounds 1` and inspect the per-round `Rounds`
table. A drawn round makes the oracle continue with another round even when
`-rounds 1` was requested.

## Clean-room rules

- Observe the engine as a running program; do not read M.U.G.E.N binaries or
  disassemble anything.
- Commit only derived assertions and the scripts that produce them.
- Never commit raw Ikemen logs, screenshots, replays, or extracted content.
