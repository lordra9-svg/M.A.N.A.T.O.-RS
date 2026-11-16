# Audio subsystem specification

**Status: APPROVED / FROZEN.** Derived from the MIT-licensed Ikemen-GO
source (`src/sound.go`, `src/bytecode.go`), public M.U.G.E.N audio
documentation, and observed content.

## 1. Scope

Define sound playback and channel management for the engine:

- SND bank playback (already implemented)
- `PlaySnd`, `StopSnd`, `SndPan`, `ModifySnd`
- Per-owner sound channels
- Volume, pan, priority, and looping
- Pause/super-pause behavior
- BGM playback, looping, volume, and pause/resume

Out of scope: 3D audio, custom DSP filters, and platform-specific backends
beyond the existing PulseAudio/cpal split.

## 2. Sound events

The simulation emits `SoundEvent` values instead of touching the audio device:

```
source: Character(owner) | FightFx | Common
group: i32
index: i32
channel: i32        // -1 = one-shot, >=0 = managed channel
volume: f32         // 0..1, 0 means "default"
pan: f32            // -1..1
loop: bool
priority: i32       // higher wins channel contention
low_priority: bool  // ignored when a higher-priority sound is playing
```

Events are cleared at the start of each tick and consumed by the audio layer
after the simulation step, so they are rollback-safe.

## 3. Channels

- Channels are keyed by `(source, channel)`.
- `channel < 0` plays a one-shot voice with no channel ownership.
- `channel >= 0` replaces the current voice on that channel.
- A new voice is rejected when the existing voice has a higher priority and
  `low_priority` is set.
- Equal priority replaces the existing voice.
- `StopSnd` stops the voice on a channel.
- Voices are removed when they finish; one-shots do not occupy channels.

Helpers use their owner's channel namespace (`Character(root_owner)`), so a
helper and its owner cannot fight over the same channel.

## 4. PlaySnd

```
[State ...]
type = PlaySnd
value = group, index
volume = 100
channel = 0
pan = 0
loop = 0
lowpriority = 0
```

- `value` selects the sound; a `F`/`S` prefix selects FightFx/Common banks
  where content uses it.
- `volume`: 0–100; 0 means the default full volume.
- `channel`: -1 by default.
- `pan`: -100..100, scaled to -1..1.
- `loop`: repeat until stopped.
- `lowpriority`: honor channel priority.
- Missing sound: log once and do nothing.

## 5. StopSnd and SndPan

```
type = StopSnd
channel = 0

type = SndPan
channel = 0
pan = 50
volume = 80
```

- `StopSnd` stops the channel for the owner.
- `SndPan` updates pan and/or volume on a live channel.
- Missing channel is a no-op.

## 6. ModifySnd

```
type = ModifySnd
channel = 0
value = 5, 2
volume = 70
pan = -20
loop = 0
```

- Replaces the sound on a channel without restarting it if the group/index is
  unchanged; otherwise stops and starts the new sound.
- `volume`, `pan`, and `loop` update the live voice when possible.

## 7. Pause behavior

- Normal pause and super pause freeze the simulation but not already-playing
  sounds, unless the content pauses them.
- `StopSnd` during pause is honored.
- `PlaySnd` during pause is queued and starts when the pause ends.
- BGM continues during pause by default; `Pause`/`Resume` APIs exist for the
  host to pause BGM explicitly.

## 8. BGM

- Decoded with Symphonia (WAV/MP3/OGG/Vorbis).
- Plays with `bgmvolume`/`bgmratio` scaling.
- `bgmloopstart`/`bgmloopend` define loop points in milliseconds.
- `stop_music` stops playback; `play_music` replaces the current track.
- BGM is not part of the rollback checksum.
- Music continues across rounds unless the stage changes.

### 8.1 BGM controllers

`PlayBGM` resolves `bgm` against the character, stage, and shared data
directories (preferring `sound/`), stores the live state in `SimState.bgm`, and
queues a `SoundCommand::PlayBgm`; an empty `bgm` stops playback. Volume is
scaled by 1/100. `loop`, `loopstart`, `loopend`, `startposition`, `freqmul`, and `loopcount`
are parsed and exposed through `BgmVar`. The mixer applies looping (or plays
once when `loop = 0`), seeks to `startposition` (milliseconds), and scales the
playback step by `freqmul`. `loopcount` is stored but not enforced.

`ModifyBGM` updates the live volume, seeks with `position`, and changes
`freqmul` immediately; loop/frequency/count fields are also stored. `BgmVar(filename|volume|loop|loopstart|loopend|
freqmul|loopcount|startposition)` reads the live controller state when active,
otherwise the stage `[Music]` definition.

The mixer reports `BgmStatus { position_ms, length_ms, sample_rate }` from the
active music voice. The app copies it into `SimState.bgm` after each
`play_events`, so `BgmVar(position|length|samplerate)` is live while music
plays; without an audio backend the fields stay 0. Stream state is presentation
data and is not part of the rollback checksum.

## 9. Mixing

- Voices are resampled to the output rate.
- Mono sources are duplicated to stereo; stereo sources are preserved.
- Volume is applied per voice; pan uses a constant-power split.
- Clipping is clamped to the output sample range.
- The mixer sums all active voices and BGM.

## 10. Error handling

- Missing banks, groups, or indices are silent no-ops with one warning.
- Invalid channel indices clamp to one-shot.
- Invalid pan/volume clamp to their ranges.
- A failed BGM decode logs and leaves the previous track stopped.
- No panics on device loss; the mixer keeps running silently.

## 11. Test plan

- Channel replacement and priority contention.
- `lowpriority` rejection while a higher-priority sound is active.
- `StopSnd` and `SndPan` on live channels.
- `ModifySnd` replacement and in-place update.
- Loop playback and explicit stop.
- Volume and pan clamping.
- BGM loop points, volume scaling, and stop/replace.
- Pause queues PlaySnd and honors StopSnd.
- Rollback determinism: sound events are not part of the checksum.

## 12. Sources

- Ikemen-GO `src/sound.go` and `src/bytecode.go` (MIT).
- Public M.U.G.E.N PlaySnd/StopSnd/SndPan documentation.
- Observed character sound usage.
