//! Hit detection and reaction handling.

use manato_core::{Dir8, Rect, Vec2};
use manato_formats::Character;

use crate::fighter::{MoveType, StateType};
use crate::hitdef::{GroundType, HitAttr, HitDef};
use crate::trigger::command_active;
use crate::{SimState, SoundEvent, SoundSource};

pub fn resolve(sim: &mut SimState) {
    for attacker in 0..sim.fighters.len() {
        let defender = sim.opponent_of(attacker);
        if defender >= 2 || attacker == defender {
            continue;
        }
        if !sim.fighters[attacker].active || !sim.fighters[defender].active {
            continue;
        }
        let Some(hitdef) = sim.fighters[attacker].hitdef.clone() else {
            continue;
        };
        if !hit_allowed(&sim.fighters[attacker].hit, defender, &hitdef) {
            continue;
        }
        if let Some(reversal) = sim.fighters[defender].reversal.clone() {
            if sim.fighters[defender].reversal_time > 0
                && attr_matches_attr(&reversal.attr, &hitdef.attr)
            {
                let origin = sim.fighters[defender].pos;
                let facing = sim.fighters[defender].facing;
                apply_hit_at(
                    sim, defender, attacker, &reversal, false, origin, facing, true,
                );
                record_hit(sim, defender, attacker);
                continue;
            }
        }
        if blocks_by_filter(sim, defender, &hitdef) {
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

        if let Some(override_) = sim.fighters[defender].hit_override.clone() {
            if override_.time > 0 && attr_matches(&override_.attr, &hitdef.attr) {
                let assets = sim.assets.clone();
                let char_id = sim.fighters[defender].char_id;
                sim.fighters[defender].enter_state(
                    override_.state,
                    &assets.characters[char_id],
                    false,
                );
                record_hit(sim, attacker, defender);
                continue;
            }
        }

        let guarding = can_guard(sim, attacker, defender, &hitdef);
        let defender_airborne = {
            let f = &sim.fighters[defender];
            f.state_type == StateType::Air || f.pos.y < -1.0
        };
        let juggle_cost = sim.fighters[attacker].juggle;
        let already_targeted = sim.fighters[attacker].hit.hit_targets.contains(&defender);
        if !juggle_allows(
            sim.fighters[defender].juggle_points,
            sim.fighters[defender].assert_flags,
            guarding,
            defender_airborne,
            juggle_cost,
            already_targeted,
        ) {
            continue;
        }
        apply_hit(sim, attacker, defender, &hitdef, guarding);
        record_hit(sim, attacker, defender);
        // Ikemen-GO consumes juggle points when the target was falling before
        // the hit or the hit itself starts a fall (trip/fall HitDefs).
        if !guarding && (defender_airborne || hitdef.fall) && juggle_cost > 0 {
            sim.fighters[defender].juggle_points =
                (sim.fighters[defender].juggle_points - juggle_cost).max(0);
        }
    }
}

/// Whether juggle points allow an airborne hit.
fn juggle_allows(
    juggle_points: i32,
    assert_flags: u32,
    guarding: bool,
    airborne: bool,
    cost: i32,
    already_targeted: bool,
) -> bool {
    if guarding || !airborne || cost <= 0 {
        return true;
    }
    if assert_flags & crate::fighter::ASSERT_NOJUGGLECHECK != 0 {
        return true;
    }
    // M.U.G.E.N lets a fresh activation juggle an enemy that is not one of its
    // targets yet; the points gate applies to repeat hits within an activation.
    if !already_targeted {
        return true;
    }
    juggle_points >= cost
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

/// Records a hit against `defender` for the attacker's current activation.
fn record_hit(sim: &mut SimState, attacker: usize, defender: usize) {
    let hit = &mut sim.fighters[attacker].hit;
    if let Some((_, count)) = hit
        .hit_counts
        .iter_mut()
        .find(|(target, _)| *target == defender)
    {
        *count += 1;
    } else {
        hit.hit_counts.push((defender, 1));
    }
    if !hit.hit_targets.contains(&defender) {
        hit.hit_targets.push(defender);
    }
}

/// Whether the current attack activation may still hit `defender`.
fn hit_allowed(hit: &crate::hitdef::HitState, defender: usize, hitdef: &HitDef) -> bool {
    if hitdef.hitonce && !hit.hit_targets.is_empty() {
        return false;
    }
    let previous_hits = hit
        .hit_counts
        .iter()
        .find(|(target, _)| *target == defender)
        .map(|(_, count)| *count)
        .unwrap_or(0);
    previous_hits < hitdef.numhits.max(1)
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
    if f.assert_flags & crate::fighter::ASSERT_UNGUARDABLE != 0 {
        return false;
    }
    let airborne = f.state_type == StateType::Air || f.pos.y < -1.0;
    let flag = &hitdef.guardflag;
    if airborne {
        if f.assert_flags & crate::fighter::ASSERT_NOAIRGUARD != 0 {
            return false;
        }
        return flag.contains('A');
    }
    if f.state_type == StateType::Crouch {
        if f.assert_flags & crate::fighter::ASSERT_NOCROUCHGUARD != 0 {
            return false;
        }
        flag.contains('L') || flag.contains('M')
    } else {
        if f.assert_flags & crate::fighter::ASSERT_NOSTANDGUARD != 0 {
            return false;
        }
        flag.contains('H') || flag.contains('M')
    }
}

fn blocks_by_filter(sim: &SimState, defender: usize, hitdef: &HitDef) -> bool {
    let fighter = &sim.fighters[defender];
    filter_blocks(
        fighter
            .hit
            .not_hit_by
            .as_ref()
            .map(|(attr, _)| attr.as_str()),
        fighter.hit.hit_by.as_ref().map(|(attr, _)| attr.as_str()),
        &hitdef.attr,
    )
}

/// M.U.G.E.N attribute filter: `states, classes[, ...]`, e.g. `SCA, NA, SA`.
fn filter_matches(attr: &str, incoming: &HitAttr) -> bool {
    let attr = attr.trim();
    if attr.is_empty() {
        return false;
    }
    let mut parts = attr.split(',');
    let states = parts.next().unwrap_or("").trim().to_ascii_uppercase();
    let classes = parts.next().unwrap_or("").trim().to_ascii_uppercase();
    let state = incoming.state_types.chars().next().unwrap_or('S');
    let class = incoming.class.chars().next().unwrap_or('N');
    let state_ok = states.is_empty() || states.contains(state) || states.contains('*');
    let class_ok = classes.is_empty() || classes.contains(class) || classes.contains('*');
    state_ok && class_ok
}

fn filter_blocks(not_hit_by: Option<&str>, hit_by: Option<&str>, incoming: &HitAttr) -> bool {
    // HitBy is a whitelist and takes precedence over NotHitBy.
    if let Some(attr) = hit_by.filter(|attr| !attr.trim().is_empty()) {
        return !filter_matches(attr, incoming);
    }
    not_hit_by
        .filter(|attr| !attr.trim().is_empty())
        .map(|attr| filter_matches(attr, incoming))
        .unwrap_or(false)
}

fn attr_matches_attr(candidate: &HitAttr, incoming: &HitAttr) -> bool {
    let incoming_state = incoming.state_types.chars().next().unwrap_or('S');
    let incoming_class = incoming.class.chars().next().unwrap_or('N');
    (candidate.state_types.contains(incoming_state) || candidate.state_types.contains('*'))
        && (candidate.class.contains(incoming_class) || candidate.class.contains('*'))
}

fn attr_matches(override_attr: &str, hit_attr: &HitAttr) -> bool {
    let mut parts = override_attr.split(',');
    let states = parts.next().unwrap_or("");
    let classes = parts.next().unwrap_or("");
    let state = hit_attr.state_types.chars().next().unwrap_or('S');
    let class = hit_attr.class.chars().next().unwrap_or('N');
    (states.contains(state) || states.contains('*'))
        && (classes.contains(class) || classes.contains('*'))
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

/// Applies a resolved hit from `attacker` to `defender`.
pub fn apply_hit_at(
    sim: &mut SimState,
    attacker: usize,
    defender: usize,
    hitdef: &HitDef,
    guarding: bool,
    origin: Vec2,
    facing: i32,
    from_fighter: bool,
) {
    if from_fighter && !guarding && hitdef.p1stateno >= 0 {
        let assets = sim.assets.clone();
        let attacker_char = sim.fighters[attacker].char_id;
        let defender_char = sim.fighters[defender].char_id;
        if let Some(state) = hitdef.p1stateno.into() {
            sim.fighters[attacker].enter_state(state, &assets.characters[attacker_char], false);
        }
        if hitdef.p2stateno >= 0 {
            sim.fighters[defender].enter_state(
                hitdef.p2stateno,
                &assets.characters[defender_char],
                false,
            );
        }
        if hitdef.p1facing != 0 {
            sim.fighters[attacker].facing = hitdef.p1facing;
        }
        if hitdef.p2facing != 0 {
            sim.fighters[defender].facing = hitdef.p2facing;
        }
        sim.fighters[attacker].sprite_priority = hitdef.p1sprpriority;
        sim.fighters[defender].sprite_priority = hitdef.p2sprpriority;
        sim.fighters[attacker].target = Some(defender);
        sim.fighters[attacker].hit.hit_targets.push(defender);
        return;
    }

    let attacker_facing = facing;
    let defender_airborne = {
        let f = &sim.fighters[defender];
        f.state_type == StateType::Air || f.pos.y < -1.0
    };
    let defender_state_type = sim.fighters[defender].state_type;

    let base_damage = if guarding {
        if hitdef.guard_damage_set {
            hitdef.guard_damage
        } else {
            hitdef.damage / 4
        }
    } else {
        hitdef.damage
    };
    let damage = (base_damage as f32
        * sim.fighters[attacker].attack_mul
        * sim.fighters[defender].defence_mul
        / sim.fighters[defender].fall_defence_mul.max(0.01)) as i32;

    let mut velocity_tuple = if guarding {
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
        let stage = &sim.assets.stage;
        let f = &sim.fighters[defender];
        let near_corner = f.pos.x <= stage.left_bound + 24.0 || f.pos.x >= stage.right_bound - 24.0;
        if near_corner {
            let push = if defender_airborne {
                hitdef.air_cornerpush
            } else {
                hitdef.ground_cornerpush
            };
            if push != 0.0 {
                velocity_tuple.0.x += push * attacker_facing as f32;
            }
        }
    }
    let (velocity, hittime, slidetime) = velocity_tuple;

    {
        let f = &mut sim.fighters[defender];
        f.hit.vars.anim_type = hitdef.anim_type.code();
        f.hit.vars.ground_type = hitdef.ground_type.code();
        f.hit.vars.air_type = hitdef.air_type.code();
        f.hit.vars.x_vel = velocity.x * -(attacker_facing as f32);
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
        f.hit.hit_shake_timer = hitdef.pausetime.1.max(0);
        f.life = (f.life - damage).max(0);
        if !guarding {
            f.received_hits = f.received_hits.saturating_add(hitdef.numhits.max(1));
            f.received_damage = f.received_damage.saturating_add(damage);
        }
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
    } else if hitdef.fall {
        if hitdef.ground_type == GroundType::Trip {
            5070
        } else {
            5030
        }
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
        facing: attacker_facing,
        blend: crate::EffectBlend::Add,
        sprite_priority: 5,
        duration,
        owner: attacker,
        ..crate::Effect::default()
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
            channel: -1,
            loop_sound: false,
            priority: 0,
            low_priority: false,
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
        let defender = sim.opponent_of(owner);
        if defender >= 2 || !sim.fighters[defender].active {
            continue;
        }
        if blocks_by_filter(sim, defender, &hitdef) {
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
        sim.fighters[owner].proj_contact_time = 0;
        if guarding {
            sim.fighters[owner].proj_guarded_time = 0;
        } else {
            sim.fighters[owner].proj_hit_time = 0;
        }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]
    use super::*;
    use crate::hitdef::HitState;

    fn hit_state() -> HitState {
        HitState::default()
    }

    #[test]
    fn hitonce_limits_activation_to_one_target() {
        let mut hitdef = HitDef::default();
        hitdef.hitonce = true;
        hitdef.numhits = 3;
        let mut hit = hit_state();
        assert!(hit_allowed(&hit, 1, &hitdef));
        hit.hit_targets.push(1);
        assert!(!hit_allowed(&hit, 1, &hitdef));
    }

    #[test]
    fn numhits_allows_repeated_hits_until_limit() {
        let mut hitdef = HitDef::default();
        hitdef.numhits = 2;
        let mut hit = hit_state();
        assert!(hit_allowed(&hit, 1, &hitdef));
        hit.hit_counts.push((1, 1));
        assert!(hit_allowed(&hit, 1, &hitdef));
        hit.hit_counts[0].1 = 2;
        assert!(!hit_allowed(&hit, 1, &hitdef));
        // Another defender still has its own budget.
        assert!(hit_allowed(&hit, 0, &hitdef));
    }

    #[test]
    fn juggle_points_gate_air_hits() {
        assert!(juggle_allows(10, 0, false, true, 5, false));
        assert!(!juggle_allows(4, 0, false, true, 5, true));
        // A fresh activation bypasses the gate for an untargeted enemy.
        assert!(juggle_allows(4, 0, false, true, 5, false));
        assert!(juggle_allows(10, 0, false, true, 5, true));
        // Guarding hits and grounded defenders ignore juggle points.
        assert!(juggle_allows(0, 0, true, true, 5, true));
        assert!(juggle_allows(0, 0, false, false, 5, true));
        // AssertSpecial NoJuggleCheck bypasses the check.
        assert!(juggle_allows(
            0,
            crate::fighter::ASSERT_NOJUGGLECHECK,
            false,
            true,
            5,
            true
        ));
    }
    #[test]
    fn attribute_filters_match_state_and_class() {
        let mut hit = HitDef::default();
        hit.attr.state_types = "S".into();
        hit.attr.class = "NA".into();
        assert!(filter_matches("SCA, NA", &hit.attr));
        assert!(filter_matches("S, NA", &hit.attr));
        assert!(!filter_matches("C, NA", &hit.attr));
        assert!(!filter_matches("SCA, SA", &hit.attr));
        assert!(filter_blocks(Some("SCA, NA"), None, &hit.attr));
        assert!(!filter_blocks(Some("SCA, SA"), None, &hit.attr));
        assert!(!filter_blocks(None, Some("SCA, NA"), &hit.attr));
        assert!(filter_blocks(None, Some("SCA, SA"), &hit.attr));
        // HitBy wins over NotHitBy.
        assert!(!filter_blocks(Some("SCA, SA"), Some("SCA, NA"), &hit.attr));
    }
}
