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
            RoundPhase::Fight => 1,
            RoundPhase::Ko => 2,
            RoundPhase::Over => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraState {
    pub x: f32,
    pub y: f32,
    pub shake: Vec2,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            shake: Vec2::ZERO,
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
    pub alpha: f32,
    pub sprite_priority: i32,
    pub time: i32,
    pub duration: i32,
    pub remove_on_anim_end: bool,
    pub id: i32,
    pub owner: usize,
    pub behind: bool,
}

impl Effect {
    pub fn is_expired(&self) -> bool {
        (self.remove_on_anim_end || self.duration >= 0) && self.time >= self.duration
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
pub struct SimState {
    pub assets: Arc<Assets>,
    pub fighters: Vec<Fighter>,
    pub frame: u64,
    pub round: RoundState,
    pub camera: CameraState,
    pub effects: Vec<Effect>,
    pub sounds: Vec<SoundEvent>,
    pub bg_time: f32,
    pub env_shake: EnvShake,
    pub options: MatchOptions,
    pub rng: Rng,
    pub super_pause: i32,
    pub super_pause_move_time: i32,
    pub hit_activation: u32,
    pub finished: bool,
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
        let mut state = Self {
            assets,
            fighters,
            frame: 0,
            round: RoundState::new(options.rounds_to_win, options.round_time),
            camera: CameraState {
                x: cam_start.x,
                y: cam_start.y,
                shake: Vec2::ZERO,
            },
            effects: Vec::new(),
            sounds: Vec::new(),
            bg_time: 0.0,
            env_shake: EnvShake::default(),
            options,
            rng: Rng::new(0x4D41_4E41_544F_5253),
            super_pause: 0,
            super_pause_move_time: 0,
            hit_activation: 1,
            finished: false,
        };
        state.start_round();
        state
    }

    pub fn character(&self, who: usize) -> &Character {
        &self.assets.characters[self.fighters[who].char_id]
    }

    pub fn other(&self, who: usize) -> usize {
        1 - who
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

        if self.super_pause > 0 {
            self.super_pause -= 1;
            if self.super_pause_move_time > 0 {
                self.super_pause_move_time -= 1;
            }
            self.update_effects();
            return;
        }

        if self.round.phase == RoundPhase::Over {
            self.update_effects();
            return;
        }

        for who in 0..2 {
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
        self.update_round();
        self.update_camera();
        self.update_effects();
        self.bg_time += 1.0;
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
            fighter.pos_freeze = (fighter.pos_freeze - 1).max(0);
            fighter.hit.invuln_timer = (fighter.hit.invuln_timer - 1).max(0);
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
        self.fighters[who].state_changed = false;

        // Advance animation last so triggers observe this tick's frame.
        let assets = self.assets.clone();
        let char = &assets.characters[self.fighters[who].char_id];
        self.fighters[who].advance_anim(char);
        self.fighters[who].refresh_widths(char);
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
            let p = self.fighters[i].pos.x;
            self.fighters[i].pos.x = clamp(p, stage_left, stage_right);
        }
        let (a, b) = (0usize, 1usize);
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
                    self.round.phase = RoundPhase::Ko;
                    self.round.phase_timer = 120;
                    self.round.display = match winner {
                        Some(w) => format!("{} Wins", self.assets.characters[w].display_name()),
                        None => "Draw Game".into(),
                    };
                    if let Some(w) = winner {
                        self.round.wins[w] += 1;
                        self.fighters[w].power = (self.fighters[w].power + 200.0).min(MAX_POWER);
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
                    }
                }
            }
            RoundPhase::Ko => {
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

    fn start_round(&mut self) {
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
            fighter.target = None;
            fighter.state_changed = false;
            fighter.enter_state(0, char, false);
            fighter.ctrl = true;
        }
        self.round.phase = RoundPhase::Intro;
        self.round.phase_timer = 40;
        self.round.timer = self.round.round_time * 60;
        self.round.winner = None;
        self.round.display = format!("Round {}", self.round.number);
        self.effects.clear();
        self.camera.x = self.assets.stage.camera.start.x;
        self.camera.y = self.assets.stage.camera.start.y;
        self.env_shake = EnvShake::default();
    }

    fn update_camera(&mut self) {
        let stage = &self.assets.stage;
        let mid_x = (self.fighters[0].pos.x + self.fighters[1].pos.x) * 0.5;
        let highest = self.fighters[0].pos.y.min(self.fighters[1].pos.y);
        let cam = &stage.camera;
        let target_x = clamp(mid_x, cam.bound_left, cam.bound_right);
        let target_y = clamp(highest, cam.bound_high, cam.bound_low);
        // Smooth follow.
        self.camera.x += (target_x - self.camera.x) * 0.35;
        self.camera.y += (target_y - self.camera.y) * 0.35;

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
        for effect in &mut self.effects {
            effect.time += 1;
            effect.pos += effect.vel;
            effect.vel += effect.accel;
        }
        self.effects.retain(|e| !e.is_expired());
    }

    /// Absolute ground position used by rendering: the stage's ground line.
    pub fn ground_screen_y(&self) -> f32 {
        self.assets.stage.zoffset
    }

    pub fn screen_pos(&self, world: Vec2) -> Vec2 {
        Vec2::new(
            manato_core::GAME_WIDTH * 0.5 + world.x - self.camera.x + self.camera.shake.x,
            self.ground_screen_y() + world.y - self.camera.y + self.camera.shake.y,
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
            vel: Vec2::ZERO,
            accel: Vec2::ZERO,
            scale: Vec2::new(1.0, 1.0),
            facing: 1,
            flip_h: false,
            flip_v: false,
            blend: EffectBlend::Add,
            alpha: 1.0,
            sprite_priority: 5,
            time: 0,
            duration,
            remove_on_anim_end: true,
            id: 0,
            owner: who,
            behind: false,
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
        h.write_u64(self.rng.state());
        h.write_i32(self.super_pause);
        h.write_u32(self.hit_activation);
        h.write_u32(self.effects.len() as u32);
        for e in &self.effects {
            h.write_i32(e.anim);
            h.write_f32(e.pos.x);
            h.write_f32(e.pos.y);
            h.write_i32(e.time);
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
            h.write_u8(f.state_type as u8);
            h.write_u8(f.move_type as u8);
            h.write_u8(f.physics as u8);
            h.write_i32(f.hit.hit_pause);
            h.write_i32(f.hit.vars.hittime);
            h.write_bool(f.hit.guarding);
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
