//! Trigger expression evaluation.
//!
//! Triggers are evaluated against a [`SimState`] and a fighter index. Redirects
//! (`p2, life`, `enemy, stateno`, ...) switch the target fighter for the inner
//! expression.

use manato_core::{Dir8, Vec2, GAME_HEIGHT, GAME_WIDTH};
use manato_formats::{CommandDef, Controller};

use crate::expr::{BinOp, Expr, UnOp, Value};
use crate::hitdef::HitAttr;
use crate::SimState;

/// Parses and evaluates a raw trigger/parameter string. Parse failures fall
/// back to treating the value as a literal string, which keeps unknown
/// parameters harmless.
pub fn eval_str(sim: &SimState, who: usize, source: &str) -> Value {
    let text = source.trim();
    if text.is_empty() {
        return Value::Int(0);
    }
    match crate::expr::parse(text) {
        Ok(expr) => eval(sim, who, &expr),
        Err(_) => Value::Str(text.to_string()),
    }
}

pub fn num(sim: &SimState, who: usize, source: &str) -> f32 {
    eval_str(sim, who, source).as_f32()
}

pub fn int(sim: &SimState, who: usize, source: &str) -> i32 {
    eval_str(sim, who, source).as_i32()
}

pub fn flag(sim: &SimState, who: usize, source: &str) -> bool {
    eval_str(sim, who, source).truthy()
}

/// True when every `triggerall` passes and any numbered trigger group passes.
pub fn triggers_met(sim: &SimState, who: usize, control: &Controller) -> bool {
    let (all, groups) = control.trigger_groups();
    for trigger in all {
        if !flag(sim, who, trigger) {
            return false;
        }
    }
    if groups.is_empty() {
        return true;
    }
    groups
        .iter()
        .any(|(_, list)| list.iter().all(|trigger| flag(sim, who, trigger)))
}

pub fn eval(sim: &SimState, who: usize, expr: &Expr) -> Value {
    match expr {
        Expr::Int(v) => Value::Int(*v),
        Expr::Float(v) => Value::Float(*v),
        Expr::Str(s) => Value::Str(s.clone()),
        Expr::Ident(name) => {
            let lower = name.to_ascii_lowercase();
            if is_trigger_name(&lower) {
                eval_trigger(sim, who, &lower, &[])
            } else {
                Value::Str(name.clone())
            }
        }
        Expr::Unary(op, inner) => {
            let v = eval(sim, who, inner);
            match op {
                UnOp::Not => Value::Bool(!v.truthy()),
                UnOp::Neg => Value::Float(-v.as_f32()),
                UnOp::BitNot => Value::Int(!v.as_i32()),
            }
        }
        Expr::Binary(op, left, right) => eval_binary(sim, who, *op, left, right),
        Expr::Call(name, args) => eval_call(sim, who, name, args),
        Expr::Redirect(name, inner) => {
            let target = redirect_target(sim, who, name);
            eval(sim, target, inner)
        }
        Expr::Range { .. } => Value::Int(0),
        Expr::List(items) => Value::Str(
            items
                .iter()
                .map(|e| eval(sim, who, e).as_str())
                .collect::<Vec<_>>()
                .join(","),
        ),
    }
}

fn redirect_target(sim: &SimState, who: usize, name: &str) -> usize {
    let lower = name.to_ascii_lowercase();
    if lower == "p1" {
        return 0;
    }
    if lower == "p2" {
        return 1;
    }
    if let Some(rest) = lower.strip_prefix("playerid(") {
        if let Some(value) = rest
            .strip_suffix(')')
            .and_then(|value| value.parse::<usize>().ok())
        {
            return value.min(1);
        }
    }
    if lower == "enemy" || lower == "enemynear" || lower.starts_with("enemy(") {
        return sim.other(who);
    }
    if let Some(rest) = lower.strip_prefix("playerindex(") {
        if let Some(value) = rest
            .strip_suffix(')')
            .and_then(|value| value.parse::<usize>().ok())
        {
            return value.min(1);
        }
    }
    if lower == "playerindex" {
        return who;
    }
    if let Some(rest) = lower.strip_prefix("helperindex(") {
        let index = rest
            .strip_suffix(')')
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(usize::MAX);
        let root = sim.root_of(who);
        let helpers: Vec<usize> = sim
            .fighters
            .iter()
            .enumerate()
            .filter(|(_, fighter)| fighter.owner == Some(root))
            .map(|(index, _)| index)
            .collect();
        return helpers.get(index).copied().unwrap_or(who);
    }
    if lower == "stateowner" {
        return sim.root_of(who);
    }
    if lower == "self" {
        return who;
    }
    if lower == "root" {
        return sim.root_of(who);
    }
    if lower == "parent" {
        return sim.parent_of(who);
    }
    if lower == "target" || lower.starts_with("target(") || lower.starts_with("target,") {
        return sim.fighters[who].target.unwrap_or(who);
    }
    if lower.starts_with("helper") {
        let id = lower
            .split_once('(')
            .and_then(|(_, rest)| rest.split_once(')'))
            .and_then(|(value, _)| value.parse::<i32>().ok())
            .unwrap_or(0);
        let owner = sim.root_of(who);
        return sim.helper_by_id(owner, id).unwrap_or(who);
    }
    // enemy / enemynear / playerid(n)
    if lower == "playerid" {
        return who;
    }
    sim.other(who)
}

fn eval_binary(sim: &SimState, who: usize, op: BinOp, left: &Expr, right: &Expr) -> Value {
    // `command = "name"` is a special string comparison.
    if let Expr::Ident(name) = left {
        let lower = name.to_ascii_lowercase();
        if lower == "command" {
            let wanted = match right {
                Expr::Str(s) => s.clone(),
                _ => eval(sim, who, right).as_str(),
            };
            let active = command_active(sim, who, &wanted);
            return Value::Bool(match op {
                BinOp::Eq => active,
                BinOp::Ne => !active,
                _ => false,
            });
        }
        if lower == "hitdefattr" {
            if let Expr::List(items) = right {
                let hitdef = sim.fighters[who].hitdef.clone().unwrap_or_default();
                let ok = hitdefattr_matches(&hitdef.attr, sim, who, items);
                return Value::Bool(if op == BinOp::Eq { ok } else { !ok });
            }
        }
        if lower == "hitbyattr" {
            let wanted = match right {
                Expr::List(items) => items
                    .iter()
                    .map(|item| eval(sim, who, item).as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                other => eval(sim, who, other).as_str(),
            };
            let ok = hitbyattr_value(sim, who, &wanted);
            return Value::Bool(if op == BinOp::Eq { ok } else { !ok });
        }
    }

    // `x = [a,b)` ranges.
    if let Expr::Range {
        lo,
        hi,
        lo_incl,
        hi_incl,
    } = right
    {
        let v = eval(sim, who, left).as_f32();
        let lo = eval(sim, who, lo).as_f32();
        let hi = eval(sim, who, hi).as_f32();
        let inside = (v > lo || (*lo_incl && v == lo)) && (v < hi || (*hi_incl && v == hi));
        return Value::Bool(match op {
            BinOp::Eq => inside,
            BinOp::Ne => !inside,
            _ => false,
        });
    }

    let a = eval(sim, who, left);
    let b = eval(sim, who, right);
    binop(op, &a, &b)
}

fn hitdefattr_matches(attr: &HitAttr, sim: &SimState, who: usize, items: &[Expr]) -> bool {
    if attr.state_types.is_empty() {
        return false;
    }
    let first = items
        .first()
        .map(|e| eval(sim, who, e).as_str().to_ascii_uppercase())
        .unwrap_or_default();
    if !first.contains(&attr.state_types) {
        return false;
    }
    if items.len() <= 1 {
        return true;
    }
    items[1..].iter().any(|item| {
        let wanted = eval(sim, who, item).as_str().to_ascii_uppercase();
        !wanted.is_empty() && attr.class.contains(&wanted)
    })
}

fn binop(op: BinOp, a: &Value, b: &Value) -> Value {
    match op {
        BinOp::Add => Value::Float(a.as_f32() + b.as_f32()),
        BinOp::Sub => Value::Float(a.as_f32() - b.as_f32()),
        BinOp::Mul => Value::Float(a.as_f32() * b.as_f32()),
        BinOp::Div => {
            let d = b.as_f32();
            if d == 0.0 {
                Value::Float(0.0)
            } else {
                Value::Float(a.as_f32() / d)
            }
        }
        BinOp::Mod => {
            let d = b.as_i32();
            if d == 0 {
                Value::Int(0)
            } else {
                Value::Int(a.as_i32() % d)
            }
        }
        BinOp::Pow => Value::Float(a.as_f32().powf(b.as_f32())),
        BinOp::And => Value::Bool(a.truthy() && b.truthy()),
        BinOp::Or => Value::Bool(a.truthy() || b.truthy()),
        BinOp::BitAnd => Value::Int(a.as_i32() & b.as_i32()),
        BinOp::BitOr => Value::Int(a.as_i32() | b.as_i32()),
        BinOp::BitXor => Value::Int(a.as_i32() ^ b.as_i32()),
        BinOp::Eq | BinOp::Ne => {
            let equal = values_equal(a, b);
            Value::Bool(if op == BinOp::Eq { equal } else { !equal })
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let (Some(x), Some(y)) = (numeric(a), numeric(b)) else {
                return Value::Bool(false);
            };
            let result = match op {
                BinOp::Lt => x < y,
                BinOp::Le => x <= y,
                BinOp::Gt => x > y,
                BinOp::Ge => x >= y,
                _ => unreachable!(),
            };
            Value::Bool(result)
        }
    }
}

fn numeric(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f as f64),
        Value::Bool(b) => Some(*b as i32 as f64),
        Value::Str(s) => s.trim().parse().ok(),
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    if let (Some(x), Some(y)) = (numeric(a), numeric(b)) {
        return (x - y).abs() < 0.0001;
    }
    a.as_str().eq_ignore_ascii_case(&b.as_str())
}

fn eval_call(sim: &SimState, who: usize, name: &str, args: &[Expr]) -> Value {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "const" => {
            let key = args
                .first()
                .map(|a| match a {
                    Expr::Ident(s) => s.clone(),
                    other => eval(sim, who, other).as_str(),
                })
                .unwrap_or_default();
            let value = sim.character(who).constants.get(&key).unwrap_or(0.0);
            Value::Float(value)
        }
        "ifelse" => {
            if args.len() >= 3 && eval(sim, who, &args[0]).truthy() {
                eval(sim, who, &args[1])
            } else if args.len() >= 3 {
                eval(sim, who, &args[2])
            } else {
                Value::Int(0)
            }
        }
        "round" => {
            let value = arg_f32(sim, who, args, 0);
            let digits = arg_i32(sim, who, args, 1).clamp(-9, 9);
            let shift = 10f32.powi(digits);
            Value::Float(((value * shift) + 0.5).floor() / shift)
        }
        "randomrange" => {
            let a = arg_i32(sim, who, args, 0);
            let b = arg_i32(sim, who, args, 1);
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let span = (hi as i64 - lo as i64 + 1).max(1) as u64;
            let x = sim
                .frame
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407)
                .wrapping_add(who as u64 * 7919);
            Value::Int((lo as i64 + (x >> 33) as i64 % span as i64) as i32)
        }
        "abs" => Value::Float(arg_f32(sim, who, args, 0).abs()),
        "ceil" => Value::Float(arg_f32(sim, who, args, 0).ceil()),
        "floor" => Value::Float(arg_f32(sim, who, args, 0).floor()),
        "sqrt" => Value::Float(arg_f32(sim, who, args, 0).max(0.0).sqrt()),
        // M.U.G.E.N trig functions take and return radians (`sin(pi/2)`).
        "sin" => Value::Float(arg_f32(sim, who, args, 0).sin()),
        "cos" => Value::Float(arg_f32(sim, who, args, 0).cos()),
        "tan" => Value::Float(arg_f32(sim, who, args, 0).tan()),
        "asin" => Value::Float(arg_f32(sim, who, args, 0).clamp(-1.0, 1.0).asin()),
        "acos" => Value::Float(arg_f32(sim, who, args, 0).clamp(-1.0, 1.0).acos()),
        "atan" => Value::Float(arg_f32(sim, who, args, 0).atan()),
        "atan2" => Value::Float(arg_f32(sim, who, args, 0).atan2(arg_f32(sim, who, args, 1))),
        "clamp" => Value::Float(
            arg_f32(sim, who, args, 0)
                .max(arg_f32(sim, who, args, 1))
                .min(arg_f32(sim, who, args, 2)),
        ),
        "lerp" => {
            let from = arg_f32(sim, who, args, 0);
            let to = arg_f32(sim, who, args, 1);
            let amount = arg_f32(sim, who, args, 2).clamp(0.0, 1.0);
            Value::Float(from + (to - from) * amount)
        }
        "sign" => {
            let value = arg_f32(sim, who, args, 0);
            Value::Int(if value < 0.0 {
                -1
            } else if value > 0.0 {
                1
            } else {
                0
            })
        }
        "rad" => Value::Float(arg_f32(sim, who, args, 0).to_radians()),
        "deg" => Value::Float(arg_f32(sim, who, args, 0).to_degrees()),
        "exp" => Value::Float(arg_f32(sim, who, args, 0).exp()),
        "log" | "ln" => Value::Float(arg_f32(sim, who, args, 0).max(0.0001).ln()),
        "min" => Value::Float(arg_f32(sim, who, args, 0).min(arg_f32(sim, who, args, 1))),
        "max" => Value::Float(arg_f32(sim, who, args, 0).max(arg_f32(sim, who, args, 1))),
        "pow" => Value::Float(arg_f32(sim, who, args, 0).powf(arg_f32(sim, who, args, 1))),
        "var" => {
            let idx = arg_i32(sim, who, args, 0).clamp(0, 59) as usize;
            Value::Int(sim.fighters[who].vars[idx])
        }
        "fvar" => {
            let idx = arg_i32(sim, who, args, 0).clamp(0, 39) as usize;
            Value::Float(sim.fighters[who].fvars[idx])
        }
        "sysvar" => {
            let idx = arg_i32(sim, who, args, 0).clamp(0, 4) as usize;
            Value::Int(sim.fighters[who].sysvars[idx])
        }
        "sysfvar" => {
            let idx = arg_i32(sim, who, args, 0).clamp(0, 4) as usize;
            Value::Float(sim.fighters[who].sysfvars[idx])
        }
        "animelemtime" => {
            let elem = arg_i32(sim, who, args, 0);
            Value::Int(sim.fighters[who].anim_elem_time(sim.anim_character(who), elem))
        }
        "animexist" | "selfanimexist" => {
            let n = arg_i32(sim, who, args, 0);
            Value::Bool(sim.character(who).animation(n).is_some())
        }
        "isasserted" => {
            let name = args
                .first()
                .map(|a| match a {
                    Expr::Ident(s) => s.clone(),
                    other => eval(sim, who, other).as_str(),
                })
                .unwrap_or_default();
            let flag = crate::fighter::assert_flag(&name.trim().to_ascii_lowercase());
            Value::Bool(flag != 0 && sim.fighters[who].assert_flags & flag != 0)
        }
        "numstagebg" => {
            let id = arg_i32(sim, who, args, 0);
            Value::Int(
                sim.assets
                    .stage
                    .layers
                    .iter()
                    .filter(|layer| id < 0 || layer.id == id)
                    .count() as i32,
            )
        }
        "animelemno" => {
            let time = arg_i32(sim, who, args, 0);
            let character = sim.anim_character(who);
            let fighter = &sim.fighters[who];
            let Some(anim) = fighter.current_anim(character) else {
                return Value::Int(0);
            };
            Value::Int(anim_elem_no(anim, fighter.anim_time, time))
        }
        "mugenversion" => {
            let part = args
                .first()
                .map(|a| match a {
                    Expr::Ident(s) => s.clone(),
                    other => eval(sim, who, other).as_str(),
                })
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            let (major, minor) = mugen_version_parts(&sim.character(who).def.mugen_version);
            Value::Int(if part == "minor" { minor } else { major })
        }
        "ikemenversion" => {
            let part = args
                .first()
                .map(|a| match a {
                    Expr::Ident(s) => s.clone(),
                    other => eval(sim, who, other).as_str(),
                })
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            let text = sim
                .character(who)
                .def
                .info
                .get("ikemenversion")
                .cloned()
                .unwrap_or_default();
            let parts = version_parts(&text);
            Value::Int(match part.as_str() {
                "major" => parts.0,
                "minor" => parts.1,
                "patch" => parts.2,
                _ => 0,
            })
        }
        "helperindexexist" => {
            let index = arg_i32(sim, who, args, 0);
            if index <= 0 {
                return Value::Bool(true);
            }
            let root = sim.root_of(who);
            let count = sim
                .fighters
                .iter()
                .enumerate()
                .filter(|(_, fighter)| fighter.owner.is_some() && fighter.active)
                .filter(|(index, _)| sim.root_of(*index) == root)
                .count() as i32;
            Value::Bool(count >= index)
        }
        "explodvar" => {
            let id = arg_i32(sim, who, args, 0);
            let index = arg_i32(sim, who, args, 1).max(0) as usize;
            let field = args
                .get(2)
                .map(|a| match a {
                    Expr::Ident(s) => s.clone(),
                    other => eval(sim, who, other).as_str(),
                })
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if let Some(effect) = sim
                .effects
                .iter()
                .filter(|effect| {
                    effect.owner == who && !effect.is_expired() && (id < 0 || effect.id == id)
                })
                .nth(index)
            {
                explod_var(sim, effect, &field)
            } else {
                Value::Int(0)
            }
        }
        "helpervar" => {
            let field = args
                .first()
                .map(|a| match a {
                    Expr::Ident(s) => s.clone(),
                    other => eval(sim, who, other).as_str(),
                })
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if sim.fighters[who].owner.is_none() {
                Value::Int(0)
            } else {
                let fighter = &sim.fighters[who];
                match field.as_str() {
                    "id" => Value::Int(fighter.helper_id),
                    "helpertype" => Value::Int(if fighter.helper_type_player { 2 } else { 1 }),
                    "keyctrl" => Value::Bool(fighter.keyctrl & 1 != 0),
                    _ => Value::Int(0),
                }
            }
        }
        "projvar" => {
            let id = arg_i32(sim, who, args, 0);
            let index = arg_i32(sim, who, args, 1).max(0) as usize;
            let field = args
                .get(2)
                .map(|a| match a {
                    Expr::Ident(s) => s.clone(),
                    other => eval(sim, who, other).as_str(),
                })
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if let Some(projectile) = sim
                .projectiles
                .iter()
                .filter(|projectile| {
                    projectile.owner == who
                        && projectile.status == crate::projectile::ProjectileStatus::Active
                        && (id < 0 || projectile.id == id)
                })
                .nth(index)
            {
                projectile_var(sim, projectile, &field)
            } else {
                Value::Int(0)
            }
        }
        "stagebgvar" => {
            let id = arg_i32(sim, who, args, 0);
            let index = arg_i32(sim, who, args, 1).max(0) as usize;
            let field = args
                .get(2)
                .map(|a| match a {
                    Expr::Ident(s) => s.clone(),
                    other => eval(sim, who, other).as_str(),
                })
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            stage_bg_var(sim, id, index, &field)
        }
        "hitdefvar" => {
            let key = args
                .first()
                .map(|a| match a {
                    Expr::Ident(s) => s.clone(),
                    other => eval(sim, who, other).as_str(),
                })
                .unwrap_or_default();
            let key = key.trim().to_ascii_lowercase();
            let Some(hitdef) = sim.fighters[who].hitdef.as_ref() else {
                return Value::Int(0);
            };
            match key.as_str() {
                "hitdamage" => Value::Int(hitdef.damage),
                "guarddamage" => Value::Int(hitdef.guard_damage),
                "hitflag" => Value::Str(hitdef.hitflag.clone()),
                "guardflag" => Value::Str(hitdef.guardflag.clone()),
                "sparkno" => Value::Int(hitdef.sparkno),
                "guard.sparkno" => Value::Int(hitdef.guard_sparkno),
                "sparkx" => Value::Float(hitdef.sparkxy.x),
                "sparky" => Value::Float(hitdef.sparkxy.y),
                "hitsound.group" => Value::Int(hitdef.hitsound.0),
                "hitsound.number" => Value::Int(hitdef.hitsound.1),
                "guardsound.group" => Value::Int(hitdef.guardsound.0),
                "guardsound.number" => Value::Int(hitdef.guardsound.1),
                "id" => Value::Int(hitdef.id),
                "p1stateno" => Value::Int(hitdef.p1stateno),
                "p2stateno" => Value::Int(hitdef.p2stateno),
                "priority" => Value::Int(hitdef.priority),
                "pausetime" => Value::Int(hitdef.pausetime.0),
                "shaketime" => Value::Int(hitdef.pausetime.1),
                "guard.pausetime" => Value::Int(hitdef.guard_pausetime.0),
                "guard.shaketime" => Value::Int(hitdef.guard_pausetime.1),
                "yaccel" => Value::Float(hitdef.yaccel),
                "ground.velocity.x" => Value::Float(hitdef.ground_velocity.x),
                "ground.velocity.y" => Value::Float(hitdef.ground_velocity.y),
                "air.velocity.x" => Value::Float(hitdef.air_velocity.x),
                "air.velocity.y" => Value::Float(hitdef.air_velocity.y),
                "down.velocity.x" => Value::Float(hitdef.down_velocity.x),
                "down.velocity.y" => Value::Float(hitdef.down_velocity.y),
                _ => Value::Int(0),
            }
        }
        "map" => {
            let key = args
                .first()
                .map(|a| match a {
                    Expr::Ident(s) => s.clone(),
                    other => eval(sim, who, other).as_str(),
                })
                .unwrap_or_default();
            let key = key.trim().trim_matches('"').trim().to_ascii_lowercase();
            sim.fighters[who]
                .map
                .get(&key)
                .cloned()
                .unwrap_or(crate::expr::Value::Int(0))
        }
        "playeridexist" => {
            let index = arg_i32(sim, who, args, 0);
            Value::Bool(
                index >= 0
                    && sim
                        .fighters
                        .get(index as usize)
                        .is_some_and(|fighter| fighter.active),
            )
        }
        "playerindexexist" => {
            let index = arg_i32(sim, who, args, 0);
            Value::Bool(
                index >= 0
                    && sim
                        .fighters
                        .get(index as usize)
                        .is_some_and(|fighter| fighter.active),
            )
        }
        "playernoexist" => {
            let number = arg_i32(sim, who, args, 0);
            let exists = if (1..=2).contains(&number) {
                sim.fighters
                    .get((number - 1) as usize)
                    .is_some_and(|fighter| fighter.owner.is_none() && fighter.active)
            } else {
                false
            };
            Value::Bool(exists)
        }
        "selfstatenoexist" => {
            let number = arg_i32(sim, who, args, 0);
            Value::Bool(
                sim.assets.characters[sim.fighters[who].char_id]
                    .state(number)
                    .is_some(),
            )
        }
        "gethitvar" => {
            let key = args
                .first()
                .map(|a| match a {
                    Expr::Ident(s) => s.clone(),
                    other => eval(sim, who, other).as_str(),
                })
                .unwrap_or_default();
            let lower = key.trim().to_ascii_lowercase();
            if lower == "hitshaketime" {
                Value::Int(sim.fighters[who].hit.hit_shake_timer)
            } else if lower == "guardpoints" {
                Value::Int(sim.fighters[who].guard_points)
            } else if lower == "dizzypoints" {
                Value::Int(sim.fighters[who].dizzy_points)
            } else if lower == "redlife" {
                Value::Int(sim.fighters[who].red_life)
            } else {
                sim.fighters[who].hit.vars.get(&lower)
            }
        }
        "numexplod" => Value::Int(
            sim.effects
                .iter()
                .filter(|e| {
                    e.owner == who
                        && matches!(e.source, crate::EffectSource::Character(_))
                        && (args.is_empty() || e.id == arg_i32(sim, who, args, 0))
                })
                .count() as i32,
        ),
        "hitvel" => {
            let axis = axis_arg(sim, who, args);
            let hit = &sim.fighters[who].hit.vars;
            if axis.starts_with('x') {
                Value::Float(hit.x_vel)
            } else {
                Value::Float(hit.y_vel)
            }
        }
        "hitbyattr" => {
            let wanted = args
                .first()
                .map(|arg| eval(sim, who, arg).as_str().to_ascii_uppercase())
                .unwrap_or_default();
            Value::Bool(hitbyattr_value(sim, who, &wanted))
        }
        "hitdefattr" => {
            let wanted = args
                .first()
                .map(|arg| eval(sim, who, arg).as_str().to_ascii_uppercase())
                .unwrap_or_default();
            let matched = sim.fighters[who]
                .hitdef
                .as_ref()
                .map(|hitdef| {
                    let state = hitdef.attr.state_types.to_ascii_uppercase();
                    let class = hitdef.attr.class.to_ascii_uppercase();
                    wanted
                        .chars()
                        .filter(|ch| ch.is_ascii_alphabetic())
                        .all(|ch| state.contains(ch) || class.contains(ch))
                })
                .unwrap_or(false);
            Value::Bool(matched)
        }
        "const240p" | "const480p" | "const720p" | "const1080p" => {
            let key = args
                .first()
                .map(|arg| match arg {
                    Expr::Ident(name) => name.clone(),
                    other => eval(sim, who, other).as_str(),
                })
                .unwrap_or_default();
            let scale = if lower == "const240p" {
                1.0
            } else if lower == "const480p" {
                2.0
            } else if lower == "const720p" {
                3.0
            } else {
                4.5
            };
            Value::Float(sim.character(who).constants.get(&key).unwrap_or(0.0) * scale)
        }
        "p3name" | "p4name" => Value::Str(String::new()),
        "numproj" => Value::Int(
            sim.projectiles
                .iter()
                .filter(|projectile| {
                    projectile.owner == who
                        && projectile.status == crate::projectile::ProjectileStatus::Active
                })
                .count() as i32,
        ),
        "numprojid" => {
            let id = arg_i32(sim, who, args, 0);
            Value::Int(
                sim.projectiles
                    .iter()
                    .filter(|projectile| {
                        projectile.owner == who
                            && projectile.id == id
                            && projectile.status == crate::projectile::ProjectileStatus::Active
                    })
                    .count() as i32,
            )
        }
        "numhelper" => {
            let wanted = if args.is_empty() {
                None
            } else {
                Some(arg_i32(sim, who, args, 0))
            };
            let root = sim.root_of(who);
            Value::Int(
                sim.fighters
                    .iter()
                    .filter(|fighter| {
                        fighter.owner == Some(root)
                            && wanted.is_none_or(|id| fighter.helper_id == id)
                    })
                    .count() as i32,
            )
        }
        "leftedge" => Value::Float(sim.camera.x - GAME_WIDTH * 0.5),
        "rightedge" => Value::Float(sim.camera.x + GAME_WIDTH * 0.5),
        "topedge" => Value::Float(sim.camera.y - sim.ground_screen_y()),
        "bottomedge" => Value::Float(sim.camera.y - sim.ground_screen_y() + GAME_HEIGHT),
        "backedge" => {
            let left = sim.camera.x - GAME_WIDTH * 0.5;
            let right = sim.camera.x + GAME_WIDTH * 0.5;
            Value::Float(if sim.fighters[who].facing < 0 {
                right
            } else {
                left
            })
        }
        "frontedge" => {
            let left = sim.camera.x - GAME_WIDTH * 0.5;
            let right = sim.camera.x + GAME_WIDTH * 0.5;
            Value::Float(if sim.fighters[who].facing < 0 {
                left
            } else {
                right
            })
        }
        "projcontacttime" => Value::Int(sim.fighters[who].proj_contact_time),
        "projhittime" => Value::Int(sim.fighters[who].proj_hit_time),
        "projguardedtime" => Value::Int(sim.fighters[who].proj_guarded_time),
        "projcontact" => Value::Bool(sim.fighters[who].proj_contact_time >= 0),
        "projhit" => Value::Bool(sim.fighters[who].proj_hit_time >= 0),
        "projguarded" => Value::Bool(sim.fighters[who].proj_guarded_time >= 0),
        "projmiss" | "projmissetime" => Value::Int(0),
        "numtarget" => Value::Int(sim.fighters[who].target.is_some() as i32),
        "random" => {
            let x = sim
                .frame
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407)
                .wrapping_add(who as u64 * 7919);
            Value::Int(((x >> 33) % 1000) as i32)
        }
        "p2bodydist" => {
            let axis = axis_arg(sim, who, args);
            let other = sim.other(who);
            let a = &sim.fighters[who];
            let b = &sim.fighters[other];
            if axis.starts_with('x') {
                let dx = (b.pos.x - a.pos.x).abs();
                Value::Float((dx - a.width_front - b.width_back).max(0.0))
            } else {
                let dy = (b.pos.y - a.pos.y).abs();
                Value::Float((dy - a.height).max(0.0))
            }
        }
        "p2dist" => {
            let axis = axis_arg(sim, who, args);
            let other = sim.other(who);
            let a = sim.fighters[who].pos;
            let b = sim.fighters[other].pos;
            let facing = sim.fighters[who].facing as f32;
            if axis.starts_with('x') {
                Value::Float((b.x - a.x) * facing)
            } else {
                Value::Float(b.y - a.y)
            }
        }
        "parentdist" => {
            let axis = axis_arg(sim, who, args);
            let parent = sim.parent_of(who);
            Value::Float(relation_axis_distance(sim, who, parent, &axis))
        }
        "rootdist" => {
            let axis = axis_arg(sim, who, args);
            let root = sim.root_of(who);
            Value::Float(relation_axis_distance(sim, who, root, &axis))
        }
        "p2screendist" => {
            let axis = axis_arg(sim, who, args);
            let other = sim.other(who);
            let a = sim.screen_pos(sim.fighters[who].pos);
            let b = sim.screen_pos(sim.fighters[other].pos);
            if axis.starts_with('x') {
                Value::Float(b.x - a.x)
            } else {
                Value::Float(b.y - a.y)
            }
        }
        "vel" => {
            let axis = axis_arg(sim, who, args);
            let f = &sim.fighters[who];
            if axis.starts_with('x') {
                Value::Float(f.vel.x * f.facing as f32)
            } else {
                Value::Float(f.vel.y)
            }
        }
        "pos" => {
            let axis = axis_arg(sim, who, args);
            let f = &sim.fighters[who];
            if axis.starts_with('x') {
                Value::Float(f.pos.x)
            } else if axis.starts_with('z') {
                Value::Float(0.0)
            } else {
                Value::Float(f.pos.y)
            }
        }
        "screenpos" => {
            let axis = axis_arg(sim, who, args);
            let f = &sim.fighters[who];
            let p = sim.screen_pos(f.pos);
            if axis.starts_with('x') {
                Value::Float(p.x)
            } else {
                Value::Float(p.y)
            }
        }
        "screenvel" => {
            let axis = axis_arg(sim, who, args);
            let f = &sim.fighters[who];
            if axis.starts_with('x') {
                Value::Float(f.vel.x)
            } else {
                Value::Float(f.vel.y)
            }
        }
        "ishelper" => {
            let helper = sim.is_helper(who);
            if args.is_empty() {
                Value::Int(helper as i32)
            } else {
                let id = arg_i32(sim, who, args, 0);
                Value::Int((helper && sim.fighters[who].helper_id == id) as i32)
            }
        }
        "playerid" => Value::Int(sim.fighters[who].owner.unwrap_or(who) as i32),
        "helperindex" => Value::Int(who.saturating_sub(2) as i32),
        "hitpausetime" => Value::Int(sim.fighters[who].hit.hit_pause_time),
        "movereversed" => Value::Bool(sim.fighters[who].attack.move_reversed),
        "camerazoom" => Value::Float(sim.camera.scale),
        "camerapos" => {
            if axis_arg(sim, who, args).starts_with('y') {
                Value::Float(sim.camera.y)
            } else {
                Value::Float(sim.camera.x)
            }
        }
        "zoomvar" => {
            let key = axis_arg(sim, who, args);
            match sim.camera.zoom_override {
                Some(zoom) => match key.as_str() {
                    "scale" => Value::Float(zoom.target_scale),
                    "pos.x" | "x" => Value::Float(zoom.target_pos.x),
                    "pos.y" | "y" => Value::Float(zoom.target_pos.y),
                    "lag" => Value::Int(zoom.lag),
                    "endlag" => Value::Int(zoom.end_lag),
                    "time" => Value::Int(zoom.time),
                    _ => Value::Float(zoom.target_scale),
                },
                None => match key.as_str() {
                    "pos.x" | "x" => Value::Float(sim.camera.x),
                    "pos.y" | "y" => Value::Float(sim.camera.y),
                    "lag" | "endlag" | "time" => Value::Int(0),
                    _ => Value::Float(sim.camera.scale),
                },
            }
        }
        _ => Value::Int(0),
    }
}

fn arg_f32(sim: &SimState, who: usize, args: &[Expr], index: usize) -> f32 {
    args.get(index)
        .map(|e| eval(sim, who, e).as_f32())
        .unwrap_or(0.0)
}

fn arg_i32(sim: &SimState, who: usize, args: &[Expr], index: usize) -> i32 {
    args.get(index)
        .map(|e| eval(sim, who, e).as_i32())
        .unwrap_or(0)
}

fn axis_arg(sim: &SimState, who: usize, args: &[Expr]) -> String {
    args.first()
        .map(|e| eval(sim, who, e).as_str().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Axis-relative distance from `who` to `other`, as used by `ParentDist` and
/// `RootDist`. A missing or unknown component (including no relation) is 0.
fn relation_axis_distance(sim: &SimState, who: usize, other: usize, axis: &str) -> f32 {
    if other == who
        || axis.is_empty()
        || !sim.fighters[other].active
        || !sim.fighters[other].alive()
    {
        return 0.0;
    }
    let a = sim.fighters[who].pos;
    let b = sim.fighters[other].pos;
    if axis.starts_with('x') {
        (b.x - a.x) * sim.fighters[who].facing as f32
    } else {
        b.y - a.y
    }
}

fn is_trigger_name(name: &str) -> bool {
    matches!(
        name,
        "ailevel"
            | "powermax"
            | "guardcount"
            | "attack"
            | "attackmul"
            | "defence"
            | "defencemul"
            | "angle"
            | "alive"
            | "anim"
            | "animelem"
            | "animtime"
            | "animlength"
            | "animloopcount"
            | "prevanim"
            | "sprpriority"
            | "author"
            | "backedge"
            | "backedgebodydist"
            | "backedgedist"
            | "bottomedge"
            | "frontedge"
            | "leftedge"
            | "rightedge"
            | "topedge"
            | "camerapos"
            | "camerazoom"
            | "zoomvar"
            | "drawgame"
            | "win"
            | "consecutivewins"
            | "winko"
            | "wintime"
            | "winperfect"
            | "winclutch"
            | "decisiveround"
            | "combocount"
            | "dizzy"
            | "dizzypoints"
            | "dizzypointsmax"
            | "gameheight"
            | "gametime"
            | "gamewidth"
            | "guardbreak"
            | "guardpoints"
            | "guardpointsmax"
            | "redlife"
            | "receivedhits"
            | "receiveddamage"
            | "ishometeam"
            | "lose"
            | "loseko"
            | "losetime"
            | "matchno"
            | "name"
            | "p1name"
            | "p2name"
            | "p3name"
            | "p4name"
            | "palno"
            | "parentdist"
            | "parentexist"
            | "rootdist"
            | "roundswon"
            | "screenheight"
            | "screenwidth"
            | "teammode"
            | "tickspersecond"
            | "timemod"
            | "canrecover"
            | "command"
            | "ctrl"
            | "drawpal"
            | "facing"
            | "frontedgebodydist"
            | "frontedgedist"
            | "hitcount"
            | "hitdefattr"
            | "hitbyattr"
            | "helperindex"
            | "hitfall"
            | "hitvel"
            | "hitover"
            | "hitpausetime"
            | "hitoverridden"
            | "index"
            | "hitshakeover"
            | "inguarddist"
            | "ishelper"
            | "life"
            | "lifemax"
            | "matchover"
            | "movecontact"
            | "moveguarded"
            | "movehit"
            | "movereversed"
            | "movetype"
            | "numenemy"
            | "numexplod"
            | "numhelper"
            | "numpartner"
            | "numproj"
            | "numprojid"
            | "projcontact"
            | "projcontacttime"
            | "projguarded"
            | "projguardedtime"
            | "projhit"
            | "projhittime"
            | "projmiss"
            | "projmissetime"
            | "numtarget"
            | "p2anim"
            | "p2bodydist"
            | "p2ctrl"
            | "p2dist"
            | "p2facing"
            | "p2life"
            | "p2movetype"
            | "p2prevstateno"
            | "p2stateno"
            | "p2statetype"
            | "physics"
            | "playerid"
            | "pos"
            | "power"
            | "prevstateno"
            | "prevstatetype"
            | "prevmovetype"
            | "random"
            | "roundno"
            | "roundsexisted"
            | "roundstate"
            | "displayname"
            | "authorname"
            | "numstagebg"
            | "roundtime"
            | "timeremaining"
            | "timeelapsed"
            | "timetotal"
            | "score"
            | "numplayer"
            | "localcoordx"
            | "stagetime"
            | "fighttime"
            | "teamsize"
            | "teamleader"
            | "memberno"
            | "standby"
            | "localcoordy"
            | "playerno"
            | "scoretotal"
            | "screenpos"
            | "screenvel"
            | "stagebackedge"
            | "stagebackedgedist"
            | "stagefrontedgedist"
            | "stateno"
            | "statetime"
            | "statetype"
            | "teamside"
            | "time"
            | "uniqhitcount"
            | "vel"
            | "winner"
            | "loser"
    )
}

/// `HitByAttr` matching: the wanted state/class groups must be subsets of the
/// filter's groups. A missing group matches anything; `*` matches anything.
fn hitbyattr_matches(wanted: &str, filter: &str) -> bool {
    let groups = |text: &str| -> (String, String) {
        let mut parts = text.splitn(2, ',');
        let states = parts.next().unwrap_or("").trim().to_ascii_uppercase();
        let classes = parts.next().unwrap_or("").trim().to_ascii_uppercase();
        (states, classes)
    };
    let (wanted_states, wanted_classes) = groups(wanted);
    let (filter_states, filter_classes) = groups(filter);
    if filter_states.is_empty() && filter_classes.is_empty() {
        return false;
    }
    let group_match = |want: &str, have: &str| {
        if want.is_empty() {
            return true;
        }
        if have.contains('*') {
            return true;
        }
        want.chars()
            .filter(|ch| *ch != '*')
            .all(|ch| have.contains(ch))
    };
    group_match(&wanted_states, &filter_states) && group_match(&wanted_classes, &filter_classes)
}

fn hitbyattr_value(sim: &SimState, who: usize, wanted: &str) -> bool {
    let fighter = &sim.fighters[who];
    fighter
        .hit
        .not_hit_by
        .as_ref()
        .or(fighter.hit.hit_by.as_ref())
        .map(|(attr, _)| hitbyattr_matches(wanted, attr))
        .unwrap_or(false)
}

fn eval_trigger(sim: &SimState, who: usize, name: &str, args: &[Expr]) -> Value {
    let f = &sim.fighters[who];
    let other = sim.other(who);
    let o = &sim.fighters[other];
    match name {
        "leftedge" => Value::Float(sim.camera.x - GAME_WIDTH * 0.5),
        "rightedge" => Value::Float(sim.camera.x + GAME_WIDTH * 0.5),
        "topedge" => Value::Float(sim.camera.y - sim.ground_screen_y()),
        "bottomedge" => Value::Float(sim.camera.y - sim.ground_screen_y() + GAME_HEIGHT),
        "backedge" => {
            let left = sim.camera.x - GAME_WIDTH * 0.5;
            let right = sim.camera.x + GAME_WIDTH * 0.5;
            Value::Float(if f.facing < 0 { right } else { left })
        }
        "frontedge" => {
            let left = sim.camera.x - GAME_WIDTH * 0.5;
            let right = sim.camera.x + GAME_WIDTH * 0.5;
            Value::Float(if f.facing < 0 { left } else { right })
        }
        "stagefrontedgedist" => Value::Float(stage_edge_distance(sim, f, true)),
        "stagebackedgedist" | "stagebackedge" => Value::Float(stage_edge_distance(sim, f, false)),
        "projcontacttime" => Value::Int(f.proj_contact_time),
        "projhittime" => Value::Int(f.proj_hit_time),
        "projguardedtime" => Value::Int(f.proj_guarded_time),
        "projcontact" => Value::Bool(f.proj_contact_time >= 0),
        "projhit" => Value::Bool(f.proj_hit_time >= 0),
        "projguarded" => Value::Bool(f.proj_guarded_time >= 0),
        "projmiss" | "projmissetime" => Value::Int(0),
        "time" | "statetime" => Value::Int(f.state_time),
        "stateno" => Value::Int(f.state_no),
        "prevstateno" => Value::Int(f.prev_state_no),
        "prevstatetype" => Value::Str(f.prev_state_type.code().to_string()),
        "prevmovetype" => Value::Str(f.prev_move_type.code().to_string()),
        "statetype" => Value::Str(f.state_type.code().to_string()),
        "movetype" => Value::Str(f.move_type.code().to_string()),
        "physics" => Value::Str(
            match f.physics {
                crate::fighter::Physics::Stand => "S",
                crate::fighter::Physics::Crouch => "C",
                crate::fighter::Physics::Air => "A",
                crate::fighter::Physics::None => "N",
            }
            .to_string(),
        ),
        "ctrl" => Value::Bool(f.ctrl),
        "alive" => Value::Bool(f.alive()),
        "life" => Value::Int(f.life),
        "lifemax" => Value::Int(f.life_max),
        "power" => Value::Float(f.power),
        "anim" => Value::Int(f.anim_no),
        "animelem" => Value::Int(f.anim_elem()),
        "animtime" => Value::Int(f.anim_time_remaining(sim.anim_character(who))),
        "animlength" => Value::Int(
            f.current_anim(sim.anim_character(who))
                .map(|anim| anim.total_time())
                .unwrap_or(0),
        ),
        "animloopcount" => Value::Int(f.anim_looped),
        "prevanim" => Value::Int(f.prev_anim),
        "sprpriority" => Value::Int(f.sprite_priority),
        "facing" => Value::Int(f.facing),
        "pos" => Value::Float(f.pos.x),
        "vel" => Value::Float(f.vel.x),
        "movecontact" => Value::Bool(f.attack.move_contact),
        "movehit" => Value::Bool(f.attack.move_hit),
        "moveguarded" => Value::Bool(f.attack.move_guarded),
        "hitcount" => Value::Int(f.attack.hit_count),
        "uniqhitcount" => Value::Int(f.attack.uniq_hit_count),
        "hitfall" => Value::Bool(f.hit.vars.fall),
        "hitover" => Value::Bool(hit_over(sim, who)),
        "hitshakeover" => Value::Bool(hit_shake_over(sim, who)),
        "canrecover" => Value::Bool(f.hit.vars.fall_recover && (5030..5050).contains(&f.state_no)),
        "inguarddist" => Value::Bool(inguard_dist(sim, who)),
        "guardbreak" => Value::Bool(f.guard_break || f.guard_crush_timer > 0),
        "dizzy" => Value::Bool(f.dizzy_flag || f.dizzy_timer > 0),
        "dizzypoints" => Value::Int(f.dizzy_points),
        "dizzypointsmax" => Value::Int(f.dizzy_max),
        "guardpoints" => Value::Int(f.guard_points),
        "guardpointsmax" => Value::Int(f.guard_max),
        "redlife" => Value::Int(f.red_life),
        "receivedhits" => Value::Int(f.received_hits),
        "receiveddamage" => Value::Int(f.received_damage),
        "combocount" => Value::Int(if f.owner.is_some() {
            0
        } else {
            sim.combo.get(f.side).map(|combo| combo.hits).unwrap_or(0)
        }),
        "score" => Value::Float(if f.owner.is_some() {
            0.0
        } else {
            sim.score.get(f.side).copied().unwrap_or(0.0)
        }),
        "numplayer" => Value::Int(sim.fighters.iter().filter(|f| f.active).count() as i32),
        "stagetime" => Value::Int(sim.bg_time as i32),
        "fighttime" => Value::Int(sim.match_time),
        // Single-member teams in the frozen scope.
        "teamsize" => Value::Int(1),
        "teamleader" => Value::Int(f.slot as i32 + 1),
        "memberno" => Value::Int(1),
        "standby" => Value::Bool(false),
        "localcoordx" => Value::Int(sim.assets.characters[f.char_id].localcoord().0 as i32),
        "localcoordy" => Value::Int(sim.assets.characters[f.char_id].localcoord().1 as i32),
        "playerno" => Value::Int(f.slot as i32 + 1),
        "scoretotal" => Value::Float(if f.owner.is_some() {
            0.0
        } else {
            let side = f.side.min(1);
            sim.score_rounds.get(side).copied().unwrap_or(0.0)
                + sim.score.get(side).copied().unwrap_or(0.0)
        }),
        "matchover" => Value::Bool(sim.round.phase == crate::RoundPhase::Over),
        "roundno" => Value::Int(sim.round.number as i32),
        "roundsexisted" => Value::Int(sim.round.number.saturating_sub(1) as i32),
        "roundstate" => Value::Int(sim.round.roundstate_code()),
        "roundtime" => Value::Int(sim.round.seconds()),
        "timeremaining" => Value::Int(sim.round.seconds()),
        "timetotal" => Value::Int(sim.round.round_time),
        "timeelapsed" => Value::Int((sim.round.round_time - sim.round.seconds()).max(0)),
        "teamside" => Value::Int(f.side as i32 + 1),
        "playerid" => Value::Int(sim.fighters[who].owner.unwrap_or(who) as i32),
        "ishelper" => Value::Int(sim.is_helper(who) as i32),
        "numenemy" => Value::Int(1),
        "numexplod" => Value::Int(
            sim.effects
                .iter()
                .filter(|effect| {
                    effect.owner == who
                        && matches!(effect.source, crate::EffectSource::Character(_))
                })
                .count() as i32,
        ),
        "numhelper" => {
            let root = sim.root_of(who);
            Value::Int(
                sim.fighters
                    .iter()
                    .filter(|fighter| fighter.owner == Some(root))
                    .count() as i32,
            )
        }
        "numpartner" => Value::Int(0),
        "numproj" => Value::Int(
            sim.projectiles
                .iter()
                .filter(|projectile| {
                    projectile.owner == who
                        && projectile.status == crate::projectile::ProjectileStatus::Active
                })
                .count() as i32,
        ),
        "numtarget" => Value::Int(f.target.is_some() as i32),
        "p2stateno" => Value::Int(o.state_no),
        "p2prevstateno" => Value::Int(o.prev_state_no),
        "p2statetype" => Value::Str(o.state_type.code().to_string()),
        "p2movetype" => Value::Str(o.move_type.code().to_string()),
        "p2life" => Value::Int(o.life),
        "p2anim" => Value::Int(o.anim_no),
        "p2ctrl" => Value::Bool(o.ctrl),
        "p2facing" => Value::Int(o.facing),
        "backedgedist" => Value::Float(edge_distance(sim, who, false, false)),
        "frontedgedist" => Value::Float(edge_distance(sim, who, true, false)),
        "backedgebodydist" => Value::Float(edge_distance(sim, who, false, true)),
        "frontedgebodydist" => Value::Float(edge_distance(sim, who, true, true)),
        "ailevel" => Value::Int(f.ai_level),
        "powermax" => Value::Int(crate::MAX_POWER as i32),
        "guardcount" => Value::Int(f.attack.guard_count),
        "attack" => Value::Float(snap3(100.0 * f.attack_mul)),
        "attackmul" => Value::Float(f.attack_mul),
        "defence" => Value::Float(snap3(100.0 * f.defence_mul)),
        "defencemul" => Value::Float(snap3(100.0 * f.defence_mul)),
        "angle" => Value::Float(if f.angle_draw {
            f.angle_draw_value.unwrap_or(f.draw_angle)
        } else {
            0.0
        }),
        "winner" => Value::Int((sim.round.winner == Some(who)) as i32),
        "loser" => Value::Int((sim.round.winner == Some(other)) as i32),
        "hitpausetime" => Value::Int(f.hit.hit_pause_time),
        "hitoverridden" => Value::Bool(f.hit_overridden),
        "index" => Value::Int(
            sim.fighters
                .iter()
                .take(who)
                .filter(|fighter| fighter.active)
                .count() as i32,
        ),
        "movereversed" => Value::Int(f.attack.move_reversed as i32),
        "screenwidth" | "gamewidth" => Value::Float(manato_core::GAME_WIDTH),
        "screenheight" | "gameheight" => Value::Float(manato_core::GAME_HEIGHT),
        "tickspersecond" => Value::Int(60),
        "gametime" => Value::Int(sim.frame as i32),
        "matchno" => Value::Int(1),
        "roundswon" => Value::Int(sim.round.wins[who] as i32),
        "win" => Value::Bool(!sim.is_helper(who) && sim.round.winner == Some(who)),
        "consecutivewins" => Value::Int(if sim.is_helper(who) {
            0
        } else {
            sim.round.consecutive_wins.get(who).copied().unwrap_or(0)
        }),
        "winko" => Value::Bool(
            !sim.is_helper(who)
                && sim.round.winner == Some(who)
                && sim.round.phase != crate::RoundPhase::Fight
                && !sim.round.time_over,
        ),
        "wintime" => Value::Bool(
            !sim.is_helper(who)
                && sim.round.winner == Some(who)
                && sim.round.phase != crate::RoundPhase::Fight
                && sim.round.time_over,
        ),
        "winperfect" => Value::Bool(
            !sim.is_helper(who)
                && sim.round.winner == Some(who)
                && sim
                    .win_history
                    .get(who)
                    .and_then(|history| history.last())
                    .is_some_and(|record| record.perfect),
        ),
        "winclutch" => Value::Bool(
            !sim.is_helper(who)
                && sim.round.winner == Some(who)
                && sim
                    .win_history
                    .get(who)
                    .and_then(|history| history.last())
                    .is_some_and(|record| record.clutch),
        ),
        "decisiveround" => Value::Bool(
            !sim.is_helper(who) && sim.round.wins[who] as i32 + 1 >= sim.round.rounds_to_win as i32,
        ),
        "lose" => Value::Bool(sim.round.winner == Some(other)),
        "loseko" => Value::Bool(
            sim.round.winner == Some(other)
                && sim.round.phase == crate::RoundPhase::Ko
                && !sim.round.time_over,
        ),
        "losetime" => Value::Bool(
            sim.round.winner == Some(other)
                && sim.round.phase != crate::RoundPhase::Fight
                && sim.round.time_over,
        ),
        "drawgame" => Value::Bool(
            sim.round.winner.is_none()
                && matches!(
                    sim.round.phase,
                    crate::RoundPhase::Ko | crate::RoundPhase::Over
                ),
        ),
        "teammode" => Value::Str("single".into()),
        // The simulator only models versus-style 1v1 play, where P1's side
        // (team side 1) is the home team. Arcade's CPU-home rule is not
        // representable because the engine does not track the CPU slot.
        "ishometeam" => Value::Int((f.side == 0) as i32),
        "parentexist" => Value::Int((sim.parent_of(who) != who) as i32),
        "rootdist" => {
            let axis = axis_arg(sim, who, args);
            let root = sim.root_of(who);
            Value::Float(relation_axis_distance(sim, who, root, &axis))
        }
        "parentdist" => {
            let axis = axis_arg(sim, who, args);
            let parent = sim.parent_of(who);
            Value::Float(relation_axis_distance(sim, who, parent, &axis))
        }
        "camerazoom" => Value::Float(sim.camera.scale),
        "camerapos" => Value::Float(sim.camera.x),
        "zoomvar" => Value::Float(sim.camera.scale),
        "hitvel" => Value::Float(f.hit.vars.x_vel),
        "p3name" | "p4name" => Value::Str(String::new()),
        "name" | "displayname" => Value::Str(sim.character(who).display_name().to_string()),
        "authorname" => Value::Str(sim.character(who).def.author.clone()),
        "numstagebg" => Value::Int(sim.stage_layers.len() as i32),
        "p1name" => Value::Str(sim.character(0).display_name().to_string()),
        "p2name" => Value::Str(sim.character(1).display_name().to_string()),
        "palno" => Value::Int(1),
        "timemod" => Value::Int(0),
        _ => Value::Int(0),
    }
}

/// Splits a dotted version string into up to three integer components.
fn version_parts(text: &str) -> (i32, i32, i32) {
    let mut parts = [0i32; 3];
    for (slot, piece) in parts.iter_mut().zip(text.split('.')) {
        *slot = piece.trim().parse().unwrap_or(0);
    }
    (parts[0], parts[1], parts[2])
}

/// Parses `mugenversion` and normalizes everything except 1.0/1.1 to WinMugen
/// (`0.5`), matching Ikemen's `ParseMugenVersion`.
fn mugen_version_parts(text: &str) -> (i32, i32) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return (1, 0);
    }
    let mut parts = trimmed.split('.');
    let major = parts
        .next()
        .and_then(|piece| piece.trim().parse::<i32>().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|piece| piece.trim().parse::<i32>().ok())
        .unwrap_or(0);
    if (major, minor) == (1, 0) || (major, minor) == (1, 1) {
        (major, minor)
    } else {
        (0, 5)
    }
}

/// `ExplodVar(id, index, field)` value lookup for one active effect.
fn explod_var(sim: &SimState, effect: &crate::Effect, field: &str) -> Value {
    let anim = match effect.source {
        crate::EffectSource::Character(char_id) => {
            sim.assets.characters[char_id].animation(effect.anim)
        }
        crate::EffectSource::FightFx => sim.assets.effects.animations.get(&effect.anim),
        crate::EffectSource::Stage => sim.assets.stage.animations.get(&effect.anim),
    };
    let start = anim
        .map(|anim| {
            anim.frames
                .iter()
                .take(effect.anim_elem.saturating_sub(1) as usize)
                .map(|frame| frame.effective_duration())
                .sum::<i32>()
        })
        .unwrap_or(0);
    let clock = if effect.anim_freeze {
        start
    } else {
        effect.time + start
    };
    let position = anim
        .and_then(|anim| anim.frame_at(clock))
        .map(|(frame, elapsed)| (frame as i32 + 1, elapsed));
    match field {
        "pos.x" => Value::Float(effect.pos.x),
        "pos.y" => Value::Float(effect.pos.y),
        "pos.z" => Value::Int(0),
        "vel.x" => Value::Float(effect.vel.x),
        "vel.y" => Value::Float(effect.vel.y),
        "vel.z" => Value::Int(0),
        "accel.x" => Value::Float(effect.accel.x),
        "accel.y" => Value::Float(effect.accel.y),
        "accel.z" => Value::Int(0),
        "scale.x" => Value::Float(effect.scale.x),
        "scale.y" => Value::Float(effect.scale.y),
        "anim" => Value::Int(effect.anim),
        "animelem" => Value::Int(position.map(|(elem, _)| elem).unwrap_or(0)),
        "animelemtime" => Value::Int(position.map(|(_, time)| time).unwrap_or(0)),
        "animtime" => Value::Int(anim.map(|anim| clock - anim.total_time()).unwrap_or(0)),
        "animprefix" => Value::Str("s".into()),
        "facing" => Value::Float(effect.facing as f32),
        "id" => Value::Int(effect.id),
        "layerno" => Value::Int(i32::from(!effect.behind)),
        "sprpriority" => Value::Int(effect.sprite_priority),
        "time" => Value::Int(effect.time),
        "removetime" => Value::Int(effect.duration),
        "bindtime" => Value::Int(effect.bind_time),
        "pausemovetime" => Value::Int(effect.pause_move_time),
        "ignorehitpause" => Value::Bool(effect.ignore_hit_pause),
        "angle" => Value::Float(effect.angle),
        "angle.x" => Value::Float(effect.x_angle),
        "angle.y" => Value::Float(effect.y_angle),
        "xshear" => Value::Float(effect.x_shear),
        _ => Value::Int(0),
    }
}

/// `ProjVar(id, index, field)` value lookup for one active projectile.
fn projectile_var(
    sim: &SimState,
    projectile: &crate::projectile::Projectile,
    field: &str,
) -> Value {
    match field {
        "pos.x" => Value::Float(projectile.pos.x),
        "pos.y" => Value::Float(projectile.pos.y),
        "pos.z" => Value::Int(0),
        "vel.x" => Value::Float(projectile.vel.x),
        "vel.y" => Value::Float(projectile.vel.y),
        "vel.z" => Value::Int(0),
        "accel.x" => Value::Float(projectile.accel.x),
        "accel.y" => Value::Float(projectile.accel.y),
        "accel.z" => Value::Int(0),
        "anim" => Value::Int(projectile.anim),
        "animelem" => {
            let character = &sim.assets.characters[sim.fighters[projectile.owner].char_id];
            Value::Int(
                character
                    .animation(projectile.anim)
                    .and_then(|anim| anim.frame_at(projectile.time))
                    .map(|(index, _)| index as i32 + 1)
                    .unwrap_or(0),
            )
        }
        "animprefix" => Value::Str("s".into()),
        "facing" => Value::Float(projectile.facing as f32),
        "projid" => Value::Int(projectile.id),
        "projlayerno" => Value::Int(projectile.layer_no),
        "projsprpriority" => Value::Int(projectile.sprite_priority),
        "projpriority" => Value::Int(projectile.priority),
        "projremovetime" => Value::Int(projectile.remove_time),
        "projstagebound" => Value::Float(projectile.stage_bound),
        "projedgebound" => Value::Float(projectile.edge_bound),
        "projhits" => Value::Int(projectile.hits),
        "projremove" => Value::Bool(projectile.remove_on_hits_exhausted),
        "projremanim" => Value::Int(projectile.rem_anim),
        "projhitanim" => Value::Int(projectile.hit_anim),
        "projcancelanim" => Value::Int(projectile.cancel_anim),
        "projmisstime" => Value::Int(projectile.miss_time),
        "scale.x" => Value::Float(projectile.scale.x),
        "scale.y" => Value::Float(projectile.scale.y),
        "angle" => Value::Float(projectile.angle),
        "angle.x" => Value::Int(0),
        "angle.y" => Value::Int(0),
        "xshear" => Value::Float(projectile.x_shear),
        "time" => Value::Int(projectile.time),
        "shadow.r" => Value::Int(projectile.shadow[0]),
        "shadow.g" => Value::Int(projectile.shadow[1]),
        "shadow.b" => Value::Int(projectile.shadow[2]),
        _ => Value::Int(0),
    }
}

/// Snaps a value to three decimals, matching Ikemen's attack/defence triggers.
fn snap3(value: f32) -> f32 {
    (value * 1000.0).round() / 1000.0
}

/// `AnimElemNo(t)`: the animation element active `t` ticks from now.
/// Positive values wrap into the loop region; negative values wrap backwards
/// through the full animation, matching Ikemen's `AnimElemNo`.
fn anim_elem_no(anim: &manato_formats::Animation, clock: i32, time: i32) -> i32 {
    if anim.frames.is_empty() {
        return 0;
    }
    let total = anim.total_time();
    if total <= 0 || anim.is_infinite() {
        return anim
            .frame_at((clock + time).max(0))
            .map(|(index, _)| index as i32 + 1)
            .unwrap_or(anim.frames.len() as i32);
    }
    let loop_start = anim.loop_start.min(anim.frames.len().saturating_sub(1));
    let loop_time = anim.frames[..loop_start].iter().fold(0i32, |acc, frame| {
        acc.saturating_add(frame.effective_duration())
    });
    let mut position = clock.saturating_add(time);
    if position < 0 {
        position %= total;
        if position < 0 {
            position += total;
        }
    }
    let span = total - loop_time;
    if position >= total && span > 0 {
        position = loop_time + (position - loop_time) % span;
    }
    let mut start = 0i32;
    for (index, frame) in anim.frames.iter().enumerate() {
        start = start.saturating_add(frame.effective_duration());
        if position < start || frame.is_infinite() {
            return index as i32 + 1;
        }
    }
    anim.frames.len() as i32
}

/// `StageBGVar(id, index, field)`: reads a stage background element state.
/// `id < 0` matches every layer; `index` is the Nth match.
fn stage_bg_var(sim: &SimState, id: i32, index: usize, field: &str) -> Value {
    let Some((layer_index, layer)) = sim
        .assets
        .stage
        .layers
        .iter()
        .enumerate()
        .filter(|(_, layer)| id < 0 || layer.id == id)
        .nth(index)
    else {
        return Value::Int(0);
    };
    let Some(state) = sim.stage_layers.get(layer_index) else {
        return Value::Int(0);
    };
    match field {
        "actionno" => Value::Int(state.action.or(layer.action).unwrap_or(-1)),
        "animelem" | "animelemtime" | "animtime" | "animloopcount" => {
            let Some(action) = state.action.or(layer.action) else {
                return Value::Int(0);
            };
            let Some(anim) = sim.assets.stage.animations.get(&action) else {
                return Value::Int(0);
            };
            let position = anim_position(anim, sim.bg_time as i32);
            match field {
                "animelem" => Value::Int(position.elem),
                "animelemtime" => Value::Int(position.elem_time),
                "animtime" => Value::Int(position.anim_time),
                _ => Value::Int(position.loop_count),
            }
        }
        "delta.x" => Value::Float(layer.delta.x),
        "delta.y" => Value::Float(layer.delta.y),
        "start.x" => Value::Float(layer.start.x),
        "start.y" => Value::Float(layer.start.y),
        "velocity.x" => Value::Float(layer.velocity.x),
        "velocity.y" => Value::Float(layer.velocity.y),
        "tile.x" => Value::Int(layer.tile.0),
        "tile.y" => Value::Int(layer.tile.1),
        "layerno" => Value::Int(layer.layer_no),
        "id" => Value::Int(layer.id),
        "pos.x" => Value::Float(layer.start.x + state.offset.x),
        "pos.y" => Value::Float(layer.start.y + state.offset.y),
        _ => Value::Int(0),
    }
}

/// Position of a stage BG animation for a monotonically increasing clock,
/// mirroring the looping `Action` model used by the renderer.
struct AnimPosition {
    elem: i32,
    elem_time: i32,
    anim_time: i32,
    loop_count: i32,
}

fn anim_position(anim: &manato_formats::Animation, clock: i32) -> AnimPosition {
    let clock = clock.max(0);
    let total = anim.total_time();
    if anim.frames.is_empty() || total <= 0 || anim.is_infinite() {
        let (elem, elem_time) = anim
            .frame_at(clock)
            .map(|(index, elapsed)| (index as i32 + 1, elapsed))
            .unwrap_or((0, 0));
        return AnimPosition {
            elem,
            elem_time,
            anim_time: 0,
            loop_count: 0,
        };
    }
    let loop_time = anim.frames[..anim.loop_start.min(anim.frames.len())]
        .iter()
        .fold(0i32, |acc, frame| {
            acc.saturating_add(frame.effective_duration())
        });
    let loop_span = (total - loop_time).max(1);
    let (position, loop_count) = if clock < loop_time {
        (clock.min(total - 1), 0)
    } else {
        (
            loop_time + (clock - loop_time) % loop_span,
            (clock - loop_time) / loop_span,
        )
    };
    let mut start = 0i32;
    let mut elem = anim.frames.len() as i32;
    let mut elem_time = position;
    for (index, frame) in anim.frames.iter().enumerate() {
        let duration = frame.effective_duration();
        if position < start.saturating_add(duration) {
            elem = index as i32 + 1;
            elem_time = position - start;
            break;
        }
        start = start.saturating_add(duration);
    }
    AnimPosition {
        elem,
        elem_time,
        anim_time: position - total,
        loop_count,
    }
}

fn hit_over(sim: &SimState, who: usize) -> bool {
    sim.fighters[who].hit.vars.hittime < 0
}

fn hit_shake_over(sim: &SimState, who: usize) -> bool {
    sim.fighters[who].hit.hit_shake_timer <= 0
}

fn inguard_dist(sim: &SimState, who: usize) -> bool {
    let f = &sim.fighters[who];
    let other = sim.other(who);
    let o = &sim.fighters[other];
    let dx = (o.pos.x - f.pos.x).abs();
    let attacking = o.move_type == crate::fighter::MoveType::Attack || o.hitdef.is_some();
    if attacking {
        let guard_dist = o
            .hitdef
            .as_ref()
            .map(|hitdef| hitdef.guard_dist)
            .unwrap_or(160) as f32;
        if dx < guard_dist {
            return true;
        }
    }
    sim.projectiles.iter().any(|projectile| {
        projectile.owner == other
            && projectile.status == crate::projectile::ProjectileStatus::Active
            && projectile.hits > 0
            && (projectile.pos.x - f.pos.x).abs() < projectile.hitdef.guard_dist as f32
    })
}

fn stage_edge_distance(sim: &SimState, f: &crate::fighter::Fighter, front: bool) -> f32 {
    let half = GAME_WIDTH * 0.5;
    let left = (sim.camera.x - half).max(sim.assets.stage.left_bound);
    let right = (sim.camera.x + half).min(sim.assets.stage.right_bound);
    let forward = f.facing >= 0;
    // `front == forward` asks for the edge in front of the fighter.
    let towards_right = front == forward;
    if towards_right {
        right - f.pos.x
    } else {
        f.pos.x - left
    }
}

fn edge_distance(sim: &SimState, who: usize, front: bool, body: bool) -> f32 {
    let f = &sim.fighters[who];
    let half = GAME_WIDTH * 0.5;
    let left = sim.camera.x - half;
    let right = sim.camera.x + half;
    let forward = f.facing >= 0;
    let towards_right = front == forward;
    if towards_right {
        let edge = right - (f.pos.x + if body { f.width_front } else { 0.0 });
        edge.max(0.0)
    } else {
        let edge = (f.pos.x - if body { f.width_back } else { 0.0 }) - left;
        edge.max(0.0)
    }
}

/// Checks a named command against the fighter's input history.
pub fn command_active(sim: &SimState, who: usize, name: &str) -> bool {
    let f = &sim.fighters[who];
    let lower = name.trim().to_ascii_lowercase();
    match lower.as_str() {
        "holdfwd" => return f.relative_dir().is_forward(),
        "holdback" => return f.relative_dir().is_back(),
        "holdup" => return f.relative_dir().is_up(),
        "holddown" => return f.relative_dir().is_down(),
        _ => {}
    }
    sim.character(who)
        .commands
        .iter()
        .any(|c: &CommandDef| c.name.eq_ignore_ascii_case(&lower) && f.match_command(c))
}

/// Absolute direction for a command relative to a facing (used by tests).
pub fn command_dir(facing: i32, dir: Dir8) -> Vec2 {
    let x = match dir {
        Dir8::Fwd | Dir8::UpFwd | Dir8::DnFwd => facing as f32,
        Dir8::Back | Dir8::UpBack | Dir8::DnBack => -(facing as f32),
        _ => 0.0,
    };
    let y = match dir {
        Dir8::Up | Dir8::UpFwd | Dir8::UpBack => -1.0,
        Dir8::Down | Dir8::DnFwd | Dir8::DnBack => 1.0,
        _ => 0.0,
    };
    Vec2::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_helpers() {
        assert_eq!(Value::Int(3).as_i32(), 3);
        assert!(Value::Str("A".into()).truthy());
        assert!(!Value::Str("0".into()).truthy());
    }
}
