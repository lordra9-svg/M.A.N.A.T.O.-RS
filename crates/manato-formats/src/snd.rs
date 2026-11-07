use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use anyhow::{bail, Context};

/// A decoded PCM sound.
#[derive(Debug, Clone)]
pub struct Sound {
    pub group: i32,
    pub index: i32,
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved signed 16-bit samples.
    pub samples: Vec<i16>,
}

impl Sound {
    pub fn duration_seconds(&self) -> f32 {
        if self.sample_rate == 0 || self.channels == 0 {
            return 0.0;
        }
        self.samples.len() as f32 / (self.sample_rate as f32 * self.channels as f32)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SoundBank {
    pub sounds: Vec<Sound>,
    by_key: HashMap<(i32, i32), usize>,
}

impl SoundBank {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let data = std::fs::read(path)
            .with_context(|| format!("failed to read sound file {}", path.display()))?;
        Self::from_bytes(&data)
    }

    pub fn get(&self, group: i32, index: i32) -> Option<&Sound> {
        self.by_key.get(&(group, index)).map(|&i| &self.sounds[i])
    }

    pub fn from_bytes(data: &[u8]) -> anyhow::Result<Self> {
        if data.len() < 24 {
            bail!("sound file too small");
        }
        if &data[..12] != b"ElecbyteSnd\0" {
            bail!("not an Elecbyte sound file");
        }
        let count = read_u32(data, 16) as usize;
        let mut offset = read_u32(data, 20) as usize;
        let mut bank = Self::default();

        for _ in 0..count.min(65_535) {
            if offset == 0 || offset + 16 > data.len() {
                break;
            }
            let entry = &data[offset..offset + 16];
            let next = read_u32(entry, 0) as usize;
            let length = read_u32(entry, 4) as usize;
            let group = read_u16(entry, 8) as i32;
            let index = read_u16(entry, 10) as i32;

            let start = offset + 16;
            let end = start.saturating_add(length).min(data.len());
            if end > start {
                match decode_wav(&data[start..end], group, index) {
                    Ok(sound) => {
                        bank.by_key
                            .entry((group, index))
                            .or_insert(bank.sounds.len());
                        bank.sounds.push(sound);
                    }
                    Err(e) => log::warn!("could not decode sound {group},{index}: {e}"),
                }
            }

            if next == 0 || next <= offset {
                break;
            }
            offset = next;
        }
        Ok(bank)
    }
}

fn decode_wav(data: &[u8], group: i32, index: i32) -> anyhow::Result<Sound> {
    if data.starts_with(b"RIFF") {
        let mut reader = hound::WavReader::new(Cursor::new(data))?;
        let spec = reader.spec();
        let samples: Vec<i16> = match spec.sample_format {
            hound::SampleFormat::Int => {
                if spec.bits_per_sample <= 8 {
                    reader
                        .samples::<i8>()
                        .filter_map(Result::ok)
                        .map(|s| (s as i16) << 8)
                        .collect()
                } else if spec.bits_per_sample <= 16 {
                    reader.samples::<i16>().filter_map(Result::ok).collect()
                } else {
                    reader
                        .samples::<i32>()
                        .filter_map(Result::ok)
                        .map(|s| (s >> 16) as i16)
                        .collect()
                }
            }
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .filter_map(Result::ok)
                .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .collect(),
        };
        return Ok(Sound {
            group,
            index,
            sample_rate: spec.sample_rate,
            channels: spec.channels,
            samples,
        });
    }

    // Raw PCM fallback (some tools emit headerless 16-bit mono data).
    Ok(Sound {
        group,
        index,
        sample_rate: 44_100,
        channels: 1,
        samples: data
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect(),
    })
}

fn read_u16(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn read_u32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}
