# Development and porting guide

This document explains how M.A.N.A.T.O.-RS is built, verified, and extended, so
that another person or team can produce their own independent version. It
assumes no prior knowledge of this repository. The authoritative compatibility
targets live in [`../COMPATIBILITY.md`](../COMPATIBILITY.md); the design is in
[`../ARCHITECTURE.md`](../ARCHITECTURE.md); the clean-room rules are in
[`../CLEANROOM_SPEC.md`](../CLEANROOM_SPEC.md).

> M.A.N.A.T.O.-RS is an independent MIT-licensed project. It is not affiliated
> with Elecbyte. It contains no M.U.G.E.N binaries or stock assets.

## 1. What you need

| Requirement | Why |
| --- | --- |
| Rust 1.84+ and Cargo | Workspace builds and tests |
| A `wgpu` GPU backend (Vulkan by default) | Windowed rendering; OpenGL via `--backend gl` |
| PulseAudio or PipeWire's Pulse server | Default audio output (`pa_simple` loaded at runtime) |
| `libudev` development headers on Linux | Optional gamepad support (`--features gamepad`, default) |
| `libxmp` headers, Go 1.27+ | Only if you rebuild the Ikemen-GO oracle runtime |
| `mingw-w64`, musl cross toolchain | Only for release cross-builds |

On Fedora the udev package is `systemd-devel`; on Debian/Ubuntu it is
`libudev-dev`. When `pkg-config` cannot find udev, point it at the `.pc`
directory explicitly:

```sh
export PKG_CONFIG_PATH=/home/linuxbrew/.linuxbrew/Cellar/systemd/262.2/lib/pkgconfig
```

Some systems have a small `/tmp` quota. Keep compiler and test temporaries in
the repository or cache:

```sh
export TMPDIR=/var/home/Lordra/.cache/manato-tmp
```

## 2. First build and run

```sh
git clone https://github.com/lordra9-svg/M.A.N.A.T.O.-RS.git
cd M.A.N.A.T.O.-RS

cargo build --workspace
cargo test --workspace
```

The repository ships three original characters, a stage, and a screenpack, so
no external content is required:

```sh
# Headless: 600 ticks, then write the final 320x240 frame.
cargo run -q -p manato-app -- --frames 600 --screenshot out.png

# Windowed Kaen vs Nival.
cargo run -q -p manato-app

# Front end: title -> select -> fight -> result.
cargo run -q -p manato-app -- --menu

# Deterministic rollback check.
cargo run -q -p manato-app -- --synctest 600
```

To load stock M.U.G.E.N content, set `MUGEN_ROOT` or pass `--root` and name the
DEF files:

```sh
MUGEN_ROOT=/path/to/mugen \
  cargo run -q -p manato-app -- \
  --p1 mugen-1.1b1/chars/kfm/kfm.def \
  --p2 content/chars/nival/nival.def \
  --stage mugen-1.1b1/stages/stage0.def
```

`manato.toml.example` documents the TOML config format. Command-line flags
override config values.

## 3. Repository map

```
crates/
  manato-core      math, input bitmask, RNG, state hashing
  manato-formats   DEF, CNS, CMD, AIR, SFF v1/v2, SND, stage, screenpack
  manato-sim       deterministic state machine, combat, rounds, AI
  manato-net       GGRS config, sync test, P2P, relays, lobby
  manato-render    wgpu atlas, scene/HUD, screenpack front end
  manato-audio     event mixer, Pulse/cpal backends, BGM
  manato-app       CLI, window loop, input, headless runner
content/           original MIT characters, stage, effects, screenpack
docs/spec/         frozen subsystem specifications
docs/oracle/       derived oracle assertions and harness notes
tools/             asset generators, oracle scripts, release builder
COMPATIBILITY.md   the M.U.G.E.N 1.1 / Ikemen-GO work queue
```

The dependency direction is deliberate: `manato-sim` does not know about GPUs,
windows, or audio devices. It consumes `[PlayerInput; 2]` and emits state plus
`Effect`/`SoundEvent` lists. That boundary is what makes snapshots cheap and
rollback deterministic.

### Netplay and GGRS

[GGRS](https://github.com/gschup/ggrs) **0.13** provides rollback session
management and transport abstraction; it does not contain game logic.
`manato-net` owns the integration:

| Piece | Contract |
| --- | --- |
| `ManatoConfig` | Implements `ggrs::Config` with `Input = PlayerInput` and `State = SimState` |
| Save/load | Clones `SimState`; assets remain shared through `Arc` |
| Input delay | Two frames |
| `PredictionThreshold` | Treated as "skip this frame", not a fatal error |
| `SyncTest` | Runs the real game through save/load/rollback with checksums |
| `Netplay` | Two-player P2P wrapper with optional relay transport |
| `RelayGgrsSocket` | Implements `ggrs::NonBlockingSocket<SocketAddr>` over the framed relay |

GGRS itself persists via `bincode` and the `instant`/`paste` crates; keep that in
mind when updating the dependency tree. The compatibility rows are N-01
(rollback determinism), N-02 (P2P input exchange), and N-03 (spectators/lobby).
Protocol details, handoff messages, and STUN/TURN behavior are specified in
[`spec/lobby.md`](spec/lobby.md).

GGRS netplay is plaintext UDP with no peer authentication or anti-cheat. The
optional relay join credentials authenticate registration only, not the ensuing
session. Treat direct netplay and relays as trusted paths and do not expose the
spectator or relay ports to untrusted networks without a firewall or VPN.

## 4. Choose work from the compatibility matrix

`COMPATIBILITY.md` is the project plan, not a marketing table. Every row names
the M.U.G.E.N requirement, the Ikemen-GO requirement, the implementation
status, and the verification evidence.

| Status | Meaning |
| --- | --- |
| `Verified` | Implemented and covered by an approved test |
| `Partial` | Implemented but incomplete or not yet verified |
| `Missing` | Not implemented |
| `N/A` | Deliberately out of scope |

Work only on a row whose acceptance test you can state in advance. If the row
is in a new subsystem, write a spec first. Never mark a row `Verified` because
code exists; mark it when the named evidence exists and passes.

## 5. Spec-first procedure

Every subsystem follows the same six steps. `docs/spec/` is the record of that
process.

1. **Audit.** Identify the subsystem, its files, and any existing partial code.
   Decide whether the current code is clean-room acceptable. Record tainted or
   unknown code explicitly rather than silently rewriting it.
2. **Specify.** Write `docs/spec/<subsystem>.md` using allowed sources only:
   public M.U.G.E.N format/engine documentation, the MIT-licensed Ikemen-GO
   source and documentation, stock/user content as input data, and black-box
   oracle observations.
3. **Freeze.** Mark the spec `APPROVED`. Add or update the corresponding
   `COMPATIBILITY.md` rows and write down the exact acceptance evidence.
4. **Implement.** Build from the frozen spec. Keep the simulation free of
   wall-clock time, unseeded randomness, hash-map iteration order, and asset
   mutation. Unknown controllers/triggers must be ignored with a log message,
   never panic.
5. **Verify.** Add unit, content, and oracle tests as appropriate. Run both
   content roots (Section 8). If the change touches serialized state, bump the
   disk snapshot version (Section 10).
6. **Record.** Update `PROVENANCE.md` and the matrix's Verification column.
   Commit the spec, code, tests, and docs together.

### The clean-room rules in one paragraph

Ikemen-GO is the only runtime oracle. Do not read, search, decompile, or
disassemble M.U.G.E.N binaries, and do not use derived reverse-engineering
projects. Do not copy or mechanically transliterate Ikemen-GO code. Commit
only derived assertions and the scripts that produce them; never commit raw
oracle logs, traces, screenshots, or extracted assets. The same contributor
may write the spec and the implementation, so this is a spec-first independent
reimplementation, not a legally separated clean room — record that limitation
rather than hiding it.

## 6. Adding or changing a subsystem

A typical increment touches five places:

| Place | Change |
| --- | --- |
| `docs/spec/<name>.md` | Frozen behavior, edge cases, source citations |
| `crates/manato-formats` or `crates/manato-sim` | Implementation |
| `crates/*/tests` | Deterministic tests, including a negative test where useful |
| `COMPATIBILITY.md` | Row status and the exact evidence |
| `docs/oracle/assertions.md` | New `O-xx`/`X-05x` assertion, if oracle-derived |

For a new trigger, controller, or HitDef field, also check:

- CNS/CMD parser and trigger AST (`manato-formats`)
- Controller execution and state defaults (`manato-sim/controller.rs`)
- Snapshot/rollback: the field must live in `SimState` and serialize
- AI and replay paths, which must see the same behavior
- Renderer or audio only if the feature emits visible/audible state

For a new content format field, prefer tolerant parsing with a documented
default over rejecting the file. Stock content is often missing optional keys.

## 7. Building your own content

`content/chars/kaen/` is the smallest working character and the recommended
template. A character DEF needs at least:

```ini
[Info]
name = My Character
displayname = My Character

[Files]
cmd     = mychar.cmd
cns     = mychar.cns
stcommon = common.cns
sprite  = mychar.sff
anim    = mychar.air
sound   = mychar.snd

[Size]
xscale = 1
yscale = 1
ground.back = 15
ground.front = 16
air.back = 12
air.front = 12
height = 60
attack.dist = 160
```

`localcoord = 320,240` keeps the classic coordinate system. The engine merges
`stcommon`, `cns`, `st`, `st0`..`stN` in natural numeric order, then the CMD.
That ordering matters for characters that put constants in `st1`.

Asset generation is scripted and reproducible:

```sh
python3 tools/gen_assets.py      # characters, stage, fightfx, common sounds
python3 tools/gen_sfx.py         # character sound banks
python3 tools/gen_screenpack.py  # Manato Grid screenpack and font
```

Do not add stock M.U.G.E.N content to a derived distribution. If you ship your
own characters/stages, keep their licenses and attribution intact, and keep
the repository's `LICENSE`, `PROVENANCE.md`, and `content/LICENSE` accurate.

## 8. Test and verification gates

Run these before every commit that claims a finished increment.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Then run the stock-content root as well:

```sh
MUGEN_ROOT=/path/to/mugen cargo test --workspace
```

Stock tests skip when no stock root is configured, so a green default run does
not prove stock compatibility. Both roots must pass.

Useful targeted commands:

```sh
# One oracle-encoded behavior.
cargo test -p manato-sim --test smoke oracle_kfm_state_200_hits_idle

# Determinism through GGRS save/load/rollback.
cargo run -q -p manato-app -- --synctest 600

# Renderer-only checks (includes the stock-stage oracle when content exists).
cargo test -p manato-render
```

If a `manato-sim` test behaves inconsistently after a large refactor, clear the
stale package artifacts before assuming a logic bug:

```sh
cargo clean -p manato-sim
```

### Security checks

Run the advisory scan whenever dependencies change:

```sh
cargo install cargo-audit --locked   # once
cargo audit
```

The current audit, threat model, fixed findings, and accepted residual risks
are in [`SECURITY_AUDIT.md`](SECURITY_AUDIT.md). The important deployment
constraints are:

- Netplay, relays, and spectator traffic are plaintext and unauthenticated;
  treat them as trusted-network features.
- `SaveFile`/`LoadFile` are sandboxed to the character directory, but symlinks
  inside that directory are not resolved specially.
- Local content is trusted. Do not load arbitrary internet content without an
  external sandbox.
- Lua storyboards and arcade paths are library-restricted and memory-capped,
  but they are content, not a security boundary for hostile native code.

## 9. Oracle harness

The harness lives in `tools/oracle/`; the derived assertions live in
`docs/oracle/assertions.md`; the procedure and pinned runtime are in
`docs/oracle/README.md`.

Summary of the workflow:

1. Pin Ikemen-GO source at the recorded commit and build a local binary against
   the official runtime data.
2. Run a deterministic scenario (`run_round_oracle.py`,
   `run_frame_capture.py`, or `run_frame_capture.sh`).
3. Reduce the output to a compact, committable assertion with exact inputs and
   observed values. Raw CSVs, logs, and screenshots stay in a temp directory.
4. Encode the assertion in a deterministic Rust test. Name it after the
   behavior, not the oracle run.
5. Revalidate periodically and record the date in `assertions.md` and the
   matrix.

```sh
export IKEMEN_DIR=/path/to/ikemen-runtime
python3 tools/oracle/run_frame_capture.py --scenario kfm_attack --json
IKEMEN_ORACLE_SCENARIO=kfm_guard tools/oracle/run_frame_capture.sh
```

On a desktop session use `SDL_VIDEODRIVER=wayland` (or `x11`) when the
offscreen GL context is unavailable. The scripts honor an inherited
`SDL_VIDEODRIVER`.

An oracle result is not a specification until it is written as a behavioral
assertion **and** a test. Do not generalize one capture into a rule the
observation does not support; record negative results as negative results.

## 10. Determinism and snapshots

- The simulation runs at a fixed 60 Hz tick.
- All randomness comes from a seeded `Rng` inside `SimState`.
- Assets are immutable and shared through `Arc`; snapshots copy only mutable
  state.
- `StateHasher` produces the rollback checksum. Snapshots themselves are
  excluded from the checksum.
- Disk snapshots (`SaveFile`/`LoadFile`) store `(format_version, asset_id,
  SimState)` with `bincode`. The format version is a single integer, currently
  **34** (see `docs/spec/save.md`). Bump it for any serialized struct change.
  Files larger than 64 MiB are rejected and relative content paths must stay
  inside the character directory.

## 11. Release procedure

Releases are Linux glibc, Linux static musl, and Windows x86_64. The musl build
is static but still loads host Vulkan drivers, so the glibc build is the
recommended Linux download.

```sh
TMPDIR=/var/home/Lordra/.cache/manato-tmp ./tools/build-release.sh
```

The script builds all three targets and copies `LICENSE` and `README.md` into
`dist/`. It requires the Windows linker and a musl C toolchain; see the script
header for the expected environment variables (`MUSL_BIN`,
`CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER`, `PKG_CONFIG_PATH`).

Publish only after the full gate in Section 8 passes:

```sh
git add -A
git commit -m "Describe the increment"
git push origin main

gh release create vX.Y.Z --target main \
  --title "M.A.N.A.T.O.-RS vX.Y.Z" \
  --notes "What changed, what was verified, and what remains." \
  dist/manato-linux-x86_64 \
  dist/manato-linux-x86_64-musl \
  dist/manato-windows-x86_64.exe \
  dist/LICENSE dist/README.md
```

The release notes must state the verification evidence and any remaining
`Partial`/blocked rows. Do not imply that a blocked row is verified.

## 12. Making your own version

1. Fork the repository and keep the MIT license and attribution.
2. Choose a new name and remove any implication of affiliation with Elecbyte,
   M.U.G.E.N, or this project.
3. Replace or regenerate the bundled content with your own MIT-compatible
   assets. Keep `content/README.md` and `content/LICENSE` truthful.
4. Rewrite `README.md`, `PROVENANCE.md`, and release notes for your project.
   Keep the clean-room constraints if you continue compatibility work.
5. Keep `COMPATIBILITY.md` as a work queue. If you reduce scope, mark rows
   `N/A` explicitly rather than deleting the evidence trail.
6. Re-run both content roots, clippy, and fmt on your own hardware before
   publishing; renderer and gamepad behavior are hardware-dependent.
7. Publish your own binaries and source. Do not redistribute stock M.U.G.E.N
   characters, stages, sounds, or screenpacks.

## 13. Troubleshooting

| Symptom | Likely cause and fix |
| --- | --- |
| `pkg-config` cannot find `libudev` | Set `PKG_CONFIG_PATH` to the directory containing `libudev.pc` |
| Linker runs out of disk or `/tmp` space | Set `TMPDIR` to a larger local directory |
| `No such file` for content | Relative paths resolve against `--root`, not the shell's CWD |
| Window is black or the backend fails | Try `--backend gl`; check that a Vulkan/GL driver is installed |
| No audio | Pulse/PipeWire server missing; the engine stays silent by design |
| Stock tests silently pass without stock content | They skip; set `MUGEN_ROOT` and re-run |
| Netplay desyncs on one machine only | Check determinism rules and run `--synctest`; report platform and content |
| Release script cannot find the Windows linker | Install `mingw-w64` and ensure `x86_64-w64-mingw32-gcc` is on `PATH` |
| Release script cannot find a musl compiler | Set `MUSL_BIN` or install `musl-tools` |

Hardware-only matrices (a real internet/router/NAT run, additional gamepads,
audio-device playback) cannot be closed by unit tests. Record them as
unexecuted rather than inferring success.
