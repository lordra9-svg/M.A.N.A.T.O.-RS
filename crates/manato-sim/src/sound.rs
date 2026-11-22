//! Per-character sound channels for `SoundVar` and `PlaySnd channel`.
//!
//! The audio mixer owns actual playback, but `SoundVar` needs deterministic,
//! sim-side state. Channels therefore track the decoded SND entry, its
//! sample length, and an elapsed position advanced once per 60 Hz tick. This
//! mirrors Ikemen's channel registry closely enough for trigger use; it does
//! not observe mixer stalls or resampling.

use serde::{Deserialize, Serialize};

use crate::SimState;

/// A live or last-played sound on one character channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundChannelState {
    pub channel: i32,
    pub group: i32,
    pub index: i32,
    /// Volume scale as authored (100 = full).
    pub volume_scale: f32,
    pub freq_mul: f32,
    pub pan: f32,
    pub priority: i32,
    pub loop_sound: bool,
    /// Sample frames per channel in the decoded sound.
    pub length: i32,
    /// Sample frames advanced at 60 Hz.
    pub position: i32,
    pub sample_rate: u32,
    pub playing: bool,
}

impl SoundChannelState {
    /// Advances this channel by one tick. Looping sounds stay active until
    /// stopped; one-shots stop once the sample position reaches the end.
    fn tick(&mut self) {
        if !self.playing {
            return;
        }
        let step = (self.sample_rate / 60).max(1) as i32;
        self.position = self.position.saturating_add(step);
        if !self.loop_sound && self.position >= self.length {
            self.position = self.length;
            self.playing = false;
        }
    }
}

/// Finds or creates a channel entry for `channel`, replacing stopped entries.
fn channel_slot(channels: &mut Vec<SoundChannelState>, channel: i32) -> usize {
    if let Some(index) = channels.iter().position(|entry| entry.channel == channel) {
        return index;
    }
    channels.push(SoundChannelState {
        channel,
        group: -1,
        index: 0,
        volume_scale: 0.0,
        freq_mul: 1.0,
        pan: 0.0,
        priority: 0,
        loop_sound: false,
        length: 0,
        position: 0,
        sample_rate: 0,
        playing: false,
    });
    channels.len() - 1
}

impl SimState {
    /// Starts (or replaces) a character sound channel. `channel < 0` picks the
    /// first free managed channel, matching Ikemen's `-1` behavior.
    #[allow(clippy::too_many_arguments)]
    pub fn play_sound_channel(
        &mut self,
        who: usize,
        group: i32,
        index: i32,
        volume_scale: f32,
        pan: f32,
        channel: i32,
        loop_sound: bool,
        priority: i32,
        low_priority: bool,
        freq_mul: f32,
    ) {
        let Some(fighter) = self.fighters.get(who) else {
            return;
        };
        let Some(character) = self.assets.characters.get(fighter.char_id) else {
            return;
        };
        let Some(sound) = character.sounds.get(group, index) else {
            return;
        };
        let sample_rate = sound.sample_rate.max(1);
        let length = (sound.samples.len() / sound.channels.max(1) as usize).max(1) as i32;
        if self.sound_channels.len() <= who {
            self.sound_channels.resize(who + 1, Vec::new());
        }
        let channels = &mut self.sound_channels[who];
        let channel = if channel < 0 {
            (0..16)
                .find(|number| {
                    !channels
                        .iter()
                        .any(|entry| entry.channel == *number && entry.playing)
                })
                .unwrap_or(0)
        } else {
            channel
        };
        let slot = channel_slot(channels, channel);
        if let Some(existing) = channels.get(slot) {
            if existing.playing {
                if low_priority {
                    return;
                }
                if existing.priority > priority {
                    return;
                }
            }
        }
        channels[slot] = SoundChannelState {
            channel,
            group,
            index,
            volume_scale,
            freq_mul,
            pan,
            priority,
            loop_sound,
            length,
            position: 0,
            sample_rate,
            playing: true,
        };
    }

    /// Stops one channel, or every channel when `channel < 0`.
    pub fn stop_sound_channel(&mut self, who: usize, channel: i32) {
        let Some(channels) = self.sound_channels.get_mut(who) else {
            return;
        };
        for entry in channels {
            if channel < 0 || entry.channel == channel {
                entry.playing = false;
            }
        }
    }

    /// Applies `SndPan` to the sim-side channel state.
    pub fn pan_sound_channel(
        &mut self,
        who: usize,
        channel: i32,
        pan: Option<f32>,
        volume_scale: Option<f32>,
    ) {
        let Some(channels) = self.sound_channels.get_mut(who) else {
            return;
        };
        for entry in channels {
            if channel >= 0 && entry.channel != channel {
                continue;
            }
            if let Some(pan) = pan {
                entry.pan = pan;
            }
            if let Some(volume) = volume_scale {
                entry.volume_scale = volume;
            }
        }
    }

    /// Applies `ModifySnd` to the sim-side channel state, replacing the sample
    /// data when `group`/`index` resolve to another sound.
    #[allow(clippy::too_many_arguments)]
    pub fn modify_sound_channel(
        &mut self,
        who: usize,
        group: i32,
        index: i32,
        channel: i32,
        volume_scale: Option<f32>,
        pan: Option<f32>,
        loop_sound: Option<bool>,
        freq_mul: Option<f32>,
    ) {
        let sound = self
            .fighters
            .get(who)
            .and_then(|fighter| self.assets.characters.get(fighter.char_id))
            .and_then(|character| character.sounds.get(group, index))
            .map(|sound| {
                (
                    sound.sample_rate.max(1),
                    (sound.samples.len() / sound.channels.max(1) as usize).max(1) as i32,
                )
            });
        let Some(channels) = self.sound_channels.get_mut(who) else {
            return;
        };
        for entry in channels {
            if channel >= 0 && entry.channel != channel {
                continue;
            }
            if let Some((sample_rate, length)) = sound {
                if entry.group != group || entry.index != index {
                    entry.position = 0;
                }
                entry.group = group;
                entry.index = index;
                entry.sample_rate = sample_rate;
                entry.length = length;
                entry.playing = true;
            }
            if let Some(volume) = volume_scale {
                entry.volume_scale = volume;
            }
            if let Some(pan) = pan {
                entry.pan = pan;
            }
            if let Some(loop_sound) = loop_sound {
                entry.loop_sound = loop_sound;
            }
            if let Some(freq_mul) = freq_mul {
                entry.freq_mul = freq_mul;
            }
        }
    }

    /// Returns the requested channel, or the first playing channel for a
    /// negative `channel`.
    pub fn sound_channel(&self, who: usize, channel: i32) -> Option<&SoundChannelState> {
        let channels = self.sound_channels.get(who)?;
        if channel < 0 {
            channels.iter().find(|entry| entry.playing)
        } else {
            channels.iter().find(|entry| entry.channel == channel)
        }
    }

    /// Advances every managed channel by one tick.
    pub(super) fn tick_sound_channels(&mut self) {
        for channels in &mut self.sound_channels {
            for entry in channels {
                entry.tick();
            }
        }
    }
}
