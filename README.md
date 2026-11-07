# M.A.N.A.T.O.-RS

An independent, MIT-licensed reimplementation of the M.U.G.E.N fighting-game
engine in Rust, built on `winit` + `wgpu` with GGRS rollback netcode.

> Not affiliated with Elecbyte. M.U.G.E.N is a trademark of Elecbyte.
> This repository contains no M.U.G.E.N binaries or game assets.

The current milestone is a playable vertical slice:

- Three original characters: **Kaen**, **Nival**, and **Ordan**
- The bundled **Training Stage**
- Stock M.U.G.E.N characters can still be loaded by pointing `--root` at a
  stock installation
- Life/power/timer HUD, rounds, KO flow, hit/guard sparks, projectiles, sound events, CPU opponent
- Deterministic simulation verified through GGRS rollback (`--synctest`)

All bundled content — the three characters, their common state file, the
procedural hit/guard effects, the common fight sounds, and the Training Grid
stage — is MIT licensed and lives in [`content/`](content/). No stock
M.U.G.E.N assets are included.

## Quick start

Requirements:

- Rust 1.84 or newer
- A GPU/backend supported by `wgpu` (Vulkan on Linux is what this was developed
  against)
- PulseAudio or PipeWire's Pulse server for audio output; the engine falls back
  to silence when neither is available
- Optional: ALSA development headers only if you enable the `cpal` backend

The repository bundles three original characters (Kaen, Nival, and Ordan) and
their training stage, so it runs out of the box:

```sh
# Headless: run 600 ticks and save the final 320x240 frame.
cargo run -q -p manato-app -- --frames 600 --screenshot out.png

# Windowed play: Kaen vs Nival.
cargo run -q -p manato-app

# Rollback determinism check.
cargo run -q -p manato-app -- --synctest 600
```

To use a stock M.U.G.E.N installation instead, point `MUGEN_ROOT` or `--root`
at it and select characters explicitly:

```sh
export MUGEN_ROOT=/path/to/mugen
cargo run -q -p manato-app --   --p1 mugen-1.1b1/chars/kfm/kfm.def   --p2 content/chars/nival/nival.def   --stage content/stages/training/stage1.def
```

The window opens at `320x240 * scale` (default scale 3). `F12` saves a
screenshot, `Esc` quits.

### Controls

| Player | Movement | X / Y / Z | A / B / C | Start |
| --- | --- | --- | --- | --- |
| P1 | `WASD` | `J` / `K` / `L` | `U` / `I` / `O` | `Enter` |
| P2 | Arrow keys | Numpad `1` / `2` / `3` | Numpad `4` / `5` / `6` | Numpad `0` |

P2 is CPU-controlled by default. Use `--ai 0` for local two-player.

## Command line

```
manato [OPTIONS]

Content:
  --config <toml>     Load defaults from a TOML config file
  --root <dir>        Content root (default $MUGEN_ROOT or the repo root)
  --p1 <def>          Player 1 character definition
  --p2 <def>          Player 2 character definition
  --stage <def>       Stage definition
  --ai <level>        CPU level for slot 2, 0 = human (default 6)
  --rounds <n>        Rounds needed to win (default 2)
  --round-time <s>    Round timer in seconds (default 99)

Modes:
  --frames <n>        Run n ticks headless, then exit
  --synctest <n>      Run n ticks through GGRS rollback verification
  --screenshot <png>  Save the final frame (headless)
  --scale <n>         Window integer scale (default 3)
```

Relative `--p1`, `--p2`, and `--stage` paths are resolved against `--root`.
See [`manato.toml.example`](manato.toml.example) for the config-file format;
command-line flags override file values.

## Workspace

| Crate | Responsibility |
| --- | --- |
| `manato-core` | Fixed-point-free vector math, input bitmasks, RNG, state hashing |
| `manato-formats` | DEF/CNS/CMD/AIR/SFF (v1 + v2)/SND parsers |
| `manato-sim` | Deterministic fighting simulation, state machine, collision, AI, rounds |
| `manato-net` | GGRS configuration, sync test, P2P rollback wrapper |
| `manato-render` | `wgpu` sprite atlas, 320x240 render target, scene and HUD |
| `manato-audio` | Event-driven mixer with PulseAudio output (optional `cpal` backend) |
| `manato-app` | CLI, headless runner, winit window, input |

See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the design and
[`PROVENANCE.md`](PROVENANCE.md) for the reference and licensing notes.

## Audio

Audio works out of the box on Linux desktops. The default backend uses the
stable PulseAudio `pa_simple` API, loaded at runtime with `libloading`, so it
works with both PulseAudio and PipeWire's Pulse server and needs no development
headers at build time. If no server is available the engine stays silent while
keeping the same API.

An optional `cpal` backend is available for systems without a Pulse server. It
requires the ALSA development package (`alsa-lib-devel` on Fedora,
`libasound2-dev` on Debian/Ubuntu):

```sh
cargo run -p manato-app --features audio
```

The three original characters have their own sound banks (`kaen.snd`,
`nival.snd`, `ordan.snd`) with attack whooshes, light/medium/heavy hit sounds,
guard rings, throw impacts, and special-move effects. They are synthesized from
scratch by [`tools/gen_sfx.py`](tools/gen_sfx.py) and can be regenerated at any
time.

## Tests

```sh
cargo test --workspace
```

Unit tests always run. The bundled character tests use `content/`; stock
M.U.G.E.N tests skip unless `MUGEN_ROOT` points at a stock installation.

## License

MIT for the original code and tools. See [`LICENSE`](LICENSE) and
[`PROVENANCE.md`](PROVENANCE.md). M.U.G.E.N and any third-party game content
remain the property of their respective authors.
