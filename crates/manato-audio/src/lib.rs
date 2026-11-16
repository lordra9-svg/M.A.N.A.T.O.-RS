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
use manato_sim::{SimState, SoundCommand, SoundSource};

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
    priority: i32,
    group: i32,
    index: i32,
    paused: bool,
}

impl Voice {
    fn new(sound: &Sound, output_rate: u32, volume: f32, pan: f32) -> Option<Self> {
        if sound.samples.is_empty() || sound.sample_rate == 0 || sound.channels == 0 {
            return None;
        }
        let mut voice = Self {
            samples: Arc::new(sound.samples.clone()),
            channels: sound.channels as usize,
            position: 0.0,
            step: sound.sample_rate as f64 / output_rate.max(1) as f64,
            gain_l: 1.0,
            gain_r: 1.0,
            finished: false,
            looping: false,
            loop_start: 0,
            loop_end: 0,
            priority: 0,
            group: sound.group,
            index: sound.index,
            paused: false,
        };
        voice.set_gains(volume, pan);
        Some(voice)
    }

    fn set_gains(&mut self, volume: f32, pan: f32) {
        let volume = volume.clamp(0.0, 1.0);
        let pan = pan.clamp(-1.0, 1.0);
        // Equal-power panning keeps the perceived loudness stable.
        let angle = (pan + 1.0) * 0.25 * std::f32::consts::PI;
        self.gain_l = angle.cos() * volume;
        self.gain_r = angle.sin() * volume;
    }

    fn set_loop(&mut self, looping: bool) {
        self.looping = looping;
        if looping && self.loop_end == 0 {
            self.loop_start = 0;
            self.loop_end = self.samples.len() / self.channels.max(1);
        }
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
            priority: 0,
            group: 0,
            index: 0,
            paused: false,
        })
    }

    fn mix(&mut self, out: &mut [f32], out_channels: usize) {
        if self.paused {
            return;
        }
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

pub(crate) type ChannelKey = (u8, usize, i32);

pub(crate) struct Mixer {
    pending: VecDeque<Voice>,
    channels: std::collections::HashMap<ChannelKey, Voice>,
    music: Option<Voice>,
    output_channels: usize,
    volume: f32,
    music_volume: f32,
    music_base: f32,
}

impl Mixer {
    fn set_music_volume(&mut self, volume: f32) {
        self.music_volume = volume.clamp(0.0, 1.0);
        let gain = (self.music_base * self.music_volume).clamp(0.0, 1.0);
        if let Some(music) = self.music.as_mut() {
            music.gain_l = gain;
            music.gain_r = gain;
        }
    }

    pub(crate) fn fill(&mut self, out: &mut [f32]) {
        for sample in out.iter_mut() {
            *sample = 0.0;
        }
        for voice in &mut self.pending {
            voice.mix(out, self.output_channels);
        }
        self.pending.clear();
        for voice in self.channels.values_mut() {
            voice.mix(out, self.output_channels);
        }
        self.channels.retain(|_, voice| !voice.finished);
        if let Some(music) = self.music.as_mut() {
            music.mix(out, self.output_channels);
            if music.finished {
                self.music = None;
            }
        }
        let volume = self.volume;
        for sample in out.iter_mut() {
            *sample = (*sample * volume).clamp(-1.0, 1.0);
        }
    }

    fn play_channel(&mut self, key: ChannelKey, voice: Voice, low_priority: bool) {
        if let Some(existing) = self.channels.get(&key) {
            if low_priority && existing.priority > voice.priority {
                return;
            }
        }
        self.channels.insert(key, voice);
    }

    fn stop_channel(&mut self, key: ChannelKey) {
        self.channels.remove(&key);
    }

    fn pan_channel(&mut self, key: ChannelKey, pan: Option<f32>, volume: Option<f32>) {
        let Some(voice) = self.channels.get_mut(&key) else {
            return;
        };
        let current_volume = (voice.gain_l * voice.gain_l + voice.gain_r * voice.gain_r)
            .sqrt()
            .clamp(0.0, 1.0);
        let current_pan = voice.gain_r.atan2(voice.gain_l) / (0.25 * std::f32::consts::PI) - 1.0;
        voice.set_gains(
            volume.unwrap_or(current_volume),
            pan.unwrap_or(current_pan.clamp(-1.0, 1.0)),
        );
    }

    fn modify_channel(
        &mut self,
        key: ChannelKey,
        sound: &Sound,
        output_rate: u32,
        volume: Option<f32>,
        pan: Option<f32>,
        loop_sound: bool,
    ) {
        if let Some(voice) = self.channels.get_mut(&key) {
            if voice.group == sound.group && voice.index == sound.index {
                let current_volume = (voice.gain_l * voice.gain_l + voice.gain_r * voice.gain_r)
                    .sqrt()
                    .clamp(0.0, 1.0);
                let current_pan =
                    voice.gain_r.atan2(voice.gain_l) / (0.25 * std::f32::consts::PI) - 1.0;
                voice.set_gains(
                    volume.unwrap_or(current_volume),
                    pan.unwrap_or(current_pan.clamp(-1.0, 1.0)),
                );
                voice.set_loop(loop_sound);
                return;
            }
        }
        if let Some(mut voice) = Voice::new(
            sound,
            output_rate,
            volume.unwrap_or(1.0),
            pan.unwrap_or(0.0),
        ) {
            voice.set_loop(loop_sound);
            self.channels.insert(key, voice);
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
        channels: std::collections::HashMap::new(),
        music: None,
        output_channels: channels,
        volume: 1.0,
        music_volume: 1.0,
        music_base: 1.0,
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
            let gain = (volume.clamp(0.0, 1.0) * mixer.music_volume).clamp(0.0, 1.0);
            let mut voice = voice;
            voice.gain_l = gain;
            voice.gain_r = gain;
            mixer.music_base = volume.clamp(0.0, 1.0);
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

    /// Scales music independently of sound effects.
    pub fn set_music_volume(&self, volume: f32) {
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.set_music_volume(volume);
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

    fn channel_key(source: SoundSource, sim: &SimState, channel: i32) -> ChannelKey {
        match source {
            SoundSource::Character(i) => {
                // Helpers share their owner's channel namespace.
                let owner = sim
                    .fighters
                    .get(i)
                    .and_then(|fighter| fighter.owner)
                    .unwrap_or(i);
                (0, owner, channel)
            }
            SoundSource::FightFx => (1, 0, channel),
            SoundSource::Common => (2, 0, channel),
        }
    }

    fn resolve_bank(sim: &SimState, source: SoundSource) -> Option<&SoundBank> {
        match source {
            SoundSource::Character(i) => sim.assets.characters.get(i).map(|c| &c.sounds),
            SoundSource::FightFx | SoundSource::Common => Some(&sim.assets.fight_sounds),
        }
    }

    /// Resolves the sound events produced by the most recent simulation tick.
    pub fn play_events(&self, sim: &SimState) {
        for event in &sim.sounds {
            let Some(bank) = Self::resolve_bank(sim, event.source) else {
                continue;
            };
            let Some(sound) = bank.get(event.group, event.index) else {
                continue;
            };
            let pan = match event.source {
                SoundSource::Character(i) if event.pan == 0.0 => sim
                    .fighters
                    .get(i)
                    .map(|f| ((f.pos.x - sim.camera.x) / 160.0).clamp(-1.0, 1.0) * 0.7)
                    .unwrap_or(0.0),
                _ => event.pan,
            };
            let volume = if event.volume <= 0.0 {
                1.0
            } else {
                (event.volume / 100.0).clamp(0.0, 1.0)
            };
            if event.channel < 0 {
                self.play(sound, volume, pan);
                continue;
            }
            let key = Self::channel_key(event.source, sim, event.channel);
            if let Ok(mut mixer) = self.mixer.lock() {
                if let Some(mut voice) = Voice::new(sound, self.rate, volume, pan) {
                    voice.priority = event.priority;
                    voice.set_loop(event.loop_sound);
                    mixer.play_channel(key, voice, event.low_priority);
                }
            }
        }

        for command in &sim.sound_commands {
            match command {
                SoundCommand::Stop { source, channel } => {
                    let key = Self::channel_key(*source, sim, *channel);
                    if let Ok(mut mixer) = self.mixer.lock() {
                        mixer.stop_channel(key);
                    }
                }
                SoundCommand::Pan {
                    source,
                    channel,
                    pan,
                    volume,
                } => {
                    let key = Self::channel_key(*source, sim, *channel);
                    if let Ok(mut mixer) = self.mixer.lock() {
                        mixer.pan_channel(key, *pan, *volume);
                    }
                }
                SoundCommand::Modify {
                    source,
                    channel,
                    group,
                    index,
                    volume,
                    pan,
                    loop_sound,
                } => {
                    let Some(bank) = Self::resolve_bank(sim, *source) else {
                        continue;
                    };
                    let Some(sound) = bank.get(*group, *index) else {
                        continue;
                    };
                    let key = Self::channel_key(*source, sim, *channel);
                    if let Ok(mut mixer) = self.mixer.lock() {
                        mixer.modify_channel(
                            key,
                            sound,
                            self.rate,
                            *volume,
                            *pan,
                            loop_sound.unwrap_or(false),
                        );
                    }
                }
                SoundCommand::PlayBgm {
                    path,
                    volume,
                    loop_start,
                    loop_end,
                    ..
                } => match Music::load(path, *loop_start, *loop_end) {
                    Ok(music) => self.play_music(&music, (*volume as f32 / 100.0).clamp(0.0, 1.0)),
                    Err(error) => log::warn!("could not play BGM {}: {error:#}", path.display()),
                },
                SoundCommand::ModifyBgm { volume, .. } => {
                    if let Some(volume) = volume {
                        self.set_music_volume((*volume as f32 / 100.0).clamp(0.0, 1.0));
                    }
                }
                SoundCommand::StopBgm => self.stop_music(),
            }
        }
    }

    /// Pauses or resumes background music without losing its position.
    pub fn pause_music(&self, paused: bool) {
        if let Ok(mut mixer) = self.mixer.lock() {
            if let Some(music) = mixer.music.as_mut() {
                music.paused = paused;
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
            channels: std::collections::HashMap::new(),
            music: None,
            output_channels: 2,
            volume: 1.0,
            music_volume: 1.0,
            music_base: 1.0,
        };
        mixer
            .pending
            .push_back(Voice::new(&mono_sound(vec![16_384; 64]), 48_000, 1.0, 0.0).unwrap());
        let mut out = vec![0.0f32; 128];
        mixer.fill(&mut out);
        assert!(out[0].abs() > 0.1);
        assert!(out[1].abs() > 0.1);
        assert_eq!(out[0], out[1]);
    }

    #[test]
    fn loads_wav_music() {
        let samples: [i16; 8] = [0, 1000, -1000, 2000, -2000, 3000, -3000, 0];
        let data_len = (samples.len() * 2) as u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8_000u32.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        let path = std::env::temp_dir().join("manato-music-test.wav");
        std::fs::write(&path, wav).unwrap();
        let music = Music::load(&path, 0, 0).unwrap();
        assert_eq!(music.sample_rate, 8_000);
        assert_eq!(music.channels, 1);
        assert_eq!(music.samples.len(), samples.len());
        let _ = std::fs::remove_file(path);
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
            channels: std::collections::HashMap::new(),
            music: None,
            output_channels: 2,
            volume: 1.0,
            music_volume: 1.0,
            music_base: 1.0,
        };
        left.pending
            .push_back(Voice::new(&mono_sound(vec![16_384; 32]), 48_000, 1.0, -1.0).unwrap());
        let mut out = vec![0.0f32; 64];
        left.fill(&mut out);
        assert!(out[0].abs() > 0.1);
        assert!(out[1].abs() < 0.01);

        let mut right = Mixer {
            pending: VecDeque::new(),
            channels: std::collections::HashMap::new(),
            music: None,
            output_channels: 2,
            volume: 1.0,
            music_volume: 1.0,
            music_base: 1.0,
        };
        right
            .pending
            .push_back(Voice::new(&mono_sound(vec![16_384; 32]), 48_000, 1.0, 1.0).unwrap());
        let mut out = vec![0.0f32; 64];
        right.fill(&mut out);
        assert!(out[0].abs() < 0.01);
        assert!(out[1].abs() > 0.1);
    }

    fn test_mixer() -> Mixer {
        Mixer {
            pending: VecDeque::new(),
            channels: std::collections::HashMap::new(),
            music: None,
            output_channels: 2,
            volume: 1.0,
            music_volume: 1.0,
            music_base: 1.0,
        }
    }

    #[test]
    fn channel_priority_and_stop() {
        let sound = mono_sound(vec![1_000; 32]);
        let key = (0u8, 0usize, 0i32);
        let mut mixer = test_mixer();
        let mut high = Voice::new(&sound, 48_000, 1.0, 0.0).unwrap();
        high.priority = 5;
        mixer.play_channel(key, high, false);
        let mut low = Voice::new(&sound, 48_000, 1.0, 0.0).unwrap();
        low.priority = 1;
        mixer.play_channel(key, low, true);
        assert_eq!(mixer.channels[&key].priority, 5);
        mixer.stop_channel(key);
        assert!(mixer.channels.is_empty());
    }

    #[test]
    fn channel_pan_and_modify() {
        let sound = mono_sound(vec![1_000; 32]);
        let key = (0u8, 0usize, 0i32);
        let mut mixer = test_mixer();
        mixer.play_channel(key, Voice::new(&sound, 48_000, 1.0, 0.0).unwrap(), false);
        mixer.pan_channel(key, Some(-1.0), Some(0.5));
        let voice = &mixer.channels[&key];
        assert!(voice.gain_l > voice.gain_r);
        assert!(!voice.looping);
        mixer.modify_channel(key, &sound, 48_000, None, None, true);
        assert!(mixer.channels[&key].looping);
    }
    #[test]
    fn music_volume_scales_independently() {
        let music = Music {
            samples: Arc::new(vec![10_000; 64]),
            sample_rate: 48_000,
            channels: 1,
            loop_start_ms: 0,
            loop_end_ms: 0,
        };
        let mut mixer = Mixer {
            pending: VecDeque::new(),
            channels: std::collections::HashMap::new(),
            music: Some(Voice::new_music(&music, 48_000, 0.8).unwrap()),
            output_channels: 2,
            volume: 1.0,
            music_volume: 1.0,
            music_base: 0.8,
        };
        mixer.set_music_volume(0.5);
        let voice = mixer.music.as_ref().unwrap();
        assert!((voice.gain_l - 0.4).abs() < f32::EPSILON);
        assert!((voice.gain_r - 0.4).abs() < f32::EPSILON);

        let mut out = vec![0.0f32; 128];
        mixer.fill(&mut out);
        assert!(out[0].abs() > 0.01 && out[0].abs() < 0.2);
    }
    #[test]
    fn pause_music_freezes_and_resumes_position() {
        let engine = AudioEngine::silent();
        let music = Music {
            samples: Arc::new((0..512).map(|i| (i % 64) as i16 * 200).collect()),
            sample_rate: 48_000,
            channels: 1,
            loop_start_ms: 0,
            loop_end_ms: 0,
        };
        engine.play_music(&music, 1.0);
        let mut out = vec![0.0f32; 64];
        engine.mixer.lock().unwrap().fill(&mut out);
        let before = engine
            .mixer
            .lock()
            .unwrap()
            .music
            .as_ref()
            .unwrap()
            .position;

        engine.pause_music(true);
        for _ in 0..3 {
            let mut paused_out = vec![0.0f32; 64];
            engine.mixer.lock().unwrap().fill(&mut paused_out);
            assert!(paused_out.iter().all(|sample| *sample == 0.0));
        }
        let during = engine
            .mixer
            .lock()
            .unwrap()
            .music
            .as_ref()
            .unwrap()
            .position;
        assert!((during - before).abs() < f64::EPSILON);

        engine.pause_music(false);
        engine.mixer.lock().unwrap().fill(&mut out);
        let after = engine
            .mixer
            .lock()
            .unwrap()
            .music
            .as_ref()
            .unwrap()
            .position;
        assert!(after > during);
    }
}
