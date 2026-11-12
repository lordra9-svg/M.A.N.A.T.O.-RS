use manato_core::{Rect, Vec2};
use manato_formats::Controller;

use crate::expr::{Expr, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimType {
    #[default]
    Light,
    Medium,
    Heavy,
    Back,
    Up,
    DiagUp,
}

impl AnimType {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "medium" => Self::Medium,
            "hard" | "heavy" => Self::Heavy,
            "back" => Self::Back,
            "up" => Self::Up,
            "diagup" => Self::DiagUp,
            _ => Self::Light,
        }
    }

    pub fn code(self) -> i32 {
        match self {
            Self::Light => 0,
            Self::Medium => 1,
            Self::Heavy => 2,
            Self::Back => 3,
            Self::Up => 4,
            Self::DiagUp => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroundType {
    None,
    #[default]
    High,
    Low,
    Trip,
}

impl GroundType {
    pub fn parse(s: &str) -> Self {
        // M.U.G.E.N documents the full words; Ikemen-GO also accepts the
        // first letter for CNS content. Unknown values fall back to High.
        let value = s.trim().to_ascii_lowercase();
        match value.as_str() {
            "none" | "n" => Self::None,
            "low" | "l" => Self::Low,
            "trip" | "t" => Self::Trip,
            _ => Self::High,
        }
    }

    /// Values reported by `GetHitVar(type|groundtype|airtype)`.
    pub fn code(self) -> i32 {
        match self {
            Self::None => 0,
            Self::High => 1,
            Self::Low => 2,
            Self::Trip => 3,
        }
    }
}

/// `priority = N[, Hit|Miss|Dodge]` clash type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PriorityType {
    #[default]
    Hit,
    Miss,
    Dodge,
}

impl PriorityType {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "miss" => Self::Miss,
            "dodge" => Self::Dodge,
            _ => Self::Hit,
        }
    }
}

/// Attack attributes (`attr = S, NA`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HitAttr {
    pub state_types: String,
    pub class: String,
}

impl HitAttr {
    pub fn parse(value: &str) -> Self {
        let mut parts = value.split(',');
        let state_types = parts.next().unwrap_or("").trim().to_ascii_uppercase();
        let class = parts.next().unwrap_or("").trim().to_ascii_uppercase();
        Self { state_types, class }
    }

    pub fn state_type_matches(&self, state: &str) -> bool {
        self.state_types.contains(state)
    }

    pub fn class_matches(&self, class: &str) -> bool {
        self.class.contains(class)
    }
}

/// A parsed `HitDef` controller. The `activation` counter distinguishes two
/// consecutive uses of the same attack so `hitonce` is evaluated per swing.
#[derive(Debug, Clone, PartialEq)]
pub struct HitDef {
    pub attr: HitAttr,
    pub damage: i32,
    pub guard_damage: i32,
    /// True when the content explicitly supplied a guard-damage value.
    pub guard_damage_set: bool,
    pub anim_type: AnimType,
    pub hitflag: String,
    pub guardflag: String,
    pub ground_type: GroundType,
    pub air_type: GroundType,
    pub ground_velocity: Vec2,
    pub air_velocity: Vec2,
    /// Velocity applied when hitting a lying-down opponent (`down.velocity`).
    pub down_velocity: Vec2,
    /// Vertical acceleration imparted to the defender (`yaccel`).
    pub yaccel: f32,
    /// `guard.velocity`; falls back to `ground.velocity` when absent.
    pub guard_velocity: Option<Vec2>,
    pub airguard_velocity: Vec2,
    pub ground_hittime: i32,
    pub air_hittime: i32,
    pub ground_slidetime: i32,
    pub guard_hittime: i32,
    /// X distance within which the defender enters guard (`guard.dist`).
    pub guard_dist: i32,
    /// Guard slide time; defaults to `guard_hittime` when omitted.
    pub guard_slidetime: i32,
    /// Time before control returns after a ground guard; defaults to
    /// `guard_slidetime`.
    pub guard_ctrltime: i32,
    /// Time before control returns after an air guard; defaults to
    /// `guard_ctrltime`.
    pub airguard_ctrltime: i32,
    pub pausetime: (i32, i32),
    /// Guarded-contact pause; defaults to `pausetime`.
    pub guard_pausetime: (i32, i32),
    /// True when the content explicitly supplied `guard.pausetime`.
    pub guard_pausetime_set: bool,
    pub sparkno: i32,
    pub guard_sparkno: i32,
    pub sparkxy: Vec2,
    pub hitsound: (i32, i32),
    pub guardsound: (i32, i32),
    pub hitonce: bool,
    /// Whether a successful hit may KO (`kill`).
    pub kill: bool,
    /// Whether a guarded hit may KO (`guard.kill`).
    pub guard_kill: bool,
    pub fall: bool,
    /// Whether an airborne hit enters the fall reaction (`air.fall`).
    pub air_fall: bool,
    /// Forces a crouching grounded defender into the standing hit reaction.
    pub forcestand: bool,
    /// Forces the defender out of an existing fall when the hit itself is not
    /// a fall (`forcenofall`).
    pub forcenofall: bool,
    pub fall_recover: bool,
    pub fall_time: i32,
    /// Time before a fall can be recovered (`fall.recovertime`).
    pub fall_recovertime: i32,
    /// Whether fall damage may KO (`fall.kill`).
    pub fall_kill: bool,
    /// Fall-impact screen shake (`fall.envshake.*`).
    pub fall_envshake_time: i32,
    pub fall_envshake_freq: f32,
    pub fall_envshake_ampl: i32,
    /// `NaN` means "derive the phase from the frequency".
    pub fall_envshake_phase: f32,
    /// Successful-hit target palette effect (`palfx.*`).
    pub palfx_time: i32,
    pub palfx_mul: [f32; 3],
    pub palfx_add: [f32; 3],
    /// Successful-hit screen shake (`envshake.*`).
    pub envshake_time: i32,
    pub envshake_freq: f32,
    pub envshake_ampl: i32,
    /// `NaN` means "derive the phase from the frequency".
    pub envshake_phase: f32,
    pub fall_damage: i32,
    pub fall_xvel: f32,
    pub fall_yvel: f32,
    pub air_juggle: i32,
    pub priority: i32,
    pub priority_type: PriorityType,
    pub numhits: i32,
    pub p1stateno: i32,
    pub p2stateno: i32,
    pub p1facing: i32,
    pub p2facing: i32,
    /// `1`/`-1` makes P1 copy P2 facing or its opposite; overrides
    /// `p1facing` when non-zero.
    pub p1getp2facing: i32,
    /// Post-hit minimum distance from the attacker (`mindist`), axis NaN
    /// when unset.
    pub mindist: Vec2,
    /// Post-hit maximum distance from the attacker (`maxdist`), axis NaN
    /// when unset.
    pub maxdist: Vec2,
    /// Exact post-hit position relative to the attacker (`snap`), axis NaN
    /// when unset.
    pub snap: Vec2,
    pub p1sprpriority: i32,
    pub p1sprpriority_set: bool,
    pub p2sprpriority: i32,
    pub p2sprpriority_set: bool,
    pub ground_cornerpush: f32,
    pub air_cornerpush: f32,
    /// Corner push applied when the hit is guarded.
    pub guard_cornerpush: f32,
    /// Corner push applied when an air hit is guarded.
    pub airguard_cornerpush: f32,
    /// Corner push applied when hitting a grounded lying-down defender.
    pub down_cornerpush: f32,
    pub activation: u32,
}

impl Default for HitDef {
    fn default() -> Self {
        Self {
            attr: HitAttr::default(),
            damage: 0,
            guard_damage: 0,
            guard_damage_set: false,
            anim_type: AnimType::Light,
            hitflag: "MAF".into(),
            guardflag: "MA".into(),
            ground_type: GroundType::High,
            air_type: GroundType::High,
            ground_velocity: Vec2::ZERO,
            air_velocity: Vec2::ZERO,
            down_velocity: Vec2::ZERO,
            yaccel: 0.35,
            guard_velocity: None,
            airguard_velocity: Vec2::ZERO,
            ground_hittime: 0,
            air_hittime: 20,
            ground_slidetime: 0,
            guard_hittime: 0,
            guard_dist: 160,
            guard_slidetime: 0,
            guard_ctrltime: 0,
            airguard_ctrltime: 0,
            pausetime: (0, 0),
            guard_pausetime: (0, 0),
            guard_pausetime_set: false,
            sparkno: -1,
            guard_sparkno: 40,
            sparkxy: Vec2::new(-10.0, -55.0),
            hitsound: (-1, -1),
            guardsound: (-1, -1),
            hitonce: false,
            kill: true,
            guard_kill: true,
            fall: false,
            air_fall: false,
            forcestand: false,
            forcenofall: false,
            fall_recover: false,
            fall_time: 0,
            fall_recovertime: 4,
            fall_kill: true,
            fall_envshake_time: 0,
            fall_envshake_freq: 60.0,
            fall_envshake_ampl: -4,
            fall_envshake_phase: f32::NAN,
            envshake_time: 0,
            envshake_freq: 60.0,
            envshake_ampl: -4,
            envshake_phase: f32::NAN,
            palfx_time: 0,
            palfx_mul: [1.0; 3],
            palfx_add: [0.0; 3],
            fall_damage: 0,
            fall_xvel: 0.0,
            fall_yvel: 0.0,
            air_juggle: 0,
            priority: 4,
            priority_type: PriorityType::Hit,
            numhits: 1,
            p1stateno: -1,
            p2stateno: -1,
            p1facing: 0,
            p2facing: 0,
            p1getp2facing: 0,
            p1sprpriority: 0,
            p1sprpriority_set: false,
            mindist: Vec2::new(f32::NAN, f32::NAN),
            maxdist: Vec2::new(f32::NAN, f32::NAN),
            snap: Vec2::new(f32::NAN, f32::NAN),
            p2sprpriority: 0,
            p2sprpriority_set: false,
            ground_cornerpush: 0.0,
            air_cornerpush: 0.0,
            guard_cornerpush: 0.0,
            airguard_cornerpush: 0.0,
            down_cornerpush: 0.0,
            activation: 0,
        }
    }
}

impl HitDef {
    pub fn from_controller(c: &Controller, activation: u32) -> Self {
        Self::from_controller_for(c, activation, (320.0, 240.0), 160)
    }

    /// Parses a HitDef with the owning character's local coordinate space so
    /// omitted `yaccel` follows the documented 240p/480p/720p defaults and
    /// `guard.dist` uses the character's `attack.dist`/`proj.attack.dist`.
    pub fn from_controller_for(
        c: &Controller,
        activation: u32,
        localcoord: (f32, f32),
        default_guard_dist: i32,
    ) -> Self {
        let default_yaccel = if localcoord.0 > 0.0 {
            0.35 * localcoord.0 / 320.0
        } else {
            0.35
        };
        let mut h = Self {
            activation,
            yaccel: default_yaccel,
            guard_dist: default_guard_dist.max(0),
            ..Self::default()
        };
        let mut air_type_set = false;
        let mut airguard_velocity_set = false;
        let mut down_velocity_set = false;
        let mut air_fall_set = false;
        let mut forcestand_set = false;
        let mut guard_hittime_set = false;
        let mut guard_slidetime_set = false;
        let mut guard_ctrltime_set = false;
        let mut airguard_ctrltime_set = false;
        let mut ground_cornerpush_set = false;
        let mut air_cornerpush_set = false;
        let mut guard_cornerpush_set = false;
        let mut airguard_cornerpush_set = false;
        let mut down_cornerpush_set = false;
        let mut guard_pausetime_set = false;
        for (key, value) in &c.params {
            match key.as_str() {
                "attr" => h.attr = HitAttr::parse(value),
                "damage" => {
                    let (dmg, guard) = parse_pair(value);
                    h.damage = dmg as i32;
                    h.guard_damage = guard as i32;
                    h.guard_damage_set = true;
                }
                "guard.damage" => {
                    h.guard_damage = parse_i32(value);
                    h.guard_damage_set = true;
                }
                "animtype" => h.anim_type = AnimType::parse(value),
                "hitflag" => h.hitflag = value.to_ascii_uppercase(),
                "guardflag" => h.guardflag = value.to_ascii_uppercase(),
                "ground.type" => h.ground_type = GroundType::parse(value),
                "air.type" => {
                    h.air_type = GroundType::parse(value);
                    air_type_set = true;
                }
                "ground.velocity" => h.ground_velocity = parse_vec(value),
                "air.velocity" => h.air_velocity = parse_vec(value),
                "down.velocity" => {
                    h.down_velocity = parse_vec(value);
                    down_velocity_set = true;
                }
                "yaccel" => h.yaccel = parse_f32(value),
                "guard.velocity" => h.guard_velocity = Some(parse_vec(value)),
                "airguard.velocity" => {
                    h.airguard_velocity = parse_vec(value);
                    airguard_velocity_set = true;
                }
                "ground.hittime" => h.ground_hittime = parse_i32(value),
                "air.hittime" => h.air_hittime = parse_i32(value),
                "ground.slidetime" => h.ground_slidetime = parse_i32(value),
                "guard.hittime" => {
                    h.guard_hittime = parse_i32(value);
                    guard_hittime_set = true;
                }
                "guard.dist" | "guard.dist.width" => {
                    h.guard_dist = parse_pair(value).0 as i32;
                }
                "guard.slidetime" => {
                    h.guard_slidetime = parse_i32(value);
                    guard_slidetime_set = true;
                }
                "guard.ctrltime" => {
                    h.guard_ctrltime = parse_i32(value);
                    guard_ctrltime_set = true;
                }
                "airguard.ctrltime" => {
                    h.airguard_ctrltime = parse_i32(value);
                    airguard_ctrltime_set = true;
                }
                "pausetime" => {
                    let (a, b) = parse_pair(value);
                    h.pausetime = (a as i32, b as i32);
                }
                "guard.pausetime" => {
                    let (a, b) = parse_pair(value);
                    h.guard_pausetime = (a as i32, b as i32);
                    h.guard_pausetime_set = true;
                    guard_pausetime_set = true;
                }
                "sparkno" => h.sparkno = parse_i32(value),
                "guard.sparkno" => h.guard_sparkno = parse_i32(value),
                "sparkxy" => h.sparkxy = parse_vec(value),
                "hitsound" => h.hitsound = parse_sound_pair(value),
                "guardsound" => h.guardsound = parse_sound_pair(value),
                "hitonce" => h.hitonce = parse_i32(value) != 0,
                "kill" => h.kill = parse_i32(value) != 0,
                "guard.kill" => h.guard_kill = parse_i32(value) != 0,
                "fall" => h.fall = parse_i32(value) != 0,
                "air.fall" => {
                    h.air_fall = parse_i32(value) != 0;
                    air_fall_set = true;
                }
                "forcestand" => {
                    h.forcestand = parse_i32(value) != 0;
                    forcestand_set = true;
                }
                "forcenofall" => h.forcenofall = parse_i32(value) != 0,
                "fall.recover" => h.fall_recover = parse_i32(value) != 0,
                "fall.time" => h.fall_time = parse_i32(value),
                "fall.recovertime" => h.fall_recovertime = parse_i32(value),
                "fall.kill" => h.fall_kill = parse_i32(value) != 0,
                "fall.envshake.time" => h.fall_envshake_time = parse_i32(value),
                "fall.envshake.freq" => h.fall_envshake_freq = parse_f32(value),
                "fall.envshake.ampl" => h.fall_envshake_ampl = parse_i32(value),
                "fall.envshake.phase" => h.fall_envshake_phase = parse_f32(value),
                "envshake.time" => h.envshake_time = parse_i32(value),
                "envshake.freq" => h.envshake_freq = parse_f32(value).max(0.0),
                "envshake.ampl" => h.envshake_ampl = parse_i32(value),
                "envshake.phase" => h.envshake_phase = parse_f32(value),
                "palfx.time" => h.palfx_time = parse_i32(value),
                "palfx.mul" => h.palfx_mul = parse_triplet(value),
                "palfx.add" => h.palfx_add = parse_triplet(value),
                "fall.damage" => h.fall_damage = parse_i32(value),
                "fall.xvelocity" => h.fall_xvel = parse_f32(value),
                "fall.yvelocity" => h.fall_yvel = parse_f32(value),
                "air.juggle" => h.air_juggle = parse_i32(value),
                "juggle" => h.air_juggle = h.air_juggle.max(parse_i32(value)),
                "priority" => {
                    let mut parts = value.split(',');
                    h.priority = parts
                        .next()
                        .and_then(|part| part.trim().parse().ok())
                        .unwrap_or(0);
                    h.priority_type = parts
                        .next()
                        .map(PriorityType::parse)
                        .unwrap_or(PriorityType::Hit);
                }
                "numhits" | "hits" => h.numhits = parse_i32(value).max(1),
                "p1stateno" => h.p1stateno = parse_i32(value),
                "p2stateno" => h.p2stateno = parse_i32(value),
                "p1facing" => h.p1facing = parse_i32(value),
                "p2facing" => h.p2facing = parse_i32(value),
                "p1getp2facing" => h.p1getp2facing = parse_i32(value),
                "p1sprpriority" => {
                    h.p1sprpriority = parse_i32(value);
                    h.p1sprpriority_set = true;
                }
                "p2sprpriority" => {
                    h.p2sprpriority = parse_i32(value);
                    h.p2sprpriority_set = true;
                }
                "mindist" => h.mindist = parse_distance(value),
                "maxdist" => h.maxdist = parse_distance(value),
                "snap" => h.snap = parse_distance(value),
                "ground.cornerpush.veloff" => {
                    h.ground_cornerpush = parse_f32(value);
                    ground_cornerpush_set = true;
                }
                "air.cornerpush.veloff" => {
                    h.air_cornerpush = parse_f32(value);
                    air_cornerpush_set = true;
                }
                "guard.cornerpush.veloff" => {
                    h.guard_cornerpush = parse_f32(value);
                    guard_cornerpush_set = true;
                }
                "airguard.cornerpush.veloff" => {
                    h.airguard_cornerpush = parse_f32(value);
                    airguard_cornerpush_set = true;
                }
                "down.cornerpush.veloff" => {
                    h.down_cornerpush = parse_f32(value);
                    down_cornerpush_set = true;
                }
                _ => {}
            }
        }
        // Documented inheritance: air.type follows ground.type, guard.hittime
        // follows ground.hittime, and guard.slidetime follows guard.hittime.
        if !air_type_set {
            h.air_type = h.ground_type;
        }
        if !air_fall_set {
            h.air_fall = h.fall;
        }
        // A non-zero ground Y velocity knocks the defender into the air, so
        // it defaults forcestand to on.
        if !forcestand_set {
            h.forcestand = h.ground_velocity.y != 0.0;
        }
        // Documented default: air guard velocity is 1.5x the air X velocity
        // and half the air Y velocity.
        if !airguard_velocity_set {
            h.airguard_velocity = Vec2::new(h.air_velocity.x * 1.5, h.air_velocity.y * 0.5);
        }
        if !down_velocity_set {
            h.down_velocity = h.air_velocity;
        }
        if !guard_hittime_set {
            h.guard_hittime = h.ground_hittime;
        }
        if !guard_slidetime_set {
            h.guard_slidetime = h.guard_hittime;
        }
        if !guard_ctrltime_set {
            h.guard_ctrltime = h.guard_slidetime;
        }
        if !airguard_ctrltime_set {
            h.airguard_ctrltime = h.guard_ctrltime;
        }
        // Documented legacy corner-push defaults: air attacks do not push,
        // otherwise 1.3x the effective guard velocity; air corner push
        // inherits the ground value.
        if !ground_cornerpush_set {
            h.ground_cornerpush = if h.attr.state_types.contains('A') {
                0.0
            } else {
                h.guard_velocity.unwrap_or(h.ground_velocity).x * 1.3
            };
        }
        if !air_cornerpush_set {
            h.air_cornerpush = h.ground_cornerpush;
        }
        if !guard_cornerpush_set {
            h.guard_cornerpush = h.ground_cornerpush;
        }
        if !airguard_cornerpush_set {
            h.airguard_cornerpush = h.guard_cornerpush;
        }
        if !down_cornerpush_set {
            h.down_cornerpush = h.ground_cornerpush;
        }
        if !guard_pausetime_set {
            h.guard_pausetime = h.pausetime;
        }
        // `snap` is implemented as an exact minimum/maximum on each set axis.
        if !h.snap.x.is_nan() {
            h.mindist.x = h.snap.x;
            h.maxdist.x = h.snap.x;
        }
        if !h.snap.y.is_nan() {
            h.mindist.y = h.snap.y;
            h.maxdist.y = h.snap.y;
        }
        h
    }

    pub fn can_hit_air(&self) -> bool {
        self.hitflag.contains('A')
    }

    pub fn can_guard_air(&self) -> bool {
        self.guardflag.contains('A')
    }

    pub fn hit_clsn_area(&self) -> Rect {
        Rect::default()
    }
}

/// Values exposed through `GetHitVar(...)`.
#[derive(Debug, Clone, PartialEq)]
pub struct GetHitVars {
    pub anim_type: i32,
    pub ground_type: i32,
    pub air_type: i32,
    /// Selected hit type for this contact: air type when the defender was
    /// airborne, otherwise the ground type.
    pub hit_type: i32,
    pub x_vel: f32,
    pub y_vel: f32,
    pub y_accel: f32,
    pub hittime: i32,
    pub slidetime: i32,
    pub ctrltime: i32,
    pub fall: bool,
    pub fall_recover: bool,
    pub fall_time: i32,
    /// HitDef `fall.recovertime`.
    pub fall_recovertime: i32,
    /// Remaining liedown recovery time (`GetHitVar(recovertime)`).
    pub down_recovertime: i32,
    /// Whether fall damage may KO (`GetHitVar(fall.kill)`).
    pub fall_kill: bool,
    /// Fall-impact screen shake (`GetHitVar(fall.envshake.*)`).
    pub fall_envshake_time: i32,
    pub fall_envshake_freq: f32,
    pub fall_envshake_ampl: i32,
    pub fall_envshake_phase: f32,
    pub fall_damage: i32,
    /// `fall.xvelocity`/`fall.yvelocity`; `None` when unset.
    pub fall_x_vel: Option<f32>,
    pub fall_y_vel: Option<f32>,
    pub hitcount: i32,
    /// Number of fall/ground impacts in the current combo.
    pub fallcount: i32,
    /// Clash priority points of the HitDef that connected.
    pub priority: i32,
    pub guarded: bool,
    pub hit_id: i32,
    pub chain_id: i32,
    pub is_bound: bool,
    pub damage: i32,
}

impl Default for GetHitVars {
    fn default() -> Self {
        Self {
            anim_type: 0,
            ground_type: 0,
            air_type: 0,
            hit_type: 0,
            x_vel: 0.0,
            y_vel: 0.0,
            y_accel: 0.0,
            hittime: 0,
            slidetime: 0,
            ctrltime: 0,
            fall: false,
            fall_recover: false,
            fall_time: 0,
            fall_recovertime: 4,
            down_recovertime: 0,
            fall_kill: true,
            fall_envshake_time: 0,
            fall_envshake_freq: 60.0,
            fall_envshake_ampl: -4,
            fall_envshake_phase: f32::NAN,
            fall_damage: 0,
            fall_x_vel: None,
            fall_y_vel: None,
            hitcount: 0,
            fallcount: 0,
            priority: 0,
            guarded: false,
            hit_id: 0,
            chain_id: 0,
            is_bound: false,
            damage: 0,
        }
    }
}

impl GetHitVars {
    pub fn get(&self, key: &str) -> Value {
        match key.trim().to_ascii_lowercase().as_str() {
            "animtype" => Value::Int(self.anim_type),
            "groundtype" => Value::Int(self.ground_type),
            "airtype" => Value::Int(self.air_type),
            "type" => Value::Int(self.hit_type),
            "xvel" => Value::Float(self.x_vel),
            "yvel" => Value::Float(self.y_vel),
            "yaccel" => Value::Float(self.y_accel),
            "hittime" => Value::Int(self.hittime),
            "slidetime" => Value::Int(self.slidetime),
            "ctrltime" => Value::Int(self.ctrltime),
            "fall" => Value::Int(self.fall as i32),
            "fall.recover" => Value::Int(self.fall_recover as i32),
            "fall.time" => Value::Int(self.fall_time),
            "fall.recovertime" => Value::Int(self.fall_recovertime),
            "recovertime" => Value::Int(self.down_recovertime),
            "fall.kill" => Value::Int(self.fall_kill as i32),
            "fall.envshake.time" => Value::Int(self.fall_envshake_time),
            "fall.envshake.freq" => Value::Float(self.fall_envshake_freq),
            "fall.envshake.ampl" => Value::Int(self.fall_envshake_ampl),
            "fall.envshake.phase" => Value::Float(self.fall_envshake_phase),
            "fall.damage" => Value::Int(self.fall_damage),
            "fall.xvelocity" => Value::Float(self.fall_x_vel.unwrap_or(0.0)),
            "fall.yvelocity" => Value::Float(self.fall_y_vel.unwrap_or(0.0)),
            "hitcount" => Value::Int(self.hitcount),
            "fallcount" => Value::Int(self.fallcount),
            "priority" => Value::Int(self.priority),
            "guarded" => Value::Int(self.guarded as i32),
            "hitid" => Value::Int(self.hit_id),
            "chainid" => Value::Int(self.chain_id),
            "isbound" => Value::Int(self.is_bound as i32),
            "damage" => Value::Int(self.damage),
            _ => Value::Int(0),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Runtime hit bookkeeping for one fighter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HitState {
    pub vars: GetHitVars,
    pub hit_pause: i32,
    /// Remaining self-hitpause reported by `HitPauseTime`. The defender's
    /// engine hit-shake freeze is tracked by `hit_pause` but does not count
    /// here.
    pub hit_pause_time: i32,
    pub super_pause: i32,
    pub hit_shake_timer: i32,
    pub hit_over_timer: i32,
    pub guarding: bool,
    pub guard_timer: i32,
    pub invuln_timer: i32,
    pub not_hit_by: Option<(String, i32)>,
    pub hit_by: Option<(String, i32)>,
    pub hit_override: Option<(String, i32)>,
    /// Attackers already struck by the current HitDef activation.
    pub hit_targets: Vec<usize>,
    pub hit_counts: Vec<(usize, i32)>,
    pub last_activation: u32,
    /// Simulation frame on which this fighter last ran a `HitDef`.
    pub last_activation_frame: i64,
    /// Pause ticks since the fighter last ran its controllers. A `HitDef`
    /// resumed after a pause is the same activation, not a new one.
    pub pause_gap: i32,
}

/// Parses expressions used as numeric parameters, falling back to `default`.
pub fn eval_param(expr: &str, default: f32, env: &mut impl FnMut(&Expr) -> Value) -> f32 {
    match crate::expr::parse(expr) {
        Ok(e) => env(&e).as_f32(),
        Err(_) => expr.trim().parse().unwrap_or(default),
    }
}

fn parse_f32(s: &str) -> f32 {
    s.trim().parse().unwrap_or(0.0)
}

fn parse_i32(s: &str) -> i32 {
    s.trim().parse().unwrap_or(0)
}

/// Parses `group, index` sound references, accepting the `S` prefix used by
/// some characters for their own sound bank (`S1, 0`).
fn parse_sound_pair(s: &str) -> (i32, i32) {
    let mut it = s.split(',');
    let group = it.next().unwrap_or("0").trim();
    let group = group
        .strip_prefix(['S', 's'])
        .unwrap_or(group)
        .parse()
        .unwrap_or(-1);
    let index = it.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    (group, index)
}

fn parse_pair(s: &str) -> (f32, f32) {
    let mut it = s.split(',');
    let a = it.next().unwrap_or("0").trim().parse().unwrap_or(0.0);
    let b = it.next().unwrap_or("0").trim().parse().unwrap_or(a);
    (a, b)
}

fn parse_vec(s: &str) -> Vec2 {
    let (x, y) = parse_pair(s);
    Vec2::new(x, y)
}

fn parse_distance(s: &str) -> Vec2 {
    let mut it = s.split(',');
    let x = it
        .next()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(f32::NAN);
    let y = it
        .next()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(f32::NAN);
    Vec2::new(x, y)
}

fn parse_triplet(s: &str) -> [f32; 3] {
    let mut it = s.split(',');
    [
        it.next().unwrap_or("0").trim().parse().unwrap_or(0.0),
        it.next().unwrap_or("0").trim().parse().unwrap_or(0.0),
        it.next().unwrap_or("0").trim().parse().unwrap_or(0.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::{parse_sound_pair, HitDef};

    #[test]
    fn parses_character_sound_prefix() {
        assert_eq!(parse_sound_pair("S1, 0"), (1, 0));
        assert_eq!(parse_sound_pair("s3,2"), (3, 2));
        assert_eq!(parse_sound_pair("5, 1"), (5, 1));
        assert_eq!(parse_sound_pair("bad"), (-1, 0));
    }

    #[test]
    fn parses_extended_hitdef_fields() {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params: vec![
                ("type".into(), "HitDef".into()),
                ("numhits".into(), "3".into()),
                ("p1stateno".into(), "810".into()),
                ("p2stateno".into(), "820".into()),
                ("p1facing".into(), "1".into()),
                ("ground.cornerpush.veloff".into(), "-8".into()),
            ],
            line: 0,
        };
        let hitdef = HitDef::from_controller(&control, 1);
        assert_eq!(hitdef.numhits, 3);
        assert_eq!(hitdef.p1stateno, 810);
        assert_eq!(hitdef.p2stateno, 820);
        assert_eq!(hitdef.p1facing, 1);
        assert_eq!(hitdef.ground_cornerpush, -8.0);
    }
}
