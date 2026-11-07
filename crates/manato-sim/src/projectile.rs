//! Projectiles spawned by the `Projectile` state controller.

use manato_core::Vec2;

use crate::hitdef::HitDef;

#[derive(Debug, Clone)]
pub struct Projectile {
    pub owner: usize,
    pub id: i32,
    pub anim: i32,
    pub pos: Vec2,
    pub vel: Vec2,
    pub accel: Vec2,
    pub facing: i32,
    pub time: i32,
    /// Remove after this many ticks; negative means no time limit.
    pub remove_time: i32,
    /// Remaining hits before the projectile disappears.
    pub hits: i32,
    pub hitdef: HitDef,
}

impl Projectile {
    pub fn is_expired(&self) -> bool {
        self.hits <= 0 || (self.remove_time >= 0 && self.time > self.remove_time)
    }

    pub fn advance(&mut self) {
        self.time += 1;
        self.pos.x += self.vel.x;
        self.pos.y += self.vel.y;
        self.vel.x += self.accel.x;
        self.vel.y += self.accel.y;
    }
}
