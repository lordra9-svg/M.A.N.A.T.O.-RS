//! Deterministic primitives for M.A.N.A.T.O.-RS.
//!
//! Everything in this crate must be safe to run inside a rollback resimulation:
//! no wall-clock time, no OS randomness, no pointer-dependent behaviour.

pub mod hash;
pub mod input;
pub mod math;
pub mod rng;

pub use hash::StateHasher;
pub use input::{Buttons, Dir8, PlayerInput, DIR_DOWN, DIR_LEFT, DIR_RIGHT, DIR_UP};
pub use math::{Rect, Vec2};
pub use rng::Rng;

/// Simulation ticks per second. M.U.G.E.N runs its logic at a fixed 60 Hz.
pub const TICKS_PER_SECOND: u32 = 60;

/// The logical coordinate space used by stock M.U.G.E.N content.
pub const GAME_WIDTH: f32 = 320.0;
pub const GAME_HEIGHT: f32 = 240.0;

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    v.max(lo).min(hi)
}
