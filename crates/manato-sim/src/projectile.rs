//! Projectiles spawned by the `Projectile` state controller.

use manato_core::Vec2;

use crate::hitdef::HitDef;

/// Lifecycle state of a projectile after it stops being active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ProjectileStatus {
    #[default]
    Active,
    Hit,
    Rem,
    Cancel,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Projectile {
    pub owner: usize,
    pub id: i32,
    pub anim: i32,
    pub pos: Vec2,
    pub vel: Vec2,
    pub accel: Vec2,
    /// Depth position (`offset` third component; follows the owner's `z`).
    pub z: f32,
    pub vel_z: f32,
    pub accel_z: f32,
    pub facing: i32,
    /// `projscale` multiplier for the animation frame.
    pub scale: Vec2,
    /// `projangle` added to the frame angle in degrees.
    pub angle: f32,
    /// `projxangle`/`projyangle` perspective rotation.
    pub x_angle: f32,
    pub y_angle: f32,
    /// `projxshear` applied to the sprite.
    pub x_shear: f32,
    /// `projprojection` perspective mode.
    pub projection: crate::ProjectionMode,
    /// `projfocallength` focal length.
    pub focal_length: f32,
    pub time: i32,
    /// Remaining lifetime in ticks; `0` is expired, `-1` never times out and
    /// `-2` expires when the animation reaches its loop end.
    pub remove_time: i32,
    /// One animation pass length in ticks; `0` when unknown.
    pub anim_loop_time: i32,
    /// Remove once `hits` is exhausted (`projremove`, default true).
    pub remove_on_hits_exhausted: bool,
    /// Clash priority points (`projpriority`); unused until clashes resolve.
    pub priority: i32,
    pub priority_points: i32,
    /// `projshadow` RGB components; all zero disables the shadow.
    pub shadow: [i32; 3],
    /// `projreflection` mode (0 = off).
    pub reflection: i32,
    /// `projmisstime` stored miss timer.
    pub miss_time: i32,
    /// `projhitanim` used after a hit; -1 keeps the projectile animation.
    pub hit_anim: i32,
    /// `projremanim` used while removing; -1 keeps the current animation.
    pub rem_anim: i32,
    /// `projcancelanim` used after a clash; -1 keeps the current animation.
    pub cancel_anim: i32,
    /// `projlayerno` draw layer (0 background, 1 foreground).
    pub layer_no: i32,
    /// `projsprpriority` draw order.
    pub sprite_priority: i32,
    /// `projedgebound` / `projstagebound` / `projheightbound` / `projdepthbound`.
    pub edge_bound: f32,
    pub stage_bound: f32,
    pub height_bound: (f32, f32),
    pub depth_bound: f32,
    /// Set when the projectile leaves its bounds and should be removed.
    pub out_of_bounds: bool,
    pub status: ProjectileStatus,
    /// Ticks left in the hit/removal animation; `0` means remove now.
    pub removal_ticks: i32,
    /// `remvelocity` applied when entering the hit/removal state.
    pub rem_velocity: Vec2,
    pub rem_velocity_z: f32,
    /// Remaining hits before the projectile disappears.
    pub hits: i32,
    /// Initial hit allowance (`projhitsmax`).
    pub hits_max: i32,
    pub hitdef: HitDef,
}

impl Projectile {
    pub fn is_expired(&self) -> bool {
        if self.status != ProjectileStatus::Active {
            return self.removal_ticks <= 0 || self.time >= self.removal_ticks;
        }
        let loop_expired =
            self.remove_time <= -2 && self.anim_loop_time > 0 && self.time >= self.anim_loop_time;
        (self.hits <= 0 && self.remove_on_hits_exhausted)
            || self.remove_time == 0
            || loop_expired
            || self.out_of_bounds
    }

    /// Switches an active projectile to its hit/removal animation once its
    /// lifetime, hits or bounds end (`projhitanim`/`projremanim`). A `-1`
    /// animation removes it immediately.
    pub fn begin_removal(&mut self, character: &manato_formats::Character) {
        if self.status != ProjectileStatus::Active {
            return;
        }
        let loop_expired =
            self.remove_time <= -2 && self.anim_loop_time > 0 && self.time >= self.anim_loop_time;
        let status = if self.hits <= 0 && self.remove_on_hits_exhausted {
            ProjectileStatus::Hit
        } else if self.remove_time == 0 || loop_expired || self.out_of_bounds {
            ProjectileStatus::Rem
        } else {
            return;
        };
        self.status = status;
        // Entering the removal state replaces the velocity.
        self.vel = self.rem_velocity;
        self.vel_z = self.rem_velocity_z;
        self.accel = Vec2::ZERO;
        self.accel_z = 0.0;
        let anim = match status {
            ProjectileStatus::Hit => self.hit_anim,
            ProjectileStatus::Rem => self.rem_anim,
            ProjectileStatus::Cancel => self.cancel_anim,
            ProjectileStatus::Active => return,
        };
        if anim < 0 {
            self.removal_ticks = 0;
            return;
        }
        match character.animation(anim) {
            Some(animation) => {
                self.anim = anim;
                self.anim_loop_time = animation.total_time();
                self.removal_ticks = animation.total_time().max(0);
                self.time = 0;
            }
            None => self.removal_ticks = 0,
        }
    }

    /// Marks the projectile for removal once it passes the screen edge or
    /// stage/height bounds (`projedgebound`, `projstagebound`,
    /// `projheightbound`; depth is not modeled).
    pub fn apply_bounds(
        &mut self,
        camera_x: f32,
        scale: f32,
        stage_left: f32,
        stage_right: f32,
        z_range: (f32, f32),
    ) {
        // `projedgebound` extends the visible camera edges; `projstagebound`
        // extends the camera's movement limits.
        let half = manato_core::GAME_WIDTH / (2.0 * scale.max(0.0001));
        let view_left = camera_x - half;
        let view_right = camera_x + half;
        let vel_x = self.vel.x * self.facing as f32;
        if self.pos.x < view_left - self.edge_bound
            || self.pos.x > view_right + self.edge_bound
            || (vel_x < 0.0 && self.pos.x < stage_left - half - self.stage_bound)
            || (vel_x > 0.0 && self.pos.x > stage_right + half + self.stage_bound)
            || (self.vel.y > 0.0 && self.pos.y > self.height_bound.1)
            || (self.vel.y < 0.0 && self.pos.y < self.height_bound.0)
            || ((z_range.1 - z_range.0).abs() >= f32::EPSILON
                && (self.z < z_range.0 - self.depth_bound || self.z > z_range.1 + self.depth_bound))
        {
            self.out_of_bounds = true;
        }
    }

    pub fn advance(&mut self) {
        self.time += 1;
        self.pos.x += self.vel.x;
        self.pos.y += self.vel.y;
        self.z += self.vel_z;
        self.vel.x += self.accel.x;
        self.vel.y += self.accel.y;
        self.vel_z += self.accel_z;
        // Ikemen decrements the countdown after the frame's contact checks.
        if self.remove_time > 0 {
            self.remove_time -= 1;
        }
    }
}
