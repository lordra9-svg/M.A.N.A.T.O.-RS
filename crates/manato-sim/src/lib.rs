//! Deterministic match simulation for M.A.N.A.T.O.-RS.
//!
//! The whole simulation is a single `Clone`-able [`SimState`]. That is the unit
//! of rollback: GGRS snapshots it before every frame and restores it when a
//! remote input arrives late. Static content (sprites, sounds, parsed scripts)
//! lives in [`Assets`] behind an `Arc`, so cloning a state is cheap.

#![allow(clippy::too_many_arguments)]

pub mod ai;
pub mod collision;
pub mod controller;
pub mod expr;
pub mod fighter;
pub mod hitdef;
pub mod projectile;
pub mod trigger;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use manato_core::{clamp, PlayerInput, Rng, StateHasher, Vec2, GAME_HEIGHT};
use manato_formats::{Character, EffectsBank, SoundBank, Stage};

use fighter::{Fighter, StateType};
use hitdef::HitState;

/// How many rounds a player must win to take the match.
pub const DEFAULT_ROUNDS_TO_WIN: u32 = 2;
/// Default round length in seconds.
pub const DEFAULT_ROUND_TIME: i32 = 99;
/// Maximum power gauge value.
pub const MAX_POWER: f32 = 3000.0;

#[derive(Debug, Clone)]
pub struct MatchConfig {
    pub mugen_root: PathBuf,
    pub p1: PathBuf,
    pub p2: PathBuf,
    pub stage: PathBuf,
    /// Optional screenpack `system.def`. When absent, the engine falls back to
    /// the built-in HUD (`docs/spec/render.md` §13).
    pub screenpack: Option<PathBuf>,
    pub rounds_to_win: u32,
    pub round_time: i32,
}

impl MatchConfig {
    /// Defaults for the project's test content. Paths are resolved against
    /// `mugen_root`; no content is bundled with the engine.
    pub fn defaults(mugen_root: impl Into<PathBuf>) -> Self {
        let root = mugen_root.into();
        Self {
            p1: PathBuf::from("content/chars/kaen/kaen.def"),
            p2: PathBuf::from("content/chars/nival/nival.def"),
            stage: PathBuf::from("content/stages/training/stage1.def"),
            mugen_root: root,
            screenpack: None,
            rounds_to_win: DEFAULT_ROUNDS_TO_WIN,
            round_time: DEFAULT_ROUND_TIME,
        }
    }

    fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.mugen_root.join(path)
        }
    }

    /// Directories searched for shared files such as `common1.cns`,
    /// `fightfx.sff` and `fightfx.air`.
    pub fn data_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        let stock = self.mugen_root.join("mugen-1.1b1");
        dirs.push(stock.join("data"));
        dirs.push(stock.clone());
        dirs.push(stock.join("chars"));
        // Bundled original effects, common sounds, and characters.
        dirs.push(self.mugen_root.join("content"));
        // Community characters sometimes ship their own common1.cns.
        for def in [self.resolve(&self.p1), self.resolve(&self.p2)] {
            if let Some(dir) = def.parent() {
                dirs.push(dir.to_path_buf());
            }
        }
        if let Some(dir) = self.resolve(&self.stage).parent() {
            dirs.push(dir.to_path_buf());
        }
        dirs.push(self.mugen_root.join("content/chars/ryu"));
        dirs
    }
}

/// Immutable content shared by every snapshot.
#[derive(Debug)]
pub struct Assets {
    pub characters: Vec<Character>,
    pub stage: Stage,
    pub effects: EffectsBank,
    pub fight_sounds: SoundBank,
    pub data_dirs: Vec<PathBuf>,
    /// Parsed screenpack, when one is configured.
    pub screenpack: Option<manato_formats::Screenpack>,
}

impl Assets {
    pub fn load(config: &MatchConfig) -> Result<Self> {
        let data_dirs = config.data_dirs();
        let p1 = config.resolve(&config.p1);
        let p2 = config.resolve(&config.p2);
        let stage_path = config.resolve(&config.stage);

        let mut characters = Vec::new();
        for path in [&p1, &p2] {
            let character = Character::load(path, &data_dirs)
                .with_context(|| format!("failed to load character {}", path.display()))?;
            characters.push(character);
        }

        let stage = Stage::load(&stage_path, &data_dirs)
            .with_context(|| format!("failed to load stage {}", stage_path.display()))?;
        let effects = EffectsBank::load(&data_dirs);

        let mut fight_sounds = SoundBank::default();
        for dir in &data_dirs {
            let candidate = dir.join("fight.snd");
            if candidate.exists() {
                match SoundBank::load(&candidate) {
                    Ok(bank) => {
                        fight_sounds = bank;
                        break;
                    }
                    Err(e) => log::warn!("failed to load {}: {e}", candidate.display()),
                }
            }
        }

        let screenpack = match &config.screenpack {
            Some(path) => {
                let resolved = config.resolve(path);
                match manato_formats::Screenpack::load(&resolved) {
                    Ok(screenpack) => Some(screenpack),
                    Err(error) => {
                        log::warn!(
                            "failed to load screenpack {}: {error:#}",
                            resolved.display()
                        );
                        None
                    }
                }
            }
            None => None,
        };

        Ok(Self {
            characters,
            stage,
            effects,
            fight_sounds,
            data_dirs,
            screenpack,
        })
    }

    pub fn character(&self, index: usize) -> &Character {
        &self.characters[index]
    }
}

#[derive(Debug, Clone)]
pub struct MatchOptions {
    pub rounds_to_win: u32,
    pub round_time: i32,
    /// Difficulty for CPU-controlled slots, 0 disables AI.
    pub ai_level: i32,
    /// Slots whose AI is enabled. `AILevel` reports 0 for every other slot.
    pub ai_sides: [bool; 2],
}

impl Default for MatchOptions {
    fn default() -> Self {
        Self {
            rounds_to_win: DEFAULT_ROUNDS_TO_WIN,
            round_time: DEFAULT_ROUND_TIME,
            ai_level: 4,
            ai_sides: [true, true],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundPhase {
    Intro,
    Fight,
    Ko,
    Over,
}

#[derive(Debug, Clone)]
pub struct RoundState {
    pub phase: RoundPhase,
    pub number: u32,
    /// Round length in seconds.
    pub round_time: i32,
    /// Remaining round time in ticks.
    pub timer: i32,
    pub phase_timer: i32,
    pub wins: [u32; 2],
    /// Consecutive match sweeps (`ConsecutiveWins`), per side.
    pub consecutive_wins: [i32; 2],
    pub winner: Option<usize>,
    /// True when the round ended on the timer rather than a KO.
    pub time_over: bool,
    pub display: String,
    pub rounds_to_win: u32,
    /// Remaining KO slow-motion ticks (`slow.time`).
    pub slow_timer: i32,
    /// Fractional update accumulator for the reduced KO cadence.
    pub slow_accum: f32,
    /// Latched when `NoKOSlow` is asserted during the KO phase.
    pub slow_disabled: bool,
}

impl RoundState {
    fn new(rounds_to_win: u32, round_time: i32) -> Self {
        Self {
            phase: RoundPhase::Intro,
            number: 1,
            round_time: round_time.max(1),
            timer: round_time.max(1) * manato_core::TICKS_PER_SECOND as i32,
            phase_timer: 40,
            wins: [0, 0],
            consecutive_wins: [0, 0],
            winner: None,
            time_over: false,
            display: "Round 1".into(),
            rounds_to_win,
            slow_timer: 0,
            slow_accum: 0.0,
            slow_disabled: false,
        }
    }

    pub fn seconds(&self) -> i32 {
        ((self.timer + 59) / 60).max(0)
    }

    pub fn roundstate_code(&self) -> i32 {
        match self.phase {
            RoundPhase::Intro => 0,
            RoundPhase::Fight => 2,
            RoundPhase::Ko => 3,
            RoundPhase::Over => 4,
        }
    }
}

/// Camera mode selected by the Ikemen-only `Camera` controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraView {
    #[default]
    Fighting,
    Follow,
    Free,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraState {
    pub x: f32,
    pub y: f32,
    pub shake: Vec2,
    /// Automatic tracking mode (`Camera` controller).
    pub view: CameraView,
    /// Fighter followed in `Follow` mode.
    pub follow: Option<usize>,
    /// Explicit position target in `Free` mode.
    pub free_pos: Vec2,
    /// Current zoom scale (`camerazoom`).
    pub scale: f32,
    /// Scale the automatic tracker is heading toward.
    pub target_scale: f32,
    /// Ticks remaining before the camera may zoom back in.
    pub zoom_delay: i32,
    /// Active `Zoom` controller override.
    pub zoom_override: Option<ZoomOverride>,
}

/// A `Zoom` state-controller override (`docs/spec/render.md` §11.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomOverride {
    pub target_pos: Vec2,
    pub target_scale: f32,
    pub lag: i32,
    pub end_lag: i32,
    pub time: i32,
    pub elapsed: i32,
    /// Countdown after `time` expires, from `endlag`.
    pub hold: i32,
    pub camera_bound: bool,
    pub stage_bound: bool,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            shake: Vec2::ZERO,
            view: CameraView::Fighting,
            follow: None,
            free_pos: Vec2::ZERO,
            scale: 1.0,
            target_scale: 1.0,
            zoom_delay: 0,
            zoom_override: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectSource {
    FightFx,
    Character(usize),
    Stage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectBlend {
    Normal,
    Add,
    Sub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionMode {
    Orthographic,
    Perspective,
    Perspective2,
}

impl ProjectionMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "perspective" => Self::Perspective,
            "perspective2" => Self::Perspective2,
            _ => Self::Orthographic,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Effect {
    pub source: EffectSource,
    pub anim: i32,
    pub pos: Vec2,
    pub vel: Vec2,
    pub accel: Vec2,
    pub scale: Vec2,
    pub facing: i32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub blend: EffectBlend,
    /// Opacity from `trans = addalpha`/`alpha`; 1.0 = fully opaque.
    pub alpha: f32,
    pub sprite_priority: i32,
    pub time: i32,
    pub duration: i32,
    pub remove_on_anim_end: bool,
    pub id: i32,
    pub owner: usize,
    pub behind: bool,
    /// Remaining bind ticks; `-1` binds forever.
    pub bind_time: i32,
    /// Anchor entity while bound.
    pub bind_target: Option<usize>,
    /// Mirrored offset from the anchor applied while bound.
    pub bind_offset: Vec2,
    /// Whether the position is recomputed from the anchor this tick.
    pub bound: bool,
    pub remove_on_get_hit: bool,
    pub remove_on_change_state: bool,
    pub ignore_hit_pause: bool,
    pub super_move_time: i32,
    pub pause_move_time: i32,
    pub shadow: bool,
    pub own_pal: bool,
    pub transparent: bool,
    pub angle: f32,
    pub x_angle: f32,
    pub y_angle: f32,
    pub x_shear: f32,
    pub projection: ProjectionMode,
    pub focal_length: f32,
    /// `x1, y1, x2, y2` screen-space clip rect.
    pub window: Option<[f32; 4]>,
    /// `space = screen` bypasses the camera.
    pub screen_space: bool,
    pub anim_elem: i32,
    pub anim_freeze: bool,
}

impl Effect {
    pub fn is_expired(&self) -> bool {
        (self.remove_on_anim_end || self.duration >= 0) && self.time >= self.duration
    }

    pub fn wants_camera(&self) -> bool {
        !self.screen_space
    }
}

impl Default for Effect {
    fn default() -> Self {
        Self {
            source: EffectSource::FightFx,
            anim: 0,
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            accel: Vec2::ZERO,
            scale: Vec2::new(1.0, 1.0),
            facing: 1,
            flip_h: false,
            flip_v: false,
            blend: EffectBlend::Normal,
            alpha: 1.0,
            sprite_priority: 0,
            time: 0,
            duration: 0,
            remove_on_anim_end: true,
            id: 0,
            owner: 0,
            behind: false,
            bind_time: 0,
            bind_target: None,
            bind_offset: Vec2::ZERO,
            bound: false,
            remove_on_get_hit: false,
            remove_on_change_state: true,
            ignore_hit_pause: true,
            super_move_time: 0,
            pause_move_time: 0,
            shadow: false,
            own_pal: false,
            transparent: false,
            angle: 0.0,
            x_angle: 0.0,
            y_angle: 0.0,
            x_shear: 0.0,
            projection: ProjectionMode::Orthographic,
            focal_length: 2048.0,
            window: None,
            screen_space: false,
            anim_elem: 1,
            anim_freeze: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundSource {
    Character(usize),
    FightFx,
    Common,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundEvent {
    pub source: SoundSource,
    pub group: i32,
    pub index: i32,
    pub volume: f32,
    pub pan: f32,
    /// Managed channel, or -1 for a one-shot.
    pub channel: i32,
    pub loop_sound: bool,
    pub priority: i32,
    pub low_priority: bool,
}

/// A non-playback sound operation emitted by a controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SoundCommand {
    Stop {
        source: SoundSource,
        channel: i32,
    },
    Pan {
        source: SoundSource,
        channel: i32,
        pan: Option<f32>,
        volume: Option<f32>,
    },
    Modify {
        source: SoundSource,
        channel: i32,
        group: i32,
        index: i32,
        volume: Option<f32>,
        pan: Option<f32>,
        loop_sound: Option<bool>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvColor {
    pub color: [f32; 4],
    pub time: i32,
    /// Draw the tint beneath fighters (`under = 1`) instead of over them.
    pub under: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvShake {
    pub time: i32,
    /// Time when the shake started, used by `decay`.
    pub initial_time: i32,
    pub freq: f32,
    pub ampl: f32,
    pub phase: f32,
    /// Amplitude multiplier applied once per full cycle.
    pub mul: f32,
    /// Initial shake direction in degrees.
    pub dir: f32,
    /// Degrees added to the direction per tick.
    pub diradd: f32,
    /// Amplitude decay exponent over the remaining duration.
    pub decay: f32,
}

impl Default for EnvShake {
    fn default() -> Self {
        Self {
            time: 0,
            initial_time: 0,
            freq: 60.0,
            ampl: -4.0,
            phase: 0.0,
            mul: 1.0,
            dir: 0.0,
            diradd: 0.0,
            decay: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StageLayerState {
    pub offset: Vec2,
    pub velocity: Vec2,
    pub sin_x: [f32; 3],
    pub sin_y: [f32; 3],
    pub cos_x: [f32; 3],
    pub cos_y: [f32; 3],
    pub action: Option<i32>,
    pub visible: bool,
    pub enabled: bool,
    /// Runtime `ModifyStageBG` overrides; `None` uses the layer definition.
    pub start: Option<Vec2>,
    pub delta: Option<Vec2>,
    pub scale: Option<Vec2>,
}

impl StageLayerState {
    pub fn from_layer(layer: &manato_formats::stage::BgLayer) -> Self {
        Self {
            offset: Vec2::ZERO,
            velocity: layer.velocity,
            sin_x: layer.sin_x,
            sin_y: layer.sin_y,
            cos_x: layer.cos_x,
            cos_y: layer.cos_y,
            action: layer.action,
            visible: layer.visible,
            enabled: layer.enabled,
            start: None,
            delta: None,
            scale: None,
        }
    }
}

/// Ticks the KO display window lasts before the next round (or match end).
pub const KO_PHASE_TICKS: i32 = 120;

/// Ticks a combo counter stays visible after the last hit.
pub const COMBO_DISPLAY_TICKS: i32 = 60;

/// Per-side combo counter state (`docs/spec/render.md` §6.8).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ComboState {
    /// Current consecutive hits taken by the opposing side.
    pub hits: i32,
    /// Hit count last shown by the combo counter; survives the combo ending
    /// until `timer` expires.
    pub shown_hits: i32,
    /// Damage dealt by the current combo.
    pub damage: i32,
    /// Remaining display ticks; visible while this is positive.
    pub timer: i32,
}

/// A transient fight-screen banner (`docs/spec/render.md` §6.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionMessage {
    pub text: String,
    /// Remaining display ticks.
    pub time: i32,
    /// Initial display ticks, used for slide animation.
    pub total: i32,
}

/// One recorded round win, rendered by the fight screen (`§6.7`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinRecord {
    /// Win on time rather than by KO.
    pub time_over: bool,
    pub perfect: bool,
    pub clutch: bool,
}

/// Fraction of max life at or below which a win counts as a clutch win.
pub const CLUTCH_THRESHOLD: f32 = 0.10;

/// Delayed ("mid") bar fill state (`docs/spec/render.md` §6.1–§6.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarFill {
    /// Delayed life ratio in `0..=1`.
    pub mid_life: f32,
    /// Delayed power ratio in `0..=1`.
    pub mid_power: f32,
    /// Delayed guard ratio in `0..=1`.
    pub mid_guard: f32,
    /// Remaining delay ticks before the mid fill starts draining.
    pub delay: i32,
}

impl Default for BarFill {
    fn default() -> Self {
        Self {
            mid_life: 1.0,
            mid_power: 0.0,
            mid_guard: 1.0,
            delay: 0,
        }
    }
}

/// Global palette effect used by `AllPalFX` and `BgPalFX`. `mul` is
/// normalized (1.0 = identity), `add` stays in 0–255 units, and `alpha` is
/// 0–1. `time = 0` means inactive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlobalPalFx {
    pub add: [f32; 3],
    pub mul: [f32; 3],
    pub alpha: f32,
    pub time: i32,
}

impl Default for GlobalPalFx {
    fn default() -> Self {
        Self {
            add: [0.0; 3],
            mul: [1.0; 3],
            alpha: 1.0,
            time: 0,
        }
    }
}

impl GlobalPalFx {
    pub fn active(&self) -> bool {
        self.time > 0
    }

    /// Ticks the remaining duration, resetting to identity on expiry.
    pub fn tick(&mut self) {
        if self.time > 0 {
            self.time -= 1;
            if self.time <= 0 {
                *self = Self::default();
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SimState {
    pub assets: Arc<Assets>,
    /// Live camera definition; `ModifyStageVar` may change it during a match.
    pub stage_camera: manato_formats::stage::CameraDef,
    /// Live playable bounds; `ModifyStageVar` may change them.
    pub stage_left_bound: f32,
    pub stage_right_bound: f32,
    pub fighters: Vec<Fighter>,
    pub frame: u64,
    /// Ticks since the match started (`FightTime`); not reset between rounds.
    pub match_time: i32,
    /// Team side that landed the first unguarded hit of the round
    /// (`FirstAttack`); `None` until it happens and reset each round.
    pub first_attack: Option<usize>,
    /// Most recently assigned fighter index (`LastPlayerID`).
    pub last_player_id: i32,
    pub round: RoundState,
    pub camera: CameraState,
    pub effects: Vec<Effect>,
    pub projectiles: Vec<crate::projectile::Projectile>,
    pub sounds: Vec<SoundEvent>,
    pub sound_commands: Vec<SoundCommand>,
    pub bg_time: f32,
    pub env_shake: EnvShake,
    pub env_color: Option<EnvColor>,
    /// Global palette effect applied to every character/effect/projectile
    /// sprite (`AllPalFX`).
    pub all_pal: GlobalPalFx,
    /// Palette effect applied to stage background layers (`BgPalFX`).
    pub bg_pal: GlobalPalFx,
    pub stage_layers: Vec<StageLayerState>,
    pub options: MatchOptions,
    pub rng: Rng,
    pub super_pause: i32,
    pub super_pause_move_time: i32,
    /// Fighter allowed to act during a `Pause` controller's movetime.
    pub pause_mover: Option<usize>,
    pub pause_movetime: i32,
    pub hit_activation: u32,
    pub finished: bool,
    /// Per-side combo counters (`§6.8`).
    pub combo: [ComboState; 2],
    /// Wins recorded this match, in order, per side (`§6.7`).
    pub win_history: [Vec<WinRecord>; 2],
    /// Delayed life/power bar fill per side (`§6.1`, `§6.2`).
    pub bars: [BarFill; 2],
    /// `[Action]` banners per team (`§6.10`).
    pub actions: [Vec<ActionMessage>; 2],
    /// `Score` for the current round, per team side.
    pub score: [f32; 2],
    /// Score banked from completed rounds, per team side.
    pub score_rounds: [f32; 2],
    /// Live stage BG controllers; `ModifyBGCtrl` edits this copy.
    pub bg_ctrls: Vec<manato_formats::BgCtrl>,
}

impl SimState {
    pub fn load(config: &MatchConfig) -> Result<Self> {
        let assets = Arc::new(Assets::load(config)?);
        Ok(Self::new(
            assets,
            MatchOptions {
                rounds_to_win: config.rounds_to_win,
                round_time: config.round_time,
                ai_level: 0,
                ai_sides: [false, false],
            },
        ))
    }

    pub fn new(assets: Arc<Assets>, options: MatchOptions) -> Self {
        let (start0, start1) = (assets.stage.p1_start, assets.stage.p2_start);
        let (face0, face1) = (assets.stage.p1_facing, assets.stage.p2_facing);
        let cam_start = assets.stage.camera.start;
        let start_zoom = assets.stage.camera.start_zoom;
        let mut fighters = Vec::new();
        for slot in 0..2 {
            let start = if slot == 0 { start0 } else { start1 };
            let facing = if slot == 0 { face0 } else { face1 };
            let mut fighter = Fighter::new(
                &assets.characters[slot],
                slot,
                slot,
                slot,
                start,
                if facing == 0 { 1 } else { facing },
            );
            fighter.ai_level = if options.ai_sides[slot] {
                options.ai_level.max(0)
            } else {
                0
            };
            fighters.push(fighter);
        }
        let stage_layers = assets
            .stage
            .layers
            .iter()
            .map(StageLayerState::from_layer)
            .collect();
        let bg_ctrls = assets.stage.bg_ctrls.clone();
        let stage_camera = assets.stage.camera.clone();
        let stage_left_bound = assets.stage.left_bound;
        let stage_right_bound = assets.stage.right_bound;
        let mut state = Self {
            assets,
            stage_camera,
            stage_left_bound,
            stage_right_bound,
            fighters,
            frame: 0,
            match_time: 0,
            first_attack: None,
            last_player_id: 1,
            round: RoundState::new(options.rounds_to_win, options.round_time),
            camera: CameraState {
                x: cam_start.x,
                y: cam_start.y,
                scale: start_zoom,
                target_scale: start_zoom,
                ..CameraState::default()
            },
            effects: Vec::new(),
            projectiles: Vec::new(),
            sounds: Vec::new(),
            sound_commands: Vec::new(),
            bg_time: 0.0,
            env_shake: EnvShake::default(),
            env_color: None,
            all_pal: GlobalPalFx::default(),
            bg_pal: GlobalPalFx::default(),
            stage_layers,
            options,
            rng: Rng::new(0x4D41_4E41_544F_5253),
            super_pause: 0,
            super_pause_move_time: 0,
            pause_mover: None,
            pause_movetime: 0,
            hit_activation: 1,
            finished: false,
            combo: [ComboState::default(); 2],
            win_history: [Vec::new(), Vec::new()],
            bars: [BarFill::default(); 2],
            actions: [Vec::new(), Vec::new()],
            score: [0.0; 2],
            score_rounds: [0.0; 2],
            bg_ctrls,
        };
        state.start_round();
        state
    }

    pub fn character(&self, who: usize) -> &Character {
        let char_id = self.fighters[who]
            .state_char_id
            .unwrap_or(self.fighters[who].char_id);
        &self.assets.characters[char_id]
    }

    /// Victory quote shown after the match: the winner's `VictoryQuote`
    /// selection when valid, otherwise the first available quote.
    pub fn victory_quote_text(&self) -> Option<&str> {
        if self.round.phase != RoundPhase::Over {
            return None;
        }
        let side = self.round.winner?;
        let fighter = self.fighters.get(side)?;
        let character = &self.assets.characters[fighter.char_id];
        if character.quotes.is_empty() {
            return None;
        }
        character
            .quotes
            .iter()
            .find(|(index, _)| *index == fighter.win_quote)
            .or_else(|| character.quotes.first())
            .map(|(_, quote)| quote.as_str())
    }

    pub fn other(&self, who: usize) -> usize {
        self.opponent_of(who)
    }

    pub fn opponent_of(&self, who: usize) -> usize {
        // Helpers (including nested helpers owned by another helper) inherit
        // their owner's side, so the side is the only stable way to find the
        // opposing player. Using the owner index directly underflows once an
        // owner is itself a helper.
        let side = self
            .fighters
            .get(who)
            .map(|fighter| fighter.side)
            .unwrap_or(who);
        1 - side.min(1)
    }

    pub fn root_of(&self, who: usize) -> usize {
        self.fighters
            .get(who)
            .and_then(|fighter| fighter.root)
            .unwrap_or(who)
    }

    pub fn parent_of(&self, who: usize) -> usize {
        self.fighters
            .get(who)
            .and_then(|fighter| fighter.parent)
            .unwrap_or(who)
    }

    pub fn is_helper(&self, who: usize) -> bool {
        self.fighters
            .get(who)
            .map(|fighter| fighter.owner.is_some())
            .unwrap_or(false)
    }

    pub fn helper_count(&self) -> usize {
        self.fighters.len().saturating_sub(2)
    }

    pub fn helper_by_id(&self, owner: usize, id: i32) -> Option<usize> {
        self.fighters
            .iter()
            .enumerate()
            .skip(2)
            .find(|(_, fighter)| fighter.owner == Some(owner) && fighter.helper_id == id)
            .map(|(index, _)| index)
    }

    /// Advances the match by exactly one 60 Hz tick.
    pub fn tick(&mut self, inputs: [PlayerInput; 2]) {
        self.frame = self.frame.wrapping_add(1);
        for fighter in &mut self.fighters {
            if fighter.hit.hit_shake_timer > 0 {
                fighter.hit.hit_shake_timer -= 1;
            }
            for timer in [
                &mut fighter.proj_contact_time,
                &mut fighter.proj_hit_time,
                &mut fighter.proj_guarded_time,
            ] {
                if *timer >= 0 {
                    *timer = timer.saturating_add(1);
                }
            }
        }
        // Control is locked during the round intro and KO slowdown, as in
        // M.U.G.E.N. This also keeps the CPU from attacking before "Fight!".
        let inputs = if matches!(self.round.phase, RoundPhase::Intro | RoundPhase::Ko) {
            [PlayerInput::default(); 2]
        } else {
            inputs
        };
        let super_paused = self.super_pause > 0;
        for (i, input) in inputs.iter().enumerate() {
            let hit_paused = self.fighters[i].hit.hit_pause > 0;
            let can_move = self.pause_mover == Some(i) && self.pause_movetime > 0;
            let pause_bits = u8::from(hit_paused) | (u8::from(super_paused && !can_move) << 1);
            self.fighters[i].append_input(*input, pause_bits);
        }
        self.sounds.clear();
        self.sound_commands.clear();

        if self.super_pause > 0 {
            self.super_pause -= 1;
            if self.super_pause_move_time > 0 {
                self.super_pause_move_time -= 1;
            }
            if self.pause_movetime > 0 {
                self.pause_movetime -= 1;
                if let Some(mover) = self.pause_mover {
                    if mover < self.fighters.len() && self.fighters[mover].active {
                        self.update_fighter(mover);
                    }
                }
            }
            self.update_round();
            self.update_effects();
            self.update_fight_screen();
            self.clear_state_changed_tick();
            self.match_time = self.match_time.saturating_add(1);
            return;
        }

        if self.round.phase == RoundPhase::Over {
            self.update_effects();
            self.update_fight_screen();
            self.clear_state_changed_tick();
            self.match_time = self.match_time.saturating_add(1);
            return;
        }

        let mut run_gameplay = true;
        if self.round.phase == RoundPhase::Ko && self.round.slow_timer > 0 {
            if self
                .fighters
                .iter()
                .any(|fighter| fighter.assert_flags & fighter::ASSERT_NOKOSLOW != 0)
            {
                self.round.slow_disabled = true;
            }
            if !self.round.slow_disabled {
                self.round.slow_accum += self.ko_slow_factor();
                if self.round.slow_accum >= 1.0 {
                    self.round.slow_accum -= 1.0;
                } else {
                    run_gameplay = false;
                }
            }
            self.round.slow_timer -= 1;
        }

        if run_gameplay {
            self.update_gameplay();
            self.update_target_binds();
            self.update_entity_binds();
        }
        self.update_round();
        self.update_camera();
        self.update_effects();
        self.update_fight_screen();
        self.bg_time += 1.0;
        self.update_stage_layers();
        self.clear_state_changed_tick();
        self.match_time = self.match_time.saturating_add(1);
    }

    fn clear_state_changed_tick(&mut self) {
        for fighter in &mut self.fighters {
            fighter.state_changed_tick = false;
        }
    }

    fn update_gameplay(&mut self) {
        for who in 0..self.fighters.len() {
            if self.fighters[who].hit.hit_pause > 0 {
                self.fighters[who].hit.hit_pause -= 1;
                if self.fighters[who].hit.hit_pause_time > 0 {
                    self.fighters[who].hit.hit_pause_time -= 1;
                }
                self.fighters[who].hit.pause_gap += 1;
                self.fighters[who].state_changed = false;
                self.run_paused_controllers(who);
                continue;
            }
            if self.fighters[who].hit.super_pause > 0 {
                self.fighters[who].hit.super_pause -= 1;
                self.fighters[who].hit.pause_gap += 1;
                self.run_paused_controllers(who);
                continue;
            }
            self.update_fighter(who);
            self.fighters[who].hit.pause_gap = 0;
        }

        collision::resolve(self);
        self.separate_pushboxes();
        self.fighters.retain(|fighter| {
            fighter.owner.is_none() || (!fighter.helper_destroy && fighter.helper_remove_time != 0)
        });
        {
            let camera_x = self.camera.x;
            let scale = self.camera.scale;
            let stage = &self.assets.stage;
            let (stage_left, stage_right) = (stage.camera.bound_left, stage.camera.bound_right);
            for projectile in &mut self.projectiles {
                projectile.advance();
                projectile.apply_bounds(camera_x, scale, stage_left, stage_right);
            }
        }
        collision::resolve_projectiles(self);
        {
            let characters = &self.assets.characters;
            let fighters = &self.fighters;
            for projectile in &mut self.projectiles {
                if projectile.status == crate::projectile::ProjectileStatus::Active {
                    let char_id = fighters
                        .get(projectile.owner)
                        .map(|fighter| fighter.char_id)
                        .unwrap_or(0);
                    if let Some(character) = characters.get(char_id) {
                        projectile.begin_removal(character);
                    }
                }
            }
        }
        self.projectiles.retain(|p| !p.is_expired());
    }

    /// Initial logic-speed fraction during KO slow-motion, ramping to 1.0 as
    /// `slow.fadetime` elapses (`docs/spec/rounds.md` §10).
    pub fn ko_slow_factor(&self) -> f32 {
        let default = manato_formats::screenpack::RoundDef::default();
        let def = self
            .assets
            .screenpack
            .as_ref()
            .map(|screenpack| &screenpack.fight.round)
            .unwrap_or(&default);
        let fade = def.slow_fade_time.max(1);
        let speed = def.slow_speed.clamp(0.01, 1.0);
        if self.round.slow_timer < fade {
            let ratio = (fade - self.round.slow_timer) as f32 / fade as f32;
            (speed + (1.0 - speed) * ratio).clamp(0.01, 1.0)
        } else {
            speed
        }
    }

    fn begin_ko_slowdown(&mut self) {
        let default = manato_formats::screenpack::RoundDef::default();
        let def = self
            .assets
            .screenpack
            .as_ref()
            .map(|screenpack| &screenpack.fight.round)
            .unwrap_or(&default);
        self.round.slow_timer = def.slow_time.max(0);
        // Run one full KO frame first so `NoKOSlow` can be asserted by the
        // win pose, then latch it.
        self.round.slow_accum = 0.75;
        self.round.slow_disabled = false;
    }

    fn update_stage_layers(&mut self) {
        let bg_time = self.bg_time as i32;
        let round_no = self.round.number;
        let round_state = self.round.roundstate_code();
        update_bg_states(
            &self.bg_ctrls,
            &mut self.stage_layers,
            bg_time,
            round_no,
            round_state,
        );
    }

    fn update_fighter(&mut self, who: usize) {
        {
            let fighter = &mut self.fighters[who];
            // `Time` stays at 0 on the tick a state was entered; the state's
            // controllers already ran on that tick.
            if fighter.state_changed {
                fighter.state_changed = false;
            } else {
                fighter.state_time = fighter.state_time.saturating_add(1);
            }
            fighter.hit.hit_over_timer = (fighter.hit.hit_over_timer - 1).max(0);
            // GetHitVar(hittime) counts down only once the hit shake is over
            // and stops at -1 (`HitOver`).
            if fighter.hit.hit_shake_timer <= 0 && fighter.hit.vars.hittime >= 0 {
                fighter.hit.vars.hittime = fighter.hit.vars.hittime.saturating_sub(1);
            }
            if fighter.state_no == 5110 && fighter.hit.vars.down_recovertime > 0 {
                let fast = fighter.hit.vars.down_recover
                    && fighter.held_input().buttons & manato_core::Buttons::ALL != 0;
                let step = if fast { 2 } else { 1 };
                fighter.hit.vars.down_recovertime =
                    (fighter.hit.vars.down_recovertime - step).max(0);
            }
            if fighter.guard_crush_timer > 0 {
                fighter.guard_crush_timer -= 1;
                if fighter.guard_crush_timer == 0 {
                    fighter.guard_points = fighter.guard_max;
                    fighter.guard_break = false;
                }
            }
            if fighter.guard_cooldown > 0 {
                fighter.guard_cooldown -= 1;
            }
            if fighter.guard_crush_timer == 0
                && fighter.guard_cooldown == 0
                && fighter.guard_points < fighter.guard_max
            {
                fighter.guard_points = (fighter.guard_points + 2).min(fighter.guard_max);
            }
            if fighter.dizzy_timer > 0 {
                fighter.dizzy_timer -= 1;
                if fighter.dizzy_timer == 0 {
                    fighter.dizzy_points = fighter.dizzy_max;
                    fighter.dizzy_flag = false;
                    fighter.ctrl = true;
                } else {
                    fighter.ctrl = false;
                }
            }
            // Leaving hitstun ends the combo the attacker was building
            // (`docs/spec/render.md` §6.8).
            if fighter.move_type != fighter::MoveType::Hit {
                fighter.received_hits = 0;
                fighter.received_damage = 0;
                fighter.fall_defence_mul = 1.0;
                fighter.hit.vars.fallcount = 0;
                fighter.hit.vars.fall = false;
                fighter.hit.down_bounce = false;
            }
            fighter.pos_freeze = (fighter.pos_freeze - 1).max(0);
            fighter.make_dust_spacing = fighter.make_dust_spacing.saturating_add(1);
            fighter.hit.invuln_timer = (fighter.hit.invuln_timer - 1).max(0);
            fighter.hit_overridden = false;
            for slot in &mut fighter.hit_overrides {
                if let Some(override_) = slot.as_mut() {
                    if override_.time > 0 {
                        override_.time -= 1;
                        if override_.time <= 0 {
                            *slot = None;
                        }
                    }
                }
            }
            if fighter.reversal_time > 0 {
                fighter.reversal_time -= 1;
                if fighter.reversal_time <= 0 {
                    fighter.reversal = None;
                }
            }
            if fighter.helper_remove_time > 0 {
                fighter.helper_remove_time -= 1;
                if fighter.helper_remove_time == 0 {
                    fighter.helper_destroy = true;
                }
            }
            // AssertSpecial is a per-tick assertion; states must re-assert it.
            fighter.assert_flags = 0;
            // `Width`/`Height` are per-tick overrides: Ikemen resets them
            // before the state controllers run each tick.
            fighter.width_override = None;
            fighter.height_player = Vec2::ZERO;
            fighter.shadow_intensity = None;
            fighter.shadow_color = None;
            fighter.shadow_offset = Vec2::ZERO;
            fighter.shadow_xscale = None;
            fighter.shadow_yscale = None;
            fighter.reflection_intensity = None;
            fighter.reflection_color = None;
            fighter.reflection_offset = Vec2::ZERO;
            fighter.reflection_xscale = None;
            fighter.reflection_yscale = None;
            fighter.angle_draw = false;
            fighter.angle_draw_value = None;
            fighter.angle_draw_scale = None;
            fighter.transform_x_shear = 0.0;
            fighter.transform_focal_length = 2048.0;
            fighter.transform_projection = crate::ProjectionMode::Orthographic;
            fighter.clsn_transforms[0].clear();
            fighter.clsn_transforms[1].clear();
            for overrides in &mut fighter.clsn_overrides {
                overrides.clear();
            }
            fighter.input_shift.clear();
            if let Some((_, t)) = fighter.hit.not_hit_by.as_mut() {
                *t -= 1;
                if *t <= 0 {
                    fighter.hit.not_hit_by = None;
                }
            }
            if let Some((_, t)) = fighter.hit.hit_by.as_mut() {
                *t -= 1;
                if *t <= 0 {
                    fighter.hit.hit_by = None;
                }
            }
        }

        // Liedown recovery: once the timer reaches zero, alive fighters get up
        // on the next state tick if the common getup state exists.
        if self.fighters[who].state_no == 5110
            && self.fighters[who].state_time >= 1
            && self.fighters[who].hit.vars.down_recovertime <= 0
            && self.fighters[who].alive()
        {
            let char_id = self.fighters[who].char_id;
            let char = &self.assets.characters[char_id];
            if char.state(5120).is_some() {
                self.fighters[who].enter_state(5120, char, false);
            }
        }

        // Special states run every tick, in order -3, -2, -1. Helpers need
        // `keyctrl` (or `helpertype = player`) to run them.
        if self.fighters[who].active {
            for state in [-3, -2, -1] {
                if self.special_state_enabled(who, state) {
                    self.run_state_controllers(who, state);
                }
            }
        }
        // Built-in default movement (stand/walk/crouch/jump).
        self.builtin_movement(who);

        // Physics and position integration.
        self.integrate_physics(who);

        // Current state controllers. A transition continues from the start of
        // the new state within the same tick, exactly like M.U.G.E.N.
        for _ in 0..8 {
            let state_no = self.fighters[who].state_no;
            if state_no < 0 || !self.run_state_controllers(who, state_no) {
                break;
            }
        }
        if self.fighters[who].face_p2_pending {
            let other = self.other(who);
            let facing = if self.fighters[other].pos.x >= self.fighters[who].pos.x {
                1
            } else {
                -1
            };
            self.fighters[who].facing = facing;
            self.fighters[who].face_p2_pending = false;
        }
        self.auto_turn(who);
        self.fighters[who].state_changed = false;

        // Advance animation last so triggers observe this tick's frame.
        let assets = self.assets.clone();
        let own_char = &assets.characters[self.fighters[who].char_id];
        let anim_char_id = self.fighters[who]
            .anim_char_id
            .unwrap_or(self.fighters[who].char_id);
        let anim_char = &assets.characters[anim_char_id];
        self.fighters[who].advance_anim(anim_char);
        self.fighters[who].refresh_widths(own_char);
        self.update_after_image(who);
        self.update_pal_fx(who);
    }

    /// Stage `autoturn`: idle fighters face each other unless
    /// `AssertSpecial(noautoturn)` is active (`docs/spec/stage.md` §6).
    fn auto_turn(&mut self, who: usize) {
        if !self.assets.stage.autoturn {
            return;
        }
        let fighter = &self.fighters[who];
        if !fighter.ctrl
            || fighter.move_type != fighter::MoveType::Idle
            || fighter.state_type == fighter::StateType::Air
            || fighter.target_bind.is_some()
            || fighter.assert_flags & fighter::ASSERT_NOAUTOTURN != 0
        {
            return;
        }
        let other = self.other(who);
        let dx = self.fighters[other].pos.x - fighter.pos.x;
        if dx.abs() < 0.001 {
            return;
        }
        let facing = if dx > 0.0 { 1 } else { -1 };
        self.fighters[who].facing = facing;
    }

    /// Character whose animation bank drives `anim_no` (`ChangeAnim2`).
    pub fn anim_character(&self, who: usize) -> &manato_formats::Character {
        let fighter = &self.fighters[who];
        &self.assets.characters[fighter.anim_char_id.unwrap_or(fighter.char_id)]
    }

    /// Whether a helper may run the given negative state (`Helper` keyctrl /
    /// helpertype semantics from `docs/spec/helpers.md`).
    fn special_state_enabled(&self, who: usize, state: i32) -> bool {
        let fighter = &self.fighters[who];
        if fighter.owner.is_none() {
            // While using another fighter's state data (throw/custom state),
            // M.U.G.E.N skips -3 and Ikemen also skips -1; -2 still runs.
            return match state {
                -3 | -1 => fighter.state_char_id.is_none(),
                _ => true,
            };
        }
        match state {
            -3 => fighter.helper_type_player || fighter.keyctrl & 4 != 0,
            -2 => fighter.helper_type_player || fighter.keyctrl & 2 != 0,
            -1 => fighter.keyctrl & 1 != 0,
            _ => true,
        }
    }

    fn update_after_image(&mut self, who: usize) {
        let fighter = &mut self.fighters[who];
        if let Some(state) = fighter.after_image.as_mut() {
            // Negative timers run forever; zero disables the emitter.
            if state.time != 0 {
                if state.time > 0 {
                    state.time -= 1;
                }
                state.tick += 1;
                if state.tick >= state.interval {
                    state.tick = 0;
                    fighter.after_images.push(crate::fighter::AfterImageFrame {
                        pos: fighter.pos,
                        facing: fighter.facing,
                        anim_no: fighter.anim_no,
                        anim_frame: fighter.anim_frame,
                        life: state.life,
                        alpha: state.alpha,
                        add: state.add,
                    });
                }
            }
            // A zero timer keeps the (inactive) state so a later
            // AfterImageTime can revive it.
        }
        for frame in &mut fighter.after_images {
            frame.life -= 1;
        }
        fighter.after_images.retain(|frame| frame.life > 0);
    }

    fn update_pal_fx(&mut self, who: usize) {
        let fighter = &mut self.fighters[who];
        if fighter.pal_time > 0 {
            fighter.pal_time -= 1;
            if fighter.pal_time == 0 {
                fighter.pal_add = [0.0, 0.0, 0.0];
                fighter.pal_base_add = [0.0, 0.0, 0.0];
                fighter.pal_sinadd = [0.0, 0.0, 0.0];
                fighter.pal_sin_period = 0;
                fighter.pal_cycle = 0;
                fighter.pal_mul = [1.0, 1.0, 1.0];
                fighter.pal_alpha = 1.0;
            } else {
                if fighter.pal_sin_period > 0 {
                    fighter.pal_cycle += 1;
                    let phase = std::f32::consts::TAU * fighter.pal_cycle as f32
                        / fighter.pal_sin_period as f32;
                    for axis in 0..3 {
                        fighter.pal_add[axis] =
                            fighter.pal_base_add[axis] + phase.sin() * fighter.pal_sinadd[axis];
                    }
                }
                if fighter.pal_sinmul_period > 0 {
                    fighter.pal_sinmul_cycle += 1;
                    let phase = std::f32::consts::TAU * fighter.pal_sinmul_cycle as f32
                        / fighter.pal_sinmul_period as f32;
                    for axis in 0..3 {
                        fighter.pal_mul[axis] =
                            fighter.pal_base_mul[axis] + phase.sin() * fighter.pal_sinmul[axis];
                    }
                }
            }
        }
    }

    /// Runs only `ignorehitpause = 1` controllers while the fighter is paused
    /// (`docs/spec/cns-cmd.md` §4.5). Special states run before the current
    /// state, and a state change stops the remaining controllers.
    fn run_paused_controllers(&mut self, who: usize) {
        if !self.fighters[who].active {
            return;
        }
        for state_no in [-3, -2, -1] {
            if self.special_state_enabled(who, state_no) {
                self.run_filtered_state_controllers(who, state_no, true);
            }
        }
        for _ in 0..8 {
            let state_no = self.fighters[who].state_no;
            if state_no < 0 || !self.run_filtered_state_controllers(who, state_no, true) {
                break;
            }
        }
    }

    fn run_state_controllers(&mut self, who: usize, state_no: i32) -> bool {
        self.run_filtered_state_controllers(who, state_no, false)
    }

    fn run_filtered_state_controllers(
        &mut self,
        who: usize,
        state_no: i32,
        paused_only: bool,
    ) -> bool {
        let assets = self.assets.clone();
        // Negative states always come from the fighter's own character, even
        // while a positive custom state borrows another bank.
        let char_id = if state_no < 0 {
            self.fighters[who].char_id
        } else {
            self.fighters[who]
                .state_char_id
                .unwrap_or(self.fighters[who].char_id)
        };
        let char = &assets.characters[char_id];
        let Some(state) = char.state(state_no) else {
            return false;
        };

        let mut changed = false;
        for (index, control) in state.controllers.iter().enumerate() {
            if paused_only && !control.ignore_hit_pause() {
                continue;
            }
            let met = trigger::triggers_met(self, who, control);
            let persistent = control.persistent();
            let slot = if state_no == -3 {
                0
            } else if state_no == -2 {
                1
            } else if state_no == -1 {
                2
            } else {
                3
            };
            let should_run = update_exec_counter(self, who, slot, index, met, persistent);
            if !met || !should_run {
                continue;
            }
            let before = self.fighters[who].state_no;
            // Ikemen's `continue = 1` lets the remaining controllers of the
            // current block run after a ChangeState/SelfState.
            let continue_after_change = matches!(
                control.kind.trim().to_ascii_lowercase().as_str(),
                "changestate" | "selfstate"
            ) && control
                .get("continue")
                .map(|raw| trigger::num(self, who, raw) != 0.0)
                .unwrap_or(false);
            controller::execute(self, who, control, &assets);
            if self.fighters[who].state_no != before {
                changed = true;
                if !continue_after_change {
                    break;
                }
            }
        }
        changed
    }

    /// The engine's built-in transitions between the default movement states
    /// (0 stand, 10/11 crouch, 20 walk, 40 jump start, 50/52 jump).
    fn builtin_movement(&mut self, who: usize) {
        let fighter = &self.fighters[who];
        if !fighter.active
            || !fighter.ctrl
            || fighter.hit_pause() > 0
            || fighter.assert_flags & fighter::ASSERT_NOHARDCODEDKEYS != 0
        {
            return;
        }
        if fighter.pos_freeze > 0 {
            return;
        }
        let assets = self.assets.clone();
        let char = &assets.characters[fighter.char_id];
        let input = fighter.held_input();
        let dir = manato_core::Dir8::relative(input, fighter.facing);
        let forward = dir.is_forward();
        let back = dir.is_back();
        let down = dir.is_down();
        let up = dir.is_up();
        let buttons = input.buttons;
        let stateno = fighter.state_no;

        let has = |n: i32| char.state(n).is_some();
        let set = |sim: &mut SimState, state: i32, vx: Option<f32>, vy: Option<f32>| {
            sim.fighters[who].enter_state(state, char, false);
            if let Some(vx) = vx {
                sim.fighters[who].vel.x = vx * sim.fighters[who].facing as f32;
            }
            if let Some(vy) = vy {
                sim.fighters[who].vel.y = vy;
            }
        };

        // Walk: return to stand when no direction is held.
        if stateno == 20 {
            if !forward && !back && has(0) {
                set(self, 0, None, None);
            }
            return;
        }
        // Crouch: stand up when down is released.
        if (stateno == 10 || stateno == 11) && !down {
            if has(0) {
                set(self, 0, None, None);
            }
            return;
        }
        if stateno != 0 {
            return;
        }

        // A button press on the ground falls back to the stock normals when the
        // character script did not bind it (KFM/standard characters do).
        let attacks: [(u8, i32); 6] = [
            (1 << 0, 200), // x
            (1 << 1, 400), // y
            (1 << 2, 400), // z
            (1 << 3, 600), // a
            (1 << 4, 600), // b
            (1 << 5, 600), // c
        ];
        for (bit, state) in attacks {
            if buttons & bit != 0 && has(state) {
                set(self, state, None, None);
                return;
            }
        }

        if up && has(40) {
            let jump_y = char.constants.get_or("movement.jump.neu.y", -8.4);
            set(self, 40, None, Some(jump_y));
            return;
        }
        if down && has(10) {
            set(self, 10, None, None);
            return;
        }
        if (forward || back)
            && has(20)
            && self.fighters[who].assert_flags & crate::fighter::ASSERT_NOWALK == 0
        {
            let speed = if forward {
                char.constants.get_or("movement.walk.fwd.x", 2.4)
            } else {
                char.constants.get_or("movement.walk.back.x", -2.2)
            };
            set(self, 20, Some(speed), None);
        }
    }

    fn integrate_physics(&mut self, who: usize) {
        let assets = self.assets.clone();
        let char = &assets.characters[self.fighters[who].char_id];
        let ground_level = char
            .constants
            .get_or("movement.air.gethit.groundlevel", 25.0);
        let fighter = &mut self.fighters[who];
        if fighter.pos_freeze > 0 {
            return;
        }

        let fall_state = matches!(fighter.state_no, 5030 | 5040 | 5050);
        match fighter.physics {
            fighter::Physics::Air => {
                // An unguarded hit uses the HitDef's `yaccel` (reported by
                // GetHitVar(yaccel)); ordinary air movement and air guard keep
                // the character's movement.yaccel.
                let hit_air =
                    fighter.move_type == fighter::MoveType::Hit && !fighter.hit.vars.guarded;
                let use_hit_yaccel = hit_air || (fall_state && fighter.hit.vars.y_accel > 0.0);
                let accel = if use_hit_yaccel {
                    fighter.hit.vars.y_accel
                } else if let Some(gravity) = fighter.gravity {
                    gravity
                } else {
                    char.constants.get_or("movement.yaccel", 0.44)
                };
                fighter.vel.y += accel;
            }
            fighter::Physics::Stand => {
                let friction = char.constants.get_or("movement.stand.friction", 0.85);
                if fighter.vel.x.abs() > 0.1 {
                    fighter.vel.x *= friction;
                } else {
                    fighter.vel.x = 0.0;
                }
                fighter.vel.y = 0.0;
            }
            fighter::Physics::Crouch => {
                let friction = char.constants.get_or("movement.crouch.friction", 0.82);
                if fighter.vel.x.abs() > 0.1 {
                    fighter.vel.x *= friction;
                } else {
                    fighter.vel.x = 0.0;
                }
                fighter.vel.y = 0.0;
            }
            fighter::Physics::None => {}
        }

        fighter.pos.x += fighter.vel.x;
        fighter.pos.y += fighter.vel.y;

        // Landing for ordinary air physics. Any airborne state that reaches
        // the ground transitions to the jump-land state (52) when it exists.
        if fighter.physics == fighter::Physics::Air
            && fighter.state_no != 52
            && !matches!(fighter.state_no, 5030 | 5040 | 5050 | 5201 | 5210)
            && fighter.pos.y >= 0.0
            && fighter.vel.y >= 0.0
        {
            fighter.pos.y = 0.0;
            fighter.vel.y = 0.0;
            if char.state(52).is_some() {
                fighter.enter_state(52, char, false);
            }
        }

        // Hit-fall ground level (common1 states use `groundlevel`). Landing
        // applies `fall.damage` once (`docs/spec/combat.md` §6).
        if fall_state && fighter.vel.y > 0.0 && fighter.pos.y >= ground_level {
            fighter.pos.y = 0.0;
            fighter.vel.y = 0.0;
            if fighter.hit.vars.fall_damage > 0 {
                let damage = fighter.hit.vars.fall_damage;
                let next = fighter.life - damage;
                fighter.life = if fighter.hit.vars.fall_kill {
                    next.max(0)
                } else {
                    next.max(1)
                };
                fighter.hit.vars.fall_damage = 0;
            }
            if fighter.hit.vars.fall_envshake_time > 0 {
                let time = fighter.hit.vars.fall_envshake_time;
                let freq = fighter.hit.vars.fall_envshake_freq;
                let ampl = fighter.hit.vars.fall_envshake_ampl as f32;
                let mut phase = fighter.hit.vars.fall_envshake_phase;
                if phase.is_nan() {
                    phase = if freq >= 90.0 { 90.0 } else { 0.0 };
                }
                phase = phase.clamp(-180.0, 180.0);
                fighter.hit.vars.fall_envshake_time = 0;
                self.env_shake = crate::EnvShake {
                    time,
                    initial_time: time,
                    freq,
                    ampl,
                    phase,
                    mul: 1.0,
                    dir: 0.0,
                    diradd: 0.0,
                    decay: 0.0,
                };
            }
            if fighter.hit.down_bounce {
                fighter.hit.down_bounce = false;
                if let Some(y) = fighter.hit.vars.fall_y_vel.filter(|y| *y != 0.0) {
                    fighter.vel = Vec2::new(fighter.hit.vars.fall_x_vel.unwrap_or(0.0), y);
                }
            }
        }
    }

    fn separate_pushboxes(&mut self) {
        let stage_left = self.stage_left_bound;
        let stage_right = self.stage_right_bound;
        // Vertical stage extent from the camera's high/low bounds: the top is
        // negative, the floor is the lower value.
        let stage_top = self.stage_camera.bound_high;
        let stage_bottom = self.stage_camera.bound_low;
        for fighter in self.fighters.iter_mut().take(2) {
            if fighter.screen_bound.0 {
                let p = fighter.pos.x;
                fighter.pos.x = clamp(p, stage_left, stage_right);
            }
            if fighter.screen_bound.1 {
                let p = fighter.pos.y;
                fighter.pos.y = clamp(p, stage_top, stage_bottom);
            }
        }
        let (a, b) = (0usize, 1usize);
        if !self.fighters[a].player_push || !self.fighters[b].player_push {
            return;
        }
        let box_a = self.fighters[a].push_box();
        let box_b = self.fighters[b].push_box();
        if !box_a.intersects(box_b) {
            return;
        }
        let overlap = (box_a.x2.min(box_b.x2) - box_a.x1.max(box_b.x1)).max(0) as f32;
        if overlap <= 0.0 {
            return;
        }
        let dir = if self.fighters[a].pos.x <= self.fighters[b].pos.x {
            -1.0
        } else {
            1.0
        };
        self.fighters[a].pos.x += dir * overlap * 0.5;
        self.fighters[b].pos.x -= dir * overlap * 0.5;
    }

    fn update_target_binds(&mut self) {
        for who in 0..self.fighters.len() {
            let Some(mut bind) = self.fighters[who].target_bind else {
                continue;
            };
            let Some(target) = self.fighters[who].target else {
                self.fighters[who].target_bind = None;
                continue;
            };
            if target >= self.fighters.len() {
                self.fighters[who].target_bind = None;
                continue;
            }
            let attacker = self.fighters[who].pos;
            let facing = self.fighters[who].facing as f32;
            self.fighters[target].pos = Vec2::new(
                attacker.x + bind.offset.x * facing,
                attacker.y + bind.offset.y,
            );
            self.fighters[target].facing = -self.fighters[who].facing;
            if bind.time > 0 {
                bind.time -= 1;
                if bind.time == 0 {
                    self.fighters[who].target_bind = None;
                } else {
                    self.fighters[who].target_bind = Some(bind);
                }
            }
        }
    }

    /// Applies persistent `BindToRoot`/`BindToParent`/`BindToTarget`
    /// bindings, following the anchor and stepping the bind timer.
    fn update_entity_binds(&mut self) {
        if self.super_pause > 0 {
            return;
        }
        for who in 0..self.fighters.len() {
            let Some(anchor) = self.fighters[who].bind_anchor else {
                continue;
            };
            if anchor >= self.fighters.len() || anchor == who || !self.fighters[anchor].active {
                let fighter = &mut self.fighters[who];
                fighter.bind_anchor = None;
                fighter.bind_time = 0;
                fighter.bind_facing = 0;
                continue;
            }
            if self.fighters[who].hit.hit_pause > 0 {
                continue;
            }
            let anchor_facing = self.fighters[anchor].facing;
            let base = self.fighters[anchor].pos;
            let offset = self.fighters[who].bind_offset;
            let facing = self.fighters[who].bind_facing;
            let fighter = &mut self.fighters[who];
            fighter.pos = Vec2::new(base.x + offset.x * anchor_facing as f32, base.y + offset.y);
            if facing != 0 {
                fighter.facing = anchor_facing * facing.signum();
            }
            if fighter.bind_time > 0 {
                fighter.bind_time -= 1;
                if fighter.bind_time == 0 {
                    fighter.bind_anchor = None;
                    fighter.bind_facing = 0;
                }
            }
        }
    }

    fn update_round(&mut self) {
        match self.round.phase {
            RoundPhase::Intro => {
                self.round.phase_timer -= 1;
                if self.round.phase_timer <= 0 {
                    self.round.phase = RoundPhase::Fight;
                    self.round.display.clear();
                    self.emit_round_sound("fight");
                }
            }
            RoundPhase::Fight => {
                let timer_frozen = self.fighters.iter().any(|fighter| {
                    fighter.active && fighter.assert_flags & fighter::ASSERT_TIMERFREEZE != 0
                });
                if !timer_frozen {
                    self.round.timer -= 1;
                }
                let dead: Vec<usize> = (0..2)
                    .filter(|&i| {
                        self.fighters[i].life <= 0
                            && self.fighters[i].assert_flags & fighter::ASSERT_NOKO == 0
                    })
                    .collect();
                if !dead.is_empty() {
                    let winner = if dead.len() == 1 {
                        Some(1 - dead[0])
                    } else {
                        None
                    };
                    self.round.winner = winner;
                    self.round.time_over = false;
                    self.round.phase = RoundPhase::Ko;
                    self.round.phase_timer = KO_PHASE_TICKS;
                    self.begin_ko_slowdown();
                    self.round.display = match winner {
                        Some(w) => format!("{} Wins", self.assets.characters[w].display_name()),
                        None => "Draw Game".into(),
                    };
                    self.emit_round_sound(if winner.is_none() { "dko" } else { "ko" });
                    if let Some(w) = winner {
                        self.round.wins[w] += 1;
                        self.round.consecutive_wins[1 - w] = 0;
                        let perfect = self.record_win(w, false);
                        self.emit_round_sound(if perfect { "win2" } else { "win" });
                        self.fighters[w].power = (self.fighters[w].power + 200.0).min(MAX_POWER);
                        self.enter_win_pose(w);
                    }
                } else if !timer_frozen && self.round.timer <= 0 {
                    self.round.timer = 0;
                    let life0 = self.fighters[0].life.max(0);
                    let life1 = self.fighters[1].life.max(0);
                    let winner = if life0 == life1 {
                        None
                    } else if life0 > life1 {
                        Some(0)
                    } else {
                        Some(1)
                    };
                    self.round.winner = winner;
                    self.round.time_over = true;
                    self.round.phase = RoundPhase::Ko;
                    self.round.phase_timer = KO_PHASE_TICKS;
                    self.begin_ko_slowdown();
                    self.round.display = match winner {
                        Some(w) => format!(
                            "Time Over - {} Wins",
                            self.assets.characters[w].display_name()
                        ),
                        None => "Time Over - Draw".into(),
                    };
                    self.emit_round_sound(if winner.is_none() { "dko" } else { "to" });
                    if let Some(w) = winner {
                        self.round.wins[w] += 1;
                        self.round.consecutive_wins[1 - w] = 0;
                        let perfect = self.record_win(w, true);
                        self.emit_round_sound(if perfect { "win2" } else { "win" });
                    }
                }
            }
            RoundPhase::Ko => {
                if self.round_not_over() {
                    return;
                }
                self.round.phase_timer -= 1;
                if self.round.phase_timer <= 0 {
                    let round_cap = self
                        .round
                        .rounds_to_win
                        .saturating_mul(2)
                        .saturating_sub(1)
                        .max(1);
                    if self.round.wins[0] >= self.round.rounds_to_win
                        || self.round.wins[1] >= self.round.rounds_to_win
                        || self.round.number >= round_cap
                    {
                        let winner = if self.round.wins[0] > self.round.wins[1] {
                            Some(0)
                        } else if self.round.wins[1] > self.round.wins[0] {
                            Some(1)
                        } else {
                            None
                        };
                        if let Some(w) = winner {
                            if self.round.wins[1 - w] == 0 {
                                self.round.consecutive_wins[w] += 1;
                            }
                        }
                        self.round.winner = winner;
                        self.round.phase = RoundPhase::Over;
                        self.round.display = match winner {
                            Some(w) => format!(
                                "{} Wins the Match",
                                self.assets.characters[w].display_name()
                            ),
                            None => "Draw Game".into(),
                        };
                        self.finished = true;
                    } else {
                        self.round.number += 1;
                        self.start_round();
                    }
                }
            }
            RoundPhase::Over => {}
        }
    }

    /// Queues an `[Action]` banner for a team. At most four are visible; the
    /// oldest is dropped (`docs/spec/render.md` §6.10`).
    pub fn push_action(&mut self, side: usize, text: impl Into<String>, time: i32) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        let time = time.max(1);
        let messages = &mut self.actions[side % 2];
        while messages.len() >= 4 {
            messages.remove(0);
        }
        messages.push(ActionMessage {
            text,
            time,
            total: time,
        });
    }

    /// Updates presentation state that lives in the simulation: combo
    /// counters (`docs/spec/render.md` §6.8`) and delayed bar fills
    /// (`§6.1`, `§6.2`).
    fn update_fight_screen(&mut self) {
        self.update_bars();
        for messages in self.actions.iter_mut().take(2) {
            for message in messages.iter_mut() {
                message.time -= 1;
            }
            messages.retain(|message| message.time > 0);
        }

        for side in 0..2 {
            let defender = 1 - side;
            let (hits, damage) = self
                .fighters
                .get(defender)
                .map(|fighter| (fighter.received_hits, fighter.received_damage))
                .unwrap_or((0, 0));
            let state = &mut self.combo[side];
            if hits > state.hits {
                state.shown_hits = hits;
                state.timer = COMBO_DISPLAY_TICKS;
            }
            state.hits = hits;
            state.damage = damage;
            if state.timer > 0 {
                state.timer -= 1;
                if state.timer == 0 {
                    state.shown_hits = 0;
                }
            }
        }
    }

    /// Drains the delayed bar fills after their configured delay.
    fn update_bars(&mut self) {
        let fight = self
            .assets
            .screenpack
            .as_ref()
            .map(|screenpack| &screenpack.fight);
        for side in 0..2 {
            let life_max = self.fighters[side].life_max.max(1);
            let life_ratio = (self.fighters[side].life as f32 / life_max as f32).clamp(0.0, 1.0);
            let power_ratio = (self.fighters[side].power / MAX_POWER).clamp(0.0, 1.0);
            let guard_ratio = if self.fighters[side].guard_max > 0 {
                (self.fighters[side].guard_points as f32 / self.fighters[side].guard_max as f32)
                    .clamp(0.0, 1.0)
            } else {
                0.0
            };
            let life_def = fight.map(|fight| &fight.life[side]);
            let power_def = fight.map(|fight| &fight.power[side]);
            let guard_def = fight.map(|fight| &fight.guard[side]);
            let life_delay = life_def.map(|bar| bar.mid_delay).unwrap_or(0);
            let life_range = life_def.map(|bar| bar.range.x).unwrap_or(0.0);
            let power_range = power_def.map(|bar| bar.range.x).unwrap_or(0.0);
            let guard_range = guard_def.map(|bar| bar.range.x).unwrap_or(0.0);
            let life_drain = life_def
                .map(|bar| {
                    let span = if life_range > 0.0 { life_range } else { 100.0 };
                    (bar.mid_steps.max(0.0) * bar.mid_mult.max(0.0) / span).max(0.0)
                })
                .unwrap_or(0.0);
            let power_drain = power_def
                .map(|bar| {
                    let span = if power_range > 0.0 {
                        power_range
                    } else {
                        100.0
                    };
                    (bar.mid_steps.max(0.0) * bar.mid_mult.max(0.0) / span).max(0.0)
                })
                .unwrap_or(0.0);
            let guard_drain = guard_def
                .map(|bar| {
                    let span = if guard_range > 0.0 {
                        guard_range
                    } else {
                        100.0
                    };
                    (bar.mid_steps.max(0.0) * bar.mid_mult.max(0.0) / span).max(0.0)
                })
                .unwrap_or(0.0);

            let bar = &mut self.bars[side];
            // Healing snaps immediately; damage waits for the delay.
            if life_ratio >= bar.mid_life {
                bar.mid_life = life_ratio;
                bar.delay = life_delay.max(bar.delay);
            } else if bar.delay > 0 {
                bar.delay -= 1;
            } else {
                bar.mid_life = (bar.mid_life - life_drain).max(life_ratio);
            }
            if power_ratio >= bar.mid_power {
                bar.mid_power = power_ratio;
            } else if bar.delay > 0 {
                // Power shares the life delay window, matching the bar draw
                // order; it drains only after the same pause.
            } else {
                bar.mid_power = (bar.mid_power - power_drain).max(power_ratio);
            }
            if guard_ratio >= bar.mid_guard {
                bar.mid_guard = guard_ratio;
            } else if bar.delay > 0 {
                // Guard shares the same delayed-fill window.
            } else {
                bar.mid_guard = (bar.mid_guard - guard_drain).max(guard_ratio);
            }
        }
    }

    /// Records a round win and its perfect/clutch classification (`§6.7`).
    fn record_win(&mut self, winner: usize, time_over: bool) -> bool {
        let fighter = &self.fighters[winner];
        let perfect = fighter.life >= fighter.life_max;
        let clutch =
            !perfect && (fighter.life as f32) <= fighter.life_max as f32 * CLUTCH_THRESHOLD;
        self.win_history[winner].push(WinRecord {
            time_over,
            perfect,
            clutch,
        });
        perfect
    }

    /// Queues a `fight.def` round sound on the common channel (§6.9).
    fn emit_round_sound(&mut self, name: &str) {
        let round_end = matches!(name, "ko" | "dko" | "to" | "win" | "win2");
        if round_end && self.ko_sounds_suppressed() {
            return;
        }
        let Some(screenpack) = &self.assets.screenpack else {
            return;
        };
        let Some((group, index)) = screenpack
            .fight
            .round
            .elements
            .get(name)
            .and_then(|element| element.snd)
        else {
            return;
        };
        self.sounds.push(SoundEvent {
            source: SoundSource::Common,
            group,
            index,
            volume: 1.0,
            pan: 0.0,
            channel: -1,
            loop_sound: false,
            priority: 0,
            low_priority: false,
        });
    }

    fn round_not_over(&self) -> bool {
        self.fighters
            .iter()
            .any(|fighter| fighter.assert_flags & fighter::ASSERT_ROUNDNOTOVER != 0)
    }

    fn enter_win_pose(&mut self, winner: usize) {
        let char = &self.assets.characters[self.fighters[winner].char_id];
        if char.state(180).is_some() {
            self.fighters[winner].enter_state(180, char, false);
        }
    }

    /// Restarts the match from round 1, clearing wins, power, and match state.
    /// The host may call this after `finished` (`docs/spec/rounds.md` §6).
    pub fn restart_match(&mut self) {
        self.round.number = 1;
        self.round.wins = [0, 0];
        self.round.consecutive_wins = [0, 0];
        self.round.winner = None;
        self.round.time_over = false;
        self.win_history = [Vec::new(), Vec::new()];
        self.finished = false;
        self.match_time = 0;
        self.first_attack = None;
        self.last_player_id = 1;
        self.stage_camera = self.assets.stage.camera.clone();
        self.stage_left_bound = self.assets.stage.left_bound;
        self.stage_right_bound = self.assets.stage.right_bound;
        self.score = [0.0; 2];
        self.score_rounds = [0.0; 2];
        for fighter in &mut self.fighters {
            fighter.power = 0.0;
        }
        self.start_round();
    }

    fn start_round(&mut self) {
        if self.round.number > 1 {
            for side in 0..2 {
                self.score_rounds[side] += self.score[side];
                self.score[side] = 0.0;
            }
        }
        self.fighters.truncate(2);
        // Unique IDs are 1-based, so two players occupy IDs 1 and 2.
        self.last_player_id = self.fighters.len() as i32;
        let stage = &self.assets.stage;
        for slot in 0..2 {
            let char = &self.assets.characters[slot];
            let fighter = &mut self.fighters[slot];
            let power = fighter.power;
            let start = if slot == 0 {
                stage.p1_start
            } else {
                stage.p2_start
            };
            let facing = if slot == 0 {
                stage.p1_facing
            } else {
                stage.p2_facing
            };
            fighter.pos = start;
            fighter.vel = Vec2::ZERO;
            fighter.facing = if facing == 0 { 1 } else { facing };
            fighter.life = fighter.life_max;
            fighter.red_life = fighter.life_max;
            fighter.guard_points = fighter.guard_max;
            fighter.guard_crush_timer = 0;
            fighter.guard_break = false;
            fighter.dizzy_points = fighter.dizzy_max;
            fighter.dizzy_timer = 0;
            fighter.dizzy_flag = false;
            fighter.guard_cooldown = 0;
            fighter.power = power.min(MAX_POWER);
            fighter.state_no = 0;
            fighter.prev_state_no = 0;
            fighter.state_time = 0;
            fighter.anim_no = 0;
            fighter.prev_anim = 0;
            fighter.draw_angle = 0.0;
            fighter.angle_draw = false;
            fighter.angle_draw_value = None;
            fighter.angle_draw_scale = None;
            fighter.anim_clock = 0;
            fighter.anim_frame = 0;
            fighter.anim_time = 0;
            fighter.anim_looped = 0;
            fighter.ctrl = true;
            fighter.state_type = StateType::Stand;
            fighter.move_type = fighter::MoveType::Idle;
            fighter.prev_state_type = StateType::Stand;
            fighter.prev_move_type = fighter::MoveType::Idle;
            fighter.physics = fighter::Physics::Stand;
            fighter.hit = HitState::default();
            fighter.attack = fighter::AttackState::default();
            fighter.hitdef = None;
            fighter.ctrl_exec.clear();
            fighter.state_char_id = None;
            fighter.anim_char_id = None;
            fighter.extra_exec = [Vec::new(), Vec::new(), Vec::new()];
            fighter.pos_freeze = 0;
            fighter.assert_flags = 0;
            fighter.face_p2_pending = false;
            fighter.player_push = true;
            fighter.gravity = None;
            fighter.after_image = None;
            fighter.after_images.clear();
            fighter.pal_add = [0.0, 0.0, 0.0];
            fighter.pal_base_add = [0.0, 0.0, 0.0];
            fighter.pal_sinadd = [0.0, 0.0, 0.0];
            fighter.pal_sin_period = 0;
            fighter.pal_cycle = 0;
            fighter.pal_mul = [1.0, 1.0, 1.0];
            fighter.pal_base_mul = [1.0, 1.0, 1.0];
            fighter.pal_sinmul = [0.0, 0.0, 0.0];
            fighter.pal_sinmul_period = 0;
            fighter.pal_sinmul_cycle = 0;
            fighter.pal_alpha = 1.0;
            fighter.pal_time = 0;
            fighter.target = None;
            fighter.win_quote = -1;
            fighter.target_bind = None;
            fighter.bind_anchor = None;
            fighter.bind_time = 0;
            fighter.bind_offset = Vec2::ZERO;
            fighter.bind_facing = 0;
            fighter.received_hits = 0;
            fighter.received_damage = 0;
            fighter.proj_contact_time = -1;
            fighter.proj_hit_time = -1;
            fighter.proj_guarded_time = -1;
            fighter.state_changed = false;
            fighter.enter_state(0, char, false);
            fighter.ctrl = true;
        }
        self.round.phase = RoundPhase::Intro;
        self.round.phase_timer = 40;
        self.round.timer = self.round.round_time * 60;
        self.round.winner = None;
        self.round.time_over = false;
        self.round.display = format!("Round {}", self.round.number);
        self.first_attack = None;
        self.effects.clear();
        self.projectiles.clear();
        self.combo = [ComboState::default(); 2];
        self.actions = [Vec::new(), Vec::new()];
        self.round.slow_timer = 0;
        self.round.slow_accum = 0.0;
        self.round.slow_disabled = false;
        self.bars = [
            BarFill {
                mid_life: 1.0,
                mid_power: (self.fighters[0].power / MAX_POWER).clamp(0.0, 1.0),
                mid_guard: 1.0,
                delay: 0,
            },
            BarFill {
                mid_life: 1.0,
                mid_power: (self.fighters[1].power / MAX_POWER).clamp(0.0, 1.0),
                mid_guard: 1.0,
                delay: 0,
            },
        ];
        self.camera.view = CameraView::Fighting;
        self.camera.follow = None;
        self.camera.free_pos = Vec2::ZERO;
        self.camera.x = self.stage_camera.start.x;
        self.camera.y = self.stage_camera.start.y;
        self.camera.scale = self.stage_camera.start_zoom;
        self.camera.target_scale = self.stage_camera.start_zoom;
        self.camera.zoom_delay = 0;
        self.camera.zoom_override = None;
        self.pause_mover = None;
        self.pause_movetime = 0;
        self.env_shake = EnvShake::default();
        self.env_color = None;
        self.all_pal = GlobalPalFx::default();
        self.bg_pal = GlobalPalFx::default();
        if self.assets.stage.reset_bg {
            self.bg_time = 0.0;
            self.bg_ctrls = self.assets.stage.bg_ctrls.clone();
            self.stage_layers = self
                .assets
                .stage
                .layers
                .iter()
                .map(StageLayerState::from_layer)
                .collect();
        }
    }

    fn update_camera(&mut self) {
        let cam = self.stage_camera.clone();
        let cam = &cam;

        if let Some(mut zoom) = self.camera.zoom_override {
            let step = if zoom.time > 0 {
                1.0 / zoom.time as f32
            } else {
                1.0
            };
            let smoothing = if zoom.lag > 0 {
                (1.0 / (zoom.lag as f32 + 1.0)).max(step)
            } else {
                step
            };
            self.camera.x += (zoom.target_pos.x - self.camera.x) * smoothing;
            self.camera.y += (zoom.target_pos.y - self.camera.y) * smoothing;
            self.camera.scale += (zoom.target_scale - self.camera.scale) * smoothing;
            self.camera.scale = clamp(self.camera.scale, cam.zoom_out, cam.zoom_in);

            if zoom.stage_bound {
                let half_w = manato_core::GAME_WIDTH * 0.5 / self.camera.scale.max(0.0001);
                let lo = self.stage_left_bound + half_w;
                let hi = self.stage_right_bound - half_w;
                if lo <= hi {
                    self.camera.x = clamp(self.camera.x, lo, hi);
                }
            }
            if zoom.camera_bound {
                let half_w = manato_core::GAME_WIDTH * 0.5 / self.camera.scale.max(0.0001);
                let lo = cam.bound_left + half_w;
                let hi = cam.bound_right - half_w;
                if lo <= hi {
                    self.camera.x = clamp(self.camera.x, lo, hi);
                }
                self.camera.y = clamp(self.camera.y, cam.bound_high, cam.bound_low);
            }

            if zoom.hold > 0 {
                zoom.hold -= 1;
                if zoom.hold == 0 {
                    self.camera.zoom_override = None;
                } else {
                    self.camera.zoom_override = Some(zoom);
                }
            } else if zoom.time > 0 {
                zoom.elapsed += 1;
                if zoom.elapsed >= zoom.time {
                    zoom.hold = zoom.end_lag;
                    if zoom.hold == 0 {
                        self.camera.zoom_override = None;
                    } else {
                        self.camera.zoom_override = Some(zoom);
                    }
                } else {
                    self.camera.zoom_override = Some(zoom);
                }
            } else {
                // Instant override: applied once above.
                if zoom.end_lag > 0 {
                    zoom.hold = zoom.end_lag;
                    self.camera.zoom_override = Some(zoom);
                } else {
                    self.camera.zoom_override = None;
                }
            }
        } else if self.camera.view == CameraView::Free {
            let half_w = manato_core::GAME_WIDTH * 0.5 / self.camera.scale.max(0.0001);
            let lo = cam.bound_left + half_w;
            let hi = cam.bound_right - half_w;
            let target_x = if lo <= hi {
                clamp(self.camera.free_pos.x, lo, hi)
            } else {
                self.camera.free_pos.x
            };
            let target_y = clamp(self.camera.free_pos.y, cam.bound_high, cam.bound_low);
            self.camera.target_scale = self.camera.scale;
            self.camera.x += (target_x - self.camera.x) * 0.35;
            self.camera.y += (target_y - self.camera.y) * 0.35;
        } else {
            let follow = if self.camera.view == CameraView::Follow {
                self.camera.follow.filter(|index| {
                    self.fighters
                        .get(*index)
                        .is_some_and(|fighter| fighter.active)
                })
            } else {
                None
            };
            let (mid_x, highest, left, right, top, bottom) = if let Some(index) = follow {
                let fighter = &self.fighters[index];
                (
                    fighter.pos.x,
                    fighter.pos.y,
                    fighter.pos.x - fighter.width_back,
                    fighter.pos.x + fighter.width_front,
                    fighter.pos.y - fighter.height - fighter.height_player.x,
                    fighter.pos.y + fighter.height_player.y,
                )
            } else {
                let mid_x = (self.fighters[0].pos.x + self.fighters[1].pos.x) * 0.5;
                let highest = self.fighters[0].pos.y.min(self.fighters[1].pos.y);
                // Visible extents of the non-helper fighters, including their
                // body boxes. Helpers never drive the fighting camera.
                let mut left = f32::INFINITY;
                let mut right = f32::NEG_INFINITY;
                let mut top = f32::INFINITY;
                let mut bottom = f32::NEG_INFINITY;
                for fighter in &self.fighters {
                    if fighter.owner.is_some() {
                        continue;
                    }
                    left = left.min(fighter.pos.x - fighter.width_back);
                    right = right.max(fighter.pos.x + fighter.width_front);
                    top = top.min(fighter.pos.y - fighter.height - fighter.height_player.x);
                    bottom = bottom.max(fighter.pos.y + fighter.height_player.y);
                }
                (mid_x, highest, left, right, top, bottom)
            };
            let (left, right, top, bottom) = if left.is_finite() {
                (left, right, top, bottom)
            } else {
                (mid_x, mid_x, highest, highest)
            };
            let extent_w = (right - left + cam.tension).max(1.0);
            let extent_h = (bottom - top + cam.tension * 0.5).max(1.0);
            let target = (manato_core::GAME_WIDTH / extent_w)
                .min(manato_core::GAME_HEIGHT / extent_h)
                .clamp(cam.zoom_out, cam.zoom_in);
            self.camera.target_scale = target;

            if target < self.camera.scale {
                self.camera.scale += (target - self.camera.scale) * cam.zoom_out_speed;
                self.camera.zoom_delay = cam.zoom_in_delay;
            } else if target > self.camera.scale {
                if self.camera.zoom_delay > 0 {
                    self.camera.zoom_delay -= 1;
                } else {
                    self.camera.scale += (target - self.camera.scale) * cam.zoom_in_speed;
                }
            }
            self.camera.scale = clamp(self.camera.scale, cam.zoom_out, cam.zoom_in);

            // Keep the visible rect inside the camera bounds.
            let half_w = manato_core::GAME_WIDTH * 0.5 / self.camera.scale.max(0.0001);
            let lo = cam.bound_left + half_w;
            let hi = cam.bound_right - half_w;
            let target_x = if lo <= hi {
                clamp(mid_x, lo, hi)
            } else {
                mid_x
            };
            // `verticalfollow` scales how far the camera tracks height;
            // `floortension` raises the target by a dead-zone offset.
            let vertical_follow = cam.vertical_follow.clamp(0.0, 1.0);
            let target_y = clamp(
                (highest + cam.floor_tension) * vertical_follow,
                cam.bound_high,
                cam.bound_low,
            );
            // Smooth follow.
            self.camera.x += (target_x - self.camera.x) * 0.35;
            self.camera.y += (target_y - self.camera.y) * 0.35;
        }

        if self.env_shake.time > 0 {
            // Ikemen recomputes the offset from the elapsed time each frame
            // (`src/system.go` EnvShake.update), so `mul` applies once per
            // full `freq` cycle rather than every tick.
            let shake = &mut self.env_shake;
            let elapsed = (shake.initial_time - shake.time).max(0) as f32;
            let current_phase = shake.phase + shake.freq * elapsed;
            let current_dir = shake.dir + shake.diradd * elapsed;
            let mut factor = 1.0_f32;
            if shake.mul != 0.0 && shake.mul != 1.0 {
                let cycles = (shake.freq * elapsed / 360.0).floor();
                factor *= shake.mul.powf(cycles);
            }
            if shake.decay != 0.0 && shake.initial_time > 0 {
                let t = shake.time as f32 / shake.initial_time as f32;
                if t > 0.0 {
                    factor *= t.powf(shake.decay);
                }
            }
            let ampl = shake.ampl * factor;
            if ampl == 0.0 {
                self.camera.shake = Vec2::ZERO;
            } else {
                let phase_rad = current_phase.to_radians();
                let dir_rad = current_dir.to_radians();
                let offset = ampl * phase_rad.sin();
                self.camera.shake = Vec2::new(offset * (-dir_rad).sin(), offset * (-dir_rad).cos());
            }
            shake.time -= 1;
        } else {
            self.camera.shake = Vec2::ZERO;
        }
    }

    fn update_effects(&mut self) {
        self.all_pal.tick();
        self.bg_pal.tick();
        if let Some(color) = self.env_color.as_mut() {
            if color.time > 0 {
                color.time -= 1;
                if color.time <= 0 {
                    self.env_color = None;
                }
            }
        }

        let super_paused = self.super_pause > 0;
        let Self {
            fighters, effects, ..
        } = self;
        effects.retain_mut(|effect| {
            let owner = fighters.get(effect.owner);
            let owner_hit_pause = owner.map(|f| f.hit.hit_pause > 0).unwrap_or(false);
            let owner_hit = owner
                .map(|f| f.move_type == crate::fighter::MoveType::Hit)
                .unwrap_or(false);
            let state_changed = owner.map(|f| f.state_changed_tick).unwrap_or(false);

            if effect.remove_on_get_hit && owner_hit {
                return false;
            }
            if effect.remove_on_change_state && state_changed {
                return false;
            }

            let mut advance = true;
            if super_paused && !effect.ignore_hit_pause {
                if effect.super_move_time > 0 {
                    effect.super_move_time -= 1;
                } else {
                    advance = false;
                }
            }
            if owner_hit_pause && !effect.ignore_hit_pause {
                if effect.pause_move_time > 0 {
                    effect.pause_move_time -= 1;
                } else {
                    advance = false;
                }
            }

            if advance {
                effect.time += 1;
                if effect.bound {
                    if let Some(target) = effect.bind_target.and_then(|i| fighters.get(i)) {
                        effect.pos = Vec2::new(
                            target.pos.x + effect.bind_offset.x,
                            target.pos.y + effect.bind_offset.y,
                        );
                    }
                    if effect.bind_time > 0 {
                        effect.bind_time -= 1;
                        if effect.bind_time == 0 {
                            effect.bound = false;
                        }
                    }
                } else {
                    effect.pos += effect.vel;
                    effect.vel += effect.accel;
                }
            }

            !effect.is_expired()
        });
    }

    /// Absolute ground position used by rendering: the stage's ground line.
    pub fn ground_screen_y(&self) -> f32 {
        self.assets.stage.zoffset
    }

    /// Maps a world position to screen space (`docs/spec/render.md` §11.5).
    ///
    /// The X axis always anchors at the viewport centre. The Y axis anchors
    /// at the stage ground when `zoomanchor = 1`, and otherwise at the
    /// viewport centre. Both forms reduce to the unzoomed mapping at
    /// `scale = 1`, so enabling zoom never jumps the scene.
    /// True when any fighter asserts `NoKOSnd` (suppress round-end sounds).
    pub fn ko_sounds_suppressed(&self) -> bool {
        self.fighters
            .iter()
            .any(|fighter| fighter.assert_flags & fighter::ASSERT_NOKOSND != 0)
    }

    /// True when any fighter asserts `NoBG` (hide background layers).
    pub fn background_hidden(&self) -> bool {
        self.fighters
            .iter()
            .any(|fighter| fighter.assert_flags & fighter::ASSERT_NOBG != 0)
    }

    /// True when any fighter asserts `NoFG` (hide foreground layers).
    pub fn foreground_hidden(&self) -> bool {
        self.fighters
            .iter()
            .any(|fighter| fighter.assert_flags & fighter::ASSERT_NOFG != 0)
    }

    /// True when any fighter asserts `NoBarDisplay` (`docs/spec/rounds.md` §9).
    pub fn bars_hidden(&self) -> bool {
        self.fighters
            .iter()
            .any(|fighter| fighter.assert_flags & fighter::ASSERT_NOBARDISPLAY != 0)
    }

    /// True when any fighter asserts `GlobalNoShadow`.
    pub fn shadows_hidden(&self) -> bool {
        self.fighters
            .iter()
            .any(|fighter| fighter.assert_flags & fighter::ASSERT_GLOBALNOSHADOW != 0)
    }

    pub fn screen_pos(&self, world: Vec2) -> Vec2 {
        let scale = self.camera.scale;
        let shake = self.camera.shake;
        let ground = self.ground_screen_y();
        let center_y = manato_core::GAME_HEIGHT * 0.5;
        let anchor_y = if self.stage_camera.zoom_anchor {
            ground
        } else {
            center_y
        };
        // Distance from the zoom anchor to the ground line at scale 1.
        let anchor_offset = if self.stage_camera.zoom_anchor {
            0.0
        } else {
            ground - center_y
        };
        Vec2::new(
            manato_core::GAME_WIDTH * 0.5 + (world.x - self.camera.x) * scale + shake.x,
            anchor_y + (world.y - self.camera.y + anchor_offset) * scale + shake.y,
        )
    }

    pub fn spawn_hit_spark(&mut self, who: usize, pos: Vec2, spark_no: i32, guarded: bool) {
        let anim = if spark_no >= 0 {
            spark_no
        } else if guarded {
            40
        } else {
            0
        };
        let duration = self
            .assets
            .effects
            .animations
            .get(&anim)
            .map(|a| a.total_time())
            .unwrap_or(8);
        self.effects.push(Effect {
            source: EffectSource::FightFx,
            anim,
            pos,
            scale: Vec2::new(1.0, 1.0),
            blend: EffectBlend::Add,
            sprite_priority: 5,
            duration,
            owner: who,
            ..Effect::default()
        });
    }

    pub fn checksum(&self) -> u64 {
        let mut h = StateHasher::new();
        h.write_u64(self.frame);
        h.write_i32(self.match_time);
        h.write_u8(self.first_attack.map(|s| s as u8 + 1).unwrap_or(0));
        h.write_i32(self.last_player_id);
        h.write_u32(self.round.number);
        h.write_u8(self.round.phase as u8);
        h.write_i32(self.round.timer);
        h.write_i32(self.round.phase_timer);
        h.write_i32(self.round.slow_timer);
        h.write_f32(self.round.slow_accum);
        h.write_bool(self.round.slow_disabled);
        h.write_u32(self.round.wins[0]);
        h.write_u32(self.round.wins[1]);
        h.write_i32(self.round.consecutive_wins[0]);
        h.write_i32(self.round.consecutive_wins[1]);
        h.write_f32(self.camera.x);
        h.write_f32(self.camera.y);
        h.write_f32(self.camera.scale);
        h.write_f32(self.camera.target_scale);
        h.write_i32(self.camera.zoom_delay);
        h.write_u8(self.camera.view as u8);
        h.write_u8(self.camera.follow.map(|i| i as u8 + 1).unwrap_or(0));
        h.write_f32(self.camera.free_pos.x);
        h.write_f32(self.camera.free_pos.y);
        h.write_f32(self.stage_camera.start.x);
        h.write_f32(self.stage_camera.start.y);
        h.write_f32(self.stage_camera.bound_left);
        h.write_f32(self.stage_camera.bound_right);
        h.write_f32(self.stage_camera.bound_high);
        h.write_f32(self.stage_camera.bound_low);
        h.write_f32(self.stage_camera.vertical_follow);
        h.write_f32(self.stage_camera.floor_tension);
        h.write_f32(self.stage_camera.tension);
        h.write_f32(self.stage_camera.cut_high);
        h.write_f32(self.stage_camera.cut_low);
        h.write_f32(self.stage_camera.start_zoom);
        h.write_f32(self.stage_camera.zoom_out);
        h.write_f32(self.stage_camera.zoom_in);
        h.write_i32(self.stage_camera.zoom_in_delay);
        h.write_f32(self.stage_camera.zoom_in_speed);
        h.write_f32(self.stage_camera.zoom_out_speed);
        h.write_bool(self.stage_camera.zoom_anchor);
        h.write_f32(self.stage_left_bound);
        h.write_f32(self.stage_right_bound);
        if let Some(zoom) = self.camera.zoom_override {
            h.write_bool(true);
            h.write_f32(zoom.target_pos.x);
            h.write_f32(zoom.target_pos.y);
            h.write_f32(zoom.target_scale);
            h.write_i32(zoom.elapsed);
            h.write_i32(zoom.hold);
        } else {
            h.write_bool(false);
        }
        h.write_u64(self.rng.state());
        h.write_i32(self.super_pause);
        h.write_i32(self.pause_movetime);
        h.write_u8(self.pause_mover.map(|i| i as u8 + 1).unwrap_or(0));
        h.write_u32(self.hit_activation);
        for side in 0..2 {
            h.write_f32(self.score[side]);
            h.write_f32(self.score_rounds[side]);
        }
        for combo in &self.combo {
            h.write_i32(combo.hits);
            h.write_i32(combo.shown_hits);
            h.write_i32(combo.damage);
            h.write_i32(combo.timer);
        }
        for side in &self.win_history {
            h.write_u32(side.len() as u32);
            for win in side {
                h.write_bool(win.time_over);
                h.write_bool(win.perfect);
                h.write_bool(win.clutch);
            }
        }
        for bar in &self.bars {
            h.write_f32(bar.mid_life);
            h.write_f32(bar.mid_power);
            h.write_f32(bar.mid_guard);
            h.write_i32(bar.delay);
        }
        for side in &self.actions {
            h.write_u32(side.len() as u32);
            for message in side {
                h.write_u32(message.text.len() as u32);
                h.write_bytes(message.text.as_bytes());
                h.write_i32(message.time);
                h.write_i32(message.total);
            }
        }
        h.write_u32(self.effects.len() as u32);
        for e in &self.effects {
            h.write_i32(e.anim);
            h.write_i32(e.id);
            h.write_f32(e.pos.x);
            h.write_f32(e.pos.y);
            h.write_f32(e.vel.x);
            h.write_f32(e.vel.y);
            h.write_f32(e.scale.x);
            h.write_f32(e.scale.y);
            h.write_i32(e.time);
            h.write_i32(e.duration);
            h.write_i32(e.bind_time);
            h.write_bool(e.bound);
            h.write_f32(e.alpha);
            h.write_f32(e.angle);
            h.write_f32(e.x_angle);
            h.write_f32(e.y_angle);
            h.write_f32(e.x_shear);
        }
        for layer in &self.stage_layers {
            h.write_f32(layer.offset.x);
            h.write_f32(layer.offset.y);
            h.write_f32(layer.velocity.x);
            h.write_f32(layer.velocity.y);
            h.write_bool(layer.visible);
            h.write_bool(layer.enabled);
            h.write_i32(layer.action.unwrap_or(i32::MIN));
            h.write_bool(layer.start.is_some());
            if let Some(start) = layer.start {
                h.write_f32(start.x);
                h.write_f32(start.y);
            }
            h.write_bool(layer.delta.is_some());
            if let Some(delta) = layer.delta {
                h.write_f32(delta.x);
                h.write_f32(delta.y);
            }
            h.write_bool(layer.scale.is_some());
            if let Some(scale) = layer.scale {
                h.write_f32(scale.x);
                h.write_f32(scale.y);
            }
        }
        if let Some(color) = self.env_color {
            h.write_f32(color.color[0]);
            h.write_f32(color.color[1]);
            h.write_f32(color.color[2]);
            h.write_f32(color.color[3]);
            h.write_i32(color.time);
        }
        for pal in [&self.all_pal, &self.bg_pal] {
            h.write_i32(pal.time);
            for value in pal.add {
                h.write_f32(value);
            }
            for value in pal.mul {
                h.write_f32(value);
            }
            h.write_f32(pal.alpha);
        }
        h.write_u32(self.bg_ctrls.len() as u32);
        for control in &self.bg_ctrls {
            h.write_u32(control.layer as u32);
            h.write_i32(control.start);
            h.write_i32(control.end);
            h.write_i32(control.loop_period);
            for byte in control.controller.kind.bytes() {
                h.write_u8(byte);
            }
            h.write_u8(0);
            h.write_u32(control.controller.params.len() as u32);
            for (key, value) in &control.controller.params {
                for byte in key.bytes() {
                    h.write_u8(byte);
                }
                h.write_u8(0);
                for byte in value.bytes() {
                    h.write_u8(byte);
                }
                h.write_u8(0);
            }
        }
        for p in &self.projectiles {
            h.write_i32(p.owner as i32);
            h.write_i32(p.id);
            h.write_i32(p.anim);
            h.write_f32(p.pos.x);
            h.write_f32(p.pos.y);
            h.write_f32(p.vel.x);
            h.write_f32(p.vel.y);
            h.write_f32(p.accel.x);
            h.write_f32(p.accel.y);
            h.write_i32(p.facing);
            h.write_i32(p.time);
            h.write_i32(p.remove_time);
            h.write_i32(p.hits);
            h.write_i32(p.hitdef.damage);
        }
        for f in &self.fighters {
            h.write_f32(f.pos.x);
            h.write_f32(f.pos.y);
            h.write_f32(f.height_player.x);
            h.write_f32(f.height_player.y);
            h.write_i32(f.shadow_intensity.unwrap_or(-1));
            for channel in f.shadow_color.unwrap_or([255; 3]) {
                h.write_u8(channel);
            }
            h.write_f32(f.shadow_offset.x);
            h.write_f32(f.shadow_offset.y);
            h.write_f32(f.shadow_xscale.unwrap_or(0.0));
            h.write_f32(f.shadow_yscale.unwrap_or(0.0));
            h.write_i32(f.reflection_intensity.unwrap_or(-1));
            for channel in f.reflection_color.unwrap_or([255; 3]) {
                h.write_u8(channel);
            }
            h.write_f32(f.reflection_offset.x);
            h.write_f32(f.reflection_offset.y);
            h.write_f32(f.reflection_xscale.unwrap_or(0.0));
            h.write_f32(f.reflection_yscale.unwrap_or(0.0));
            h.write_f32(f.draw_angle);
            h.write_bool(f.angle_draw);
            h.write_f32(f.angle_draw_value.unwrap_or(f32::NAN));
            let draw_scale = f.angle_draw_scale.unwrap_or(Vec2::new(f32::NAN, f32::NAN));
            h.write_f32(draw_scale.x);
            h.write_f32(draw_scale.y);
            h.write_f32(f.transform_x_shear);
            h.write_f32(f.transform_focal_length);
            h.write_u8(f.transform_projection as u8);
            for transforms in &f.clsn_transforms {
                h.write_u32(transforms.len() as u32);
                for transform in transforms {
                    h.write_i32(transform.index);
                    h.write_f32(transform.scale.x);
                    h.write_f32(transform.scale.y);
                }
            }
            for overrides in &f.clsn_overrides {
                h.write_u32(overrides.len() as u32);
                for entry in overrides {
                    h.write_i32(entry.index);
                    for value in entry.rect {
                        h.write_f32(value);
                    }
                }
            }
            h.write_u32(f.input_shift.len() as u32);
            for (source, target) in &f.input_shift {
                h.write_u32(*source as u32);
                h.write_i32(target.map(|value| value as i32).unwrap_or(-1));
            }
            h.write_f32(f.vel.x);
            h.write_f32(f.vel.y);
            h.write_i32(f.facing);
            h.write_i32(f.state_no);
            h.write_i32(f.prev_state_no);
            h.write_u8(f.prev_state_type as u8);
            h.write_u8(f.prev_move_type as u8);
            h.write_i32(f.state_time);
            h.write_i32(f.anim_no);
            h.write_u8(f.anim_char_id.map(|id| id as u8 + 1).unwrap_or(0));
            h.write_u8(f.state_char_id.map(|id| id as u8 + 1).unwrap_or(0));
            h.write_i32(f.anim_clock);
            h.write_i32(f.anim_frame as i32);
            h.write_i32(f.anim_time);
            h.write_i32(f.life);
            h.write_i32(f.red_life);
            h.write_i32(f.guard_points);
            h.write_i32(f.guard_max);
            h.write_i32(f.guard_crush_timer);
            h.write_bool(f.guard_break);
            h.write_i32(f.guard_cooldown);
            h.write_i32(f.dizzy_points);
            h.write_i32(f.dizzy_max);
            h.write_i32(f.dizzy_timer);
            h.write_bool(f.dizzy_flag);
            h.write_f32(f.power);
            h.write_bool(f.ctrl);
            h.write_bool(f.face_p2_pending);
            h.write_bool(f.player_push);
            h.write_bool(f.owner.is_some());
            if let Some(owner) = f.owner {
                h.write_i32(owner as i32);
            }
            h.write_i32(f.helper_id);
            h.write_i32(f.helper_remove_time);
            h.write_bool(f.helper_destroy);
            h.write_bool(f.screen_bound.0);
            h.write_bool(f.screen_bound.1);
            h.write_f32(f.draw_offset.x);
            h.write_f32(f.draw_offset.y);
            h.write_bool(f.hit_overridden);
            h.write_u32(f.hit_overrides.iter().flatten().count() as u32);
            for override_ in f.hit_overrides.iter().flatten() {
                h.write_i32(override_.state);
                h.write_i32(override_.time);
                h.write_bool(override_.keepstate);
                h.write_bool(override_.forceguard);
            }
            h.write_bool(f.reversal.is_some());
            h.write_i32(f.reversal_time);
            h.write_f32(f.trans_alpha);
            h.write_i32(f.trans_alpha_source);
            h.write_i32(f.trans_alpha_dest);
            h.write_bool(f.after_image.is_some());
            if let Some(after) = &f.after_image {
                h.write_i32(after.time);
                h.write_i32(after.interval);
                h.write_i32(after.life);
            }
            h.write_i32(f.after_images.len() as i32);
            for frame in &f.after_images {
                h.write_f32(frame.pos.x);
                h.write_f32(frame.pos.y);
                h.write_i32(frame.facing);
                h.write_i32(frame.anim_no);
                h.write_i32(frame.anim_frame as i32);
                h.write_i32(frame.life);
            }
            h.write_f32(f.pal_add[0]);
            h.write_f32(f.pal_add[1]);
            h.write_f32(f.pal_add[2]);
            h.write_f32(f.pal_mul[0]);
            h.write_f32(f.pal_mul[1]);
            h.write_f32(f.pal_mul[2]);
            h.write_f32(f.pal_alpha);
            h.write_i32(f.pal_time);
            for value in f.pal_base_add {
                h.write_f32(value);
            }
            for value in f.pal_sinadd {
                h.write_f32(value);
            }
            h.write_i32(f.pal_sin_period);
            h.write_i32(f.pal_cycle);
            for value in f.pal_base_mul {
                h.write_f32(value);
            }
            for value in f.pal_sinmul {
                h.write_f32(value);
            }
            h.write_i32(f.pal_sinmul_period);
            h.write_i32(f.pal_sinmul_cycle);
            h.write_bool(f.gravity.is_some());
            if let Some(gravity) = f.gravity {
                h.write_f32(gravity);
            }
            h.write_u8(f.state_type as u8);
            h.write_u8(f.move_type as u8);
            h.write_u8(f.physics as u8);
            h.write_i32(f.hit.hit_pause);
            h.write_i32(f.hit.hit_pause_time);
            h.write_i32(f.hit.hit_shake_timer);
            h.write_bool(f.state_changed_tick);
            h.write_i32(f.hit.pause_gap);
            h.write_i32(f.hit.vars.hittime);
            h.write_i32(f.juggle_points);
            h.write_i32(f.juggle);
            h.write_f32(f.fall_defence_mul);
            h.write_bool(f.target_bind.is_some());
            if let Some(bind) = f.target_bind {
                h.write_f32(bind.offset.x);
                h.write_f32(bind.offset.y);
                h.write_i32(bind.time);
            }
            h.write_bool(f.bind_anchor.is_some());
            if let Some(anchor) = f.bind_anchor {
                h.write_u32(anchor as u32);
                h.write_i32(f.bind_time);
                h.write_f32(f.bind_offset.x);
                h.write_f32(f.bind_offset.y);
                h.write_i32(f.bind_facing);
            }
            h.write_bool(f.hit.guarding);
            h.write_i32(f.received_hits);
            h.write_i32(f.received_damage);
            h.write_i32(f.proj_contact_time);
            h.write_i32(f.proj_hit_time);
            h.write_i32(f.proj_guarded_time);
            for &v in &f.vars {
                h.write_i32(v);
            }
            for &v in &f.fvars {
                h.write_f32(v);
            }
            for &v in &f.sysvars {
                h.write_i32(v);
            }
            for &v in &f.sysfvars {
                h.write_f32(v);
            }
            h.write_u32(f.map.len() as u32);
            for (key, value) in &f.map {
                h.write_u32(key.len() as u32);
                for byte in key.bytes() {
                    h.write_u8(byte);
                }
                match value {
                    crate::expr::Value::Int(v) => {
                        h.write_u8(0);
                        h.write_i32(*v);
                    }
                    crate::expr::Value::Float(v) => {
                        h.write_u8(1);
                        h.write_f32(*v);
                    }
                    crate::expr::Value::Bool(v) => {
                        h.write_u8(2);
                        h.write_bool(*v);
                    }
                    crate::expr::Value::Str(v) => {
                        h.write_u8(3);
                        h.write_u32(v.len() as u32);
                        for byte in v.bytes() {
                            h.write_u8(byte);
                        }
                    }
                }
            }
            for frame in &f.history {
                h.write_u8(frame.input.dir);
                h.write_u8(frame.input.buttons);
                h.write_u8(frame.pause_bits);
            }
        }
        h.finish()
    }
}

/// Advances screen-background layers and applies their controllers.
///
/// Shared by the stage and by front-end `[TitleBGdef]`/`[SelectBGdef]`
/// backgrounds (`docs/spec/render.md` §5.1).
pub fn update_bg_states(
    controls: &[manato_formats::BgCtrl],
    states: &mut [StageLayerState],
    bg_time: i32,
    round_no: u32,
    round_state: i32,
) {
    for (index, state) in states.iter_mut().enumerate() {
        state.offset.x += state.velocity.x;
        state.offset.y += state.velocity.y;
        for control in controls
            .iter()
            .filter(|control| control.layer == index && control.active_at(bg_time))
        {
            if !stage_controller_met(bg_time, round_no, round_state, state, &control.controller) {
                continue;
            }
            apply_stage_controller(state, &control.controller);
        }
    }
}

fn stage_controller_met(
    bg_time: i32,
    round_no: u32,
    round_state: i32,
    layer: &StageLayerState,
    control: &manato_formats::Controller,
) -> bool {
    let (all, groups) = control.trigger_groups();
    if !all
        .iter()
        .all(|trigger| stage_trigger_met(bg_time, round_no, round_state, layer, trigger))
    {
        return false;
    }
    if groups.is_empty() {
        return true;
    }
    groups.iter().any(|(_, triggers)| {
        triggers
            .iter()
            .all(|trigger| stage_trigger_met(bg_time, round_no, round_state, layer, trigger))
    })
}

fn stage_trigger_met(
    bg_time: i32,
    round_no: u32,
    round_state: i32,
    layer: &StageLayerState,
    text: &str,
) -> bool {
    let text = text.trim().to_ascii_lowercase();
    if text == "1" {
        return true;
    }
    let value = |name: &str| -> Option<i32> {
        match name {
            "time" => Some(bg_time),
            "anim" => Some(layer.action.unwrap_or(-1)),
            "roundno" => Some(round_no as i32),
            "roundstate" => Some(round_state),
            _ => None,
        }
    };
    if let Some((left, rest)) = text.split_once('%') {
        if let Some((modulus, expected)) = rest.split_once('=') {
            let Some(lhs) = value(left.trim()) else {
                return false;
            };
            let Ok(modulus) = modulus.trim().parse::<i32>() else {
                return false;
            };
            let Ok(expected) = expected.trim().parse::<i32>() else {
                return false;
            };
            return modulus != 0 && lhs % modulus == expected;
        }
    }
    for op in ["!=", "<=", ">=", "=", "<", ">"] {
        if let Some((left, right)) = text.split_once(op) {
            let Some(lhs) = value(left.trim()) else {
                return false;
            };
            let Ok(rhs) = right.trim().parse::<i32>() else {
                return false;
            };
            return match op {
                "=" => lhs == rhs,
                "!=" => lhs != rhs,
                "<" => lhs < rhs,
                "<=" => lhs <= rhs,
                ">" => lhs > rhs,
                ">=" => lhs >= rhs,
                _ => false,
            };
        }
    }
    false
}

fn stage_num(control: &manato_formats::Controller, key: &str) -> Option<f32> {
    control.get(key).and_then(|value| value.trim().parse().ok())
}

fn stage_triple(control: &manato_formats::Controller, key: &str) -> Option<[f32; 3]> {
    let value = control.get(key)?;
    let mut parts = value.split(',');
    Some([
        parts.next()?.trim().parse().ok()?,
        parts.next()?.trim().parse().ok()?,
        parts.next()?.trim().parse().ok()?,
    ])
}

fn apply_stage_controller(layer: &mut StageLayerState, control: &manato_formats::Controller) {
    match control.kind.as_str() {
        "anim" => {
            if let Some(value) = control.get_i32("value") {
                layer.action = Some(value);
            }
        }
        "visible" => {
            if let Some(value) = control.get_i32("value") {
                layer.visible = value != 0;
            }
        }
        "enable" => {
            if let Some(value) = control.get_i32("value") {
                layer.enabled = value != 0;
            }
        }
        "posset" => {
            if let Some(x) = stage_num(control, "x") {
                layer.offset.x = x;
            }
            if let Some(y) = stage_num(control, "y") {
                layer.offset.y = y;
            }
        }
        "posadd" => {
            if let Some(x) = stage_num(control, "x") {
                layer.offset.x += x;
            }
            if let Some(y) = stage_num(control, "y") {
                layer.offset.y += y;
            }
        }
        "velset" => {
            if let Some(x) = stage_num(control, "x") {
                layer.velocity.x = x;
            }
            if let Some(y) = stage_num(control, "y") {
                layer.velocity.y = y;
            }
        }
        "veladd" => {
            if let Some(x) = stage_num(control, "x") {
                layer.velocity.x += x;
            }
            if let Some(y) = stage_num(control, "y") {
                layer.velocity.y += y;
            }
        }
        "sinx" => {
            if let Some(value) = stage_triple(control, "value") {
                layer.sin_x = value;
            }
        }
        "siny" => {
            if let Some(value) = stage_triple(control, "value") {
                layer.sin_y = value;
            }
        }
        "cosx" => {
            if let Some(value) = stage_triple(control, "value") {
                layer.cos_x = value;
            }
        }
        "cosy" => {
            if let Some(value) = stage_triple(control, "value") {
                layer.cos_y = value;
            }
        }
        _ => {}
    }
}

fn update_exec_counter(
    sim: &mut SimState,
    who: usize,
    slot: usize,
    index: usize,
    met: bool,
    persistent: i32,
) -> bool {
    let counters = if slot >= 3 {
        &mut sim.fighters[who].ctrl_exec
    } else {
        &mut sim.fighters[who].extra_exec[slot]
    };
    if counters.len() <= index {
        counters.resize(index + 1, 0);
    }
    if !met {
        counters[index] = 0;
        return false;
    }
    if persistent == 0 {
        if counters[index] >= 1 {
            return false;
        }
        counters[index] = 1;
        return true;
    }
    // Negative values run on every matching tick in Ikemen-GO.
    if persistent < 0 {
        return true;
    }
    // Ikemen executes on the first matching tick, then skips `N - 1`
    // matching ticks (counter holds the remaining skip count).
    let period = persistent as u32;
    if counters[index] > 0 {
        counters[index] -= 1;
        return false;
    }
    counters[index] = period.saturating_sub(1);
    true
}

/// Renders a `Display` for a fighter slot (used by the CLI).
pub fn fighter_label(sim: &SimState, who: usize) -> String {
    sim.assets.characters[who].display_name().to_string()
}

/// Convenience for tests: run `ticks` frames with the supplied input function.
pub fn run_frames(
    sim: &mut SimState,
    ticks: u32,
    mut input: impl FnMut(&SimState, usize) -> PlayerInput,
) {
    for _ in 0..ticks {
        let i0 = input(sim, 0);
        let i1 = input(sim, 1);
        sim.tick([i0, i1]);
    }
}

/// Unused import guard for `GAME_HEIGHT` in no_std-ish builds.
const _: f32 = GAME_HEIGHT;

#[cfg(test)]
mod stage_tests {
    use super::*;
    use manato_formats::Controller;

    fn layer() -> StageLayerState {
        StageLayerState {
            offset: Vec2::ZERO,
            velocity: Vec2::ZERO,
            sin_x: [0.0; 3],
            sin_y: [0.0; 3],
            cos_x: [0.0; 3],
            cos_y: [0.0; 3],
            action: None,
            visible: true,
            enabled: true,
            start: None,
            delta: None,
            scale: None,
        }
    }

    #[test]
    fn stage_time_triggers() {
        let layer = layer();
        assert!(stage_trigger_met(10, 1, 2, &layer, "time = 10"));
        assert!(stage_trigger_met(10, 1, 2, &layer, "time >= 5"));
        assert!(stage_trigger_met(10, 1, 2, &layer, "time % 5 = 0"));
        assert!(!stage_trigger_met(10, 1, 2, &layer, "time < 5"));
        assert!(stage_trigger_met(10, 1, 2, &layer, "1"));
    }

    #[test]
    fn stage_controllers_mutate_layer() {
        let mut layer = layer();
        let velset = Controller {
            kind: "velset".into(),
            params: vec![
                ("type".into(), "VelSet".into()),
                ("x".into(), "2".into()),
                ("y".into(), "-1".into()),
            ],
            line: 0,
        };
        apply_stage_controller(&mut layer, &velset);
        assert_eq!(layer.velocity, Vec2::new(2.0, -1.0));

        let anim = Controller {
            kind: "anim".into(),
            params: vec![("type".into(), "Anim".into()), ("value".into(), "3".into())],
            line: 0,
        };
        apply_stage_controller(&mut layer, &anim);
        assert_eq!(layer.action, Some(3));

        let visible = Controller {
            kind: "visible".into(),
            params: vec![
                ("type".into(), "Visible".into()),
                ("value".into(), "0".into()),
            ],
            line: 0,
        };
        apply_stage_controller(&mut layer, &visible);
        assert!(!layer.visible);
    }
}
