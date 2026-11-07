//! Lightweight CPU opponent.
//!
//! The AI only produces [`PlayerInput`]; it never touches simulation state, so
//! it stays fully deterministic and rollback-safe.

use manato_core::{Buttons, PlayerInput, DIR_DOWN, DIR_LEFT, DIR_RIGHT, DIR_UP};

use crate::fighter::MoveType;
use crate::SimState;

fn hash(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ceb9fe1a85ec53);
    x ^ (x >> 33)
}

pub fn input_for(sim: &SimState, who: usize, level: i32) -> PlayerInput {
    let me = &sim.fighters[who];
    if level <= 0 || !me.alive() || !matches!(sim.round.phase, crate::RoundPhase::Fight) {
        return PlayerInput::default();
    }
    let other = &sim.fighters[sim.other(who)];
    if me.hit.hit_pause > 0 || sim.super_pause > 0 {
        return PlayerInput::default();
    }
    // Already attacking or in hitstun: keep holding forward for combos.
    if me.move_type == MoveType::Attack || me.move_type == MoveType::Hit {
        return PlayerInput::default();
    }

    let dx = other.pos.x - me.pos.x;
    let facing = if dx >= 0.0 { 1 } else { -1 };
    let forward = if facing >= 0 { DIR_RIGHT } else { DIR_LEFT };
    let back = if facing >= 0 { DIR_LEFT } else { DIR_RIGHT };
    // Axis-aligned body gap: 0 means the pushboxes are touching.
    let gap = (dx.abs() - me.width_front - other.width_back).max(0.0);

    // Re-decide every few ticks; the hash keeps it deterministic.
    let slot = (sim.frame / 5) ^ (who as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let roll = hash(slot) % 100;
    let aggression = (20 + level as u64 * 8).min(85);

    if gap > 70.0 {
        if roll < 10 {
            return PlayerInput::new(forward | DIR_UP, 0);
        }
        if roll < 16 {
            return PlayerInput::new(back | DIR_DOWN, 0);
        }
        return PlayerInput::new(forward, 0);
    }

    if gap > 45.0 {
        // Mid range: walk in, occasionally jump in or throw a poke.
        if roll < 12 {
            return PlayerInput::new(forward | DIR_UP, 0);
        }
        if roll < 24 {
            return PlayerInput::new(forward, Buttons::Y);
        }
        return PlayerInput::new(forward, 0);
    }

    // Close range: mostly attack, sometimes guard or crouch.
    if roll < 8 {
        return PlayerInput::new(back, 0);
    }
    if roll < 16 {
        return PlayerInput::new(back | DIR_DOWN, 0);
    }
    if roll < 18 {
        return PlayerInput::new(forward | DIR_UP, 0);
    }
    if roll < 24 {
        return PlayerInput::new(DIR_DOWN, 0);
    }
    if roll < aggression {
        let button = match hash(slot ^ 0xA5A5) % 6 {
            0 => Buttons::X,
            1 => Buttons::Y,
            2 => Buttons::Z,
            3 => Buttons::A,
            4 => Buttons::B,
            _ => Buttons::C,
        };
        let dir = if roll < aggression / 3 {
            forward | DIR_DOWN
        } else {
            forward
        };
        return PlayerInput::new(dir, button);
    }
    // Low kick / sweep to cover short characters.
    PlayerInput::new(DIR_DOWN, Buttons::B)
}
