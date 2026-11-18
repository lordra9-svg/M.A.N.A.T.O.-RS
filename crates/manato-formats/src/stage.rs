//! M.U.G.E.N/Ikemen-GO stage DEF and background parsing.
//!
//! The layout and rendering rules are specified in `docs/spec/stage.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use manato_core::Vec2;

use crate::air::{parse_air_str, Animation, BlendMode};
use crate::cns::Controller;
use crate::def::{section_map, StageDefFile};
use crate::sff::SffFile;
use crate::text::{decode_text, parse_pair};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CameraDef {
    pub start: Vec2,
    pub bound_left: f32,
    pub bound_right: f32,
    pub bound_high: f32,
    pub bound_low: f32,
    pub vertical_follow: f32,
    pub floor_tension: f32,
    pub tension: f32,
    pub overdraw_high: f32,
    pub overdraw_low: f32,
    pub cut_high: f32,
    pub cut_low: f32,
    /// Ikemen-GO zoom keys (`docs/spec/render.md` §11.1).
    pub start_zoom: f32,
    pub zoom_out: f32,
    pub zoom_in: f32,
    pub zoom_in_delay: i32,
    pub zoom_in_speed: f32,
    pub zoom_out_speed: f32,
    pub zoom_anchor: bool,
}

impl Default for CameraDef {
    fn default() -> Self {
        Self {
            start: Vec2::ZERO,
            bound_left: -1000.0,
            bound_right: 1000.0,
            bound_high: -100.0,
            bound_low: 0.0,
            vertical_follow: 0.2,
            floor_tension: 0.0,
            tension: 50.0,
            overdraw_high: 0.0,
            overdraw_low: 0.0,
            cut_high: 0.0,
            cut_low: 0.0,
            start_zoom: 1.0,
            zoom_out: 1.0,
            zoom_in: 1.0,
            zoom_in_delay: 0,
            zoom_in_speed: 1.0,
            zoom_out_speed: 1.0,
            zoom_anchor: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ShadowDef {
    pub intensity: i32,
    pub color: [u8; 3],
    pub xscale: f32,
    pub yscale: f32,
    pub ydelta: f32,
    pub fade_start: f32,
    pub fade_end: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReflectionDef {
    pub intensity: i32,
    pub color: [u8; 3],
    pub xscale: f32,
    pub yscale: f32,
    pub ydelta: f32,
}

#[derive(Debug, Clone, Default)]
pub struct MusicDef {
    pub path: Option<PathBuf>,
    pub volume: i32,
    pub loop_start: i32,
    pub loop_end: i32,
    pub ratio: f32,
}

#[derive(Debug, Clone, Default)]
pub struct BgLayer {
    pub kind: String,
    pub sprite: Option<(i32, i32)>,
    pub action: Option<i32>,
    pub start: Vec2,
    pub delta: Vec2,
    pub trans: BlendMode,
    pub tile: (i32, i32),
    pub tile_spacing: (i32, i32),
    pub layer_no: i32,
    pub mask: bool,
    pub velocity: Vec2,
    pub sin_x: [f32; 3],
    pub sin_y: [f32; 3],
    pub cos_x: [f32; 3],
    pub cos_y: [f32; 3],
    pub position_link: bool,
    pub visible: bool,
    pub enabled: bool,
    pub xscale: f32,
    pub yscale: f32,
    pub window: [f32; 4],
    pub start_rect: [f32; 4],
    pub window_delta: Vec2,
    pub id: i32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BgCtrl {
    pub layer: usize,
    /// First active tick (`time` first value).
    pub start: i32,
    /// Last active tick (`time` second value); equals `start` when omitted.
    pub end: i32,
    /// Repeat period (`time` third value); `-1` disables looping.
    pub loop_period: i32,
    pub controller: Controller,
}

impl BgCtrl {
    /// Whether the controller runs at stage tick `time`, following Ikemen's
    /// `bgCtrlAction` window rules.
    pub fn active_at(&self, time: i32) -> bool {
        if self.start < 0 || (self.loop_period >= 0 && self.start >= self.loop_period) {
            return false;
        }
        if time < self.start {
            return false;
        }
        if self.loop_period > 0 {
            let end = self.end.min(self.loop_period) - self.start;
            (time - self.start) % self.loop_period <= end
        } else {
            time <= self.end
        }
    }
}

/// A standalone background scene used by screenpack motifs (title, select).
///
/// It shares the stage BG format but has no camera or player data.
#[derive(Debug, Clone, Default)]
pub struct BgScene {
    pub sprites: SffFile,
    pub animations: HashMap<i32, Animation>,
    pub layers: Vec<BgLayer>,
    pub bg_ctrls: Vec<BgCtrl>,
    pub clear_color: [u8; 3],
    pub clear_alpha: u8,
    pub clear_layerno: i32,
    pub path: PathBuf,
}

impl BgScene {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let def = StageDefFile::load(path)
            .with_context(|| format!("failed to load background def {}", path.display()))?;
        let raw = std::fs::read(path)
            .map(|bytes| decode_text(&bytes))
            .unwrap_or_default();
        let animations: HashMap<i32, Animation> = parse_air_str(&raw)
            .into_iter()
            .map(|animation| (animation.number, animation))
            .collect();

        let spr_name = def
            .bgdef
            .get("spr")
            .or_else(|| def.info.get("spr"))
            .cloned()
            .unwrap_or_default();
        let mut sprites = SffFile::default();
        if !spr_name.is_empty() {
            let candidate = crate::def::resolve_path(&def.dir, &spr_name);
            if candidate.exists() {
                sprites = SffFile::load(&candidate).with_context(|| {
                    format!("failed to load background SFF {}", candidate.display())
                })?;
            }
        }

        let mut layers = Vec::new();
        let mut bg_ctrls = Vec::new();
        for section in &def.text.sections {
            match section.kind().as_str() {
                "bg" => layers.push(parse_layer(section_map(section))),
                "bgctrl" => {
                    let layer = section
                        .header_lower()
                        .split_whitespace()
                        .nth(1)
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(0);
                    let params: Vec<(String, String)> = section
                        .entries
                        .iter()
                        .map(|entry| {
                            (
                                entry.key.trim().to_ascii_lowercase(),
                                entry.value.trim().to_string(),
                            )
                        })
                        .collect();
                    let kind = params
                        .iter()
                        .find(|(key, _)| key == "type")
                        .map(|(_, value)| value.to_ascii_lowercase())
                        .unwrap_or_else(|| "null".into());
                    let (start, end, loop_period) = params
                        .iter()
                        .find(|(key, _)| key == "time")
                        .map(|(_, value)| {
                            let mut parts = value
                                .split(',')
                                .map(|part| part.trim().parse::<i32>().unwrap_or(0));
                            let start = parts.next().unwrap_or(0);
                            let end = parts.next().unwrap_or(start);
                            let loop_period = parts.next().unwrap_or(-1);
                            (start, end, loop_period)
                        })
                        .unwrap_or((0, 0, -1));
                    bg_ctrls.push(BgCtrl {
                        layer,
                        start,
                        end,
                        loop_period,
                        controller: Controller {
                            kind,
                            params,
                            line: section.line,
                        },
                    });
                }
                _ => {}
            }
        }

        let alpha = def
            .bgdef
            .get("bgclearalpha")
            .and_then(|value| value.split(',').next())
            .and_then(|value| value.trim().parse::<i32>().ok())
            .unwrap_or(255)
            .clamp(0, 255) as u8;
        let layerno = def
            .bgdef
            .get("bgclearlayerno")
            .and_then(|value| value.trim().parse::<i32>().ok())
            .unwrap_or(0);

        Ok(Self {
            sprites,
            animations,
            layers,
            bg_ctrls,
            clear_color: parse_color(def.bgdef.get("bgclearcolor")),
            clear_alpha: alpha,
            clear_layerno: layerno,
            path: path.to_path_buf(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Stage {
    /// Stage DEF directory (used to resolve BGM paths).
    pub dir: PathBuf,
    pub name: String,
    pub display_name: String,
    pub author: String,
    pub localcoord: (f32, f32),
    pub camera: CameraDef,
    pub ground_level: f32,
    pub left_bound: f32,
    pub right_bound: f32,
    /// `[PlayerInfo] topbound`/`botbound`: depth range for z-axis checks.
    pub z_min: f32,
    pub z_max: f32,
    pub screen_left: f32,
    pub screen_right: f32,
    pub zoffset: f32,
    /// `[Scaling] topz`/`botz`: depth range used for perspective.
    pub z_top: f32,
    pub z_bottom: f32,
    /// `[Scaling] topscale`/`botscale`: sprite scale at `z_top`/`z_bottom`.
    pub z_top_scale: f32,
    pub z_bottom_scale: f32,
    /// `[Scaling] depthtoscreen`: converts depth units to screen Y offset.
    pub depth_to_screen: f32,
    pub autoturn: bool,
    pub reset_bg: bool,
    pub debug_bg: bool,
    pub bg_clear_color: [u8; 3],
    /// `[Model] model`: optional glTF scene model.
    pub model: Option<PathBuf>,
    /// `[Model] anim`: glTF animation clip name or index.
    pub model_anim: Option<String>,
    /// `[Model] envmap`: equirectangular environment image.
    pub model_env: Option<PathBuf>,
    pub xscale: f32,
    pub yscale: f32,
    pub portrait_scale: f32,
    pub p1_start: Vec2,
    pub p2_start: Vec2,
    pub p1_facing: i32,
    pub p2_facing: i32,
    pub shadow: ShadowDef,
    pub reflection: ReflectionDef,
    pub music: MusicDef,
    pub sprites: SffFile,
    pub animations: HashMap<i32, Animation>,
    /// Raw `[Constants]` values (`StageConst(key)`); numeric values parse on
    /// read and other values are returned as strings.
    pub constants: HashMap<String, String>,
    pub layers: Vec<BgLayer>,
    pub bg_ctrls: Vec<BgCtrl>,
    pub path: PathBuf,
}

impl Default for Stage {
    fn default() -> Self {
        Self {
            dir: PathBuf::new(),
            name: String::new(),
            display_name: String::new(),
            author: String::new(),
            localcoord: (320.0, 240.0),
            camera: CameraDef::default(),
            ground_level: 0.0,
            left_bound: 0.0,
            right_bound: 0.0,
            z_min: 0.0,
            z_max: 0.0,
            screen_left: 0.0,
            screen_right: 0.0,
            zoffset: 0.0,
            z_top: 0.0,
            z_bottom: 0.0,
            z_top_scale: 1.0,
            z_bottom_scale: 1.0,
            depth_to_screen: 0.0,
            autoturn: false,
            reset_bg: true,
            debug_bg: false,
            bg_clear_color: [0, 0, 0],
            model: None,
            model_anim: None,
            model_env: None,
            xscale: 1.0,
            yscale: 1.0,
            portrait_scale: 1.0,
            p1_start: Vec2::ZERO,
            p2_start: Vec2::ZERO,
            p1_facing: 1,
            p2_facing: -1,
            shadow: ShadowDef::default(),
            reflection: ReflectionDef::default(),
            music: MusicDef::default(),
            sprites: crate::sff::SffFile::default(),
            animations: HashMap::new(),
            constants: HashMap::new(),
            layers: Vec::new(),
            bg_ctrls: Vec::new(),
            path: PathBuf::new(),
        }
    }
}

impl Stage {
    /// Sprite scale at depth `z`, interpolated across `z_top..z_bottom`.
    /// Returns `1.0` when the stage has no depth range.
    pub fn z_scale(&self, z: f32) -> f32 {
        let range = self.z_bottom - self.z_top;
        if range.abs() < f32::EPSILON {
            return 1.0;
        }
        let d = (z - self.z_top) / range;
        (self.z_top_scale + d * (self.z_bottom_scale - self.z_top_scale)).max(0.0)
    }

    /// Screen-space Y offset for depth `z` (`depthtoscreen`).
    pub fn z_y_offset(&self, z: f32) -> f32 {
        z * self.depth_to_screen
    }

    pub fn load(path: &Path, data_dirs: &[PathBuf]) -> anyhow::Result<Self> {
        let def = StageDefFile::load(path)
            .with_context(|| format!("failed to load stage def {}", path.display()))?;
        let raw = std::fs::read(path)
            .map(|bytes| decode_text(&bytes))
            .unwrap_or_default();
        let animations: HashMap<i32, Animation> = parse_air_str(&raw)
            .into_iter()
            .map(|animation| (animation.number, animation))
            .collect();

        let spr_name = def
            .bgdef
            .get("spr")
            .or_else(|| def.info.get("spr"))
            .cloned()
            .unwrap_or_default();
        let mut sprites = SffFile::default();
        if !spr_name.is_empty() {
            let candidate = crate::def::resolve_path(&def.dir, &spr_name);
            if candidate.exists() {
                sprites = SffFile::load(&candidate)
                    .with_context(|| format!("failed to load stage SFF {}", candidate.display()))?;
            } else {
                for dir in data_dirs {
                    let candidate = crate::def::resolve_path(dir, &spr_name);
                    if candidate.exists() {
                        sprites = SffFile::load(&candidate).unwrap_or_default();
                        break;
                    }
                }
            }
        }

        let mut constants = HashMap::new();
        for section in &def.text.sections {
            if section.kind() == "constants" {
                for entry in &section.entries {
                    constants.insert(
                        entry.key.trim().to_ascii_lowercase(),
                        entry.value.trim().to_string(),
                    );
                }
            }
        }

        let mut layers = Vec::new();
        let mut bg_ctrls = Vec::new();
        for section in &def.text.sections {
            match section.kind().as_str() {
                "bg" => layers.push(parse_layer(section_map(section))),
                "bgctrl" => {
                    let layer = section
                        .header_lower()
                        .split_whitespace()
                        .nth(1)
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(0);
                    let params: Vec<(String, String)> = section
                        .entries
                        .iter()
                        .map(|entry| {
                            (
                                entry.key.trim().to_ascii_lowercase(),
                                entry.value.trim().to_string(),
                            )
                        })
                        .collect();
                    let kind = params
                        .iter()
                        .find(|(key, _)| key == "type")
                        .map(|(_, value)| value.to_ascii_lowercase())
                        .unwrap_or_else(|| "null".into());
                    let (start, end, loop_period) = params
                        .iter()
                        .find(|(key, _)| key == "time")
                        .map(|(_, value)| {
                            let mut parts = value
                                .split(',')
                                .map(|part| part.trim().parse::<i32>().unwrap_or(0));
                            let start = parts.next().unwrap_or(0);
                            let end = parts.next().unwrap_or(start);
                            let loop_period = parts.next().unwrap_or(-1);
                            (start, end, loop_period)
                        })
                        .unwrap_or((0, 0, -1));
                    bg_ctrls.push(BgCtrl {
                        layer,
                        start,
                        end,
                        loop_period,
                        controller: Controller {
                            kind,
                            params,
                            line: section.line,
                        },
                    });
                }
                _ => {}
            }
        }

        if layers.is_empty() {
            layers.push(BgLayer {
                kind: "normal".into(),
                sprite: Some((0, 0)),
                visible: true,
                enabled: true,
                xscale: 1.0,
                yscale: 1.0,
                ..BgLayer::default()
            });
        }

        let camera_map = &def.camera;
        let zoom_out = num(camera_map, "zoomout", 1.0).max(0.0001);
        let zoom_in = num(camera_map, "zoomin", 1.0).max(zoom_out);
        let start_zoom = num(camera_map, "startzoom", 1.0).clamp(zoom_out, zoom_in);
        let camera = CameraDef {
            start: Vec2::new(
                num(camera_map, "startx", 0.0),
                num(camera_map, "starty", 0.0),
            ),
            bound_left: num(camera_map, "boundleft", -1000.0),
            bound_right: num(camera_map, "boundright", 1000.0),
            bound_high: num(camera_map, "boundhigh", -100.0),
            bound_low: num(camera_map, "boundlow", 0.0),
            vertical_follow: num(camera_map, "verticalfollow", 0.2),
            floor_tension: num(camera_map, "floortension", 0.0),
            tension: num(camera_map, "tension", 50.0),
            overdraw_high: num(camera_map, "overdrawhigh", 0.0),
            overdraw_low: num(camera_map, "overdrawlow", 0.0),
            cut_high: num(camera_map, "cuthigh", 0.0),
            cut_low: num(camera_map, "cutlow", 0.0),
            start_zoom,
            zoom_out,
            zoom_in,
            zoom_in_delay: num(camera_map, "zoomindelay", 0.0).max(0.0) as i32,
            zoom_in_speed: num(camera_map, "zoominspeed", 1.0).clamp(0.0, 1.0),
            zoom_out_speed: num(camera_map, "zoomoutspeed", 1.0).clamp(0.0, 1.0),
            zoom_anchor: num(camera_map, "zoomanchor", 0.0) != 0.0,
        };

        let stage_info = def
            .text
            .find("stageinfo")
            .map(section_map)
            .unwrap_or_default();
        let shadow_map = def.text.find("shadow").map(section_map).unwrap_or_default();
        let reflection_map = def
            .text
            .find("reflection")
            .map(section_map)
            .unwrap_or_default();
        let music_map = def.text.find("music").map(section_map).unwrap_or_default();
        let scaling = def
            .text
            .find("scaling")
            .map(crate::def::section_map)
            .unwrap_or_default();
        let bgdef = &def.bgdef;

        Ok(Self {
            dir: def.dir.clone(),
            name: def.name,
            display_name: def.display_name,
            author: def.author,
            localcoord: def.localcoord,
            camera,
            ground_level: 0.0,
            left_bound: num(&def.player_info, "leftbound", -1000.0),
            right_bound: num(&def.player_info, "rightbound", 1000.0),
            z_min: num(&def.player_info, "topbound", 0.0),
            z_max: num(&def.player_info, "botbound", 0.0),
            screen_left: num(&def.player_info, "screenleft", 15.0),
            screen_right: num(&def.player_info, "screenright", 15.0),
            zoffset: num(&stage_info, "zoffset", 204.0),
            z_top: scaling
                .get("topz")
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0.0),
            z_bottom: scaling
                .get("botz")
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0.0),
            z_top_scale: scaling
                .get("topscale")
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(1.0),
            z_bottom_scale: scaling
                .get("botscale")
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(1.0),
            depth_to_screen: scaling
                .get("depthtoscreen")
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0.0),
            autoturn: num(&stage_info, "autoturn", 1.0) != 0.0,
            reset_bg: num(&stage_info, "resetbg", 1.0) != 0.0,
            debug_bg: num(bgdef, "debugbg", 0.0) != 0.0,
            bg_clear_color: parse_color(bgdef.get("bgclearcolor")),
            model: def
                .text
                .find("model")
                .map(section_map)
                .and_then(|map| map.get("model").cloned())
                .filter(|value| !value.trim().is_empty())
                .map(|value| {
                    let path = Path::new(value.trim());
                    if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        def.dir.join(path)
                    }
                }),
            model_anim: def
                .text
                .find("model")
                .map(section_map)
                .and_then(|map| map.get("anim").or_else(|| map.get("modelanim")).cloned())
                .filter(|value| !value.trim().is_empty()),
            model_env: def
                .text
                .find("model")
                .map(section_map)
                .and_then(|map| map.get("envmap").cloned())
                .filter(|value| !value.trim().is_empty())
                .map(|value| {
                    let path = Path::new(value.trim());
                    if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        def.dir.join(path)
                    }
                }),
            xscale: num(&stage_info, "xscale", 1.0),
            yscale: num(&stage_info, "yscale", 1.0),
            portrait_scale: num(&stage_info, "portraitscale", 0.41666),
            p1_start: Vec2::new(
                num(&def.player_info, "p1startx", -70.0),
                num(&def.player_info, "p1starty", 0.0),
            ),
            p2_start: Vec2::new(
                num(&def.player_info, "p2startx", 70.0),
                num(&def.player_info, "p2starty", 0.0),
            ),
            p1_facing: num(&def.player_info, "p1facing", 1.0) as i32,
            p2_facing: num(&def.player_info, "p2facing", -1.0) as i32,
            shadow: ShadowDef {
                intensity: num(&shadow_map, "intensity", 128.0) as i32,
                color: parse_color(shadow_map.get("color")),
                xscale: num(&shadow_map, "xscale", 1.0),
                yscale: num(&shadow_map, "yscale", 0.3),
                ydelta: num(&shadow_map, "ydelta", 1.0),
                fade_start: parse_pair(
                    shadow_map
                        .get("fade.range")
                        .map(String::as_str)
                        .unwrap_or("0,0"),
                )
                .0,
                fade_end: parse_pair(
                    shadow_map
                        .get("fade.range")
                        .map(String::as_str)
                        .unwrap_or("0,0"),
                )
                .1,
            },
            reflection: ReflectionDef {
                intensity: num(&reflection_map, "intensity", 0.0) as i32,
                color: parse_color(reflection_map.get("color")),
                xscale: num(&reflection_map, "xscale", 1.0),
                yscale: num(&reflection_map, "yscale", 1.0),
                ydelta: num(&reflection_map, "ydelta", 1.0),
            },
            music: MusicDef {
                path: music_map
                    .get("bgmusic")
                    .map(|value| crate::def::resolve_path(&def.dir, value))
                    .filter(|path| path.exists()),
                volume: num(&music_map, "bgmvolume", 100.0) as i32,
                loop_start: num(&music_map, "bgmloopstart", 0.0) as i32,
                loop_end: num(&music_map, "bgmloopend", 0.0) as i32,
                ratio: num(&music_map, "bgmratio", 0.3),
            },
            sprites,
            animations,
            constants,
            layers,
            bg_ctrls,
            path: path.to_path_buf(),
        })
    }

    pub fn render_layers(&self) -> impl Iterator<Item = &BgLayer> {
        self.layers.iter()
    }
}

fn parse_layer(map: HashMap<String, String>) -> BgLayer {
    BgLayer {
        kind: map
            .get("type")
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_else(|| "normal".into()),
        sprite: map.get("spriteno").map(|value| {
            let (x, y) = parse_pair(value);
            (x as i32, y as i32)
        }),
        action: map
            .get("actionno")
            .and_then(|value| value.trim().parse::<i32>().ok()),
        start: map
            .get("start")
            .map(|value| pair(value))
            .unwrap_or(Vec2::ZERO),
        delta: map
            .get("delta")
            .map(|value| pair(value))
            .unwrap_or(Vec2::new(1.0, 1.0)),
        trans: match map
            .get("trans")
            .map(|value| value.to_ascii_lowercase())
            .as_deref()
        {
            Some("add") | Some("add1") => BlendMode::Add,
            Some("sub") => BlendMode::Sub,
            _ => BlendMode::Normal,
        },
        tile: map
            .get("tile")
            .map(|value| {
                let (x, y) = parse_pair(value);
                (x as i32, y as i32)
            })
            .unwrap_or((0, 0)),
        tile_spacing: map
            .get("tilespacing")
            .map(|value| {
                let (x, y) = parse_pair(value);
                (x as i32, y as i32)
            })
            .unwrap_or((0, 0)),
        layer_no: map
            .get("layerno")
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0),
        mask: map
            .get("mask")
            .map(|value| value.trim() != "0")
            .unwrap_or(false),
        velocity: map
            .get("velocity")
            .map(|value| pair(value))
            .unwrap_or(Vec2::ZERO),
        sin_x: map
            .get("sin.x")
            .map(|value| triple(value))
            .unwrap_or([0.0; 3]),
        sin_y: map
            .get("sin.y")
            .map(|value| triple(value))
            .unwrap_or([0.0; 3]),
        cos_x: map
            .get("cos.x")
            .map(|value| triple(value))
            .unwrap_or([0.0; 3]),
        cos_y: map
            .get("cos.y")
            .map(|value| triple(value))
            .unwrap_or([0.0; 3]),
        position_link: map
            .get("positionlink")
            .map(|value| value.trim() != "0")
            .unwrap_or(false),
        visible: map
            .get("visible")
            .map(|value| value.trim() != "0")
            .unwrap_or(true),
        enabled: map
            .get("enabled")
            .map(|value| value.trim() != "0")
            .unwrap_or(true),
        xscale: map
            .get("xscale")
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(1.0),
        yscale: map
            .get("yscale")
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(1.0),
        window: map
            .get("window")
            .map(|value| quad(value))
            .unwrap_or([0.0; 4]),
        start_rect: map
            .get("startrect")
            .map(|value| quad(value))
            .unwrap_or([0.0; 4]),
        window_delta: map
            .get("windowdelta")
            .map(|value| pair(value))
            .unwrap_or(Vec2::ZERO),
        id: map
            .get("id")
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0),
    }
}

fn pair(value: &str) -> Vec2 {
    let (x, y) = parse_pair(value);
    Vec2::new(x, y)
}

fn triple(value: &str) -> [f32; 3] {
    let mut parts = value.split(',');
    [
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or(0.0),
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or(0.0),
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or(0.0),
    ]
}

fn quad(value: &str) -> [f32; 4] {
    let mut parts = value.split(',');
    [
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or(0.0),
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or(0.0),
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or(0.0),
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or(0.0),
    ]
}

fn parse_color(value: Option<&String>) -> [u8; 3] {
    let Some(value) = value else {
        return [0, 0, 0];
    };
    let mut parts = value.split(',');
    [
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or(0),
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or(0),
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or(0),
    ]
}

fn num(map: &HashMap<String, String>, key: &str, default: f32) -> f32 {
    map.get(key)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_layer_motion_and_visibility() {
        let mut map = HashMap::new();
        map.insert("type".into(), "anim".into());
        map.insert("actionno".into(), "1".into());
        map.insert("velocity".into(), "2,-1".into());
        map.insert("sin.x".into(), "3,60,0".into());
        map.insert("visible".into(), "0".into());
        map.insert("xscale".into(), "2".into());
        let layer = parse_layer(map);
        assert_eq!(layer.kind, "anim");
        assert_eq!(layer.action, Some(1));
        assert_eq!(layer.velocity, Vec2::new(2.0, -1.0));
        assert_eq!(layer.sin_x, [3.0, 60.0, 0.0]);
        assert!(!layer.visible);
        assert_eq!(layer.xscale, 2.0);
    }
    #[test]
    fn normalizes_backslash_music_paths() {
        let dir = std::env::temp_dir().join(format!("manato-stage-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).expect("dirs");
        std::fs::write(dir.join("sub/theme.ogg"), b"x").expect("music");
        let def = dir.join("stage.def");
        std::fs::write(
            &def,
            "[Info]\nname = Paths\n[Camera]\n[PlayerInfo]\n[BGdef]\n\
             [Music]\nbgmusic = sub\\theme.ogg\nbgmvolume = 90\n",
        )
        .expect("write def");
        let stage = Stage::load(&def, &[]).expect("stage load");
        let music = stage.music.path.expect("music path");
        assert!(music.ends_with("sub/theme.ogg"), "got {music:?}");
        assert_eq!(stage.music.volume, 90);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_model_reference() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = root.join("content/stages/training/stage1.def");
        if !source.exists() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("manato-stage-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let mut text = std::fs::read_to_string(&source).expect("def");
        text.push_str("\n[Model]\nmodel = triangle.gltf\nanim = back\nenvmap = sky.png\n");
        let def = dir.join("stage.def");
        std::fs::write(&def, text).expect("write");
        std::fs::copy(
            root.join("content/stages/training/stage1.sff"),
            dir.join("stage1.sff"),
        )
        .expect("sff");
        let stage = Stage::load(&def, std::slice::from_ref(&dir)).expect("stage");
        assert_eq!(
            stage.model.as_deref(),
            Some(dir.join("triangle.gltf").as_path())
        );
        assert_eq!(stage.model_anim.as_deref(), Some("back"));
        assert_eq!(
            stage.model_env.as_deref(),
            Some(dir.join("sky.png").as_path())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_scaling_and_projects_depth() {
        let dir = std::env::temp_dir().join(format!("manato-stage-scale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dirs");
        let def = dir.join("stage.def");
        std::fs::write(
            &def,
            "[Info]\nname = Depth\n[Camera]\n[PlayerInfo]\n[BGdef]\n\
             [StageInfo]\nzoffset = 100\n\
             [Scaling]\ntopz = -100\nbotz = 100\ntopscale = 0.5\n\
             botscale = 1.5\ndepthtoscreen = 0.25\n",
        )
        .expect("write def");
        let stage = Stage::load(&def, &[]).expect("stage load");
        assert_eq!(stage.zoffset, 100.0);
        assert_eq!(stage.z_top, -100.0);
        assert_eq!(stage.z_bottom, 100.0);
        assert_eq!(stage.z_top_scale, 0.5);
        assert_eq!(stage.z_bottom_scale, 1.5);
        assert_eq!(stage.depth_to_screen, 0.25);
        assert!((stage.z_scale(-100.0) - 0.5).abs() < 1e-5);
        assert!((stage.z_scale(0.0) - 1.0).abs() < 1e-5);
        assert!((stage.z_scale(50.0) - 1.25).abs() < 1e-5);
        assert!((stage.z_scale(100.0) - 1.5).abs() < 1e-5);
        assert!((stage.z_y_offset(40.0) - 10.0).abs() < 1e-5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_layer_geometry_and_clipping() {
        let mut map = HashMap::new();
        map.insert("type".into(), "normal".into());
        map.insert("spriteno".into(), "3,4".into());
        map.insert("start".into(), "12,-8".into());
        map.insert("delta".into(), "0.5,0.75".into());
        map.insert("trans".into(), "add".into());
        map.insert("tile".into(), "1,1".into());
        map.insert("tilespacing".into(), "4,6".into());
        map.insert("layerno".into(), "1".into());
        map.insert("mask".into(), "1".into());
        map.insert("positionlink".into(), "1".into());
        map.insert("xscale".into(), "1.5".into());
        map.insert("yscale".into(), "2.5".into());
        map.insert("window".into(), "1,2,300,200".into());
        map.insert("startrect".into(), "0,10,320,240".into());
        map.insert("windowdelta".into(), "1,-1".into());
        map.insert("id".into(), "9".into());
        let layer = parse_layer(map);
        assert_eq!(layer.sprite, Some((3, 4)));
        assert_eq!(layer.start, Vec2::new(12.0, -8.0));
        assert_eq!(layer.delta, Vec2::new(0.5, 0.75));
        assert_eq!(layer.trans, BlendMode::Add);
        assert_eq!(layer.tile, (1, 1));
        assert_eq!(layer.tile_spacing, (4, 6));
        assert_eq!(layer.layer_no, 1);
        assert!(layer.mask);
        assert!(layer.position_link);
        assert!((layer.xscale - 1.5).abs() < f32::EPSILON);
        assert!((layer.yscale - 2.5).abs() < f32::EPSILON);
        assert_eq!(layer.window, [1.0, 2.0, 300.0, 200.0]);
        assert_eq!(layer.start_rect, [0.0, 10.0, 320.0, 240.0]);
        assert_eq!(layer.window_delta, Vec2::new(1.0, -1.0));
        assert_eq!(layer.id, 9);
    }
}
