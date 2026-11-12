//! Hit detection and reaction handling.

use manato_core::{Dir8, Rect, Vec2};
use manato_formats::Character;

use crate::fighter::{MoveType, StateType};
use crate::hitdef::{GroundType, HitAttr, HitDef, TeamFilter};
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
        if !affect_team_allows(sim, attacker, defender, &hitdef) {
            continue;
        }
        if !chain_gate_allows(sim, defender, &hitdef) {
            continue;
        }
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
                sim.fighters[attacker].attack.move_reversed = true;
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

        let mut forced_guard = false;
        if let Some(override_) = sim.fighters[defender].matched_override(&hitdef) {
            if override_.forceguard {
                forced_guard = true;
            } else {
                if !override_.keepstate {
                    let assets = sim.assets.clone();
                    let char_id = sim.fighters[defender].char_id;
                    sim.fighters[defender].enter_state(
                        override_.state,
                        &assets.characters[char_id],
                        false,
                    );
                }
                record_hit(sim, attacker, defender);
                continue;
            }
        }

        let guarding =
            forced_guard || can_guard(sim, defender, &hitdef, sim.fighters[attacker].pos);
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
        if !guarding
            && (defender_airborne || hitdef.fall)
            && juggle_cost > 0
            && sim.fighters[defender].assert_flags & crate::fighter::ASSERT_NOJUGGLECHECK == 0
        {
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
    let Some(frame) = fighter.current_frame(sim.anim_character(who)) else {
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
    // `F`: a falling opponent may only be hit when the flag is present
    // (`src/char.go` HF_F check).
    let falling = f.hit.vars.fall || matches!(f.state_no, 5030 | 5040 | 5050);
    if falling && !hitdef.hitflag.contains('F') {
        return false;
    }
    let airborne = f.state_type == StateType::Air || f.pos.y < -1.0;
    if airborne {
        return hitdef.hitflag.contains('A');
    }
    if f.state_type == StateType::Liedown {
        return hitdef.hitflag.contains('D');
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

fn can_guard(sim: &SimState, defender: usize, hitdef: &HitDef, origin: Vec2) -> bool {
    let f = &sim.fighters[defender];
    if f.move_type == MoveType::Hit || f.state_no >= 5000 {
        return false;
    }
    let dx = (origin.x - f.pos.x).abs();
    if dx >= hitdef.guard_dist as f32 {
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
    let airborne = f.state_type == StateType::Air || f.pos.y < -1.0;
    if !f.ctrl
        && !(130..=155).contains(&f.state_no)
        && f.state_no != 0
        && f.state_no != 11
        && !airborne
    {
        return false;
    }
    if f.assert_flags & crate::fighter::ASSERT_UNGUARDABLE != 0 {
        return false;
    }
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

/// Whether `affectteam` lets this attacker hit this defender.
fn affect_team_allows(sim: &SimState, attacker: usize, defender: usize, hitdef: &HitDef) -> bool {
    let attacker_side = sim.fighters[attacker].side;
    let defender_side = sim.fighters[defender].side;
    match hitdef.affect_team {
        TeamFilter::Both => true,
        TeamFilter::Enemy => attacker_side != defender_side,
        TeamFilter::Friendly => attacker_side == defender_side,
    }
}

/// Whether `chainid`/`nochainID` allow this hit against the defender's last
/// recorded hit id.
fn chain_gate_allows(sim: &SimState, defender: usize, hitdef: &HitDef) -> bool {
    let f = &sim.fighters[defender];
    if hitdef.chainid >= 0 && f.hit.vars.hit_id != hitdef.chainid {
        return false;
    }
    if f.move_type == MoveType::Hit {
        for id in hitdef.nochainid {
            if id >= 0 && f.hit.vars.hit_id == id {
                return false;
            }
        }
    }
    true
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

pub(crate) fn override_attr_matches(override_attr: &str, hit_attr: &HitAttr) -> bool {
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
    // NotHitBy / HitBy filters apply on every hit path, including direct
    // callers and projectiles.
    if !guarding && blocks_by_filter(sim, defender, hitdef) {
        return;
    }
    // `ground.type`/`air.type = None` is documented as a pause-only contact:
    // it applies both pausetime values but no damage, velocity, spark, or
    // state change.
    let defender_airborne_at_contact = {
        let f = &sim.fighters[defender];
        f.state_type == StateType::Air || f.pos.y < -1.0
    };
    let selected_type = if defender_airborne_at_contact {
        hitdef.air_type
    } else {
        hitdef.ground_type
    };
    if selected_type == GroundType::None {
        record_hit(sim, attacker, defender);
        {
            let f = &mut sim.fighters[attacker];
            f.hit.hit_pause = hitdef.pausetime.0.max(0);
            f.hit.hit_pause_time = hitdef.pausetime.0.max(0);
        }
        {
            let f = &mut sim.fighters[defender];
            f.hit.hit_pause = hitdef.pausetime.1.max(0);
            f.hit.hit_pause_time = 0;
            f.hit.hit_shake_timer = hitdef.pausetime.1.max(0);
        }
        return;
    }
    // `HitOverride` wins over any hit application path, including projectiles
    // and direct callers (`docs/spec/cns-cmd.md` §4.2).
    if !guarding {
        if let Some(override_) = sim.fighters[defender].matched_override(hitdef) {
            if override_.forceguard {
                apply_hit_at(
                    sim,
                    attacker,
                    defender,
                    hitdef,
                    true,
                    origin,
                    facing,
                    from_fighter,
                );
                return;
            }
            if !override_.keepstate {
                let assets = sim.assets.clone();
                let char_id = sim.fighters[defender].char_id;
                sim.fighters[defender].enter_state(
                    override_.state,
                    &assets.characters[char_id],
                    false,
                );
            }
            record_hit(sim, attacker, defender);
            return;
        }
    }
    if !guarding {
        if from_fighter && hitdef.p1getp2facing != 0 {
            let sign = if hitdef.p1getp2facing < 0 { -1 } else { 1 };
            sim.fighters[attacker].facing = sim.fighters[defender].facing * sign;
        } else if from_fighter && hitdef.p1facing != 0 {
            sim.fighters[attacker].facing = hitdef.p1facing;
        }
        if hitdef.p2facing != 0 {
            sim.fighters[defender].facing = hitdef.p2facing;
        }
    }
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
        if hitdef.p1sprpriority_set {
            sim.fighters[attacker].sprite_priority = hitdef.p1sprpriority;
        }
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
    let mut defender_state_type = sim.fighters[defender].state_type;
    let liedown_time = sim.assets.characters[sim.fighters[defender].char_id]
        .constants
        .get_or("data.liedown.time", 60.0) as i32;

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

    let velocity_tuple = if guarding {
        let v = if defender_airborne {
            hitdef.airguard_velocity
        } else {
            hitdef.guard_velocity.unwrap_or(hitdef.ground_velocity)
        };
        let ctrltime = if defender_airborne {
            hitdef.airguard_ctrltime
        } else {
            hitdef.guard_ctrltime
        };
        (
            v,
            hitdef.guard_hittime.max(1),
            hitdef.guard_slidetime,
            ctrltime,
        )
    } else if defender_airborne {
        (
            hitdef.air_velocity,
            hitdef.air_hittime.max(1),
            hitdef.ground_slidetime,
            hitdef.air_hittime,
        )
    } else if defender_state_type == StateType::Liedown {
        if hitdef.down_velocity.y == 0.0 {
            (
                hitdef.down_velocity,
                hitdef.down_hittime.max(1),
                hitdef.ground_slidetime,
                hitdef.down_hittime,
            )
        } else {
            (
                hitdef.down_velocity,
                hitdef.ground_hittime.max(1),
                hitdef.ground_slidetime,
                hitdef.ground_hittime,
            )
        }
    } else {
        (
            hitdef.ground_velocity,
            hitdef.ground_hittime.max(1),
            hitdef.ground_slidetime,
            hitdef.ground_hittime,
        )
    };
    let corner_push = {
        let stage = &sim.assets.stage;
        let f = &sim.fighters[defender];
        let near_corner = f.pos.x <= stage.left_bound + 24.0 || f.pos.x >= stage.right_bound - 24.0;
        if near_corner {
            let downed = !defender_airborne && defender_state_type == StateType::Liedown;
            match (guarding, defender_airborne, downed) {
                (true, true, _) => hitdef.airguard_cornerpush,
                (true, false, _) => hitdef.guard_cornerpush,
                (false, true, _) => hitdef.air_cornerpush,
                (false, false, true) => hitdef.down_cornerpush,
                (false, false, false) => hitdef.ground_cornerpush,
            }
        } else {
            0.0
        }
    };
    let (velocity, hittime, slidetime, ctrltime) = velocity_tuple;
    let hit_pause = if guarding {
        if hitdef.guard_pausetime_set {
            hitdef.guard_pausetime
        } else {
            hitdef.pausetime
        }
    } else {
        hitdef.pausetime
    };

    if from_fighter && hitdef.p1sprpriority_set {
        sim.fighters[attacker].sprite_priority = hitdef.p1sprpriority;
    }
    {
        let f = &mut sim.fighters[defender];
        f.sprite_priority = hitdef.p2sprpriority;
        f.hit.vars.anim_type = hitdef.anim_type.code();
        f.hit.vars.ground_type = hitdef.ground_type.code();
        f.hit.vars.air_type = hitdef.air_type.code();
        f.hit.vars.hit_type = if defender_airborne {
            hitdef.air_type.code()
        } else {
            hitdef.ground_type.code()
        };
        // Corner push is a world-space position offset in Ikemen, so it is
        // applied to the stored (screen-space) velocity directly.
        f.hit.vars.x_vel = velocity.x * -(attacker_facing as f32) + corner_push;
        f.hit.vars.y_vel = velocity.y;
        f.hit.vars.y_accel = hitdef.yaccel;
        f.hit.vars.hittime = hittime;
        f.hit.vars.slidetime = slidetime;
        f.hit.vars.ctrltime = ctrltime;
        let source_fall = if defender_airborne {
            hitdef.air_fall
        } else {
            hitdef.fall
        };
        let mut fall_flag = source_fall || f.hit.vars.fall;
        if hitdef.forcenofall && !source_fall {
            fall_flag = false;
        }
        f.hit.vars.fall = fall_flag;
        f.hit.vars.fall_recover = hitdef.fall_recover;
        f.hit.vars.fall_time = hitdef.fall_time;
        f.hit.vars.fall_recovertime = hitdef.fall_recovertime;
        if !guarding {
            f.hit.vars.down_recovertime = liedown_time;
            f.hit.vars.fall_kill = hitdef.fall_kill;
            f.hit.vars.fall_envshake_time = hitdef.fall_envshake_time;
            f.hit.vars.fall_envshake_freq = hitdef.fall_envshake_freq;
            f.hit.vars.fall_envshake_ampl = hitdef.fall_envshake_ampl;
            f.hit.vars.fall_envshake_phase = hitdef.fall_envshake_phase;
        }
        f.hit.vars.fall_damage = hitdef.fall_damage;
        f.hit.vars.fall_x_vel = Some(hitdef.fall_xvel * -(attacker_facing as f32));
        f.hit.vars.fall_y_vel = Some(hitdef.fall_yvel);
        let down_launch =
            !guarding && defender_state_type == StateType::Liedown && hitdef.down_velocity.y != 0.0;
        f.hit.down_bounce = down_launch && hitdef.down_bounce;
        if down_launch && !hitdef.down_bounce {
            f.hit.vars.fall_x_vel = None;
            f.hit.vars.fall_y_vel = Some(0.0);
        }
        f.hit.vars.damage = damage;
        f.hit.vars.guarded = guarding;
        f.hit.vars.hitcount += 1;
        f.hit.vars.priority = hitdef.priority;
        f.hit.vars.hit_id = hitdef.id;
        f.hit.vars.chain_id = hitdef.chainid;
        f.hit.guarding = guarding;
        f.hit.hit_pause = hit_pause.1.max(0);
        f.hit.hit_pause_time = 0;
        f.hit.hit_shake_timer = hit_pause.1.max(0);
        let can_ko = if guarding {
            hitdef.guard_kill
        } else {
            hitdef.kill
        };
        let next_life = f.life - damage;
        f.life = if can_ko || f.life <= 0 {
            next_life.max(0)
        } else {
            next_life.max(1)
        };
        if !guarding {
            f.received_hits = f.received_hits.saturating_add(hitdef.numhits.max(1));
            f.received_damage = f.received_damage.saturating_add(damage);
        }
        f.ctrl = false;
        f.attack = Default::default();
        f.hitdef = None;
        if hitdef.palfx_time > 0 {
            f.pal_add = hitdef.palfx_add;
            f.pal_base_add = hitdef.palfx_add;
            f.pal_sinadd = [0.0; 3];
            f.pal_sin_period = 0;
            f.pal_cycle = 0;
            f.pal_mul = hitdef.palfx_mul;
            f.pal_alpha = 1.0;
            f.pal_time = hitdef.palfx_time;
        }
    }

    {
        let f = &mut sim.fighters[defender];
        if !hitdef.mindist.x.is_nan() {
            let bound = origin.x + hitdef.mindist.x * facing as f32;
            if (facing >= 0 && f.pos.x < bound) || (facing < 0 && f.pos.x > bound) {
                f.pos.x = bound;
            }
        }
        if !hitdef.maxdist.x.is_nan() {
            let bound = origin.x + hitdef.maxdist.x * facing as f32;
            if (facing >= 0 && f.pos.x > bound) || (facing < 0 && f.pos.x < bound) {
                f.pos.x = bound;
            }
        }
        if !guarding || defender_airborne {
            if !hitdef.mindist.y.is_nan() {
                let bound = origin.y + hitdef.mindist.y;
                if f.pos.y < bound {
                    f.pos.y = bound;
                }
            }
            if !hitdef.maxdist.y.is_nan() {
                let bound = origin.y + hitdef.maxdist.y;
                if f.pos.y > bound {
                    f.pos.y = bound;
                }
            }
        }
    }

    if !guarding
        && !defender_airborne
        && hitdef.forcestand
        && defender_state_type == StateType::Crouch
    {
        defender_state_type = StateType::Stand;
        sim.fighters[defender].state_type = StateType::Stand;
    }

    let char = &sim.assets.characters[sim.fighters[defender].char_id];
    let mut new_state = if guarding {
        match defender_state_type {
            StateType::Air => 154,
            StateType::Crouch => 152,
            _ => 150,
        }
    } else if defender_airborne {
        5020
    } else if defender_state_type == StateType::Liedown && sim.fighters[defender].pos.y >= 0.0 {
        5080
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
    // `p2stateno` overrides the get-hit state on unguarded hits even when the
    // attacker has no `p1stateno` (`docs/spec/combat.md` §5).
    if !guarding && hitdef.p2stateno >= 0 {
        new_state = hitdef.p2stateno;
    }

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
        let first_contact = f.hit.hit_targets.is_empty();
        f.attack.move_contact = true;
        f.attack.move_hit |= !guarding;
        f.attack.move_guarded |= guarding;
        f.attack.uniq_hit_count += 1;
        // Ikemen adds `numhits` once per swing (on the first contact) and
        // tracks guarded contacts separately.
        if first_contact {
            if guarding {
                f.attack.guard_count += hitdef.numhits.max(1);
            } else {
                f.attack.hit_count += hitdef.numhits.max(1);
            }
        }
        f.hit.hit_targets.push(defender);
        f.hit.hit_pause = hit_pause.0.max(0);
        f.hit.hit_pause_time = hit_pause.0.max(0);
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
            initial_time: 6,
            freq: 60.0,
            ampl: -2.0,
            phase: 0.0,
            mul: 1.0,
            dir: 0.0,
            diradd: 0.0,
            decay: 0.0,
        };
    }
    // An explicit HitDef envshake wins over the generic heavy-hit kick.
    if hitdef.envshake_time > 0 {
        let mut phase = hitdef.envshake_phase;
        if phase.is_nan() {
            phase = if hitdef.envshake_freq >= 90.0 {
                90.0
            } else {
                0.0
            };
        }
        phase = phase.clamp(-180.0, 180.0);
        sim.env_shake = crate::EnvShake {
            time: hitdef.envshake_time,
            initial_time: hitdef.envshake_time,
            freq: hitdef.envshake_freq,
            ampl: hitdef.envshake_ampl as f32,
            phase,
            mul: 1.0,
            dir: 0.0,
            diradd: 0.0,
            decay: 1.0,
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
        if hits <= 0 || sim.projectiles[i].status != crate::projectile::ProjectileStatus::Active {
            continue;
        }
        let defender = sim.opponent_of(owner);
        if defender >= 2 || !sim.fighters[defender].active {
            continue;
        }
        if !affect_team_allows(sim, owner, defender, &hitdef) {
            continue;
        }
        if !chain_gate_allows(sim, defender, &hitdef) {
            continue;
        }
        if blocks_by_filter(sim, defender, &hitdef) {
            continue;
        }
        if !hitflag_allows(sim, defender, &hitdef) {
            continue;
        }
        let defender_airborne = {
            let f = &sim.fighters[defender];
            f.state_type == StateType::Air || f.pos.y < -1.0
        };
        // Projectiles use their own `air.juggle` cost.
        let juggle_cost = hitdef.air_juggle;
        if !juggle_allows(
            sim.fighters[defender].juggle_points,
            sim.fighters[defender].assert_flags,
            false,
            defender_airborne,
            juggle_cost,
            // Each projectile contact is its own activation, so the
            // melee "fresh activation" bypass does not apply.
            true,
        ) {
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

        let guarding = can_guard(sim, defender, &hitdef, pos);
        apply_hit_at(sim, owner, defender, &hitdef, guarding, pos, facing, false);
        sim.fighters[owner].proj_contact_time = 0;
        if guarding {
            sim.fighters[owner].proj_guarded_time = 0;
        } else {
            sim.fighters[owner].proj_hit_time = 0;
        }
        if !defender_airborne && !hitdef.fall
            || juggle_cost <= 0
            || sim.fighters[defender].assert_flags & crate::fighter::ASSERT_NOJUGGLECHECK != 0
        {
        } else if !guarding {
            sim.fighters[defender].juggle_points =
                (sim.fighters[defender].juggle_points - juggle_cost).max(0);
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
