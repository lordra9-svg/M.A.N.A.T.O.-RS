use manato_core::{Rect, Vec2};

use crate::text::{decode_text, strip_comment};

/// A single collision box from an AIR action. Coordinates are in the
/// character's local space (y grows downward, feet at y=0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionBox {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

impl CollisionBox {
    pub fn rect(self) -> Rect {
        Rect::new(self.x1, self.y1, self.x2, self.y2).normalized()
    }

    pub fn parse(s: &str) -> Option<Self> {
        let nums: Vec<i32> = s
            .split(',')
            .filter_map(|p| p.trim().parse::<i32>().ok())
            .collect();
        if nums.len() < 4 {
            return None;
        }
        Some(Self {
            x1: nums[0],
            y1: nums[1],
            x2: nums[2],
            y2: nums[3],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Sub,
}

#[derive(Debug, Clone)]
pub struct AnimFrame {
    pub group: i32,
    pub index: i32,
    pub offset: Vec2,
    /// Duration in game ticks. `-1` means the frame is displayed forever.
    pub duration: i32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub blend: BlendMode,
    pub clsn1: Vec<CollisionBox>,
    pub clsn2: Vec<CollisionBox>,
    pub interpolate: bool,
}

impl AnimFrame {
    pub fn is_infinite(&self) -> bool {
        self.duration < 0
    }

    pub fn effective_duration(&self) -> i32 {
        if self.duration < 0 {
            i32::MAX
        } else {
            self.duration.max(1)
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Animation {
    pub number: i32,
    pub frames: Vec<AnimFrame>,
    /// Index of the frame playback returns to after the last frame.
    pub loop_start: usize,
    pub interpolate: bool,
}

impl Animation {
    pub fn total_time(&self) -> i32 {
        self.frames
            .iter()
            .fold(0, |acc, f| acc.saturating_add(f.effective_duration()))
    }

    pub fn is_infinite(&self) -> bool {
        self.frames.iter().any(|f| f.duration < 0)
    }

    /// Returns `(frame index, elapsed ticks in frame)` for a monotonically
    /// increasing animation clock. Looping actions wrap from `loop_start`.
    pub fn frame_at(&self, elapsed: i32) -> Option<(usize, i32)> {
        if self.frames.is_empty() {
            return None;
        }
        let mut remaining = elapsed.max(0);
        for _ in 0..4 {
            let mut t: i32 = 0;
            for (i, f) in self.frames.iter().enumerate() {
                let d = f.effective_duration();
                let end = t.saturating_add(d);
                if remaining < end {
                    return Some((i, remaining - t));
                }
                t = end;
            }
            if self.is_infinite() {
                // The final frame never advances.
                return Some((self.frames.len() - 1, 0));
            }
            let loop_at = self.loop_start.min(self.frames.len().saturating_sub(1));
            let loop_time = self.frames[..loop_at]
                .iter()
                .fold(0i32, |acc, f| acc.saturating_add(f.effective_duration()));
            if loop_time <= 0 || remaining < loop_time {
                return Some((self.frames.len() - 1, 0));
            }
            remaining = (remaining - loop_time) % (t - loop_time).max(1);
        }
        Some((self.frames.len() - 1, 0))
    }
}

/// Parses one or more `[Begin Action n]` blocks.
pub fn parse_air_bytes(bytes: &[u8]) -> Vec<Animation> {
    parse_air_str(&decode_text(bytes))
}

pub fn parse_air(path: &std::path::Path) -> anyhow::Result<Vec<Animation>> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
    Ok(parse_air_bytes(&bytes))
}

pub fn parse_air_str(text: &str) -> Vec<Animation> {
    let mut out = Vec::new();
    let mut current: Option<AirBuilder> = None;

    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some(b) = current.take() {
                out.push(b.finish());
            }
            if let Some(rest) = inner
                .trim()
                .strip_prefix("Begin Action")
                .or_else(|| inner.trim().strip_prefix("begin action"))
            {
                if let Ok(number) = rest.trim().parse::<i32>() {
                    current = Some(AirBuilder::new(number));
                }
            }
            continue;
        }
        if let Some(b) = current.as_mut() {
            b.consume_line(line);
        }
    }
    if let Some(b) = current.take() {
        out.push(b.finish());
    }
    out
}

struct AirBuilder {
    anim: Animation,
    default1: Vec<CollisionBox>,
    default2: Vec<CollisionBox>,
    pending1: Option<Vec<CollisionBox>>,
    pending2: Option<Vec<CollisionBox>>,
    target1_default: bool,
    target2_default: bool,
}

impl AirBuilder {
    fn new(number: i32) -> Self {
        Self {
            anim: Animation {
                number,
                frames: Vec::new(),
                loop_start: 0,
                interpolate: false,
            },
            default1: Vec::new(),
            default2: Vec::new(),
            pending1: None,
            pending2: None,
            target1_default: true,
            target2_default: true,
        }
    }

    fn consume_line(&mut self, line: &str) {
        let lower = line.to_ascii_lowercase();
        if lower == "loopstart" {
            self.anim.loop_start = self.anim.frames.len();
            return;
        }
        if lower == "interpolate" {
            self.anim.interpolate = true;
            return;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_ascii_lowercase();
            if key.starts_with("clsn1") || key.starts_with("clsn2") {
                let is_1 = key.starts_with("clsn1");
                let is_default = key.contains("default");
                let count = value
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .filter(|n| *n <= 512)
                    .unwrap_or(0);
                if is_1 {
                    if is_default {
                        self.default1.clear();
                        self.target1_default = true;
                    } else {
                        self.pending1 = Some(Vec::with_capacity(count));
                        self.target1_default = false;
                    }
                } else if is_default {
                    self.default2.clear();
                    self.target2_default = true;
                } else {
                    self.pending2 = Some(Vec::with_capacity(count));
                    self.target2_default = false;
                }
                return;
            }
        }
        if lower.starts_with("clsn1[") || lower.starts_with("clsn2[") {
            let is_1 = lower.starts_with("clsn1[");
            let Some((_, value)) = line.split_once('=') else {
                return;
            };
            let Some(bx) = CollisionBox::parse(value) else {
                return;
            };
            if is_1 {
                if self.target1_default {
                    self.default1.push(bx);
                } else if let Some(v) = self.pending1.as_mut() {
                    v.push(bx);
                }
            } else if self.target2_default {
                self.default2.push(bx);
            } else if let Some(v) = self.pending2.as_mut() {
                v.push(bx);
            }
            return;
        }
        if let Some(frame) = parse_frame_line(line) {
            let (clsn1, clsn2) = if !self.pending1.is_none() || !self.pending2.is_none() {
                (
                    self.pending1
                        .take()
                        .unwrap_or_else(|| self.default1.clone()),
                    self.pending2
                        .take()
                        .unwrap_or_else(|| self.default2.clone()),
                )
            } else {
                (self.default1.clone(), self.default2.clone())
            };
            let mut frame = frame;
            frame.clsn1 = clsn1;
            frame.clsn2 = clsn2;
            self.anim.frames.push(frame);
        }
    }

    fn finish(self) -> Animation {
        self.anim
    }
}

fn parse_frame_line(line: &str) -> Option<AnimFrame> {
    let parts: Vec<&str> = line.split(',').map(str::trim).collect();
    if parts.len() < 5 {
        return None;
    }
    let group = parts[0].parse::<i32>().ok()?;
    let index = parts[1].parse::<i32>().ok()?;
    let x = parts[2].parse::<f32>().ok()?;
    let y = parts[3].parse::<f32>().ok()?;
    let duration = parts[4].parse::<i32>().ok()?;

    let mut flip_h = false;
    let mut flip_v = false;
    let mut blend = BlendMode::Normal;
    let mut interpolate = false;
    for part in parts.iter().skip(5) {
        let p = part.to_ascii_uppercase();
        match p.as_str() {
            "H" => flip_h = true,
            "V" => flip_v = true,
            "HV" | "VH" => {
                flip_h = true;
                flip_v = true;
            }
            "A" => blend = BlendMode::Add,
            "S" => blend = BlendMode::Sub,
            "AS" | "SA" => blend = BlendMode::Add,
            "I" => interpolate = true,
            _ => {}
        }
    }
    Some(AnimFrame {
        group,
        index,
        offset: Vec2::new(x, y),
        duration,
        flip_h,
        flip_v,
        blend,
        clsn1: Vec::new(),
        clsn2: Vec::new(),
        interpolate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clsn_defaults_and_overrides() {
        let txt = r#"
[Begin Action 200]
Clsn2Default: 2
 Clsn2[0] = -10, 0, 19,-80
 Clsn2[1] = 0,-94, 12,-80
200,0, 0,0, 2
200,1, 0,0, 1
Clsn1: 1
 Clsn1[0] = 16,-80, 61,-71
Clsn2: 1
 Clsn2[0] = 19,0,-10,-80
200,2, 0,0, 4
200,1, 0,0, 3
"#;
        let anims = parse_air_str(txt);
        assert_eq!(anims.len(), 1);
        let a = &anims[0];
        assert_eq!(a.frames.len(), 4);
        assert_eq!(a.frames[0].clsn2.len(), 2);
        assert_eq!(a.frames[2].clsn1.len(), 1);
        assert_eq!(a.frames[2].clsn2.len(), 1);
        assert_eq!(a.frames[3].clsn2.len(), 2);
        assert!(a.frames[3].clsn1.is_empty());
    }
}
