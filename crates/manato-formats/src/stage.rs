use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use manato_core::Vec2;

use crate::air::{parse_air_str, Animation, BlendMode};
use crate::def::{section_map, StageDefFile};
use crate::sff::SffFile;
use crate::text::{decode_text, parse_pair};

#[derive(Debug, Clone)]
pub struct CameraDef {
    pub start: Vec2,
    pub bound_left: f32,
    pub bound_right: f32,
    pub bound_high: f32,
    pub bound_low: f32,
    pub vertical_follow: f32,
    pub floor_tension: f32,
    pub tension: f32,
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
        }
    }
}

#[derive(Debug, Clone)]
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
    pub id: i32,
}

impl Default for BgLayer {
    fn default() -> Self {
        Self {
            kind: "normal".into(),
            sprite: None,
            action: None,
            start: Vec2::ZERO,
            delta: Vec2::new(1.0, 1.0),
            trans: BlendMode::Normal,
            tile: (0, 0),
            tile_spacing: (0, 0),
            layer_no: 0,
            mask: false,
            velocity: Vec2::ZERO,
            id: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Stage {
    pub name: String,
    pub display_name: String,
    pub localcoord: (f32, f32),
    pub camera: CameraDef,
    pub ground_level: f32,
    pub left_bound: f32,
    pub right_bound: f32,
    pub zoffset: f32,
    pub p1_start: Vec2,
    pub p2_start: Vec2,
    pub p1_facing: i32,
    pub p2_facing: i32,
    pub sprites: SffFile,
    pub animations: HashMap<i32, Animation>,
    pub layers: Vec<BgLayer>,
    pub music: Option<PathBuf>,
    pub path: PathBuf,
}

impl Stage {
    pub fn load(path: &Path, data_dirs: &[PathBuf]) -> anyhow::Result<Self> {
        let def = StageDefFile::load(path)
            .with_context(|| format!("failed to load stage def {}", path.display()))?;
        let raw = std::fs::read(path)
            .map(|b| decode_text(&b))
            .unwrap_or_default();
        let animations: HashMap<i32, Animation> = parse_air_str(&raw)
            .into_iter()
            .map(|a| (a.number, a))
            .collect();

        let spr_name = def
            .bgdef
            .get("spr")
            .or_else(|| def.info.get("spr"))
            .cloned()
            .unwrap_or_default();
        let mut sprites = SffFile::default();
        if !spr_name.is_empty() {
            let candidate = def.dir.join(&spr_name);
            if candidate.exists() {
                sprites = SffFile::load(&candidate)
                    .with_context(|| format!("failed to load stage SFF {}", candidate.display()))?;
            } else {
                for dir in data_dirs {
                    let p = dir.join(&spr_name);
                    if p.exists() {
                        sprites = SffFile::load(&p).unwrap_or_default();
                        break;
                    }
                }
            }
        }

        let mut layers = Vec::new();
        for section in &def.text.sections {
            if section.kind() != "bg" {
                continue;
            }
            let m = section_map(section);
            let mut layer = BgLayer::default();
            layer.kind = m
                .get("type")
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_else(|| "normal".into());
            layer.sprite = m.get("spriteno").map(|v| {
                let (x, y) = parse_pair(v);
                (x as i32, y as i32)
            });
            layer.action = m.get("actionno").and_then(|v| v.trim().parse::<i32>().ok());
            layer.start = m
                .get("start")
                .map(|v| {
                    let (x, y) = parse_pair(v);
                    Vec2::new(x, y)
                })
                .unwrap_or(Vec2::ZERO);
            layer.delta = m
                .get("delta")
                .map(|v| {
                    let (x, y) = parse_pair(v);
                    Vec2::new(x, y)
                })
                .unwrap_or(Vec2::new(1.0, 1.0));
            layer.trans = match m.get("trans").map(|s| s.to_ascii_lowercase()).as_deref() {
                Some("add") | Some("add1") => BlendMode::Add,
                Some("sub") => BlendMode::Sub,
                _ => BlendMode::Normal,
            };
            if let Some(v) = m.get("tile") {
                let (x, y) = parse_pair(v);
                layer.tile = (x as i32, y as i32);
            }
            if let Some(v) = m.get("tilespacing") {
                let (x, y) = parse_pair(v);
                layer.tile_spacing = (x as i32, y as i32);
            }
            layer.layer_no = m
                .get("layerno")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            layer.mask = m.get("mask").map(|v| v.trim() != "0").unwrap_or(false);
            layer.velocity = m
                .get("velocity")
                .map(|v| {
                    let (x, y) = parse_pair(v);
                    Vec2::new(x, y)
                })
                .unwrap_or(Vec2::ZERO);
            layer.id = m.get("id").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
            layers.push(layer);
        }

        // Fall back to a solid background if a stage declares no layers.
        if layers.is_empty() {
            layers.push(BgLayer {
                kind: "normal".into(),
                sprite: Some((0, 0)),
                ..Default::default()
            });
        }

        let camera_map = &def.camera;
        let camera = CameraDef {
            start: camera_map
                .get("startx")
                .and_then(|v| v.trim().parse().ok())
                .map(|x| {
                    Vec2::new(
                        x,
                        camera_map
                            .get("starty")
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0.0),
                    )
                })
                .unwrap_or(Vec2::ZERO),
            bound_left: num(camera_map, "boundleft", -1000.0),
            bound_right: num(camera_map, "boundright", 1000.0),
            bound_high: num(camera_map, "boundhigh", -100.0),
            bound_low: num(camera_map, "boundlow", 0.0),
            vertical_follow: num(camera_map, "verticalfollow", 0.2),
            floor_tension: num(camera_map, "floortension", 0.0),
            tension: num(camera_map, "tension", 50.0),
        };

        let zoffset = num(&dashboard_map(&def), "zoffset", 204.0);
        let music = def
            .text
            .find("music")
            .and_then(|s| s.get("bgmusic"))
            .map(|m| def.dir.join(m.trim()))
            .filter(|p| p.exists());

        Ok(Self {
            name: def.name,
            display_name: def.display_name,
            localcoord: def.localcoord,
            camera,
            ground_level: 0.0,
            left_bound: num(&def.player_info, "leftbound", -1000.0),
            right_bound: num(&def.player_info, "rightbound", 1000.0),
            zoffset,
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
            sprites,
            animations,
            layers,
            music,
            path: path.to_path_buf(),
        })
    }

    pub fn render_layers(&self) -> impl Iterator<Item = &BgLayer> {
        self.layers.iter()
    }
}

fn dashboard_map(def: &StageDefFile) -> HashMap<String, String> {
    def.text
        .find("stageinfo")
        .map(section_map)
        .unwrap_or_default()
}

fn num(map: &HashMap<String, String>, key: &str, default: f32) -> f32 {
    map.get(key)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}
