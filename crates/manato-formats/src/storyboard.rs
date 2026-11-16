//! Storyboard DEF scenes (`[SceneDef]` / `[Scene ...]`) with inline AIR.
//!
//! This is the in-engine storyboard player used for intro/ending scenes. Lua
//! scripting, 3D models, dynamic BGs, and custom fonts are not modeled.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use manato_core::Vec2;

use crate::air::{parse_air_str, Animation};
use crate::sff::SffFile;
use crate::snd::SoundBank;
use crate::text::{parse_pair, TextFile};

#[derive(Debug, Clone, Default)]
pub struct Fade {
    pub time: i32,
    pub color: [u8; 3],
    pub alpha: [u8; 2],
}

#[derive(Debug, Clone)]
pub struct StoryboardSound {
    pub group: i32,
    pub index: i32,
    pub start_time: i32,
    pub volume: f32,
    pub pan: f32,
}

#[derive(Debug, Clone)]
pub struct StoryboardLayer {
    pub sprite: Option<(i32, i32)>,
    pub anim: i32,
    pub offset: Vec2,
    pub scale: Vec2,
    pub facing: i32,
    pub layer_no: i32,
    pub start_time: i32,
    /// `-1` runs until the scene ends.
    pub end_time: i32,
    pub velocity: Vec2,
    pub accel: Vec2,
    pub friction: Vec2,
    pub text: Option<String>,
    pub font: i32,
    pub text_delay: f32,
    pub color: [u8; 4],
}

impl Default for StoryboardLayer {
    fn default() -> Self {
        Self {
            sprite: None,
            anim: -1,
            offset: Vec2::ZERO,
            scale: Vec2::new(1.0, 1.0),
            facing: 1,
            layer_no: 2,
            start_time: 0,
            end_time: -1,
            velocity: Vec2::ZERO,
            accel: Vec2::ZERO,
            friction: Vec2::new(1.0, 1.0),
            text: None,
            font: -1,
            text_delay: 2.0,
            color: [255, 255, 255, 255],
        }
    }
}

impl StoryboardLayer {
    pub fn visible_at(&self, time: i32) -> bool {
        time >= self.start_time && (self.end_time < 0 || time < self.end_time)
    }

    /// Reveals `textdelay` ticks per character.
    pub fn visible_text(&self, time: i32) -> String {
        let Some(text) = &self.text else {
            return String::new();
        };
        if self.text_delay <= 0.0 {
            return text.clone();
        }
        let elapsed = (time - self.start_time).max(0) as f32;
        let count = (elapsed / self.text_delay) as usize;
        text.chars().take(count).collect()
    }

    /// Offset after integrating velocity/acceleration (friction ignored).
    pub fn position_at(&self, time: i32) -> Vec2 {
        let t = (time - self.start_time).max(0) as f32;
        self.offset + self.velocity * t + self.accel * (0.5 * t * t)
    }
}

#[derive(Debug, Clone, Default)]
pub struct StoryboardScene {
    pub name: String,
    pub end_time: i32,
    pub clear_color: Option<[u8; 3]>,
    pub clear_alpha: u8,
    pub fade_in: Fade,
    pub fade_out: Fade,
    pub jump: Option<String>,
    pub layers: Vec<StoryboardLayer>,
    pub sounds: Vec<StoryboardSound>,
}

#[derive(Debug, Clone)]
pub struct Storyboard {
    pub path: PathBuf,
    pub name: String,
    pub author: String,
    pub start_scene: usize,
    pub sprites: SffFile,
    pub animations: HashMap<i32, Animation>,
    pub scenes: Vec<StoryboardScene>,
    /// `[SceneDef] fontN` files (index → path).
    pub font_files: BTreeMap<i32, PathBuf>,
    /// `[SceneDef] snd` bank.
    pub sounds: SoundBank,
}

impl Storyboard {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = TextFile::load(path)?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let raw = std::fs::read(path)?;
        let raw = crate::text::decode_text(&raw);
        let animations: HashMap<i32, Animation> = parse_air_str(&raw)
            .into_iter()
            .map(|anim| (anim.number, anim))
            .collect();

        let info = text.find("info");
        let get = |section: Option<&crate::text::Section>, key: &str| {
            section.and_then(|section| {
                section
                    .entries
                    .iter()
                    .find(|entry| entry.key.eq_ignore_ascii_case(key))
                    .map(|entry| entry.value.trim().to_string())
            })
        };
        let scene_def = text.find("scenedef");
        let name = get(info, "name").unwrap_or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("storyboard")
                .to_string()
        });
        let author = get(info, "author").unwrap_or_default();
        let start_scene = get(scene_def, "startscene")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let sprite_path = get(scene_def, "spr").map(|name| resolve(&dir, &name));
        let sprites = match sprite_path {
            Some(path) if path.exists() => SffFile::load(&path)?,
            _ => SffFile::default(),
        };
        let sound_path = get(scene_def, "snd").map(|name| resolve(&dir, &name));
        let sounds = match sound_path {
            Some(path) if path.exists() => SoundBank::load(&path)?,
            _ => SoundBank::default(),
        };
        let mut font_files = BTreeMap::new();
        if let Some(section) = scene_def {
            for entry in &section.entries {
                let key = entry.key.trim().to_ascii_lowercase();
                let Some(suffix) = key.strip_prefix("font") else {
                    continue;
                };
                let Ok(index) = suffix.parse::<i32>() else {
                    continue;
                };
                let value = entry.value.trim();
                if !value.is_empty() {
                    font_files.insert(index, resolve(&dir, value));
                }
            }
        }

        let mut scenes = Vec::new();
        for section in &text.sections {
            let header = section.header.trim();
            let lower = header.to_ascii_lowercase();
            if !lower.starts_with("scene") || lower == "scenedef" {
                continue;
            }
            let scene_name = header[5..]
                .trim()
                .trim_start_matches('_')
                .trim()
                .to_string();
            let mut scene = StoryboardScene {
                name: scene_name.clone(),
                end_time: -1,
                ..StoryboardScene::default()
            };
            let mut layers: Vec<(usize, StoryboardLayer)> = Vec::new();
            let mut scene_sounds: Vec<(usize, StoryboardSound)> = Vec::new();
            for entry in &section.entries {
                let key = entry.key.trim().to_ascii_lowercase();
                let value = entry.value.trim();
                match key.as_str() {
                    "end.time" => scene.end_time = value.parse().unwrap_or(-1),
                    "jump" => scene.jump = Some(value.to_string()),
                    "clearcolor" => {
                        let color = parse_triplet(value);
                        scene.clear_color =
                            Some([clamp_u8(color[0]), clamp_u8(color[1]), clamp_u8(color[2])]);
                    }
                    "clearalpha" => {
                        let (first, _) = parse_pair(value);
                        scene.clear_alpha = clamp_u8(first);
                    }
                    "fadein.time" => scene.fade_in.time = value.parse().unwrap_or(0),
                    "fadeout.time" => scene.fade_out.time = value.parse().unwrap_or(0),
                    "fadein.color" => scene.fade_in.color = triplet_color(value),
                    "fadeout.color" => scene.fade_out.color = triplet_color(value),
                    "fadein.alpha" => scene.fade_in.alpha = alpha_pair(value),
                    "fadeout.alpha" => scene.fade_out.alpha = alpha_pair(value),
                    _ => {
                        if let Some(rest) = key.strip_prefix("sound") {
                            let Some((index, field)) = rest.split_once('.') else {
                                continue;
                            };
                            let Ok(index) = index.parse::<usize>() else {
                                continue;
                            };
                            let sound = match scene_sounds.iter_mut().find(|(i, _)| *i == index) {
                                Some((_, sound)) => sound,
                                None => {
                                    scene_sounds.push((
                                        index,
                                        StoryboardSound {
                                            group: -1,
                                            index: 0,
                                            start_time: 0,
                                            volume: 100.0,
                                            pan: 0.0,
                                        },
                                    ));
                                    &mut scene_sounds.last_mut().unwrap().1
                                }
                            };
                            match field {
                                "value" => {
                                    let (group, sound_index) = parse_pair(value);
                                    sound.group = group as i32;
                                    sound.index = sound_index as i32;
                                }
                                "starttime" => sound.start_time = value.parse().unwrap_or(0),
                                "volume" | "volumescale" => {
                                    sound.volume = value.parse().unwrap_or(100.0)
                                }
                                "pan" => sound.pan = value.parse().unwrap_or(0.0),
                                _ => {}
                            }
                            continue;
                        }
                        let Some(rest) = key.strip_prefix("layer") else {
                            continue;
                        };
                        let Some((index, field)) = rest.split_once('.') else {
                            continue;
                        };
                        let Ok(index) = index.parse::<usize>() else {
                            continue;
                        };
                        let layer = match layers.iter_mut().find(|(i, _)| *i == index) {
                            Some((_, layer)) => layer,
                            None => {
                                layers.push((index, StoryboardLayer::default()));
                                &mut layers.last_mut().unwrap().1
                            }
                        };
                        match field {
                            "anim" => layer.anim = value.parse().unwrap_or(-1),
                            "spr" => layer.sprite = Some(pair_i32(value)),
                            "offset" => layer.offset = pair_vec(value),
                            "scale" => layer.scale = pair_vec(value),
                            "facing" => layer.facing = value.parse().unwrap_or(1),
                            "layerno" => layer.layer_no = value.parse().unwrap_or(2),
                            "starttime" => layer.start_time = value.parse().unwrap_or(0),
                            "endtime" => layer.end_time = value.parse().unwrap_or(-1),
                            "velocity" => layer.velocity = pair_vec(value),
                            "accel" => layer.accel = pair_vec(value),
                            "friction" => layer.friction = pair_vec(value),
                            "text" => layer.text = Some(unquote(value)),
                            "textdelay" => layer.text_delay = value.parse().unwrap_or(2.0),
                            "font" => {
                                let parts: Vec<i32> = value
                                    .split(',')
                                    .map(|part| part.trim().parse().unwrap_or(-1))
                                    .collect();
                                layer.font = parts.first().copied().unwrap_or(-1);
                                if parts.len() >= 8 {
                                    layer.color = [
                                        clamp_u8(parts[3] as f32),
                                        clamp_u8(parts[4] as f32),
                                        clamp_u8(parts[5] as f32),
                                        clamp_u8(parts[6] as f32),
                                    ];
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            layers.sort_by_key(|(index, _)| *index);
            scene.layers = layers.into_iter().map(|(_, layer)| layer).collect();
            scene_sounds.sort_by_key(|(index, _)| *index);
            scene.sounds = scene_sounds.into_iter().map(|(_, sound)| sound).collect();
            scenes.push(scene);
        }

        let start_scene = start_scene.min(scenes.len().saturating_sub(1));
        Ok(Self {
            path: path.to_path_buf(),
            name,
            author,
            start_scene,
            sprites,
            animations,
            scenes,
            font_files,
            sounds,
        })
    }

    pub fn scene(&self, name: &str) -> Option<usize> {
        let wanted = name.to_ascii_lowercase().replace(' ', "_");
        self.scenes
            .iter()
            .position(|scene| scene.name.to_ascii_lowercase().replace(' ', "_") == wanted)
    }
}

/// Storyboard playback state.
#[derive(Debug, Clone)]
pub struct StoryboardPlayer {
    pub scene: usize,
    pub time: i32,
    pub done: bool,
    pending_sounds: Vec<(i32, i32, f32, f32)>,
}

impl StoryboardPlayer {
    pub fn new(storyboard: &Storyboard) -> Self {
        Self {
            scene: storyboard.start_scene,
            time: 0,
            done: storyboard.scenes.is_empty(),
            pending_sounds: Vec::new(),
        }
    }

    /// Sound events triggered since the previous tick.
    pub fn take_sounds(&mut self) -> Vec<(i32, i32, f32, f32)> {
        std::mem::take(&mut self.pending_sounds)
    }

    pub fn current<'a>(&self, storyboard: &'a Storyboard) -> Option<&'a StoryboardScene> {
        storyboard.scenes.get(self.scene)
    }

    /// Advances one tick and moves to the next scene/jump when the current
    /// scene's `end.time` elapses.
    pub fn tick(&mut self, storyboard: &Storyboard) {
        if self.done {
            return;
        }
        self.time += 1;
        self.pending_sounds.clear();
        let Some(scene) = storyboard.scenes.get(self.scene) else {
            self.done = true;
            return;
        };
        for sound in &scene.sounds {
            if sound.start_time == self.time && sound.group >= 0 {
                self.pending_sounds
                    .push((sound.group, sound.index, sound.volume, sound.pan));
            }
        }
        if scene.end_time <= 0 || self.time < scene.end_time {
            return;
        }
        let next = scene
            .jump
            .as_ref()
            .and_then(|name| storyboard.scene(name))
            .unwrap_or(self.scene + 1);
        if next >= storyboard.scenes.len() || next == self.scene {
            self.done = true;
        } else {
            self.scene = next;
            self.time = 0;
        }
    }

    /// Overlay alpha for the fade in/out at the current time.
    pub fn fade_alpha(&self, scene: &StoryboardScene) -> Option<([u8; 3], u8)> {
        if scene.fade_in.time > 0 && self.time < scene.fade_in.time {
            let alpha = 255 - (255.0 * self.time as f32 / scene.fade_in.time as f32) as i32;
            return Some((scene.fade_in.color, alpha.clamp(0, 255) as u8));
        }
        if scene.fade_out.time > 0 && scene.end_time > 0 {
            let start = scene.end_time - scene.fade_out.time;
            if self.time >= start {
                let alpha =
                    (255.0 * (self.time - start) as f32 / scene.fade_out.time as f32) as i32;
                return Some((scene.fade_out.color, alpha.clamp(0, 255) as u8));
            }
        }
        None
    }
}

fn resolve(dir: &Path, name: &str) -> PathBuf {
    let path = Path::new(name);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        dir.join(path)
    }
}

fn parse_triplet(value: &str) -> [f32; 3] {
    let mut parts = value.split(',');
    let mut next = || {
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or(0.0)
    };
    [next(), next(), next()]
}

fn triplet_color(value: &str) -> [u8; 3] {
    let color = parse_triplet(value);
    [clamp_u8(color[0]), clamp_u8(color[1]), clamp_u8(color[2])]
}

fn alpha_pair(value: &str) -> [u8; 2] {
    let (first, second) = parse_pair(value);
    [clamp_u8(first), clamp_u8(second)]
}

fn pair_vec(value: &str) -> Vec2 {
    let (x, y) = parse_pair(value);
    Vec2::new(x, y)
}

fn pair_i32(value: &str) -> (i32, i32) {
    let (x, y) = parse_pair(value);
    (x as i32, y as i32)
}

fn clamp_u8(value: f32) -> u8 {
    value.clamp(0.0, 255.0) as u8
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches('"').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_storyboard(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("manato-story-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let def = dir.join("story.def");
        std::fs::write(
            &def,
            r#"[Info]
name = Test Story
author = Tester
[SceneDef]
startscene = 0
font0 = red.fnt
[Begin Action 10]
0,0, 0,0, 4, , , 1,1, 0
0,1, 8,16, 4
[Scene 0]
end.time = 20
clearcolor = 0,0,0
clearalpha = 255
fadein.time = 10
fadeout.time = 5
jump = Scene 1
sound0.value = 5,2
sound0.starttime = 3
sound0.volume = 80
sound0.pan = -0.5
layer0.anim = 10
layer0.offset = 100,120
layer0.layerno = 1
layer1.text = "Hello"
layer1.textdelay = 2
layer1.starttime = 4
[Scene Scene 1]
end.time = 10
layer0.spr = 0,1
layer0.scale = 2,2
"#,
        )
        .expect("write");
        def
    }

    #[test]
    fn parses_scenes_layers_and_animations() {
        let def = write_storyboard("parse");
        let story = Storyboard::load(&def).expect("load");
        assert_eq!(story.name, "Test Story");
        assert_eq!(story.scenes.len(), 2);
        assert_eq!(story.scenes[0].end_time, 20);
        assert_eq!(story.scenes[0].clear_color, Some([0, 0, 0]));
        assert_eq!(story.scenes[0].layers.len(), 2);
        assert_eq!(story.scenes[0].layers[0].anim, 10);
        assert_eq!(story.scenes[0].layers[0].offset, Vec2::new(100.0, 120.0));
        assert!(story.animations.contains_key(&10));
        assert_eq!(story.scenes[1].layers[0].sprite, Some((0, 1)));
        assert_eq!(story.scene("Scene 1"), Some(1));
        assert!(story.font_files.contains_key(&0));
        assert_eq!(story.scenes[0].sounds.len(), 1);
        let sound = &story.scenes[0].sounds[0];
        assert_eq!((sound.group, sound.index), (5, 2));
        assert_eq!(sound.start_time, 3);
        assert!((sound.volume - 80.0).abs() < f32::EPSILON);
        assert!((sound.pan + 0.5).abs() < f32::EPSILON);
        let _ = std::fs::remove_dir_all(def.parent().unwrap());
    }

    #[test]
    fn player_advances_scenes_and_reveals_text() {
        let def = write_storyboard("player");
        let story = Storyboard::load(&def).expect("load");
        let mut player = StoryboardPlayer::new(&story);
        let layer = &story.scenes[0].layers[1];
        assert_eq!(layer.visible_text(4), "");
        assert_eq!(layer.visible_text(6), "H");
        assert_eq!(layer.visible_text(8), "He");
        assert!(!layer.visible_at(0));
        assert!(layer.visible_at(4));

        assert!(player.fade_alpha(&story.scenes[0]).unwrap().1 > 200);

        // Sound events fire once at their start time.
        player.tick(&story);
        player.tick(&story);
        assert!(player.take_sounds().is_empty());
        player.tick(&story);
        let sounds = player.take_sounds();
        assert_eq!(sounds.len(), 1);
        assert_eq!(sounds[0].0, 5);
        assert_eq!(sounds[0].1, 2);
        player.tick(&story);
        assert!(player.take_sounds().is_empty());
        assert!(player.fade_alpha(&story.scenes[0]).unwrap().1 < 200);

        for _ in 0..20 {
            player.tick(&story);
        }
        assert_eq!(player.scene, 1, "jump to Scene 1");
        for _ in 0..15 {
            player.tick(&story);
        }
        assert!(player.done);
        let _ = std::fs::remove_dir_all(def.parent().unwrap());
    }
}
