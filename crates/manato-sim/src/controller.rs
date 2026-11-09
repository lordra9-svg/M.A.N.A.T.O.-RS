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
    &assets.characters[sim.fighters[who].char_id]
}

pub fn execute(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let kind = control.kind.trim().to_ascii_lowercase();
    match kind.as_str() {
        "null" => {}
        "changestate" | "selfstate" => change_state(sim, who, control, assets),
        "changeanim" | "changeanim2" => change_anim(sim, who, control, assets),
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
        "lifeset" => life_set(sim, who, control),
        "hitadd" => {
            let value = int_param(sim, who, control, "value").unwrap_or(0);
            sim.fighters[who].attack.hit_count =
                sim.fighters[who].attack.hit_count.saturating_add(value);
        }
        "movehitreset" => {
            sim.fighters[who].attack.move_contact = false;
            sim.fighters[who].attack.move_hit = false;
            sim.fighters[who].attack.move_guarded = false;
            sim.fighters[who].attack.hit_count = 0;
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
            sim.env_color = Some(crate::EnvColor {
                color: [r / 255.0, g / 255.0, b / 255.0, 1.0],
                time,
            });
        }
        "fallenvshake" => env_shake(sim, who, control),
        "envshake" => env_shake(sim, who, control),
        "zoom" => zoom(sim, who, control),
        "lifebaraction" => life_bar_action(sim, who, control),
        "explod" => explod(sim, who, control, assets),
        "modifyexplod" => modify_explod(sim, who, control),
        "removeexplod" => remove_explod(sim, who, control),
        "makedust" => make_dust(sim, who, control),
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
        "targetvelset" => target_vel_set(sim, who, control),
        "targetveladd" => target_vel_add(sim, who, control),
        "afterimage" => after_image(sim, who, control),
        "afterimagetime" => {
            let time = int_param(sim, who, control, "time");
            if let (Some(state), Some(time)) = (sim.fighters[who].after_image.as_mut(), time) {
                state.time = time;
            }
        }
        "palfx" => pal_fx(sim, who, control),
        "trans" => trans(sim, who, control),
        "remappal" | "forcefeedback" => {}
        "projectile" => projectile(sim, who, control, assets),
        "modifyprojectile" => modify_projectile(sim, who, control),
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

fn change_state(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let current = sim.fighters[who].state_no;
    let Some(value) = int_param(sim, who, control, "value") else {
        return;
    };
    if value < 0 {
        return;
    }
    let ctrl = int_param(sim, who, control, "ctrl").map(|v| v != 0);
    let anim = int_param(sim, who, control, "anim");
    let char = char_of(assets, sim, who);
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

fn state_type_set(sim: &mut SimState, who: usize, control: &Controller) {
    use crate::fighter::{MoveType, Physics, StateType};
    if let Some(v) = param(control, "statetype").and_then(StateType::parse) {
        sim.fighters[who].state_type = v;
    }
    if let Some(v) = param(control, "movetype").and_then(MoveType::parse) {
        sim.fighters[who].move_type = v;
    }
    if let Some(v) = param(control, "physics").and_then(Physics::parse) {
        sim.fighters[who].physics = v;
    }
}

fn reversal_def(sim: &mut SimState, who: usize, control: &Controller) {
    let hitdef = HitDef::from_controller(control, sim.hit_activation);
    let time = int_param(sim, who, control, "time").unwrap_or(1).max(1);
    sim.fighters[who].reversal = Some(hitdef);
    sim.fighters[who].reversal_time = time;
}

fn trans(sim: &mut SimState, who: usize, control: &Controller) {
    let kind = str_param(control, "trans")
        .unwrap_or("default")
        .to_ascii_lowercase();
    let alpha = int_param(sim, who, control, "alpha");
    sim.fighters[who].trans_alpha = match (kind.as_str(), alpha) {
        ("default", _) => 1.0,
        (_, Some(alpha)) => (alpha as f32 / 255.0).clamp(0.0, 1.0),
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
    sim.fighters[who].after_image = Some(crate::fighter::AfterImageState {
        time: time.max(0),
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
    if let Some(value) = control.get("add") {
        if let Some(add) = parse_triplet(sim, who, value) {
            sim.fighters[who].pal_add = add;
        }
    }
    if let Some(value) = control.get("mul") {
        if let Some(mul) = parse_triplet(sim, who, value) {
            sim.fighters[who].pal_mul = mul;
        }
    }
    if let Some(value) = control.get("alpha") {
        sim.fighters[who].pal_alpha = trigger::num(sim, who, value).clamp(0.0, 1.0);
    }
    sim.fighters[who].pal_time = int_param(sim, who, control, "time").unwrap_or(1);
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
    let base = match postype.as_str() {
        "p2" => sim.fighters[opponent].pos,
        "front" => Vec2::new(owner_pos.x + facing as f32 * 100.0, owner_pos.y),
        "back" => Vec2::new(owner_pos.x - facing as f32 * 100.0, owner_pos.y),
        "left" => Vec2::new(owner_pos.x - 100.0, owner_pos.y),
        "right" => Vec2::new(owner_pos.x + 100.0, owner_pos.y),
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
    entity.helper_id = id;
    entity.helper_remove_time = int_param(sim, who, control, "removetime").unwrap_or(-1);
    entity.enter_state(stateno, character, false);
    sim.fighters.push(entity);
}

fn hit_override(sim: &mut SimState, who: usize, control: &Controller) {
    let Some(attr) = str_param(control, "attr") else {
        return;
    };
    let state = int_param(sim, who, control, "stateno")
        .or_else(|| int_param(sim, who, control, "state"))
        .unwrap_or(0);
    let time = int_param(sim, who, control, "time").unwrap_or(1);
    sim.fighters[who].hit_override = Some(crate::fighter::HitOverride {
        attr: attr.to_ascii_uppercase(),
        state,
        time: time.max(0),
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
}

fn life_set(sim: &mut SimState, who: usize, control: &Controller) {
    let value = int_param(sim, who, control, "value").unwrap_or(0);
    let kill = int_param(sim, who, control, "kill").unwrap_or(1) != 0;
    let fighter = &mut sim.fighters[who];
    fighter.life = value.clamp(0, fighter.life_max);
    if !kill && fighter.life <= 0 {
        fighter.life = 1;
    }
}

fn target_power_add(sim: &mut SimState, who: usize, control: &Controller) {
    let value = num_param(sim, who, control, "value").unwrap_or(0.0);
    if let Some(target) = sim.fighters[who].target {
        sim.fighters[target].power = clamp(sim.fighters[target].power + value, 0.0, MAX_POWER);
    }
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
    let base = match pos_type.as_str() {
        "p2" => sim.fighters[sim.other(who)].pos,
        "front" => Vec2::new(
            sim.fighters[who].pos.x + facing as f32 * 100.0,
            sim.fighters[who].pos.y,
        ),
        "back" => Vec2::new(
            sim.fighters[who].pos.x - facing as f32 * 100.0,
            sim.fighters[who].pos.y,
        ),
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

fn modify_explod(sim: &mut SimState, who: usize, control: &Controller) {
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
        if value > 0 {
            effect.duration = value;
            effect.remove_on_anim_end = false;
        }
    }
    if let Some(value) = sprite_priority {
        effect.sprite_priority = value;
    }
    if let Some(value) = ontop {
        effect.behind = value == 0;
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
    let mut hitdef = HitDef::from_controller(control, sim.hit_activation);
    // Projectile sparks sit at the impact point, not at a body offset.
    hitdef.sparkxy = Vec2::ZERO;
    let remove_time = int_param(sim, who, control, "removetime").unwrap_or(-1);
    let hits = int_param(sim, who, control, "hits").unwrap_or(1).max(1);
    let id = int_param(sim, who, control, "id").unwrap_or(0);
    let pos = Vec2::new(
        sim.fighters[who].pos.x + px * facing as f32,
        sim.fighters[who].pos.y + py,
    );
    sim.projectiles.push(crate::projectile::Projectile {
        owner: who,
        id,
        anim,
        pos,
        vel: Vec2::new(vx * facing as f32, vy),
        accel: Vec2::new(ax * facing as f32, ay),
        facing,
        time: 0,
        remove_time,
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
fn modify_projectile(sim: &mut SimState, who: usize, control: &Controller) {
    let explicit_id = int_param(sim, who, control, "id");
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
    let remove_time = int_param(sim, who, control, "removetime");
    let hits = int_param(sim, who, control, "hits");
    let damage = control.get("damage").and_then(|raw| {
        raw.split(',')
            .next()
            .and_then(|value| value.trim().parse::<i32>().ok())
    });
    for index in indices {
        let projectile = &mut sim.projectiles[index];
        if let Some(anim) = anim {
            projectile.anim = anim;
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
    let hitdef = HitDef::from_controller(control, sim.hit_activation);
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
    if let Some(value) = int_param(sim, who, control, "value") {
        sim.fighters[who].hit.vars.fall = value != 0;
    }
    if let Some(x) = num_param(sim, who, control, "xvel") {
        sim.fighters[who].hit.vars.x_vel = x * sim.fighters[who].facing as f32;
    }
    if let Some(y) = num_param(sim, who, control, "yvel") {
        sim.fighters[who].hit.vars.y_vel = y;
    }
}

fn hit_fall_vel(sim: &mut SimState, who: usize, _control: &Controller) {
    let vars = sim.fighters[who].hit.vars.clone();
    sim.fighters[who].vel = Vec2::new(vars.x_vel, vars.y_vel);
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
        let flag = match name {
            "invisible" => crate::fighter::ASSERT_INVISIBLE,
            "noshadow" => crate::fighter::ASSERT_NOSHADOW,
            "nowalk" => crate::fighter::ASSERT_NOWALK,
            "noairguard" => crate::fighter::ASSERT_NOAIRGUARD,
            "nostandguard" => crate::fighter::ASSERT_NOSTANDGUARD,
            "nocrouchguard" => crate::fighter::ASSERT_NOCROUCHGUARD,
            "unguardable" => crate::fighter::ASSERT_UNGUARDABLE,
            "nojugglecheck" => crate::fighter::ASSERT_NOJUGGLECHECK,
            "noautoturn" => crate::fighter::ASSERT_NOAUTOTURN,
            "intro" => crate::fighter::ASSERT_INTRO,
            "nobg" => crate::fighter::ASSERT_NOBG,
            "nofg" => crate::fighter::ASSERT_NOFG,
            "noko" => crate::fighter::ASSERT_NOKO,
            "nokoslow" => crate::fighter::ASSERT_NOKOSLOW,
            "nokosnd" => crate::fighter::ASSERT_NOKOSND,
            "nomusic" => crate::fighter::ASSERT_NOMUSIC,
            "globalnoshadow" => crate::fighter::ASSERT_GLOBALNOSHADOW,
            "nobardisplay" => crate::fighter::ASSERT_NOBARDISPLAY,
            "roundnotover" => crate::fighter::ASSERT_ROUNDNOTOVER,
            _ => 0,
        };
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

fn width(sim: &mut SimState, who: usize, control: &Controller) {
    let Some((back, front)) = control.get_pair("edge") else {
        return;
    };
    sim.fighters[who].width_override = Some((back, front));
    sim.fighters[who].width_back = back;
    sim.fighters[who].width_front = front;
}

fn env_shake(sim: &mut SimState, who: usize, control: &Controller) {
    let time = int_param(sim, who, control, "time").unwrap_or(0);
    let freq = num_param(sim, who, control, "freq").unwrap_or(60.0);
    let ampl = num_param(sim, who, control, "ampl").unwrap_or(-4.0);
    let phase = num_param(sim, who, control, "phase").unwrap_or(0.0);
    sim.env_shake = EnvShake {
        time: time.max(0),
        freq,
        ampl,
        phase,
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
        let cam = &sim.assets.stage.camera;
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
    let (base, bind_target) = match pos_type.as_str() {
        "p2" => (sim.fighters[sim.other(who)].pos, Some(sim.other(who))),
        "front" => (
            Vec2::new(
                sim.fighters[who].pos.x + facing_f * 100.0,
                sim.fighters[who].pos.y,
            ),
            Some(who),
        ),
        "back" => (
            Vec2::new(
                sim.fighters[who].pos.x - facing_f * 100.0,
                sim.fighters[who].pos.y,
            ),
            Some(who),
        ),
        "left" => (
            Vec2::new(
                sim.camera.x - manato_core::GAME_WIDTH * 0.5,
                sim.fighters[who].pos.y,
            ),
            None,
        ),
        "right" => (
            Vec2::new(
                sim.camera.x + manato_core::GAME_WIDTH * 0.5,
                sim.fighters[who].pos.y,
            ),
            None,
        ),
        _ => (sim.fighters[who].pos, Some(who)),
    };

    let source = if fight_fx {
        EffectSource::FightFx
    } else {
        EffectSource::Character(sim.fighters[who].char_id)
    };
    let anim_time = if fight_fx {
        assets.effects.animations.get(&anim).map(|a| a.total_time())
    } else {
        assets.characters[sim.fighters[who].char_id]
            .animation(anim)
            .map(|a| a.total_time())
    }
    .unwrap_or(10);

    let remove_time = int_param(sim, who, control, "removetime").unwrap_or(-1);
    let duration = if remove_time > 0 {
        remove_time
    } else {
        anim_time
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
        remove_on_anim_end: remove_time <= 0,
        id,
        owner: who,
        behind: int_param(sim, who, control, "ontop").unwrap_or(1) == 0,
        bind_time,
        bind_target: if bind_time != 0 { bind_target } else { None },
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
        anim_elem: int_param(sim, who, control, "animelem").unwrap_or(1),
        anim_freeze: int_param(sim, who, control, "animfreeze").unwrap_or(0) != 0,
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
    let (px, py) = control.get_pair("pos").unwrap_or((0.0, 0.0));
    let facing = sim.fighters[who].facing as f32;
    let pos = Vec2::new(
        sim.fighters[who].pos.x + px * facing,
        sim.fighters[who].pos.y + py,
    );
    let duration = sim
        .assets
        .effects
        .animations
        .get(&120)
        .map(|a| a.total_time())
        .unwrap_or(10);
    sim.effects.push(Effect {
        source: EffectSource::FightFx,
        anim: 120,
        pos,
        facing: sim.fighters[who].facing,
        sprite_priority: -1,
        duration,
        owner: who,
        behind: true,
        ..Effect::default()
    });
}

fn pause(sim: &mut SimState, who: usize, control: &Controller) {
    let time = int_param(sim, who, control, "time").unwrap_or(0).max(0);
    let _move = int_param(sim, who, control, "movetime").unwrap_or(0);
    if time > 0 {
        sim.super_pause = time;
        sim.super_pause_move_time = 0;
    }
}

fn super_pause(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let time = int_param(sim, who, control, "time").unwrap_or(0).max(0);
    let movetime = int_param(sim, who, control, "movetime").unwrap_or(0);
    sim.super_pause = time;
    sim.super_pause_move_time = movetime;
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
    let life = sim.fighters[target].life + value;
    sim.fighters[target].life = life.clamp(0, sim.fighters[target].life_max);
}

fn bracket_index(key: &str, prefix: &str) -> Option<usize> {
    let rest = key.strip_prefix(prefix)?.trim();
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    inner.trim().parse().ok()
}
