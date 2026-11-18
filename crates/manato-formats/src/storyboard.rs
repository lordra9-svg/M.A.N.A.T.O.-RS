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
pub struct StoryboardBgm {
    pub path: PathBuf,
    pub loop_music: bool,
    pub volume: i32,
    pub loop_start: i32,
    pub loop_end: i32,
    pub freq_mul: f32,
    pub loop_count: i32,
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
    /// Storyboard `palfx.add`/`palfx.mul` (0..256 scale; 256 = neutral mul).
    pub palfx_add: [f32; 3],
    pub palfx_mul: [f32; 3],
    /// `palfx.time`: `-1` = infinite, `0` = disabled, otherwise ticks.
    pub palfx_time: i32,
    /// `palfx.color` (0..256 scale; 256 = normal saturation).
    pub palfx_color: f32,
    /// `palfx.hue` as a fraction of a full hue turn.
    pub palfx_hue: f32,
    pub palfx_sinadd: [f32; 3],
    pub palfx_sinadd_time: i32,
    pub palfx_sinmul: [f32; 3],
    pub palfx_sinmul_time: i32,
    pub palfx_sincolor: f32,
    pub palfx_sincolor_time: i32,
    pub palfx_sinhue: f32,
    pub palfx_sinhue_time: i32,
    pub palfx_invertall: bool,
    pub palfx_invertblend: i32,
}

/// Effective PalFX values for one storyboard tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PalFxSample {
    /// Additive color in 0..256 units.
    pub add: [f32; 3],
    /// Multiplicative color in 0..256 units.
    pub mul: [f32; 3],
    /// Saturation factor (`1.0` = unchanged).
    pub color: f32,
    /// Hue shift as a fraction of a full turn.
    pub hue: f32,
    pub invert: bool,
    /// Effective `palfx.invertblend`: `>= 1` swaps add/subtract passes and
    /// `3` additionally disables the shader invert.
    pub invert_blend: i32,
}

impl PalFxSample {
    pub fn identity() -> Self {
        Self {
            add: [0.0; 3],
            mul: [256.0; 3],
            color: 1.0,
            hue: 0.0,
            invert: false,
            invert_blend: 0,
        }
    }

    pub fn is_identity(&self) -> bool {
        self.add == [0.0; 3]
            && self.mul == [256.0; 3]
            && self.color == 1.0
            && self.hue == 0.0
            && !self.invert
            && self.invert_blend == 0
    }

    /// `invertblend >= 1` swaps the add and subtract blend passes.
    pub fn swaps_blend_passes(&self) -> bool {
        self.invert_blend >= 1
    }

    /// `invertblend == 3` disables the shader invert because subtractive
    /// blending already inverts the colors.
    pub fn disables_shader_invert(&self) -> bool {
        self.invert_blend == 3
    }
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
            palfx_add: [0.0; 3],
            palfx_mul: [1.0; 3],
            palfx_time: -1,
            palfx_color: 1.0,
            palfx_hue: 0.0,
            palfx_sinadd: [0.0; 3],
            palfx_sinadd_time: 0,
            palfx_sinmul: [0.0; 3],
            palfx_sinmul_time: 0,
            palfx_sincolor: 0.0,
            palfx_sincolor_time: 0,
            palfx_sinhue: 0.0,
            palfx_sinhue_time: 0,
            palfx_invertall: false,
            palfx_invertblend: 0,
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

    /// Effective PalFX values at scene `time`, including sine modulations.
    /// `palfx.time = 0` disables the effect and a positive time expires.
    pub fn palfx_at(&self, time: i32) -> PalFxSample {
        if self.palfx_time == 0 || (self.palfx_time > 0 && time >= self.palfx_time) {
            return PalFxSample::identity();
        }
        let time = time.max(0);
        let add_phase = sin_phase(time, self.palfx_sinadd_time);
        let mul_phase = sin_phase(time, self.palfx_sinmul_time);
        let color_phase = sin_phase(time, self.palfx_sincolor_time);
        let hue_phase = sin_phase(time, self.palfx_sinhue_time);
        let mut sample = PalFxSample {
            add: [
                self.palfx_add[0] + add_phase * self.palfx_sinadd[0],
                self.palfx_add[1] + add_phase * self.palfx_sinadd[1],
                self.palfx_add[2] + add_phase * self.palfx_sinadd[2],
            ],
            mul: [
                self.palfx_mul[0] * 256.0 + mul_phase * self.palfx_sinmul[0],
                self.palfx_mul[1] * 256.0 + mul_phase * self.palfx_sinmul[1],
                self.palfx_mul[2] * 256.0 + mul_phase * self.palfx_sinmul[2],
            ],
            color: self.palfx_color + color_phase * (self.palfx_sincolor / 256.0),
            hue: self.palfx_hue + hue_phase * (self.palfx_sinhue / 512.0),
            invert: self.palfx_invertall,
            // Ikemen maps negative blend variants to 3 when inversion is on.
            invert_blend: if self.palfx_invertblend <= -2 && self.palfx_invertall {
                3
            } else {
                self.palfx_invertblend
            },
        };
        sample.mul = [
            sample.mul[0].clamp(0.0, 256.0),
            sample.mul[1].clamp(0.0, 256.0),
            sample.mul[2].clamp(0.0, 256.0),
        ];
        sample.color = sample.color.clamp(0.0, 1.0);
        sample
    }

    /// Applies PalFX at scene `time` to a tint color. The transform follows the
    /// Ikemen sprite shader order: hue, invert, saturation, add, multiply.
    pub fn tint_at(&self, time: i32, base: [f32; 4]) -> [f32; 4] {
        let palfx = self.palfx_at(time);
        let mut rgb = [base[0], base[1], base[2]];
        if palfx.hue != 0.0 {
            rgb = hue_shift(rgb, palfx.hue);
        }
        if palfx.invert {
            rgb = [1.0 - rgb[0], 1.0 - rgb[1], 1.0 - rgb[2]];
        }
        if palfx.color != 1.0 {
            let gray = (rgb[0] + rgb[1] + rgb[2]) / 3.0;
            for channel in &mut rgb {
                *channel = gray + (*channel - gray) * palfx.color;
            }
        }
        [
            (rgb[0] * palfx.mul[0] / 256.0 + palfx.add[0] / 256.0).clamp(0.0, 1.0),
            (rgb[1] * palfx.mul[1] / 256.0 + palfx.add[1] / 256.0).clamp(0.0, 1.0),
            (rgb[2] * palfx.mul[2] / 256.0 + palfx.add[2] / 256.0).clamp(0.0, 1.0),
            base[3].clamp(0.0, 1.0),
        ]
    }

    /// Applies the layer's PalFX to a base color at the first scene tick.
    pub fn tint(&self, base: [f32; 4]) -> [f32; 4] {
        self.tint_at(0, base)
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
    /// `layerall.pos` offset applied to every layer.
    pub layer_all_pos: Vec2,
    pub layers: Vec<StoryboardLayer>,
    pub sounds: Vec<StoryboardSound>,
    pub bgm: Option<StoryboardBgm>,
    /// `bg.name` stage definition rendered beneath the scene layers.
    pub bg: Option<crate::stage::Stage>,
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
    /// `[SceneDef] lua`/`script` source, executed by the sandboxed runtime.
    pub lua_source: Option<String>,
    /// `[SceneDef] model`: optional glTF scene model.
    pub model: Option<PathBuf>,
    /// `[SceneDef] modelanim`: glTF animation clip name or index.
    pub model_anim: Option<String>,
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
        let lua_source = get(scene_def, "lua")
            .or_else(|| get(scene_def, "script"))
            .filter(|name| !name.is_empty())
            .and_then(|name| {
                let path = resolve(&dir, &name);
                match std::fs::read_to_string(&path) {
                    Ok(source) => Some(source),
                    Err(error) => {
                        log::warn!("storyboard lua {}: {error}", path.display());
                        None
                    }
                }
            });
        let model = get(scene_def, "model")
            .filter(|value| !value.is_empty())
            .map(|value| resolve(&dir, &value));
        let model_anim = get(scene_def, "modelanim")
            .or_else(|| get(scene_def, "model.anim"))
            .filter(|value| !value.is_empty());
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
            let mut bg_name: Option<String> = None;
            let mut bgm_path: Option<String> = None;
            let mut bgm_loop = true;
            let mut bgm_volume = 100;
            let mut bgm_loop_start = 0;
            let mut bgm_loop_end = 0;
            let mut bgm_freq = 1.0;
            let mut bgm_loop_count = -1;
            for entry in &section.entries {
                let key = entry.key.trim().to_ascii_lowercase();
                let value = entry.value.trim();
                match key.as_str() {
                    "end.time" => scene.end_time = value.parse().unwrap_or(-1),
                    "jump" => scene.jump = Some(value.to_string()),
                    "layerall.pos" => scene.layer_all_pos = pair_vec(value),
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
                    "bg.name" | "bg" => bg_name = Some(value.to_string()),
                    "bgm" | "bgmusic" => bgm_path = Some(value.to_string()),
                    "bgm.loop" => bgm_loop = value.parse::<i32>().unwrap_or(1) != 0,
                    "bgm.volume" => bgm_volume = value.parse().unwrap_or(100),
                    "bgm.loopstart" => bgm_loop_start = value.parse().unwrap_or(0),
                    "bgm.loopend" => bgm_loop_end = value.parse().unwrap_or(0),
                    "bgm.freqmul" => bgm_freq = value.parse().unwrap_or(1.0),
                    "bgm.loopcount" => bgm_loop_count = value.parse().unwrap_or(-1),
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
                            "palfx.add" => layer.palfx_add = parse_triplet(value),
                            "palfx.mul" => {
                                let color = parse_triplet(value);
                                layer.palfx_mul =
                                    [color[0] / 256.0, color[1] / 256.0, color[2] / 256.0];
                            }
                            "palfx.time" => layer.palfx_time = value.parse().unwrap_or(-1),
                            "palfx.color" => {
                                layer.palfx_color = value.parse::<f32>().unwrap_or(256.0) / 256.0;
                            }
                            "palfx.hue" => layer.palfx_hue = value.parse().unwrap_or(0.0),
                            "palfx.sinadd" => {
                                let sin = parse_floats(value);
                                layer.palfx_sinadd = [sin[0], sin[1], sin[2]];
                                layer.palfx_sinadd_time = sin[3] as i32;
                            }
                            "palfx.sinmul" => {
                                let sin = parse_floats(value);
                                layer.palfx_sinmul = [sin[0], sin[1], sin[2]];
                                layer.palfx_sinmul_time = sin[3] as i32;
                            }
                            "palfx.sincolor" => {
                                let sin = parse_floats(value);
                                layer.palfx_sincolor = sin[0];
                                layer.palfx_sincolor_time = sin[1] as i32;
                            }
                            "palfx.sinhue" => {
                                let sin = parse_floats(value);
                                layer.palfx_sinhue = sin[0];
                                layer.palfx_sinhue_time = sin[1] as i32;
                            }
                            "palfx.invertall" => {
                                layer.palfx_invertall = matches!(
                                    value.trim().to_ascii_lowercase().as_str(),
                                    "1" | "true" | "yes" | "on"
                                );
                            }
                            "palfx.invertblend" => {
                                layer.palfx_invertblend = value.parse().unwrap_or(0);
                            }
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
            scene.bg = bg_name.filter(|name| !name.is_empty()).and_then(|name| {
                let path = resolve(&dir, &name);
                match crate::stage::Stage::load(&path, std::slice::from_ref(&dir)) {
                    Ok(stage) => Some(stage),
                    Err(error) => {
                        log::warn!("storyboard bg {}: {error:#}", path.display());
                        None
                    }
                }
            });
            scene.bgm = bgm_path.map(|path| StoryboardBgm {
                path: resolve(&dir, &path),
                loop_music: bgm_loop,
                volume: bgm_volume,
                loop_start: bgm_loop_start,
                loop_end: bgm_loop_end,
                freq_mul: bgm_freq,
                loop_count: bgm_loop_count,
            });
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
            lua_source,
            model,
            model_anim,
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

    /// Jumps to a named scene, resetting scene time. Returns false when the
    /// name does not exist.
    pub fn jump_to(&mut self, storyboard: &Storyboard, name: &str) -> bool {
        let Some(index) = storyboard.scene(name) else {
            return false;
        };
        self.scene = index;
        self.time = 0;
        self.done = false;
        self.pending_sounds.clear();
        true
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

/// Sine modulation phase for a PalFX cycle, matching Ikemen's `sinAdd`.
fn sin_phase(time: i32, cycle: i32) -> f32 {
    if cycle <= 1 {
        return 0.0;
    }
    let sintime = time.rem_euclid(cycle);
    let mut phase = std::f32::consts::TAU * sintime as f32;
    if cycle == 2 {
        phase += std::f32::consts::FRAC_PI_2;
    }
    (phase / cycle as f32).sin()
}

/// HSV hue rotation matching Ikemen's sprite shader.
fn hue_shift(rgb: [f32; 3], delta: f32) -> [f32; 3] {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    let min = rgb[0].min(rgb[1]).min(rgb[2]);
    let chroma = max - min;
    if chroma <= f32::EPSILON {
        return rgb;
    }
    let mut hue = if max == rgb[0] {
        ((rgb[1] - rgb[2]) / chroma).rem_euclid(6.0)
    } else if max == rgb[1] {
        (rgb[2] - rgb[0]) / chroma + 2.0
    } else {
        (rgb[0] - rgb[1]) / chroma + 4.0
    } / 6.0;
    hue = (hue + delta).rem_euclid(1.0);
    let saturation = if max <= f32::EPSILON {
        0.0
    } else {
        chroma / max
    };
    let value = max;
    let h = hue * 6.0;
    let sector = h.floor();
    let fraction = h - sector;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - saturation * fraction);
    let t = value * (1.0 - saturation * (1.0 - fraction));
    match sector as i32 % 6 {
        0 => [value, t, p],
        1 => [q, value, p],
        2 => [p, value, t],
        3 => [p, q, value],
        4 => [t, p, value],
        _ => [value, p, q],
    }
}

fn parse_floats(value: &str) -> [f32; 4] {
    let mut parts = value.split(',');
    let mut next = || {
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or(0.0)
    };
    [next(), next(), next(), next()]
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
lua = script.lua
[Begin Action 10]
0,0, 0,0, 4, , , 1,1, 0
Interpolate Offset
0,1, 8,16, 4
[Scene 0]
end.time = 20
clearcolor = 0,0,0
clearalpha = 255
fadein.time = 10
fadeout.time = 5
jump = Scene 1
layerall.pos = 5, -10
layer0.palfx.mul = 256,128,256
layer0.palfx.add = 32,0,0
layer0.palfx.time = 30
layer0.palfx.color = 256
layer0.palfx.hue = 0
layer0.palfx.sinadd = 4,0,0,8
layer0.palfx.sinmul = 0,8,0,8
layer0.palfx.sincolor = 64,8
layer0.palfx.sinhue = 64,8
layer0.palfx.invertblend = 1
sound0.value = 5,2
sound0.starttime = 3
sound0.volume = 80
sound0.pan = -0.5
bgm = theme.ogg
bgm.loop = 0
bgm.volume = 80
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
        std::fs::write(
            dir.join("script.lua"),
            "function update(ctx)\n  ctx.x = ctx.x + 1\nend\n",
        )
        .expect("write lua");
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
        assert_eq!(story.scenes[0].layer_all_pos, Vec2::new(5.0, -10.0));
        assert!((story.scenes[0].layers[0].palfx_mul[1] - 0.5).abs() < 1e-6);
        assert!((story.scenes[0].layers[0].palfx_add[0] - 32.0).abs() < 1e-6);
        let tint = story.scenes[0].layers[0].tint([1.0, 1.0, 1.0, 1.0]);
        assert!((tint[0] - 1.0f32).abs() < 1e-6, "add clamps");
        assert!((tint[1] - 0.5).abs() < 1e-6);
        let layer = &story.scenes[0].layers[0];
        assert_eq!(layer.palfx_time, 30);
        assert!((layer.palfx_color - 1.0).abs() < 1e-6);
        assert_eq!(layer.palfx_sinadd, [4.0, 0.0, 0.0]);
        assert_eq!(layer.palfx_sinadd_time, 8);
        assert_eq!(layer.palfx_sinmul, [0.0, 8.0, 0.0]);
        assert_eq!(layer.palfx_sincolor, 64.0);
        assert_eq!(layer.palfx_sinhue, 64.0);
        assert_eq!(layer.palfx_invertblend, 1);
        let anim = story.animations.get(&10).expect("action 10");
        // Sprite layers use the AIR interpolated pose halfway through frame 0.
        let pose = anim.pose_at(2).expect("pose");
        assert!((pose.offset.x - 4.0).abs() < 1e-6);
        assert!((pose.offset.y - 8.0).abs() < 1e-6);
        assert_eq!(story.scenes[1].layers[0].sprite, Some((0, 1)));
        assert_eq!(story.scene("Scene 1"), Some(1));
        assert!(story.font_files.contains_key(&0));
        assert!(story
            .lua_source
            .as_deref()
            .is_some_and(|source| source.contains("function update")));
        assert_eq!(story.scenes[0].sounds.len(), 1);
        let sound = &story.scenes[0].sounds[0];
        assert_eq!((sound.group, sound.index), (5, 2));
        assert_eq!(sound.start_time, 3);
        assert!((sound.volume - 80.0).abs() < f32::EPSILON);
        assert!((sound.pan + 0.5).abs() < f32::EPSILON);
        let bgm = story.scenes[0].bgm.as_ref().expect("bgm");
        assert!(bgm.path.ends_with("theme.ogg"));
        assert!(!bgm.loop_music);
        assert_eq!(bgm.volume, 80);
        let _ = std::fs::remove_dir_all(def.parent().unwrap());
    }

    #[test]
    fn layer_palfx_time_varying_color_hue_and_expiry() {
        let mut layer = StoryboardLayer::default();
        assert_eq!(layer.palfx_at(0), PalFxSample::identity());

        // Saturation 0 turns any color gray.
        layer.palfx_color = 0.0;
        let tint = layer.tint_at(0, [1.0, 0.25, 0.0, 1.0]);
        assert!((tint[0] - tint[1]).abs() < 1e-6);
        assert!((tint[1] - tint[2]).abs() < 1e-6);

        // Invert is applied after hue and before saturation.
        layer.palfx_color = 1.0;
        layer.palfx_invertall = true;
        let tint = layer.tint_at(0, [1.0, 0.0, 0.0, 1.0]);
        assert!(tint[0].abs() < 1e-6 && (tint[1] - 1.0).abs() < 1e-6);

        // A third of a turn maps red to green.
        layer.palfx_invertall = false;
        layer.palfx_hue = 1.0 / 3.0;
        let tint = layer.tint_at(0, [1.0, 0.0, 0.0, 1.0]);
        assert!(tint[0].abs() < 1e-5 && (tint[1] - 1.0).abs() < 1e-5);

        // sinadd cycles every 8 ticks: 0 at t=0, +100 at t=2 (quarter turn).
        let mut layer = StoryboardLayer {
            palfx_sinadd: [100.0, 0.0, 0.0],
            palfx_sinadd_time: 8,
            ..Default::default()
        };
        let tint = layer.tint_at(2, [0.0, 0.0, 0.0, 1.0]);
        assert!((tint[0] - 100.0 / 256.0).abs() < 1e-5);

        // A positive time expires back to the identity.
        layer.palfx_time = 4;
        assert!(!layer.palfx_at(3).is_identity());
        assert_eq!(layer.palfx_at(4), PalFxSample::identity());
        // time = 0 disables the effect immediately.
        layer.palfx_time = 0;
        assert_eq!(layer.palfx_at(0), PalFxSample::identity());
    }

    #[test]
    fn layer_palfx_invertblend_variants() {
        let mut layer = StoryboardLayer::default();
        assert!(!layer.palfx_at(0).swaps_blend_passes());
        layer.palfx_invertblend = 1;
        let sample = layer.palfx_at(0);
        assert!(sample.swaps_blend_passes());
        assert!(!sample.disables_shader_invert());
        layer.palfx_invertblend = 3;
        let sample = layer.palfx_at(0);
        assert!(sample.swaps_blend_passes());
        assert!(sample.disables_shader_invert());
        // Negative variants become 3 only while invertall is active.
        layer.palfx_invertblend = -2;
        assert_eq!(layer.palfx_at(0).invert_blend, -2);
        layer.palfx_invertall = true;
        let sample = layer.palfx_at(0);
        assert_eq!(sample.invert_blend, 3);
        assert!(sample.disables_shader_invert());
    }

    #[test]
    fn parses_scene_model_reference() {
        let dir = std::env::temp_dir().join(format!("manato-story-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let def = dir.join("story.def");
        std::fs::write(
            &def,
            "[Info]\nname = Model\n[SceneDef]\nstartscene = 0\nmodel = scene.gltf\nmodelanim = back\n[Scene 0]\nend.time = 10\n",
        )
        .expect("write");
        let story = Storyboard::load(&def).expect("load");
        assert_eq!(
            story.model.as_deref(),
            Some(dir.join("scene.gltf").as_path())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_and_loads_scene_background() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let stage = root.join("content/stages/training/stage1.def");
        if !stage.exists() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("manato-storybg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let def = dir.join("story.def");
        std::fs::write(
            &def,
            format!(
                "[Info]\nname = Bg\n[SceneDef]\nstartscene = 0\n[Scene 0]\nend.time = 10\nbg.name = {}\n",
                stage.display()
            ),
        )
        .expect("write");
        let story = Storyboard::load(&def).expect("load");
        let bg = story.scenes[0].bg.as_ref().expect("stage background");
        assert!(!bg.layers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn player_jumps_to_named_scene() {
        let def = write_storyboard("jump_to");
        let story = Storyboard::load(&def).expect("load");
        let mut player = StoryboardPlayer::new(&story);
        assert_eq!(player.scene, 0);
        player.time = 7;
        assert!(player.jump_to(&story, "Scene 1"));
        assert_eq!(player.scene, 1);
        assert_eq!(player.time, 0);
        assert!(!player.jump_to(&story, "missing"));
        assert_eq!(player.scene, 1);
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
