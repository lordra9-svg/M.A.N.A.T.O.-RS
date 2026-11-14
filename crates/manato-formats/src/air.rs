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

    /// Whether the frame defines a transparency/blend. A plain frame is
    /// `Normal` with full source alpha and zero destination alpha.
    pub fn has_transparency(&self) -> bool {
        self.blend != BlendMode::Normal || self.src_alpha != 255 || self.dst_alpha != 0
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

/// Draw parameters for one animation frame with `Interpolate` markers
/// applied. `offset`, `scale`, and `angle` blend toward the next frame over
/// the current frame's duration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FramePose {
    pub frame: usize,
    /// Elapsed ticks inside the frame.
    pub elapsed: i32,
    pub offset: Vec2,
    pub scale: Vec2,
    pub angle: f32,
    /// Interpolated source alpha in 0–255 units.
    pub src_alpha: f32,
    /// Interpolated destination alpha in 0–255 units.
    pub dst_alpha: f32,
}

impl Animation {
    /// Pose for the frame at `elapsed`, applying interpolation markers.
    pub fn pose_at(&self, elapsed: i32) -> Option<FramePose> {
        let (frame, frame_time) = self.frame_at(elapsed)?;
        self.interpolated_pose(frame, frame_time)
    }

    /// Pose for a known frame index and elapsed ticks inside it. A marker
    /// recorded before the following frame blends `offset`/`scale`/`angle`
    /// from this frame toward that frame; the last frame blends toward
    /// `loop_start`, matching Ikemen-GO.
    pub fn interpolated_pose(&self, frame_index: usize, elapsed: i32) -> Option<FramePose> {
        let frame = self.frames.get(frame_index)?;
        let mut pose = FramePose {
            frame: frame_index,
            elapsed,
            offset: frame.offset,
            scale: frame.scale,
            angle: frame.angle,
            src_alpha: frame.src_alpha as f32,
            dst_alpha: frame.dst_alpha as f32,
        };
        // Infinite/zero-length frames and single-frame actions do not
        // interpolate.
        if frame.duration <= 0 || elapsed <= 0 || self.frames.len() < 2 {
            return Some(pose);
        }
        let next = if frame_index + 1 < self.frames.len() {
            frame_index + 1
        } else {
            self.loop_start.min(self.frames.len().saturating_sub(1))
        };
        let Some(target) = self.frames.get(next) else {
            return Some(pose);
        };
        let progress = (elapsed as f32 / frame.duration as f32).clamp(0.0, 1.0);
        if progress <= 0.0 {
            return Some(pose);
        }
        if self.interpolate_offset.contains(&next) {
            pose.offset.x += (target.offset.x - frame.offset.x) * progress;
            pose.offset.y += (target.offset.y - frame.offset.y) * progress;
        }
        if self.interpolate_scale.contains(&next) {
            pose.scale.x += (target.scale.x - frame.scale.x) * progress;
            pose.scale.y += (target.scale.y - frame.scale.y) * progress;
        }
        if self.interpolate_angle.contains(&next) {
            pose.angle += (target.angle - frame.angle) * progress;
        }
        // Alpha interpolates only from a frame that actually has a blend,
        // matching Ikemen-GO's `curtrans != TransNone` guard.
        if self.interpolate_blend.contains(&next) && frame.has_transparency() {
            pose.src_alpha += (target.src_alpha as f32 - frame.src_alpha as f32) * progress;
            pose.dst_alpha += (target.dst_alpha as f32 - frame.dst_alpha as f32) * progress;
        }
        Some(pose)
    }

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
    #[test]
    fn parses_subadd_transparency_variants() {
        let animation = &parse_air_str(
            "[Begin Action 0]\n0,0, 0,0, 4, , SAS64d32\n0,1, 0,0, 4, , SA\n0,2, 0,0, 4, , SS128d64",
        )[0];
        let subadd = &animation.frames[0];
        assert_eq!(subadd.blend, BlendMode::SubAdd);
        assert_eq!(subadd.src_alpha, 64);
        assert_eq!(subadd.dst_alpha, 32);
        let plain = &animation.frames[1];
        assert_eq!(plain.blend, BlendMode::SubAdd);
        assert_eq!(plain.src_alpha, 255);
        let sub = &animation.frames[2];
        assert_eq!(sub.blend, BlendMode::Sub);
        assert_eq!(sub.src_alpha, 128);
        assert_eq!(sub.dst_alpha, 64);
    }

    #[test]
    fn pose_at_interpolates_marked_offset_scale_angle() {
        let text = "[Begin Action 0]\n\
                    0,0, 0,0, 4, , , 1,1, 0\n\
                    Interpolate Offset\nInterpolate Scale\nInterpolate Angle\n\
                    0,1, 8,16, 4, , , 2,2, 90\n\
                    0,2, 8,16, 4\n";
        let anim = &parse_air_str(text)[0];

        // Frame 0 start: untouched values.
        let pose = anim.pose_at(0).expect("pose");
        assert_eq!(pose.frame, 0);
        assert_eq!(pose.offset, Vec2::ZERO);
        assert_eq!(pose.scale, Vec2::new(1.0, 1.0));
        assert_eq!(pose.angle, 0.0);

        // Halfway through frame 0: halfway toward frame 1.
        let pose = anim.pose_at(2).expect("pose");
        assert_eq!(pose.frame, 0);
        assert!((pose.offset.x - 4.0).abs() < 1e-5);
        assert!((pose.offset.y - 8.0).abs() < 1e-5);
        assert!((pose.scale.x - 1.5).abs() < 1e-5);
        assert!((pose.scale.y - 1.5).abs() < 1e-5);
        assert!((pose.angle - 45.0).abs() < 1e-5);

        // Frame 1 start: its own values.
        let pose = anim.pose_at(4).expect("pose");
        assert_eq!(pose.frame, 1);
        assert_eq!(pose.offset, Vec2::new(8.0, 16.0));
        assert_eq!(pose.scale, Vec2::new(2.0, 2.0));
        assert_eq!(pose.angle, 90.0);

        // Frame 1 has no marker for frame 2, so values stay fixed.
        let pose = anim.pose_at(6).expect("pose");
        assert_eq!(pose.frame, 1);
        assert_eq!(pose.offset, Vec2::new(8.0, 16.0));
        assert_eq!(pose.scale, Vec2::new(2.0, 2.0));
        assert_eq!(pose.angle, 90.0);
    }

    #[test]
    fn pose_at_wraps_last_frame_interpolation_to_loop_start() {
        // The marker before frame 0 applies when the last frame walks to the
        // loop start.
        let text = "[Begin Action 0]\nloopstart\n\
                    Interpolate Offset\n\
                    0,0, 0,0, 2\n\
                    0,1, 20,10, 2\n";
        let anim = &parse_air_str(text)[0];
        assert_eq!(anim.loop_start, 0);
        assert_eq!(anim.interpolate_offset, vec![0]);

        let pose = anim.pose_at(2).expect("pose");
        assert_eq!(pose.frame, 1);
        assert_eq!(pose.elapsed, 0);
        assert_eq!(pose.offset, Vec2::new(20.0, 10.0));

        let pose = anim.pose_at(3).expect("pose");
        assert_eq!(pose.frame, 1);
        assert!((pose.offset.x - 10.0).abs() < 1e-5);
        assert!((pose.offset.y - 5.0).abs() < 1e-5);
    }

    #[test]
    fn pose_at_interpolates_blend_alphas() {
        let text = "[Begin Action 0]\n\
                    0,0, 0,0, 4, , a1\n\
                    Interpolate Blend\n\
                    0,1, 0,0, 4, , as64d32\n";
        let anim = &parse_air_str(text)[0];
        let frame0 = &anim.frames[0];
        assert!(frame0.has_transparency());
        assert_eq!(anim.interpolate_blend, vec![1]);

        // Frame 0 start keeps its own alpha values.
        let pose = anim.pose_at(0).expect("pose");
        assert_eq!(pose.src_alpha, 255.0);
        assert_eq!(pose.dst_alpha, 128.0);

        // Halfway to the marked frame.
        let pose = anim.pose_at(2).expect("pose");
        assert!((pose.src_alpha - 159.5).abs() < 1e-5);
        assert!((pose.dst_alpha - 80.0).abs() < 1e-5);

        // The marked frame then uses its own values.
        let pose = anim.pose_at(4).expect("pose");
        assert_eq!(pose.src_alpha, 64.0);
        assert_eq!(pose.dst_alpha, 32.0);
    }

    #[test]
    fn pose_at_blend_requires_current_frame_transparency() {
        let text = "[Begin Action 0]\n\
                    0,0, 0,0, 4\n\
                    Interpolate Blend\n\
                    0,1, 0,0, 4, , a1\n";
        let anim = &parse_air_str(text)[0];
        assert!(!anim.frames[0].has_transparency());
        let pose = anim.pose_at(2).expect("pose");
        assert_eq!(pose.src_alpha, 255.0);
        assert_eq!(pose.dst_alpha, 0.0);
    }

    #[test]
    fn pose_at_without_markers_or_for_infinite_frames_is_static() {
        let text = "[Begin Action 0]\n\
                    0,0, 4,6, 4, , , 1,1, 0\n\
                    0,1, 8,10, 4, , , 2,2, 90\n";
        let anim = &parse_air_str(text)[0];
        let pose = anim.pose_at(2).expect("pose");
        assert_eq!(pose.frame, 0);
        assert_eq!(pose.offset, Vec2::new(4.0, 6.0));
        assert_eq!(pose.scale, Vec2::new(1.0, 1.0));
        assert_eq!(pose.angle, 0.0);

        let text = "[Begin Action 0]\n\
                    Interpolate Scale\n\
                    0,0, 0,0, -1, , , 1,1, 0\n\
                    0,1, 0,0, 4, , , 2,2, 90\n";
        let anim = &parse_air_str(text)[0];
        let pose = anim.pose_at(3).expect("pose");
        assert_eq!(pose.frame, 0);
        assert_eq!(pose.scale, Vec2::new(1.0, 1.0));
    }
}
