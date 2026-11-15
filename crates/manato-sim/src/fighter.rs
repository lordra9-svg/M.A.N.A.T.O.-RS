use std::collections::{BTreeMap, VecDeque};

use manato_core::{Buttons, Dir8, PlayerInput, Rect, Vec2, DIR_DOWN, DIR_LEFT, DIR_RIGHT, DIR_UP};
use manato_formats::{Animation, Character, CommandDef, CommandStep, StateDef};

use crate::expr::Value;
use crate::hitdef::{HitDef, HitState};

/// One `TransformClsn` entry; `index < 0` applies to every box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClsnTransform {
    pub index: i32,
    pub scale: Vec2,
}

/// One `OverrideClsn` entry. A zero rect deletes matching boxes; otherwise
/// the rect replaces boxes (`index < 0` for every box) or adds a new box when
/// `index` is past the end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClsnOverride {
    pub index: i32,
    /// Normalized `(x1, y1, x2, y2)` in local coordinates.
    pub rect: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StateType {
    #[default]
    Stand,
    Crouch,
    Air,
    Liedown,
}

impl StateType {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "S" => Some(Self::Stand),
            "C" => Some(Self::Crouch),
            "A" => Some(Self::Air),
            "L" => Some(Self::Liedown),
            "U" => None,
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Stand => "S",
            Self::Crouch => "C",
            Self::Air => "A",
            Self::Liedown => "L",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MoveType {
    #[default]
    Idle,
    Attack,
    Hit,
}

impl MoveType {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "I" => Some(Self::Idle),
            "A" => Some(Self::Attack),
            "H" => Some(Self::Hit),
            "U" => None,
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Idle => "I",
            Self::Attack => "A",
            Self::Hit => "H",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Physics {
    #[default]
    Stand,
    Crouch,
    Air,
    None,
}

impl Physics {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "S" => Some(Self::Stand),
            "C" => Some(Self::Crouch),
            "A" => Some(Self::Air),
            "N" => Some(Self::None),
            "U" => None,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttackState {
    /// `HitCount`: sum of `numhits` from the first contact of each swing.
    pub hit_count: i32,
    /// `UniqHitCount`: every individual contact in the current swing.
    pub uniq_hit_count: i32,
    pub move_contact: bool,
    pub move_hit: bool,
    pub move_guarded: bool,
    pub move_reversed: bool,
    /// `MoveCountered`: the current hit connected while the defender was
    /// attacking (`MoveType = A`).
    pub counter_hit: bool,
    pub guard_count: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistoryFrame {
    pub input: PlayerInput,
    pub dir: Dir8,
    pub prev_dir: Dir8,
    pub pressed_buttons: u8,
    pub released_buttons: u8,
    /// Pause state when the input was sampled: bit 0 = hit pause,
    /// bit 1 = Pause/SuperPause.
    pub pause_bits: u8,
}

/// Mutable state of one fighter. This is the unit of rollback snapshots, so it
/// must not contain references to OS resources or unordered collections.
#[derive(Debug, Clone)]
pub struct AfterImageFrame {
    pub pos: Vec2,
    pub facing: i32,
    pub anim_no: i32,
    pub anim_frame: usize,
    pub life: i32,
    pub alpha: f32,
    pub add: bool,
}

#[derive(Debug, Clone)]
pub struct AfterImageState {
    pub time: i32,
    pub interval: i32,
    pub tick: i32,
    pub life: i32,
    pub alpha: f32,
    pub add: bool,
}

#[derive(Debug, Clone)]
pub struct HitOverride {
    pub attr: String,
    /// Target state, or the current state when `keepstate` is set.
    pub state: i32,
    /// Remaining ticks; negative = infinite, `0` = inactive.
    pub time: i32,
    /// `keepstate = 1`: negate the hit without changing state.
    pub keepstate: bool,
    /// `forceguard = 1`: convert the overridden hit into a guarded hit.
    pub forceguard: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetBind {
    pub offset: Vec2,
    pub time: i32,
}

#[derive(Debug, Clone)]
pub struct Fighter {
    pub char_id: usize,
    /// Character whose state/controller bank is used when `p2getp1state`
    /// borrows P1's state data.
    pub state_char_id: Option<usize>,
    pub side: usize,
    pub slot: usize,
    pub active: bool,
    /// Difficulty of the AI driving this slot; 0 when AI is off. Helpers
    /// inherit their owner's level.
    pub ai_level: i32,

    pub pos: Vec2,
    pub vel: Vec2,
    pub facing: i32,

    pub state_no: i32,
    pub prev_state_no: i32,
    /// State/move type before the most recent type change (`PrevStateType`,
    /// `PrevMoveType`).
    pub prev_state_type: StateType,
    pub prev_move_type: MoveType,
    pub state_time: i32,

    pub anim_no: i32,
    /// Animation number active before the most recent change (`PrevAnim`).
    pub prev_anim: i32,
    /// Character whose animation bank supplies `anim_no`; `None` = own bank.
    pub anim_char_id: Option<usize>,
    pub anim_clock: i32,
    pub anim_frame: usize,
    pub anim_time: i32,
    pub anim_looped: i32,

    pub ctrl: bool,
    pub state_type: StateType,
    pub move_type: MoveType,
    pub physics: Physics,

    pub life: i32,
    pub life_max: i32,
    /// Recoverable life shown by the red life bar (`RedLife`).
    pub red_life: i32,
    /// Guard points and their maximum; 0 means no guard bar.
    pub guard_points: i32,
    pub guard_max: i32,
    /// Remaining ticks of the guard-crush no-guard window.
    pub guard_crush_timer: i32,
    /// Persistent guard-break flag (`GuardBreakSet`).
    pub guard_break: bool,
    /// Dizzy points/max and remaining stun duration.
    pub dizzy_points: i32,
    pub dizzy_max: i32,
    pub dizzy_timer: i32,
    /// Persistent dizzy flag (`DizzySet`).
    pub dizzy_flag: bool,
    /// Remaining ticks before guard-point regeneration resumes.
    pub guard_cooldown: i32,
    pub power: f32,
    pub defence_mul: f32,
    pub attack_mul: f32,

    pub vars: [i32; 60],
    pub fvars: [f32; 40],
    pub sysvars: [i32; 5],
    pub sysfvars: [f32; 5],

    pub juggle_points: i32,
    /// Air-juggle cost of the current state's attack (`[Statedef] juggle`).
    pub juggle: i32,
    /// Active `TargetBind` offset and remaining ticks (`-1` = infinite).
    pub target_bind: Option<TargetBind>,
    /// Damage multiplier applied after the first fall (Ikemen fall defence).
    pub fall_defence_mul: f32,
    pub sprite_priority: i32,
    pub width_back: f32,
    pub width_front: f32,
    pub height: f32,
    /// `Height` controller offsets: `x` extends the top, `y` the bottom.
    pub height_player: Vec2,
    /// Per-tick `ModifyShadow` overrides; `None` uses the stage shadow.
    pub shadow_intensity: Option<i32>,
    pub shadow_color: Option<[u8; 3]>,
    pub shadow_offset: Vec2,
    pub shadow_xscale: Option<f32>,
    pub shadow_yscale: Option<f32>,
    /// Persistent drawing angle set by `AngleSet`/`AngleAdd`/`AngleMul`.
    pub draw_angle: f32,
    /// Per-tick `AngleDraw` activation and overrides.
    pub angle_draw: bool,
    pub angle_draw_value: Option<f32>,
    pub angle_draw_scale: Option<Vec2>,
    /// Per-tick `TransformSprite` overrides.
    pub transform_x_shear: f32,
    pub transform_focal_length: f32,
    pub transform_projection: crate::ProjectionMode,
    /// Per-tick `TransformClsn` entries for Clsn1/Clsn2.
    pub clsn_transforms: [Vec<ClsnTransform>; 2],
    /// Per-tick `OverrideClsn` entries. Groups 1–4 mirror Ikemen (Clsn1,
    /// Clsn2, size, dummy); only the first two affect this engine.
    pub clsn_overrides: [Vec<ClsnOverride>; 4],
    /// Per-tick `ShiftInput` key remapping. Index 0–3 are U/D/L/R, 4–10 are
    /// a/b/c/x/y/z/s. `None` disables the source key.
    pub input_shift: Vec<(usize, Option<usize>)>,
    /// Per-tick `ModifyReflection` overrides; `None` uses the stage reflection.
    pub reflection_intensity: Option<i32>,
    pub reflection_color: Option<[u8; 3]>,
    pub reflection_offset: Vec2,
    pub reflection_xscale: Option<f32>,
    pub reflection_yscale: Option<f32>,
    /// Per-entity sprite scale (Helper `size.xscale`/`size.yscale`).
    pub size_xscale: f32,
    pub size_yscale: f32,
    /// Ticks since the last MakeDust emission (`spacing` gate).
    pub make_dust_spacing: i32,
    /// A `Width` controller override that lasts until the state changes.
    pub width_override: Option<(f32, f32)>,

    pub hitdef: Option<HitDef>,
    pub hit: HitState,
    pub attack: AttackState,

    pub history: VecDeque<HistoryFrame>,
    pub ctrl_exec: Vec<u32>,
    pub extra_exec: [Vec<u32>; 3],
    pub pos_freeze: i32,
    pub assert_flags: u32,
    pub face_p2_pending: bool,
    pub player_push: bool,
    pub gravity: Option<f32>,
    pub screen_bound: (bool, bool),
    pub draw_offset: Vec2,
    pub hit_overrides: [Option<HitOverride>; 8],
    /// True when a `HitOverride` matched during the current tick.
    pub hit_overridden: bool,
    pub after_image: Option<AfterImageState>,
    pub after_images: Vec<AfterImageFrame>,
    pub pal_add: [f32; 3],
    /// Base add before the `sinadd` oscillation.
    pub pal_base_add: [f32; 3],
    pub pal_sinadd: [f32; 3],
    /// `sinadd` cycle length in ticks; 0 disables the oscillation.
    pub pal_sin_period: i32,
    pub pal_cycle: i32,
    pub pal_mul: [f32; 3],
    /// Base multiplier before the `sinmul` oscillation.
    pub pal_base_mul: [f32; 3],
    pub pal_sinmul: [f32; 3],
    /// `sinmul` cycle length in ticks; 0 disables the oscillation.
    pub pal_sinmul_period: i32,
    pub pal_sinmul_cycle: i32,
    pub pal_alpha: f32,
    pub pal_time: i32,
    /// Destination alpha used by rendering (0..1).
    pub trans_alpha: f32,
    /// Raw trans source/destination alpha bytes reported by
    /// `Alpha.Source`/`Alpha.Dest` (0..255).
    pub trans_alpha_source: i32,
    pub trans_alpha_dest: i32,
    pub reversal: Option<HitDef>,
    pub reversal_time: i32,
    pub owner: Option<usize>,
    pub parent: Option<usize>,
    pub root: Option<usize>,
    pub helper_id: i32,
    /// Display name of a helper (`HelperName`); `None` for players.
    pub helper_name: Option<String>,
    pub helper_remove_time: i32,
    pub helper_destroy: bool,
    /// `keyctrl` bits for helpers: 1 = state -1, 2 = state -2, 4 = state -3.
    pub keyctrl: u8,
    /// `helpertype = player` lets states -3/-2 run like a player.
    pub helper_type_player: bool,
    pub target: Option<usize>,
    /// Persistent `BindToRoot`/`BindToParent`/`BindToTarget` state. The
    /// anchor moves and this entity follows while `bind_time` runs; `-1`
    /// binds forever.
    pub bind_anchor: Option<usize>,
    pub bind_time: i32,
    pub bind_offset: Vec2,
    /// `BindToRoot`/`BindToParent` facing: 1 = face the anchor, -1 = face
    /// away, 0 = leave facing alone.
    pub bind_facing: i32,
    /// `VictoryQuote` index; `-1` selects a default quote.
    pub win_quote: i32,
    /// Per-fighter `Map()` storage; deterministic ordering for rollback.
    pub map: BTreeMap<String, Value>,
    pub pal_remap: Option<(i32, i32)>,
    /// Ticks since the last projectile contact (`-1` = never).
    pub proj_contact_time: i32,
    /// Ticks since the last projectile hit (`-1` = never).
    pub proj_hit_time: i32,
    /// Ticks since the last guarded projectile hit (`-1` = never).
    pub proj_guarded_time: i32,
    /// Hits taken while in the current hitstun (`combo` tracking).
    pub received_hits: i32,
    /// Damage taken while in the current hitstun.
    pub received_damage: i32,

    /// Set when the fighter changed state this tick (skip duplicate updates).
    pub state_changed: bool,
    /// Set when a state change occurs this tick and cleared after effects, so
    /// explod removal can observe it (`docs/spec/render.md` §4).
    pub state_changed_tick: bool,
}

impl Fighter {
    pub fn new(
        char: &Character,
        char_id: usize,
        side: usize,
        slot: usize,
        pos: Vec2,
        facing: i32,
    ) -> Self {
        let life = char
            .constants
            .get_or("data.life", char.constants.get_or("life", 1000.0)) as i32;
        let life = if life <= 0 { 1000 } else { life };
        let guard_max = char
            .constants
            .get("guardpoints")
            .or_else(|| char.constants.get("data.guardpoints"))
            .unwrap_or(0.0)
            .max(0.0) as i32;
        let dizzy_max = char
            .constants
            .get("dizzypoints")
            .or_else(|| char.constants.get("data.dizzypoints"))
            .unwrap_or(0.0)
            .max(0.0) as i32;
        let mut f = Self {
            char_id,
            state_char_id: None,
            side,
            slot,
            active: true,
            ai_level: 0,
            pos,
            vel: Vec2::ZERO,
            facing,
            state_no: 0,
            prev_state_no: 0,
            prev_state_type: StateType::Stand,
            prev_move_type: MoveType::Idle,
            state_time: 0,
            anim_no: 0,
            prev_anim: 0,
            anim_char_id: None,
            anim_clock: 0,
            anim_frame: 0,
            anim_time: 0,
            anim_looped: 0,
            ctrl: true,
            state_type: StateType::Stand,
            move_type: MoveType::Idle,
            physics: Physics::Stand,
            life,
            life_max: life,
            red_life: life,
            guard_points: guard_max,
            guard_max,
            guard_crush_timer: 0,
            guard_break: false,
            dizzy_points: dizzy_max,
            dizzy_max,
            dizzy_timer: 0,
            dizzy_flag: false,
            guard_cooldown: 0,
            power: 0.0,
            defence_mul: 1.0,
            attack_mul: 1.0,
            vars: [0; 60],
            fvars: [0.0; 40],
            sysvars: [0; 5],
            sysfvars: [0.0; 5],
            juggle_points: char.constants.get_or("data.airjuggle", 15.0) as i32,
            juggle: 0,
            target_bind: None,
            fall_defence_mul: 1.0,
            sprite_priority: 0,
            width_back: char.constants.get_or("size.ground.back", 15.0),
            width_front: char.constants.get_or("size.ground.front", 16.0),
            height: char.constants.get_or("size.height", 60.0),
            height_player: Vec2::ZERO,
            shadow_intensity: None,
            shadow_color: None,
            shadow_offset: Vec2::ZERO,
            shadow_xscale: None,
            shadow_yscale: None,
            draw_angle: 0.0,
            angle_draw: false,
            angle_draw_value: None,
            angle_draw_scale: None,
            transform_x_shear: 0.0,
            transform_focal_length: 2048.0,
            transform_projection: crate::ProjectionMode::Orthographic,
            clsn_transforms: [Vec::new(), Vec::new()],
            clsn_overrides: std::array::from_fn(|_| Vec::new()),
            input_shift: Vec::new(),
            reflection_intensity: None,
            reflection_color: None,
            reflection_offset: Vec2::ZERO,
            reflection_xscale: None,
            reflection_yscale: None,
            size_xscale: 1.0,
            size_yscale: 1.0,
            make_dust_spacing: 0,
            width_override: None,
            hitdef: None,
            hit: HitState::default(),
            attack: AttackState::default(),
            history: VecDeque::with_capacity(128),
            ctrl_exec: Vec::new(),
            extra_exec: [Vec::new(), Vec::new(), Vec::new()],
            pos_freeze: 0,
            assert_flags: 0,
            face_p2_pending: false,
            player_push: true,
            gravity: None,
            screen_bound: (true, true),
            draw_offset: Vec2::ZERO,
            hit_overrides: std::array::from_fn(|_| None),
            hit_overridden: false,
            after_image: None,
            after_images: Vec::new(),
            pal_add: [0.0, 0.0, 0.0],
            pal_base_add: [0.0, 0.0, 0.0],
            pal_sinadd: [0.0, 0.0, 0.0],
            pal_sin_period: 0,
            pal_cycle: 0,
            pal_mul: [1.0, 1.0, 1.0],
            pal_base_mul: [1.0, 1.0, 1.0],
            pal_sinmul: [0.0, 0.0, 0.0],
            pal_sinmul_period: 0,
            pal_sinmul_cycle: 0,
            pal_alpha: 1.0,
            pal_time: 0,
            trans_alpha: 1.0,
            trans_alpha_source: 255,
            trans_alpha_dest: 0,
            reversal: None,
            reversal_time: 0,
            owner: None,
            parent: None,
            root: None,
            helper_id: 0,
            helper_name: None,
            helper_remove_time: -1,
            helper_destroy: false,
            keyctrl: 7,
            helper_type_player: false,
            target: None,
            bind_anchor: None,
            bind_time: 0,
            bind_offset: Vec2::ZERO,
            bind_facing: 0,
            win_quote: -1,
            map: BTreeMap::new(),
            pal_remap: None,
            proj_contact_time: -1,
            proj_hit_time: -1,
            proj_guarded_time: -1,
            received_hits: 0,
            received_damage: 0,
            state_changed: false,
            state_changed_tick: false,
        };
        f.enter_state(0, char, false);
        f.ctrl = true;
        f
    }

    pub fn alive(&self) -> bool {
        self.life > 0
    }

    /// Clamps red life into `life..=life_max`, mirroring the controller rule.
    pub fn clamp_red_life(&mut self) {
        if self.life <= 0 {
            self.red_life = 0;
        } else {
            self.red_life = self.red_life.clamp(self.life, self.life_max.max(self.life));
        }
    }

    pub fn is_airborne(&self) -> bool {
        self.pos.y < 0.0 || self.state_type == StateType::Air
    }

    pub fn relative_dir(&self) -> Dir8 {
        self.history.back().map(|h| h.dir).unwrap_or(Dir8::Neutral)
    }

    pub fn held_input(&self) -> PlayerInput {
        self.history.back().map(|h| h.input).unwrap_or_default()
    }

    /// Applies per-tick `ShiftInput` mappings to a raw input frame. Unmapped
    /// keys pass through; a `None` target disables the source key.
    fn apply_input_shift(&self, input: PlayerInput) -> PlayerInput {
        if self.input_shift.is_empty() {
            return input;
        }
        const KEYS: [u8; 11] = [
            DIR_UP,
            DIR_DOWN,
            DIR_LEFT,
            DIR_RIGHT,
            Buttons::A,
            Buttons::B,
            Buttons::C,
            Buttons::X,
            Buttons::Y,
            Buttons::Z,
            Buttons::START,
        ];
        let mut out = PlayerInput::default();
        for (index, mask) in KEYS.iter().enumerate() {
            let active = if index < 4 {
                input.dir & mask != 0
            } else {
                input.buttons & mask != 0
            };
            if !active {
                continue;
            }
            let target = match self
                .input_shift
                .iter()
                .rev()
                .find(|(source, _)| *source == index)
            {
                Some((_, target)) => *target,
                None => Some(index),
            };
            let Some(target) = target else {
                continue;
            };
            if target < 4 {
                out.dir |= KEYS[target];
            } else if target < KEYS.len() {
                out.buttons |= KEYS[target];
            }
        }
        out
    }

    pub fn append_input(&mut self, input: PlayerInput, pause_bits: u8) {
        let input = self.apply_input_shift(input);
        let prev = self.history.back().copied();
        let dir = Dir8::relative(input, self.facing);
        let prev_dir = prev.map(|p| p.dir).unwrap_or(Dir8::Neutral);
        let prev_buttons = prev.map(|p| p.input.buttons).unwrap_or(0);
        let frame = HistoryFrame {
            input,
            dir,
            prev_dir,
            pressed_buttons: input.buttons & !prev_buttons,
            released_buttons: !input.buttons & prev_buttons,
            pause_bits,
        };
        self.history.push_back(frame);
        while self.history.len() > 180 {
            self.history.pop_front();
        }
    }

    pub fn enter_state(&mut self, number: i32, char: &Character, face_p2: bool) {
        let persisted = char.state(number);
        let persist_hitdef = persisted
            .and_then(|state| state.get_i32("hitdefpersist"))
            .unwrap_or(0)
            != 0;
        let persist_movehit = persisted
            .and_then(|state| state.get_i32("movehitpersist"))
            .unwrap_or(0)
            != 0;
        let persist_hitcount = persisted
            .and_then(|state| state.get_i32("hitcountpersist"))
            .unwrap_or(0)
            != 0;
        let previous_attack = self.attack;
        let previous_hitdef = self.hitdef.clone();
        let previous_targets = std::mem::take(&mut self.hit.hit_targets);
        let previous_counts = std::mem::take(&mut self.hit.hit_counts);

        self.prev_state_no = self.state_no;
        self.state_no = number;
        self.state_time = 0;
        self.state_changed = true;
        self.state_changed_tick = true;
        self.ctrl_exec.clear();
        self.attack = AttackState::default();
        self.hitdef = None;
        self.face_p2_pending = false;
        self.gravity = None;
        self.draw_offset = Vec2::ZERO;
        self.hit_overrides = std::array::from_fn(|_| None);
        self.trans_alpha = 1.0;
        self.trans_alpha_source = 255;
        self.trans_alpha_dest = 0;
        self.reversal = None;
        self.reversal_time = 0;
        self.hit.hit_targets.clear();
        self.hit.hit_counts.clear();
        self.hit.pause_gap = 0;
        self.width_override = None;

        if persist_hitdef {
            self.hitdef = previous_hitdef;
            self.hit.hit_targets = previous_targets;
            self.hit.hit_counts = previous_counts;
        }
        if persist_movehit {
            self.attack.move_contact = previous_attack.move_contact;
            self.attack.move_hit = previous_attack.move_hit;
            self.attack.move_guarded = previous_attack.move_guarded;
            self.attack.move_reversed = previous_attack.move_reversed;
        }
        if persist_hitcount {
            self.attack.hit_count = previous_attack.hit_count;
            self.attack.uniq_hit_count = previous_attack.uniq_hit_count;
            self.attack.guard_count = previous_attack.guard_count;
        }

        let _ = face_p2;
        if number == 0 || number == 52 {
            self.juggle_points = char.constants.get_or("data.airjuggle", 15.0) as i32;
        }
        self.juggle = char
            .state(number)
            .and_then(|state| state.get_i32("juggle"))
            .unwrap_or(0);
        if number == 5070 || number == 5100 {
            let up = char.constants.get_or("data.fall.defence_up", 50.0);
            self.fall_defence_mul *= (up + 100.0) / 100.0;
            // One fall per entry into the trip shake or the downed state.
            self.hit.vars.fallcount += 1;
        }
        if let Some(state) = char.state(number) {
            self.apply_state_def(state, char);
        } else if number >= 0 {
            log::warn!("character {} has no statedef {number}", char.name());
        }
        if let Some(state_char) = self.state_char_id {
            self.anim_char_id = Some(state_char);
        }
        if number == 0 {
            self.ctrl = true;
        }
        self.refresh_widths(char);
    }

    pub fn change_state_override(
        &mut self,
        number: i32,
        char: &Character,
        ctrl: Option<bool>,
        anim: Option<i32>,
        velset: Option<Vec2>,
        face_p2: bool,
    ) {
        self.enter_state(number, char, face_p2);
        if let Some(c) = ctrl {
            self.ctrl = c;
        }
        if let Some(a) = anim {
            self.set_anim(a, char, None);
        }
        if let Some(v) = velset {
            self.vel = v;
        }
    }

    fn apply_state_def(&mut self, state: &StateDef, char: &Character) {
        // Statedef defaults per the CNS docs: type = S, movetype = I,
        // physics = N. An explicit "U" means "unchanged", so only absent keys
        // get the default and a present-but-unparsable key keeps the old value.
        self.prev_state_type = self.state_type;
        self.prev_move_type = self.move_type;
        self.state_type = match state.get("type") {
            Some(v) => StateType::parse(v).unwrap_or(self.state_type),
            None => StateType::Stand,
        };
        self.move_type = match state.get("movetype") {
            Some(v) => MoveType::parse(v).unwrap_or(self.move_type),
            None => MoveType::Idle,
        };
        self.physics = match state.get("physics") {
            Some(v) => Physics::parse(v).unwrap_or(self.physics),
            None => Physics::None,
        };
        if let Some(a) = state.get_i32("anim") {
            self.set_anim(a, char, None);
        }
        if let Some(c) = state.get_i32("ctrl") {
            self.ctrl = c != 0;
        }
        if let Some((x, y)) = state.get_pair("velset") {
            self.vel = Vec2::new(x, y);
        }
        if let Some(p) = state.get_f32("poweradd") {
            self.power = (self.power + p).min(3000.0);
        }
        if let Some(p) = state.get_i32("sprpriority") {
            self.sprite_priority = p;
        }
        if state.get_i32("facep2").unwrap_or(0) != 0 {
            self.face_p2_pending = true;
        }
    }

    pub fn set_anim(&mut self, number: i32, char: &Character, elem: Option<i32>) {
        self.set_anim_at(number, char, elem, 0);
    }

    /// `ChangeAnim`/`ChangeAnim2` with Ikemen's `elemtime` offset inside the
    /// selected element. An invalid element starts at the first frame; a
    /// negative or out-of-duration `elemtime` resets to 0.
    pub fn set_anim_at(&mut self, number: i32, char: &Character, elem: Option<i32>, elemtime: i32) {
        self.anim_char_id = None;
        self.prev_anim = self.anim_no;
        self.anim_no = number;
        self.anim_clock = 0;
        self.anim_frame = 0;
        self.anim_time = 0;
        self.anim_looped = 0;
        if let Some(e) = elem {
            if let Some(anim) = char.animation(number) {
                if !anim.frames.is_empty() {
                    let valid_elem = e >= 1 && (e as usize) <= anim.frames.len();
                    let idx = if valid_elem { (e - 1) as usize } else { 0 };
                    let mut offset = if valid_elem { elemtime } else { 0 };
                    let frame = &anim.frames[idx];
                    if offset < 0 || (frame.duration != -1 && offset >= frame.duration) {
                        offset = 0;
                    }
                    let start = anim.frames[..idx]
                        .iter()
                        .fold(0i32, |acc, f| acc.saturating_add(f.effective_duration()));
                    self.anim_frame = idx;
                    self.anim_clock = start.saturating_add(offset);
                    self.anim_time = start.saturating_add(offset);
                }
            }
        }
        if char.animation(number).is_none() && number >= 0 {
            log::debug!("character {} has no animation {number}", char.name());
        }
    }

    pub fn current_anim<'a>(&self, char: &'a Character) -> Option<&'a Animation> {
        char.animation(self.anim_no)
    }

    pub fn current_frame<'a>(&self, char: &'a Character) -> Option<&'a manato_formats::AnimFrame> {
        let anim = self.current_anim(char)?;
        anim.frames
            .get(self.anim_frame.min(anim.frames.len().saturating_sub(1)))
    }

    /// `AnimElem` trigger (1-based).
    pub fn anim_elem(&self) -> i32 {
        self.anim_frame as i32 + 1
    }

    /// `AnimTime` trigger: ticks remaining in the whole animation. A finite
    /// animation reports 0 for exactly one tick before it loops.
    pub fn anim_time_remaining(&self, char: &Character) -> i32 {
        let Some(anim) = self.current_anim(char) else {
            return 0;
        };
        if anim.is_infinite() {
            return -1;
        }
        (anim.total_time() - self.anim_time).max(0)
    }

    pub fn anim_elem_time(&self, char: &Character, elem: i32) -> i32 {
        let Some(anim) = self.current_anim(char) else {
            return -1000000;
        };
        if elem <= 0 || anim.frames.is_empty() || elem as usize > anim.frames.len() {
            return -1000000;
        }
        let idx = elem as usize - 1;
        let start = anim.frames[..idx]
            .iter()
            .fold(0i32, |acc, f| acc.saturating_add(f.effective_duration()));
        self.anim_time - start
    }

    /// `ChangeAnim2`: use another character's animation bank for `number`.
    pub fn set_anim_from(
        &mut self,
        number: i32,
        anim_char: &Character,
        elem: Option<i32>,
        anim_char_id: usize,
    ) {
        self.set_anim_from_at(number, anim_char, elem, 0, anim_char_id);
    }

    pub fn set_anim_from_at(
        &mut self,
        number: i32,
        anim_char: &Character,
        elem: Option<i32>,
        elemtime: i32,
        anim_char_id: usize,
    ) {
        self.set_anim_at(number, anim_char, elem, elemtime);
        self.anim_char_id = Some(anim_char_id);
    }

    pub fn advance_anim(&mut self, char: &Character) {
        let Some(anim) = char.animation(self.anim_no) else {
            self.anim_clock = self.anim_clock.saturating_add(1);
            return;
        };
        if anim.frames.is_empty() {
            return;
        }
        self.anim_time = self.anim_time.saturating_add(1);
        self.anim_clock = self.anim_time;
        if anim.is_infinite() {
            if let Some((idx, _)) = anim.frame_at(self.anim_time) {
                self.anim_frame = idx;
            }
            return;
        }
        let total = anim.total_time();
        if total <= 0 {
            self.anim_frame = 0;
            return;
        }
        if self.anim_time > total {
            // The extra tick at `total` is where `AnimTime = 0` fires.
            self.anim_time %= total;
            self.anim_clock = self.anim_time;
            self.anim_looped += 1;
        }
        if self.anim_time >= total {
            self.anim_frame = anim.frames.len() - 1;
            return;
        }
        if let Some((idx, _)) = anim.frame_at(self.anim_time) {
            self.anim_frame = idx;
        }
    }

    pub fn refresh_widths(&mut self, char: &Character) {
        if let Some((back, front)) = self.width_override {
            self.width_back = back;
            self.width_front = front;
            return;
        }
        if self.state_type == StateType::Air {
            self.width_back = char.constants.get_or("size.air.back", 12.0);
            self.width_front = char.constants.get_or("size.air.front", 12.0);
        } else {
            self.width_back = char.constants.get_or("size.ground.back", 15.0);
            self.width_front = char.constants.get_or("size.ground.front", 16.0);
        }
    }

    pub fn push_box(&self) -> Rect {
        let (back, front) = if self.facing >= 0 {
            (self.width_back, self.width_front)
        } else {
            (self.width_front, self.width_back)
        };
        Rect::new(
            (self.pos.x - back).round() as i32,
            (self.pos.y - self.height).round() as i32,
            (self.pos.x + front).round() as i32,
            self.pos.y.round() as i32,
        )
    }

    /// Local collision boxes for `group` (0 = Clsn1, 1 = Clsn2) after
    /// `OverrideClsn` entries. `TransformClsn` scaling is applied later by
    /// [`Self::world_clsn_box`] or by the `ClsnVar` trigger.
    pub fn local_clsn_boxes(
        &self,
        char: &Character,
        group: usize,
    ) -> Vec<manato_formats::CollisionBox> {
        let attack = group == 0;
        let mut boxes: Vec<manato_formats::CollisionBox> = self
            .current_frame(char)
            .map(|frame| {
                if attack {
                    frame.clsn1.clone()
                } else {
                    frame.clsn2.clone()
                }
            })
            .unwrap_or_default();
        for entry in &self.clsn_overrides[group] {
            // An all-zero rect deletes boxes.
            if entry.rect.iter().all(|value| *value == 0.0) {
                if entry.index < 0 {
                    boxes.clear();
                } else if (entry.index as usize) < boxes.len() {
                    boxes.remove(entry.index as usize);
                }
                continue;
            }
            let rect = Rect::new(
                entry.rect[0].round() as i32,
                entry.rect[1].round() as i32,
                entry.rect[2].round() as i32,
                entry.rect[3].round() as i32,
            )
            .normalized();
            let bx = manato_formats::CollisionBox {
                x1: rect.x1,
                y1: rect.y1,
                x2: rect.x2,
                y2: rect.y2,
            };
            if entry.index < 0 {
                boxes.fill(bx);
            } else if (entry.index as usize) >= boxes.len() {
                boxes.push(bx);
            } else {
                boxes[entry.index as usize] = bx;
            }
        }
        boxes
    }

    /// Converts an animation-local collision box to world space.
    pub fn world_box(&self, bx: &manato_formats::CollisionBox) -> Rect {
        let mut r = bx.rect();
        if self.facing < 0 {
            r = r.mirror_x();
        }
        r.translate(self.pos.x.round() as i32, self.pos.y.round() as i32)
    }

    /// World-space collision box after per-tick `TransformClsn` scaling.
    /// `group` is 0 for Clsn1 and 1 for Clsn2.
    pub fn world_clsn_box(
        &self,
        bx: &manato_formats::CollisionBox,
        group: usize,
        index: usize,
    ) -> Rect {
        let mut scale = Vec2::new(1.0, 1.0);
        for transform in &self.clsn_transforms[group] {
            if transform.index < 0 || transform.index as usize == index {
                scale.x *= transform.scale.x;
                scale.y *= transform.scale.y;
            }
        }
        if scale == Vec2::new(1.0, 1.0) {
            return self.world_box(bx);
        }
        let scaled = manato_formats::CollisionBox {
            x1: (bx.x1 as f32 * scale.x).round() as i32,
            y1: (bx.y1 as f32 * scale.y).round() as i32,
            x2: (bx.x2 as f32 * scale.x).round() as i32,
            y2: (bx.y2 as f32 * scale.y).round() as i32,
        };
        self.world_box(&scaled)
    }

    pub fn hit_pause(&self) -> i32 {
        self.hit.hit_pause.max(self.hit.super_pause).max(0)
    }

    pub fn command_active(&self, name: &str) -> bool {
        let lower = name.trim().to_ascii_lowercase();
        match lower.as_str() {
            "holdfwd" => return self.relative_dir().is_forward(),
            "holdback" => return self.relative_dir().is_back(),
            "holdup" => return self.relative_dir().is_up(),
            "holddown" => return self.relative_dir().is_down(),
            _ => {}
        }
        if self.history.is_empty() {
            return false;
        }
        // Commands are stored on the character, not the fighter. The caller
        // (SimState) passes the evaluated result of all matching definitions,
        // so this method only covers pseudo-commands.
        false
    }

    /// First active `HitOverride` slot matching the incoming attack.
    pub fn matched_override(&self, hitdef: &HitDef) -> Option<HitOverride> {
        self.hit_overrides
            .iter()
            .flatten()
            .find(|override_| {
                override_.time != 0
                    && crate::collision::override_attr_matches(&override_.attr, &hitdef.attr)
            })
            .cloned()
    }

    pub fn match_command(&self, def: &CommandDef) -> bool {
        if def.steps.is_empty() || self.history.is_empty() {
            return false;
        }
        let n = self.history.len();
        let buffer = def.buffer_time.max(0) as usize;
        let window = def.time.max(1) as usize + 2;
        let window_start = n.saturating_sub(window + buffer + 1);
        let last_step = *def.steps.last().unwrap();

        // The final step may be buffered for `buffer.time` ticks.
        let search_end = n.saturating_sub(1);
        let search_start = n.saturating_sub(1 + buffer);
        let greater = def.auto_greater;
        let frame_usable = |frame: &HistoryFrame| {
            (frame.pause_bits & 1 == 0 || def.buffer_hitpause)
                && (frame.pause_bits & 2 == 0 || def.buffer_pauseend)
        };
        for last_idx in (search_start..=search_end).rev() {
            if !frame_usable(self.history.get(last_idx).unwrap()) {
                continue;
            }
            if !step_matches(
                self.history.get(last_idx).unwrap(),
                last_step,
                true,
                greater,
            ) || !self.charge_ok(last_idx, last_step, greater)
            {
                continue;
            }
            let mut cursor = last_idx;
            let mut ok = true;
            let mut first = last_idx;
            for step in def.steps[..def.steps.len() - 1].iter().rev() {
                // Hold / release / pooled direction steps may share the frame
                // with the following input; taps must be strictly earlier.
                let start = if step.hold || step.pool4 || step.release {
                    Some(cursor)
                } else {
                    cursor.checked_sub(1)
                };
                let Some(start) = start else {
                    ok = false;
                    break;
                };
                let mut found = None;
                for j in (window_start..=start).rev() {
                    if !frame_usable(self.history.get(j).unwrap()) {
                        continue;
                    }
                    if !step_matches(self.history.get(j).unwrap(), *step, !step.hold, greater)
                        || !self.charge_ok(j, *step, greater)
                    {
                        continue;
                    }
                    if def.step_time >= 0
                        && cursor.saturating_sub(j) > def.step_time.max(0) as usize
                    {
                        continue;
                    }
                    found = Some(j);
                    break;
                }
                match found {
                    Some(j) => {
                        cursor = j;
                        first = j;
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && last_idx.saturating_sub(first) <= def.time.max(1) as usize {
                return true;
            }
        }
        false
    }
}

impl Fighter {
    /// True when a `/` or `~` charge prefix was held for its full duration.
    fn charge_ok(&self, end: usize, step: CommandStep, greater: bool) -> bool {
        let ticks = step.charge_time.max(0) as usize;
        if ticks == 0 {
            return true;
        }
        let Some(dir) = step.dir else {
            return true;
        };
        if end < ticks {
            return false;
        }
        (end - ticks..end).all(|index| {
            self.history
                .get(index)
                .is_some_and(|frame| dir_matches(frame.dir, dir, step.pool4 && !step.star, greater))
        })
    }
}

fn dir_matches(frame_dir: Dir8, want: Dir8, pool4: bool, greater: bool) -> bool {
    if frame_dir == want {
        return true;
    }
    if greater {
        // `>` (and `autogreater`) accepts any diagonal containing the wanted
        // cardinal direction.
        return matches!(
            (want, frame_dir),
            (Dir8::Down, Dir8::DnFwd | Dir8::DnBack)
                | (Dir8::Fwd, Dir8::UpFwd | Dir8::DnFwd)
                | (Dir8::Back, Dir8::UpBack | Dir8::DnBack)
                | (Dir8::Up, Dir8::UpFwd | Dir8::UpBack)
        );
    }
    if !pool4 {
        return false;
    }
    matches!(
        (want, frame_dir),
        (Dir8::Down, Dir8::DnFwd | Dir8::DnBack)
            | (Dir8::Fwd, Dir8::UpFwd | Dir8::DnFwd)
            | (Dir8::Back, Dir8::UpBack | Dir8::DnBack)
            | (Dir8::Up, Dir8::UpFwd | Dir8::UpBack)
    )
}

fn step_matches(
    frame: &HistoryFrame,
    step: CommandStep,
    require_press: bool,
    greater: bool,
) -> bool {
    if let Some(dir) = step.dir {
        let matches =
            |d: Dir8| dir_matches(d, dir, step.pool4 && !step.star, greater || step.greater);
        if step.release {
            if !matches(frame.prev_dir) || matches(frame.dir) {
                return false;
            }
        } else if step.hold {
            if !matches(frame.dir) {
                return false;
            }
        } else if require_press {
            if !matches(frame.dir) || matches(frame.prev_dir) {
                return false;
            }
        } else if !matches(frame.dir) {
            return false;
        }
    }
    if step.buttons != 0 {
        if step.button_release {
            if frame.released_buttons & step.buttons != step.buttons {
                return false;
            }
        } else if frame.pressed_buttons & step.buttons != step.buttons {
            return false;
        }
    }
    if step.or_buttons != 0 {
        if step.button_release {
            if frame.released_buttons & step.or_buttons == 0 {
                return false;
            }
        } else if frame.pressed_buttons & step.or_buttons == 0 {
            return false;
        }
    }
    true
}

/// Convenience: is this input pressing any of the given buttons?
pub fn pressed(input: PlayerInput, prev: PlayerInput, buttons: u8) -> bool {
    input.buttons & buttons != 0 && prev.buttons & buttons == 0
}

pub fn buttons_mask(buttons: &[u8]) -> u8 {
    buttons.iter().fold(0, |a, b| a | b)
}

pub const ALL_BUTTONS: u8 = Buttons::ALL;

/// `AssertSpecial` flags.
pub const ASSERT_INVISIBLE: u32 = 1 << 0;
pub const ASSERT_NOSHADOW: u32 = 1 << 1;
pub const ASSERT_NOWALK: u32 = 1 << 2;
pub const ASSERT_NOAIRGUARD: u32 = 1 << 3;
pub const ASSERT_NOSTANDGUARD: u32 = 1 << 4;
pub const ASSERT_NOCROUCHGUARD: u32 = 1 << 5;
pub const ASSERT_UNGUARDABLE: u32 = 1 << 6;
pub const ASSERT_NOJUGGLECHECK: u32 = 1 << 7;
pub const ASSERT_NOAUTOTURN: u32 = 1 << 8;
pub const ASSERT_INTRO: u32 = 1 << 9;
pub const ASSERT_NOBG: u32 = 1 << 10;
pub const ASSERT_NOFG: u32 = 1 << 11;
pub const ASSERT_NOKO: u32 = 1 << 12;
pub const ASSERT_NOKOSLOW: u32 = 1 << 13;
pub const ASSERT_NOKOSND: u32 = 1 << 14;
pub const ASSERT_NOMUSIC: u32 = 1 << 15;
pub const ASSERT_GLOBALNOSHADOW: u32 = 1 << 16;
pub const ASSERT_NOBARDISPLAY: u32 = 1 << 17;
pub const ASSERT_ROUNDNOTOVER: u32 = 1 << 18;
pub const ASSERT_NOHARDCODEDKEYS: u32 = 1 << 19;

/// Maps an `AssertSpecial`/`IsAsserted` flag name to its bit.
pub fn assert_flag(name: &str) -> u32 {
    match name {
        "invisible" => ASSERT_INVISIBLE,
        "noshadow" => ASSERT_NOSHADOW,
        "nowalk" => ASSERT_NOWALK,
        "noairguard" => ASSERT_NOAIRGUARD,
        "nostandguard" => ASSERT_NOSTANDGUARD,
        "nocrouchguard" => ASSERT_NOCROUCHGUARD,
        "unguardable" => ASSERT_UNGUARDABLE,
        "nojugglecheck" => ASSERT_NOJUGGLECHECK,
        "noautoturn" => ASSERT_NOAUTOTURN,
        "intro" => ASSERT_INTRO,
        "nobg" => ASSERT_NOBG,
        "nofg" => ASSERT_NOFG,
        "noko" => ASSERT_NOKO,
        "nokoslow" => ASSERT_NOKOSLOW,
        "nokosnd" => ASSERT_NOKOSND,
        "nomusic" => ASSERT_NOMUSIC,
        "globalnoshadow" => ASSERT_GLOBALNOSHADOW,
        "nobardisplay" => ASSERT_NOBARDISPLAY,
        "roundnotover" => ASSERT_ROUNDNOTOVER,
        "nohardcodedkeys" => ASSERT_NOHARDCODEDKEYS,
        _ => 0,
    }
}
