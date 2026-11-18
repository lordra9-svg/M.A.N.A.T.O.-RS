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
        "tagin" => {
            let incoming = int_param(sim, who, control, "stateno")
                .or_else(|| int_param(sim, who, control, "partnerstateno"))
                .filter(|state| *state >= 0);
            let ctrl = int_param(sim, who, control, "partnerctrl")
                .or_else(|| int_param(sim, who, control, "ctrl"))
                .unwrap_or(1)
                != 0;
            sim.tag_in(who, incoming, ctrl);
        }
        "tagout" => {
            let outgoing = int_param(sim, who, control, "stateno").filter(|state| *state >= 0);
            let incoming = int_param(sim, who, control, "partnerstateno")
                .or_else(|| int_param(sim, who, control, "stateno"))
                .filter(|state| *state >= 0);
            let ctrl = int_param(sim, who, control, "partnerctrl")
                .or_else(|| int_param(sim, who, control, "ctrl"))
                .unwrap_or(1)
                != 0;
            sim.tag_out(who, outgoing, incoming, ctrl);
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
        "playbgm" => play_bgm(sim, who, control),
        "modifybgm" => modify_bgm(sim, who, control),
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
            if let Some(raw) = control.get("value") {
                // Ikemen accepts one value for both axes or an x,y pair.
                let parts: Vec<f32> = raw
                    .split(',')
                    .map(|part| trigger::num(sim, who, part.trim()))
                    .collect();
                let x = parts.first().copied().unwrap_or(1.0) != 0.0;
                let y = parts.get(1).map(|value| *value != 0.0).unwrap_or(x);
                sim.fighters[who].screen_bound = (x, y);
            }
            let mut move_camera = sim.fighters[who].move_camera;
            if let Some(raw) = control.get("movecamera") {
                let parts: Vec<f32> = raw
                    .split(',')
                    .map(|part| trigger::num(sim, who, part.trim()))
                    .collect();
                move_camera.0 = parts.first().copied().unwrap_or(1.0) != 0.0;
                move_camera.1 = parts
                    .get(1)
                    .map(|value| *value != 0.0)
                    .unwrap_or(move_camera.0);
            }
            if let Some(raw) = control.get("movecamerax") {
                move_camera.0 = trigger::num(sim, who, raw) != 0.0;
            }
            if let Some(raw) = control.get("movecameray") {
                move_camera.1 = trigger::num(sim, who, raw) != 0.0;
            }
            sim.fighters[who].move_camera = move_camera;
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
        "overrideclsn" => override_clsn(sim, who, control),
        "shiftinput" => shift_input(sim, who, control),
        "savestate" => sim.save_state_requested = true,
        "savefile" => {
            if control.get("path").is_some() {
                let path = text_value(sim, who, control, "path");
                if let Some(path) = sandboxed_path(sim, who, &path) {
                    sim.save_file_requested = Some(path);
                }
            }
        }
        "loadfile" => {
            if control.get("path").is_some() {
                let path = text_value(sim, who, control, "path");
                if let Some(path) = sandboxed_path(sim, who, &path) {
                    sim.load_file_requested = Some(path);
                }
            }
        }
        "loadstate" => sim.load_state_requested = true,
        "matchrestart" => sim.restart_requested = true,
        "depth" => depth_controller(sim, who, control),
        "depthbound" => {
            sim.fighters[who].depth_bound = int_param(sim, who, control, "value").unwrap_or(0) != 0;
        }
        "shaderset" => shader_set(sim, who, control),
        "groundleveloffset" => {
            sim.fighters[who].ground_level = num_param(sim, who, control, "value").unwrap_or(0.0);
        }
        "remapsprite" => {
            if int_param(sim, who, control, "reset").unwrap_or(0) != 0 {
                sim.fighters[who].sprite_remap.clear();
            }
            if control.get("preset").is_some() {
                let name = text_value(sim, who, control, "preset").to_ascii_lowercase();
                let table = sim.character(who).def.remap_presets.get(&name).cloned();
                if let Some(table) = table {
                    for (src, dst) in table {
                        sim.fighters[who].sprite_remap.insert(src, dst);
                    }
                }
            }
            let pair = |key: &str| {
                control.get(key).and_then(|raw| {
                    let mut parts = raw.split(',');
                    let first = parts.next().map(|part| trigger::int(sim, who, part))?;
                    let second = parts
                        .next()
                        .map(|part| trigger::int(sim, who, part))
                        .unwrap_or(-1);
                    Some((first, second))
                })
            };
            if let (Some(src), Some(dst)) = (pair("source"), pair("dest")) {
                if src.0 >= 0 && src.1 >= 0 && dst.0 >= 0 && dst.1 >= 0 {
                    sim.fighters[who].sprite_remap.insert(src, dst);
                }
            }
        }
        "remappal" => {
            let pair = |key: &str| {
                control.get(key).map(|raw| {
                    let mut parts = raw.split(',');
                    let first = parts
                        .next()
                        .map(|part| trigger::int(sim, who, part))
                        .unwrap_or(-1);
                    let second = parts
                        .next()
                        .map(|part| trigger::int(sim, who, part))
                        .unwrap_or(0);
                    (first, second)
                })
            };
            let src = pair("source").unwrap_or((-1, 0));
            let mut dst = pair("dest").unwrap_or((-1, 0));
            if src.0 == -1 && dst.0 == -1 {
                sim.fighters[who].pal_remap = None;
                sim.fighters[who].remap_palette = None;
                sim.fighters[who].pal_remap_source = None;
                return;
            }
            if dst.0 == -1 {
                dst = src;
            }
            let palette = sim.character(who).palettes.get(&dst).cloned();
            if let Some(palette) = palette {
                sim.fighters[who].pal_remap = Some(dst);
                sim.fighters[who].remap_palette = Some(palette);
                sim.fighters[who].pal_remap_source = (src.0 >= 0).then_some(src);
            }
        }
        "changemovelist" => {
            let value = int_param(sim, who, control, "value").unwrap_or(0);
            if sim.character(who).movelists.contains_key(&value) {
                sim.fighters[who].movelist = value;
            }
        }
        "displaytoclipboard" | "appendtoclipboard" => clipboard_controller(sim, who, control),
        "clearclipboard" => sim.debug_output.clear(),
        "text" => text_controller(sim, who, control),
        "storyboard" => storyboard_controller(sim, who, control),
        "modifystoryboard" => modify_storyboard(sim, who, control),
        "modifytext" => modify_text(sim, who, control),
        "removetext" => remove_text(sim, who, control),
        "assertcommand" => assert_command(sim, who, control),
        "assertinput" => assert_input(sim, who, control),
        "assertanalogvector" => assert_analog_vector(sim, who, control),
        "modifystagebg" => modify_stage_bg(sim, who, control),
        "modifybgctrl" => modify_bg_ctrl(sim, who, control),
        "modifybgctrl3d" => modify_bg_ctrl_3d(sim, who, control),
        "allpalfx" => {
            let pal = parse_global_pal(sim, who, control);
            sim.all_pal = pal;
        }
        "bgpalfx" => {
            let pal = parse_global_pal(sim, who, control);
            sim.bg_pal = pal;
        }
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
        "forcefeedback" => {}
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
    let elemtime = int_param(sim, who, control, "elemtime").unwrap_or(0);
    let char = char_of(assets, sim, who);
    sim.fighters[who].set_anim_at(value, char, elem, elemtime);
}

/// `ChangeAnim2`: like `ChangeAnim`, but the animation is read from another
/// player's bank (`readplayerid`, defaulting to the state owner).
fn change_anim2(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let Some(value) = int_param(sim, who, control, "value") else {
        return;
    };
    let elem = int_param(sim, who, control, "elem");
    let elemtime = int_param(sim, who, control, "elemtime").unwrap_or(0);
    let player = int_param(sim, who, control, "readplayerid")
        .map(|id| id.clamp(0, 1) as usize)
        .unwrap_or_else(|| sim.root_of(who));
    let anim_char_id = sim.fighters[player].char_id;
    let anim_char = &assets.characters[anim_char_id];
    sim.fighters[who].set_anim_from_at(value, anim_char, elem, elemtime, anim_char_id);
}

fn vel_set(sim: &mut SimState, who: usize, control: &Controller) {
    let x = num_param(sim, who, control, "x");
    let y = num_param(sim, who, control, "y");
    let z = num_param(sim, who, control, "z");
    let facing = sim.fighters[who].facing as f32;
    if let Some(x) = x {
        sim.fighters[who].vel.x = x * facing;
    }
    if let Some(y) = y {
        sim.fighters[who].vel.y = y;
    }
    if let Some(z) = z {
        sim.fighters[who].vel_z = z;
    }
}

fn vel_add(sim: &mut SimState, who: usize, control: &Controller) {
    let x = num_param(sim, who, control, "x");
    let y = num_param(sim, who, control, "y");
    let z = num_param(sim, who, control, "z");
    let facing = sim.fighters[who].facing as f32;
    if let Some(x) = x {
        sim.fighters[who].vel.x += x * facing;
    }
    if let Some(y) = y {
        sim.fighters[who].vel.y += y;
    }
    if let Some(z) = z {
        sim.fighters[who].vel_z += z;
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
    let z = num_param(sim, who, control, "z");
    if let Some(x) = x {
        sim.fighters[who].pos.x = x;
    }
    if let Some(y) = y {
        sim.fighters[who].pos.y = y;
    }
    if let Some(z) = z {
        sim.fighters[who].z = z;
    }
}

fn pos_add(sim: &mut SimState, who: usize, control: &Controller) {
    let x = num_param(sim, who, control, "x");
    let y = num_param(sim, who, control, "y");
    let z = num_param(sim, who, control, "z");
    let facing = sim.fighters[who].facing as f32;
    if let Some(x) = x {
        sim.fighters[who].pos.x += x * facing;
    }
    if let Some(y) = y {
        sim.fighters[who].pos.y += y;
    }
    if let Some(z) = z {
        sim.fighters[who].z += z;
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
    // Ikemen accepts the first letter of a type name, so quoted and full-word
    // forms such as "Air" work as well as `A`.
    let letter = |raw: &str| -> Option<char> {
        raw.trim()
            .trim_matches('"')
            .trim()
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase())
    };
    if let Some(raw) = param(control, "statetype") {
        let parsed = match letter(raw) {
            Some('S') => Some(StateType::Stand),
            Some('C') => Some(StateType::Crouch),
            Some('A') => Some(StateType::Air),
            Some('L') => Some(StateType::Liedown),
            _ => None,
        };
        if let Some(v) = parsed {
            sim.fighters[who].prev_state_type = sim.fighters[who].state_type;
            sim.fighters[who].state_type = v;
        }
    }
    if let Some(raw) = param(control, "movetype") {
        let parsed = match letter(raw) {
            Some('I') => Some(MoveType::Idle),
            Some('A') => Some(MoveType::Attack),
            Some('H') => Some(MoveType::Hit),
            _ => None,
        };
        if let Some(v) = parsed {
            sim.fighters[who].prev_move_type = sim.fighters[who].move_type;
            sim.fighters[who].move_type = v;
        }
    }
    if let Some(raw) = param(control, "physics") {
        let parsed = match letter(raw) {
            Some('S') => Some(Physics::Stand),
            Some('C') => Some(Physics::Crouch),
            Some('A') => Some(Physics::Air),
            Some('N') => Some(Physics::None),
            _ => None,
        };
        if let Some(v) = parsed {
            sim.fighters[who].physics = v;
        }
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
    let mut pal_add = [0.0; 3];
    if let Some(value) = control.get("palbright") {
        if let Some(bright) = parse_triplet(sim, who, value) {
            pal_add = bright;
        }
    }
    let mut pal_mul = [1.0; 3];
    if let Some(value) = control.get("palcontrast") {
        if let Some(contrast) = parse_triplet(sim, who, value) {
            pal_mul = [
                contrast[0] / 256.0,
                contrast[1] / 256.0,
                contrast[2] / 256.0,
            ];
        }
    }
    let mut pal_step_add = [0.0; 3];
    if let Some(value) = control.get("paladd") {
        if let Some(step) = parse_triplet(sim, who, value) {
            pal_step_add = step;
        }
    }
    let mut pal_step_mul = [1.0; 3];
    if let Some(value) = control.get("palmul") {
        let parts: Vec<f32> = value
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        for (axis, step) in pal_step_mul.iter_mut().enumerate() {
            if let Some(part) = parts.get(axis) {
                *step = *part;
            }
        }
    }
    let mut pal_post = [0.0; 3];
    if let Some(value) = control.get("palpostbright") {
        if let Some(post) = parse_triplet(sim, who, value) {
            pal_post = post;
        }
    }
    let pal_color = control
        .get("palcolor")
        .map(|value| (trigger::num(sim, who, value) / 256.0).clamp(0.0, 1.0))
        .unwrap_or(1.0);
    let pal_hue = control
        .get("palhue")
        .map(|value| (trigger::num(sim, who, value) / 512.0).clamp(-0.5, 0.5))
        .unwrap_or(0.0);
    let mut pal_invert = control
        .get("palinvertall")
        .map(|value| trigger::num(sim, who, value) != 0.0)
        .unwrap_or(false);
    if control
        .get("palinvertblend")
        .map(|value| trigger::num(sim, who, value).abs() >= 2.0)
        .unwrap_or(false)
    {
        pal_invert = true;
    }
    // Negative time runs forever; zero disables emission but keeps the state.
    sim.fighters[who].after_image = Some(crate::fighter::AfterImageState {
        time,
        interval,
        tick: 0,
        life,
        alpha: 0.45,
        add,
        pal_add,
        pal_mul,
        pal_step_add,
        pal_step_mul,
        pal_post,
        pal_color,
        pal_hue,
        pal_invert,
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
        fighter.pal_base_mul = [1.0; 3];
        fighter.pal_sinmul = [0.0; 3];
        fighter.pal_sinmul_period = 0;
        fighter.pal_sinmul_cycle = 0;
        fighter.pal_alpha = 1.0;
        fighter.pal_color = 1.0;
        fighter.pal_base_color = 1.0;
        fighter.pal_sincolor = 0.0;
        fighter.pal_sincolor_period = 0;
        fighter.pal_sincolor_cycle = 0;
        fighter.pal_hue = 0.0;
        fighter.pal_base_hue = 0.0;
        fighter.pal_sinhue = 0.0;
        fighter.pal_sinhue_period = 0;
        fighter.pal_sinhue_cycle = 0;
        fighter.pal_invert = false;
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
    if let Some(value) = control.get("sinmul") {
        let parts: Vec<f32> = value
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        if parts.len() >= 3 {
            let period = parts.get(3).copied().unwrap_or(0.0) as i32;
            let side = if period < 0 { -1.0 } else { 1.0 };
            // Ikemen's `sinmul` uses the same 0–256 units as `mul`.
            sim.fighters[who].pal_sinmul = [
                parts[0] / 256.0 * side,
                parts[1] / 256.0 * side,
                parts[2] / 256.0 * side,
            ];
            sim.fighters[who].pal_sinmul_period = period.abs();
            sim.fighters[who].pal_sinmul_cycle = 0;
            sim.fighters[who].pal_mul = sim.fighters[who].pal_base_mul;
        }
    }
    if let Some(value) = control.get("mul") {
        if let Some(mul) = parse_triplet(sim, who, value) {
            // `PalFX` documents `mul` in 0..256 units with 256 as identity;
            // store the normalized multiplier used by rendering and triggers.
            sim.fighters[who].pal_mul = [mul[0] / 256.0, mul[1] / 256.0, mul[2] / 256.0];
            sim.fighters[who].pal_base_mul = sim.fighters[who].pal_mul;
        }
    }
    if let Some(value) = control.get("alpha") {
        sim.fighters[who].pal_alpha = trigger::num(sim, who, value).clamp(0.0, 1.0);
    }
    if let Some(value) = control.get("color") {
        let color = (trigger::num(sim, who, value) / 256.0).clamp(0.0, 1.0);
        sim.fighters[who].pal_color = color;
        sim.fighters[who].pal_base_color = color;
    }
    if let Some(value) = control.get("hue") {
        let hue = (trigger::num(sim, who, value) / 512.0).clamp(-0.5, 0.5);
        sim.fighters[who].pal_hue = hue;
        sim.fighters[who].pal_base_hue = hue;
    }
    if let Some(value) = control.get("sincolor") {
        let parts: Vec<f32> = value
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        if let Some(magnitude) = parts.first() {
            let period = parts.get(1).copied().unwrap_or(0.0) as i32;
            let side = if period < 0 { -1.0 } else { 1.0 };
            let fighter = &mut sim.fighters[who];
            fighter.pal_sincolor = magnitude / 256.0 * side;
            fighter.pal_sincolor_period = period.abs();
            fighter.pal_sincolor_cycle = 0;
            fighter.pal_color = fighter.pal_base_color;
        }
    }
    if let Some(value) = control.get("sinhue") {
        let parts: Vec<f32> = value
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        if let Some(magnitude) = parts.first() {
            let period = parts.get(1).copied().unwrap_or(0.0) as i32;
            let side = if period < 0 { -1.0 } else { 1.0 };
            let fighter = &mut sim.fighters[who];
            fighter.pal_sinhue = magnitude / 512.0 * side;
            fighter.pal_sinhue_period = period.abs();
            fighter.pal_sinhue_cycle = 0;
            fighter.pal_hue = fighter.pal_base_hue;
        }
    }
    if let Some(value) = control.get("invertall") {
        sim.fighters[who].pal_invert = trigger::num(sim, who, value) != 0.0;
    }
    if let Some(value) = control.get("invertblend") {
        // Ikemen clamps to -1..2; blends of 2 or more add the inverted pass.
        let blend = trigger::num(sim, who, value);
        if blend.abs() >= 2.0 {
            sim.fighters[who].pal_invert = true;
        }
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
    let time = int_param(sim, who, control, "time").unwrap_or(1);
    let (px, py) = control.get_pair("pos").unwrap_or((0.0, 0.0));
    let facing = match target {
        // BindToTarget has no facing parameter; it never changes facing.
        BindTarget::Target => 0,
        _ => int_param(sim, who, control, "facing")
            .unwrap_or(0)
            .clamp(-1, 1),
    };
    if time == 0 {
        let fighter = &mut sim.fighters[who];
        fighter.bind_anchor = None;
        fighter.bind_time = 0;
        fighter.bind_facing = 0;
        return;
    }
    let anchor_facing = sim.fighters[anchor].facing;
    let base = sim.fighters[anchor].pos;
    let fighter = &mut sim.fighters[who];
    fighter.bind_anchor = Some(anchor);
    fighter.bind_time = time;
    fighter.bind_offset = Vec2::new(px, py);
    fighter.bind_facing = facing;
    // Apply immediately so the position changes on the controller's tick.
    fighter.pos = Vec2::new(base.x + px * anchor_facing as f32, base.y + py);
    if facing != 0 {
        fighter.facing = anchor_facing * facing.signum();
    }
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
    let absolute = int_param(sim, who, control, "absolute").unwrap_or(0) != 0;
    let fighter = &mut sim.fighters[who];
    let mut add = value as f32;
    // Damage (negative values) goes through final defence unless absolute.
    // Healing is never scaled, matching M.U.G.E.N.
    if !absolute && add < 0.0 {
        add /= fighter.defence_mul.max(0.01);
        // At least one point of damage regardless of defence.
        if add > -1.0 {
            add = -1.0;
        }
    }
    let add = add.round() as i32;
    let previous = fighter.life;
    fighter.life = (fighter.life.saturating_add(add)).clamp(0, fighter.life_max);
    if add < 0 {
        let applied = (previous - fighter.life).max(0);
        fighter.received_damage = fighter.received_damage.saturating_add(applied);
    }
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

/// `OverrideClsn`: per-tick collision-box replacement. Group 0 clears all
/// groups, -1 applies to every group, and 1–4 selects Clsn1, Clsn2, size, or
/// dummy. A zero rect deletes boxes; otherwise `index < 0` replaces every
/// box and an index past the end appends a new one.
fn override_clsn(sim: &mut SimState, who: usize, control: &Controller) {
    let group = int_param(sim, who, control, "group").unwrap_or(0);
    let index = int_param(sim, who, control, "index").unwrap_or(0);
    let rect = control
        .get("rect")
        .map(|raw| {
            let mut parts = raw
                .split(',')
                .map(|value| value.trim().parse::<f32>().unwrap_or(0.0));
            let x1 = parts.next().unwrap_or(0.0);
            let y1 = parts.next().unwrap_or(0.0);
            let x2 = parts.next().unwrap_or(x1);
            let y2 = parts.next().unwrap_or(y1);
            [x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)]
        })
        .unwrap_or([0.0; 4]);
    let overrides = &mut sim.fighters[who].clsn_overrides;
    match group {
        0 => {
            for entries in overrides.iter_mut() {
                entries.clear();
            }
        }
        -1 => {
            let entry = crate::fighter::ClsnOverride { index, rect };
            for entries in overrides.iter_mut() {
                entries.push(entry);
            }
        }
        1..=4 => {
            overrides[(group - 1) as usize].push(crate::fighter::ClsnOverride { index, rect });
        }
        _ => {}
    }
}

/// Resolves a `SaveFile`/`LoadFile` path inside the executing character's
/// directory. Absolute paths and `..` escapes are rejected so untrusted
/// content cannot read or overwrite arbitrary files.
fn sandboxed_path(sim: &SimState, who: usize, raw: &str) -> Option<std::path::PathBuf> {
    let raw = raw.trim().trim_matches('"');
    if raw.is_empty() {
        return None;
    }
    let base = &sim.assets.characters[sim.fighters[who].char_id].def.dir;
    let base = base.canonicalize().unwrap_or_else(|_| base.clone());
    let candidate = std::path::Path::new(raw);
    if candidate.is_absolute() {
        log::warn!("SaveFile/LoadFile rejects absolute path {raw}");
        return None;
    }
    let mut normalized = base.clone();
    for component in candidate.components() {
        match component {
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !normalized.pop() || !normalized.starts_with(&base) {
                    log::warn!("SaveFile/LoadFile rejects path escape {raw}");
                    return None;
                }
            }
            _ => return None,
        }
    }
    if !normalized.starts_with(&base) {
        log::warn!("SaveFile/LoadFile rejects path escape {raw}");
        return None;
    }
    Some(normalized)
}

/// Evaluates a controller string, stripping literal quotes.
fn text_value(sim: &SimState, who: usize, control: &Controller, key: &str) -> String {
    control
        .get(key)
        .map(|raw| {
            let value = trigger::eval_str(sim, who, raw).as_str();
            value.trim_matches('"').to_string()
        })
        .unwrap_or_default()
}

/// Applies the `Text`/`ModifyText` visual parameters shared by both.
fn apply_text_params(
    sim: &SimState,
    who: usize,
    control: &Controller,
    text: &mut crate::text::TextObject,
) {
    if let Some(raw) = control.get("removetime") {
        text.remove_time = trigger::int(sim, who, raw);
    }
    if let Some(raw) = control.get("layerno") {
        text.layer_no = trigger::int(sim, who, raw);
    }
    if let Some(raw) = control.get("id") {
        text.id = trigger::int(sim, who, raw);
    }
    if let Some(raw) = control.get("velocity") {
        let (x, y) = split_pair(raw);
        text.vel = Vec2::new(trigger::num(sim, who, &x), trigger::num(sim, who, &y));
    }
    if let Some(raw) = control.get("accel") {
        let (x, y) = split_pair(raw);
        text.accel = Vec2::new(trigger::num(sim, who, &x), trigger::num(sim, who, &y));
    }
    if let Some(raw) = control.get("friction") {
        let (x, y) = split_pair(raw);
        text.friction = Vec2::new(trigger::num(sim, who, &x), trigger::num(sim, who, &y));
    }
    if let Some(raw) = control.get("scale") {
        let (x, y) = split_pair(raw);
        text.scale = Vec2::new(trigger::num(sim, who, &x), trigger::num(sim, who, &y));
    }
    if let Some(raw) = control.get("textspacing") {
        let (x, y) = split_pair(raw);
        text.spacing = Vec2::new(trigger::num(sim, who, &x), trigger::num(sim, who, &y));
    }
    if let Some(raw) = control.get("textdelay") {
        text.text_delay = trigger::num(sim, who, raw);
    }
    if let Some(raw) = control.get("align") {
        text.align = trigger::int(sim, who, raw);
    }
    if let Some(raw) = control.get("angle") {
        text.angle = trigger::num(sim, who, raw);
    }
    if let Some(raw) = control.get("xshear") {
        text.x_shear = trigger::num(sim, who, raw);
    }
    if let Some(raw) = control.get("color") {
        let mut parts = raw
            .split(',')
            .map(|part| trigger::int(sim, who, part).clamp(0, 255) as u8);
        text.color[0] = parts.next().unwrap_or(255);
        text.color[1] = parts.next().unwrap_or(255);
        text.color[2] = parts.next().unwrap_or(255);
        text.color[3] = parts.next().unwrap_or(255);
    }
    if let Some(raw) = control.get("font") {
        let mut parts = raw.split(',');
        let bank = parts
            .next()
            .map(|part| {
                trigger::eval_str(sim, who, part.trim())
                    .as_str()
                    .trim_matches('"')
                    .chars()
                    .next()
                    .unwrap_or('f')
                    .to_ascii_lowercase()
            })
            .unwrap_or('f');
        text.font_bank = bank;
        text.font = parts
            .next()
            .map(|part| trigger::int(sim, who, part))
            .unwrap_or(-1);
        let _ = who;
    }
}

/// Formats `%d`/`%i`/`%f`/`%s` placeholders from the controller's `params`.
fn format_text(sim: &SimState, who: usize, control: &Controller, template: &str) -> String {
    let raw = control.get("params").unwrap_or_default();
    let params: Vec<String> = if raw.trim().is_empty() {
        Vec::new()
    } else {
        raw.split(',')
            .map(|part| {
                let value = trigger::eval_str(sim, who, part.trim());
                match value {
                    crate::expr::Value::Str(_) => value.as_str(),
                    crate::expr::Value::Float(f) => {
                        if f.fract() == 0.0 {
                            format!("{}", f as i32)
                        } else {
                            format!("{f}")
                        }
                    }
                    _ => format!("{}", value.as_i32()),
                }
            })
            .collect()
    };
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    let mut index = 0;
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('d') | Some('i') => {
                out.push_str(params.get(index).map(String::as_str).unwrap_or("0"));
                index += 1;
            }
            Some('f') => {
                out.push_str(params.get(index).map(String::as_str).unwrap_or("0"));
                index += 1;
            }
            Some('s') => {
                out.push_str(params.get(index).map(String::as_str).unwrap_or(""));
                index += 1;
            }
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

fn split_pair(raw: &str) -> (String, String) {
    match raw.split_once(',') {
        Some((x, y)) => (x.trim().to_string(), y.trim().to_string()),
        None => (raw.trim().to_string(), "0".to_string()),
    }
}

/// Resolves a storyboard path against the executing character's directory.
fn resolve_storyboard_path(sim: &SimState, who: usize, path: &str) -> std::path::PathBuf {
    let path = path.trim().trim_matches('"');
    let path = std::path::Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        sim.assets.characters[sim.fighters[who].char_id]
            .def
            .dir
            .join(path)
    }
}

/// `Storyboard`: spawns an in-match storyboard overlay (text layers).
fn storyboard_controller(sim: &mut SimState, who: usize, control: &Controller) {
    let path = text_value(sim, who, control, "path");
    if path.is_empty() {
        return;
    }
    let path = resolve_storyboard_path(sim, who, &path);
    let storyboard = match manato_formats::storyboard::Storyboard::load(&path) {
        Ok(storyboard) => storyboard,
        Err(error) => {
            log::warn!("Storyboard {} failed: {error:#}", path.display());
            return;
        }
    };
    let player = manato_formats::storyboard::StoryboardPlayer::new(&storyboard);
    let (px, py) = control.get_pair("pos").unwrap_or((0.0, 0.0));
    let (sx, sy) = control.get_pair("scale").unwrap_or((1.0, 1.0));
    let id = int_param(sim, who, control, "id").unwrap_or(0);
    let facing = int_param(sim, who, control, "facing").unwrap_or(1);
    let screen_space = str_param(control, "space")
        .map(|space| space.eq_ignore_ascii_case("screen"))
        .unwrap_or(false);
    let bind_time = int_param(sim, who, control, "bindtime").unwrap_or(0);
    let bind_target = (bind_time != 0).then_some(who);
    sim.storyboards.push(crate::StoryboardOverlay {
        storyboard: std::sync::Arc::new(storyboard),
        player,
        owner: who,
        id,
        pos: Vec2::new(px, py),
        scale: Vec2::new(sx, sy),
        facing,
        screen_space,
        bind_target,
        bind_time,
        script_alpha: 1.0,
    });
}

/// `ModifyStoryboard`: updates matching overlays owned by the fighter.
fn modify_storyboard(sim: &mut SimState, who: usize, control: &Controller) {
    let id = int_param(sim, who, control, "id").unwrap_or(-1);
    let pos = control.get_pair("pos");
    let scale = control.get_pair("scale");
    let facing = int_param(sim, who, control, "facing");
    let screen_space =
        str_param(control, "space").map(|space| space.eq_ignore_ascii_case("screen"));
    for overlay in &mut sim.storyboards {
        if overlay.owner != who || (id >= 0 && overlay.id != id) {
            continue;
        }
        if let Some((x, y)) = pos {
            overlay.pos = Vec2::new(x, y);
        }
        if let Some((x, y)) = scale {
            overlay.scale = Vec2::new(x, y);
        }
        if let Some(facing) = facing {
            overlay.facing = facing;
        }
        if let Some(screen_space) = screen_space {
            overlay.screen_space = screen_space;
        }
    }
}

/// `DisplayToClipboard`/`AppendToClipboard`: debug output. The engine does not
/// touch the OS clipboard; output is logged and exposed via `debug_output`.
fn clipboard_controller(sim: &mut SimState, who: usize, control: &Controller) {
    if control.get("text").is_none() {
        return;
    }
    let template = text_value(sim, who, control, "text");
    let message = format_text(sim, who, control, &template);
    log::info!("clipboard: {message}");
    if sim.debug_output.len() < 64 {
        sim.debug_output.push(message);
    }
}

/// `Text`: spawns a screen-space text object owned by the fighter.
fn text_controller(sim: &mut SimState, who: usize, control: &Controller) {
    let template = text_value(sim, who, control, "text");
    let text_string = format_text(sim, who, control, &template);
    let (px, py, pz) = triplet(control, "pos");
    let mut text = crate::text::TextObject::new(who, text_string, Vec2::new(px, py));
    text.z = pz;
    apply_text_params(sim, who, control, &mut text);
    sim.texts.push(text);
}

/// `ModifyText`: updates matching texts of the executing fighter.
fn modify_text(sim: &mut SimState, who: usize, control: &Controller) {
    let id = int_param(sim, who, control, "id").unwrap_or(-1);
    let index = int_param(sim, who, control, "index").unwrap_or(-1);
    let positions: Vec<usize> = sim
        .texts
        .iter()
        .enumerate()
        .filter(|(_, text)| text.owner == who && (id < 0 || text.id == id))
        .map(|(position, _)| position)
        .collect();
    let targets: Vec<usize> = if index < 0 {
        positions
    } else {
        positions.get(index as usize).copied().into_iter().collect()
    };
    if targets.is_empty() {
        return;
    }
    let template = control
        .get("text")
        .map(|_| text_value(sim, who, control, "text"));
    let formatted = template
        .as_ref()
        .map(|template| format_text(sim, who, control, template));
    for position in targets {
        if let Some(formatted) = &formatted {
            sim.texts[position].text = formatted.clone();
        }
        if let Some(raw) = control.get("pos") {
            let (x, y, z) = triplet(control, "pos");
            let _ = raw;
            sim.texts[position].pos = Vec2::new(x, y);
            sim.texts[position].z = z;
        }
        let mut text = sim.texts[position].clone();
        apply_text_params(sim, who, control, &mut text);
        // `id` selects the targets; it does not rewrite the text id.
        text.id = sim.texts[position].id;
        sim.texts[position] = text;
    }
}

/// `RemoveText`: removes matching texts (`id < 0` or `index < 0` selects all).
fn remove_text(sim: &mut SimState, who: usize, control: &Controller) {
    let id = int_param(sim, who, control, "id").unwrap_or(-1);
    let index = int_param(sim, who, control, "index").unwrap_or(-1);
    let mut match_index = 0;
    sim.texts.retain(|text| {
        let matches = text.owner == who && (id < 0 || text.id == id);
        let keep = !(matches && (index < 0 || index == match_index));
        if matches {
            match_index += 1;
        }
        keep
    });
}

/// Searches the character, stage, and shared data directories for a BGM
/// file, preferring their `sound/` subdirectory.
fn resolve_bgm_path(sim: &SimState, who: usize, name: &str) -> Option<std::path::PathBuf> {
    let name = name.trim().trim_matches('"');
    if name.is_empty() {
        return None;
    }
    let direct = std::path::PathBuf::from(name);
    if direct.is_absolute() && direct.exists() {
        return Some(direct);
    }
    let mut dirs = vec![
        sim.assets.characters[sim.fighters[who].char_id]
            .def
            .dir
            .clone(),
        sim.assets.stage.dir.clone(),
    ];
    dirs.extend(sim.assets.data_dirs.iter().cloned());
    for dir in dirs {
        for candidate in [dir.join("sound").join(name), dir.join(name)] {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// `PlayBGM`: starts (or stops, with an empty `bgm`) background music.
/// Only the direct `bgm` form is modeled; `source = "..."` lookups and
/// `startposition`/`freqmul` are parsed but not applied by the mixer.
fn play_bgm(sim: &mut SimState, who: usize, control: &Controller) {
    if control.get("bgm").is_none() {
        return;
    }
    let name = text_value(sim, who, control, "bgm");
    if name.is_empty() {
        sim.bgm = crate::BgmState::default();
        sim.sound_commands.push(crate::SoundCommand::StopBgm);
        return;
    }
    let Some(path) = resolve_bgm_path(sim, who, &name) else {
        log::warn!("PlayBGM could not find {name}");
        return;
    };
    if sim.bgm.active && sim.bgm.filename == name {
        return;
    }
    let volume = int_param(sim, who, control, "volume").unwrap_or(100).max(0);
    let loop_music = int_param(sim, who, control, "loop").unwrap_or(1) != 0;
    let loop_start = int_param(sim, who, control, "loopstart").unwrap_or(0);
    let loop_end = int_param(sim, who, control, "loopend").unwrap_or(0);
    let start_position = int_param(sim, who, control, "startposition").unwrap_or(0);
    let freq_mul = num_param(sim, who, control, "freqmul").unwrap_or(1.0);
    let loop_count = int_param(sim, who, control, "loopcount").unwrap_or(-1);
    sim.bgm = crate::BgmState {
        active: true,
        filename: name,
        volume,
        loop_music,
        loop_start,
        loop_end,
        start_position,
        freq_mul,
        loop_count,
        position_ms: 0,
        length_ms: 0,
        sample_rate: 0,
    };
    sim.sound_commands.push(crate::SoundCommand::PlayBgm {
        path,
        volume,
        loop_music,
        loop_start,
        loop_end,
        start_position,
        freq_mul,
        loop_count,
    });
}

/// `ModifyBGM`: updates the live BGM state and forwards the same values to
/// the audio engine.
fn modify_bgm(sim: &mut SimState, who: usize, control: &Controller) {
    if !sim.bgm.active {
        return;
    }
    let volume = int_param(sim, who, control, "volume");
    let loop_start = int_param(sim, who, control, "loopstart");
    let loop_end = int_param(sim, who, control, "loopend");
    let position = int_param(sim, who, control, "position");
    let freq_mul = num_param(sim, who, control, "freqmul");
    let loop_count = int_param(sim, who, control, "loopcount");
    if let Some(volume) = volume {
        sim.bgm.volume = volume.max(0);
    }
    if let Some(value) = loop_start {
        sim.bgm.loop_start = value;
    }
    if let Some(value) = loop_end {
        sim.bgm.loop_end = value;
    }
    if let Some(value) = freq_mul {
        sim.bgm.freq_mul = value;
    }
    if let Some(value) = loop_count {
        sim.bgm.loop_count = value;
    }
    sim.sound_commands.push(crate::SoundCommand::ModifyBgm {
        volume,
        loop_start,
        loop_end,
        position,
        freq_mul,
        loop_count,
    });
}

/// `Depth`: per-tick depth extents used by 3D collision checks.
/// `player`/`Edge`/`value` each take `top, bottom`.
fn depth_controller(sim: &mut SimState, who: usize, control: &Controller) {
    let pair = |key: &str| {
        control.get(key).map(|raw| {
            let mut parts = raw.split(',');
            let top = parts
                .next()
                .map(|part| trigger::num(sim, who, part))
                .unwrap_or(0.0);
            let bottom = parts
                .next()
                .map(|part| trigger::num(sim, who, part))
                .unwrap_or(top);
            [top, bottom]
        })
    };
    let player = pair("player");
    let edge = pair("edge");
    let value = pair("value");
    if let Some(value) = player {
        sim.fighters[who].depth_player = value;
    }
    if let Some(value) = edge {
        sim.fighters[who].depth_edge = value;
    }
    if let Some(value) = value {
        sim.fighters[who].depth_player = value;
        sim.fighters[who].depth_edge = value;
    }
}

/// `ShaderSet`: selects a `[Shaders]` entry by name for `time` ticks.
/// Shader files are stored but not executed (see V-09).
fn shader_set(sim: &mut SimState, who: usize, control: &Controller) {
    let time = int_param(sim, who, control, "time").unwrap_or(1);
    if control.get("shader").is_none() {
        return;
    }
    let name = text_value(sim, who, control, "shader").to_ascii_lowercase();
    if name.is_empty() {
        sim.fighters[who].shader = None;
        sim.fighters[who].shader_time = 0;
        sim.fighters[who].shader_params = [0.0; 16];
        sim.fighters[who].shader_tex1 = None;
        sim.fighters[who].shader_tex2 = None;
        return;
    }
    if !sim.character(who).def.shaders.contains_key(&name) {
        log::warn!("ShaderSet: unknown shader {name}");
        return;
    }
    // `shaderparam = count, idx, value[, idx, value...]`.
    if let Some(raw) = control.get("shaderparam") {
        let parts: Vec<f32> = raw
            .split(',')
            .map(|part| part.trim().parse::<f32>().unwrap_or(0.0))
            .collect();
        if let Some(count) = parts.first().map(|value| *value as usize) {
            for pair in 0..count {
                let idx = parts.get(1 + pair * 2).copied().unwrap_or(-1.0) as i32;
                let value = parts.get(2 + pair * 2).copied().unwrap_or(0.0);
                if (0..16).contains(&idx) {
                    sim.fighters[who].shader_params[idx as usize] = value;
                }
            }
        }
    }
    // Sprite references used by the `tex1`/`tex2` shader samplers.
    let tex = |key: &str| {
        control.get_pair(key).and_then(|(g, i)| {
            let group = g as i32;
            let index = i as i32;
            (group >= 0 && index >= 0).then_some((group, index))
        })
    };
    if let Some(tex1) = tex("tex1") {
        sim.fighters[who].shader_tex1 = Some(tex1);
    }
    if let Some(tex2) = tex("tex2") {
        sim.fighters[who].shader_tex2 = Some(tex2);
    }
    sim.fighters[who].shader = Some(name);
    sim.fighters[who].shader_time = if time < 0 { -1 } else { time.max(1) };
}

/// `AssertCommand`: keeps a named command active for `buffertime` ticks
/// (`time` is accepted as an alias). An empty name selects the first command
/// in the fighter's list, matching the random-selection fallback closely
/// enough for deterministic content.
fn assert_command(sim: &mut SimState, who: usize, control: &Controller) {
    let mut name = control
        .get("name")
        .map(|value| value.trim().trim_matches('"').to_string())
        .unwrap_or_default();
    let time = int_param(sim, who, control, "buffertime")
        .or_else(|| int_param(sim, who, control, "time"))
        .unwrap_or(1);
    if time == 0 {
        return;
    }
    if name.is_empty() {
        let Some(command) = sim.character(who).commands.first() else {
            return;
        };
        name = command.name.clone();
    }
    let fighter = &mut sim.fighters[who];
    if let Some(entry) = fighter
        .asserted_commands
        .iter_mut()
        .find(|(existing, _)| existing.eq_ignore_ascii_case(&name))
    {
        entry.1 = time;
    } else {
        fighter.asserted_commands.push((name, time));
    }
}

/// `AssertInput`: merges directional/button bits into the next input frame.
/// `flag` accepts `U`/`D`/`L`/`R`, `a`–`c`, `x`–`z`, and `s`; `flag.B` and
/// `flag.F` add back/forward relative to the fighter's facing.
fn assert_input(sim: &mut SimState, who: usize, control: &Controller) {
    use manato_core::{Buttons, DIR_DOWN, DIR_LEFT, DIR_RIGHT, DIR_UP};
    let facing = sim.fighters[who].facing;
    let mut dir = 0u8;
    let mut buttons = 0u8;
    for (key, value) in &control.params {
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().trim_matches('"');
        if key == "flag"
            || key
                .strip_prefix("flag")
                .is_some_and(|n| n.parse::<u8>().is_ok())
        {
            if value.eq_ignore_ascii_case("B") {
                dir |= if facing >= 0 { DIR_LEFT } else { DIR_RIGHT };
            } else if value.eq_ignore_ascii_case("F") {
                dir |= if facing >= 0 { DIR_RIGHT } else { DIR_LEFT };
            } else {
                for ch in value.chars() {
                    match ch.to_ascii_uppercase() {
                        'U' => dir |= DIR_UP,
                        'D' => dir |= DIR_DOWN,
                        'L' => dir |= DIR_LEFT,
                        'R' => dir |= DIR_RIGHT,
                        _ => {
                            if let Some(button) = Buttons::from_letter(ch) {
                                buttons |= button;
                            }
                        }
                    }
                }
            }
        }
    }
    let fighter = &mut sim.fighters[who];
    fighter.asserted_input.dir |= dir;
    fighter.asserted_input.buttons |= buttons;
}

/// `AssertAnalogVector`: sets the six analog axes read by `Analog(...)` for
/// the next tick. Values are clamped like Ikemen (sticks -1..1, triggers
/// 0..1).
fn assert_analog_vector(sim: &mut SimState, who: usize, control: &Controller) {
    let values = [
        control.get("leftx"),
        control.get("lefty"),
        control.get("rightx"),
        control.get("righty"),
        control.get("lefttrigger"),
        control.get("righttrigger"),
    ]
    .map(|raw| raw.map(|raw| trigger::num(sim, who, raw)));
    let axes = &mut sim.fighters[who].analog_axes;
    for (index, value) in values.into_iter().enumerate() {
        let Some(value) = value else {
            continue;
        };
        axes[index] = if index < 4 {
            value.clamp(-1.0, 1.0)
        } else {
            value.clamp(0.0, 1.0)
        };
    }
}

/// `ShiftInput`: per-tick key remapping applied to the next input frame.
/// `input = none, output = none` clears every mapping; an `output` of `none`
/// disables the source key.
fn shift_input(sim: &mut SimState, who: usize, control: &Controller) {
    let input_name = control
        .get("input")
        .map(|value| value.trim())
        .unwrap_or_default();
    let output_name = control
        .get("output")
        .map(|value| value.trim())
        .unwrap_or_default();
    let source = shift_key(input_name);
    let target = if output_name.eq_ignore_ascii_case("none") {
        Some(None)
    } else {
        shift_key(output_name).map(Some)
    };
    let shifts = &mut sim.fighters[who].input_shift;
    let Some(source) = source else {
        if matches!(target, Some(None)) {
            shifts.clear();
        }
        return;
    };
    let Some(target) = target else {
        return;
    };
    if let Some(entry) = shifts.iter_mut().find(|(key, _)| *key == source) {
        entry.1 = target;
    } else {
        shifts.push((source, target));
    }
}

/// `ShiftInput` key indices: 0–3 U/D/L/R, 4–10 a/b/c/x/y/z/s. The
/// Ikemen-only d/w/m buttons have no input bits here and are unmapped.
fn shift_key(name: &str) -> Option<usize> {
    match name {
        "U" => Some(0),
        "D" => Some(1),
        "L" => Some(2),
        "R" => Some(3),
        "a" => Some(4),
        "b" => Some(5),
        "c" => Some(6),
        "x" => Some(7),
        "y" => Some(8),
        "z" => Some(9),
        "s" => Some(10),
        _ => None,
    }
}

/// `AllPalFX`/`BgPalFX`: global palette effects. `add` uses 0–255 units,
/// `mul` 0–256 (256 = identity), and `alpha` 0–1. `time = 0` disables the
/// effect. Color/hue/invert and sine variants apply to per-fighter `PalFX`
/// only and are not modeled globally.
fn parse_global_pal(sim: &SimState, who: usize, control: &Controller) -> crate::GlobalPalFx {
    let time = int_param(sim, who, control, "time").unwrap_or(1);
    let mut pal = crate::GlobalPalFx::default();
    if time == 0 {
        return pal;
    }
    pal.time = time;
    if let Some(value) = control.get("add") {
        if let Some(add) = parse_triplet(sim, who, value) {
            pal.add = add;
            pal.base_add = add;
        }
    }
    if let Some(value) = control.get("mul") {
        if let Some(mul) = parse_triplet(sim, who, value) {
            pal.mul = [mul[0] / 256.0, mul[1] / 256.0, mul[2] / 256.0];
            pal.base_mul = pal.mul;
        }
    }
    if let Some(value) = control.get("alpha") {
        pal.alpha = trigger::num(sim, who, value).clamp(0.0, 1.0);
    }
    if let Some(value) = control.get("color") {
        pal.color = (trigger::num(sim, who, value) / 256.0).clamp(0.0, 1.0);
        pal.base_color = pal.color;
    }
    if let Some(value) = control.get("hue") {
        pal.hue = (trigger::num(sim, who, value) / 512.0).clamp(-0.5, 0.5);
        pal.base_hue = pal.hue;
    }
    if let Some(value) = control.get("sinadd") {
        let parts: Vec<f32> = value
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        if parts.len() >= 3 {
            let period = parts.get(3).copied().unwrap_or(0.0) as i32;
            let side = if period < 0 { -1.0 } else { 1.0 };
            pal.sinadd = [parts[0] * side, parts[1] * side, parts[2] * side];
            pal.sinadd_period = period.abs();
            pal.sinadd_cycle = 0;
            pal.add = pal.base_add;
        }
    }
    if let Some(value) = control.get("sinmul") {
        let parts: Vec<f32> = value
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        if parts.len() >= 3 {
            let period = parts.get(3).copied().unwrap_or(0.0) as i32;
            let side = if period < 0 { -1.0 } else { 1.0 };
            pal.sinmul = [
                parts[0] / 256.0 * side,
                parts[1] / 256.0 * side,
                parts[2] / 256.0 * side,
            ];
            pal.sinmul_period = period.abs();
            pal.sinmul_cycle = 0;
            pal.mul = pal.base_mul;
        }
    }
    if let Some(value) = control.get("sincolor") {
        let parts: Vec<f32> = value
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        if let Some(magnitude) = parts.first() {
            let period = parts.get(1).copied().unwrap_or(0.0) as i32;
            let side = if period < 0 { -1.0 } else { 1.0 };
            pal.sincolor = magnitude / 256.0 * side;
            pal.sincolor_period = period.abs();
            pal.sincolor_cycle = 0;
            pal.color = pal.base_color;
        }
    }
    if let Some(value) = control.get("sinhue") {
        let parts: Vec<f32> = value
            .split(',')
            .map(|part| trigger::num(sim, who, part.trim()))
            .collect();
        if let Some(magnitude) = parts.first() {
            let period = parts.get(1).copied().unwrap_or(0.0) as i32;
            let side = if period < 0 { -1.0 } else { 1.0 };
            pal.sinhue = magnitude / 512.0 * side;
            pal.sinhue_period = period.abs();
            pal.sinhue_cycle = 0;
            pal.hue = pal.base_hue;
        }
    }
    if let Some(value) = control.get("invertall") {
        pal.invert = trigger::num(sim, who, value) != 0.0;
    }
    if let Some(value) = control.get("invertblend") {
        if trigger::num(sim, who, value).abs() >= 2.0 {
            pal.invert = true;
        }
    }
    pal
}

/// `ModifyStageBG` subset: selects stage layers by `id` (`-1` = every id) and
/// `index` (`-1` = every layer), then overrides `actionno`, `velocity.x/y`,
/// `delta.x/y`, `start.x/y`, and `scalestart`. `pos`, `layerno`, `spriteno`,
/// `trans`, angles, shear, projection, and shaders are not modeled.
fn modify_stage_bg(sim: &mut SimState, who: usize, control: &Controller) {
    let id = int_param(sim, who, control, "id").unwrap_or(-1);
    let index = int_param(sim, who, control, "index").unwrap_or(-1);
    let targets: Vec<usize> = sim
        .assets
        .stage
        .layers
        .iter()
        .enumerate()
        .filter(|(position, layer)| {
            (id < 0 || layer.id == id) && (index < 0 || *position as i32 == index)
        })
        .map(|(position, _)| position)
        .collect();
    if targets.is_empty() {
        return;
    }
    let action = int_param(sim, who, control, "actionno");
    let velocity_x = num_param(sim, who, control, "velocity.x");
    let velocity_y = num_param(sim, who, control, "velocity.y");
    let delta_x = num_param(sim, who, control, "delta.x");
    let delta_y = num_param(sim, who, control, "delta.y");
    let start_x = num_param(sim, who, control, "start.x");
    let start_y = num_param(sim, who, control, "start.y");
    let scale = control.get_pair("scalestart");
    let pos_x = num_param(sim, who, control, "pos.x");
    let pos_y = num_param(sim, who, control, "pos.y");
    let layer_no = int_param(sim, who, control, "layerno");
    let sprite = control
        .get_pair("spriteno")
        .map(|(group, index)| (group as i32, index as i32));
    let trans = control.get("trans").and_then(parse_trans_mode);
    for position in targets {
        let base = &sim.assets.stage.layers[position];
        let state = &mut sim.stage_layers[position];
        if let Some(action) = action {
            state.action = Some(action);
        }
        if let Some(value) = velocity_x {
            state.velocity.x = value;
        }
        if let Some(value) = velocity_y {
            state.velocity.y = value;
        }
        if delta_x.is_some() || delta_y.is_some() {
            let mut value = state.delta.unwrap_or(base.delta);
            if let Some(x) = delta_x {
                value.x = x;
            }
            if let Some(y) = delta_y {
                value.y = y;
            }
            state.delta = Some(value);
        }
        if start_x.is_some() || start_y.is_some() {
            let mut value = state.start.unwrap_or(base.start);
            if let Some(x) = start_x {
                value.x = x;
            }
            if let Some(y) = start_y {
                value.y = y;
            }
            state.start = Some(value);
        }
        if let Some(scale) = scale {
            state.scale = Some(Vec2::new(scale.0, scale.1));
        }
        if pos_x.is_some() || pos_y.is_some() {
            let mut value = state.pos.unwrap_or(Vec2::ZERO);
            if let Some(x) = pos_x {
                value.x = x;
            }
            if let Some(y) = pos_y {
                value.y = y;
            }
            state.pos = Some(value);
        }
        if let Some(layer_no) = layer_no {
            state.layer_no = Some(layer_no);
        }
        if let Some(sprite) = sprite {
            state.sprite = Some(sprite);
        }
        if let Some(trans) = trans {
            state.trans = Some(trans);
        }
    }
}

/// Parses `ModifyStageBG trans = src, dst, type` (0 normal, 1 add, 2 sub,
/// 3 subadd). The alpha values are not modeled for stage layers.
fn parse_trans_mode(raw: &str) -> Option<manato_formats::air::BlendMode> {
    use manato_formats::air::BlendMode;
    let kind = raw.split(',').nth(2)?.trim().parse::<i32>().ok()?;
    Some(match kind {
        1 => BlendMode::Add,
        2 => BlendMode::Sub,
        3 => BlendMode::SubAdd,
        _ => BlendMode::Normal,
    })
}

/// `ModifyBGCtrl` subset: matches controllers by their `sctrlid` and updates
/// the run window (`time = start[, end[, loop]]`), `value`, and `x`/`y`.
/// Palette/shader parameters are not modeled. Stage controllers without an
/// `sctrlid` use id 0, matching Ikemen's default.
fn modify_bg_ctrl(sim: &mut SimState, who: usize, control: &Controller) {
    let id = int_param(sim, who, control, "id").unwrap_or(0);
    let times = int_param_list(sim, who, control, "time", 3);
    let values = int_param_list(sim, who, control, "value", 3);
    let x = num_param(sim, who, control, "x");
    let y = num_param(sim, who, control, "y");
    if times.is_empty() && values.is_empty() && x.is_none() && y.is_none() {
        return;
    }
    for ctrl in &mut sim.bg_ctrls {
        if ctrl.controller.get_i32("sctrlid").unwrap_or(0) != id {
            continue;
        }
        if let Some(Some(value)) = times.first() {
            ctrl.start = *value;
        }
        if let Some(Some(value)) = times.get(1) {
            ctrl.end = *value;
        }
        if let Some(Some(value)) = times.get(2) {
            ctrl.loop_period = *value;
        }
        if !values.is_empty() {
            let mut current: Vec<i32> = ctrl
                .controller
                .get("value")
                .map(|raw| {
                    raw.split(',')
                        .map(|part| part.trim().parse::<i32>().unwrap_or(0))
                        .collect()
                })
                .unwrap_or_default();
            if current.is_empty() {
                current.push(0);
            }
            // Preserve the original component count unless more are supplied.
            current.resize(current.len().max(values.len()), 0);
            for (index, value) in values.iter().enumerate() {
                if let Some(value) = value {
                    current[index] = *value;
                }
            }
            let text = current
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            controller_set_param(&mut ctrl.controller, "value", text);
        }
        if let Some(x) = x {
            controller_set_param(&mut ctrl.controller, "x", x.to_string());
        }
        if let Some(y) = y {
            controller_set_param(&mut ctrl.controller, "y", y.to_string());
        }
    }
}

/// `ModifyBGCtrl3D`: depth position/scale for stage layers. Ikemen targets
/// GLTF 3D nodes here; this engine has no 3D node graph, so `ctrlid` selects
/// BG layers by id and `value = z[, xscale[, yscale]]`. `time` is accepted but
/// not applied (the values persist until the next modification or round).
fn modify_bg_ctrl_3d(sim: &mut SimState, who: usize, control: &Controller) {
    let id = int_param(sim, who, control, "ctrlid")
        .or_else(|| int_param(sim, who, control, "id"))
        .unwrap_or(-1);
    let Some(raw) = control.get("value") else {
        return;
    };
    let values: Vec<f32> = raw
        .split(',')
        .map(|part| part.trim().parse::<f32>().unwrap_or(0.0))
        .collect();
    let z = values.first().copied().unwrap_or(0.0);
    let xscale = values.get(1).copied().unwrap_or(1.0);
    let yscale = values.get(2).copied().unwrap_or(xscale);
    let ids: Vec<i32> = sim
        .assets
        .stage
        .layers
        .iter()
        .map(|layer| layer.id)
        .collect();
    for (index, layer_id) in ids.iter().enumerate() {
        if id >= 0 && *layer_id != id {
            continue;
        }
        if let Some(state) = sim.stage_layers.get_mut(index) {
            state.z = Some(z);
            state.z_scale = Some(Vec2::new(xscale, yscale));
        }
    }
    // 3D model nodes use ctrlid as the glTF node index (-1 = all nodes).
    if id >= -1 {
        sim.model_nodes.insert(id, (z, xscale, yscale));
    }
}

/// Sets or inserts one controller parameter.
fn controller_set_param(control: &mut manato_formats::Controller, key: &str, value: String) {
    if let Some(entry) = control
        .params
        .iter_mut()
        .find(|(existing, _)| existing == key)
    {
        entry.1 = value;
    } else {
        control.params.push((key.to_string(), value));
    }
}

/// Reads up to `max` comma-separated integer expressions from a parameter,
/// preserving gaps for empty entries.
fn int_param_list(
    sim: &SimState,
    who: usize,
    control: &Controller,
    key: &str,
    max: usize,
) -> Vec<Option<i32>> {
    let Some(raw) = control.get(key) else {
        return Vec::new();
    };
    raw.split(',')
        .take(max)
        .map(|part| {
            let text = part.trim();
            if text.is_empty() {
                None
            } else {
                Some(trigger::num(sim, who, text).round() as i32)
            }
        })
        .collect()
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
    match int_param(sim, who, control, "id") {
        Some(id) if id >= 0 => {
            sim.effects
                .retain(|effect| effect.id != id || effect.owner != who);
        }
        // Omitted or -1 removes every explod owned by this fighter.
        _ => sim.effects.retain(|effect| effect.owner != who),
    }
}

/// Reads up to three literal components from a controller value.
fn triplet(control: &Controller, key: &str) -> (f32, f32, f32) {
    let Some(raw) = control.get(key) else {
        return (0.0, 0.0, 0.0);
    };
    let mut parts = raw.split(',');
    let x = parts
        .next()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0.0);
    let y = parts
        .next()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0.0);
    let z = parts
        .next()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0.0);
    (x, y, z)
}

fn projectile(sim: &mut SimState, who: usize, control: &Controller, assets: &crate::Assets) {
    let char = char_of(assets, sim, who);
    let anim = int_param(sim, who, control, "projanim").unwrap_or(0);
    if anim < 0 || char.animation(anim).is_none() {
        log::warn!("projectile uses missing animation {anim}");
        return;
    }
    let (px, py, pz) = {
        let (x, y, z) = triplet(control, "pos");
        let (ox, oy, oz) = triplet(control, "offset");
        let (x, y) = if control.get("pos").is_some() {
            (x, y)
        } else {
            (ox, oy)
        };
        (x, y, if control.get("pos").is_some() { z } else { oz })
    };
    let (vx, vy, vz) = triplet(control, "velocity");
    let (ax, ay, az) = triplet(control, "accel");
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
    let priority = int_param(sim, who, control, "projpriority").unwrap_or(1);
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
    let (rem_velocity, rem_velocity_z) = if control.get("remvelocity").is_some() {
        let (x, y, z) = triplet(control, "remvelocity");
        ((x, y), z)
    } else {
        let (x, y, z) = triplet(control, "projremvelocity");
        ((x, y), z)
    };
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
    let base_z = match postype.as_str() {
        "p2" => sim.fighters[sim.other(who)].z,
        _ => sim.fighters[who].z,
    };
    let z = base_z + pz;
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
        z,
        vel_z: vz,
        accel_z: az,
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
        rem_velocity_z,
        hits,
        hits_max: hits,
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
    let velocity = control
        .get("velocity")
        .map(|_| triplet(control, "velocity"));
    let accel = control.get("accel").map(|_| triplet(control, "accel"));
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
    let rem_velocity = if control.get("remvelocity").is_some() {
        Some(triplet(control, "remvelocity"))
    } else {
        control
            .get("projremvelocity")
            .map(|_| triplet(control, "projremvelocity"))
    };
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
        if let Some((x, y, z)) = velocity {
            projectile.vel = Vec2::new(x * facing as f32, y);
            projectile.vel_z = z;
            projectile.facing = facing;
        }
        if let Some((x, y, z)) = accel {
            projectile.accel = Vec2::new(x * facing as f32, y);
            projectile.accel_z = z;
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
        if let Some((x, y, z)) = rem_velocity {
            projectile.rem_velocity = Vec2::new(x, y);
            projectile.rem_velocity_z = z;
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
    let (px, py, pz) = triplet(control, "pos");
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

    let base_z = match pos_type.as_str() {
        "p2" => sim.fighters[sim.other(who)].z,
        _ => sim.fighters[who].z,
    };
    let z = base_z + pz;
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
        z,
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
    // Ikemen-GO and M.U.G.E.N default an omitted SuperPause time to 30 ticks.
    let time = int_param(sim, who, control, "time").unwrap_or(30).max(0);
    let movetime = int_param(sim, who, control, "movetime").unwrap_or(0);
    sim.super_pause = time;
    sim.super_pause_move_time = movetime;
    // Like `Pause`, the superpausing fighter may act for `movetime` ticks.
    sim.pause_mover = Some(who);
    sim.pause_movetime = if movetime > time { 0 } else { movetime.max(0) };
    if let Some(value) = int_param(sim, who, control, "poweradd") {
        sim.fighters[who].power = clamp(sim.fighters[who].power + value as f32, 0.0, MAX_POWER);
    }
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
