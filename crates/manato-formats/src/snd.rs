//! M.U.G.E.N/Ikemen-GO SND sound-bank decoding.
//!
//! The layout and conversion rules are specified in `docs/spec/snd.md`.

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
            bail!("sound file is too small");
        }
        if &data[..12] != b"ElecbyteSnd\0" {
            bail!("not an Elecbyte sound file");
        }
        let count = u32_at(data, 16) as usize;
        let offset = u32_at(data, 20) as usize;
        if count > 0 {
            if offset < 24 || offset >= data.len() {
                bail!("sound subheader offset out of bounds");
            }
            if slice(data, offset, 16).is_none() {
                bail!("truncated sound subheader");
            }
        }
        let mut offset = offset;
        let mut bank = Self::default();

        for _ in 0..count.min(65_535) {
            let Some(header) = slice(data, offset, 16) else {
                break;
            };
            let next = u32_at(header, 0) as usize;
            let length = u32_at(header, 4) as usize;
            let group = u16_at(header, 8) as i32;
            let index = u16_at(header, 10) as i32;

            let start = offset.saturating_add(16);
            let declared_end = start.saturating_add(length);
            if length == 0 {
                log::warn!("empty sound entry {group},{index}; skipping");
            } else if declared_end > data.len() {
                log::warn!("truncated sound entry {group},{index}; skipping");
            } else {
                let payload = &data[start..declared_end];
                match decode_wav(payload, group, index) {
                    Ok(sound) => {
                        if bank.by_key.contains_key(&(group, index)) {
                            log::warn!("duplicate sound key {group},{index}; ignoring later entry");
                        } else {
                            bank.by_key.insert((group, index), bank.sounds.len());
                            bank.sounds.push(sound);
                        }
                    }
                    Err(error) => {
                        log::warn!("could not decode sound {group},{index}: {error}");
                    }
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
    if !data.starts_with(b"RIFF") {
        bail!("sound payload is not RIFF/WAVE");
    }
    let mut reader = hound::WavReader::new(Cursor::new(data))?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 {
        bail!("WAV has no channels or sample rate");
    }
    let samples: Vec<i16> = match spec.sample_format {
        hound::SampleFormat::Int => match spec.bits_per_sample {
            0..=8 => reader
                .samples::<i8>()
                .filter_map(Result::ok)
                .map(|s| (s as i16) << 8)
                .collect(),
            9..=16 => reader.samples::<i16>().filter_map(Result::ok).collect(),
            17..=24 => reader
                .samples::<i32>()
                .filter_map(Result::ok)
                .map(|s| (s >> 8) as i16)
                .collect(),
            _ => reader
                .samples::<i32>()
                .filter_map(Result::ok)
                .map(|s| (s >> 16) as i16)
                .collect(),
        },
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .filter_map(Result::ok)
            .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect(),
    };
    Ok(Sound {
        group,
        index,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        samples,
    })
}

fn slice(data: &[u8], offset: usize, length: usize) -> Option<&[u8]> {
    let end = offset.checked_add(length)?;
    data.get(offset..end)
}

fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn wav(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut writer = hound::WavWriter::new(
                Cursor::new(&mut out),
                hound::WavSpec {
                    channels,
                    sample_rate: rate,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            for sample in samples {
                writer.write_sample(*sample).unwrap();
            }
            writer.finalize().unwrap();
        }
        out
    }

    fn wav_u8(samples: &[u8], channels: u16, rate: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let data_len = samples.len() as u32;
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * channels as u32).to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        out.extend_from_slice(samples);
        out
    }

    fn wav_i24(samples: &[i32], channels: u16, rate: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let data_len = (samples.len() * 3) as u32;
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * channels as u32 * 3).to_le_bytes());
        out.extend_from_slice(&(channels * 3).to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            out.extend_from_slice(&sample.to_le_bytes()[..3]);
        }
        out
    }

    fn bank(entries: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
        let mut subheaders = Vec::new();
        let mut payloads = Vec::new();
        let mut offsets = Vec::new();
        let mut cursor = 512usize;
        for (_, _, wav) in entries {
            offsets.push(cursor);
            let mut subheader = vec![0u8; 16];
            subheader[4..8].copy_from_slice(&(wav.len() as u32).to_le_bytes());
            subheaders.push(subheader);
            payloads.push(wav.clone());
            cursor += 16 + wav.len();
        }
        for (i, subheader) in subheaders.iter_mut().enumerate() {
            let next = offsets.get(i + 1).copied().unwrap_or(0) as u32;
            subheader[0..4].copy_from_slice(&next.to_le_bytes());
        }
        for (i, (group, index, _)) in entries.iter().enumerate() {
            subheaders[i][8..10].copy_from_slice(&group.to_le_bytes());
            subheaders[i][10..12].copy_from_slice(&index.to_le_bytes());
        }

        let mut out = vec![0u8; 512];
        out[..12].copy_from_slice(b"ElecbyteSnd\0");
        out[12..14].copy_from_slice(&4u16.to_le_bytes());
        out[16..20].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        out[20..24].copy_from_slice(&(512u32).to_le_bytes());
        for (subheader, payload) in subheaders.iter().zip(payloads.iter()) {
            out.extend_from_slice(subheader);
            out.extend_from_slice(payload);
        }
        out
    }

    #[test]
    fn decodes_stereo_16_bit() {
        let data = bank(&[(1, 2, wav(&[100, -100, 200, -200], 2, 22_050))]);
        let bank = SoundBank::from_bytes(&data).expect("bank");
        let sound = bank.get(1, 2).expect("sound");
        assert_eq!(sound.channels, 2);
        assert_eq!(sound.sample_rate, 22_050);
        assert_eq!(sound.samples, [100, -100, 200, -200]);
    }

    #[test]
    fn duplicate_keys_keep_first() {
        let data = bank(&[
            (0, 0, wav(&[1, 2], 1, 8_000)),
            (0, 0, wav(&[3, 4], 1, 8_000)),
        ]);
        let bank = SoundBank::from_bytes(&data).expect("bank");
        assert_eq!(bank.get(0, 0).unwrap().samples, [1, 2]);
    }

    #[test]
    fn rejects_non_riff_payload() {
        let mut data = bank(&[]);
        data.extend_from_slice(&[0u8; 16]);
        let mut bad = vec![0u8; 512];
        bad[..12].copy_from_slice(b"ElecbyteSnd\0");
        bad[16..20].copy_from_slice(&1u32.to_le_bytes());
        bad[20..24].copy_from_slice(&512u32.to_le_bytes());
        bad.extend_from_slice(&[0u8; 16]);
        bad[512 + 4..512 + 8].copy_from_slice(&4u32.to_le_bytes());
        assert!(SoundBank::from_bytes(&bad).unwrap().sounds.is_empty());
    }
    #[test]
    fn converts_unsigned_8_bit_mono_and_stereo() {
        let bytes = [128u8, 255, 0, 129];
        let mono =
            SoundBank::from_bytes(&bank(&[(2, 1, wav_u8(&bytes, 1, 8_000))])).expect("mono bank");
        let sound = mono.get(2, 1).expect("mono sound");
        assert_eq!(sound.channels, 1);
        assert_eq!(sound.sample_rate, 8_000);
        assert_eq!(sound.samples, [0, 32512, -32768, 256]);

        let stereo = SoundBank::from_bytes(&bank(&[(2, 2, wav_u8(&bytes, 2, 44_100))]))
            .expect("stereo bank");
        let sound = stereo.get(2, 2).expect("stereo sound");
        assert_eq!(sound.channels, 2);
        assert_eq!(sound.sample_rate, 44_100);
        assert_eq!(sound.samples, [0, 32512, -32768, 256]);
    }

    #[test]
    fn converts_signed_24_bit_samples() {
        let samples = [0, 256, 0x7F_FFFF, -0x80_0000];
        let data = bank(&[(3, 4, wav_i24(&samples, 1, 22_050))]);
        let bank = SoundBank::from_bytes(&data).expect("bank");
        let sound = bank.get(3, 4).expect("sound");
        assert_eq!(sound.samples, [0, 1, 32767, -32768]);
    }

    fn wav_i32(samples: &[i32], channels: u16, rate: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut writer = hound::WavWriter::new(
                Cursor::new(&mut out),
                hound::WavSpec {
                    channels,
                    sample_rate: rate,
                    bits_per_sample: 32,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            for sample in samples {
                writer.write_sample(*sample).unwrap();
            }
            writer.finalize().unwrap();
        }
        out
    }

    fn wav_f32(samples: &[f32], channels: u16, rate: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut writer = hound::WavWriter::new(
                Cursor::new(&mut out),
                hound::WavSpec {
                    channels,
                    sample_rate: rate,
                    bits_per_sample: 32,
                    sample_format: hound::SampleFormat::Float,
                },
            )
            .unwrap();
            for sample in samples {
                writer.write_sample(*sample).unwrap();
            }
            writer.finalize().unwrap();
        }
        out
    }

    #[test]
    fn converts_signed_32_bit_and_float_samples() {
        let int_data = bank(&[(5, 6, wav_i32(&[0, 1 << 16, i32::MAX, i32::MIN], 1, 44_100))]);
        let sounds = SoundBank::from_bytes(&int_data).expect("32-bit bank");
        let sound = sounds.get(5, 6).expect("sound");
        assert_eq!(sound.samples, [0, 1, 32767, -32768]);

        let float_data = bank(&[(7, 8, wav_f32(&[0.0, 0.5, -0.5, 2.0, -2.0], 1, 48_000))]);
        let sounds = SoundBank::from_bytes(&float_data).expect("float bank");
        let sound = sounds.get(7, 8).expect("sound");
        assert_eq!(sound.samples, [0, 16383, -16383, 32767, -32767]);
    }

    #[test]
    fn rejects_out_of_bounds_first_subheader() {
        let mut data = vec![0u8; 24];
        data[..12].copy_from_slice(b"ElecbyteSnd\0");
        data[16..20].copy_from_slice(&1u32.to_le_bytes());
        data[20..24].copy_from_slice(&999_999u32.to_le_bytes());
        assert!(SoundBank::from_bytes(&data).is_err());
    }

    #[test]
    fn rejects_truncated_first_subheader() {
        let mut data = vec![0u8; 32];
        data[..12].copy_from_slice(b"ElecbyteSnd\0");
        data[16..20].copy_from_slice(&1u32.to_le_bytes());
        data[20..24].copy_from_slice(&28u32.to_le_bytes());
        assert!(SoundBank::from_bytes(&data).is_err());
    }

    #[test]
    fn empty_bank_is_ok() {
        let mut data = vec![0u8; 24];
        data[..12].copy_from_slice(b"ElecbyteSnd\0");
        data[16..20].copy_from_slice(&0u32.to_le_bytes());
        let bank = SoundBank::from_bytes(&data).expect("empty bank");
        assert!(bank.sounds.is_empty());
    }

    #[test]
    fn skips_truncated_entry_and_keeps_later_sounds() {
        let second = wav(&[7, 8], 1, 8_000);
        let mut data = vec![0u8; 24];
        data[..12].copy_from_slice(b"ElecbyteSnd\0");
        data[16..20].copy_from_slice(&2u32.to_le_bytes());
        data[20..24].copy_from_slice(&24u32.to_le_bytes());

        // First entry claims more data than the file holds.
        let mut first = vec![0u8; 16];
        first[0..4].copy_from_slice(&44u32.to_le_bytes());
        first[4..8].copy_from_slice(&1000u32.to_le_bytes());
        first[8..10].copy_from_slice(&0u16.to_le_bytes());
        first[10..12].copy_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&first);
        data.extend_from_slice(&[0u8; 4]);

        // Second entry is valid.
        let mut second_header = vec![0u8; 16];
        second_header[4..8].copy_from_slice(&(second.len() as u32).to_le_bytes());
        second_header[8..10].copy_from_slice(&1u16.to_le_bytes());
        second_header[10..12].copy_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&second_header);
        data.extend_from_slice(&second);

        let bank = SoundBank::from_bytes(&data).expect("bank");
        assert_eq!(bank.sounds.len(), 1);
        assert_eq!(bank.get(1, 1).expect("second sound").samples, [7, 8]);
        assert!(bank.get(0, 0).is_none());
    }
}
