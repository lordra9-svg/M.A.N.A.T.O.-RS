//! Event-driven sound mixer and audio output.
//!
//! Game logic never blocks on audio: it emits [`manato_sim::SoundEvent`]s and
//! the mixer resolves them against the loaded sound banks.
//!
//! On Linux the default backend is PulseAudio's stable `pa_simple` API, loaded
//! at runtime, so no development headers are needed. The optional
//! `cpal-backend` feature adds a second backend for systems without a Pulse
//! server. If neither can open a device the engine stays silent while keeping
//! the same API.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use manato_formats::{Sound, SoundBank};
use manato_sim::{SimState, SoundSource};

#[cfg(target_os = "linux")]
mod pulse;

#[cfg(feature = "cpal-backend")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Decoded background music.
#[derive(Debug, Clone)]
pub struct Music {
    pub samples: Arc<Vec<i16>>,
    pub sample_rate: u32,
    pub channels: u16,
    pub loop_start_ms: i32,
    pub loop_end_ms: i32,
}

impl Music {
    pub fn load(path: &Path, loop_start_ms: i32, loop_end_ms: i32) -> anyhow::Result<Self> {
        use std::fs::File;

        use symphonia::core::audio::SampleBuffer;
        use symphonia::core::codecs::DecoderOptions;
        use symphonia::core::errors::Error as SymphoniaError;
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;
        use symphonia::default::{get_codecs, get_probe};

        let file = File::open(path)?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
            hint.with_extension(extension);
        }
        let probed = get_probe().format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;
        let mut format = probed.format;
        let track = format
            .default_track()
            .ok_or_else(|| anyhow::anyhow!("music file has no default track"))?;
        let codec_params = track.codec_params.clone();
        let mut decoder = get_codecs().make(&codec_params, &DecoderOptions::default())?;
        let sample_rate = codec_params.sample_rate.unwrap_or(44_100);
        let channels = codec_params
            .channels
            .map(|channels| channels.count())
            .unwrap_or(2) as u16;
        let mut samples = Vec::new();
        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(SymphoniaError::ResetRequired) => break,
                Err(error) => return Err(error.into()),
            };
            let decoded = decoder.decode(&packet)?;
            let spec = *decoded.spec();
            let mut buffer = SampleBuffer::<i16>::new(decoded.capacity() as u64, spec);
            buffer.copy_interleaved_ref(decoded);
            samples.extend_from_slice(buffer.samples());
        }
        if samples.is_empty() {
            anyhow::bail!("music file decoded to zero samples");
        }
        Ok(Self {
            samples: Arc::new(samples),
            sample_rate,
            channels,
            loop_start_ms,
            loop_end_ms,
        })
    }
}

struct Voice {
    samples: Arc<Vec<i16>>,
    channels: usize,
    position: f64,
    step: f64,
    gain_l: f32,
    gain_r: f32,
    finished: bool,
    looping: bool,
    loop_start: usize,
    loop_end: usize,
}

impl Voice {
    fn new(sound: &Sound, output_rate: u32, volume: f32, pan: f32) -> Option<Self> {
        if sound.samples.is_empty() || sound.sample_rate == 0 || sound.channels == 0 {
            return None;
        }
        let volume = (1.0 + volume / 100.0).clamp(0.0, 2.0);
        let pan = pan.clamp(-1.0, 1.0);
        // Equal-power panning keeps the perceived loudness stable.
        let angle = (pan + 1.0) * 0.25 * std::f32::consts::PI;
        Some(Self {
            samples: Arc::new(sound.samples.clone()),
            channels: sound.channels as usize,
            position: 0.0,
            step: sound.sample_rate as f64 / output_rate.max(1) as f64,
            gain_l: angle.cos() * volume,
            gain_r: angle.sin() * volume,
            finished: false,
            looping: false,
            loop_start: 0,
            loop_end: 0,
        })
    }

    fn new_music(music: &Music, output_rate: u32, volume: f32) -> Option<Self> {
        if music.samples.is_empty() || music.sample_rate == 0 || music.channels == 0 {
            return None;
        }
        let frames = music.samples.len() / music.channels as usize;
        let loop_start = ((music.loop_start_ms.max(0) as u64 * music.sample_rate as u64) / 1000)
            .min(frames.saturating_sub(1) as u64) as usize;
        let loop_end = if music.loop_end_ms > 0 {
            ((music.loop_end_ms as u64 * music.sample_rate as u64) / 1000).min(frames as u64)
                as usize
        } else {
            frames
        };
        Some(Self {
            samples: music.samples.clone(),
            channels: music.channels as usize,
            position: 0.0,
            step: music.sample_rate as f64 / output_rate.max(1) as f64,
            gain_l: volume,
            gain_r: volume,
            finished: false,
            looping: true,
            loop_start,
            loop_end: loop_end.max(loop_start.saturating_add(1)),
        })
    }

    fn mix(&mut self, out: &mut [f32], out_channels: usize) {
        let frames = self.samples.len() / self.channels;
        for frame in 0..out.len() / out_channels {
            let mut idx = self.position as usize;
            if idx >= frames || (self.looping && idx >= self.loop_end) {
                if self.looping && self.loop_end > self.loop_start {
                    self.position = self.loop_start as f64;
                    idx = self.loop_start;
                } else {
                    self.finished = true;
                    break;
                }
            }
            let (l, r) = if self.channels >= 2 {
                (
                    self.samples[idx * self.channels] as f32 / 32768.0,
                    self.samples[idx * self.channels + 1] as f32 / 32768.0,
                )
            } else {
                let s = self.samples[idx] as f32 / 32768.0;
                (s, s)
            };
            let base = frame * out_channels;
            if out_channels >= 2 {
                out[base] += l * self.gain_l;
                out[base + 1] += r * self.gain_r;
            } else {
                out[base] += (l * self.gain_l + r * self.gain_r) * 0.5;
            }
            self.position += self.step;
        }
    }
}

pub(crate) struct Mixer {
    pending: VecDeque<Voice>,
    active: Vec<Voice>,
    music: Option<Voice>,
    output_channels: usize,
    volume: f32,
}

impl Mixer {
    pub(crate) fn fill(&mut self, out: &mut [f32]) {
        for sample in out.iter_mut() {
            *sample = 0.0;
        }
        self.active.extend(self.pending.drain(..));
        for voice in &mut self.active {
            voice.mix(out, self.output_channels);
        }
        if let Some(music) = self.music.as_mut() {
            music.mix(out, self.output_channels);
            if music.finished {
                self.music = None;
            }
        }
        self.active.retain(|v| !v.finished);
        let volume = self.volume;
        for sample in out.iter_mut() {
            *sample = (*sample * volume).clamp(-1.0, 1.0);
        }
    }
}

#[allow(dead_code)] // Variants are held for their Drop implementations.
enum Sink {
    Silent,
    #[cfg(target_os = "linux")]
    Pulse(pulse::PulseSink),
    #[cfg(feature = "cpal-backend")]
    Cpal(cpal::Stream),
}

pub struct AudioEngine {
    mixer: Arc<Mutex<Mixer>>,
    rate: u32,
    _sink: Sink,
}

fn new_mixer(channels: usize) -> Arc<Mutex<Mixer>> {
    Arc::new(Mutex::new(Mixer {
        pending: VecDeque::new(),
        active: Vec::new(),
        music: None,
        output_channels: channels,
        volume: 1.0,
    }))
}

impl AudioEngine {
    /// Opens the best available output backend. On failure the engine is still
    /// constructed, but silent.
    pub fn new() -> Self {
        #[cfg(feature = "cpal-backend")]
        {
            match Self::try_cpal() {
                Ok(engine) => return engine,
                Err(e) => log::warn!("cpal audio unavailable: {e:#}"),
            }
        }
        #[cfg(target_os = "linux")]
        {
            match Self::try_pulse() {
                Ok(engine) => return engine,
                Err(e) => log::warn!("pulse audio unavailable: {e:#}"),
            }
        }
        log::warn!("audio disabled: no usable output backend");
        Self::silent()
    }

    pub fn silent() -> Self {
        Self {
            mixer: new_mixer(2),
            rate: 48_000,
            _sink: Sink::Silent,
        }
    }

    #[cfg(target_os = "linux")]
    fn try_pulse() -> anyhow::Result<Self> {
        const RATE: u32 = 48_000;
        const CHANNELS: usize = 2;
        let mixer = new_mixer(CHANNELS);
        let sink = pulse::start(mixer.clone(), RATE, CHANNELS)?;
        Ok(Self {
            mixer,
            rate: RATE,
            _sink: Sink::Pulse(sink),
        })
    }

    #[cfg(feature = "cpal-backend")]
    fn try_cpal() -> anyhow::Result<Self> {
        use anyhow::Context as _;

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no default output device")?;
        let supported = device.default_output_config()?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let rate = config.sample_rate.0;
        let channels = config.channels as usize;

        let mixer = new_mixer(channels);
        let shared = mixer.clone();
        let err_fn = |err| log::warn!("audio stream error: {err}");

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    if let Ok(mut mixer) = shared.lock() {
                        mixer.fill(data);
                    }
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => {
                let shared = mixer.clone();
                device.build_output_stream(
                    &config,
                    move |data: &mut [i16], _| {
                        let mut tmp = vec![0.0f32; data.len()];
                        if let Ok(mut mixer) = shared.lock() {
                            mixer.fill(&mut tmp);
                        }
                        for (dst, src) in data.iter_mut().zip(tmp) {
                            *dst = (src.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::U16 => {
                let shared = mixer.clone();
                device.build_output_stream(
                    &config,
                    move |data: &mut [u16], _| {
                        let mut tmp = vec![0.0f32; data.len()];
                        if let Ok(mut mixer) = shared.lock() {
                            mixer.fill(&mut tmp);
                        }
                        for (dst, src) in data.iter_mut().zip(tmp) {
                            let v = (src.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32;
                            *dst = v as u16;
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            other => anyhow::bail!("unsupported output sample format {other:?}"),
        };
        stream.play()?;
        Ok(Self {
            mixer,
            rate,
            _sink: Sink::Cpal(stream),
        })
    }

    pub fn play_music(&self, music: &Music, volume: f32) {
        let Some(voice) = Voice::new_music(music, self.rate, volume.clamp(0.0, 1.0)) else {
            return;
        };
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.music = Some(voice);
        }
    }

    pub fn stop_music(&self) {
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.music = None;
        }
    }

    pub fn set_volume(&self, volume: f32) {
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.volume = volume.clamp(0.0, 1.0);
        }
    }

    pub fn play(&self, sound: &Sound, volume: f32, pan: f32) {
        let Some(voice) = Voice::new(sound, self.rate, volume, pan) else {
            return;
        };
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.pending.push_back(voice);
        }
    }

    pub fn play_bank(&self, bank: &SoundBank, group: i32, index: i32, volume: f32, pan: f32) {
        if let Some(sound) = bank.get(group, index) {
            self.play(sound, volume, pan);
        }
    }

    /// Resolves the sound events produced by the most recent simulation tick.
    pub fn play_events(&self, sim: &SimState) {
        for event in &sim.sounds {
            let (bank, pan) = match event.source {
                SoundSource::Character(i) => {
                    let bank = sim.assets.characters.get(i).map(|c| &c.sounds);
                    // Pan character sounds by their position on screen.
                    let pan = sim
                        .fighters
                        .get(i)
                        .map(|f| ((f.pos.x - sim.camera.x) / 160.0).clamp(-1.0, 1.0) * 0.7)
                        .unwrap_or(event.pan);
                    (bank, pan)
                }
                SoundSource::FightFx | SoundSource::Common => {
                    (Some(&sim.assets.fight_sounds), event.pan)
                }
            };
            if let Some(bank) = bank {
                self.play_bank(bank, event.group, event.index, event.volume, pan);
            }
        }
    }

    /// Output sample rate used for resampling (informational).
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// Name of the active output backend.
    pub fn backend_name(&self) -> &'static str {
        match self._sink {
            Sink::Silent => "silent",
            #[cfg(target_os = "linux")]
            Sink::Pulse(_) => "pulse",
            #[cfg(feature = "cpal-backend")]
            Sink::Cpal(_) => "cpal",
        }
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono_sound(samples: Vec<i16>) -> Sound {
        Sound {
            group: 0,
            index: 0,
            sample_rate: 48_000,
            channels: 1,
            samples,
        }
    }

    #[test]
    fn mixes_mono_into_stereo() {
        let mut mixer = Mixer {
            pending: VecDeque::new(),
            active: Vec::new(),
            music: None,
            output_channels: 2,
            volume: 1.0,
        };
        mixer
            .active
            .push(Voice::new(&mono_sound(vec![16_384; 64]), 48_000, 0.0, 0.0).unwrap());
        let mut out = vec![0.0f32; 128];
        mixer.fill(&mut out);
        assert!(out[0].abs() > 0.1);
        assert!(out[1].abs() > 0.1);
        assert_eq!(out[0], out[1]);
    }

    #[test]
    fn music_loops() {
        let music = Music {
            samples: Arc::new(vec![1_000, 2_000, 3_000, 4_000]),
            sample_rate: 48_000,
            channels: 1,
            loop_start_ms: 0,
            loop_end_ms: 0,
        };
        let mut voice = Voice::new_music(&music, 48_000, 1.0).unwrap();
        let mut out = vec![0.0f32; 16];
        voice.mix(&mut out, 2);
        assert!(out.iter().any(|sample| sample.abs() > 0.0));
        assert!(!voice.finished);
    }

    #[test]
    fn pans_left_and_right() {
        let mut left = Mixer {
            pending: VecDeque::new(),
            active: vec![Voice::new(&mono_sound(vec![16_384; 32]), 48_000, 0.0, -1.0).unwrap()],
            music: None,
            output_channels: 2,
            volume: 1.0,
        };
        let mut out = vec![0.0f32; 64];
        left.fill(&mut out);
        assert!(out[0].abs() > 0.1);
        assert!(out[1].abs() < 0.01);

        let mut right = Mixer {
            pending: VecDeque::new(),
            active: vec![Voice::new(&mono_sound(vec![16_384; 32]), 48_000, 0.0, 1.0).unwrap()],
            music: None,
            output_channels: 2,
            volume: 1.0,
        };
        let mut out = vec![0.0f32; 64];
        right.fill(&mut out);
        assert!(out[0].abs() < 0.01);
        assert!(out[1].abs() > 0.1);
    }
}
