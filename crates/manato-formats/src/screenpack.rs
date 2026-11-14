//! Screenpack (`system.def`, `fight.def`, `select.def`) parsing.
//!
//! Implements the subset frozen in `docs/spec/render.md` §5–§8. The parser is
//! tolerant: unknown sections and keys are ignored, and a malformed element is
//! skipped individually instead of failing the whole screenpack.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::Context;
use manato_core::Vec2;

use crate::air::BlendMode;
use crate::def::section_map;
use crate::sff::SffFile;
use crate::stage::{BgCtrl, BgLayer};
use crate::text::{parse_pair, split_params, Section, TextFile};

/// Default screenpack localcoord.
pub const DEFAULT_LOCALCOORD: (f32, f32) = (320.0, 240.0);

#[derive(Debug, Clone)]
pub struct FontDecl {
    pub path: Option<PathBuf>,
    pub bank: i32,
    pub tint: [u8; 3],
}

impl Default for FontDecl {
    fn default() -> Self {
        Self {
            path: None,
            bank: 0,
            tint: [255, 255, 255],
        }
    }
}

/// A single sprite or animation layer.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub sprite: Option<(i32, i32)>,
    pub anim: Option<i32>,
    pub offset: Vec2,
    pub scale: Vec2,
    pub layerno: i32,
    pub blend: BlendMode,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            sprite: None,
            anim: None,
            offset: Vec2::ZERO,
            scale: Vec2::new(1.0, 1.0),
            layerno: 0,
            blend: BlendMode::Normal,
        }
    }
}

impl Layer {
    pub fn is_empty(&self) -> bool {
        self.sprite.is_none() && self.anim.is_none()
    }
}

/// A text element.
#[derive(Debug, Clone, PartialEq)]
pub struct TextDef {
    pub font: u32,
    pub text: String,
    pub offset: Vec2,
    pub scale: Vec2,
    pub align: i32,
    pub color: [u8; 4],
}

impl Default for TextDef {
    fn default() -> Self {
        Self {
            font: 0,
            text: String::new(),
            offset: Vec2::ZERO,
            scale: Vec2::new(1.0, 1.0),
            align: 0,
            color: [255, 255, 255, 255],
        }
    }
}

/// A combined sprite/animation/text element used by title and round screens.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Element {
    pub pos: Vec2,
    pub offset: Vec2,
    pub scale: Vec2,
    pub layerno: i32,
    pub sprite: Option<(i32, i32)>,
    pub anim: Option<i32>,
    pub text: String,
    pub font: u32,
    pub align: i32,
    pub color: [u8; 4],
    pub snd: Option<(i32, i32)>,
}

impl Element {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.sprite.is_none() && self.anim.is_none()
    }
}

/// A M.U.G.E.N life/power/guard bar.
#[derive(Debug, Clone, Default)]
pub struct BarDef {
    pub pos: Vec2,
    pub range: Vec2,
    pub bg0: Layer,
    pub bg1: Layer,
    pub bg2: Layer,
    pub top: Layer,
    pub front: Layer,
    pub mid: Layer,
    pub red: Layer,
    pub shift: Layer,
    pub warn: Layer,
    pub value: TextDef,
    pub counter: TextDef,
    pub level: [Layer; 3],
    pub mid_delay: i32,
    pub mid_freeze: bool,
    pub mid_mult: f32,
    pub mid_steps: f32,
    pub warn_range: Vec2,
    pub scalefill: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FaceDef {
    pub pos: Vec2,
    pub bg: Layer,
    pub bg0: Layer,
    pub bg1: Layer,
    pub bg2: Layer,
    pub top: Layer,
    pub ko: Layer,
    pub face_spr: Option<(i32, i32)>,
    pub palshare: bool,
}

#[derive(Debug, Clone, Default)]
pub struct NameDef {
    pub pos: Vec2,
    pub bg: Layer,
    pub top: Layer,
    pub name: TextDef,
}

#[derive(Debug, Clone, Default)]
pub struct TimeDef {
    pub pos: Vec2,
    pub bg: Layer,
    pub top: Layer,
    pub counter: TextDef,
    pub displaytime: i32,
    /// Frames per displayed timer count (`time.framespercount`).
    pub framespercount: i32,
}

#[derive(Debug, Clone, Default)]
pub struct WinIconDef {
    pub pos: Vec2,
    pub icon_offset: Vec2,
    pub bg0: Layer,
    pub top: Layer,
    pub normal: Layer,
    pub special: Layer,
    pub hyper: Layer,
    pub cheese: Layer,
    pub time: Layer,
    pub perfect: Layer,
    pub clutch: Layer,
    pub counter: TextDef,
    pub use_icon_upto: i32,
}

/// A fight-screen action banner (`docs/spec/render.md` §6.10).
#[derive(Debug, Clone, Default)]
pub struct ActionDef {
    pub pos: Vec2,
    pub spacing: Vec2,
    pub start_x: f32,
    pub displaytime: i32,
    pub showspeed: f32,
    pub hidespeed: f32,
    pub bg: Layer,
    pub front: Layer,
    pub text: TextDef,
}

#[derive(Debug, Clone, Default)]
pub struct ComboDef {
    pub pos: Vec2,
    pub start_x: f32,
    pub bg: Layer,
    pub top: Layer,
    pub counter: TextDef,
    pub text: TextDef,
    pub displaytime: i32,
    pub showspeed: f32,
    pub hidespeed: f32,
}

/// Round announcements and their timings.
#[derive(Debug, Clone)]
pub struct RoundDef {
    pub pos: Vec2,
    pub start_waittime: i32,
    pub round_time: i32,
    pub fight_time: i32,
    pub ctrl_time: i32,
    pub ko_time: i32,
    pub dko_time: i32,
    pub to_time: i32,
    pub win_time: i32,
    pub win2_time: i32,
    /// `over.*` outcome windows and the hardcoded-style `callfight.time`.
    pub over_waittime: i32,
    pub over_hittime: i32,
    pub over_wintime: i32,
    pub over_forcewintime: i32,
    pub over_time: i32,
    pub callfight_time: i32,
    pub dko_showdraw: bool,
    pub elements: BTreeMap<String, Element>,
    /// Ticks of KO slow-motion (`slow.time`).
    pub slow_time: i32,
    /// Ticks over which the slow-motion fades back to normal (`slow.fadetime`).
    pub slow_fade_time: i32,
    /// Initial logic-speed fraction during KO slow-motion (`slow.speed`).
    pub slow_speed: f32,
}

impl Default for RoundDef {
    fn default() -> Self {
        Self {
            pos: Vec2::ZERO,
            start_waittime: 90,
            round_time: 45,
            fight_time: 45,
            ctrl_time: 30,
            ko_time: 30,
            dko_time: 30,
            to_time: 30,
            win_time: 45,
            win2_time: 45,
            over_waittime: 45,
            over_hittime: 10,
            over_wintime: 45,
            over_forcewintime: 900,
            over_time: 210,
            callfight_time: 60,
            dko_showdraw: false,
            elements: BTreeMap::new(),
            slow_time: 60,
            slow_fade_time: 45,
            slow_speed: 0.25,
        }
    }
}

impl RoundDef {
    pub fn element(&self, name: &str) -> Option<&Element> {
        self.elements.get(name)
    }
}

#[derive(Debug, Clone, Default)]
pub struct FightDef {
    pub name: String,
    pub author: String,
    pub localcoord: (f32, f32),
    pub life: [BarDef; 2],
    pub power: [BarDef; 2],
    pub guard: [BarDef; 2],
    pub face: [FaceDef; 2],
    pub name_def: [NameDef; 2],
    pub time: TimeDef,
    pub win_icon: [WinIconDef; 2],
    pub combo: [ComboDef; 2],
    /// `[Action]` banners, indexed by team side.
    pub action: [ActionDef; 2],
    pub round: RoundDef,
    /// Animations from `[Files] air`, used by `anim` layers.
    pub animations: HashMap<i32, crate::air::Animation>,
    /// `[Files] snd`, if declared.
    pub sound_path: Option<PathBuf>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct MenuItemDef {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, Default)]
pub struct CursorDef {
    pub sprite: Option<(i32, i32)>,
    pub anim: Option<i32>,
    pub offset: Vec2,
    pub active: Option<(i32, i32)>,
    pub done: Option<(i32, i32)>,
    pub move_snd: Option<(i32, i32)>,
    pub done_snd: Option<(i32, i32)>,
}

#[derive(Debug, Clone, Default)]
pub struct TitleDef {
    pub title: Element,
    pub menu_pos: Vec2,
    pub item_offset: Vec2,
    pub item_spacing: Vec2,
    pub item_scale: Vec2,
    pub item_font: u32,
    pub item_color: [u8; 4],
    pub active_font: u32,
    pub active_scale: Vec2,
    pub active_color: [u8; 4],
    pub active_sprite: Option<(i32, i32)>,
    pub bg_sprite: Option<(i32, i32)>,
    pub items: Vec<MenuItemDef>,
    pub cursor_p1: CursorDef,
    pub cursor_p2: CursorDef,
}

#[derive(Debug, Clone, Default)]
pub struct SelectPlayerDef {
    pub cursor_start: (i32, i32),
    pub cursor: CursorDef,
    pub cursor_done: CursorDef,
    pub face: Layer,
    pub face_spr: Option<(i32, i32)>,
    pub name: TextDef,
}

#[derive(Debug, Clone, Default)]
pub struct SelectDef {
    pub rows: i32,
    pub columns: i32,
    pub wrapping: bool,
    pub pos: Vec2,
    pub show_empty_boxes: bool,
    pub move_over_empty_boxes: bool,
    pub cell_size: Vec2,
    pub cell_spacing: Vec2,
    pub cell_bg: Layer,
    pub cell_random: Layer,
    pub cell_face: Layer,
    pub cell_name: TextDef,
    pub players: [SelectPlayerDef; 2],
    pub stage: Element,
    pub done_keys: Vec<String>,
    pub cancel_keys: Vec<String>,
    pub timer: TimeDef,
    pub timer_count: i32,
}

/// Inline background scene (`[TitleBGdef]`, `[SelectBGdef]`).
#[derive(Debug, Clone, Default)]
pub struct ScreenBg {
    pub sprites: SffFile,
    pub layers: Vec<BgLayer>,
    pub animations: HashMap<i32, crate::air::Animation>,
    pub bg_ctrls: Vec<BgCtrl>,
    pub clear_color: [u8; 3],
    pub clear_alpha: u8,
    pub clear_layerno: i32,
}

#[derive(Debug, Clone, Default)]
pub struct Screenpack {
    pub dir: PathBuf,
    pub system_def: PathBuf,
    pub name: String,
    pub author: String,
    pub localcoord: (f32, f32),
    pub fonts: BTreeMap<u32, FontDecl>,
    /// Fonts that loaded successfully, keyed by `[Files] fontN` index.
    pub loaded_fonts: BTreeMap<u32, crate::font::BitmapFont>,
    pub sprites: SffFile,
    pub title: TitleDef,
    pub title_bg: Option<ScreenBg>,
    pub select: SelectDef,
    pub select_bg: Option<ScreenBg>,
    pub fight: FightDef,
    /// `[WinQuote]` text element from `system.def`.
    pub win_quote: TextDef,
    pub fight_path: Option<PathBuf>,
    pub select_def_path: Option<PathBuf>,
}

impl Screenpack {
    pub fn load(system_def: &Path) -> anyhow::Result<Self> {
        let text = TextFile::load(system_def)
            .with_context(|| format!("failed to read screenpack {}", system_def.display()))?;
        let dir = system_def.parent().unwrap_or(Path::new(".")).to_path_buf();
        let info = text.find("info").map(section_map).unwrap_or_default();
        let files = text.find("files").map(section_map).unwrap_or_default();
        let localcoord = info
            .get("localcoord")
            .map(|value| parse_pair(value))
            .filter(|(w, h)| *w > 0.0 && *h > 0.0)
            .unwrap_or(DEFAULT_LOCALCOORD);

        let mut fonts = BTreeMap::new();
        collect_fonts(&mut fonts, &text, &dir);

        let sprites = files
            .get("spr")
            .and_then(|value| resolve_asset(&dir, value))
            .and_then(|path| SffFile::load(&path).ok())
            .unwrap_or_default();

        let fight_path = files
            .get("fight")
            .and_then(|value| resolve_asset(&dir, value));
        let fight = match &fight_path {
            Some(path) => {
                collect_fonts_for_path(&mut fonts, path, &dir);
                FightDef::load(path).unwrap_or_else(|error| {
                    log_warn(&format!("failed to load {}: {error:#}", path.display()));
                    FightDef::default()
                })
            }
            None => FightDef::default(),
        };

        let select_def_path = files
            .get("select")
            .and_then(|value| resolve_asset(&dir, value));

        // Load every declared font once; failures fall back to the built-in
        // engine font at draw time (`docs/spec/render.md` §3.3).
        let mut loaded_fonts = BTreeMap::new();
        for (index, decl) in &fonts {
            let Some(path) = &decl.path else {
                continue;
            };
            match crate::font::BitmapFont::load(path) {
                Ok(font) => {
                    loaded_fonts.insert(*index, font);
                }
                Err(error) => {
                    log_warn(&format!(
                        "failed to load font {index} from {}: {error:#}",
                        path.display()
                    ));
                }
            }
        }

        let win_quote = text
            .find("winquote")
            .map(section_map)
            .map(|map| parse_text(&map, ""))
            .unwrap_or_default();

        Ok(Self {
            dir: dir.clone(),
            system_def: system_def.to_path_buf(),
            name: info
                .get("name")
                .cloned()
                .unwrap_or_else(|| "Screenpack".into()),
            author: info.get("author").cloned().unwrap_or_default(),
            localcoord,
            fonts,
            loaded_fonts,
            sprites,
            title: TitleDef::from_text(&text),
            title_bg: ScreenBg::from_text(&text, &dir, "titlebgdef"),
            select: SelectDef::from_text(&text),
            select_bg: ScreenBg::from_text(&text, &dir, "selectbgdef"),
            fight,
            win_quote,
            fight_path,
            select_def_path,
        })
    }

    pub fn title_background(&self) -> Option<&ScreenBg> {
        self.title_bg.as_ref()
    }
}

impl FightDef {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = TextFile::load(path)
            .with_context(|| format!("failed to read fight def {}", path.display()))?;
        let info = text
            .find_header("info")
            .or_else(|| text.find("fightinfo"))
            .map(section_map)
            .unwrap_or_default();
        let localcoord = info
            .get("localcoord")
            .map(|value| parse_pair(value))
            .filter(|(w, h)| *w > 0.0 && *h > 0.0)
            .unwrap_or(DEFAULT_LOCALCOORD);

        let base_life = text
            .find_header("lifebar")
            .map(section_map)
            .unwrap_or_default();
        let base_power = text
            .find_header("powerbar")
            .map(section_map)
            .unwrap_or_default();
        let base_guard = text
            .find_header("guardbar")
            .map(section_map)
            .unwrap_or_default();
        let base_face = text
            .find_header("face")
            .map(section_map)
            .unwrap_or_default();
        let base_name = text
            .find_header("name")
            .map(section_map)
            .unwrap_or_default();
        let base_win = text
            .find_header("winicon")
            .map(section_map)
            .unwrap_or_default();
        let base_combo = text
            .find_header("combo")
            .map(section_map)
            .unwrap_or_default();

        let mut life = [BarDef::default(), BarDef::default()];
        let mut power = [BarDef::default(), BarDef::default()];
        let mut guard = [BarDef::default(), BarDef::default()];
        let mut face = [FaceDef::default(), FaceDef::default()];
        let mut name_def = [NameDef::default(), NameDef::default()];
        let mut win_icon = [WinIconDef::default(), WinIconDef::default()];
        let mut combo = [ComboDef::default(), ComboDef::default()];
        for side in 0..2 {
            let prefix = if side == 0 { "p1." } else { "p2." };
            life[side] = parse_bar(&overlay(&base_life, prefix), true);
            power[side] = parse_bar(&overlay(&base_power, prefix), false);
            guard[side] = parse_bar(&overlay(&base_guard, prefix), false);
            face[side] = parse_face(&overlay(&base_face, prefix));
            name_def[side] = parse_name(&overlay(&base_name, prefix));
            win_icon[side] = parse_win_icon(&overlay(&base_win, prefix));
            combo[side] = parse_combo(&overlay(&base_combo, prefix));
        }

        // Section variants such as [LifeBarP1] overlay the shared base, so a
        // variant only has to restate what differs (`pos`, for example).
        let variant = |name: &str, base: &HashMap<String, String>, prefix: &str| {
            let mut map = overlay(base, prefix);
            if let Some(section) = text.find_header(name) {
                map.extend(section_map(section));
            }
            map
        };
        life[0] = parse_bar(&variant("lifebarp1", &base_life, "p1."), true);
        life[1] = parse_bar(&variant("lifebarp2", &base_life, "p2."), true);
        power[0] = parse_bar(&variant("powerbarp1", &base_power, "p1."), false);
        power[1] = parse_bar(&variant("powerbarp2", &base_power, "p2."), false);
        guard[0] = parse_bar(&variant("guardbarp1", &base_guard, "p1."), false);
        guard[1] = parse_bar(&variant("guardbarp2", &base_guard, "p2."), false);
        face[0] = parse_face(&variant("facep1", &base_face, "p1."));
        face[1] = parse_face(&variant("facep2", &base_face, "p2."));
        name_def[0] = parse_name(&variant("namep1", &base_name, "p1."));
        name_def[1] = parse_name(&variant("namep2", &base_name, "p2."));
        win_icon[0] = parse_win_icon(&variant("winiconp1", &base_win, "p1."));
        win_icon[1] = parse_win_icon(&variant("winiconp2", &base_win, "p2."));
        combo[0] = parse_combo(&variant("combop1", &base_combo, "p1."));
        combo[1] = parse_combo(&variant("combop2", &base_combo, "p2."));

        // `[Action]` uses team1./team2. overrides rather than p1./p2.
        let base_action = text
            .find_header("action")
            .map(section_map)
            .unwrap_or_default();
        let action = [
            parse_action(&overlay(&base_action, "team1.")),
            parse_action(&overlay(&base_action, "team2.")),
        ];

        let time = text
            .find_header("time")
            .map(section_map)
            .map(|map| parse_time(&map, ""))
            .unwrap_or_default();
        let round = text
            .find_header("round")
            .map(section_map)
            .map(|map| parse_round(&map))
            .unwrap_or_default();

        let dir = path.parent().unwrap_or(Path::new("."));
        let files = text.find("files").map(section_map).unwrap_or_default();
        let animations = files
            .get("air")
            .and_then(|value| resolve_asset(dir, value))
            .and_then(|air| match crate::air::parse_air(&air) {
                Ok(animations) => Some(
                    animations
                        .into_iter()
                        .map(|animation| (animation.number, animation))
                        .collect(),
                ),
                Err(error) => {
                    log_warn(&format!("failed to load {}: {error:#}", air.display()));
                    None
                }
            })
            .unwrap_or_default();
        let sound_path = files.get("snd").and_then(|value| resolve_asset(dir, value));

        Ok(Self {
            name: info.get("name").cloned().unwrap_or_default(),
            author: info.get("author").cloned().unwrap_or_default(),
            localcoord,
            life,
            power,
            guard,
            face,
            name_def,
            time,
            win_icon,
            combo,
            action,
            round,
            animations,
            sound_path,
            path: path.to_path_buf(),
        })
    }
}

impl TitleDef {
    fn from_text(text: &TextFile) -> Self {
        let map = text
            .find_header("title info")
            .map(section_map)
            .unwrap_or_default();
        let title = parse_element(&map, "title.");
        let mut items = Vec::new();
        if let Some(section) = text.find_header("title info") {
            for entry in &section.entries {
                if let Some(key) = entry
                    .key
                    .trim()
                    .to_ascii_lowercase()
                    .strip_prefix("menu.itemname.")
                {
                    let label = entry.value.trim().to_string();
                    if !label.is_empty() {
                        items.push(MenuItemDef {
                            key: key.to_string(),
                            label,
                        });
                    }
                }
            }
        }
        let item_scale = map
            .get("menu.item.scale")
            .map(|value| parse_pair(value))
            .map(|(x, y)| Vec2::new(x, y))
            .unwrap_or(Vec2::new(1.0, 1.0));
        let active_scale = map
            .get("menu.item.active.scale")
            .map(|value| parse_pair(value))
            .map(|(x, y)| Vec2::new(x, y))
            .unwrap_or(item_scale);
        Self {
            title,
            menu_pos: map
                .get("menu.pos")
                .map(|value| vec2(value))
                .unwrap_or_default(),
            item_offset: map
                .get("menu.item.offset")
                .map(|value| vec2(value))
                .unwrap_or_default(),
            item_spacing: map
                .get("menu.item.spacing")
                .map(|value| vec2(value))
                .unwrap_or_else(|| Vec2::new(0.0, 12.0)),
            item_scale,
            item_font: font_index(&map, "menu.item.font"),
            item_color: color(&map, "menu.item.color"),
            active_font: map
                .get("menu.item.active.font")
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or_else(|| font_index(&map, "menu.item.font")),
            active_scale,
            active_color: color(&map, "menu.item.active.color"),
            active_sprite: map
                .get("menu.item.active.spr")
                .map(|value| int_pair(value))
                .or_else(|| map.get("menu.item.bg.spr").map(|value| int_pair(value))),
            bg_sprite: map.get("menu.background.spr").map(|value| int_pair(value)),
            items,
            cursor_p1: parse_cursor(&map, "menu.p1.cursor."),
            cursor_p2: parse_cursor(&map, "menu.p2.cursor."),
        }
    }
}

impl SelectDef {
    fn from_text(text: &TextFile) -> Self {
        let map = text
            .find_header("select info")
            .map(section_map)
            .unwrap_or_default();
        let cell_size = map
            .get("cell.size")
            .map(|value| vec2(value))
            .unwrap_or_default();
        let cell_spacing = map
            .get("cell.spacing")
            .map(|value| vec2(value))
            .unwrap_or_default();
        let mut players = [SelectPlayerDef::default(), SelectPlayerDef::default()];
        for (side, slot) in players.iter_mut().enumerate() {
            let prefix = if side == 0 { "p1." } else { "p2." };
            let start = map
                .get(&format!("{prefix}cursor.startcell"))
                .map(|value| int_pair(value))
                .unwrap_or((0, 0));
            slot.cursor_start = start;
            slot.cursor = parse_cursor(&map, &format!("{prefix}cursor."));
            slot.cursor_done = parse_cursor(&map, &format!("{prefix}cursor.done."));
            slot.face = parse_layer(&map, &format!("{prefix}face."));
            slot.face_spr = map
                .get(&format!("{prefix}face.spr"))
                .map(|value| int_pair(value));
            slot.name = parse_text(&map, &format!("{prefix}name."));
        }
        let done_keys = key_list(&map, "done.key");
        let cancel_keys = key_list(&map, "cancel.key");
        Self {
            rows: int(&map, "rows", 2).max(1),
            columns: int(&map, "columns", 5).max(1),
            wrapping: flag(&map, "wrapping", false),
            pos: map.get("pos").map(|value| vec2(value)).unwrap_or_default(),
            show_empty_boxes: flag(&map, "showemptyboxes", false),
            move_over_empty_boxes: flag(&map, "moveoveremptyboxes", false),
            cell_size,
            cell_spacing,
            cell_bg: parse_layer(&map, "cell.bg."),
            cell_random: parse_layer(&map, "cell.random."),
            cell_face: parse_layer(&map, "cell.face."),
            cell_name: parse_text(&map, "cell.name."),
            players,
            stage: parse_element(&map, "stage."),
            done_keys,
            cancel_keys,
            timer: parse_time(&map, "timer."),
            timer_count: int(&map, "timer.count", 0),
        }
    }
}

impl ScreenBg {
    pub fn from_text(text: &TextFile, dir: &Path, header: &str) -> Option<Self> {
        let head = text.find_header(header).map(section_map)?;
        let sprites = head
            .get("spr")
            .and_then(|value| resolve_asset(dir, value))
            .and_then(|path| SffFile::load(&path).ok())
            .unwrap_or_default();
        let mut layers = Vec::new();
        let mut bg_ctrls = Vec::new();
        let prefix = format!("{header} bg");
        let ctrl_prefix = format!("{header} bgctrl");
        for section in &text.sections {
            let lowered = section.header_lower();
            if lowered.starts_with(&ctrl_prefix) {
                let layer = section
                    .header_lower()
                    .split_whitespace()
                    .nth(2)
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                let params = params_of(section);
                let kind = params
                    .iter()
                    .find(|(key, _)| key == "type")
                    .map(|(_, value)| value.to_ascii_lowercase())
                    .unwrap_or_else(|| "null".into());
                bg_ctrls.push(BgCtrl {
                    layer,
                    controller: crate::cns::Controller {
                        kind,
                        params,
                        line: section.line,
                    },
                });
            } else if lowered.starts_with(&prefix) {
                let suffix = lowered[prefix.len()..].trim();
                if suffix.is_empty() || !suffix.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    continue;
                }
                let mut map = section_map(section);
                if map.is_empty() {
                    continue;
                }
                normalize_bg_keys(&mut map);
                layers.push(layer_from_map(&map));
            }
        }
        let alpha = head
            .get("bgclearalpha")
            .and_then(|value| value.split(',').next())
            .and_then(|value| value.trim().parse::<i32>().ok())
            .unwrap_or(255)
            .clamp(0, 255) as u8;
        let animations = text
            .find("files")
            .map(section_map)
            .and_then(|files| files.get("air").cloned())
            .and_then(|air| resolve_asset(dir, &air))
            .and_then(|air| match crate::air::parse_air(&air) {
                Ok(animations) => Some(
                    animations
                        .into_iter()
                        .map(|animation| (animation.number, animation))
                        .collect(),
                ),
                Err(error) => {
                    log_warn(&format!("failed to load {}: {error:#}", air.display()));
                    None
                }
            })
            .unwrap_or_default();
        Some(Self {
            sprites,
            layers,
            animations,
            bg_ctrls,
            clear_color: head
                .get("bgclearcolor")
                .map(|value| rgb(value))
                .unwrap_or([0, 0, 0]),
            clear_alpha: alpha,
            clear_layerno: int(&head, "bgclearlayerno", 0),
        })
    }
}

/// A parsed `select.def` roster.
#[derive(Debug, Clone, Default)]
pub struct Roster {
    pub entries: Vec<RosterEntry>,
    /// `[ExtraStages]` entries, in file order.
    pub stages: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RosterEntry {
    pub def: String,
    pub stage: Option<String>,
    pub random: bool,
}

impl Roster {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read roster {}", path.display()))?;
        Ok(Self::parse_str(&crate::text::decode_text(&bytes)))
    }

    /// `select.def` `[Characters]` lines are positional, not `key = value`, so
    /// they are parsed from raw lines rather than through [`TextFile`].
    pub fn parse_str(raw: &str) -> Self {
        let mut entries = Vec::new();
        let mut stages = Vec::new();
        let mut in_characters = false;
        let mut in_stages = false;
        for line in raw.lines() {
            let line = crate::text::strip_comment(line).trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                let header = line.trim_matches(|c| c == '[' || c == ']').trim();
                in_characters = header.eq_ignore_ascii_case("characters");
                in_stages = header.eq_ignore_ascii_case("extrastages");
                continue;
            }
            if in_stages {
                let stage = line.trim().trim_matches('"');
                if !stage.is_empty() {
                    stages.push(stage.to_string());
                }
                continue;
            }
            if !in_characters {
                continue;
            }
            let parts = split_params(line);
            let Some(first) = parts.first() else {
                continue;
            };
            let def = first.trim().trim_matches('"');
            if def.is_empty() {
                continue;
            }
            if def.eq_ignore_ascii_case("randomselect") {
                entries.push(RosterEntry {
                    def: String::new(),
                    stage: None,
                    random: true,
                });
                continue;
            }
            entries.push(RosterEntry {
                def: def.to_string(),
                stage: parts
                    .get(1)
                    .map(|value| value.trim().trim_matches('"').to_string())
                    .filter(|value| !value.is_empty()),
                random: false,
            });
        }
        Self { entries, stages }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

fn parse_bar(map: &HashMap<String, String>, has_value: bool) -> BarDef {
    let range = map
        .get("range.x")
        .map(|x| {
            let x = x.trim().parse::<f32>().unwrap_or(0.0);
            let y = map
                .get("range.y")
                .and_then(|value| value.trim().parse::<f32>().ok())
                .unwrap_or(x);
            Vec2::new(x, y)
        })
        .unwrap_or_default();
    let mut red = parse_layer(map, "red.");
    if red.is_empty() {
        red = parse_layer(map, "danger.");
    }
    let shift = parse_layer(map, "shift.");
    BarDef {
        pos: map.get("pos").map(|value| vec2(value)).unwrap_or_default(),
        range,
        bg0: parse_layer(map, "bg0."),
        bg1: parse_layer(map, "bg1."),
        bg2: parse_layer(map, "bg2."),
        top: parse_layer(map, "top."),
        front: parse_layer(map, "front."),
        mid: parse_layer(map, "mid."),
        red,
        shift,
        warn: parse_layer(map, "warn."),
        value: if has_value {
            parse_text(map, "value.")
        } else {
            TextDef::default()
        },
        counter: if has_value {
            TextDef::default()
        } else {
            parse_text(map, "counter.")
        },
        level: [
            parse_layer(map, "level1."),
            parse_layer(map, "level2."),
            parse_layer(map, "level3."),
        ],
        mid_delay: int(map, "mid.delay", 30),
        mid_freeze: flag(map, "mid.freeze", true),
        mid_mult: num(map, "mid.mult", 1.0),
        mid_steps: num(map, "mid.steps", 8.0).max(1.0),
        warn_range: map
            .get("warn.range")
            .map(|value| vec2(value))
            .unwrap_or_default(),
        scalefill: flag(map, "scalefill", false),
    }
}

fn parse_face(map: &HashMap<String, String>) -> FaceDef {
    FaceDef {
        pos: map.get("pos").map(|value| vec2(value)).unwrap_or_default(),
        bg: parse_layer(map, "bg."),
        bg0: parse_layer(map, "bg0."),
        bg1: parse_layer(map, "bg1."),
        bg2: parse_layer(map, "bg2."),
        top: parse_layer(map, "top."),
        ko: parse_layer(map, "ko."),
        face_spr: map.get("face.spr").map(|value| int_pair(value)),
        palshare: flag(map, "face.palshare", false),
    }
}

fn parse_name(map: &HashMap<String, String>) -> NameDef {
    NameDef {
        pos: map.get("pos").map(|value| vec2(value)).unwrap_or_default(),
        bg: parse_layer(map, "bg."),
        top: parse_layer(map, "top."),
        name: parse_text(map, "name."),
    }
}

fn parse_time(map: &HashMap<String, String>, prefix: &str) -> TimeDef {
    let pos_key = format!("{prefix}pos");
    TimeDef {
        pos: map
            .get(&pos_key)
            .map(|value| vec2(value))
            .unwrap_or_default(),
        bg: parse_layer(map, &format!("{prefix}bg.")),
        top: parse_layer(map, &format!("{prefix}top.")),
        counter: parse_text(map, &format!("{prefix}counter.")),
        displaytime: int(
            map,
            &format!("{prefix}counter.displaytime"),
            int(map, &format!("{prefix}displaytime"), 0),
        ),
        framespercount: int(map, &format!("{prefix}framespercount"), 60),
    }
}

fn parse_win_icon(map: &HashMap<String, String>) -> WinIconDef {
    WinIconDef {
        pos: map.get("pos").map(|value| vec2(value)).unwrap_or_default(),
        icon_offset: map
            .get("iconoffset")
            .map(|value| vec2(value))
            .unwrap_or_default(),
        bg0: parse_layer(map, "bg0."),
        top: parse_layer(map, "top."),
        normal: parse_layer(map, "n."),
        special: parse_layer(map, "s."),
        hyper: parse_layer(map, "h."),
        cheese: parse_layer(map, "c."),
        time: parse_layer(map, "t."),
        perfect: parse_layer(map, "perfect."),
        clutch: parse_layer(map, "clutch."),
        counter: parse_text(map, "counter."),
        use_icon_upto: int(map, "useiconupto", 0),
    }
}

fn parse_combo(map: &HashMap<String, String>) -> ComboDef {
    ComboDef {
        pos: map.get("pos").map(|value| vec2(value)).unwrap_or_default(),
        start_x: num(map, "start.x", 0.0),
        bg: parse_layer(map, "bg."),
        top: parse_layer(map, "top."),
        counter: parse_text(map, "counter."),
        text: parse_text(map, "text."),
        displaytime: int(map, "displaytime", 60),
        showspeed: num(map, "showspeed", 2.0).max(0.1),
        hidespeed: num(map, "hidespeed", 2.0).max(0.1),
    }
}

fn parse_action(map: &HashMap<String, String>) -> ActionDef {
    ActionDef {
        pos: map.get("pos").map(|value| vec2(value)).unwrap_or_default(),
        spacing: map
            .get("spacing")
            .map(|value| vec2(value))
            .unwrap_or_else(|| Vec2::new(0.0, 12.0)),
        start_x: num(map, "start.x", 0.0),
        displaytime: int(map, "displaytime", 60).max(1),
        showspeed: num(map, "showspeed", 2.0).max(0.1),
        hidespeed: num(map, "hidespeed", 2.0).max(0.1),
        bg: parse_layer(map, "bg."),
        front: parse_layer(map, "front."),
        text: parse_text(map, "text."),
    }
}

fn parse_round(map: &HashMap<String, String>) -> RoundDef {
    let mut elements = BTreeMap::new();
    let mut names: Vec<String> = vec![
        "round.default".into(),
        "round.single".into(),
        "round.final".into(),
        "fight".into(),
        "ko".into(),
        "dko".into(),
        "to".into(),
        "win".into(),
        "win2".into(),
    ];
    for entry in map.keys() {
        if let Some(rest) = entry.strip_prefix("round") {
            let number = rest.split('.').next().unwrap_or("");
            if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) {
                names.push(format!("round{number}"));
            }
        }
    }
    for name in names {
        let prefix = format!("{name}.");
        let element = parse_element(map, &prefix);
        if !element.is_empty() {
            elements.insert(name.to_string(), element);
        }
    }
    RoundDef {
        pos: map.get("pos").map(|value| vec2(value)).unwrap_or_default(),
        start_waittime: int(map, "start.waittime", 90).max(1),
        round_time: int(map, "round.time", 45),
        fight_time: int(map, "fight.time", 45),
        ctrl_time: int(map, "ctrl.time", 30).max(1),
        ko_time: int(map, "ko.time", 30),
        dko_time: int(map, "dko.time", 30),
        to_time: int(map, "to.time", 30),
        win_time: int(map, "win.time", 45),
        win2_time: int(map, "win2.time", 45),
        over_waittime: int(map, "over.waittime", 45).max(1),
        over_hittime: int(map, "over.hittime", 10).max(1),
        over_wintime: int(map, "over.wintime", 45).max(1),
        over_forcewintime: int(map, "over.forcewintime", 900).max(1),
        over_time: int(map, "over.time", 210).max(1),
        callfight_time: int(map, "callfight.time", 60).max(1),
        dko_showdraw: flag(map, "dko.showdraw", false),
        elements,
        slow_time: int(map, "slow.time", 60),
        slow_fade_time: int(map, "slow.fadetime", 45),
        slow_speed: num(map, "slow.speed", 0.25).clamp(0.01, 1.0),
    }
}

fn parse_element(map: &HashMap<String, String>, prefix: &str) -> Element {
    Element {
        pos: map
            .get(&format!("{prefix}pos"))
            .map(|value| vec2(value))
            .or_else(|| {
                map.get("pos")
                    .filter(|_| prefix.is_empty())
                    .map(|value| vec2(value))
            })
            .unwrap_or_default(),
        offset: map
            .get(&format!("{prefix}offset"))
            .map(|value| vec2(value))
            .unwrap_or_default(),
        scale: map
            .get(&format!("{prefix}scale"))
            .map(|value| vec2(value))
            .unwrap_or_else(|| Vec2::new(1.0, 1.0)),
        layerno: int(map, &format!("{prefix}layerno"), 0).clamp(0, 1),
        sprite: map
            .get(&format!("{prefix}spr"))
            .map(|value| int_pair(value)),
        anim: map
            .get(&format!("{prefix}anim"))
            .and_then(|value| value.trim().parse().ok()),
        text: map
            .get(&format!("{prefix}text"))
            .cloned()
            .unwrap_or_default(),
        font: font_index(map, &format!("{prefix}font")),
        align: int(map, &format!("{prefix}align"), 0),
        color: color(map, &format!("{prefix}color")),
        snd: map
            .get(&format!("{prefix}snd"))
            .map(|value| int_pair(value))
            .filter(|(group, _)| *group >= 0),
    }
}

fn parse_layer(map: &HashMap<String, String>, prefix: &str) -> Layer {
    Layer {
        sprite: map
            .get(&format!("{prefix}spr"))
            .map(|value| int_pair(value)),
        anim: map
            .get(&format!("{prefix}anim"))
            .and_then(|value| value.trim().parse().ok()),
        offset: map
            .get(&format!("{prefix}offset"))
            .map(|value| vec2(value))
            .unwrap_or_default(),
        scale: map
            .get(&format!("{prefix}scale"))
            .map(|value| vec2(value))
            .unwrap_or_else(|| Vec2::new(1.0, 1.0)),
        layerno: int(map, &format!("{prefix}layerno"), 0).clamp(0, 1),
        blend: blend(map, &format!("{prefix}trans")),
    }
}

fn parse_text(map: &HashMap<String, String>, prefix: &str) -> TextDef {
    TextDef {
        font: font_index(map, &format!("{prefix}font")),
        text: map
            .get(&format!("{prefix}text"))
            .cloned()
            .unwrap_or_default(),
        offset: map
            .get(&format!("{prefix}offset"))
            .map(|value| vec2(value))
            .unwrap_or_default(),
        scale: map
            .get(&format!("{prefix}scale"))
            .map(|value| vec2(value))
            .unwrap_or_else(|| Vec2::new(1.0, 1.0)),
        align: int(map, &format!("{prefix}align"), 0),
        color: color(map, &format!("{prefix}color")),
    }
}

fn parse_cursor(map: &HashMap<String, String>, prefix: &str) -> CursorDef {
    CursorDef {
        sprite: map
            .get(&format!("{prefix}spr"))
            .map(|value| int_pair(value)),
        anim: map
            .get(&format!("{prefix}anim"))
            .and_then(|value| value.trim().parse().ok()),
        offset: map
            .get(&format!("{prefix}offset"))
            .map(|value| vec2(value))
            .unwrap_or_default(),
        active: map
            .get(&format!("{prefix}active.spr"))
            .map(|value| int_pair(value)),
        done: map
            .get(&format!("{prefix}done.spr"))
            .map(|value| int_pair(value)),
        move_snd: map
            .get(&format!("{prefix}move.snd"))
            .map(|value| int_pair(value))
            .filter(|(group, _)| *group >= 0),
        done_snd: map
            .get(&format!("{prefix}done.snd"))
            .map(|value| int_pair(value))
            .filter(|(group, _)| *group >= 0),
    }
}

fn layer_from_map(map: &HashMap<String, String>) -> BgLayer {
    BgLayer {
        kind: map
            .get("type")
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_else(|| "normal".into()),
        sprite: map.get("spriteno").map(|value| int_pair(value)),
        action: map
            .get("actionno")
            .and_then(|value| value.trim().parse().ok()),
        start: map
            .get("start")
            .map(|value| vec2(value))
            .unwrap_or_default(),
        delta: map
            .get("delta")
            .map(|value| vec2(value))
            .unwrap_or_else(|| Vec2::new(1.0, 1.0)),
        trans: blend(map, "trans"),
        tile: map
            .get("tile")
            .map(|value| int_pair(value))
            .unwrap_or((0, 0)),
        tile_spacing: map
            .get("tilespacing")
            .map(|value| int_pair(value))
            .unwrap_or((0, 0)),
        layer_no: int(map, "layerno", 0),
        mask: flag(map, "mask", false),
        velocity: map
            .get("velocity")
            .map(|value| vec2(value))
            .unwrap_or_default(),
        sin_x: triple(map, "sin.x"),
        sin_y: triple(map, "sin.y"),
        cos_x: triple(map, "cos.x"),
        cos_y: triple(map, "cos.y"),
        position_link: flag(map, "positionlink", false),
        visible: flag(map, "visible", true),
        enabled: flag(map, "enabled", true),
        xscale: num(map, "xscale", 1.0),
        yscale: num(map, "yscale", 1.0),
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
            .map(|value| vec2(value))
            .unwrap_or_default(),
        id: int(map, "id", 0),
    }
}

/// Screen-background sections use plain keys; strip any inherited prefix.
fn normalize_bg_keys(map: &mut HashMap<String, String>) {
    for key in [
        "type", "spriteno", "actionno", "start", "delta", "trans", "tile",
    ] {
        if let Some(value) = map.remove(&format!("bg {key}")) {
            map.insert(key.to_string(), value);
        }
    }
}

fn collect_fonts(fonts: &mut BTreeMap<u32, FontDecl>, text: &TextFile, dir: &Path) {
    let Some(section) = text.find_header("files") else {
        return;
    };
    for entry in &section.entries {
        let key = entry.key.trim().to_ascii_lowercase();
        let Some(number) = key.strip_prefix("font") else {
            continue;
        };
        let Ok(index) = number.parse::<u32>() else {
            continue;
        };
        fonts.insert(index, font_decl(&entry.value, dir));
    }
}

fn collect_fonts_for_path(fonts: &mut BTreeMap<u32, FontDecl>, path: &Path, fallback: &Path) {
    let Ok(text) = TextFile::load(path) else {
        return;
    };
    for section in &text.sections {
        if section.kind() != "files" {
            continue;
        }
        for entry in &section.entries {
            let key = entry.key.trim().to_ascii_lowercase();
            let Some(number) = key.strip_prefix("font") else {
                continue;
            };
            let Ok(index) = number.parse::<u32>() else {
                continue;
            };
            let dir = path.parent().unwrap_or(fallback);
            fonts
                .entry(index)
                .or_insert_with(|| font_decl(&entry.value, dir));
        }
    }
}

fn font_decl(value: &str, dir: &Path) -> FontDecl {
    let parts = split_params(value);
    let path = parts
        .first()
        .map(|value| value.trim().trim_matches('"'))
        .filter(|value| !value.is_empty())
        .and_then(|value| resolve_asset(dir, value));
    let bank = parts
        .get(1)
        .and_then(|value| value.trim().parse::<i32>().ok())
        .unwrap_or(0);
    let mut tint = [255u8; 3];
    for (slot, part) in tint.iter_mut().zip(parts.iter().skip(2)) {
        *slot = part.trim().parse::<u8>().unwrap_or(255);
    }
    FontDecl { path, bank, tint }
}

fn resolve_asset(dir: &Path, value: &str) -> Option<PathBuf> {
    let value = value.trim().trim_matches('"');
    if value.is_empty() {
        return None;
    }
    let direct = dir.join(value);
    if direct.is_file() {
        return Some(direct);
    }
    let data = dir.join("data").join(value);
    if data.is_file() {
        return Some(data);
    }
    None
}

fn overlay(base: &HashMap<String, String>, prefix: &str) -> HashMap<String, String> {
    let mut map = base.clone();
    for (key, value) in base {
        if let Some(rest) = key.strip_prefix(prefix) {
            map.insert(rest.to_string(), value.clone());
        }
    }
    map
}

fn params_of(section: &Section) -> Vec<(String, String)> {
    section
        .entries
        .iter()
        .map(|entry| {
            (
                entry.key.trim().to_ascii_lowercase(),
                entry.value.trim().to_string(),
            )
        })
        .collect()
}

fn num(map: &HashMap<String, String>, key: &str, default: f32) -> f32 {
    map.get(key)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn int(map: &HashMap<String, String>, key: &str, default: i32) -> i32 {
    map.get(key)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn flag(map: &HashMap<String, String>, key: &str, default: bool) -> bool {
    map.get(key)
        .map(|value| value.trim() != "0")
        .unwrap_or(default)
}

fn vec2(value: &str) -> Vec2 {
    let (x, y) = parse_pair(value);
    Vec2::new(x, y)
}

fn int_pair(value: &str) -> (i32, i32) {
    let (x, y) = parse_pair(value);
    (x as i32, y as i32)
}

fn triple(map: &HashMap<String, String>, key: &str) -> [f32; 3] {
    let Some(value) = map.get(key) else {
        return [0.0; 3];
    };
    let parts: Vec<f32> = value
        .split(',')
        .map(|part| part.trim().parse().unwrap_or(0.0))
        .collect();
    [
        *parts.first().unwrap_or(&0.0),
        *parts.get(1).unwrap_or(&0.0),
        *parts.get(2).unwrap_or(&0.0),
    ]
}

fn quad(value: &str) -> [f32; 4] {
    let parts: Vec<f32> = value
        .split(',')
        .map(|part| part.trim().parse().unwrap_or(0.0))
        .collect();
    [
        *parts.first().unwrap_or(&0.0),
        *parts.get(1).unwrap_or(&0.0),
        *parts.get(2).unwrap_or(&0.0),
        *parts.get(3).unwrap_or(&0.0),
    ]
}

fn font_index(map: &HashMap<String, String>, key: &str) -> u32 {
    map.get(key)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

fn color(map: &HashMap<String, String>, key: &str) -> [u8; 4] {
    let Some(value) = map.get(key) else {
        return [255, 255, 255, 255];
    };
    let parts: Vec<u8> = value
        .split(',')
        .map(|part| part.trim().parse().unwrap_or(255))
        .collect();
    [
        *parts.first().unwrap_or(&255),
        *parts.get(1).unwrap_or(&255),
        *parts.get(2).unwrap_or(&255),
        *parts.get(3).unwrap_or(&255),
    ]
}

fn rgb(value: &str) -> [u8; 3] {
    let parts: Vec<u8> = value
        .split(',')
        .map(|part| part.trim().parse().unwrap_or(0))
        .collect();
    [
        *parts.first().unwrap_or(&0),
        *parts.get(1).unwrap_or(&0),
        *parts.get(2).unwrap_or(&0),
    ]
}

fn blend(map: &HashMap<String, String>, key: &str) -> BlendMode {
    match map
        .get(key)
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("add") | Some("add1") => BlendMode::Add,
        Some("sub") | Some("sub1") => BlendMode::Sub,
        _ => BlendMode::Normal,
    }
}

fn key_list(map: &HashMap<String, String>, key: &str) -> Vec<String> {
    map.get(key)
        .map(|value| {
            value
                .split(',')
                .map(|part| part.trim().to_ascii_lowercase())
                .filter(|part| !part.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn log_warn(message: &str) {
    eprintln!("manato screenpack: {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fight_def_bars_and_round() {
        let text = TextFile::parse_str(
            "[LifeBar]\npos = 10, 12\nrange.x = 100\n\
             bg0.spr = 0, 0\nfront.spr = 1, 0\ntop.spr = 2, 0\n\
             p2.bg0.spr = 3, 0\nmid.delay = 20\nmid.steps = 4\n\
             value.font = 1\nvalue.text = %d\n\
             [Round]\nstart.waittime = 60\nround.time = 30\n\
             round1.text = Round 1\nround1.font = 2\nfight.text = Fight!\n\
             [Time]\npos = 160, 10\ncounter.font = 3\n",
        );
        let base_life = text.find_header("lifebar").map(section_map).unwrap();
        let life = parse_bar(&overlay(&base_life, "p2."), true);
        assert_eq!(life.pos, Vec2::new(10.0, 12.0));
        assert_eq!(life.range, Vec2::new(100.0, 100.0));
        assert_eq!(life.bg0.sprite, Some((3, 0)));
        assert_eq!(life.mid_delay, 20);
        assert_eq!(life.mid_steps, 4.0);
        assert_eq!(life.value.font, 1);
        let round = parse_round(&text.find_header("round").map(section_map).unwrap());
        assert_eq!(round.start_waittime, 60);
        assert_eq!(round.element("round1").unwrap().text, "Round 1");
        assert_eq!(round.element("fight").unwrap().text, "Fight!");
    }

    #[test]
    fn roster_parses_lines_and_random() {
        let roster = Roster::parse_str(
            "[Characters]\nchars/kaen/kaen.def, stages/a.def\n\
             randomselect\n\n; comment\nchars/nival/nival.def\n",
        );
        assert_eq!(roster.entries.len(), 3);
        assert_eq!(roster.entries[0].def, "chars/kaen/kaen.def");
        assert_eq!(roster.entries[0].stage.as_deref(), Some("stages/a.def"));
        assert!(roster.entries[1].random);
        assert_eq!(roster.entries[2].def, "chars/nival/nival.def");
        assert_eq!(roster.entries[2].stage, None);
    }

    #[test]
    fn title_menu_items_keep_order() {
        let text = TextFile::parse_str(
            "[Title Info]\nmenu.pos = 160, 200\nmenu.item.spacing = 0, 10\n\
             menu.itemname.arcade = Arcade\nmenu.itemname.versus = Versus\n\
             menu.itemname.quit = Quit\nmenu.p1.cursor.spr = 9, 0\n",
        );
        let title = TitleDef::from_text(&text);
        let keys: Vec<&str> = title.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["arcade", "versus", "quit"]);
        assert_eq!(title.item_spacing, Vec2::new(0.0, 10.0));
        assert_eq!(title.cursor_p1.sprite, Some((9, 0)));
    }

    #[test]
    fn select_info_parses_grid_and_players() {
        let text = TextFile::parse_str(
            "[Select Info]\nrows = 3\ncolumns = 6\nwrapping = 1\npos = 20, 40\n\
             cell.size = 30, 30\ncell.spacing = 4\n\
             p1.cursor.startcell = 1, 2\np2.cursor.startcell = 0, 0\n\
             done.key = start, x\ncancel.key = esc\n",
        );
        let select = SelectDef::from_text(&text);
        assert_eq!(select.rows, 3);
        assert_eq!(select.columns, 6);
        assert!(select.wrapping);
        assert_eq!(select.cell_size, Vec2::new(30.0, 30.0));
        assert_eq!(select.players[0].cursor_start, (1, 2));
        assert_eq!(select.done_keys, ["start", "x"]);
        assert_eq!(select.cancel_keys, ["esc"]);
    }
}
