//! Hit detection and reaction handling.

use manato_core::{Dir8, Rect, Vec2};
use manato_formats::Character;

use crate::fighter::{MoveType, StateType};
use crate::hitdef::{HitAttr, HitDef};
use crate::trigger::command_active;
use crate::{SimState, SoundEvent, SoundSource};

pub fn resolve(sim: &mut SimState) {
    for attacker in 0..2 {
        let defender = 1 - attacker;
        if !sim.fighters[attacker].active || !sim.fighters[defender].active {
            continue;
        }
        let Some(hitdef) = sim.fighters[attacker].hitdef.clone() else {
            continue;
        };
        if sim.fighters[attacker].hit.hit_targets.contains(&defender) {
            continue;
        }
        if sim.fighters[defender].hit.not_hit_by.is_some()
            && blocks_by_filter(sim, defender, &hitdef)
        {
            continue;
        }
        if !hitflag_allows(sim, defender, &hitdef) {
            continue;
        }

        let attack_boxes = clsn_boxes(sim, attacker, true);
        let hurt_boxes = clsn_boxes(sim, defender, false);
        if attack_boxes.is_empty() || hurt_boxes.is_empty() {
            continue;
        }
        if !any_overlap(&attack_boxes, &hurt_boxes) {
            continue;
        }

        let guarding = can_guard(sim, attacker, defender, &hitdef);
        apply_hit(sim, attacker, defender, &hitdef, guarding);
    }
}

fn clsn_boxes(sim: &SimState, who: usize, attack: bool) -> Vec<Rect> {
    let fighter = &sim.fighters[who];
    let char = &sim.assets.characters[fighter.char_id];
    let Some(frame) = fighter.current_frame(char) else {
        return Vec::new();
    };
    let boxes = if attack { &frame.clsn1 } else { &frame.clsn2 };
    boxes.iter().map(|b| fighter.world_box(b)).collect()
}

fn any_overlap(a: &[Rect], b: &[Rect]) -> bool {
    a.iter().any(|x| b.iter().any(|y| x.intersects(*y)))
}

fn hitflag_allows(sim: &SimState, defender: usize, hitdef: &HitDef) -> bool {
    let f = &sim.fighters[defender];
    let airborne = f.state_type == StateType::Air || f.pos.y < -1.0;
    if airborne {
        return hitdef.hitflag.contains('A');
    }
    let low = f.state_type == StateType::Crouch;
    if low && !(hitdef.hitflag.contains('L') || hitdef.hitflag.contains('M')) {
        return false;
    }
    if !low && !(hitdef.hitflag.contains('H') || hitdef.hitflag.contains('M')) {
        return false;
    }
    true
}

fn can_guard(sim: &SimState, _attacker: usize, defender: usize, hitdef: &HitDef) -> bool {
    let f = &sim.fighters[defender];
    if f.move_type == MoveType::Hit || f.state_no >= 5000 {
        return false;
    }
    let input = f.held_input();
    let dir = Dir8::relative(input, f.facing);
    if !dir.is_back() {
        // Some content uses a dedicated "blocking" command with forward held.
        if !command_active(sim, defender, "blocking") {
            return false;
        }
    }
    if !f.ctrl && !(130..=155).contains(&f.state_no) && f.state_no != 0 && f.state_no != 11 {
        return false;
    }
    let airborne = f.state_type == StateType::Air || f.pos.y < -1.0;
    let flag = &hitdef.guardflag;
    if airborne {
        return flag.contains('A');
    }
    if f.state_type == StateType::Crouch {
        flag.contains('L') || flag.contains('M')
    } else {
        flag.contains('H') || flag.contains('M')
    }
}

fn blocks_by_filter(sim: &SimState, defender: usize, hitdef: &HitDef) -> bool {
    let Some((attr, _)) = &sim.fighters[defender].hit.not_hit_by else {
        return false;
    };
    if attr.is_empty() {
        return false;
    }
    let state = hitdef.attr.state_types.chars().next().unwrap_or('S');
    attr.contains(state) || attr.contains('*')
}

fn anim_duration(char: &Character, anim: i32) -> i32 {
    char.animation(anim).map(|a| a.total_time()).unwrap_or(10)
}

fn apply_hit(
    sim: &mut SimState,
    attacker: usize,
    defender: usize,
    hitdef: &HitDef,
    guarding: bool,
) {
    let origin = sim.fighters[attacker].pos;
    let facing = sim.fighters[attacker].facing;
    apply_hit_at(
        sim, attacker, defender, hitdef, guarding, origin, facing, true,
    );
}

pub(crate) fn apply_hit_at(
    sim: &mut SimState,
    attacker: usize,
    defender: usize,
    hitdef: &HitDef,
    guarding: bool,
    origin: Vec2,
    facing: i32,
    from_fighter: bool,
) {
    let attacker_facing = facing;
    let defender_airborne = {
        let f = &sim.fighters[defender];
        f.state_type == StateType::Air || f.pos.y < -1.0
    };
    let defender_state_type = sim.fighters[defender].state_type;

    let base_damage = if guarding {
        if hitdef.guard_damage > 0 {
            hitdef.guard_damage
        } else {
            hitdef.damage / 4
        }
    } else {
        hitdef.damage
    };
    let damage = (base_damage as f32
        * sim.fighters[attacker].attack_mul
        * sim.fighters[defender].defence_mul) as i32;

    let (velocity, hittime, slidetime) = if guarding {
        let mut v = if defender_airborne {
            hitdef.airguard_velocity
        } else {
            hitdef.ground_velocity
        };
        v.x *= 0.5;
        (v, hitdef.guard_hittime.max(1), hitdef.ground_slidetime)
    } else if defender_airborne {
        (
            hitdef.air_velocity,
            hitdef.air_hittime.max(1),
            hitdef.ground_slidetime,
        )
    } else {
        (
            hitdef.ground_velocity,
            hitdef.ground_hittime.max(1),
            hitdef.ground_slidetime,
        )
    };

    {
        let f = &mut sim.fighters[defender];
        f.hit.vars.anim_type = hitdef.anim_type.code();
        f.hit.vars.ground_type = hitdef.ground_type.code();
        f.hit.vars.air_type = hitdef.air_type.code();
        f.hit.vars.x_vel = velocity.x * attacker_facing as f32;
        f.hit.vars.y_vel = velocity.y;
        f.hit.vars.y_accel = 0.0;
        f.hit.vars.hittime = hittime;
        f.hit.vars.slidetime = slidetime;
        f.hit.vars.ctrltime = hittime + slidetime;
        f.hit.vars.fall = hitdef.fall;
        f.hit.vars.fall_recover = hitdef.fall_recover;
        f.hit.vars.fall_time = hitdef.fall_time;
        f.hit.vars.fall_damage = hitdef.fall_damage;
        f.hit.vars.damage = damage;
        f.hit.vars.guarded = guarding;
        f.hit.vars.hitcount += 1;
        f.hit.vars.hit_id = hitdef.activation as i32;
        f.hit.guarding = guarding;
        f.hit.hit_pause = hitdef.pausetime.0.max(0);
        f.life = (f.life - damage).max(0);
        f.ctrl = false;
        f.attack = Default::default();
        f.hitdef = None;
    }

    let char = &sim.assets.characters[sim.fighters[defender].char_id];
    let new_state = if guarding {
        match defender_state_type {
            StateType::Air => 154,
            StateType::Crouch => 152,
            _ => 150,
        }
    } else if defender_airborne {
        5020
    } else if defender_state_type == StateType::Crouch {
        5010
    } else {
        5000
    };

    if char.state(new_state).is_some() {
        sim.fighters[defender].enter_state(new_state, char, false);
        sim.fighters[defender].ctrl = false;
    } else {
        // Content without common1: fall back to a minimal reaction.
        sim.fighters[defender].state_time = 0;
        sim.fighters[defender].vel = Vec2::new(
            sim.fighters[defender].hit.vars.x_vel,
            sim.fighters[defender].hit.vars.y_vel,
        );
    }

    if from_fighter {
        let f = &mut sim.fighters[attacker];
        f.attack.move_contact = true;
        f.attack.move_hit |= !guarding;
        f.attack.move_guarded |= guarding;
        f.attack.hit_count += 1;
        f.hit.hit_targets.push(defender);
        f.hit.hit_pause = hitdef.pausetime.1.max(0);
    }

    // Hit spark and sound.
    let spark_anim = if guarding {
        if hitdef.guard_sparkno >= 0 {
            hitdef.guard_sparkno
        } else if hitdef.sparkno >= 0 {
            hitdef.sparkno
        } else {
            40
        }
    } else if hitdef.sparkno >= 0 {
        hitdef.sparkno
    } else {
        0
    };
    let spark_pos = Vec2::new(
        origin.x + hitdef.sparkxy.x * attacker_facing as f32,
        origin.y + hitdef.sparkxy.y,
    );
    let duration = sim
        .assets
        .effects
        .animations
        .get(&spark_anim)
        .map(|a| a.total_time())
        .unwrap_or(8);
    sim.effects.push(crate::Effect {
        source: crate::EffectSource::FightFx,
        anim: spark_anim,
        pos: spark_pos,
        vel: Vec2::ZERO,
        accel: Vec2::ZERO,
        scale: Vec2::new(1.0, 1.0),
        facing: attacker_facing,
        flip_h: false,
        flip_v: false,
        blend: crate::EffectBlend::Add,
        alpha: 1.0,
        sprite_priority: 5,
        time: 0,
        duration,
        remove_on_anim_end: true,
        id: 0,
        owner: attacker,
        behind: false,
    });

    let (sound_group, sound_index) = if guarding {
        hitdef.guardsound
    } else {
        hitdef.hitsound
    };
    if sound_group >= 0 {
        sim.sounds.push(SoundEvent {
            source: SoundSource::Character(attacker),
            group: sound_group,
            index: sound_index,
            volume: 0.0,
            pan: 0.0,
        });
    }

    // Camera kick for heavy hits.
    if !guarding && hitdef.damage >= 80 {
        sim.env_shake = crate::EnvShake {
            time: 6,
            freq: 60.0,
            ampl: -2.0,
            phase: 0.0,
        };
    }
    let _ = anim_duration;
    let _ = HitAttr::default();
}

/// Resolves projectile hits against the opponent.
pub fn resolve_projectiles(sim: &mut SimState) {
    let count = sim.projectiles.len();
    for i in 0..count {
        let (owner, anim, time, facing, pos, hits, hitdef) = {
            let p = &sim.projectiles[i];
            (
                p.owner,
                p.anim,
                p.time,
                p.facing,
                p.pos,
                p.hits,
                p.hitdef.clone(),
            )
        };
        if hits <= 0 {
            continue;
        }
        let defender = 1 - owner;
        if !sim.fighters[defender].active {
            continue;
        }
        if sim.fighters[defender].hit.not_hit_by.is_some()
            && blocks_by_filter(sim, defender, &hitdef)
        {
            continue;
        }
        if !hitflag_allows(sim, defender, &hitdef) {
            continue;
        }

        let attack_boxes = projectile_boxes(sim, owner, anim, time, facing, pos);
        let hurt_boxes = clsn_boxes(sim, defender, false);
        if attack_boxes.is_empty() || hurt_boxes.is_empty() {
            continue;
        }
        if !any_overlap(&attack_boxes, &hurt_boxes) {
            continue;
        }

        let guarding = can_guard(sim, owner, defender, &hitdef);
        apply_hit_at(sim, owner, defender, &hitdef, guarding, pos, facing, false);
        sim.projectiles[i].hits -= 1;
    }
}

fn projectile_boxes(
    sim: &SimState,
    owner: usize,
    anim_no: i32,
    time: i32,
    facing: i32,
    pos: Vec2,
) -> Vec<Rect> {
    let char = &sim.assets.characters[sim.fighters[owner].char_id];
    let Some(anim) = char.animation(anim_no) else {
        return Vec::new();
    };
    let Some((frame_idx, _)) = anim.frame_at(time) else {
        return Vec::new();
    };
    let Some(frame) = anim.frames.get(frame_idx) else {
        return Vec::new();
    };
    frame
        .clsn1
        .iter()
        .map(|b| {
            let mut r = b.rect();
            if facing < 0 {
                r = r.mirror_x();
            }
            r.translate(pos.x.round() as i32, pos.y.round() as i32)
        })
        .collect()
}
