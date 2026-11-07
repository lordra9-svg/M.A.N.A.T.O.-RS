//! M.U.G.E.N/Ikemen-GO AIR animation parsing.
//!
//! The format and timing rules are specified in `docs/spec/air.md`.

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
            .filter_map(|part| part.trim().parse::<i32>().ok())
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
    SubAdd,
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
    pub src_alpha: u8,
    pub dst_alpha: u8,
    pub scale: Vec2,
    /// Rotation in degrees.
    pub angle: f32,
    pub clsn1: Vec<CollisionBox>,
    pub clsn2: Vec<CollisionBox>,
}

impl AnimFrame {
    pub fn is_infinite(&self) -> bool {
        self.duration < 0
    }

    /// Zero-duration frames are skipped by playback.
    pub fn effective_duration(&self) -> i32 {
        if self.duration < 0 {
            i32::MAX
        } else {
            self.duration.max(0)
        }
    }

    pub fn alpha(&self) -> f32 {
        self.src_alpha as f32 / 255.0
    }
}

#[derive(Debug, Clone, Default)]
pub struct Animation {
    pub number: i32,
    pub frames: Vec<AnimFrame>,
    /// Index of the frame playback returns to after the last frame.
    pub loop_start: usize,
    pub interpolate_offset: Vec<usize>,
    pub interpolate_scale: Vec<usize>,
    pub interpolate_angle: Vec<usize>,
    pub interpolate_blend: Vec<usize>,
    pub copy_action: Option<i32>,
}

impl Animation {
    pub fn total_time(&self) -> i32 {
        self.frames.iter().fold(0, |acc, frame| {
            acc.saturating_add(frame.effective_duration())
        })
    }

    pub fn is_infinite(&self) -> bool {
        self.frames.iter().any(AnimFrame::is_infinite)
    }

    /// Returns `(frame index, elapsed ticks in frame)` for a monotonically
    /// increasing animation clock. Looping actions wrap from `loop_start`.
    pub fn frame_at(&self, elapsed: i32) -> Option<(usize, i32)> {
        if self.frames.is_empty() {
            return None;
        }
        let mut remaining = elapsed.max(0);
        for _ in 0..4 {
            let mut time = 0i32;
            for (index, frame) in self.frames.iter().enumerate() {
                let duration = frame.effective_duration();
                let end = time.saturating_add(duration);
                if remaining < end {
                    return Some((index, remaining - time));
                }
                time = end;
            }
            if self.is_infinite() {
                return Some((self.frames.len() - 1, 0));
            }
            let loop_at = self.loop_start.min(self.frames.len().saturating_sub(1));
            let loop_time = self.frames[..loop_at].iter().fold(0i32, |acc, frame| {
                acc.saturating_add(frame.effective_duration())
            });
            if loop_time <= 0 || remaining < loop_time {
                return Some((self.frames.len() - 1, 0));
            }
            let span = time.saturating_sub(loop_time).max(1);
            remaining = (remaining - loop_time) % span;
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
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
    Ok(parse_air_bytes(&bytes))
}

pub fn parse_air_str(text: &str) -> Vec<Animation> {
    let mut animations = Vec::new();
    let mut builder: Option<AirBuilder> = None;

    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some(done) = builder.take() {
                animations.push(done.finish());
            }
            let header = header.trim().to_ascii_lowercase();
            if let Some(rest) = header.strip_prefix("begin action") {
                if let Ok(number) = rest.trim().parse::<i32>() {
                    builder = Some(AirBuilder::new(number));
                }
            }
            continue;
        }
        if let Some(builder) = builder.as_mut() {
            builder.consume_line(line);
        }
    }
    if let Some(done) = builder.take() {
        animations.push(done.finish());
    }

    resolve_copy_actions(&mut animations);
    animations
}

fn resolve_copy_actions(animations: &mut [Animation]) {
    let numbers: Vec<i32> = animations
        .iter()
        .map(|animation| animation.number)
        .collect();
    for index in 0..animations.len() {
        let Some(source_number) = animations[index].copy_action else {
            continue;
        };
        let Some(source_index) = numbers.iter().rposition(|number| *number == source_number) else {
            log::warn!(
                "AIR: action {} copies missing action {}",
                animations[index].number,
                source_number
            );
            animations[index].copy_action = None;
            continue;
        };
        if source_index == index {
            animations[index].copy_action = None;
            continue;
        }
        let source = animations[source_index].clone();
        let number = animations[index].number;
        animations[index] = source;
        animations[index].number = number;
        animations[index].copy_action = None;
    }
}

struct AirBuilder {
    animation: Animation,
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
            animation: Animation {
                number,
                ..Animation::default()
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
            self.animation.loop_start = self.animation.frames.len();
            return;
        }
        if let Some(rest) = lower.strip_prefix("interpolate") {
            let marker = self.animation.frames.len();
            match rest.trim() {
                "offset" => self.animation.interpolate_offset.push(marker),
                "scale" => self.animation.interpolate_scale.push(marker),
                "angle" => self.animation.interpolate_angle.push(marker),
                "blend" => self.animation.interpolate_blend.push(marker),
                _ => {}
            }
            return;
        }
        if let Some(rest) = lower.strip_prefix("copy action") {
            if let Ok(number) = rest.trim().parse::<i32>() {
                self.animation.copy_action = Some(number);
            }
            return;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_ascii_lowercase();
            if key.starts_with("clsn1") || key.starts_with("clsn2") {
                let is_first = key.starts_with("clsn1");
                let is_default = key.contains("default");
                let count = value
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .filter(|count| *count <= 512)
                    .unwrap_or(0);
                if is_first {
                    if is_default {
                        self.default1.clear();
                        self.default1.reserve(count);
                        self.target1_default = true;
                    } else {
                        self.pending1 = Some(Vec::with_capacity(count));
                        self.target1_default = false;
                    }
                } else if is_default {
                    self.default2.clear();
                    self.default2.reserve(count);
                    self.target2_default = true;
                } else {
                    self.pending2 = Some(Vec::with_capacity(count));
                    self.target2_default = false;
                }
                return;
            }
        }
        if lower.starts_with("clsn1[") || lower.starts_with("clsn2[") {
            let is_first = lower.starts_with("clsn1[");
            let Some((_, value)) = line.split_once('=') else {
                return;
            };
            let Some(box_) = CollisionBox::parse(value) else {
                return;
            };
            if is_first {
                if self.target1_default {
                    self.default1.push(box_);
                } else if let Some(boxes) = self.pending1.as_mut() {
                    boxes.push(box_);
                }
            } else if self.target2_default {
                self.default2.push(box_);
            } else if let Some(boxes) = self.pending2.as_mut() {
                boxes.push(box_);
            }
            return;
        }
        if let Some(mut frame) = parse_frame_line(line) {
            frame.clsn1 = self
                .pending1
                .take()
                .unwrap_or_else(|| self.default1.clone());
            frame.clsn2 = self
                .pending2
                .take()
                .unwrap_or_else(|| self.default2.clone());
            self.animation.frames.push(frame);
        }
    }

    fn finish(self) -> Animation {
        self.animation
    }
}

fn parse_frame_line(line: &str) -> Option<AnimFrame> {
    let parts: Vec<&str> = line.split(',').map(str::trim).collect();
    if parts.len() < 5 {
        return None;
    }
    let group = parts[0].parse::<i32>().ok()?;
    let index = parts[1].parse::<i32>().ok()?;
    let mut x = parts[2].parse::<f32>().ok()?;
    let mut y = parts[3].parse::<f32>().ok()?;
    let duration = parts[4].parse::<i32>().ok()?;

    let mut flip_h = false;
    let mut flip_v = false;
    if let Some(flags) = parts.get(5) {
        for flag in flags.chars() {
            match flag.to_ascii_uppercase() {
                'H' => {
                    flip_h = true;
                    x = -x;
                }
                'V' => {
                    flip_v = true;
                    y = -y;
                }
                _ => {}
            }
        }
    }

    let (blend, src_alpha, dst_alpha) = parts
        .get(6)
        .map(|value| parse_transparency(value))
        .unwrap_or((BlendMode::Normal, 255, 0));

    let scale = Vec2::new(
        parts
            .get(7)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(1.0),
        parts
            .get(8)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(1.0),
    );
    let angle = parts
        .get(9)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(0.0);

    Some(AnimFrame {
        group,
        index,
        offset: Vec2::new(x, y),
        duration,
        flip_h,
        flip_v,
        blend,
        src_alpha,
        dst_alpha,
        scale,
        angle,
        clsn1: Vec::new(),
        clsn2: Vec::new(),
    })
}

fn parse_transparency(value: &str) -> (BlendMode, u8, u8) {
    let lower = value.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return (BlendMode::Normal, 255, 0);
    }
    if lower == "a1" {
        return (BlendMode::Add, 255, 128);
    }
    if let Some(rest) = lower.strip_prefix("sas") {
        let (src, dst) = parse_alpha(rest, 255, 255);
        return (BlendMode::SubAdd, src, dst);
    }
    if let Some(rest) = lower.strip_prefix("sa") {
        let (src, dst) = parse_alpha(rest, 255, 255);
        return (BlendMode::SubAdd, src, dst);
    }
    if let Some(rest) = lower.strip_prefix("as") {
        let (src, dst) = parse_alpha(rest, 255, 0);
        return (BlendMode::Add, src, dst);
    }
    if let Some(rest) = lower.strip_prefix("ss") {
        let (src, dst) = parse_alpha(rest, 255, 255);
        return (BlendMode::Sub, src, dst);
    }
    if lower.starts_with('a') {
        return (BlendMode::Add, 255, 255);
    }
    if lower.starts_with('s') {
        return (BlendMode::Sub, 255, 255);
    }
    (BlendMode::Normal, 255, 0)
}

fn parse_alpha(value: &str, default_src: u8, default_dst: u8) -> (u8, u8) {
    let mut chars = value.chars().peekable();
    let mut src = 0u32;
    let mut has_src = false;
    while let Some(ch) = chars.peek().copied() {
        if !ch.is_ascii_digit() {
            break;
        }
        has_src = true;
        src = src * 10 + (ch as u32 - '0' as u32);
        chars.next();
    }
    let src = if has_src {
        src.min(255) as u8
    } else {
        default_src
    };
    let mut dst = default_dst;
    if chars.next() == Some('d') {
        let mut value = 0u32;
        let mut has_dst = false;
        for ch in chars {
            if !ch.is_ascii_digit() {
                break;
            }
            has_dst = true;
            value = value * 10 + (ch as u32 - '0' as u32);
        }
        if has_dst {
            dst = value.min(255) as u8;
        }
    }
    (src, dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clsn_defaults_and_overrides() {
        let text = r#"
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
        let animations = parse_air_str(text);
        assert_eq!(animations.len(), 1);
        let animation = &animations[0];
        assert_eq!(animation.frames.len(), 4);
        assert_eq!(animation.frames[0].clsn2.len(), 2);
        assert_eq!(animation.frames[2].clsn1.len(), 1);
        assert_eq!(animation.frames[2].clsn2.len(), 1);
        assert_eq!(animation.frames[3].clsn2.len(), 2);
        assert!(animation.frames[3].clsn1.is_empty());
    }

    #[test]
    fn flips_negate_offsets() {
        let animation = &parse_air_str("[Begin Action 0]\n0,0, 10,-20, 4, HV")[0];
        let frame = &animation.frames[0];
        assert!(frame.flip_h && frame.flip_v);
        assert_eq!(frame.offset, Vec2::new(-10.0, 20.0));
    }

    #[test]
    fn parses_transparency_and_scale_angle() {
        let animation = &parse_air_str("[Begin Action 0]\n0,0, 0,0, 4, , AS64d32, 2, 3, 45")[0];
        let frame = &animation.frames[0];
        assert_eq!(frame.blend, BlendMode::Add);
        assert_eq!(frame.src_alpha, 64);
        assert_eq!(frame.dst_alpha, 32);
        assert_eq!(frame.scale, Vec2::new(2.0, 3.0));
        assert_eq!(frame.angle, 45.0);
    }

    #[test]
    fn zero_duration_frames_are_skipped() {
        let animation =
            &parse_air_str("[Begin Action 0]\n0,0, 0,0, 0\n0,1, 0,0, 3\n0,2, 0,0, 2")[0];
        assert_eq!(animation.frame_at(0), Some((1, 0)));
        assert_eq!(animation.frame_at(2), Some((1, 2)));
        assert_eq!(animation.frame_at(3), Some((2, 0)));
    }

    #[test]
    fn copy_action_duplicates_frames() {
        let animations = parse_air_str(
            "[Begin Action 1]\n0,0, 0,0, 2\n0,1, 0,0, 3\n\n[Begin Action 2]\ncopy action 1",
        );
        let copy = animations
            .iter()
            .find(|animation| animation.number == 2)
            .unwrap();
        assert_eq!(copy.frames.len(), 2);
        assert_eq!(copy.frames[0].index, 0);
    }

    #[test]
    fn loopstart_and_infinite_last_frame() {
        let animation = &parse_air_str("[Begin Action 0]\n0,0, 0,0, 2\nloopstart\n0,1, 0,0, -1")[0];
        assert_eq!(animation.loop_start, 1);
        assert!(animation.is_infinite());
        assert_eq!(animation.frame_at(100).map(|(index, _)| index), Some(1));
    }
}
