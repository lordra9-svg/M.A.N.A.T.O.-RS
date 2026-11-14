//! State controller implementations.
//!
//! Each `[State ...]` block becomes one [`Controller`]. `execute` is called
//! only when the triggers already passed, so it focuses purely on the action.

use manato_core::{clamp, Vec2};
use manato_formats::{Character, Controller};

use crate::hitdef::HitDef;
use crate::trigger;
use crate::SimState;
use crate::{Effect, EffectBlend, EffectSource, EnvShake, SoundEvent, SoundSource, MAX_POWER};

fn param<'a>(control: &'a Controller, key: &str) -> Option<&'a str> {
    control.get(key)
}

fn num_param(sim: &SimState, who: usize, control: &Controller, key: &str) -> Option<f32> {
    param(control, key).map(|v| trigger::num(sim, who, v))
}

fn int_param(sim: &SimState, who: usize, control: &Controller, key: &str) -> Option<i32> {
    param(control, key).map(|v| trigger::int(sim, who, v))
}

fn str_param<'a>(control: &'a Controller, key: &str) -> Option<&'a str> {
    param(control, key).map(str::trim)
}

fn char_of<'a>(assets: &'a crate::Assets, sim: &SimState, who: usize) -> &'a Character {
    let char_id = sim.fighters[who]
        .state_char_id
        .unwrap_or(sim.fighters[who].char_id);
    &assets.characters[char_id]
}

/// Default `guard.dist` from `[Size] attack.dist` (melee) or
/// `proj.attack.dist` (projectile), both of which default to 160/90.
fn character_guard_dist(char: &Character, projectile: bool) -> i32 {
    let (specific, base, default) = if projectile {
        ("size.proj.attack.dist.x", "size.proj.attack.dist", 90.0)
    } else {
        ("size.attack.dist.x", "size.attack.dist", 160.0)
    };
    char.constants
        .get(specific)
        .or_else(|| char.constants.get(base))
        .unwrap_or(default)
        .max(0.0)
        .round() as i32
}

pub fn execute(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let kind = control.kind.trim().to_ascii_lowercase();
    match kind.as_str() {
        "null" => {}
        "changestate" => change_state(sim, who, control, assets, false),
        "selfstate" => change_state(sim, who, control, assets, true),
        "changeanim" => change_anim(sim, who, control, assets),
        "changeanim2" => change_anim2(sim, who, control, assets),
        "velset" => vel_set(sim, who, control),
        "veladd" => vel_add(sim, who, control),
        "velmul" => vel_mul(sim, who, control),
        "posset" => pos_set(sim, who, control),
        "posadd" => pos_add(sim, who, control),
        "posfreeze" => {
            let value = int_param(sim, who, control, "value").unwrap_or(1);
            sim.fighters[who].pos_freeze = value.max(0);
        }
        "varset" => var_op(sim, who, control, VarOp::Set),
        "varadd" => var_op(sim, who, control, VarOp::Add),
        "varrangeset" => var_range_set(sim, who, control),
        "varrandom" => var_random(sim, who, control),
        "ctrlset" => {
            let value = int_param(sim, who, control, "value").unwrap_or(1) != 0;
            sim.fighters[who].ctrl = value;
        }
        "statetypeset" => state_type_set(sim, who, control),
        "defencemulset" => {
            let value = num_param(sim, who, control, "value").unwrap_or(1.0);
            sim.fighters[who].defence_mul = value;
        }
        "attackmulset" => {
            let value = num_param(sim, who, control, "value").unwrap_or(1.0);
            sim.fighters[who].attack_mul = value;
        }
        "turn" => {
            sim.fighters[who].facing = -sim.fighters[who].facing;
        }
        "poweradd" => {
            let value = num_param(sim, who, control, "value").unwrap_or(0.0);
            sim.fighters[who].power = clamp(sim.fighters[who].power + value, 0.0, MAX_POWER);
        }
        "powerset" => {
            let value = num_param(sim, who, control, "value").unwrap_or(0.0);
            sim.fighters[who].power = clamp(value, 0.0, MAX_POWER);
        }
        "lifeadd" => life_add(sim, who, control),
        "guardpointsadd" => guard_points_add(sim, who, control),
        "guardpointsset" => guard_points_set(sim, who, control),
        "guardbreakset" => guard_break_set(sim, who, control),
        "gethitvarset" => get_hit_var_set(sim, who, control),
        "roundtimeadd" => round_time_add(sim, who, control),
        "roundtimeset" => round_time_set(sim, who, control),
        "victoryquote" => victory_quote(sim, who, control),
        "scoreadd" => score_add(sim, who, control),
        "targetscoreadd" => target_score_add(sim, who, control),
        "redlifeadd" => red_life_add(sim, who, control),
        "redlifeset" => red_life_set(sim, who, control),
        "targetredlifeadd" => target_red_life_add(sim, who, control),
        "mapset" => map_write(sim, who, control, false),
        "mapadd" => map_write(sim, who, control, true),
        "mapreset" => map_reset(sim, who, control),
        "parentmapset" => map_write_relative(sim, who, control, false, Relative::Parent),
        "parentmapadd" => map_write_relative(sim, who, control, true, Relative::Parent),
        "rootmapset" => map_write_relative(sim, who, control, false, Relative::Root),
        "rootmapadd" => map_write_relative(sim, who, control, true, Relative::Root),
        "teammapset" => map_write_relative(sim, who, control, false, Relative::Team),
        "teammapadd" => map_write_relative(sim, who, control, true, Relative::Team),
        "dizzypointsadd" => dizzy_points_add(sim, who, control),
        "dizzypointsset" => dizzy_points_set(sim, who, control),
        "dizzyset" => dizzy_set(sim, who, control),
        "modifyplayer" => modify_player(sim, who, control),
        "lifeset" => life_set(sim, who, control),
        "hitadd" => {
            let value = int_param(sim, who, control, "value").unwrap_or(0);
            let attack = &mut sim.fighters[who].attack;
            attack.hit_count = attack.hit_count.saturating_add(value);
            attack.uniq_hit_count = attack.uniq_hit_count.saturating_add(value);
        }
        "movehitreset" => {
            sim.fighters[who].attack.move_contact = false;
            sim.fighters[who].attack.move_hit = false;
            sim.fighters[who].attack.move_guarded = false;
            sim.fighters[who].attack.move_reversed = false;
            sim.fighters[who].attack.counter_hit = false;
            sim.fighters[who].attack.hit_count = 0;
            sim.fighters[who].attack.uniq_hit_count = 0;
        }
        "playsnd" => play_snd(sim, who, control),
        "stopsnd" => stop_snd(sim, who, control),
        "sndpan" => snd_pan(sim, who, control),
        "modifysnd" => modify_snd(sim, who, control),
        "hitdef" => hit_def(sim, who, control),
        "reversaldef" => reversal_def(sim, who, control),
        "hitvelset" => hit_vel_set(sim, who, control),
        "hitfallset" => hit_fall_set(sim, who, control),
        "hitfallvel" => hit_fall_vel(sim, who, control),
        "hitfalldamage" => {
            let value = int_param(sim, who, control, "value").unwrap_or(0);
            sim.fighters[who].hit.vars.fall_damage = value;
        }
        "nothitby" | "hitby" => hit_filter(sim, who, control, kind == "nothitby"),
        "assertspecial" => assert_special(sim, who, control),
        "sprpriority" => {
            let value = int_param(sim, who, control, "value").unwrap_or(0);
            sim.fighters[who].sprite_priority = value;
        }
        "width" => width(sim, who, control),
        "height" => height(sim, who, control),
        "camera" => camera(sim, who, control),
        "modifyshadow" => modify_shadow(sim, who, control),
        "modifyreflection" => modify_reflection(sim, who, control),
        "angleset" => {
            let value = num_param(sim, who, control, "value").unwrap_or(0.0);
            sim.fighters[who].draw_angle = value;
        }
        "angleadd" => {
            let value = num_param(sim, who, control, "value").unwrap_or(0.0);
            sim.fighters[who].draw_angle += value;
        }
        "anglemul" => {
            let value = num_param(sim, who, control, "value").unwrap_or(1.0);
            sim.fighters[who].draw_angle *= value;
        }
        "angledraw" => {
            let value = num_param(sim, who, control, "value");
            let scale = control.get("scale").map(|raw| {
                let mut parts = raw.split(',');
                let x = parts
                    .next()
                    .and_then(|v| v.trim().parse::<f32>().ok())
                    .unwrap_or(1.0);
                let y = parts
                    .next()
                    .and_then(|v| v.trim().parse::<f32>().ok())
                    .unwrap_or(x);
                Vec2::new(x, y)
            });
            let fighter = &mut sim.fighters[who];
            fighter.angle_draw = true;
            if value.is_some() {
                fighter.angle_draw_value = value;
            }
            if scale.is_some() {
                fighter.angle_draw_scale = scale;
            }
        }
        "attackdist" => {
            let value = int_param(sim, who, control, "value").unwrap_or(0);
            if let Some(hitdef) = sim.fighters[who].hitdef.as_mut() {
                hitdef.guard_dist = value.max(0);
            }
        }
        "modifyhitdef" => {
            if let Some(hitdef) = sim.fighters[who].hitdef.as_mut() {
                hitdef.apply_modifiers(control);
            }
        }
        "modifyreversaldef" => {
            if let Some(reversal) = sim.fighters[who].reversal.as_mut() {
                if let Some(attr) = control.get("reversal.attr") {
                    reversal.attr = crate::hitdef::HitAttr::parse(attr);
                }
                if let Some(flag) = control.get("reversal.guardflag") {
                    reversal.reversal_guardflag = flag.to_ascii_uppercase();
                }
                if let Some(flag) = control.get("reversal.guardflag.not") {
                    reversal.reversal_guardflag_not = flag.to_ascii_uppercase();
                }
                reversal.apply_modifiers(control);
            }
        }
        "screenbound" => {
            let (x, y) = control.get_pair("value").unwrap_or((1.0, 1.0));
            sim.fighters[who].screen_bound = (x != 0.0, y != 0.0);
        }
        "offset" => {
            let x = num_param(sim, who, control, "x").unwrap_or(0.0);
            let y = num_param(sim, who, control, "y").unwrap_or(0.0);
            sim.fighters[who].draw_offset = Vec2::new(x, y);
        }
        "envcolor" => {
            let raw = control.get("value").unwrap_or("0,0,0");
            let mut parts = raw.split(',');
            let r = parts
                .next()
                .and_then(|value| trigger::num(sim, who, value).into())
                .unwrap_or(0.0);
            let g = parts
                .next()
                .and_then(|value| trigger::num(sim, who, value).into())
                .unwrap_or(0.0);
            let b = parts
                .next()
                .and_then(|value| trigger::num(sim, who, value).into())
                .unwrap_or(0.0);
            let time = int_param(sim, who, control, "time").unwrap_or(1);
            // `time = 0` leaves no tint; a negative time stays until replaced.
            sim.env_color = if time == 0 {
                None
            } else {
                Some(crate::EnvColor {
                    color: [r / 255.0, g / 255.0, b / 255.0, 1.0],
                    time,
                    under: int_param(sim, who, control, "under").unwrap_or(0) != 0,
                })
            };
        }
        "fallenvshake" => fall_env_shake(sim, who),
        "envshake" => env_shake(sim, who, control),
        "zoom" => zoom(sim, who, control),
        "lifebaraction" => life_bar_action(sim, who, control),
        "explod" => explod(sim, who, control, assets),
        "modifyexplod" => modify_explod(sim, who, control, assets),
        "removeexplod" => remove_explod(sim, who, control),
        "makedust" => make_dust(sim, who, control),
        "explodbindtime" => explod_bind_time(sim, who, control),
        "modifystagevar" => modify_stage_var(sim, who, control),
        "transformsprite" => transform_sprite(sim, who, control),
        "transformclsn" => transform_clsn(sim, who, control),
        "gamemakeanim" => game_make_anim(sim, who, control),
        "pause" => pause(sim, who, control),
        "superpause" => super_pause(sim, who, control, assets),
        "targetbind" => target_bind(sim, who, control),
        "targetdrop" => {
            sim.fighters[who].target = None;
            sim.fighters[who].target_bind = None;
        }
        "targetadd" => sim.fighters[who].target = Some(sim.other(who)),
        "hitoverride" => hit_override(sim, who, control),
        "targetfacing" => target_facing(sim, who, control),
        "targetstate" => target_state(sim, who, control, assets),
        "targetlifeadd" => target_life_add(sim, who, control),
        "targetpoweradd" => target_power_add(sim, who, control),
        "targetguardpointsadd" => target_guard_points_add(sim, who, control),
        "targetdizzypointsadd" => target_dizzy_points_add(sim, who, control),
        "targetguardpointsset" => target_guard_points_set(sim, who, control),
        "targetdizzypointsset" => target_dizzy_points_set(sim, who, control),
        "targetvelset" => target_vel_set(sim, who, control),
        "targetveladd" => target_vel_add(sim, who, control),
        "afterimage" => after_image(sim, who, control),
        "afterimagetime" => {
            let time = int_param(sim, who, control, "time");
            if let Some(time) = time {
                // Ikemen maps `time = 1` to 0, and a zero timer disables the
                // emitter. Negative values keep it running forever.
                let time = if time == 1 { 0 } else { time };
                if let Some(state) = sim.fighters[who].after_image.as_mut() {
                    // Zero disables emission but keeps the state so a later
                    // AfterImageTime can revive it, like Ikemen's persistent
                    // `aimg` object.
                    state.time = time;
                    state.tick = 0;
                }
            }
        }
        "palfx" => pal_fx(sim, who, control),
        "trans" => trans(sim, who, control),
        "remappal" | "forcefeedback" => {}
        "projectile" => projectile(sim, who, control, assets),
        "modifyprojectile" => modify_projectile(sim, who, control, assets),
        "playerpush" => {
            let value = int_param(sim, who, control, "value").unwrap_or(1) != 0;
            sim.fighters[who].player_push = value;
        }
        "gravity" => {
            let value = num_param(sim, who, control, "value");
            sim.fighters[who].gravity = value;
        }
        "helper" => helper(sim, who, control, assets),
        "destroyself" => sim.fighters[who].helper_destroy = true,
        "bindtoroot" => bind_to(sim, who, control, BindTarget::Root),
        "bindtoparent" => bind_to(sim, who, control, BindTarget::Parent),
        "bindtotarget" => bind_to(sim, who, control, BindTarget::Target),
        "parentvarset" => var_op(sim, sim.parent_of(who), control, VarOp::Set),
        "parentvaradd" => var_op(sim, sim.parent_of(who), control, VarOp::Add),
        "rootvarset" => var_op(sim, sim.root_of(who), control, VarOp::Set),
        "rootvaradd" => var_op(sim, sim.root_of(who), control, VarOp::Add),
        _ => {
            log::debug!("unimplemented state controller: {kind}");
        }
    }
}

fn change_state(
    sim: &mut SimState,
    who: usize,
    control: &Controller,
    assets: &crate::Assets,
    self_state: bool,
) {
    let current = sim.fighters[who].state_no;
    let Some(value) = int_param(sim, who, control, "value") else {
        return;
    };
    if value < 0 {
        return;
    }
    let ctrl = int_param(sim, who, control, "ctrl").map(|v| v != 0);
    let anim = int_param(sim, who, control, "anim");
    if self_state {
        sim.fighters[who].state_char_id = None;
        sim.fighters[who].anim_char_id = None;
    }
    let char = if self_state {
        &assets.characters[sim.fighters[who].char_id]
    } else {
        char_of(assets, sim, who)
    };
    sim.fighters[who].change_state_override(value, char, ctrl, anim, None, false);
    if value != current {
        // Reset per-state animation bookkeeping happens in enter_state.
    }
}

fn change_anim(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let Some(value) = int_param(sim, who, control, "value") else {
        return;
    };
    let elem = int_param(sim, who, control, "elem");
    let char = char_of(assets, sim, who);
    sim.fighters[who].set_anim(value, char, elem);
}

/// `ChangeAnim2`: like `ChangeAnim`, but the animation is read from another
/// player's bank (`readplayerid`, defaulting to the state owner).
fn change_anim2(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let Some(value) = int_param(sim, who, control, "value") else {
        return;
    };
    let elem = int_param(sim, who, control, "elem");
    let player = int_param(sim, who, control, "readplayerid")
        .map(|id| id.clamp(0, 1) as usize)
        .unwrap_or_else(|| sim.root_of(who));
    let anim_char_id = sim.fighters[player].char_id;
    let anim_char = &assets.characters[anim_char_id];
    sim.fighters[who].set_anim_from(value, anim_char, elem, anim_char_id);
}

fn vel_set(sim: &mut SimState, who: usize, control: &Controller) {
    let x = num_param(sim, who, control, "x");
    let y = num_param(sim, who, control, "y");
    let facing = sim.fighters[who].facing as f32;
    if let Some(x) = x {
        sim.fighters[who].vel.x = x * facing;
    }
    if let Some(y) = y {
        sim.fighters[who].vel.y = y;
    }
}

fn vel_add(sim: &mut SimState, who: usize, control: &Controller) {
    let x = num_param(sim, who, control, "x");
    let y = num_param(sim, who, control, "y");
    let facing = sim.fighters[who].facing as f32;
    if let Some(x) = x {
        sim.fighters[who].vel.x += x * facing;
    }
    if let Some(y) = y {
        sim.fighters[who].vel.y += y;
    }
}

fn vel_mul(sim: &mut SimState, who: usize, control: &Controller) {
    let x = num_param(sim, who, control, "x");
    let y = num_param(sim, who, control, "y");
    let facing = sim.fighters[who].facing as f32;
    if let Some(x) = x {
        // Store velocities world-relative but accept facing-relative factors.
        let _ = facing;
        sim.fighters[who].vel.x *= x;
    }
    if let Some(y) = y {
        sim.fighters[who].vel.y *= y;
    }
}

fn pos_set(sim: &mut SimState, who: usize, control: &Controller) {
    let x = num_param(sim, who, control, "x");
    let y = num_param(sim, who, control, "y");
    if let Some(x) = x {
        sim.fighters[who].pos.x = x;
    }
    if let Some(y) = y {
        sim.fighters[who].pos.y = y;
    }
}

fn pos_add(sim: &mut SimState, who: usize, control: &Controller) {
    let x = num_param(sim, who, control, "x");
    let y = num_param(sim, who, control, "y");
    let facing = sim.fighters[who].facing as f32;
    if let Some(x) = x {
        sim.fighters[who].pos.x += x * facing;
    }
    if let Some(y) = y {
        sim.fighters[who].pos.y += y;
    }
}

enum VarOp {
    Set,
    Add,
}

fn var_op(sim: &mut SimState, who: usize, control: &Controller, op: VarOp) {
    // `var(n) = expr` style.
    let mut assignments: Vec<(usize, i32)> = Vec::new();
    let mut fassignments: Vec<(usize, f32)> = Vec::new();
    let mut index: Option<usize> = None;
    let mut findex: Option<usize> = None;
    for (key, value) in &control.params {
        let lower = key.trim().to_ascii_lowercase();
        if let Some(n) = bracket_index(&lower, "var") {
            assignments.push((n.min(59), trigger::int(sim, who, value)));
        } else if let Some(n) = bracket_index(&lower, "fvar") {
            fassignments.push((n.min(39), trigger::num(sim, who, value)));
        } else if lower == "v" {
            index = Some(trigger::int(sim, who, value).clamp(0, 59) as usize);
        } else if lower == "fv" {
            findex = Some(trigger::int(sim, who, value).clamp(0, 39) as usize);
        }
    }
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let fvalue = num_param(sim, who, control, "value").unwrap_or(0.0);
    for (idx, val) in assignments {
        let slot = &mut sim.fighters[who].vars[idx];
        match op {
            VarOp::Set => *slot = val,
            VarOp::Add => *slot = slot.saturating_add(val),
        }
    }
    if let Some(idx) = index {
        let slot = &mut sim.fighters[who].vars[idx];
        match op {
            VarOp::Set => *slot = value,
            VarOp::Add => *slot = slot.saturating_add(value),
        }
    }
    for (idx, val) in fassignments {
        let slot = &mut sim.fighters[who].fvars[idx];
        match op {
            VarOp::Set => *slot = val,
            VarOp::Add => *slot += val,
        }
    }
    if let Some(idx) = findex {
        let slot = &mut sim.fighters[who].fvars[idx];
        match op {
            VarOp::Set => *slot = fvalue,
            VarOp::Add => *slot += fvalue,
        }
    }
}

fn var_range_set(sim: &mut SimState, who: usize, control: &Controller) {
    let Some(value) = int_param(sim, who, control, "value") else {
        return;
    };
    let first = int_param(sim, who, control, "first").unwrap_or(0);
    let last = int_param(sim, who, control, "last").unwrap_or(first);
    let (lo, hi) = if first <= last {
        (first, last)
    } else {
        (last, first)
    };
    for idx in lo.max(0)..=hi.min(59) {
        sim.fighters[who].vars[idx as usize] = value;
    }
}

/// `VarRandom`: stores a uniformly random value in `[range.0, range.1]`.
///
/// `v` selects an integer variable (`0..=59`) or a float variable
/// (`60..=119`). Float results are divided by 100, matching M.U.G.E.N's
/// hundredths convention. The default range is `0, 1000`.
fn var_random(sim: &mut SimState, who: usize, control: &Controller) {
    let Some(v) = int_param(sim, who, control, "v") else {
        return;
    };
    let (lo, hi) = control.get_pair_i32("range").unwrap_or((0, 1000));
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    let value = sim.rng.range_i32(lo, hi);
    if v >= 60 {
        let index = ((v - 60).clamp(0, 39)) as usize;
        sim.fighters[who].fvars[index] = value as f32 / 100.0;
    } else {
        let index = v.clamp(0, 59) as usize;
        sim.fighters[who].vars[index] = value;
    }
}

fn state_type_set(sim: &mut SimState, who: usize, control: &Controller) {
    use crate::fighter::{MoveType, Physics, StateType};
    if let Some(v) = param(control, "statetype").and_then(StateType::parse) {
        sim.fighters[who].prev_state_type = sim.fighters[who].state_type;
        sim.fighters[who].state_type = v;
    }
    if let Some(v) = param(control, "movetype").and_then(MoveType::parse) {
        sim.fighters[who].prev_move_type = sim.fighters[who].move_type;
        sim.fighters[who].move_type = v;
    }
    if let Some(v) = param(control, "physics").and_then(Physics::parse) {
        sim.fighters[who].physics = v;
    }
}

fn reversal_def(sim: &mut SimState, who: usize, control: &Controller) {
    let character = &sim.assets.characters[sim.fighters[who].char_id];
    let mut hitdef = HitDef::from_controller_for(
        control,
        sim.hit_activation,
        character.localcoord(),
        character_guard_dist(character, false),
    );
    // ReversalDef uses `reversal.attr`; accept legacy `attr` as a fallback.
    if let Some(attr) = control.get("reversal.attr") {
        hitdef.attr = crate::hitdef::HitAttr::parse(attr);
    }
    let time = int_param(sim, who, control, "time").unwrap_or(1).max(1);
    sim.fighters[who].reversal = Some(hitdef);
    sim.fighters[who].reversal_time = time;
}

fn trans(sim: &mut SimState, who: usize, control: &Controller) {
    let kind = str_param(control, "trans")
        .unwrap_or("default")
        .to_ascii_lowercase();
    // Ikemen-GO's documented trans defaults, used by the raw
    // `Alpha.Source`/`Alpha.Dest` triggers.
    let (mut source, mut dest) = match kind.as_str() {
        "add" => (255, 255),
        "add1" => (255, 128),
        "sub" | "subadd" | "sub1" => (255, 255),
        _ => (255, 0),
    };
    // `alpha` is either a single 0..255 value or a `source, dest` pair; the
    // destination byte (0..256) supplies opacity, matching explod trans.
    let alpha = control.get("alpha").map(|raw| {
        let parts: Vec<f32> = raw
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        match parts.as_slice() {
            [src, dst, ..] => {
                source = *src as i32;
                dest = *dst as i32;
                (dst / 256.0).clamp(0.0, 1.0)
            }
            [src, ..] => {
                source = *src as i32;
                (src / 255.0).clamp(0.0, 1.0)
            }
            [] => 1.0,
        }
    });
    let fighter = &mut sim.fighters[who];
    fighter.trans_alpha_source = source.clamp(0, 255);
    fighter.trans_alpha_dest = dest.clamp(0, 255);
    // Only `addalpha`/`alpha` use the alpha parameter for rendering; the
    // blend-only variants stay fully opaque (`docs/spec/render.md` semantics).
    fighter.trans_alpha = match kind.as_str() {
        "addalpha" | "alpha" => alpha.unwrap_or(1.0),
        _ => 1.0,
    };
}

fn after_image(sim: &mut SimState, who: usize, control: &Controller) {
    let time = int_param(sim, who, control, "time").unwrap_or(1);
    let interval = int_param(sim, who, control, "timegap").unwrap_or(1).max(1);
    let life = int_param(sim, who, control, "length").unwrap_or(10).max(1);
    let add = str_param(control, "trans")
        .map(|value| {
            let value = value.to_ascii_lowercase();
            value == "add" || value == "add1"
        })
        .unwrap_or(false);
    // Negative time runs forever; zero disables emission but keeps the state.
    sim.fighters[who].after_image = Some(crate::fighter::AfterImageState {
        time,
        interval,
        tick: 0,
        life,
        alpha: 0.45,
        add,
    });
}

fn parse_triplet(sim: &SimState, who: usize, value: &str) -> Option<[f32; 3]> {
    let mut parts = value.split(',');
    let r = trigger::num(sim, who, parts.next()?.trim());
    let g = trigger::num(sim, who, parts.next()?.trim());
    let b = trigger::num(sim, who, parts.next()?.trim());
    Some([r, g, b])
}

fn pal_fx(sim: &mut SimState, who: usize, control: &Controller) {
    let time = int_param(sim, who, control, "time").unwrap_or(1);
    if time == 0 {
        // `time = 0` disables the effect entirely.
        let fighter = &mut sim.fighters[who];
        fighter.pal_add = [0.0; 3];
        fighter.pal_base_add = [0.0; 3];
        fighter.pal_sinadd = [0.0; 3];
        fighter.pal_sin_period = 0;
        fighter.pal_cycle = 0;
        fighter.pal_mul = [1.0; 3];
        fighter.pal_alpha = 1.0;
        fighter.pal_time = 0;
        return;
    }
    if let Some(value) = control.get("add") {
        if let Some(add) = parse_triplet(sim, who, value) {
            sim.fighters[who].pal_add = add;
            sim.fighters[who].pal_base_add = add;
        }
    }
    if let Some(value) = control.get("sinadd") {
        let parts: Vec<f32> = value
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        if parts.len() >= 3 {
            let period = parts.get(3).copied().unwrap_or(0.0) as i32;
            let side = if period < 0 { -1.0 } else { 1.0 };
            sim.fighters[who].pal_sinadd = [parts[0] * side, parts[1] * side, parts[2] * side];
            sim.fighters[who].pal_sin_period = period.abs();
            sim.fighters[who].pal_cycle = 0;
            sim.fighters[who].pal_add = sim.fighters[who].pal_base_add;
        }
    }
    if let Some(value) = control.get("mul") {
        if let Some(mul) = parse_triplet(sim, who, value) {
            // `PalFX` documents `mul` in 0..256 units with 256 as identity;
            // store the normalized multiplier used by rendering and triggers.
            sim.fighters[who].pal_mul = [mul[0] / 256.0, mul[1] / 256.0, mul[2] / 256.0];
        }
    }
    if let Some(value) = control.get("alpha") {
        sim.fighters[who].pal_alpha = trigger::num(sim, who, value).clamp(0.0, 1.0);
    }
    sim.fighters[who].pal_time = time;
}

enum BindTarget {
    Root,
    Parent,
    Target,
}

fn bind_to(sim: &mut SimState, who: usize, control: &Controller, target: BindTarget) {
    let anchor = match target {
        BindTarget::Root => sim.root_of(who),
        BindTarget::Parent => sim.parent_of(who),
        BindTarget::Target => sim.fighters[who].target.unwrap_or(who),
    };
    if anchor == who {
        return;
    }
    let (px, py) = control.get_pair("pos").unwrap_or((0.0, 0.0));
    let facing = sim.fighters[anchor].facing;
    let base = sim.fighters[anchor].pos;
    sim.fighters[who].pos = Vec2::new(base.x + px * facing as f32, base.y + py);
}

fn helper(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    if sim.helper_count() >= 32 {
        log::warn!("helper cap reached; skipping Helper");
        return;
    }
    let char_id = sim.fighters[who].char_id;
    let character = &assets.characters[char_id];
    let stateno = int_param(sim, who, control, "stateno").unwrap_or(0);
    if character.state(stateno).is_none() {
        log::warn!("Helper uses missing state {stateno}");
        return;
    }
    let id = int_param(sim, who, control, "id").unwrap_or(0);
    let (px, py) = control.get_pair("pos").unwrap_or((0.0, 0.0));
    let postype = str_param(control, "postype")
        .unwrap_or("p1")
        .to_ascii_lowercase();
    let facing = int_param(sim, who, control, "facing").unwrap_or(sim.fighters[who].facing);
    let owner_pos = sim.fighters[who].pos;
    let opponent = sim.opponent_of(who);
    let left_edge = sim.camera.x - manato_core::GAME_WIDTH * 0.5;
    let right_edge = sim.camera.x + manato_core::GAME_WIDTH * 0.5;
    let base = match postype.as_str() {
        "p2" => sim.fighters[opponent].pos,
        "front" => Vec2::new(if facing > 0 { right_edge } else { left_edge }, owner_pos.y),
        "back" => Vec2::new(if facing > 0 { left_edge } else { right_edge }, owner_pos.y),
        "left" => Vec2::new(left_edge, owner_pos.y),
        "right" => Vec2::new(right_edge, owner_pos.y),
        _ => owner_pos,
    };
    let pos = Vec2::new(base.x + px * facing as f32, base.y + py);
    let mut entity = crate::fighter::Fighter::new(
        character,
        char_id,
        sim.fighters[who].side,
        sim.fighters[who].slot,
        pos,
        facing,
    );
    entity.owner = Some(who);
    entity.parent = Some(who);
    entity.root = Some(sim.root_of(who));
    entity.ai_level = sim.fighters[who].ai_level;
    entity.helper_id = id;
    entity.helper_name = Some(
        str_param(control, "name")
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}'s helper", character.name())),
    );
    if let Some((vx, vy)) = control.get_pair("velocity") {
        entity.vel = Vec2::new(vx * facing as f32, vy);
    }
    entity.helper_remove_time = int_param(sim, who, control, "removetime").unwrap_or(-1);
    entity.size_xscale = num_param(sim, who, control, "size.xscale")
        .unwrap_or(1.0)
        .max(0.01);
    entity.size_yscale = num_param(sim, who, control, "size.yscale")
        .unwrap_or(1.0)
        .max(0.01);
    entity.keyctrl = int_param(sim, who, control, "keyctrl")
        .unwrap_or(0)
        .clamp(0, 7) as u8;
    entity.helper_type_player = str_param(control, "helpertype")
        .map(|value| value.eq_ignore_ascii_case("player"))
        .unwrap_or(false);
    entity.enter_state(stateno, character, false);
    sim.fighters.push(entity);
    // Each created fighter receives the next 1-based unique ID.
    sim.last_player_id = sim.fighters.len() as i32;
}

fn hit_override(sim: &mut SimState, who: usize, control: &Controller) {
    let Some(attr) = str_param(control, "attr") else {
        return;
    };
    let state = int_param(sim, who, control, "stateno")
        .or_else(|| int_param(sim, who, control, "state"))
        .unwrap_or(0);
    // M.U.G.E.N normalizes 0 and values below -1 to 1; -1 is infinite.
    let raw = int_param(sim, who, control, "time").unwrap_or(1);
    let time = if raw == 0 || raw < -1 { 1 } else { raw };
    let keepstate = int_param(sim, who, control, "keepstate").unwrap_or(0) != 0;
    let forceguard = int_param(sim, who, control, "forceguard").unwrap_or(0) != 0;
    let slot = int_param(sim, who, control, "slot")
        .unwrap_or(0)
        .clamp(0, 7) as usize;
    sim.fighters[who].hit_overrides[slot] = Some(crate::fighter::HitOverride {
        attr: attr.to_ascii_uppercase(),
        state,
        time,
        keepstate,
        forceguard,
    });
}

fn life_add(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let kill = int_param(sim, who, control, "kill").unwrap_or(1) != 0;
    let fighter = &mut sim.fighters[who];
    fighter.life = (fighter.life.saturating_add(value)).clamp(0, fighter.life_max);
    if !kill && fighter.life <= 0 {
        fighter.life = 1;
    }
    fighter.clamp_red_life();
}

fn life_set(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let kill = int_param(sim, who, control, "kill").unwrap_or(1) != 0;
    let fighter = &mut sim.fighters[who];
    fighter.life = value.clamp(0, fighter.life_max);
    if !kill && fighter.life <= 0 {
        fighter.life = 1;
    }
    fighter.clamp_red_life();
}

fn guard_points_add(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let absolute = int_param(sim, who, control, "absolute").unwrap_or(0) != 0;
    let fighter = &mut sim.fighters[who];
    if fighter.guard_max <= 0 || value == 0 {
        return;
    }
    let add = if absolute {
        value as f32
    } else {
        value as f32 / fighter.defence_mul.max(0.01)
    };
    fighter.guard_points = (fighter.guard_points + add.round() as i32).clamp(0, fighter.guard_max);
}

fn guard_points_set(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let fighter = &mut sim.fighters[who];
    if fighter.guard_max <= 0 {
        return;
    }
    fighter.guard_points = value.clamp(0, fighter.guard_max);
}

fn guard_break_set(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0) != 0;
    sim.fighters[who].guard_break = value;
}

/// `GetHitVarSet` writes directly into the fighter's stored GetHitVar block.
/// Only fields the engine models are accepted; unknown parameters are ignored.
fn get_hit_var_set(sim: &mut SimState, who: usize, control: &Controller) {
    let i = |key: &str| int_param(sim, who, control, key);
    let n = |key: &str| num_param(sim, who, control, key);
    let (xvel, yvel, yaccel) = (n("xvel"), n("yvel"), n("yaccel"));
    let (hittime, slidetime, ctrltime) = (i("hittime"), i("slidetime"), i("ctrltime"));
    let (damage, hitcount, fallcount) = (i("damage"), i("hitcount"), i("fallcount"));
    let (ground_type, air_type, anim_type) = (i("groundtype"), i("airtype"), i("animtype"));
    let guarded = i("guarded");
    let fall = i("fall");
    let (fall_damage, fall_kill, fall_recover, fall_recovertime) = (
        i("fall.damage"),
        i("fall.kill"),
        i("fall.recover"),
        i("fall.recovertime"),
    );
    let (fall_xvel, fall_yvel) = (n("fall.xvel"), n("fall.yvel"));
    let (env_time, env_freq, env_ampl, env_phase) = (
        i("fall.envshake.time"),
        n("fall.envshake.freq"),
        i("fall.envshake.ampl"),
        n("fall.envshake.phase"),
    );
    let (down_recover, down_recovertime) = (i("down.recover"), i("down.recovertime"));
    let (guardpoints, dizzypoints, hitshaketime) =
        (i("guardpoints"), i("dizzypoints"), i("hitshaketime"));

    let f = &mut sim.fighters[who];
    let vars = &mut f.hit.vars;
    if let Some(value) = xvel {
        vars.x_vel = value;
    }
    if let Some(value) = yvel {
        vars.y_vel = value;
    }
    if let Some(value) = yaccel {
        vars.y_accel = value;
    }
    if let Some(value) = hittime {
        vars.hittime = value;
    }
    if let Some(value) = slidetime {
        vars.slidetime = value;
    }
    if let Some(value) = ctrltime {
        vars.ctrltime = value;
    }
    if let Some(value) = damage {
        vars.damage = value;
    }
    if let Some(value) = hitcount {
        vars.hitcount = value;
    }
    if let Some(value) = fallcount {
        vars.fallcount = value;
    }
    if let Some(value) = ground_type {
        vars.ground_type = value;
    }
    if let Some(value) = air_type {
        vars.air_type = value;
    }
    if let Some(value) = anim_type {
        vars.anim_type = value;
    }
    if let Some(value) = guarded {
        vars.guarded = value != 0;
    }
    if let Some(value) = fall {
        vars.fall = value != 0;
    }
    if let Some(value) = fall_damage {
        vars.fall_damage = value;
    }
    if let Some(value) = fall_kill {
        vars.fall_kill = value != 0;
    }
    if let Some(value) = fall_recover {
        vars.fall_recover = value != 0;
    }
    if let Some(value) = fall_recovertime {
        vars.fall_recovertime = value;
    }
    if let Some(value) = fall_xvel {
        vars.fall_x_vel = Some(value);
    }
    if let Some(value) = fall_yvel {
        vars.fall_y_vel = Some(value);
    }
    if let Some(value) = env_time {
        vars.fall_envshake_time = value;
    }
    if let Some(value) = env_freq {
        vars.fall_envshake_freq = value;
    }
    if let Some(value) = env_ampl {
        vars.fall_envshake_ampl = value;
    }
    if let Some(value) = env_phase {
        vars.fall_envshake_phase = value;
    }
    if let Some(value) = down_recover {
        vars.down_recover = value != 0;
    }
    if let Some(value) = down_recovertime {
        vars.down_recovertime = value;
    }
    // Our GetHitVar exposes the live meters, so mirror that decision here.
    if let Some(value) = guardpoints {
        f.guard_points = value.clamp(0, f.guard_max);
    }
    if let Some(value) = dizzypoints {
        f.dizzy_points = value.clamp(0, f.dizzy_max);
    }
    if let Some(value) = hitshaketime {
        f.hit.hit_shake_timer = value.max(0);
    }
}

/// `RoundTimeAdd`/`RoundTimeSet` operate on the internal tick timer, clamped
/// to the round's configured duration. Ikemen's equivalent controllers use the
/// same tick units.
fn round_time_add(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let max = (sim.round.round_time * manato_core::TICKS_PER_SECOND as i32).max(0);
    let timer = (sim.round.timer + value).clamp(0, max);
    sim.round.timer = timer;
}

fn round_time_set(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let max = (sim.round.round_time * manato_core::TICKS_PER_SECOND as i32).max(0);
    sim.round.timer = value.clamp(0, max);
}

fn victory_quote(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(-1);
    sim.fighters[who].win_quote = value;
}

fn score_add(sim: &mut SimState, who: usize, control: &Controller) {
    let value = num_param(sim, who, control, "value").unwrap_or(0.0);
    let fighter = &sim.fighters[who];
    if value == 0.0 || fighter.owner.is_some() {
        return;
    }
    let side = fighter.side.min(1);
    sim.score[side] += value;
}

fn target_score_add(sim: &mut SimState, who: usize, control: &Controller) {
    let value = num_param(sim, who, control, "value").unwrap_or(0.0);
    let Some(target) = sim.fighters[who].target else {
        return;
    };
    let fighter = &sim.fighters[target];
    if value == 0.0 || fighter.owner.is_some() {
        return;
    }
    let side = fighter.side.min(1);
    sim.score[side] += value;
}

/// Resolves a map name from `map = ...` or a `map(name) = ...` key.
fn map_key(name: &str) -> String {
    name.trim().trim_matches('"').trim().to_ascii_lowercase()
}

fn map_assignment(
    sim: &SimState,
    who: usize,
    control: &Controller,
) -> Option<(String, crate::expr::Value)> {
    let mut name: Option<String> = None;
    let mut assigned: Option<String> = None;
    for (key, value) in &control.params {
        if key == "map" {
            name = Some(map_key(value));
        } else if let Some(inner) = key
            .strip_prefix("map(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            name = Some(map_key(inner));
            assigned = Some(value.clone());
        }
    }
    let value_expr = control
        .get("value")
        .map(str::to_string)
        .or(assigned)
        .unwrap_or_default();
    let name = name.filter(|name| !name.is_empty())?;
    Some((name, trigger::eval_str(sim, who, &value_expr)))
}

fn apply_map(sim: &mut SimState, who: usize, name: String, value: crate::expr::Value, add: bool) {
    let map = &mut sim.fighters[who].map;
    if !add {
        map.insert(name, value);
        return;
    }
    let Some(previous) = map.get(&name).cloned() else {
        map.insert(name, value);
        return;
    };
    if matches!(previous, crate::expr::Value::Str(_)) || matches!(value, crate::expr::Value::Str(_))
    {
        return;
    }
    let sum = match (previous, value) {
        (crate::expr::Value::Int(a), crate::expr::Value::Int(b)) => {
            crate::expr::Value::Int(a.saturating_add(b))
        }
        (a, b) => crate::expr::Value::Float(a.as_f32() + b.as_f32()),
    };
    map.insert(name, sum);
}

#[derive(Clone, Copy)]
enum Relative {
    Parent,
    Root,
    Team,
}

fn map_write_relative(
    sim: &mut SimState,
    who: usize,
    control: &Controller,
    add: bool,
    relative: Relative,
) {
    let Some((name, value)) = map_assignment(sim, who, control) else {
        return;
    };
    match relative {
        Relative::Parent => {
            let dest = sim.parent_of(who);
            apply_map(sim, dest, name, value, add);
        }
        Relative::Root => {
            let dest = sim.root_of(who);
            apply_map(sim, dest, name, value, add);
        }
        Relative::Team => {
            let side = sim.fighters[who].side;
            let destinations: Vec<usize> = sim
                .fighters
                .iter()
                .enumerate()
                .filter(|(_, fighter)| fighter.owner.is_none() && fighter.side == side)
                .map(|(index, _)| index)
                .collect();
            for dest in destinations {
                apply_map(sim, dest, name.clone(), value.clone(), add);
            }
        }
    }
}

fn map_write(sim: &mut SimState, who: usize, control: &Controller, add: bool) {
    let Some((name, value)) = map_assignment(sim, who, control) else {
        return;
    };
    apply_map(sim, who, name, value, add);
}

fn map_reset(sim: &mut SimState, who: usize, control: &Controller) {
    let excludes: Vec<String> = control
        .params
        .iter()
        .filter(|(key, _)| key == "exclude" || key.starts_with("exclude"))
        .map(|(_, value)| map_key(value))
        .filter(|value| !value.is_empty())
        .collect();
    let map = &mut sim.fighters[who].map;
    if excludes.is_empty() {
        map.clear();
    } else {
        map.retain(|key, _| !excludes.iter().any(|exclude| key.contains(exclude)));
    }
}

fn red_life_add(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let absolute = int_param(sim, who, control, "absolute").unwrap_or(0) != 0;
    let fighter = &mut sim.fighters[who];
    if value == 0 {
        return;
    }
    let add = if absolute {
        value
    } else {
        (value as f32 / fighter.defence_mul.max(0.01)).round() as i32
    };
    fighter.red_life = fighter.red_life.saturating_add(add);
    fighter.clamp_red_life();
}

fn red_life_set(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let fighter = &mut sim.fighters[who];
    fighter.red_life = value;
    fighter.clamp_red_life();
}

fn target_red_life_add(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let absolute = int_param(sim, who, control, "absolute").unwrap_or(0) != 0;
    let Some(target) = sim.fighters[who].target else {
        return;
    };
    let fighter = &mut sim.fighters[target];
    if value == 0 {
        return;
    }
    let add = if absolute {
        value
    } else {
        (value as f32 / fighter.defence_mul.max(0.01)).round() as i32
    };
    fighter.red_life = fighter.red_life.saturating_add(add);
    fighter.clamp_red_life();
}

fn dizzy_points_add(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let absolute = int_param(sim, who, control, "absolute").unwrap_or(0) != 0;
    let fighter = &mut sim.fighters[who];
    if fighter.dizzy_max <= 0 || value == 0 {
        return;
    }
    let add = if absolute {
        value as f32
    } else {
        value as f32 / fighter.defence_mul.max(0.01)
    };
    fighter.dizzy_points = (fighter.dizzy_points + add.round() as i32).clamp(0, fighter.dizzy_max);
}

fn dizzy_points_set(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let fighter = &mut sim.fighters[who];
    if fighter.dizzy_max <= 0 {
        return;
    }
    fighter.dizzy_points = value.clamp(0, fighter.dizzy_max);
}

fn dizzy_set(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0) != 0;
    sim.fighters[who].dizzy_flag = value;
}

fn modify_player(sim: &mut SimState, who: usize, control: &Controller) {
    let life_max = int_param(sim, who, control, "lifemax");
    let guard_max = int_param(sim, who, control, "guardpointsmax");
    let dizzy_max = int_param(sim, who, control, "dizzypointsmax");
    let attack = num_param(sim, who, control, "attack");
    let defence = num_param(sim, who, control, "defence");
    let ai_level = int_param(sim, who, control, "ailevel");
    let fighter = &mut sim.fighters[who];
    if let Some(value) = life_max {
        fighter.life_max = value.max(1);
        fighter.life = fighter.life.clamp(0, fighter.life_max);
        fighter.clamp_red_life();
    }
    if let Some(value) = guard_max {
        fighter.guard_max = value.max(0);
        fighter.guard_points = fighter.guard_points.clamp(0, fighter.guard_max);
    }
    if let Some(value) = dizzy_max {
        fighter.dizzy_max = value.max(0);
        fighter.dizzy_points = fighter.dizzy_points.clamp(0, fighter.dizzy_max);
    }
    if let Some(value) = attack {
        fighter.attack_mul = (value / 100.0).max(0.0);
    }
    if let Some(value) = defence {
        fighter.defence_mul = (value / 100.0).max(0.0);
    }
    if let Some(value) = ai_level {
        fighter.ai_level = value.max(0);
    }
}

fn target_power_add(sim: &mut SimState, who: usize, control: &Controller) {
    let value = num_param(sim, who, control, "value").unwrap_or(0.0);
    if let Some(target) = sim.fighters[who].target {
        sim.fighters[target].power = clamp(sim.fighters[target].power + value, 0.0, MAX_POWER);
    }
}

fn target_guard_points_add(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let absolute = int_param(sim, who, control, "absolute").unwrap_or(0) != 0;
    let Some(target) = sim.fighters[who].target else {
        return;
    };
    let fighter = &mut sim.fighters[target];
    if fighter.guard_max <= 0 || value == 0 {
        return;
    }
    let add = if absolute {
        value as f32
    } else {
        value as f32 / fighter.defence_mul.max(0.01)
    };
    fighter.guard_points = (fighter.guard_points + add.round() as i32).clamp(0, fighter.guard_max);
}

fn target_dizzy_points_add(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let absolute = int_param(sim, who, control, "absolute").unwrap_or(0) != 0;
    let Some(target) = sim.fighters[who].target else {
        return;
    };
    let fighter = &mut sim.fighters[target];
    if fighter.dizzy_max <= 0 || value == 0 {
        return;
    }
    let add = if absolute {
        value as f32
    } else {
        value as f32 / fighter.defence_mul.max(0.01)
    };
    fighter.dizzy_points = (fighter.dizzy_points + add.round() as i32).clamp(0, fighter.dizzy_max);
}

fn target_guard_points_set(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let Some(target) = sim.fighters[who].target else {
        return;
    };
    let fighter = &mut sim.fighters[target];
    if fighter.guard_max <= 0 {
        return;
    }
    fighter.guard_points = value.clamp(0, fighter.guard_max);
}

fn target_dizzy_points_set(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let Some(target) = sim.fighters[who].target else {
        return;
    };
    let fighter = &mut sim.fighters[target];
    if fighter.dizzy_max <= 0 {
        return;
    }
    fighter.dizzy_points = value.clamp(0, fighter.dizzy_max);
}

fn target_vel_set(sim: &mut SimState, who: usize, control: &Controller) {
    let x = num_param(sim, who, control, "x").unwrap_or(0.0);
    let y = num_param(sim, who, control, "y").unwrap_or(0.0);
    if let Some(target) = sim.fighters[who].target {
        let facing = sim.fighters[target].facing as f32;
        sim.fighters[target].vel = Vec2::new(x * facing, y);
    }
}

fn target_vel_add(sim: &mut SimState, who: usize, control: &Controller) {
    let x = num_param(sim, who, control, "x").unwrap_or(0.0);
    let y = num_param(sim, who, control, "y").unwrap_or(0.0);
    if let Some(target) = sim.fighters[who].target {
        let facing = sim.fighters[target].facing as f32;
        sim.fighters[target].vel.x += x * facing;
        sim.fighters[target].vel.y += y;
    }
}

fn game_make_anim(sim: &mut SimState, who: usize, control: &Controller) {
    let Some(anim) = int_param(sim, who, control, "anim") else {
        return;
    };
    let (px, py) = control.get_pair("pos").unwrap_or((0.0, 0.0));
    let pos_type = str_param(control, "postype")
        .unwrap_or("p1")
        .to_ascii_lowercase();
    let facing = sim.fighters[who].facing;
    let left_edge = sim.camera.x - manato_core::GAME_WIDTH * 0.5;
    let right_edge = sim.camera.x + manato_core::GAME_WIDTH * 0.5;
    let base = match pos_type.as_str() {
        "p2" => sim.fighters[sim.other(who)].pos,
        "front" => Vec2::new(
            if facing > 0 { right_edge } else { left_edge },
            sim.fighters[who].pos.y,
        ),
        "back" => Vec2::new(
            if facing > 0 { left_edge } else { right_edge },
            sim.fighters[who].pos.y,
        ),
        "left" => Vec2::new(left_edge, sim.fighters[who].pos.y),
        "right" => Vec2::new(right_edge, sim.fighters[who].pos.y),
        _ => sim.fighters[who].pos,
    };
    let duration = sim
        .assets
        .effects
        .animations
        .get(&anim)
        .map(|animation| animation.total_time())
        .unwrap_or(10);
    sim.effects.push(Effect {
        source: EffectSource::FightFx,
        anim,
        pos: Vec2::new(base.x + px * facing as f32, base.y + py),
        vel: Vec2::ZERO,
        accel: Vec2::ZERO,
        scale: Vec2::new(1.0, 1.0),
        facing,
        flip_h: false,
        flip_v: false,
        blend: EffectBlend::Add,
        alpha: 1.0,
        sprite_priority: int_param(sim, who, control, "sprpriority").unwrap_or(0),
        time: 0,
        duration,
        remove_on_anim_end: true,
        id: int_param(sim, who, control, "id").unwrap_or(0),
        owner: who,
        behind: false,
        ..Effect::default()
    });
}

fn modify_explod(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let id = int_param(sim, who, control, "id").unwrap_or(0);
    let pos = control.get_pair("pos");
    let scale = control.get_pair("scale");
    let vel = control.get_pair("vel");
    let accel = control.get_pair("accel");
    let trans = str_param(control, "trans").map(|value| value.to_ascii_lowercase());
    let alpha_pair = control.get("alpha").map(|raw| {
        let parts: Vec<f32> = raw
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        (
            *parts.first().unwrap_or(&256.0),
            *parts.get(1).unwrap_or(&0.0),
        )
    });
    let angle = num_param(sim, who, control, "angle");
    let x_angle = num_param(sim, who, control, "xangle");
    let y_angle = num_param(sim, who, control, "yangle");
    let x_shear = num_param(sim, who, control, "xshear");
    let projection = str_param(control, "projection").map(crate::ProjectionMode::parse);
    let focal_length = num_param(sim, who, control, "focallength");
    let remove_time = int_param(sim, who, control, "removetime");
    let sprite_priority = int_param(sim, who, control, "sprpriority");
    let ontop = int_param(sim, who, control, "ontop");
    let bind_time = int_param(sim, who, control, "bindtime");
    let remove_on_get_hit = int_param(sim, who, control, "removeongethit").map(|v| v != 0);
    let own_pal = int_param(sim, who, control, "ownpal").map(|v| v != 0);
    let shadow = int_param(sim, who, control, "shadow").map(|v| v != 0);
    let anim_elem = int_param(sim, who, control, "animelem");
    let anim_freeze = int_param(sim, who, control, "animfreeze").map(|v| v != 0);
    let fighter_positions: Vec<Vec2> = sim.fighters.iter().map(|fighter| fighter.pos).collect();

    // Modify the newest matching explod, per `docs/spec/render.md` §4.
    let Some(effect) = sim
        .effects
        .iter_mut()
        .rev()
        .find(|effect| effect.id == id && effect.owner == who)
    else {
        return;
    };
    if let Some((x, y)) = pos {
        effect.pos = Vec2::new(x, y);
        effect.bind_offset = Vec2::new(x, y);
    }
    if let Some((x, y)) = scale {
        effect.scale = Vec2::new(x, y);
    }
    if let Some((x, y)) = vel {
        effect.vel = Vec2::new(x, y);
    }
    if let Some((x, y)) = accel {
        effect.accel = Vec2::new(x, y);
    }
    if let Some(trans) = trans {
        match trans.as_str() {
            "none" => {
                effect.transparent = false;
                effect.blend = EffectBlend::Normal;
                effect.alpha = 1.0;
            }
            "add" | "add1" => {
                effect.transparent = false;
                effect.blend = EffectBlend::Add;
                effect.alpha = 1.0;
            }
            "sub" | "sub1" => {
                effect.transparent = false;
                effect.blend = EffectBlend::Sub;
                effect.alpha = 1.0;
            }
            "addalpha" => {
                effect.transparent = true;
                effect.blend = EffectBlend::Add;
                effect.alpha = alpha_pair
                    .map(|(_, dest)| (dest / 256.0).clamp(0.0, 1.0))
                    .unwrap_or(0.0);
            }
            "alpha" => {
                effect.transparent = true;
                effect.blend = EffectBlend::Normal;
                effect.alpha = alpha_pair
                    .map(|(_, dest)| (dest / 256.0).clamp(0.0, 1.0))
                    .unwrap_or(0.0);
            }
            _ => {}
        }
    } else if let Some((_, dest)) = alpha_pair {
        effect.alpha = (dest / 256.0).clamp(0.0, 1.0);
    }
    if let Some(value) = angle {
        effect.angle = value;
    }
    if let Some(value) = x_angle {
        effect.x_angle = value;
    }
    if let Some(value) = y_angle {
        effect.y_angle = value;
    }
    if let Some(value) = x_shear {
        effect.x_shear = value;
    }
    if let Some(value) = projection {
        effect.projection = value;
    }
    if let Some(value) = focal_length {
        effect.focal_length = value.max(1.0);
    }
    if let Some(value) = remove_time {
        eprintln!(
            "DBG modify removetime={value} elem={} freeze={}",
            effect.anim_elem, effect.anim_freeze
        );
        if value > 0 {
            effect.duration = value;
            effect.remove_on_anim_end = false;
        } else if !effect.anim_freeze {
            // Non-positive `removetime` restores animation-length expiry
            // (from the current `animelem`, when set).
            let animation = match effect.source {
                crate::EffectSource::FightFx => assets.effects.animations.get(&effect.anim),
                crate::EffectSource::Character(char_id) => assets
                    .characters
                    .get(char_id)
                    .and_then(|char| char.animation(effect.anim)),
                crate::EffectSource::Stage => None,
            };
            if let Some(animation) = animation {
                let start = animation
                    .frames
                    .iter()
                    .take(effect.anim_elem.max(1) as usize - 1)
                    .map(|frame| frame.effective_duration())
                    .sum::<i32>();
                effect.duration = (animation.total_time() - start).max(1);
                effect.remove_on_anim_end = true;
            }
        }
    }
    if let Some(value) = sprite_priority {
        effect.sprite_priority = value;
    }
    if let Some(value) = ontop {
        effect.behind = value == 0;
    }
    if let Some(value) = bind_time {
        effect.bind_time = value;
        if value == 0 {
            effect.bound = false;
            effect.bind_target = None;
        } else {
            let target = effect.bind_target.unwrap_or(effect.owner);
            if let Some(target_pos) = fighter_positions.get(target) {
                effect.bind_target = Some(target);
                effect.bind_offset =
                    Vec2::new(effect.pos.x - target_pos.x, effect.pos.y - target_pos.y);
                effect.bound = true;
            }
        }
    }
    if let Some(value) = remove_on_get_hit {
        effect.remove_on_get_hit = value;
    }
    if let Some(value) = own_pal {
        effect.own_pal = value;
    }
    if let Some(value) = shadow {
        effect.shadow = value;
    }
    if let Some(elem) = anim_elem {
        effect.anim_elem = elem.max(1);
        effect.time = 0;
        let animation = match effect.source {
            crate::EffectSource::FightFx => assets.effects.animations.get(&effect.anim),
            crate::EffectSource::Character(char_id) => assets
                .characters
                .get(char_id)
                .and_then(|char| char.animation(effect.anim)),
            crate::EffectSource::Stage => None,
        };
        if let Some(animation) = animation {
            let start = animation
                .frames
                .iter()
                .take(effect.anim_elem as usize - 1)
                .map(|frame| frame.effective_duration())
                .sum::<i32>();
            if effect.remove_on_anim_end {
                effect.duration = (animation.total_time() - start).max(1);
            }
        }
    }
    if let Some(freeze) = anim_freeze {
        effect.anim_freeze = freeze;
        if freeze {
            // A frozen animation must not expire with its animation.
            effect.remove_on_anim_end = false;
            effect.duration = -1;
        }
    }
}

/// `ModifyStageVar` subset: camera framing/zoom and playable bounds.
fn modify_stage_var(sim: &mut SimState, who: usize, control: &Controller) {
    let (
        bound_left,
        bound_right,
        bound_high,
        bound_low,
        vertical_follow,
        floor_tension,
        tension,
        cut_high,
        cut_low,
        start_zoom,
        zoom_out,
        zoom_in,
        zoom_in_delay,
        zoom_in_speed,
        zoom_out_speed,
        left_bound,
        right_bound,
    ) = {
        let value = |key: &str| num_param(sim, who, control, key);
        let int_value = |key: &str| int_param(sim, who, control, key);
        (
            value("camera.boundleft"),
            value("camera.boundright"),
            value("camera.boundhigh"),
            value("camera.boundlow"),
            value("camera.verticalfollow"),
            value("camera.floortension"),
            value("camera.tension"),
            value("camera.cuthigh"),
            value("camera.cutlow"),
            value("camera.startzoom"),
            value("camera.zoomout"),
            value("camera.zoomin"),
            int_value("camera.zoomindelay"),
            value("camera.zoominspeed"),
            value("camera.zoomoutspeed"),
            value("playerinfo.leftbound"),
            value("playerinfo.rightbound"),
        )
    };

    let camera = &mut sim.stage_camera;
    if let Some(v) = bound_left {
        camera.bound_left = v;
    }
    if let Some(v) = bound_right {
        camera.bound_right = v;
    }
    if let Some(v) = bound_high {
        camera.bound_high = v;
    }
    if let Some(v) = bound_low {
        camera.bound_low = v;
    }
    if let Some(v) = vertical_follow {
        camera.vertical_follow = v;
    }
    if let Some(v) = floor_tension {
        camera.floor_tension = v;
    }
    if let Some(v) = tension {
        camera.tension = v;
    }
    if let Some(v) = cut_high {
        camera.cut_high = v;
    }
    if let Some(v) = cut_low {
        camera.cut_low = v;
    }
    if let Some(v) = start_zoom {
        camera.start_zoom = v.max(0.0001);
    }
    if let Some(v) = zoom_out {
        camera.zoom_out = v.max(0.0001);
    }
    if let Some(v) = zoom_in {
        camera.zoom_in = v.max(0.0001);
    }
    if let Some(v) = zoom_in_delay {
        camera.zoom_in_delay = v.max(0);
    }
    if let Some(v) = zoom_in_speed {
        camera.zoom_in_speed = v;
    }
    if let Some(v) = zoom_out_speed {
        camera.zoom_out_speed = v;
    }
    if let Some(v) = left_bound {
        sim.stage_left_bound = v;
    }
    if let Some(v) = right_bound {
        sim.stage_right_bound = v;
    }
}

/// `TransformClsn` subset: per-tick collision-box scaling for Clsn1/Clsn2.
/// `angle`/`pivot` are not modeled (collision boxes stay axis-aligned).
fn transform_clsn(sim: &mut SimState, who: usize, control: &Controller) {
    let group = int_param(sim, who, control, "group").unwrap_or(-1);
    let index = int_param(sim, who, control, "index").unwrap_or(-1);
    let scale = control
        .get("scale")
        .map(|raw| {
            let mut parts = raw.split(',');
            let x = parts
                .next()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .unwrap_or(1.0);
            let y = parts
                .next()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .unwrap_or(x);
            Vec2::new(x, y)
        })
        .unwrap_or(Vec2::new(1.0, 1.0));
    let transforms = &mut sim.fighters[who].clsn_transforms;
    match group {
        0 => {
            transforms[0].clear();
            transforms[1].clear();
        }
        -1 => {
            for group in transforms.iter_mut() {
                group.push(crate::fighter::ClsnTransform { index, scale });
            }
        }
        1..=2 => {
            transforms[(group - 1) as usize].push(crate::fighter::ClsnTransform { index, scale });
        }
        _ => {}
    }
}

/// `TransformSprite` subset: x-shear, focal length, and projection.
/// `window` is parsed but not modeled (the renderer has no per-fighter scissor).
fn transform_sprite(sim: &mut SimState, who: usize, control: &Controller) {
    let shear = num_param(sim, who, control, "xshear");
    let focal = num_param(sim, who, control, "focallength");
    let projection = str_param(control, "projection").map(crate::ProjectionMode::parse);
    let fighter = &mut sim.fighters[who];
    if let Some(value) = shear {
        fighter.transform_x_shear = value;
    }
    if let Some(value) = focal {
        fighter.transform_focal_length = value.max(1.0);
    }
    if let Some(value) = projection {
        fighter.transform_projection = value;
    }
}

fn explod_bind_time(sim: &mut SimState, who: usize, control: &Controller) {
    let id = int_param(sim, who, control, "id").unwrap_or(-1);
    let time = int_param(sim, who, control, "time")
        .or_else(|| int_param(sim, who, control, "value"))
        .unwrap_or(0);
    for effect in &mut sim.effects {
        if effect.owner != who || (id >= 0 && effect.id != id) {
            continue;
        }
        effect.bind_time = time;
        effect.bound = time != 0 && effect.bind_target.is_some();
    }
}

fn remove_explod(sim: &mut SimState, who: usize, control: &Controller) {
    let id = int_param(sim, who, control, "id").unwrap_or(0);
    sim.effects
        .retain(|effect| effect.id != id || effect.owner != who);
}

fn projectile(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let char = char_of(assets, sim, who);
    let anim = int_param(sim, who, control, "projanim").unwrap_or(0);
    if anim <= 0 || char.animation(anim).is_none() {
        log::warn!("projectile uses missing animation {anim}");
        return;
    }
    let (px, py) = control.get_pair("pos").unwrap_or((0.0, 0.0));
    let (vx, vy) = control.get_pair("velocity").unwrap_or((0.0, 0.0));
    let (ax, ay) = control.get_pair("accel").unwrap_or((0.0, 0.0));
    let facing = sim.fighters[who].facing;
    let mut hitdef = HitDef::from_controller_for(
        control,
        sim.hit_activation,
        char.localcoord(),
        character_guard_dist(char, true),
    );
    // Each projectile spawn is its own hit activation.
    sim.hit_activation = sim.hit_activation.wrapping_add(1);
    // Projectile sparks sit at the impact point, not at a body offset.
    hitdef.sparkxy = Vec2::ZERO;
    // Both the plain and `proj*`-prefixed aliases are accepted.
    let remove_time = int_param(sim, who, control, "projremovetime")
        .or_else(|| int_param(sim, who, control, "removetime"))
        .unwrap_or(-1);
    let hits = int_param(sim, who, control, "projhits")
        .or_else(|| int_param(sim, who, control, "hits"))
        .unwrap_or(1)
        .max(1);
    let id = int_param(sim, who, control, "projid")
        .or_else(|| int_param(sim, who, control, "id"))
        .unwrap_or(0);
    let priority = int_param(sim, who, control, "projpriority").unwrap_or(0);
    let remove_on_hits_exhausted = int_param(sim, who, control, "projremove").unwrap_or(1) != 0;
    let shadow = parse_shadow(sim, who, control, "projshadow").unwrap_or([0; 3]);
    let scale = control.get_pair("projscale").unwrap_or((1.0, 1.0));
    let angle = num_param(sim, who, control, "projangle").unwrap_or(0.0);
    let x_shear = num_param(sim, who, control, "projxshear").unwrap_or(0.0);
    let projection = str_param(control, "projprojection")
        .map(crate::ProjectionMode::parse)
        .unwrap_or(crate::ProjectionMode::Orthographic);
    let focal_length = num_param(sim, who, control, "projfocallength")
        .unwrap_or(1024.0)
        .max(1.0);
    let reflection = int_param(sim, who, control, "projreflection").unwrap_or(0);
    let miss_time = int_param(sim, who, control, "projmisstime").unwrap_or(0);
    let hit_anim = int_param(sim, who, control, "projhitanim").unwrap_or(-1);
    let rem_anim = int_param(sim, who, control, "projremanim").unwrap_or(-1);
    let cancel_anim = int_param(sim, who, control, "projcancelanim").unwrap_or(-1);
    let layer_no = int_param(sim, who, control, "projlayerno").unwrap_or(0);
    let sprite_priority = int_param(sim, who, control, "projsprpriority").unwrap_or(3);
    let edge_bound = num_param(sim, who, control, "projedgebound").unwrap_or(40.0);
    let stage_bound = num_param(sim, who, control, "projstagebound").unwrap_or(40.0);
    let height_bound = control.get_pair("projheightbound").unwrap_or((-240.0, 1.0));
    let depth_bound = num_param(sim, who, control, "projdepthbound").unwrap_or(f32::MAX);
    let rem_velocity = control
        .get_pair("remvelocity")
        .or_else(|| control.get_pair("projremvelocity"))
        .unwrap_or((0.0, 0.0));
    let postype = str_param(control, "postype")
        .unwrap_or("p1")
        .to_ascii_lowercase();
    let owner = sim.fighters[who].pos;
    let left_edge = sim.camera.x - manato_core::GAME_WIDTH * 0.5;
    let right_edge = sim.camera.x + manato_core::GAME_WIDTH * 0.5;
    let base = match postype.as_str() {
        "p2" => sim.fighters[sim.other(who)].pos,
        "front" => Vec2::new(if facing > 0 { right_edge } else { left_edge }, owner.y),
        "back" => Vec2::new(if facing > 0 { left_edge } else { right_edge }, owner.y),
        "left" => Vec2::new(left_edge, owner.y),
        "right" => Vec2::new(right_edge, owner.y),
        _ => owner,
    };
    let pos = Vec2::new(base.x + px * facing as f32, base.y + py);
    // `removetime = 0` expires before the projectile can move or hit.
    if remove_time == 0 {
        return;
    }
    let anim_loop_time = char.animation(anim).map(|a| a.total_time()).unwrap_or(0);
    sim.projectiles.push(crate::projectile::Projectile {
        owner: who,
        id,
        anim,
        pos,
        vel: Vec2::new(vx * facing as f32, vy),
        accel: Vec2::new(ax * facing as f32, ay),
        facing,
        scale: Vec2::new(scale.0, scale.1),
        angle,
        x_shear,
        projection,
        focal_length,
        time: 0,
        remove_time,
        anim_loop_time,
        remove_on_hits_exhausted,
        priority,
        priority_points: priority,
        shadow,
        reflection,
        miss_time,
        hit_anim,
        rem_anim,
        cancel_anim,
        layer_no,
        sprite_priority,
        edge_bound,
        stage_bound,
        height_bound: (height_bound.0, height_bound.1),
        depth_bound,
        out_of_bounds: false,
        status: crate::projectile::ProjectileStatus::Active,
        removal_ticks: 0,
        rem_velocity: Vec2::new(rem_velocity.0, rem_velocity.1),
        hits,
        hitdef,
    });
    if let Some(raw) = control.get("projsnd") {
        let mut parts = raw.split(',');
        let group = parts
            .next()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(-1);
        let index = parts
            .next()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        if group >= 0 {
            sim.sounds.push(SoundEvent {
                source: SoundSource::Character(who),
                group,
                index,
                volume: 0.0,
                pan: 0.0,
                channel: -1,
                loop_sound: false,
                priority: 0,
                low_priority: false,
            });
        }
    }
}

/// `ModifyProjectile`: updates a live projectile owned by the fighter.
///
/// With an explicit `id`, every matching projectile is updated; otherwise the
/// most recently spawned projectile is. Only supplied parameters change.
fn parse_shadow(sim: &SimState, who: usize, control: &Controller, key: &str) -> Option<[i32; 3]> {
    let raw = control.get(key)?;
    let mut parts = raw.split(',').map(str::trim);
    let mut shadow = [0_i32; 3];
    let mut any = false;
    for slot in &mut shadow {
        let Some(part) = parts.next() else {
            break;
        };
        if part.is_empty() {
            continue;
        }
        *slot = trigger::int(sim, who, part);
        any = true;
    }
    any.then_some(shadow)
}

fn modify_projectile(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let character = char_of(assets, sim, who);
    let explicit_id =
        int_param(sim, who, control, "id").or_else(|| int_param(sim, who, control, "projid"));
    let last = sim
        .projectiles
        .iter()
        .rposition(|projectile| projectile.owner == who);
    let Some(last) = last else {
        return;
    };
    let facing = sim.fighters[who].facing;
    let indices: Vec<usize> = sim
        .projectiles
        .iter()
        .enumerate()
        .filter(|(index, projectile)| {
            projectile.owner == who
                && match explicit_id {
                    Some(id) => projectile.id == id,
                    None => *index == last,
                }
        })
        .map(|(index, _)| index)
        .collect();
    let anim = int_param(sim, who, control, "projanim");
    let velocity = control.get_pair("velocity");
    let accel = control.get_pair("accel");
    let remove_time = int_param(sim, who, control, "projremovetime")
        .or_else(|| int_param(sim, who, control, "removetime"));
    let hits =
        int_param(sim, who, control, "projhits").or_else(|| int_param(sim, who, control, "hits"));
    let priority = int_param(sim, who, control, "projpriority");
    let remove = int_param(sim, who, control, "projremove").map(|value| value != 0);
    let shadow = parse_shadow(sim, who, control, "projshadow");
    let scale = control.get_pair("projscale");
    let angle = num_param(sim, who, control, "projangle");
    let x_shear = num_param(sim, who, control, "projxshear");
    let projection = str_param(control, "projprojection").map(crate::ProjectionMode::parse);
    let focal_length = num_param(sim, who, control, "projfocallength").map(|v| v.max(1.0));
    let reflection = int_param(sim, who, control, "projreflection");
    let miss_time = int_param(sim, who, control, "projmisstime");
    let hit_anim = int_param(sim, who, control, "projhitanim");
    let rem_anim = int_param(sim, who, control, "projremanim");
    let cancel_anim = int_param(sim, who, control, "projcancelanim");
    let layer_no = int_param(sim, who, control, "projlayerno");
    let sprite_priority = int_param(sim, who, control, "projsprpriority");
    let edge_bound = num_param(sim, who, control, "projedgebound");
    let stage_bound = num_param(sim, who, control, "projstagebound");
    let height_bound = control.get_pair("projheightbound");
    let depth_bound = num_param(sim, who, control, "projdepthbound");
    let rem_velocity = control
        .get_pair("remvelocity")
        .or_else(|| control.get_pair("projremvelocity"));
    let damage = control.get("damage").and_then(|raw| {
        raw.split(',')
            .next()
            .and_then(|value| value.trim().parse::<i32>().ok())
    });
    for index in indices {
        let projectile = &mut sim.projectiles[index];
        if let Some(anim) = anim {
            projectile.anim = anim;
            projectile.anim_loop_time = character
                .animation(anim)
                .map(|animation| animation.total_time())
                .unwrap_or(0);
        }
        if let Some((x, y)) = velocity {
            projectile.vel = Vec2::new(x * facing as f32, y);
            projectile.facing = facing;
        }
        if let Some((x, y)) = accel {
            projectile.accel = Vec2::new(x * facing as f32, y);
        }
        if let Some(remove_time) = remove_time {
            projectile.remove_time = remove_time;
        }
        if let Some(hits) = hits {
            projectile.hits = hits.max(0);
        }
        if let Some(priority) = priority {
            projectile.priority = priority;
            projectile.priority_points = priority;
        }
        if let Some(remove) = remove {
            projectile.remove_on_hits_exhausted = remove;
        }
        if let Some(shadow) = shadow {
            projectile.shadow = shadow;
        }
        if let Some((x, y)) = scale {
            projectile.scale = Vec2::new(x, y);
        }
        if let Some(angle) = angle {
            projectile.angle = angle;
        }
        if let Some(x_shear) = x_shear {
            projectile.x_shear = x_shear;
        }
        if let Some(projection) = projection {
            projectile.projection = projection;
        }
        if let Some(focal_length) = focal_length {
            projectile.focal_length = focal_length;
        }
        if let Some(reflection) = reflection {
            projectile.reflection = reflection;
        }
        if let Some(miss_time) = miss_time {
            projectile.miss_time = miss_time;
        }
        if let Some(hit_anim) = hit_anim {
            projectile.hit_anim = hit_anim;
        }
        if let Some(rem_anim) = rem_anim {
            projectile.rem_anim = rem_anim;
        }
        if let Some(cancel_anim) = cancel_anim {
            projectile.cancel_anim = cancel_anim;
        }
        if let Some(layer_no) = layer_no {
            projectile.layer_no = layer_no;
        }
        if let Some(sprite_priority) = sprite_priority {
            projectile.sprite_priority = sprite_priority;
        }
        if let Some(edge_bound) = edge_bound {
            projectile.edge_bound = edge_bound;
        }
        if let Some(stage_bound) = stage_bound {
            projectile.stage_bound = stage_bound;
        }
        if let Some((low, high)) = height_bound {
            projectile.height_bound = (low, high);
        }
        if let Some(depth_bound) = depth_bound {
            projectile.depth_bound = depth_bound;
        }
        if let Some((x, y)) = rem_velocity {
            projectile.rem_velocity = Vec2::new(x, y);
        }
        if let Some(damage) = damage {
            projectile.hitdef.damage = damage;
        }
    }
}

fn play_snd(sim: &mut SimState, who: usize, control: &Controller) {
    let Some(raw) = param(control, "value") else {
        return;
    };
    let mut parts = raw.split(',');
    let group = parts
        .next()
        .and_then(|v| trigger::int(sim, who, v).into())
        .unwrap_or(-1);
    let index = parts
        .next()
        .and_then(|v| trigger::int(sim, who, v).into())
        .unwrap_or(0);
    let volume = num_param(sim, who, control, "volume").unwrap_or(0.0);
    let pan = (num_param(sim, who, control, "pan").unwrap_or(0.0) / 100.0).clamp(-1.0, 1.0);
    let channel = int_param(sim, who, control, "channel").unwrap_or(-1);
    let loop_sound = int_param(sim, who, control, "loop").unwrap_or(0) != 0;
    let priority = int_param(sim, who, control, "priority").unwrap_or(0);
    let low_priority = int_param(sim, who, control, "lowpriority").unwrap_or(0) != 0;
    sim.sounds.push(SoundEvent {
        source: SoundSource::Character(who),
        group,
        index,
        volume,
        pan,
        channel,
        loop_sound,
        priority,
        low_priority,
    });
}

fn stop_snd(sim: &mut SimState, who: usize, control: &Controller) {
    let channel = int_param(sim, who, control, "channel").unwrap_or(-1);
    sim.sound_commands.push(crate::SoundCommand::Stop {
        source: SoundSource::Character(who),
        channel,
    });
}

fn snd_pan(sim: &mut SimState, who: usize, control: &Controller) {
    let channel = int_param(sim, who, control, "channel").unwrap_or(-1);
    let pan = num_param(sim, who, control, "pan").map(|value| (value / 100.0).clamp(-1.0, 1.0));
    let volume = num_param(sim, who, control, "volume").map(|value| value.clamp(0.0, 1.0));
    sim.sound_commands.push(crate::SoundCommand::Pan {
        source: SoundSource::Character(who),
        channel,
        pan,
        volume,
    });
}

fn modify_snd(sim: &mut SimState, who: usize, control: &Controller) {
    let Some(raw) = param(control, "value") else {
        return;
    };
    let mut parts = raw.split(',');
    let group = parts
        .next()
        .and_then(|v| trigger::int(sim, who, v).into())
        .unwrap_or(-1);
    let index = parts
        .next()
        .and_then(|v| trigger::int(sim, who, v).into())
        .unwrap_or(0);
    let channel = int_param(sim, who, control, "channel").unwrap_or(-1);
    let pan = num_param(sim, who, control, "pan").map(|value| (value / 100.0).clamp(-1.0, 1.0));
    let volume = num_param(sim, who, control, "volume").map(|value| value.clamp(0.0, 1.0));
    let loop_sound = int_param(sim, who, control, "loop").map(|value| value != 0);
    sim.sound_commands.push(crate::SoundCommand::Modify {
        source: SoundSource::Character(who),
        channel,
        group,
        index,
        volume,
        pan,
        loop_sound,
    });
}

fn hit_def(sim: &mut SimState, who: usize, control: &Controller) {
    let same_activation = sim.fighters[who]
        .hit
        .last_activation_frame
        .saturating_add(1)
        == sim.frame as i64
        || sim.fighters[who].hit.pause_gap > 0;
    if !same_activation {
        sim.hit_activation = sim.hit_activation.wrapping_add(1);
        sim.fighters[who].hit.hit_targets.clear();
        sim.fighters[who].hit.hit_counts.clear();
    }
    sim.fighters[who].hit.last_activation_frame = sim.frame as i64;
    let character = &sim.assets.characters[sim.fighters[who].char_id];
    let hitdef = HitDef::from_controller_for(
        control,
        sim.hit_activation,
        character.localcoord(),
        character_guard_dist(character, false),
    );
    if hitdef.air_juggle > 0 {
        sim.fighters[who].juggle = hitdef.air_juggle;
    }
    sim.fighters[who].hitdef = Some(hitdef);
}

fn hit_vel_set(sim: &mut SimState, who: usize, control: &Controller) {
    let x = int_param(sim, who, control, "x").unwrap_or(0) != 0;
    let y = int_param(sim, who, control, "y").unwrap_or(0) != 0;
    let vars = sim.fighters[who].hit.vars.clone();
    if x {
        sim.fighters[who].vel.x = vars.x_vel;
    }
    if y {
        sim.fighters[who].vel.y = vars.y_vel;
    }
}

fn hit_fall_set(sim: &mut SimState, who: usize, control: &Controller) {
    // Negative values leave the flag untouched, like Ikemen's `hitFallSet`.
    if let Some(value) = int_param(sim, who, control, "value").filter(|value| *value >= 0) {
        sim.fighters[who].hit.vars.fall = value != 0;
    }
    if let Some(x) = num_param(sim, who, control, "xvel") {
        sim.fighters[who].hit.vars.fall_x_vel = Some(x * sim.fighters[who].facing as f32);
    }
    if let Some(y) = num_param(sim, who, control, "yvel") {
        sim.fighters[who].hit.vars.fall_y_vel = Some(y);
    }
}

fn hit_fall_vel(sim: &mut SimState, who: usize, _control: &Controller) {
    let vars = sim.fighters[who].hit.vars.clone();
    if let Some(x) = vars.fall_x_vel {
        sim.fighters[who].vel.x = x;
    }
    if let Some(y) = vars.fall_y_vel {
        sim.fighters[who].vel.y = y;
    }
}

fn hit_filter(sim: &mut SimState, who: usize, control: &Controller, not_hit_by: bool) {
    let attr = str_param(control, "value")
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_default();
    let time = int_param(sim, who, control, "time").unwrap_or(1);
    if not_hit_by {
        sim.fighters[who].hit.not_hit_by = Some((attr, time));
    } else {
        sim.fighters[who].hit.hit_by = Some((attr, time));
    }
}

fn assert_special(sim: &mut SimState, who: usize, control: &Controller) {
    let mut apply = |name: &str| {
        let flag = crate::fighter::assert_flag(name);
        if flag != 0 {
            sim.fighters[who].assert_flags |= flag;
        }
    };
    // M.U.G.E.N writes assertion flags as keys (`invisible = 1`); some
    // content uses the older `flag = invisible` form.
    for (key, value) in &control.params {
        let lower = key.to_ascii_lowercase();
        if lower == "flag" {
            apply(&value.to_ascii_lowercase());
        } else if value.trim() != "0" {
            apply(&lower);
        }
    }
}

fn modify_reflection(sim: &mut SimState, who: usize, control: &Controller) {
    let intensity = int_param(sim, who, control, "intensity");
    let color = control
        .get("color")
        .and_then(|value| parse_triplet(sim, who, value))
        .map(|c| [c[0] as u8, c[1] as u8, c[2] as u8]);
    let offset = control.get_pair("offset");
    let xscale = num_param(sim, who, control, "xscale");
    let yscale = num_param(sim, who, control, "yscale");
    let fighter = &mut sim.fighters[who];
    if let Some(value) = intensity {
        fighter.reflection_intensity = Some(value);
    }
    if color.is_some() {
        fighter.reflection_color = color;
    }
    if let Some((x, y)) = offset {
        fighter.reflection_offset = Vec2::new(x, y);
    }
    if let Some(value) = xscale {
        fighter.reflection_xscale = Some(value);
    }
    if let Some(value) = yscale {
        fighter.reflection_yscale = Some(value);
    }
}

fn modify_shadow(sim: &mut SimState, who: usize, control: &Controller) {
    let intensity = int_param(sim, who, control, "intensity");
    let color = control
        .get("color")
        .and_then(|value| parse_triplet(sim, who, value))
        .map(|c| [c[0] as u8, c[1] as u8, c[2] as u8]);
    let offset = control.get_pair("offset");
    let xscale = num_param(sim, who, control, "xscale");
    let yscale = num_param(sim, who, control, "yscale");
    let fighter = &mut sim.fighters[who];
    if let Some(value) = intensity {
        fighter.shadow_intensity = Some(value);
    }
    if color.is_some() {
        fighter.shadow_color = color;
    }
    if let Some((x, y)) = offset {
        fighter.shadow_offset = Vec2::new(x, y);
    }
    if let Some(value) = xscale {
        fighter.shadow_xscale = Some(value);
    }
    if let Some(value) = yscale {
        fighter.shadow_yscale = Some(value);
    }
}

fn camera(sim: &mut SimState, who: usize, control: &Controller) {
    if let Some(index) = int_param(sim, who, control, "followid") {
        if index >= 0 && (index as usize) < sim.fighters.len() {
            sim.camera.follow = Some(index as usize);
        }
    }
    if let Some(view) = str_param(control, "view") {
        match view.to_ascii_lowercase().as_str() {
            "follow" => {
                sim.camera.view = crate::CameraView::Follow;
                if sim.camera.follow.is_none() {
                    sim.camera.follow = Some(who);
                }
            }
            "free" => sim.camera.view = crate::CameraView::Free,
            "fighting" => {
                sim.camera.view = crate::CameraView::Fighting;
                sim.camera.follow = None;
            }
            _ => {}
        }
    }
    if let Some(pos) = control.get_pair("pos") {
        sim.camera.free_pos = Vec2::new(pos.0, pos.1);
    }
}

fn height(sim: &mut SimState, who: usize, control: &Controller) {
    // `Height` takes a two-component value: top extension, then bottom.
    let Some((top, bottom)) = control.get_pair("value") else {
        return;
    };
    sim.fighters[who].height_player = Vec2::new(top, bottom);
}

fn width(sim: &mut SimState, who: usize, control: &Controller) {
    // `edge` wins, then `player`, then the older single `value` form.
    let Some((back, front)) = control
        .get_pair("edge")
        .or_else(|| control.get_pair("player"))
        .or_else(|| control.get_pair("value"))
    else {
        return;
    };
    sim.fighters[who].width_override = Some((back, front));
    sim.fighters[who].width_back = back;
    sim.fighters[who].width_front = front;
}

/// `FallEnvShake` consumes the `fall.envshake.*` values stored by the last
/// unguarded hit and clears the time so it cannot retrigger.
fn fall_env_shake(sim: &mut SimState, who: usize) {
    let f = &sim.fighters[who];
    let time = f.hit.vars.fall_envshake_time;
    if time <= 0 {
        return;
    }
    let freq = f.hit.vars.fall_envshake_freq;
    let ampl = f.hit.vars.fall_envshake_ampl as f32;
    let mut phase = f.hit.vars.fall_envshake_phase;
    if phase.is_nan() {
        phase = if freq >= 90.0 { 90.0 } else { 0.0 };
    }
    phase = phase.clamp(-180.0, 180.0);
    sim.fighters[who].hit.vars.fall_envshake_time = 0;
    sim.env_shake = crate::EnvShake {
        time,
        initial_time: time,
        freq,
        ampl,
        phase,
        mul: 1.0,
        dir: 0.0,
        diradd: 0.0,
        decay: 0.0,
    };
}

fn env_shake(sim: &mut SimState, who: usize, control: &Controller) {
    let time = int_param(sim, who, control, "time").unwrap_or(0);
    let freq = num_param(sim, who, control, "freq")
        .unwrap_or(60.0)
        .max(0.0);
    let ampl = num_param(sim, who, control, "ampl").unwrap_or(-4.0);
    // Ikemen clamps the phase to [-180, 180] and defaults it to 90 when the
    // frequency is at least 90 degrees per tick.
    let phase = num_param(sim, who, control, "phase")
        .map(|phase| phase.clamp(-180.0, 180.0))
        .unwrap_or(if freq >= 90.0 { 90.0 } else { 0.0 });
    let mul = num_param(sim, who, control, "mul").unwrap_or(1.0);
    let dir = num_param(sim, who, control, "dir").unwrap_or(0.0);
    let diradd = num_param(sim, who, control, "diradd").unwrap_or(0.0);
    let decay = num_param(sim, who, control, "decay").unwrap_or(0.0);
    let time = time.max(0);
    sim.env_shake = EnvShake {
        time,
        initial_time: time,
        freq,
        ampl,
        phase,
        mul,
        dir,
        diradd,
        decay,
    };
}

/// `LifeBarAction`: queue a fight-screen banner for the executing team
/// (`docs/spec/render.md` §6.10`).
fn life_bar_action(sim: &mut SimState, who: usize, control: &Controller) {
    let Some(text) = str_param(control, "text") else {
        return;
    };
    let side = sim.fighters[who].side;
    let default_time = sim
        .assets
        .screenpack
        .as_ref()
        .map(|screenpack| screenpack.fight.action[side].displaytime)
        .unwrap_or(60);
    let time = int_param(sim, who, control, "time")
        .or_else(|| int_param(sim, who, control, "displaytime"))
        .unwrap_or(default_time);
    let text = text.trim_matches('"').to_string();
    sim.push_action(side, text, time);
}

fn zoom(sim: &mut SimState, who: usize, control: &Controller) {
    let (px, py) = control
        .get_pair("pos")
        .unwrap_or((sim.camera.x, sim.camera.y));
    let target_scale = num_param(sim, who, control, "scale")
        .unwrap_or(sim.camera.scale)
        .max(0.0001);
    let lag = int_param(sim, who, control, "lag").unwrap_or(0).max(0);
    let end_lag = int_param(sim, who, control, "endlag").unwrap_or(0).max(0);
    let time = int_param(sim, who, control, "time").unwrap_or(0).max(0);
    let mut target_pos = Vec2::new(px, py);
    if int_param(sim, who, control, "camerabound").unwrap_or(0) != 0 {
        let cam = &sim.stage_camera;
        let half_w = manato_core::GAME_WIDTH * 0.5 / target_scale;
        let lo = cam.bound_left + half_w;
        let hi = cam.bound_right - half_w;
        if lo <= hi {
            target_pos.x = target_pos.x.clamp(lo, hi);
        }
        target_pos.y = target_pos.y.clamp(cam.bound_high, cam.bound_low);
    }
    sim.camera.zoom_override = Some(crate::ZoomOverride {
        target_pos,
        target_scale,
        lag,
        end_lag,
        time,
        elapsed: 0,
        hold: 0,
        camera_bound: int_param(sim, who, control, "camerabound").unwrap_or(0) != 0,
        stage_bound: int_param(sim, who, control, "stagebound").unwrap_or(0) != 0,
    });
}

fn explod(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let raw_anim = str_param(control, "anim").unwrap_or("");
    let fight_fx = raw_anim
        .trim_start()
        .chars()
        .next()
        .map(|c| c.eq_ignore_ascii_case(&'f'))
        .unwrap_or(false);
    let anim_text = if fight_fx {
        &raw_anim.trim_start()[1..]
    } else {
        raw_anim
    };
    let anim = match anim_text.trim().parse::<i32>() {
        Ok(value) => value,
        Err(_) => int_param(sim, who, control, "anim").unwrap_or(0),
    };
    let id = int_param(sim, who, control, "id").unwrap_or(0);
    let (px, py) = control.get_pair("pos").unwrap_or((0.0, 0.0));
    let (vx, vy) = control.get_pair("vel").unwrap_or((0.0, 0.0));
    let (ax, ay) = control.get_pair("accel").unwrap_or((0.0, 0.0));
    let (sx, sy) = control.get_pair("scale").unwrap_or((1.0, 1.0));

    // `facing` accepts -1/1 or `p1`/`p2`.
    let owner_facing = sim.fighters[who].facing;
    let facing = match str_param(control, "facing").map(|s| s.to_ascii_lowercase()) {
        Some(value) if value == "p1" => sim.fighters[0].facing,
        Some(value) if value == "p2" => sim.fighters[1].facing,
        Some(_) => int_param(sim, who, control, "facing").unwrap_or(owner_facing),
        None => owner_facing,
    };
    let facing_f = facing as f32;

    let pos_type = str_param(control, "postype")
        .unwrap_or("p1")
        .to_ascii_lowercase();
    let left_edge = sim.camera.x - manato_core::GAME_WIDTH * 0.5;
    let right_edge = sim.camera.x + manato_core::GAME_WIDTH * 0.5;
    let (base, bind_target) = match pos_type.as_str() {
        "p2" => (sim.fighters[sim.other(who)].pos, Some(sim.other(who))),
        "front" => (
            Vec2::new(
                if facing_f > 0.0 {
                    right_edge
                } else {
                    left_edge
                },
                sim.fighters[who].pos.y,
            ),
            None,
        ),
        "back" => (
            Vec2::new(
                if facing_f > 0.0 {
                    left_edge
                } else {
                    right_edge
                },
                sim.fighters[who].pos.y,
            ),
            None,
        ),
        "left" => (Vec2::new(left_edge, sim.fighters[who].pos.y), None),
        "right" => (Vec2::new(right_edge, sim.fighters[who].pos.y), None),
        _ => (sim.fighters[who].pos, Some(who)),
    };

    let source = if fight_fx {
        EffectSource::FightFx
    } else {
        EffectSource::Character(sim.fighters[who].char_id)
    };
    let animation = if fight_fx {
        assets.effects.animations.get(&anim)
    } else {
        assets.characters[sim.fighters[who].char_id].animation(anim)
    };
    let anim_time = animation.map(|a| a.total_time()).unwrap_or(10);
    let anim_elem = int_param(sim, who, control, "animelem").unwrap_or(1).max(1) as usize;
    let anim_freeze = int_param(sim, who, control, "animfreeze").unwrap_or(0) != 0;
    // `animelem` starts the animation part-way through.
    let start_time = animation
        .map(|a| {
            a.frames
                .iter()
                .take(anim_elem - 1)
                .map(|frame| frame.effective_duration())
                .sum::<i32>()
        })
        .unwrap_or(0);

    let remove_time = int_param(sim, who, control, "removetime").unwrap_or(-1);
    let (duration, remove_on_anim_end) = if remove_time > 0 {
        (remove_time, false)
    } else if anim_freeze {
        // A frozen animation never reaches its end on its own.
        (-1, false)
    } else {
        ((anim_time - start_time).max(1), true)
    };

    let (blend, alpha, transparent) = parse_trans(sim, who, control);

    let window = control.get("window").and_then(|raw| {
        let parts: Vec<f32> = raw
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        let rect = [
            *parts.first().unwrap_or(&0.0),
            *parts.get(1).unwrap_or(&0.0),
            *parts.get(2).unwrap_or(&0.0),
            *parts.get(3).unwrap_or(&0.0),
        ];
        if rect.iter().all(|value| *value == 0.0) {
            None
        } else {
            Some(rect)
        }
    });

    let bind_time = int_param(sim, who, control, "bindtime").unwrap_or(0);
    let bind_offset = Vec2::new(px * facing_f, py);
    sim.effects.push(Effect {
        source,
        anim,
        pos: Vec2::new(base.x + bind_offset.x, base.y + bind_offset.y),
        vel: Vec2::new(vx * facing_f, vy),
        accel: Vec2::new(ax * facing_f, ay),
        scale: Vec2::new(sx, sy),
        facing,
        flip_v: int_param(sim, who, control, "vfacing").unwrap_or(1) < 0,
        blend,
        alpha,
        sprite_priority: int_param(sim, who, control, "sprpriority").unwrap_or(0),
        duration,
        remove_on_anim_end,
        id,
        owner: who,
        behind: int_param(sim, who, control, "ontop").unwrap_or(1) == 0,
        bind_time,
        // Keep the anchor even while unbound so `ExplodBindTime` can re-bind
        // the effect later.
        bind_target,
        bind_offset,
        bound: bind_time != 0 && bind_target.is_some(),
        remove_on_get_hit: int_param(sim, who, control, "removeongethit").unwrap_or(0) != 0,
        remove_on_change_state: int_param(sim, who, control, "removeonchangestate").unwrap_or(1)
            != 0,
        ignore_hit_pause: int_param(sim, who, control, "ignorehitpause").unwrap_or(1) != 0,
        super_move_time: int_param(sim, who, control, "supermovetime")
            .unwrap_or(0)
            .max(0),
        pause_move_time: int_param(sim, who, control, "pausemovetime")
            .unwrap_or(0)
            .max(0),
        shadow: int_param(sim, who, control, "shadow").unwrap_or(0) != 0,
        own_pal: int_param(sim, who, control, "ownpal").unwrap_or(0) != 0,
        transparent,
        angle: num_param(sim, who, control, "angle").unwrap_or(0.0),
        x_angle: num_param(sim, who, control, "xangle").unwrap_or(0.0),
        y_angle: num_param(sim, who, control, "yangle").unwrap_or(0.0),
        x_shear: num_param(sim, who, control, "xshear").unwrap_or(0.0),
        projection: str_param(control, "projection")
            .map(crate::ProjectionMode::parse)
            .unwrap_or(crate::ProjectionMode::Orthographic),
        focal_length: num_param(sim, who, control, "focallength")
            .unwrap_or(2048.0)
            .max(1.0),
        window,
        screen_space: str_param(control, "space")
            .map(|value| value.eq_ignore_ascii_case("screen"))
            .unwrap_or(false),
        anim_elem: anim_elem as i32,
        anim_freeze,
        ..Effect::default()
    });
}

/// Resolves `trans` and `alpha` into a blend mode, opacity, and transparency
/// flag. `docs/spec/render.md` §4.
fn parse_trans(sim: &SimState, who: usize, control: &Controller) -> (EffectBlend, f32, bool) {
    let trans = str_param(control, "trans")
        .unwrap_or("default")
        .to_ascii_lowercase();
    let pair = control.get("alpha").map(|raw| {
        let parts: Vec<f32> = raw
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        (
            *parts.first().unwrap_or(&256.0),
            *parts.get(1).unwrap_or(&0.0),
        )
    });
    match trans.as_str() {
        "none" => (EffectBlend::Normal, 1.0, false),
        "add" | "add1" => (EffectBlend::Add, 1.0, false),
        "sub" | "sub1" => (EffectBlend::Sub, 1.0, false),
        "addalpha" => {
            let (_, dest) = pair.unwrap_or((256.0, 0.0));
            (EffectBlend::Add, (dest / 256.0).clamp(0.0, 1.0), true)
        }
        "alpha" => {
            let (_, dest) = pair.unwrap_or((256.0, 0.0));
            (EffectBlend::Normal, (dest / 256.0).clamp(0.0, 1.0), true)
        }
        _ => (EffectBlend::Normal, 1.0, false),
    }
}
fn make_dust(sim: &mut SimState, who: usize, control: &Controller) {
    let spacing = int_param(sim, who, control, "spacing").unwrap_or(3).max(1);
    // `spacing` is a tick gate, not a positional offset.
    if sim.fighters[who].make_dust_spacing < spacing {
        return;
    }
    let facing = sim.fighters[who].facing as f32;
    let base = sim.fighters[who].pos;
    let mut positions: Vec<(f32, f32)> = vec![control.get_pair("pos").unwrap_or((0.0, 0.0))];
    if let Some(pos2) = control.get_pair("pos2") {
        positions.push(pos2);
    }
    let duration = sim
        .assets
        .effects
        .animations
        .get(&120)
        .map(|a| a.total_time())
        .unwrap_or(10);
    for (px, py) in positions {
        let pos = Vec2::new(base.x + px * facing, base.y + py);
        sim.effects.push(Effect {
            source: EffectSource::FightFx,
            anim: 120,
            pos,
            facing: sim.fighters[who].facing,
            sprite_priority: i32::MAX,
            duration,
            owner: who,
            behind: true,
            own_pal: true,
            ..Effect::default()
        });
    }
    sim.fighters[who].make_dust_spacing = 0;
}

fn pause(sim: &mut SimState, who: usize, control: &Controller) {
    let time = int_param(sim, who, control, "time").unwrap_or(0).max(0);
    let movetime = int_param(sim, who, control, "movetime").unwrap_or(0);
    if time > 0 {
        sim.super_pause = time;
        sim.super_pause_move_time = 0;
        // The pausing fighter may act for up to `movetime` ticks.
        sim.pause_mover = Some(who);
        sim.pause_movetime = if movetime > time { 0 } else { movetime.max(0) };
    }
}

fn super_pause(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let time = int_param(sim, who, control, "time").unwrap_or(0).max(0);
    let movetime = int_param(sim, who, control, "movetime").unwrap_or(0);
    sim.super_pause = time;
    sim.super_pause_move_time = movetime;
    // Like `Pause`, the superpausing fighter may act for `movetime` ticks.
    sim.pause_mover = Some(who);
    sim.pause_movetime = if movetime > time { 0 } else { movetime.max(0) };
    if let Some(raw) = param(control, "sound") {
        let mut parts = raw.split(',');
        let group = parts
            .next()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(-1);
        let index = parts
            .next()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        sim.sounds.push(SoundEvent {
            source: SoundSource::Character(who),
            group,
            index,
            volume: 0.0,
            pan: 0.0,
            channel: -1,
            loop_sound: false,
            priority: 0,
            low_priority: false,
        });
    }
    let _ = assets;
}

fn target_bind(sim: &mut SimState, who: usize, control: &Controller) {
    let Some(target) = sim.fighters[who].target else {
        return;
    };
    let (px, py) = control.get_pair("pos").unwrap_or((0.0, 0.0));
    let time = int_param(sim, who, control, "time").unwrap_or(1);
    let facing = sim.fighters[who].facing as f32;
    sim.fighters[target].pos = Vec2::new(
        sim.fighters[who].pos.x + px * facing,
        sim.fighters[who].pos.y + py,
    );
    sim.fighters[target].facing = -sim.fighters[who].facing;
    sim.fighters[who].target_bind = Some(crate::fighter::TargetBind {
        offset: Vec2::new(px, py),
        time,
    });
}

fn target_facing(sim: &mut SimState, who: usize, control: &Controller) {
    let Some(target) = sim.fighters[who].target else {
        return;
    };
    let value = int_param(sim, who, control, "value").unwrap_or(1);
    sim.fighters[target].facing = if value >= 0 {
        sim.fighters[who].facing
    } else {
        -sim.fighters[who].facing
    };
}

fn target_state(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let Some(target) = sim.fighters[who].target else {
        return;
    };
    let Some(value) = int_param(sim, who, control, "value") else {
        return;
    };
    if value < 0 {
        return;
    }
    let char = char_of(assets, sim, target);
    sim.fighters[target].enter_state(value, char, false);
}

fn target_life_add(sim: &mut SimState, who: usize, control: &Controller) {
    let Some(target) = sim.fighters[who].target else {
        return;
    };
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let kill = int_param(sim, who, control, "kill").unwrap_or(1) != 0;
    let fighter = &mut sim.fighters[target];
    fighter.life = (fighter.life + value).clamp(0, fighter.life_max);
    if !kill && fighter.life <= 0 {
        fighter.life = 1;
    }
}

fn bracket_index(key: &str, prefix: &str) -> Option<usize> {
    let rest = key.strip_prefix(prefix)?.trim();
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    inner.trim().parse().ok()
}
