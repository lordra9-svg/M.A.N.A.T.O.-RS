# Architecture

M.A.N.A.T.O.-RS separates deterministic game state from presentation:

```
          content files
                |
        manato-formats
                |
          manato-sim  <---- manato-net (GGRS snapshots + inputs)
           /        \
   manato-render   manato-audio
           \        /
            manato-app
```

`manato-sim` never touches the GPU, window system, or audio device. It consumes
`[PlayerInput; 2]` and produces drawable state plus event lists (`Effect`,
`SoundEvent`). This is what makes rollback cheap: a snapshot is just the
`SimState`, while parsed assets live behind `Arc<Assets>` and are never cloned.

## Time and coordinates

- Fixed **60 Hz** simulation tick. The window loop accumulates real time and
  steps the simulation at most five times per redraw.
- Game space is **320x240**. X grows right, Y grows down, and the ground plane
  is `y = 0`.
- Screen position is
  `screen_x = 160 + world_x - camera.x`,
  `screen_y = stage.zoffset + world_y - camera.y`.
- The camera follows the midpoint of the fighters, clamped to the stage's
  `boundLeft`/`boundRight`, with smoothing factor 0.35.

## Parsing (`manato-formats`)

- Text files are decoded tolerantly (BOM, UTF-16, comments, quoted values) and
  parsed case-insensitively. Duplicate sections are preserved in order, which
  matters for repeated `[BG ...]` blocks.
- **DEF** locates files and exposes sections such as `[Files]`, `[Camera]`,
  `[Bound]`, `[PlayerInfo]`, and `[BGdef]`.
- **CNS** produces `StateDef` headers plus ordered controllers. A controller
  belongs to the enclosing `[Statedef]`; the number in `[State N, name]` is only
  a label. Trigger expressions are parsed into an AST and evaluated per tick.
- Character DEF state files are merged in M.U.G.E.N order: `stcommon`, `cns`,
  `st`, `st1`..`st9`, then the CMD file. This matters for characters that put
  configuration (for example a damage multiplier) in an `st1` file.
- **CMD** parses command steps and matching prefixes: `$` pooled direction tap,
  `/` held direction, `~` release. Commands are matched facing-relative.
- **AIR** produces frame lists with sprite keys, offsets, durations, flips, and
  blend modes. Collision boxes are attached to frames.
- **SFF v1** decodes embedded PCX subfiles and their inline palettes.
- **SFF v2** decodes raw, RLE8, RLE5, LZ5, and PNG sprites. Indexed formats and
  PNG sprites carry a 4-byte uncompressed-size prefix; PNG8 indices are mapped
  through the SFF palette node because the PNG's own `PLTE` is often a dummy.
- **SND** decodes WAV data into PCM with sample rate and channel count.

## Simulation (`manato-sim`)

`SimState` owns fighters, the stage reference, round state, camera, effects,
sound events, and the RNG. `Assets` owns parsed characters, the stage, fight
effects, and shared data directories.

Each tick:

1. Lock control during the round intro and KO slowdown, then append inputs to
   each fighter's history (absolute directions + buttons).
2. Run special states `-3`, `-2`, `-1` in order.
3. Run built-in movement transitions (stand 0, walk 20, crouch 10/11, jump
   40/50/52) when the fighter has control.
4. Integrate physics (stand/crouch friction, air acceleration, landing).
5. Run the current state's controllers. A `ChangeState` continues from the new
   state in the same tick (capped at 8 transitions to prevent loops).
6. Resolve hit/guard collisions, separate pushboxes, update rounds, camera,
   effects, and background time.

State entry applies the CNS defaults: `type = S`, `movetype = I`, `physics = N`,
with an explicit `U` meaning "keep the previous value". This matters after
hitstun: without the idle default the CPU would keep believing it is attacking.

Animation timing uses `anim_time` as total elapsed ticks. `AnimTime` reports
`total_time - anim_time`; the extra tick at `total_time` is intentional so
`AnimTime = 0` fires before a finite animation wraps. All duration sums use
saturating arithmetic because stock animations can contain `-1` ("forever")
frames.

## Collision

Attack (`Clsn1`) and hurt (`Clsn2`) boxes are transformed from character space
to world space using position, facing, and scale. A hit applies damage,
hit/guard timers, velocities, and power changes. Hit flags, `NotHitBy` filters,
and guard flags are checked before resolution. Hit pause freezes the affected
fighters while effects continue to animate.

## Rendering (`manato-render`)

- Every sprite from the stage, both characters, and `fightfx` is packed into a
  small number of 1024px RGBA atlas pages using a shelf packer.
- The scene is drawn into a fixed 320x240 texture with nearest-neighbour
  sampling, then blitted letterboxed to the window. Screenshots read the same
  texture back, so headless and windowed output are identical.
- Background layers respect `start`, `delta` (parallax), `tile`, and
  `tileSpacing`; negative layers draw behind fighters and positive layers in
  front.
- The HUD uses an embedded 8x8 bitmap font and draws life, power, timer, names,
  round wins, and the round/result banner.
- A configured screenpack replaces the HUD: `fight.def` drives life/power
  bars, faces, names, timer, win icons, combo counters, and round
  announcements; `system.def` drives the title and select screens, including
  their `[TitleBGdef]`/`[SelectBGdef]` layers. Screenpack `.fnt` v1/v2 fonts
  are uploaded as their own glyph atlases.
- The front end (`--menu`) is app-owned: title → select → fight → result →
  title. It never allocates a `SimState` until a match is confirmed and is
  never part of rollback. The select screen supports roster pages, stage
  cycling, portraits, and an optional countdown.
- Input is keyboard + gilrs gamepads. Keyboard bindings and gamepad menus are
  app-level; the simulation only sees `PlayerInput`.
- `AssertSpecial` flags that affect presentation (`NoBarDisplay`, `NoBG`,
  `NoFG`, `Invisible`, `NoShadow`) are applied by the renderer.

## Netcode (`manato-net`)

`ManatoConfig` implements `ggrs::Config` with `Input = PlayerInput` and
`State = SimState`. Snapshots are cloned states; loading replaces the local
`SimState`. Assets are shared by `Arc`, so save/load only copies mutable state.

- `SyncTest` runs the real game through GGRS save/load/rollback cycles with
  checksums.
- `Netplay` wraps a two-player P2P session, uses two frames of input delay, and
  reports `PredictionThreshold` as "skip this frame" rather than an error.

## Audio (`manato-audio`)

The mixer is event-driven. `manato-sim` emits `SoundEvent { source, group,
index, volume, pan }`; the audio crate resolves them against the loaded sound
banks, resamples to the device rate, and mixes with equal-power panning.

On Linux the default output backend uses PulseAudio's `pa_simple` API through
`libloading`, so the binary links without ALSA/Pulse development headers and
works with either PulseAudio or PipeWire's Pulse server. The optional
`cpal-backend` feature adds a cpal/ALSA backend for other setups. If no backend
can open a device, the engine keeps the same API but is silent.

Character `PlaySnd` controllers and `HitDef` `hitsound`/`guardsound` values are
resolved against the attacker's own `SoundBank`, while `F`-prefixed common
sounds use the shared `fight.snd` bank. Character sounds are panned by their
on-screen position.

## Determinism rules

- No wall-clock time, floating-point randomness, or hash-map iteration order in
  simulation code.
- Randomness comes from a seeded `Rng` stored in `SimState`.
- Assets are immutable and shared, never mutated during a tick.
- `StateHasher` produces a stable checksum for desync detection.
- CPU input is derived only from the current snapshot and the frame number.

## Known limitations

- Only a subset of M.U.G.E.N controllers and triggers is implemented; unknown
  controllers are ignored with a log message.
- Helpers, custom state files beyond `stsl`, advanced `HitDef` parameters, and
  some IKEMEN extensions are not yet covered.
- The renderer targets the classic 320x240 layout; there is no widescreen or
  high-resolution stage support yet.
- Audio requires ALSA development headers on Linux to enable the real backend.
