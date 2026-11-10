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
    /// Remaining lifetime in ticks; `0` is expired, `-1`/`-2` have no
    /// timeout (Ikemen treats `-2` as animation-loop expiry).
    pub remove_time: i32,
    /// Remaining hits before the projectile disappears.
    pub hits: i32,
    pub hitdef: HitDef,
}

impl Projectile {
    pub fn is_expired(&self) -> bool {
        self.hits <= 0 || self.remove_time == 0
    }

    pub fn advance(&mut self) {
        self.time += 1;
        self.pos.x += self.vel.x;
        self.pos.y += self.vel.y;
        self.vel.x += self.accel.x;
        self.vel.y += self.accel.y;
        // Ikemen decrements the countdown after the frame's contact checks.
        if self.remove_time > 0 {
            self.remove_time -= 1;
        }
    }
}
