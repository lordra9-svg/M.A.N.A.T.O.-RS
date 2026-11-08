//! Deterministic match simulation for M.A.N.A.T.O.-RS.
//!
//! The whole simulation is a single `Clone`-able [`SimState`]. That is the unit
//! of rollback: GGRS snapshots it before every frame and restores it when a
//! remote input arrives late. Static content (sprites, sounds, parsed scripts)
//! lives in [`Assets`] behind an `Arc`, so cloning a state is cheap.

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

        Ok(Self {
            characters,
            stage,
            effects,
            fight_sounds,
            data_dirs,
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
}

impl Default for MatchOptions {
    fn default() -> Self {
        Self {
            rounds_to_win: DEFAULT_ROUNDS_TO_WIN,
            round_time: DEFAULT_ROUND_TIME,
            ai_level: 4,
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
    pub winner: Option<usize>,
    /// True when the round ended on the timer rather than a KO.
    pub time_over: bool,
    pub display: String,
    pub rounds_to_win: u32,
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
            winner: None,
            time_over: false,
            display: "Round 1".into(),
            rounds_to_win,
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraState {
    pub x: f32,
    pub y: f32,
    pub shake: Vec2,
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvShake {
    pub time: i32,
    pub freq: f32,
    pub ampl: f32,
    pub phase: f32,
}

impl Default for EnvShake {
    fn default() -> Self {
        Self {
            time: 0,
            freq: 60.0,
            ampl: -4.0,
            phase: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
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
}

impl StageLayerState {
    fn from_layer(layer: &manato_formats::stage::BgLayer) -> Self {
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
        }
    }
}

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

#[derive(Debug, Clone)]
pub struct SimState {
    pub assets: Arc<Assets>,
    pub fighters: Vec<Fighter>,
    pub frame: u64,
    pub round: RoundState,
    pub camera: CameraState,
    pub effects: Vec<Effect>,
    pub projectiles: Vec<crate::projectile::Projectile>,
    pub sounds: Vec<SoundEvent>,
    pub sound_commands: Vec<SoundCommand>,
    pub bg_time: f32,
    pub env_shake: EnvShake,
    pub env_color: Option<EnvColor>,
    pub stage_layers: Vec<StageLayerState>,
    pub options: MatchOptions,
    pub rng: Rng,
    pub super_pause: i32,
    pub super_pause_move_time: i32,
    pub hit_activation: u32,
    pub finished: bool,
    /// Per-side combo counters (`§6.8`).
    pub combo: [ComboState; 2],
    /// Wins recorded this match, in order, per side (`§6.7`).
    pub win_history: [Vec<WinRecord>; 2],
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
            fighters.push(Fighter::new(
                &assets.characters[slot],
                slot,
                slot,
                slot,
                start,
                if facing == 0 { 1 } else { facing },
            ));
        }
        let stage_layers = assets
            .stage
            .layers
            .iter()
            .map(StageLayerState::from_layer)
            .collect();
        let mut state = Self {
            assets,
            fighters,
            frame: 0,
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
            stage_layers,
            options,
            rng: Rng::new(0x4D41_4E41_544F_5253),
            super_pause: 0,
            super_pause_move_time: 0,
            hit_activation: 1,
            finished: false,
            combo: [ComboState::default(); 2],
            win_history: [Vec::new(), Vec::new()],
        };
        state.start_round();
        state
    }

    pub fn character(&self, who: usize) -> &Character {
        &self.assets.characters[self.fighters[who].char_id]
    }

    pub fn other(&self, who: usize) -> usize {
        self.opponent_of(who)
    }

    pub fn opponent_of(&self, who: usize) -> usize {
        if let Some(owner) = self.fighters.get(who).and_then(|fighter| fighter.owner) {
            1 - owner
        } else {
            1 - who
        }
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
        // Control is locked during the round intro and KO slowdown, as in
        // M.U.G.E.N. This also keeps the CPU from attacking before "Fight!".
        let inputs = if matches!(self.round.phase, RoundPhase::Intro | RoundPhase::Ko) {
            [PlayerInput::default(); 2]
        } else {
            inputs
        };
        for (i, input) in inputs.iter().enumerate() {
            self.fighters[i].append_input(*input);
        }
        self.sounds.clear();
        self.sound_commands.clear();

        if self.super_pause > 0 {
            self.super_pause -= 1;
            if self.super_pause_move_time > 0 {
                self.super_pause_move_time -= 1;
            }
            self.update_round();
            self.update_effects();
            self.update_fight_screen();
            return;
        }

        if self.round.phase == RoundPhase::Over {
            self.update_effects();
            self.update_fight_screen();
            return;
        }

        for who in 0..self.fighters.len() {
            if self.fighters[who].hit.hit_pause > 0 {
                self.fighters[who].hit.hit_pause -= 1;
                self.fighters[who].state_changed = false;
                continue;
            }
            if self.fighters[who].hit.super_pause > 0 {
                self.fighters[who].hit.super_pause -= 1;
                continue;
            }
            self.update_fighter(who);
        }

        collision::resolve(self);
        self.separate_pushboxes();
        self.fighters.retain(|fighter| {
            fighter.owner.is_none()
                || (!fighter.helper_destroy
                    && (fighter.helper_remove_time < 0 || fighter.helper_remove_time > 0))
        });
        for projectile in &mut self.projectiles {
            projectile.advance();
        }
        collision::resolve_projectiles(self);
        self.projectiles.retain(|p| !p.is_expired());
        self.update_round();
        self.update_camera();
        self.update_effects();
        self.update_fight_screen();
        self.bg_time += 1.0;
        self.update_stage_layers();
    }

    fn update_stage_layers(&mut self) {
        let stage = &self.assets.stage;
        let bg_time = self.bg_time as i32;
        let round_no = self.round.number;
        let round_state = self.round.roundstate_code();
        for (index, state) in self.stage_layers.iter_mut().enumerate() {
            state.offset.x += state.velocity.x;
            state.offset.y += state.velocity.y;
            for ctrl in stage.bg_ctrls.iter().filter(|ctrl| ctrl.layer == index) {
                if !stage_controller_met(bg_time, round_no, round_state, state, &ctrl.controller) {
                    continue;
                }
                apply_stage_controller(state, &ctrl.controller);
            }
        }
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
            fighter.hit.hit_shake_timer = (fighter.hit.hit_shake_timer - 1).max(0);
            fighter.hit.hit_over_timer = (fighter.hit.hit_over_timer - 1).max(0);
            // Leaving hitstun ends the combo the attacker was building
            // (`docs/spec/render.md` §6.8).
            if fighter.move_type != fighter::MoveType::Hit {
                fighter.received_hits = 0;
                fighter.received_damage = 0;
            }
            fighter.pos_freeze = (fighter.pos_freeze - 1).max(0);
            fighter.hit.invuln_timer = (fighter.hit.invuln_timer - 1).max(0);
            if let Some(override_) = fighter.hit_override.as_mut() {
                if override_.time > 0 {
                    override_.time -= 1;
                    if override_.time <= 0 {
                        fighter.hit_override = None;
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
            if let Some((_, t)) = fighter.hit.not_hit_by.as_mut() {
                *t -= 1;
                if *t <= 0 {
                    fighter.hit.not_hit_by = None;
                }
            }
        }

        // Special states run every tick, in order -3, -2, -1.
        if self.fighters[who].active {
            self.run_state_controllers(who, -3);
            self.run_state_controllers(who, -2);
            self.run_state_controllers(who, -1);
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
        self.fighters[who].state_changed = false;

        // Advance animation last so triggers observe this tick's frame.
        let assets = self.assets.clone();
        let char = &assets.characters[self.fighters[who].char_id];
        self.fighters[who].advance_anim(char);
        self.fighters[who].refresh_widths(char);
        self.update_after_image(who);
        self.update_pal_fx(who);
    }

    fn update_after_image(&mut self, who: usize) {
        let fighter = &mut self.fighters[who];
        if let Some(state) = fighter.after_image.as_mut() {
            if state.time > 0 {
                state.time -= 1;
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
            } else {
                fighter.after_image = None;
            }
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
                fighter.pal_mul = [1.0, 1.0, 1.0];
                fighter.pal_alpha = 1.0;
            }
        }
    }

    /// Runs `[Statedef n]` controllers in file order. Returns true when a
    /// controller changed the state (the remaining controllers are skipped).
    fn run_state_controllers(&mut self, who: usize, state_no: i32) -> bool {
        let assets = self.assets.clone();
        let char = &assets.characters[self.fighters[who].char_id];
        let Some(state) = char.state(state_no) else {
            return false;
        };

        let mut changed = false;
        for (index, control) in state.controllers.iter().enumerate() {
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
            controller::execute(self, who, control, &assets);
            if self.fighters[who].state_no != before {
                changed = true;
                break;
            }
        }
        changed
    }

    /// The engine's built-in transitions between the default movement states
    /// (0 stand, 10/11 crouch, 20 walk, 40 jump start, 50/52 jump).
    fn builtin_movement(&mut self, who: usize) {
        let fighter = &self.fighters[who];
        if !fighter.active || !fighter.ctrl || fighter.hit_pause() > 0 {
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
                let accel = if fall_state && fighter.hit.vars.y_accel > 0.0 {
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

        // Hit-fall ground level (common1 states use `groundlevel`).
        if fall_state
            && fighter.vel.y > 0.0
            && fighter.pos.y >= ground_level
            && fighter.state_no == 5030
        {
            fighter.pos.y = 0.0;
            fighter.vel.y = 0.0;
        }
    }

    fn separate_pushboxes(&mut self) {
        let stage_left = self.assets.stage.left_bound;
        let stage_right = self.assets.stage.right_bound;
        for i in 0..2 {
            if self.fighters[i].screen_bound.0 {
                let p = self.fighters[i].pos.x;
                self.fighters[i].pos.x = clamp(p, stage_left, stage_right);
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

    fn update_round(&mut self) {
        match self.round.phase {
            RoundPhase::Intro => {
                self.round.phase_timer -= 1;
                if self.round.phase_timer <= 0 {
                    self.round.phase = RoundPhase::Fight;
                    self.round.display.clear();
                }
            }
            RoundPhase::Fight => {
                self.round.timer -= 1;
                let dead: Vec<usize> = (0..2).filter(|&i| self.fighters[i].life <= 0).collect();
                if !dead.is_empty() {
                    let winner = if dead.len() == 1 {
                        Some(1 - dead[0])
                    } else {
                        None
                    };
                    self.round.winner = winner;
                    self.round.time_over = false;
                    self.round.phase = RoundPhase::Ko;
                    self.round.phase_timer = 120;
                    self.round.display = match winner {
                        Some(w) => format!("{} Wins", self.assets.characters[w].display_name()),
                        None => "Draw Game".into(),
                    };
                    if let Some(w) = winner {
                        self.round.wins[w] += 1;
                        self.record_win(w, false);
                        self.fighters[w].power = (self.fighters[w].power + 200.0).min(MAX_POWER);
                        self.enter_win_pose(w);
                    }
                } else if self.round.timer <= 0 {
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
                    self.round.phase_timer = 120;
                    self.round.display = match winner {
                        Some(w) => format!(
                            "Time Over - {} Wins",
                            self.assets.characters[w].display_name()
                        ),
                        None => "Time Over - Draw".into(),
                    };
                    if let Some(w) = winner {
                        self.round.wins[w] += 1;
                        self.record_win(w, true);
                    }
                }
            }
            RoundPhase::Ko => {
                if self.round_not_over() {
                    return;
                }
                self.round.phase_timer -= 1;
                if self.round.phase_timer <= 0 {
                    if self.round.wins[0] >= self.round.rounds_to_win
                        || self.round.wins[1] >= self.round.rounds_to_win
                        || self.round.number >= 3
                    {
                        let winner = if self.round.wins[0] > self.round.wins[1] {
                            Some(0)
                        } else if self.round.wins[1] > self.round.wins[0] {
                            Some(1)
                        } else {
                            None
                        };
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

    /// Updates presentation state that lives in the simulation: combo
    /// counters (`docs/spec/render.md` §6.8).
    fn update_fight_screen(&mut self) {
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

    /// Records a round win and its perfect/clutch classification (`§6.7`).
    fn record_win(&mut self, winner: usize, time_over: bool) {
        let fighter = &self.fighters[winner];
        let perfect = fighter.life >= fighter.life_max;
        let clutch = !perfect
            && (fighter.life as f32) <= fighter.life_max as f32 * CLUTCH_THRESHOLD;
        self.win_history[winner].push(WinRecord {
            time_over,
            perfect,
            clutch,
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

    fn start_round(&mut self) {
        self.fighters.truncate(2);
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
            fighter.power = power.min(MAX_POWER);
            fighter.state_no = 0;
            fighter.prev_state_no = 0;
            fighter.state_time = 0;
            fighter.anim_no = 0;
            fighter.anim_clock = 0;
            fighter.anim_frame = 0;
            fighter.anim_time = 0;
            fighter.anim_looped = 0;
            fighter.ctrl = true;
            fighter.state_type = StateType::Stand;
            fighter.move_type = fighter::MoveType::Idle;
            fighter.physics = fighter::Physics::Stand;
            fighter.hit = HitState::default();
            fighter.attack = fighter::AttackState::default();
            fighter.hitdef = None;
            fighter.ctrl_exec.clear();
            fighter.extra_exec = [Vec::new(), Vec::new(), Vec::new()];
            fighter.pos_freeze = 0;
            fighter.assert_flags = 0;
            fighter.face_p2_pending = false;
            fighter.player_push = true;
            fighter.gravity = None;
            fighter.after_image = None;
            fighter.after_images.clear();
            fighter.pal_add = [0.0, 0.0, 0.0];
            fighter.pal_mul = [1.0, 1.0, 1.0];
            fighter.pal_alpha = 1.0;
            fighter.pal_time = 0;
            fighter.target = None;
            fighter.received_hits = 0;
            fighter.received_damage = 0;
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
        self.effects.clear();
        self.projectiles.clear();
        self.combo = [ComboState::default(); 2];
        self.camera.x = self.assets.stage.camera.start.x;
        self.camera.y = self.assets.stage.camera.start.y;
        self.camera.scale = self.assets.stage.camera.start_zoom;
        self.camera.target_scale = self.assets.stage.camera.start_zoom;
        self.camera.zoom_delay = 0;
        self.camera.zoom_override = None;
        self.env_shake = EnvShake::default();
        self.env_color = None;
        if self.assets.stage.reset_bg {
            self.bg_time = 0.0;
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
        let stage = &self.assets.stage;
        let cam = &stage.camera;

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
        } else {
            let mid_x = (self.fighters[0].pos.x + self.fighters[1].pos.x) * 0.5;
            let highest = self.fighters[0].pos.y.min(self.fighters[1].pos.y);

            // Visible extents of the non-helper fighters, including their body
            // boxes. Helpers never drive the camera.
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
                top = top.min(fighter.pos.y - fighter.height);
                bottom = bottom.max(fighter.pos.y);
            }
            if !left.is_finite() {
                left = mid_x;
                right = mid_x;
                top = highest;
                bottom = highest;
            }
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
            let target_y = clamp(highest, cam.bound_high, cam.bound_low);
            // Smooth follow.
            self.camera.x += (target_x - self.camera.x) * 0.35;
            self.camera.y += (target_y - self.camera.y) * 0.35;
        }

        if self.env_shake.time > 0 {
            self.env_shake.time -= 1;
            self.env_shake.phase += 1.0;
            let amp = self.env_shake.ampl;
            self.camera.shake = Vec2::new(
                (self.env_shake.phase * self.env_shake.freq * 0.1).sin() * amp,
                (self.env_shake.phase * self.env_shake.freq * 0.13).cos() * amp * 0.5,
            );
        } else {
            self.camera.shake = Vec2::ZERO;
        }
    }

    fn update_effects(&mut self) {
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
            let state_changed = owner.map(|f| f.state_changed).unwrap_or(false);

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
    pub fn screen_pos(&self, world: Vec2) -> Vec2 {
        let scale = self.camera.scale;
        let shake = self.camera.shake;
        let ground = self.ground_screen_y();
        let center_y = manato_core::GAME_HEIGHT * 0.5;
        let anchor_y = if self.assets.stage.camera.zoom_anchor {
            ground
        } else {
            center_y
        };
        // Distance from the zoom anchor to the ground line at scale 1.
        let anchor_offset = if self.assets.stage.camera.zoom_anchor {
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
        h.write_u32(self.round.number);
        h.write_u8(self.round.phase as u8);
        h.write_i32(self.round.timer);
        h.write_i32(self.round.phase_timer);
        h.write_u32(self.round.wins[0]);
        h.write_u32(self.round.wins[1]);
        h.write_f32(self.camera.x);
        h.write_f32(self.camera.y);
        h.write_f32(self.camera.scale);
        h.write_f32(self.camera.target_scale);
        h.write_i32(self.camera.zoom_delay);
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
        h.write_u32(self.hit_activation);
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
        }
        if let Some(color) = self.env_color {
            h.write_f32(color.color[0]);
            h.write_f32(color.color[1]);
            h.write_f32(color.color[2]);
            h.write_f32(color.color[3]);
            h.write_i32(color.time);
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
            h.write_f32(f.vel.x);
            h.write_f32(f.vel.y);
            h.write_i32(f.facing);
            h.write_i32(f.state_no);
            h.write_i32(f.prev_state_no);
            h.write_i32(f.state_time);
            h.write_i32(f.anim_no);
            h.write_i32(f.anim_clock);
            h.write_i32(f.anim_frame as i32);
            h.write_i32(f.anim_time);
            h.write_i32(f.life);
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
            h.write_bool(f.hit_override.is_some());
            if let Some(override_) = &f.hit_override {
                h.write_i32(override_.state);
                h.write_i32(override_.time);
            }
            h.write_bool(f.reversal.is_some());
            h.write_i32(f.reversal_time);
            h.write_f32(f.trans_alpha);
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
            h.write_bool(f.gravity.is_some());
            if let Some(gravity) = f.gravity {
                h.write_f32(gravity);
            }
            h.write_u8(f.state_type as u8);
            h.write_u8(f.move_type as u8);
            h.write_u8(f.physics as u8);
            h.write_i32(f.hit.hit_pause);
            h.write_i32(f.hit.vars.hittime);
            h.write_bool(f.hit.guarding);
            h.write_i32(f.received_hits);
            h.write_i32(f.received_damage);
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
            for frame in &f.history {
                h.write_u8(frame.input.dir);
                h.write_u8(frame.input.buttons);
            }
        }
        h.finish()
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
    let period = persistent.max(1) as u32;
    counters[index] += 1;
    if counters[index] >= period {
        counters[index] = 0;
        true
    } else {
        false
    }
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
