#![allow(clippy::field_reassign_with_default)]

use std::path::PathBuf;

use manato_core::PlayerInput;
use manato_sim::{ai, MatchConfig, MatchOptions, SimState};

fn mugen_root() -> PathBuf {
    std::env::var("MANATO_MUGEN_ROOT")
        .or_else(|_| std::env::var("MUGEN_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn load(p1: &str, p2: &str) -> Option<SimState> {
    let root = mugen_root();
    let config = MatchConfig {
        mugen_root: root.clone(),
        p1: PathBuf::from(p1),
        p2: PathBuf::from(p2),
        stage: PathBuf::from("content/stages/training/stage1.def"),
        screenpack: None,
        rounds_to_win: 2,
        round_time: 99,
    };
    let p1_path = root.join(p1);
    let p2_path = root.join(p2);
    let stage_path = root.join("content/stages/training/stage1.def");
    if !p1_path.exists() || !p2_path.exists() || !stage_path.exists() {
        eprintln!("skipping: content not present at {}", root.display());
        return None;
    }
    let mut options = MatchOptions::default();
    options.ai_level = 0;
    Some(SimState::new(
        std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
        options,
    ))
}

fn run(mut sim: SimState, ticks: u32, ai_level: i32) {
    for _ in 0..ticks {
        let inputs = if ai_level > 0 {
            [
                ai::input_for(&sim, 0, ai_level),
                ai::input_for(&sim, 1, ai_level),
            ]
        } else {
            [PlayerInput::default(); 2]
        };
        sim.tick(inputs);
        assert!(sim
            .fighters
            .iter()
            .all(|f| f.pos.x.is_finite() && f.pos.y.is_finite()));
        assert!(sim
            .fighters
            .iter()
            .all(|f| f.life >= 0 && f.life <= f.life_max));
    }
}

#[test]
fn kfm_mirror_idles() {
    let Some(sim) = load(
        "mugen-1.1b1/chars/kfm/kfm.def",
        "mugen-1.1b1/chars/kfm/kfm.def",
    ) else {
        return;
    };
    run(sim, 300, 0);
}

#[test]
fn custom_characters_fight() {
    let Some(sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    run(sim, 3000, 6);
}

#[test]
fn bundled_characters_fight() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let start_life = [sim.fighters[0].life, sim.fighters[1].life];
    let mut min_life = start_life;
    let mut heard_hit = false;
    for _ in 0..1_800 {
        sim.tick([ai::input_for(&sim, 0, 6), ai::input_for(&sim, 1, 6)]);
        for event in &sim.sounds {
            if matches!(event.source, manato_sim::SoundSource::Character(_)) && event.group == 5 {
                heard_hit = true;
            }
        }
        for (i, fighter) in sim.fighters.iter().enumerate() {
            min_life[i] = min_life[i].min(fighter.life);
        }
    }
    assert!(
        min_life[0] < start_life[0] || min_life[1] < start_life[1],
        "bundled characters dealt no damage"
    );
    assert!(heard_hit, "bundled characters emitted no hit sound");
}

#[test]
fn kfm_vs_custom() {
    let Some(sim) = load(
        "mugen-1.1b1/chars/kfm/kfm.def",
        "content/chars/ordan/ordan.def",
    ) else {
        return;
    };
    run(sim, 2000, 8);
}

#[test]
fn rollback_clone_is_deterministic() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    for _ in 0..120 {
        sim.tick([PlayerInput::default(); 2]);
    }
    let mut a = sim.clone();
    let mut b = sim.clone();
    for _ in 0..600 {
        a.tick([
            PlayerInput::new(manato_core::DIR_RIGHT, 1),
            PlayerInput::new(manato_core::DIR_LEFT, 2),
        ]);
        b.tick([
            PlayerInput::new(manato_core::DIR_RIGHT, 1),
            PlayerInput::new(manato_core::DIR_LEFT, 2),
        ]);
    }
    assert_eq!(a.checksum(), b.checksum());
}

#[test]
fn kfm_vs_nival_ai_deals_damage_and_lands() {
    let Some(mut sim) = load(
        "mugen-1.1b1/chars/kfm/kfm.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let start_life = [sim.fighters[0].life, sim.fighters[1].life];
    let mut min_life = start_life;
    let mut heard_attack = false;
    let mut heard_hit = false;
    for _ in 0..1800 {
        sim.tick([ai::input_for(&sim, 0, 6), ai::input_for(&sim, 1, 6)]);
        for event in &sim.sounds {
            if matches!(event.source, manato_sim::SoundSource::Character(_)) {
                if event.group == 0 {
                    heard_attack = true;
                }
                if event.group == 5 {
                    heard_hit = true;
                }
            }
        }
        for (i, fighter) in sim.fighters.iter().enumerate() {
            min_life[i] = min_life[i].min(fighter.life);
            // An air state must not remain on the ground: the engine should
            // transition to jump-land (52) once the jump has actually landed.
            // State 50 is allowed briefly on the ground while it lifts off.
            if fighter.physics == manato_sim::fighter::Physics::Air
                && fighter.pos.y >= 0.0
                && fighter.state_no != 52
                && fighter.state_time > 30
            {
                panic!(
                    "fighter stuck in air state {} on the ground for {} ticks",
                    fighter.state_no, fighter.state_time
                );
            }
        }
    }
    assert!(
        min_life[0] < start_life[0] || min_life[1] < start_life[1],
        "AI match dealt no damage (move types may be stuck)"
    );
    assert!(heard_attack, "attacks emitted no character sound");
    assert!(heard_hit, "hits emitted no character hit sound");
}

#[test]
fn bundled_projectile_hits() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -10.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 10.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(1000, &assets.characters[0], false);
    let mut spawned = false;
    for _ in 0..120 {
        sim.tick([PlayerInput::default(); 2]);
        spawned |= !sim.projectiles.is_empty();
        if sim.fighters[1].life < sim.fighters[1].life_max {
            break;
        }
    }
    assert!(spawned, "state 1000 did not spawn a projectile");
    assert_eq!(
        sim.fighters[1].life,
        sim.fighters[1].life_max - 55,
        "Kaen projectile deals 55 unguarded damage"
    );
    assert!(sim.fighters[1].state_no >= 5000);
    assert!(
        sim.fighters[0].proj_contact_time >= 0,
        "projectile contact timer should start at the hit tick"
    );
    assert!(sim.fighters[0].proj_hit_time >= 0);
    assert_eq!(sim.fighters[0].proj_guarded_time, -1);
}

#[test]
fn projectile_state_is_rollback_deterministic() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(1000, &assets.characters[0], false);
    let mut a = sim.clone();
    let mut b = sim.clone();
    for _ in 0..80 {
        a.tick([PlayerInput::default(); 2]);
        b.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(a.checksum(), b.checksum());
    assert!(a.projectiles.len() <= 1);
}

#[test]
fn life_controllers_work() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    let damage = manato_formats::Controller {
        kind: "lifeadd".into(),
        params: vec![
            ("type".into(), "LifeAdd".into()),
            ("value".into(), "-100".into()),
            ("kill".into(), "0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &damage, &assets);
    assert_eq!(sim.fighters[0].life, 900);
    sim.fighters[0].life = 50;
    manato_sim::controller::execute(&mut sim, 0, &damage, &assets);
    assert_eq!(sim.fighters[0].life, 1, "kill=0 must prevent KO");

    let set = manato_formats::Controller {
        kind: "lifeset".into(),
        params: vec![
            ("type".into(), "LifeSet".into()),
            ("value".into(), "750".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &set, &assets);
    assert_eq!(sim.fighters[0].life, 750);
}

#[test]
fn facep2_turns_toward_opponent() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    sim.fighters[0].pos.x = 0.0;
    sim.fighters[1].pos.x = -200.0;
    let assets = sim.assets.clone();
    sim.fighters[1].enter_state(40, &assets.characters[1], true);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].facing, 1);
}

#[test]
fn hit_override_redirects_instead_of_damage() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    sim.fighters[0].pos.x = -20.0;
    sim.fighters[1].pos.x = 20.0;
    sim.fighters[1].hit_override = Some(manato_sim::fighter::HitOverride {
        attr: "S, NA".into(),
        state: 0,
        time: 100,
    });
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    for _ in 0..30 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].life < sim.fighters[1].life_max {
            break;
        }
    }
    assert_eq!(
        sim.fighters[1].life, sim.fighters[1].life_max,
        "HitOverride should prevent damage"
    );
}

#[test]
fn afterimage_and_palfx_execute() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    let after = manato_formats::Controller {
        kind: "afterimage".into(),
        params: vec![
            ("type".into(), "AfterImage".into()),
            ("time".into(), "10".into()),
            ("length".into(), "5".into()),
            ("timegap".into(), "1".into()),
            ("trans".into(), "add".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &after, &assets);
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(!sim.fighters[0].after_images.is_empty());

    let palfx = manato_formats::Controller {
        kind: "palfx".into(),
        params: vec![
            ("type".into(), "PalFX".into()),
            ("add".into(), "32,0,0".into()),
            ("mul".into(), "1,1,1".into()),
            ("time".into(), "5".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &palfx, &assets);
    assert_eq!(sim.fighters[0].pal_add, [32.0, 0.0, 0.0]);
    assert_eq!(sim.fighters[0].pal_time, 5);
}

#[test]
fn reversal_and_trans_execute() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    let reversal = manato_formats::Controller {
        kind: "reversaldef".into(),
        params: vec![
            ("type".into(), "ReversalDef".into()),
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "10".into()),
            ("time".into(), "5".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &reversal, &assets);
    assert!(sim.fighters[0].reversal.is_some());
    assert_eq!(sim.fighters[0].reversal_time, 5);

    let trans = manato_formats::Controller {
        kind: "trans".into(),
        params: vec![
            ("type".into(), "Trans".into()),
            ("trans".into(), "add".into()),
            ("alpha".into(), "128".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &trans, &assets);
    assert!((sim.fighters[0].trans_alpha - 128.0 / 255.0).abs() < 0.001);
}

#[test]
fn helper_spawn_bind_and_remove() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("type".into(), "Helper".into()),
            ("stateno".into(), "0".into()),
            ("id".into(), "7".into()),
            ("pos".into(), "10,0".into()),
            ("postype".into(), "p1".into()),
            ("removetime".into(), "5".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    assert_eq!(sim.fighters.len(), 3);
    let helper_index = 2;
    assert_eq!(sim.fighters[helper_index].owner, Some(0));
    assert_eq!(sim.fighters[helper_index].helper_id, 7);
    assert_eq!(
        sim.fighters[helper_index].pos.x,
        sim.fighters[0].pos.x + 10.0
    );

    let bind = manato_formats::Controller {
        kind: "bindtoroot".into(),
        params: vec![
            ("type".into(), "BindToRoot".into()),
            ("pos".into(), "0,-20".into()),
        ],
        line: 0,
    };
    sim.fighters[0].pos.x = -50.0;
    manato_sim::controller::execute(&mut sim, helper_index, &bind, &assets);
    assert_eq!(sim.fighters[helper_index].pos.x, -50.0);
    assert_eq!(sim.fighters[helper_index].pos.y, -20.0);

    let parent_var = manato_formats::Controller {
        kind: "parentvarset".into(),
        params: vec![
            ("type".into(), "ParentVarSet".into()),
            ("v".into(), "5".into()),
            ("value".into(), "42".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, helper_index, &parent_var, &assets);
    assert_eq!(sim.fighters[0].vars[5], 42);

    for _ in 0..6 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.fighters.len(), 2, "helper should expire");
}

#[test]
fn helper_hitdef_damages_player() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    sim.fighters[0].pos.x = -20.0;
    sim.fighters[1].pos.x = 20.0;
    let assets = sim.assets.clone();
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("type".into(), "Helper".into()),
            ("stateno".into(), "200".into()),
            ("id".into(), "1".into()),
            ("pos".into(), "0,0".into()),
            ("postype".into(), "p1".into()),
            ("removetime".into(), "60".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    for _ in 0..30 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].life < sim.fighters[1].life_max {
            break;
        }
    }
    assert!(
        sim.fighters[1].life < sim.fighters[1].life_max,
        "helper HitDef should damage the player"
    );
}

#[test]
fn intro_locks_control() {
    let Some(mut sim) = load(
        "mugen-1.1b1/chars/kfm/kfm.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    for _ in 0..30 {
        sim.tick([ai::input_for(&sim, 0, 8), ai::input_for(&sim, 1, 8)]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Intro);
    assert_eq!(sim.fighters[0].life, sim.fighters[0].life_max);
    assert_eq!(sim.fighters[1].life, sim.fighters[1].life_max);
}

fn hitdef() -> manato_sim::hitdef::HitDef {
    let mut hitdef = manato_sim::hitdef::HitDef::default();
    hitdef.damage = 20;
    hitdef.guard_damage = 5;
    hitdef.ground_velocity = manato_core::Vec2::new(4.0, 0.0);
    hitdef.air_velocity = manato_core::Vec2::new(4.0, -6.0);
    hitdef.ground_hittime = 8;
    hitdef.air_hittime = 8;
    hitdef.attr.state_types = "S".into();
    hitdef
}

#[test]
fn throw_hitdef_enters_throw_states_and_binds() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let mut throw = hitdef();
    throw.p1stateno = 800;
    throw.p2stateno = 820;
    throw.p1facing = 1;
    throw.p2facing = -1;
    let origin = sim.fighters[0].pos;
    let facing = sim.fighters[0].facing;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &throw, false, origin, facing, true);
    assert_eq!(sim.fighters[0].state_no, 800);
    assert_eq!(sim.fighters[1].state_no, 820);
    assert_eq!(sim.fighters[0].target, Some(1));
    assert_eq!(sim.fighters[1].facing, -1);
}

#[test]
fn corner_push_adds_velocity_near_the_stage_edge() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let right = sim.assets.stage.right_bound;
    sim.fighters[1].pos.x = right;
    let mut push = hitdef();
    push.ground_cornerpush = -8.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &push, false, origin, 1, true);
    // Base velocity 4 plus the -8 corner push.
    assert_eq!(sim.fighters[1].hit.vars.x_vel, -4.0);

    // Away from the corner the push does not apply.
    sim.fighters[1].pos.x = 0.0;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &push, false, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, 4.0);
}

#[test]
fn guard_damage_uses_guard_damage() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let start = sim.fighters[1].life;
    let hit = hitdef();
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.fighters[1].life, start - 5);
    assert!(sim.fighters[1].hit.guarding);
    assert_eq!(sim.fighters[1].state_no, 150);
}

fn fight_phase_with(p1: &str, p2: &str) -> Option<SimState> {
    let mut sim = load(p1, p2)?;
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Fight);
    Some(sim)
}

fn fight_phase() -> Option<SimState> {
    let mut sim = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    )?;
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Fight);
    Some(sim)
}

#[test]
fn ko_awards_round_and_enters_ko_phase() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    assert_eq!(sim.round.winner, Some(0));
    assert_eq!(sim.round.wins[0], 1);
    assert_eq!(sim.round.roundstate_code(), 3);
    assert!(!sim.round.time_over);
}

#[test]
fn time_over_uses_remaining_life() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.round.timer = 1;
    sim.fighters[0].life = 900;
    sim.fighters[1].life = 700;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    assert_eq!(sim.round.winner, Some(0));
    assert!(sim.round.time_over);
}

#[test]
fn double_ko_is_a_draw() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].life = 0;
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    assert_eq!(sim.round.winner, None);
    assert_eq!(sim.round.wins, [0, 0]);
}

#[test]
fn next_round_resets_life_and_keeps_wins() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    for _ in 0..130 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.number, 2);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Intro);
    assert_eq!(sim.round.wins[0], 1);
    assert_eq!(sim.fighters[0].life, sim.fighters[0].life_max);
    assert_eq!(sim.fighters[1].life, sim.fighters[1].life_max);
    assert_eq!(sim.fighters.len(), 2, "helpers must be removed");
}

#[test]
fn match_over_after_final_round() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.round.rounds_to_win = 1;
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    for _ in 0..130 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Over);
    assert!(sim.finished);
    assert_eq!(sim.round.roundstate_code(), 4);
    assert_eq!(sim.round.winner, Some(0));
}

#[test]
fn round_not_over_delays_the_next_phase() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    // AssertSpecial is per-tick, so install it in state 0 to re-assert.
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let state = assets.characters[0].states.get_mut(&0).expect("statedef 0");
        state.controllers.push(manato_formats::Controller {
            kind: "assertspecial".into(),
            params: vec![
                ("type".into(), "AssertSpecial".into()),
                ("roundnotover".into(), "1".into()),
            ],
            line: 0,
        });
    }
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    for _ in 0..200 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let state = assets.characters[0].states.get_mut(&0).expect("statedef 0");
        state.controllers.retain(|c| c.kind != "assertspecial");
    }
    for _ in 0..130 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_ne!(sim.round.phase, manato_sim::RoundPhase::Ko);
}

#[test]
fn round_triggers_report_phase_and_time() {
    let Some(sim) = fight_phase() else {
        return;
    };
    assert_eq!(sim.round.roundstate_code(), 2);
    assert_eq!(sim.round.seconds(), 99);
    let expr = manato_sim::expr::Expr::Ident("roundtime".into());
    assert_eq!(
        manato_sim::trigger::eval(&sim, 0, &expr).as_i32(),
        99,
        "RoundTime trigger should report remaining seconds"
    );
}

#[test]
fn winner_enters_win_pose_when_present() {
    let Some(mut sim) = load(
        "mugen-1.1b1/chars/kfm/kfm.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Fight);
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.winner, Some(0));
    assert_eq!(
        sim.fighters[0].state_no, 180,
        "winner should enter state 180"
    );
}

#[test]
fn sound_controllers_emit_channel_events() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    let play = manato_formats::Controller {
        kind: "playsnd".into(),
        params: vec![
            ("type".into(), "PlaySnd".into()),
            ("value".into(), "5, 1".into()),
            ("volume".into(), "80".into()),
            ("channel".into(), "2".into()),
            ("pan".into(), "-50".into()),
            ("loop".into(), "1".into()),
            ("priority".into(), "7".into()),
            ("lowpriority".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &play, &assets);
    let event = sim.sounds.last().expect("sound event");
    assert_eq!(event.group, 5);
    assert_eq!(event.index, 1);
    assert_eq!(event.channel, 2);
    assert!(event.loop_sound);
    assert_eq!(event.priority, 7);
    assert!(event.low_priority);
    assert!((event.pan + 0.5).abs() < 0.001);

    let stop = manato_formats::Controller {
        kind: "stopsnd".into(),
        params: vec![
            ("type".into(), "StopSnd".into()),
            ("channel".into(), "2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &stop, &assets);
    assert!(matches!(
        sim.sound_commands.last(),
        Some(manato_sim::SoundCommand::Stop { channel: 2, .. })
    ));

    let pan = manato_formats::Controller {
        kind: "sndpan".into(),
        params: vec![
            ("type".into(), "SndPan".into()),
            ("channel".into(), "2".into()),
            ("pan".into(), "25".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &pan, &assets);
    match sim.sound_commands.last() {
        Some(manato_sim::SoundCommand::Pan { channel, pan, .. }) => {
            assert_eq!(*channel, 2);
            assert!((pan.unwrap() - 0.25).abs() < 0.001);
        }
        other => panic!("unexpected command {other:?}"),
    }

    let modify = manato_formats::Controller {
        kind: "modifysnd".into(),
        params: vec![
            ("type".into(), "ModifySnd".into()),
            ("channel".into(), "2".into()),
            ("value".into(), "6, 3".into()),
            ("loop".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    match sim.sound_commands.last() {
        Some(manato_sim::SoundCommand::Modify {
            channel,
            group,
            index,
            loop_sound,
            ..
        }) => {
            assert_eq!(*channel, 2);
            assert_eq!(*group, 6);
            assert_eq!(*index, 3);
            assert_eq!(*loop_sound, Some(true));
        }
        other => panic!("unexpected command {other:?}"),
    }
}

#[test]
fn explod_parses_transparency_binding_and_projection() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    let explod = manato_formats::Controller {
        kind: "explod".into(),
        params: vec![
            ("type".into(), "Explod".into()),
            ("anim".into(), "10".into()),
            ("id".into(), "3".into()),
            ("pos".into(), "10, 20".into()),
            ("bindtime".into(), "2".into()),
            ("removeongethit".into(), "1".into()),
            ("trans".into(), "addalpha".into()),
            ("alpha".into(), "256, 128".into()),
            ("angle".into(), "30".into()),
            ("xangle".into(), "15".into()),
            ("yangle".into(), "10".into()),
            ("xshear".into(), "0.25".into()),
            ("projection".into(), "perspective".into()),
            ("focallength".into(), "1024".into()),
            ("space".into(), "screen".into()),
            ("window".into(), "1, 2, 300, 200".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &explod, &assets);
    let effect = sim.effects.last().expect("explod created");
    assert_eq!(effect.id, 3);
    assert_eq!(effect.blend, manato_sim::EffectBlend::Add);
    assert!(effect.transparent);
    assert!((effect.alpha - 0.5).abs() < 0.001);
    assert!((effect.angle - 30.0).abs() < 0.001);
    assert!((effect.x_angle - 15.0).abs() < 0.001);
    assert!((effect.y_angle - 10.0).abs() < 0.001);
    assert!((effect.x_shear - 0.25).abs() < 0.001);
    assert_eq!(effect.projection, manato_sim::ProjectionMode::Perspective);
    assert!((effect.focal_length - 1024.0).abs() < 0.001);
    assert!(effect.screen_space);
    assert_eq!(effect.window, Some([1.0, 2.0, 300.0, 200.0]));
    assert!(effect.bound);
    assert!(effect.remove_on_get_hit);
    assert_eq!(effect.bind_time, 2);

    // Binding follows the anchor until `bindtime` expires.
    let facing = sim.fighters[0].facing as f32;
    let offset_x = 10.0 * facing;
    let offset_y = 20.0;
    let start = sim.fighters[0].pos;
    sim.tick([PlayerInput::default(); 2]);
    let bound = sim.effects.last().expect("explod still alive");
    assert!(bound.bound);
    assert!((bound.pos.x - (sim.fighters[0].pos.x + offset_x)).abs() < 0.001);
    assert!((bound.pos.y - (sim.fighters[0].pos.y + offset_y)).abs() < 0.001);
    assert!((sim.fighters[0].pos.x - start.x).abs() < 100.0);

    // Release after the remaining bind tick; the effect then moves itself.
    sim.effects.last_mut().unwrap().remove_on_change_state = false;
    sim.tick([PlayerInput::default(); 2]);
    let released = sim.effects.last().expect("explod still alive");
    assert!(!released.bound || released.bind_time == 0);
}

#[test]
fn zoom_controller_and_triggers_round_trip() {
    // The bundled stage declares no zoom range (`[Camera]` defaults to 1.0,
    // matching Ikemen-GO). Patch in an explicit range so the controller is
    // observable, then verify the override drives `camerazoom`.
    let root = mugen_root();
    let config = MatchConfig {
        mugen_root: root.clone(),
        p1: PathBuf::from("content/chars/kaen/kaen.def"),
        p2: PathBuf::from("content/chars/nival/nival.def"),
        stage: PathBuf::from("content/stages/training/stage1.def"),
        screenpack: None,
        rounds_to_win: 2,
        round_time: 99,
    };
    if !root.join(&config.stage).exists() {
        eprintln!("skipping: content not present at {}", root.display());
        return;
    }
    let mut loaded = manato_sim::Assets::load(&config).expect("assets load");
    loaded.stage.camera.zoom_out = 0.5;
    loaded.stage.camera.zoom_in = 2.0;
    let mut options = MatchOptions::default();
    options.ai_level = 0;
    let mut sim = SimState::new(std::sync::Arc::new(loaded), options);
    let assets = sim.assets.clone();
    let zoom = manato_formats::Controller {
        kind: "zoom".into(),
        params: vec![
            ("type".into(), "Zoom".into()),
            ("pos".into(), "40, -10".into()),
            ("scale".into(), "0.5".into()),
            ("time".into(), "4".into()),
            ("endlag".into(), "2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &zoom, &assets);
    assert!(sim.camera.zoom_override.is_some());
    assert!(
        (manato_sim::trigger::num(&sim, 0, "ZoomVar(scale)") - 0.5).abs() < 0.001,
        "ZoomVar(scale) should read the override target"
    );

    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
        assert!(sim.camera.scale < 0.95, "zoom should steer toward 0.5");
    }
    assert!(
        (manato_sim::trigger::num(&sim, 0, "camerazoom") - sim.camera.scale).abs() < 0.001,
        "camerazoom should read the live scale"
    );

    for _ in 0..10 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        sim.camera.zoom_override.is_none(),
        "override should release after time + endlag"
    );

    // Screen mapping (§11.5): the floor is exact at scale 1, and with the
    // default centre anchor the anchor world point stays at the view centre
    // for any scale (no jump when zoom engages).
    sim.camera.scale = 1.0;
    let floor = sim.screen_pos(manato_core::Vec2::new(0.0, 0.0));
    let legacy = sim.ground_screen_y() - sim.camera.y + sim.camera.shake.y;
    assert!((floor.y - legacy).abs() < 0.001);
    let center_world = manato_core::Vec2::new(
        0.0,
        sim.camera.y + manato_core::GAME_HEIGHT * 0.5 - sim.ground_screen_y(),
    );
    for scale in [0.5, 1.0, 1.7] {
        sim.camera.scale = scale;
        let mapped = sim.screen_pos(center_world);
        let expected = manato_core::GAME_HEIGHT * 0.5 + sim.camera.shake.y;
        assert!(
            (mapped.y - expected).abs() < 0.001,
            "centre anchor must be scale independent (scale {scale})"
        );
    }
}

#[test]
fn combo_counter_tracks_hits_and_expires() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let origin = sim.fighters[0].pos;
    let hit = hitdef();
    for _ in 0..3 {
        manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    }
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.combo[0].hits, 3);
    assert_eq!(sim.combo[0].shown_hits, 3);
    assert!(sim.combo[0].timer > 0, "combo keeps a display timer");
    assert_eq!(sim.combo[0].damage, 60);
    assert_eq!(sim.combo[1].shown_hits, 0);

    // The counter survives recovery, then hides once the timer expires.
    for _ in 0..180 {
        if sim.fighters[1].received_hits == 0 {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.fighters[1].received_hits, 0);
    assert_eq!(sim.combo[0].shown_hits, 3);
    for _ in 0..manato_sim::COMBO_DISPLAY_TICKS + 1 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.combo[0].shown_hits, 0);
    assert_eq!(sim.combo[0].timer, 0);
}

#[test]
fn win_history_records_perfect_and_time_over() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.winner, Some(0));
    let record = *sim.win_history[0].last().expect("KO win recorded");
    assert!(!record.time_over);
    assert!(record.perfect);
    assert!(!record.clutch);

    // A damaged winner is neither perfect nor clutch.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].life = sim.fighters[0].life_max / 2;
    sim.fighters[1].life = sim.fighters[1].life_max / 2 - 100;
    sim.round.timer = 1;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.winner, Some(0));
    let record = *sim.win_history[0].last().expect("time over recorded");
    assert!(record.time_over);
    assert!(!record.perfect);
    assert!(!record.clutch);

    // A win at or below the clutch threshold is flagged.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].life = (sim.fighters[0].life_max as f32 * manato_sim::CLUTCH_THRESHOLD) as i32;
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    let record = *sim.win_history[0].last().expect("clutch recorded");
    assert!(record.clutch);
}

#[test]
fn mid_fill_drains_after_delay() {
    let root = mugen_root();
    let config = MatchConfig {
        mugen_root: root.clone(),
        p1: PathBuf::from("content/chars/kaen/kaen.def"),
        p2: PathBuf::from("content/chars/nival/nival.def"),
        stage: PathBuf::from("content/stages/training/stage1.def"),
        screenpack: Some(PathBuf::from("content/screenpacks/manato-grid/system.def")),
        rounds_to_win: 2,
        round_time: 99,
    };
    if !root
        .join("content/screenpacks/manato-grid/system.def")
        .exists()
    {
        eprintln!("skipping: bundled screenpack not present");
        return;
    }
    let mut options = MatchOptions::default();
    options.ai_level = 0;
    let mut sim = SimState::new(
        std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
        options.clone(),
    );
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }

    sim.fighters[1].life = 500;
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        sim.bars[1].mid_life > 0.9,
        "mid fill holds while the configured delay runs"
    );
    for _ in 0..80 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        (sim.bars[1].mid_life - 0.5).abs() < 0.02,
        "mid fill drains to the current life ratio, got {}",
        sim.bars[1].mid_life
    );
    assert!(sim.bars[1].mid_life >= 0.5 - f32::EPSILON);
}

#[test]
fn bg_controllers_update_screen_backgrounds() {
    use manato_formats::{BgCtrl, Controller};

    let layer = manato_formats::stage::BgLayer {
        visible: true,
        enabled: true,
        ..Default::default()
    };
    let mut states = vec![manato_sim::StageLayerState::from_layer(&layer)];
    let controls = [BgCtrl {
        layer: 0,
        controller: Controller {
            kind: "visible".into(),
            params: vec![("value".into(), "0".into())],
            line: 0,
        },
    }];
    manato_sim::update_bg_states(&controls, &mut states, 0, 1, 0);
    assert!(!states[0].visible, "BGCtrl should hide the layer");
}

/// Win classification derived from the Ikemen-GO black-box oracle
/// (`docs/oracle/assertions.md`, O-02..O-05). Each scenario mirrors an
/// oracle run with the same starting life values.
#[test]
fn oracle_round_win_classification() {
    // O-02: full-life KO => KO win, perfect, not time over.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    let record = *sim.win_history[0].last().expect("KO win");
    assert!(!record.time_over);
    assert!(record.perfect);
    assert!(!record.clutch);

    // O-04: exactly 10% life is a clutch win; 10.1% is not.
    for (life, clutch) in [(100, true), (101, false)] {
        let Some(mut sim) = fight_phase() else {
            return;
        };
        sim.fighters[0].life = life;
        sim.fighters[1].life = 0;
        sim.tick([PlayerInput::default(); 2]);
        let record = *sim.win_history[0].last().expect("KO win");
        assert!(!record.perfect, "life {life} is not perfect");
        assert_eq!(record.clutch, clutch, "life {life} clutch classification");
    }

    // O-03: a time-over win with full life is both time over and perfect.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].life = sim.fighters[0].life_max;
    sim.fighters[1].life = 500;
    sim.round.timer = 1;
    sim.tick([PlayerInput::default(); 2]);
    let record = *sim.win_history[0].last().expect("time-over win");
    assert!(record.time_over);
    assert!(record.perfect);
    assert!(!record.clutch);

    // O-08: equal life at time over is a draw, with no win recorded.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].life = 500;
    sim.fighters[1].life = 500;
    sim.round.timer = 1;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.winner, None);
    assert!(sim.round.time_over);
    assert_eq!(sim.round.wins, [0, 0]);
    assert!(sim.win_history[0].is_empty());
    assert!(sim.win_history[1].is_empty());
}

/// O-01: the oracle round timer is `seconds * 60` ticks.
#[test]
fn oracle_round_timer_is_sixty_ticks_per_second() {
    let root = mugen_root();
    let config = MatchConfig {
        mugen_root: root.clone(),
        p1: PathBuf::from("content/chars/kaen/kaen.def"),
        p2: PathBuf::from("content/chars/nival/nival.def"),
        stage: PathBuf::from("content/stages/training/stage1.def"),
        screenpack: None,
        rounds_to_win: 1,
        round_time: 5,
    };
    if !root.join(&config.stage).exists() {
        return;
    }
    let sim = SimState::new(
        std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
        MatchOptions {
            rounds_to_win: 1,
            round_time: 5,
            ai_level: 0,
        },
    );
    assert_eq!(sim.round.timer, 5 * 60);
}

#[test]
fn action_banners_queue_drop_and_expire() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "lifebaraction".into(),
        params: vec![("text".into(), "TEST".into()), ("time".into(), "3".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    assert_eq!(sim.actions[0].len(), 1);
    assert_eq!(sim.actions[0][0].text, "TEST");
    assert_eq!(sim.actions[0][0].time, 3);

    // At most four messages are visible; the oldest is dropped.
    for index in 0..3 {
        sim.push_action(0, format!("m{index}"), 3);
    }
    sim.push_action(0, "new", 3);
    assert_eq!(sim.actions[0].len(), 4);
    assert_eq!(sim.actions[0][0].text, "m0");
    assert_eq!(sim.actions[0][3].text, "new");

    // Expiry is tick-driven and rollback-safe.
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.actions[0].is_empty());
    sim.push_action(0, "deterministic", 4);
    let mut clone = sim.clone();
    for _ in 0..2 {
        sim.tick([PlayerInput::default(); 2]);
        clone.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.checksum(), clone.checksum());
}

#[test]
fn edge_triggers_track_camera_and_facing() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.camera.x = 0.0;
    sim.camera.y = 0.0;
    assert_eq!(manato_sim::trigger::num(&sim, 0, "LeftEdge"), -160.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "RightEdge"), 160.0);
    let top = manato_sim::trigger::num(&sim, 0, "TopEdge");
    let bottom = manato_sim::trigger::num(&sim, 0, "BottomEdge");
    assert!((bottom - top - 240.0).abs() < f32::EPSILON);

    sim.fighters[0].facing = 1;
    assert_eq!(manato_sim::trigger::num(&sim, 0, "BackEdge"), -160.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "FrontEdge"), 160.0);
    sim.fighters[0].facing = -1;
    assert_eq!(manato_sim::trigger::num(&sim, 0, "BackEdge"), 160.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "FrontEdge"), -160.0);
}

#[test]
fn projectile_timers_and_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].proj_contact_time = 0;
    sim.fighters[0].proj_hit_time = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjContactTime"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjHitTime"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjGuardedTime"), -1);
    assert!(manato_sim::trigger::int(&sim, 0, "ProjHit") != 0);
    sim.fighters[0].proj_guarded_time = 3;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjGuardedTime"), 3);
}

#[test]
fn no_bar_display_flag_hides_bars() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert!(!sim.bars_hidden());
    sim.fighters[0].assert_flags |= manato_sim::fighter::ASSERT_NOBARDISPLAY;
    assert!(sim.bars_hidden());
    sim.fighters[0].assert_flags = 0;
    assert!(!sim.bars_hidden());
    assert!(!sim.background_hidden());
    sim.fighters[1].assert_flags |= manato_sim::fighter::ASSERT_NOBG;
    assert!(sim.background_hidden());
    sim.fighters[1].assert_flags = manato_sim::fighter::ASSERT_NOFG;
    assert!(sim.foreground_hidden());
    assert!(!sim.background_hidden());
}

fn enter_ko(mut sim: SimState) -> SimState {
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    sim
}

#[test]
fn ko_slowdown_preserves_timing_and_input_lock() {
    let Some(sim) = fight_phase() else {
        return;
    };
    let mut sim = enter_ko(sim);
    assert_eq!(sim.round.slow_timer, 60);
    assert!(!sim.round.slow_disabled);
    assert!((sim.ko_slow_factor() - 0.25).abs() < 0.001);

    let start_state = sim.fighters[0].state_time;
    let start_phase = sim.round.phase_timer;
    for _ in 0..8 {
        sim.tick([PlayerInput::new(manato_core::DIR_RIGHT, manato_core::Buttons::X); 2]);
    }
    // Round timing is not slowed by the KO cadence.
    assert_eq!(sim.round.phase_timer, start_phase - 8);
    // Input is locked: the history records neutral input during KO.
    assert_eq!(
        sim.fighters[0].history.back().map(|frame| frame.input),
        Some(PlayerInput::default())
    );
    // The gameplay cadence is reduced (8 ticks produce fewer updates).
    let elapsed = sim.fighters[0].state_time - start_state;
    assert!(
        (1..8).contains(&elapsed),
        "expected reduced cadence, got {elapsed} updates"
    );
    assert_eq!(sim.round.slow_timer, 60 - 8);
}

#[test]
fn nokoslow_latches_and_restores_full_cadence() {
    let Some(sim) = fight_phase() else {
        return;
    };
    let mut sim = enter_ko(sim);
    let start_state = sim.fighters[0].state_time;
    sim.fighters[0].assert_flags |= manato_sim::fighter::ASSERT_NOKOSLOW;
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.round.slow_disabled, "NoKOSlow should latch");

    // Once latched, the flag no longer needs to be reasserted.
    for _ in 0..8 {
        sim.tick([PlayerInput::default(); 2]);
    }
    let elapsed = sim.fighters[0].state_time - start_state;
    assert!(
        elapsed >= 8,
        "NoKOSlow should run every tick, got {elapsed}"
    );
    assert_eq!(sim.round.phase_timer, 120 - 9);
}

#[test]
fn ko_slowdown_is_rollback_deterministic() {
    let Some(sim) = fight_phase() else {
        return;
    };
    let sim = enter_ko(sim);
    let mut a = sim.clone();
    let mut b = sim.clone();
    for _ in 0..33 {
        a.tick([PlayerInput::default(); 2]);
        b.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(a.checksum(), b.checksum());
    assert_eq!(a.round.slow_timer, b.round.slow_timer);
}

#[test]
fn ko_slow_factor_fades_toward_full_speed() {
    let Some(sim) = fight_phase() else {
        return;
    };
    let mut sim = enter_ko(sim);
    let start = sim.ko_slow_factor();
    sim.round.slow_timer = 0;
    let end = sim.ko_slow_factor();
    assert!((start - 0.25).abs() < 0.001);
    assert!((end - 1.0).abs() < 0.001);
    sim.round.slow_timer = 22;
    let mid = sim.ko_slow_factor();
    assert!(mid > start && mid < end, "fade should be monotonic: {mid}");
}

#[test]
fn noko_assert_prevents_ko_until_cleared() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = root.join("content/chars/kaen");
    if !source.join("common.cns").exists() {
        eprintln!("skipping: bundled character missing");
        return;
    }
    let dir = std::env::temp_dir().join(format!("manato-noko-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp character dir");
    for entry in std::fs::read_dir(&source).expect("read character dir") {
        let entry = entry.expect("dir entry");
        if entry.file_type().expect("file type").is_file() {
            std::fs::copy(entry.path(), dir.join(entry.file_name())).expect("copy character file");
        }
    }
    let common = dir.join("common.cns");
    let text = std::fs::read_to_string(&common).expect("read common.cns");
    let patched = text.replace(
        "[Statedef 10]",
        "[State 0, Test NoKo]\ntype = AssertSpecial\ntrigger1 = 1\nflag = noko\n\n[Statedef 10]",
    );
    std::fs::write(&common, patched).expect("patch common.cns");

    let config = MatchConfig {
        mugen_root: root.clone(),
        p1: PathBuf::from("content/chars/kaen/kaen.def"),
        p2: dir.join("kaen.def"),
        stage: PathBuf::from("content/stages/training/stage1.def"),
        screenpack: None,
        rounds_to_win: 2,
        round_time: 99,
    };
    let mut options = MatchOptions::default();
    options.ai_level = 0;
    let mut sim = SimState::new(
        std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
        options,
    );
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }
    sim.fighters[1].life = 0;
    for _ in 0..5 {
        sim.tick([PlayerInput::default(); 2]);
        assert_eq!(
            sim.round.phase,
            manato_sim::RoundPhase::Fight,
            "NoKo must keep the round alive"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nokosnd_flag_suppresses_round_end_sounds() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert!(!sim.ko_sounds_suppressed());
    sim.fighters[0].assert_flags |= manato_sim::fighter::ASSERT_NOKOSND;
    assert!(sim.ko_sounds_suppressed());
}

#[test]
fn oracle_kfm_state_200_hits_idle() {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = project.canonicalize().unwrap_or(project);
    let root = std::env::var_os("MANATO_MUGEN_ROOT")
        .or_else(|| std::env::var_os("MUGEN_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| project.parent().unwrap_or(&project).join("mugen"));
    let kfm = root.join("mugen-1.1b1/chars/kfm/kfm.def");
    let stage =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/stages/training/stage1.def");
    if !kfm.exists() || !stage.exists() {
        eprintln!("skipping: stock KFM or bundled stage missing");
        return;
    }
    let config = MatchConfig {
        mugen_root: root,
        p1: kfm.clone(),
        p2: kfm,
        stage,
        screenpack: None,
        rounds_to_win: 2,
        round_time: 99,
    };
    let mut options = MatchOptions::default();
    options.ai_level = 0;
    let mut sim = SimState::new(
        std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
        options,
    );
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }
    sim.fighters[0].pos.x = -16.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 16.0;
    sim.fighters[1].facing = -1;
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(200, &assets.characters[0], false);

    let mut hit = None;
    for index in 0..8 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].life < sim.fighters[1].life_max {
            hit = Some((index, sim.fighters[1].life, sim.fighters[1].state_no));
            break;
        }
    }
    let (index, life, state) = hit.expect("KFM state 200 should hit an idle opponent");
    // Ikemen-GO oracle O-09: contact within 4 ticks, 23 damage, state 5000.
    assert!(index <= 5, "contact took {index} ticks");
    assert_eq!(life, 977);
    assert_eq!(state, 5000);
    assert_eq!(sim.fighters[1].received_damage, 23);
    assert_eq!(sim.fighters[1].received_hits, 1);
    let contact_x = sim.fighters[1].pos.x;
    let mut transition = None;
    let mut min_life = sim.fighters[1].life;
    for tick in 0..30 {
        sim.tick([PlayerInput::default(); 2]);
        min_life = min_life.min(sim.fighters[1].life);
        if transition.is_none() && sim.fighters[1].state_no >= 5001 {
            transition = Some(tick + 1);
        }
    }
    // Oracle O-12: the get-hit state advances on the first tick after the
    // 8-tick hit-shake window (contact + 9 ticks).
    assert_eq!(transition, Some(9));
    // Oracle O-10: one activation, so life never drops below 977.
    assert_eq!(min_life, 977);
    // Oracle O-11: the defender is pushed away from the attacker.
    assert!(
        sim.fighters[1].pos.x > contact_x + 8.0,
        "defender should be pushed away, x={}",
        sim.fighters[1].pos.x
    );
}

fn kfm_close_match() -> Option<SimState> {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = project.canonicalize().unwrap_or(project);
    let root = std::env::var_os("MANATO_MUGEN_ROOT")
        .or_else(|| std::env::var_os("MUGEN_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| project.parent().unwrap_or(&project).join("mugen"));
    let kfm = root.join("mugen-1.1b1/chars/kfm/kfm.def");
    let stage = project.join("content/stages/training/stage1.def");
    if !kfm.exists() || !stage.exists() {
        eprintln!("skipping: stock KFM or bundled stage missing");
        return None;
    }
    let config = MatchConfig {
        mugen_root: root,
        p1: kfm.clone(),
        p2: kfm,
        stage,
        screenpack: None,
        rounds_to_win: 2,
        round_time: 99,
    };
    let mut options = MatchOptions::default();
    options.ai_level = 0;
    let mut sim = SimState::new(
        std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
        options,
    );
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }
    Some(sim)
}

#[test]
fn oracle_kfm_guard_flow() {
    let Some(mut sim) = kfm_close_match() else {
        return;
    };
    sim.fighters[0].pos.x = -16.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 16.0;
    sim.fighters[1].facing = -1;
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(200, &assets.characters[0], false);

    let guard_input = PlayerInput::new(manato_core::DIR_RIGHT, 0);
    let mut guard = None;
    for index in 0..12 {
        sim.tick([PlayerInput::default(), guard_input]);
        if sim.fighters[0].attack.move_guarded {
            guard = Some(index);
            break;
        }
    }
    let index = guard.expect("KFM state 200 should be guarded while holding back");
    assert!(index <= 6, "guard contact took {index} ticks");
    let defender = &sim.fighters[1];
    assert_eq!(defender.life, defender.life_max, "guard damage is zero");
    assert_eq!(defender.state_no, 150);
    assert_eq!(defender.received_damage, 0);
    assert!(!sim.fighters[0].attack.move_hit);
    let start_x = defender.pos.x;
    let mut transition = None;
    for tick in 0..12 {
        sim.tick([PlayerInput::default(), guard_input]);
        if transition.is_none() && sim.fighters[1].state_no >= 151 {
            transition = Some(tick + 1);
        }
    }
    // Ikemen-GO oracle O-14: guard slide begins 9 ticks after contact.
    assert_eq!(transition, Some(9));
    assert_eq!(sim.fighters[1].state_no, 151);
    assert!(
        sim.fighters[1].pos.x > start_x + 2.0,
        "guarded defender should slide away, x={}",
        sim.fighters[1].pos.x
    );
}

#[test]
fn oracle_kfm_launch_flow() {
    let Some(mut sim) = kfm_close_match() else {
        return;
    };
    sim.fighters[0].pos.x = -16.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 16.0;
    sim.fighters[1].facing = -1;
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(440, &assets.characters[0], false);

    let mut hit = None;
    for index in 0..15 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].life < sim.fighters[1].life_max {
            hit = Some((index, sim.fighters[1].life, sim.fighters[1].state_no));
            break;
        }
    }
    let (index, life, state) = hit.expect("KFM state 440 should launch an idle opponent");
    eprintln!("launch hit index={index} life={life} state={state}");
    assert_eq!(life, 928);
    assert_eq!(state, 5070);
    assert_eq!(sim.fighters[1].received_damage, 72);
    let mut transition = None;
    for tick in 0..25 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].state_no >= 5071 {
            transition = Some(tick + 1);
            break;
        }
    }
    // Ikemen-GO oracle O-16: trip state 5070 advances to 5071 on contact + 13.
    assert_eq!(transition, Some(13));
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.fighters[1].pos.y < 0.0, "trip should become airborne");
    assert!(sim.fighters[1].vel.y < 0.0, "trip should launch upward");
}

#[test]
fn oracle_kfm_juggle_flow() {
    let Some(mut sim) = kfm_close_match() else {
        return;
    };
    sim.fighters[0].pos.x = -16.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 16.0;
    sim.fighters[1].facing = -1;
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(440, &assets.characters[0], false);

    let mut first = None;
    for _ in 0..15 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].life < sim.fighters[1].life_max {
            first = Some(sim.fighters[1].life);
            break;
        }
    }
    assert_eq!(first, Some(928));
    assert_eq!(sim.fighters[1].state_no, 5070);
    // O-17: KFM state 440 has Statedef juggle = 7, consumed on the fall hit.
    assert_eq!(sim.fighters[1].juggle_points, 8);

    let mut airborne = false;
    for _ in 0..20 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].state_no >= 5071 {
            airborne = true;
            break;
        }
    }
    assert!(airborne, "trip should transition to an airborne state");
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(210, &assets.characters[0], false);

    let mut second = None;
    for _ in 0..25 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].life < 928 {
            second = Some(sim.fighters[1].life);
            break;
        }
    }
    assert_eq!(second, Some(890), "state 210 should deal 38 on the air hit");
    assert_eq!(sim.fighters[1].state_no, 5020);
    // State 210 has Statedef juggle = 4, consuming 4 more points.
    assert_eq!(sim.fighters[1].juggle_points, 4);
}

#[test]
fn restart_match_resets_rounds_wins_and_power() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.round.wins = [2, 1];
    sim.round.number = 3;
    sim.finished = true;
    sim.fighters[0].power = 1500.0;
    sim.win_history[0].push(manato_sim::WinRecord {
        time_over: false,
        perfect: true,
        clutch: false,
    });
    sim.restart_match();
    assert_eq!(sim.round.number, 1);
    assert_eq!(sim.round.wins, [0, 0]);
    assert!(!sim.finished);
    assert!(sim.win_history[0].is_empty());
    assert_eq!(sim.fighters[0].power, 0.0);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Intro);
    assert_eq!(sim.round.timer, sim.round.round_time * 60);
}

#[test]
fn game_make_anim_spawns_fightfx_effect() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let p2 = sim.fighters[1].pos;
    let control = manato_formats::Controller {
        kind: "gamemakeanim".into(),
        params: vec![
            ("anim".into(), "40".into()),
            ("pos".into(), "10, -20".into()),
            ("postype".into(), "p2".into()),
            ("sprpriority".into(), "3".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let effect = sim
        .effects
        .last()
        .expect("GameMakeAnim should spawn an effect");
    assert_eq!(effect.source, manato_sim::EffectSource::FightFx);
    assert_eq!(effect.anim, 40);
    assert_eq!(effect.sprite_priority, 3);
    assert!((effect.pos.x - (p2.x + 10.0)).abs() < 0.001);
    assert!((effect.pos.y - (p2.y - 20.0)).abs() < 0.001);
}

#[test]
fn make_dust_spawns_behind_owner() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "makedust".into(),
        params: vec![("pos".into(), "6, -2".into())],
        line: 0,
    };
    let owner_x = sim.fighters[0].pos.x;
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let effect = sim.effects.last().expect("MakeDust should spawn an effect");
    assert_eq!(effect.source, manato_sim::EffectSource::FightFx);
    assert_eq!(effect.anim, 120);
    assert!(effect.behind);
    assert_eq!(effect.owner, 0);
    assert!((effect.pos.x - (owner_x + 6.0)).abs() < 0.001);
}

#[test]
fn get_hitvar_and_hit_shake_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.pausetime = (5, 5);
    let origin = sim.fighters[0].pos;
    let facing = sim.fighters[0].facing;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, facing, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(damage)"), 20);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitShakeOver"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveHit"), 1);
    for _ in 0..6 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitShakeOver"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProj"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumExplod"), 0);
}

#[test]
fn common_trigger_expressions() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].vars[0] = 7;
    sim.fighters[0].fvars[0] = 2.5;
    sim.fighters[0].sysvars[0] = 9;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Var(0)"), 7);
    assert!((manato_sim::trigger::num(&sim, 0, "FVar(0)") - 2.5).abs() < 0.001);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "SysVar(0)"), 9);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Const(data.life)"), 1000);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "IfElse(1, 10, 20)"), 10);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Min(3, 5)"), 3);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Max(3, 5)"), 5);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Abs(-4)"), 4);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Ceil(1.2)"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Floor(1.8)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumEnemy"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumPartner"), 0);
}

#[test]
fn assert_special_flags_cover_supported_names() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let cases = [
        ("invisible", manato_sim::fighter::ASSERT_INVISIBLE),
        ("noshadow", manato_sim::fighter::ASSERT_NOSHADOW),
        ("nowalk", manato_sim::fighter::ASSERT_NOWALK),
        ("noairguard", manato_sim::fighter::ASSERT_NOAIRGUARD),
        ("nostandguard", manato_sim::fighter::ASSERT_NOSTANDGUARD),
        ("nocrouchguard", manato_sim::fighter::ASSERT_NOCROUCHGUARD),
        ("unguardable", manato_sim::fighter::ASSERT_UNGUARDABLE),
        ("nojugglecheck", manato_sim::fighter::ASSERT_NOJUGGLECHECK),
        ("noautoturn", manato_sim::fighter::ASSERT_NOAUTOTURN),
        ("intro", manato_sim::fighter::ASSERT_INTRO),
        ("nobg", manato_sim::fighter::ASSERT_NOBG),
        ("nofg", manato_sim::fighter::ASSERT_NOFG),
        ("noko", manato_sim::fighter::ASSERT_NOKO),
        ("nokoslow", manato_sim::fighter::ASSERT_NOKOSLOW),
        ("nokosnd", manato_sim::fighter::ASSERT_NOKOSND),
        ("nomusic", manato_sim::fighter::ASSERT_NOMUSIC),
        ("globalnoshadow", manato_sim::fighter::ASSERT_GLOBALNOSHADOW),
        ("nobardisplay", manato_sim::fighter::ASSERT_NOBARDISPLAY),
        ("roundnotover", manato_sim::fighter::ASSERT_ROUNDNOTOVER),
    ];
    for (name, flag) in cases {
        sim.fighters[0].assert_flags = 0;
        let control = manato_formats::Controller {
            kind: "assertspecial".into(),
            params: vec![
                ("type".into(), "AssertSpecial".into()),
                ("flag".into(), name.into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(&mut sim, 0, &control, &assets);
        assert_eq!(
            sim.fighters[0].assert_flags & flag,
            flag,
            "AssertSpecial {name} should set its flag"
        );
    }
}

#[test]
fn hit_filters_gate_real_attacks() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].pos.x = -16.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 16.0;
    sim.fighters[1].facing = -1;
    let assets = sim.assets.clone();

    sim.fighters[1].hit.not_hit_by = Some(("SCA, NA".into(), 30));
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    for _ in 0..20 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(
        sim.fighters[1].life, sim.fighters[1].life_max,
        "NotHitBy should block matching normal attacks"
    );

    sim.fighters[1].hit.not_hit_by = None;
    sim.fighters[1].hit.hit_by = Some(("SCA, SA".into(), 30));
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    for _ in 0..20 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(
        sim.fighters[1].life, sim.fighters[1].life_max,
        "HitBy whitelist should reject non-matching attacks"
    );

    sim.fighters[1].hit.hit_by = None;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    for _ in 0..20 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].life < sim.fighters[1].life_max {
            break;
        }
    }
    assert!(
        sim.fighters[1].life < sim.fighters[1].life_max,
        "normal attack should hit once the filters expire"
    );
}

#[test]
fn redirect_triggers_resolve_helper_target_and_enemy() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("type".into(), "Helper".into()),
            ("stateno".into(), "0".into()),
            ("id".into(), "7".into()),
            ("pos".into(), "10,0".into()),
            ("postype".into(), "p1".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    let helper_index = 2;
    let parent_var = manato_formats::Controller {
        kind: "parentvarset".into(),
        params: vec![
            ("type".into(), "ParentVarSet".into()),
            ("v".into(), "5".into()),
            ("value".into(), "42".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, helper_index, &parent_var, &assets);

    let p1_state = sim.fighters[0].state_no;
    let p2_life = sim.fighters[1].life;
    assert_eq!(
        manato_sim::trigger::int(&sim, helper_index, "root, stateno"),
        p1_state
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, helper_index, "parent, stateno"),
        p1_state
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, helper_index, "parent, Var(5)"),
        42
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "p2, life"), p2_life);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "enemy, life"), p2_life);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "helper(7), stateno"),
        sim.fighters[helper_index].state_no
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumHelper"), 1);

    sim.fighters[0].target = Some(1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "target, life"), p2_life);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumTarget"), 1);
}

#[test]
fn bare_numproj_tracks_projectile_lifetime() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(1000, &assets.characters[0], false);
    let mut spawned = false;
    for _ in 0..120 {
        sim.tick([PlayerInput::default(); 2]);
        if !sim.projectiles.is_empty() {
            spawned = true;
            break;
        }
    }
    assert!(spawned, "state 1000 should spawn a projectile");
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProj"), 1);
    for _ in 0..130 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProj"), 0);
}

#[test]
fn projectile_hit_limit_consumes_hits_and_expires() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].pos.x = -16.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 16.0;
    sim.fighters[1].facing = -1;
    let mut hitdef = manato_sim::hitdef::HitDef::default();
    hitdef.damage = 10;
    hitdef.hitflag = "MAF".into();
    hitdef.ground_hittime = 12;
    hitdef.attr.state_types = "S".into();
    sim.projectiles.push(manato_sim::projectile::Projectile {
        owner: 0,
        id: 0,
        anim: 1500,
        pos: sim.fighters[1].pos,
        vel: manato_core::Vec2::ZERO,
        accel: manato_core::Vec2::ZERO,
        facing: 1,
        time: 0,
        remove_time: 5,
        hits: 2,
        hitdef,
    });
    let start = sim.fighters[1].life;
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(
        sim.fighters[1].life,
        start - 10,
        "first projectile hit lands"
    );
    if let Some(projectile) = sim.projectiles.first() {
        assert_eq!(projectile.hits, 1, "hit count is consumed");
    }
    for _ in 0..8 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.projectiles.is_empty(), "projectile expires by lifetime");
}

#[test]
fn target_controllers_modify_bound_player() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].target = Some(1);
    let life = sim.fighters[1].life;
    let power = sim.fighters[1].power;

    let life_add = manato_formats::Controller {
        kind: "targetlifeadd".into(),
        params: vec![("value".into(), "-100".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &life_add, &assets);
    assert_eq!(sim.fighters[1].life, life - 100);

    let power_add = manato_formats::Controller {
        kind: "targetpoweradd".into(),
        params: vec![("value".into(), "250".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &power_add, &assets);
    assert!((sim.fighters[1].power - (power + 250.0)).abs() < 0.001);

    let vel_set = manato_formats::Controller {
        kind: "targetvelset".into(),
        params: vec![("x".into(), "3".into()), ("y".into(), "-2".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &vel_set, &assets);
    assert!((sim.fighters[1].vel.x - -3.0).abs() < 0.001);
    assert!((sim.fighters[1].vel.y - -2.0).abs() < 0.001);

    let facing = manato_formats::Controller {
        kind: "targetfacing".into(),
        params: vec![("value".into(), "1".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &facing, &assets);
    assert_eq!(sim.fighters[1].facing, sim.fighters[0].facing);

    let state = manato_formats::Controller {
        kind: "targetstate".into(),
        params: vec![("value".into(), "0".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &state, &assets);
    assert_eq!(sim.fighters[1].state_no, 0);

    let bind = manato_formats::Controller {
        kind: "targetbind".into(),
        params: vec![("pos".into(), "10, -20".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &bind, &assets);
    assert!((sim.fighters[1].pos.x - (sim.fighters[0].pos.x + 10.0)).abs() < 0.001);
    assert!((sim.fighters[1].pos.y - (sim.fighters[0].pos.y - 20.0)).abs() < 0.001);

    let drop = manato_formats::Controller {
        kind: "targetdrop".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &drop, &assets);
    assert_eq!(sim.fighters[0].target, None);
}

#[test]
fn target_bind_expires_and_can_be_rebound() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].target = Some(1);
    let bind = manato_formats::Controller {
        kind: "targetbind".into(),
        params: vec![
            ("pos".into(), "10, -20".into()),
            ("time".into(), "2".into()),
        ],
        line: 0,
    };
    sim.fighters[0].pos = manato_core::Vec2::new(-50.0, 0.0);
    manato_sim::controller::execute(&mut sim, 0, &bind, &assets);
    assert!((sim.fighters[1].pos.x - -40.0).abs() < 0.001);
    assert!(sim.fighters[0].target_bind.is_some());

    sim.fighters[0].pos.x = 0.0;
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.fighters[1].pos.x - 10.0).abs() < 0.001);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.fighters[0].target_bind.is_none());

    sim.fighters[0].pos.x = 100.0;
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        (sim.fighters[1].pos.x - 110.0).abs() > 1.0,
        "expired binding must not follow the attacker"
    );

    // Re-binding restarts the duration.
    manato_sim::controller::execute(&mut sim, 0, &bind, &assets);
    assert!(sim.fighters[0].target_bind.is_some());
    sim.fighters[0].pos.x = 120.0;
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        sim.fighters[1].pos.x > 120.0,
        "rebound target should follow the attacker again"
    );
}

#[test]
fn helper_owned_projectile_damages_opponent() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    sim.fighters[0].pos.x = -60.0;
    sim.fighters[1].pos.x = 60.0;
    let assets = sim.assets.clone();
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("type".into(), "Helper".into()),
            ("stateno".into(), "1000".into()),
            ("id".into(), "9".into()),
            ("pos".into(), "0,0".into()),
            ("postype".into(), "p1".into()),
            ("removetime".into(), "180".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    let helper_index = 2;
    let mut owned = false;
    for _ in 0..150 {
        sim.tick([PlayerInput::default(); 2]);
        owned |= sim
            .projectiles
            .iter()
            .any(|projectile| projectile.owner == helper_index);
        if sim.fighters[1].life < sim.fighters[1].life_max {
            break;
        }
    }
    assert!(
        owned,
        "helper should spawn a projectile with owner = helper"
    );
    assert!(
        sim.fighters[1].life < sim.fighters[1].life_max,
        "helper-owned projectile should damage the opponent"
    );
}

#[test]
fn modify_projectile_updates_live_projectile() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(1000, &assets.characters[0], false);
    for _ in 0..120 {
        sim.tick([PlayerInput::default(); 2]);
        if !sim.projectiles.is_empty() {
            break;
        }
    }
    let before = sim.projectiles.first().expect("projectile spawns").clone();
    assert!(before.vel.x > 0.0);

    let modify = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("velocity".into(), "0, 0".into()),
            ("removetime".into(), "3".into()),
            ("hits".into(), "2".into()),
            ("damage".into(), "40".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    let projectile = &sim.projectiles[0];
    assert_eq!(projectile.vel.x, 0.0);
    assert_eq!(projectile.remove_time, 3);
    assert_eq!(projectile.hits, 2);
    assert_eq!(projectile.hitdef.damage, 40);

    // An explicit id that matches nothing leaves projectiles untouched.
    let id = projectile.id;
    let miss = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("id".into(), (id + 1000).to_string()),
            ("velocity".into(), "9, 9".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &miss, &assets);
    assert_eq!(sim.projectiles[0].vel.x, 0.0);

    for _ in 0..5 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.projectiles.is_empty(), "modified remove time expires");
}

#[test]
fn var_random_is_bounded_and_deterministic() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let int_control = manato_formats::Controller {
        kind: "varrandom".into(),
        params: vec![("v".into(), "0".into()), ("range".into(), "5, 9".into())],
        line: 0,
    };
    for _ in 0..50 {
        manato_sim::controller::execute(&mut sim, 0, &int_control, &assets);
        assert!((5..=9).contains(&sim.fighters[0].vars[0]));
    }

    let float_control = manato_formats::Controller {
        kind: "varrandom".into(),
        params: vec![
            ("v".into(), "60".into()),
            ("range".into(), "150, 250".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &float_control, &assets);
    assert!((1.5..=2.5).contains(&sim.fighters[0].fvars[0]));

    let mut a = sim.clone();
    let mut b = sim.clone();
    for _ in 0..20 {
        manato_sim::controller::execute(&mut a, 0, &int_control, &assets);
        manato_sim::controller::execute(&mut b, 0, &int_control, &assets);
    }
    assert_eq!(a.checksum(), b.checksum());
    assert_eq!(a.fighters[0].vars[0], b.fighters[0].vars[0]);
}

#[test]
fn modify_and_remove_explod_target_the_newest_matching_id() {
    let Some(mut sim) = load(
        "content/chars/kaen/kaen.def",
        "content/chars/nival/nival.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    let explod = manato_formats::Controller {
        kind: "explod".into(),
        params: vec![
            ("anim".into(), "10".into()),
            ("id".into(), "5".into()),
            ("pos".into(), "4, 8".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &explod, &assets);
    assert_eq!(sim.effects.len(), 1);

    let modify = manato_formats::Controller {
        kind: "modifyexplod".into(),
        params: vec![
            ("id".into(), "5".into()),
            ("pos".into(), "30, -10".into()),
            ("scale".into(), "2, 3".into()),
            ("vel".into(), "1, -2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    let effect = sim.effects.last().expect("explod still exists");
    assert!((effect.pos.x - 30.0).abs() < 0.001);
    assert!((effect.pos.y - -10.0).abs() < 0.001);
    assert!((effect.scale.x - 2.0).abs() < 0.001);
    assert!((effect.scale.y - 3.0).abs() < 0.001);
    assert!((effect.vel.x - 1.0).abs() < 0.001);
    assert!((effect.vel.y - -2.0).abs() < 0.001);

    // A different id leaves the effect alone.
    let unrelated = manato_formats::Controller {
        kind: "modifyexplod".into(),
        params: vec![("id".into(), "6".into()), ("pos".into(), "99, 99".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &unrelated, &assets);
    assert!((sim.effects[0].pos.x - 30.0).abs() < 0.001);

    let remove = manato_formats::Controller {
        kind: "removeexplod".into(),
        params: vec![("id".into(), "5".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &remove, &assets);
    assert!(sim.effects.is_empty());
}

#[test]
fn persistent_and_ignorehitpause_control_execution() {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = project.canonicalize().unwrap_or(project);
    let source = project.join("content/chars/kaen");
    if !source.join("common.cns").exists() {
        eprintln!("skipping: bundled character missing");
        return;
    }
    let dir = std::env::temp_dir().join(format!("manato-persistent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp character dir");
    for entry in std::fs::read_dir(&source).expect("read character dir") {
        let entry = entry.expect("dir entry");
        if entry.file_type().expect("file type").is_file() {
            std::fs::copy(entry.path(), dir.join(entry.file_name())).expect("copy character file");
        }
    }
    let common = dir.join("common.cns");
    let mut text = std::fs::read_to_string(&common).expect("read common.cns");
    text.push_str(
        "\n[Statedef 900]\ntype = S\nphysics = S\nanim = 0\nctrl = 0\n\
         \n[State 900, Once]\ntype = VarAdd\ntrigger1 = 1\nv = 0\nvalue = 1\npersistent = 0\n\
         \n[State 900, Normal]\ntype = VarAdd\ntrigger1 = 1\nv = 1\nvalue = 10\npersistent = 1\n\
         \n[State 900, Paused]\ntype = VarAdd\ntrigger1 = 1\nv = 2\nvalue = 100\npersistent = 1\nignorehitpause = 1\n",
    );
    std::fs::write(&common, text).expect("patch common.cns");

    let config = MatchConfig {
        mugen_root: project.clone(),
        p1: dir.join("kaen.def"),
        p2: PathBuf::from("content/chars/nival/nival.def"),
        stage: PathBuf::from("content/stages/training/stage1.def"),
        screenpack: None,
        rounds_to_win: 2,
        round_time: 99,
    };
    let mut options = MatchOptions::default();
    options.ai_level = 0;
    let mut sim = SimState::new(
        std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
        options,
    );
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(900, &assets.characters[0], false);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].vars[0], 1, "persistent=0 runs once");
    assert_eq!(sim.fighters[0].vars[1], 10);
    assert_eq!(sim.fighters[0].vars[2], 100);

    sim.fighters[0].hit.hit_pause = 2;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].vars[1], 10, "normal controller pauses");
    assert_eq!(sim.fighters[0].vars[2], 200, "ignorehitpause keeps running");
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].vars[2], 300);

    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].vars[1], 20, "normal controller resumes");
    assert_eq!(sim.fighters[0].vars[2], 400);
    assert_eq!(sim.fighters[0].vars[0], 1, "persistent=0 never repeats");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn state_and_animation_controllers_round_trip() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let turn = manato_formats::Controller {
        kind: "turn".into(),
        params: vec![],
        line: 0,
    };
    let facing = sim.fighters[0].facing;
    manato_sim::controller::execute(&mut sim, 0, &turn, &assets);
    assert_eq!(sim.fighters[0].facing, -facing);

    let anim = manato_formats::Controller {
        kind: "changeanim".into(),
        params: vec![("value".into(), "200".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &anim, &assets);
    assert_eq!(sim.fighters[0].anim_no, 200);

    let state = manato_formats::Controller {
        kind: "changestate".into(),
        params: vec![("value".into(), "200".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &state, &assets);
    assert_eq!(sim.fighters[0].state_no, 200);
    assert_eq!(sim.fighters[0].prev_state_no, 0);

    let self_state = manato_formats::Controller {
        kind: "selfstate".into(),
        params: vec![("value".into(), "0".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &self_state, &assets);
    assert_eq!(sim.fighters[0].state_no, 0);

    // ChangeAnim2 is the same engine path but resolves the owner's palette in
    // full content; here it must at least change the animation number.
    let anim2 = manato_formats::Controller {
        kind: "changeanim2".into(),
        params: vec![("value".into(), "10".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &anim2, &assets);
    assert_eq!(sim.fighters[0].anim_no, 10);
}

#[test]
fn pause_superpause_and_envshake_controllers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    let pause = manato_formats::Controller {
        kind: "pause".into(),
        params: vec![("time".into(), "5".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &pause, &assets);
    assert_eq!(sim.super_pause, 5);
    assert_eq!(sim.super_pause_move_time, 0);

    let super_pause = manato_formats::Controller {
        kind: "superpause".into(),
        params: vec![
            ("time".into(), "10".into()),
            ("movetime".into(), "3".into()),
            ("sound".into(), "5, 2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &super_pause, &assets);
    assert_eq!(sim.super_pause, 10);
    assert_eq!(sim.super_pause_move_time, 3);
    assert!(sim
        .sounds
        .iter()
        .any(|sound| sound.group == 5 && sound.index == 2));

    let shake = manato_formats::Controller {
        kind: "envshake".into(),
        params: vec![
            ("time".into(), "20".into()),
            ("freq".into(), "90".into()),
            ("ampl".into(), "-8".into()),
            ("phase".into(), "1.5".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &shake, &assets);
    assert_eq!(sim.env_shake.time, 20);
    assert!((sim.env_shake.freq - 90.0).abs() < 0.001);
    assert!((sim.env_shake.ampl - -8.0).abs() < 0.001);
    assert!((sim.env_shake.phase - 1.5).abs() < 0.001);
}

#[test]
fn width_screenbound_and_playerpush_controllers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    // Pushbox separation happens by default.
    sim.fighters[0].pos.x = 0.0;
    sim.fighters[1].pos.x = 0.0;
    let before = (sim.fighters[0].pos.x, sim.fighters[1].pos.x);
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        (sim.fighters[0].pos.x - before.0).abs() > 0.1
            || (sim.fighters[1].pos.x - before.1).abs() > 0.1,
        "overlapping pushboxes should separate"
    );

    // PlayerPush = 0 disables separation for that fighter.
    sim.fighters[0].pos.x = 0.0;
    sim.fighters[1].pos.x = 0.0;
    let push = manato_formats::Controller {
        kind: "playerpush".into(),
        params: vec![("value".into(), "0".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &push, &assets);
    assert!(!sim.fighters[0].player_push);
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.fighters[0].pos.x - 0.0).abs() < 0.001);
    assert!((sim.fighters[1].pos.x - 0.0).abs() < 0.001);

    let width = manato_formats::Controller {
        kind: "width".into(),
        params: vec![("edge".into(), "-20, 30".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &width, &assets);
    assert_eq!(sim.fighters[0].width_override, Some((-20.0, 30.0)));
    assert!((sim.fighters[0].width_back - -20.0).abs() < 0.001);
    assert!((sim.fighters[0].width_front - 30.0).abs() < 0.001);

    // Disabling vertical screen bounds persists until changed.
    let bounds = manato_formats::Controller {
        kind: "screenbound".into(),
        params: vec![("value".into(), "1, 0".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &bounds, &assets);
    assert_eq!(sim.fighters[0].screen_bound, (true, false));

    let stage_left = sim.assets.stage.left_bound;
    sim.fighters[0].player_push = true;
    sim.fighters[0].pos.x = stage_left - 100.0;
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        sim.fighters[0].pos.x > stage_left - 100.0,
        "screen bound should clamp horizontal position"
    );
}

#[test]
fn special_states_negative_three_two_one_run_each_tick() {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = project.canonicalize().unwrap_or(project);
    let source = project.join("content/chars/kaen");
    if !source.join("common.cns").exists() {
        eprintln!("skipping: bundled character missing");
        return;
    }
    let dir = std::env::temp_dir().join(format!("manato-special-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp character dir");
    for entry in std::fs::read_dir(&source).expect("read character dir") {
        let entry = entry.expect("dir entry");
        if entry.file_type().expect("file type").is_file() {
            std::fs::copy(entry.path(), dir.join(entry.file_name())).expect("copy character file");
        }
    }
    let common = dir.join("common.cns");
    let mut text = std::fs::read_to_string(&common).expect("read common.cns");
    text.push_str(
        "\n[Statedef -3]\n[State -3, Tick]\ntype = VarAdd\ntrigger1 = 1\nv = 3\nvalue = 1\n\
         \n[Statedef -2]\n[State -2, Tick]\ntype = VarAdd\ntrigger1 = 1\nv = 4\nvalue = 1\n",
    );
    std::fs::write(&common, text).expect("patch common.cns");
    // Kaen already defines state -1 in kaen.cns, so extend that state in place.
    let states = dir.join("kaen.cns");
    let text = std::fs::read_to_string(&states).expect("read kaen.cns");
    let text = text.replacen(
        "[Statedef -1]",
        "[Statedef -1]\n[State -1, Oracle Tick]\ntype = VarAdd\ntrigger1 = 1\nv = 5\nvalue = 1",
        1,
    );
    std::fs::write(&states, text).expect("patch kaen.cns");

    let config = MatchConfig {
        mugen_root: project.clone(),
        p1: dir.join("kaen.def"),
        p2: PathBuf::from("content/chars/nival/nival.def"),
        stage: PathBuf::from("content/stages/training/stage1.def"),
        screenpack: None,
        rounds_to_win: 2,
        round_time: 99,
    };
    let mut options = MatchOptions::default();
    options.ai_level = 0;
    let mut sim = SimState::new(
        std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
        options,
    );
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }
    let base = [
        sim.fighters[0].vars[3],
        sim.fighters[0].vars[4],
        sim.fighters[0].vars[5],
    ];
    for _ in 0..5 {
        sim.tick([PlayerInput::default(); 2]);
    }
    for (index, slot) in [3usize, 4, 5].into_iter().enumerate() {
        assert_eq!(
            sim.fighters[0].vars[slot],
            base[index] + 5,
            "state -{} should run every tick",
            3 - index
        );
    }

    // Special states also run while the round is in KO control lock.
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    let before_ko = [
        sim.fighters[0].vars[3],
        sim.fighters[0].vars[4],
        sim.fighters[0].vars[5],
    ];
    sim.tick([PlayerInput::default(); 2]);
    for (index, slot) in [3usize, 4, 5].into_iter().enumerate() {
        assert_eq!(
            sim.fighters[0].vars[slot],
            before_ko[index] + 1,
            "state -{} should run during KO",
            3 - index
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn command_matching_handles_motion_and_button_windows() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    // Simple button command: active on the press tick only.
    sim.tick([
        PlayerInput::new(0, manato_core::Buttons::X),
        PlayerInput::default(),
    ]);
    assert!(manato_sim::trigger::command_active(&sim, 0, "x"));
    assert!(!manato_sim::trigger::command_active(&sim, 0, "y"));

    // Motion command `~D, DF, F, x` (FirePalm).
    let sequence = [
        (manato_core::DIR_DOWN, 0),
        (manato_core::DIR_DOWN | manato_core::DIR_RIGHT, 0),
        (manato_core::DIR_RIGHT, 0),
        (manato_core::DIR_RIGHT, manato_core::Buttons::X),
    ];
    for (dir, buttons) in sequence {
        sim.tick([PlayerInput::new(dir, buttons), PlayerInput::default()]);
    }
    assert!(
        manato_sim::trigger::command_active(&sim, 0, "FirePalm"),
        "down, down-forward, forward, x should match the FirePalm motion"
    );
    assert!(!manato_sim::trigger::command_active(&sim, 0, "BlazingRush"));

    // Two-button command `~D, DF, F, x+y` (InfernoStorm).
    let sequence = [
        (manato_core::DIR_DOWN, 0),
        (manato_core::DIR_DOWN | manato_core::DIR_RIGHT, 0),
        (manato_core::DIR_RIGHT, 0),
        (
            manato_core::DIR_RIGHT,
            manato_core::Buttons::X | manato_core::Buttons::Y,
        ),
    ];
    for (dir, buttons) in sequence {
        sim.tick([PlayerInput::new(dir, buttons), PlayerInput::default()]);
    }
    assert!(
        manato_sim::trigger::command_active(&sim, 0, "InfernoStorm"),
        "x+y should match the two-button command"
    );

    // Commands expire once their buffer window elapses.
    for _ in 0..40 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(!manato_sim::trigger::command_active(&sim, 0, "FirePalm"));
    assert!(!manato_sim::trigger::command_active(&sim, 0, "x"));
}

#[test]
fn animation_and_position_triggers_track_state() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Anim"), 200);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElem"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimTime"), 11);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElemTime(1)"), 1);
    assert!(manato_sim::trigger::int(&sim, 0, "AnimExist(200)") != 0);
    assert!(manato_sim::trigger::int(&sim, 0, "SelfAnimExist(200)") != 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimExist(9999)"), 0);

    // Frame 0 lasts 3 ticks, then frame 1 starts.
    for _ in 0..2 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElem"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElemTime(2)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElemTime(1)"), 3);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimTime"), 9);

    sim.fighters[0].pos = manato_core::Vec2::new(12.5, -4.0);
    sim.fighters[0].vel = manato_core::Vec2::new(-1.5, 2.0);
    sim.fighters[0].facing = 1;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Pos X"), 12);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Pos Y"), -4);
    assert!((manato_sim::trigger::num(&sim, 0, "Vel X") - -1.5).abs() < 0.001);
    assert!((manato_sim::trigger::num(&sim, 0, "Vel Y") - 2.0).abs() < 0.001);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Facing"), 1);

    // `Vel X` is facing-relative; `ScreenVel X` stays in screen space.
    sim.fighters[0].facing = -1;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Facing"), -1);
    assert!((manato_sim::trigger::num(&sim, 0, "Vel X") - 1.5).abs() < 0.001);
    let screen = sim.screen_pos(sim.fighters[0].pos);
    assert!((manato_sim::trigger::num(&sim, 0, "ScreenPos X") - screen.x).abs() < 0.001);
    assert!((manato_sim::trigger::num(&sim, 0, "ScreenPos Y") - screen.y).abs() < 0.001);
    assert!((manato_sim::trigger::num(&sim, 0, "ScreenVel X") - -1.5).abs() < 0.001);
}

#[test]
fn ctrl_state_type_and_move_type_controllers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    let ctrl_off = manato_formats::Controller {
        kind: "ctrlset".into(),
        params: vec![("value".into(), "0".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &ctrl_off, &assets);
    assert!(!sim.fighters[0].ctrl);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Ctrl"), 0);

    let ctrl_on = manato_formats::Controller {
        kind: "ctrlset".into(),
        params: vec![("value".into(), "1".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &ctrl_on, &assets);
    assert!(sim.fighters[0].ctrl);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Ctrl"), 1);

    let types = manato_formats::Controller {
        kind: "statetypeset".into(),
        params: vec![
            ("statetype".into(), "A".into()),
            ("movetype".into(), "H".into()),
            ("physics".into(), "N".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &types, &assets);
    assert_eq!(
        sim.fighters[0].state_type,
        manato_sim::fighter::StateType::Air
    );
    assert_eq!(
        sim.fighters[0].move_type,
        manato_sim::fighter::MoveType::Hit
    );
    assert_eq!(sim.fighters[0].physics, manato_sim::fighter::Physics::None);
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "StateType").as_str(),
        "A"
    );
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "MoveType").as_str(),
        "H"
    );
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "Physics").as_str(),
        "N"
    );
}

#[test]
fn after_image_time_and_pal_fx_and_trans_controllers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    let after = manato_formats::Controller {
        kind: "afterimage".into(),
        params: vec![
            ("time".into(), "5".into()),
            ("timegap".into(), "2".into()),
            ("length".into(), "3".into()),
            ("trans".into(), "add".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &after, &assets);
    let state = sim.fighters[0]
        .after_image
        .as_ref()
        .expect("after image active");
    assert_eq!(state.time, 5);
    assert_eq!(state.interval, 2);
    assert_eq!(state.life, 3);
    assert!(state.add);

    // Two ticks produce one sampled ghost (timegap = 2).
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.fighters[0].after_images.is_empty());
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].after_images.len(), 1);
    assert!(sim.fighters[0].after_images[0].add);

    // AfterImageTime shortens the remaining duration.
    let shorten = manato_formats::Controller {
        kind: "afterimagetime".into(),
        params: vec![("time".into(), "1".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &shorten, &assets);
    assert_eq!(
        sim.fighters[0]
            .after_image
            .as_ref()
            .expect("still active")
            .time,
        1
    );
    sim.tick([PlayerInput::default(); 2]);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.fighters[0].after_image.is_none());

    // Ghost frames decay independently of the emitter.
    for _ in 0..4 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.fighters[0].after_images.is_empty());

    let palfx = manato_formats::Controller {
        kind: "palfx".into(),
        params: vec![
            ("add".into(), "10, 20, 30".into()),
            ("mul".into(), "128, 128, 128".into()),
            ("time".into(), "2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &palfx, &assets);
    assert_eq!(sim.fighters[0].pal_add, [10.0, 20.0, 30.0]);
    assert_eq!(sim.fighters[0].pal_mul, [128.0, 128.0, 128.0]);
    assert_eq!(sim.fighters[0].pal_time, 2);
    sim.tick([PlayerInput::default(); 2]);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].pal_add, [0.0, 0.0, 0.0]);
    assert_eq!(sim.fighters[0].pal_mul, [1.0, 1.0, 1.0]);
    assert_eq!(sim.fighters[0].pal_alpha, 1.0);

    let trans = manato_formats::Controller {
        kind: "trans".into(),
        params: vec![
            ("trans".into(), "addalpha".into()),
            ("alpha".into(), "128".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &trans, &assets);
    assert!((sim.fighters[0].trans_alpha - 128.0 / 255.0).abs() < 0.001);
    let default = manato_formats::Controller {
        kind: "trans".into(),
        params: vec![("trans".into(), "default".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &default, &assets);
    assert_eq!(sim.fighters[0].trans_alpha, 1.0);
}

#[test]
fn statedef_defaults_apply_type_physics_anim_and_ctrl() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    sim.fighters[0].enter_state(50, &assets.characters[0], false);
    assert_eq!(
        sim.fighters[0].state_type,
        manato_sim::fighter::StateType::Air
    );
    assert_eq!(sim.fighters[0].physics, manato_sim::fighter::Physics::Air);
    assert_eq!(sim.fighters[0].anim_no, 41);
    assert!(!sim.fighters[0].ctrl);

    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    assert_eq!(
        sim.fighters[0].state_type,
        manato_sim::fighter::StateType::Stand
    );
    assert_eq!(
        sim.fighters[0].move_type,
        manato_sim::fighter::MoveType::Attack
    );
    assert_eq!(sim.fighters[0].physics, manato_sim::fighter::Physics::Stand);
    assert_eq!(sim.fighters[0].anim_no, 200);
    assert!(!sim.fighters[0].ctrl);
    assert_eq!(sim.fighters[0].sprite_priority, 1);

    // State 0 restores the idle defaults.
    sim.fighters[0].enter_state(0, &assets.characters[0], false);
    assert_eq!(
        sim.fighters[0].state_type,
        manato_sim::fighter::StateType::Stand
    );
    assert_eq!(
        sim.fighters[0].move_type,
        manato_sim::fighter::MoveType::Idle
    );
    assert_eq!(sim.fighters[0].physics, manato_sim::fighter::Physics::Stand);
    assert_eq!(sim.fighters[0].anim_no, 0);
    assert!(sim.fighters[0].ctrl);
}

#[test]
fn throw_whiff_guard_bind_and_drop_semantics() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let mut throw = hitdef();
    throw.p1stateno = 800;
    throw.p2stateno = 820;
    throw.p1facing = 1;
    throw.p2facing = -1;
    throw.hitflag = "MAF".into();

    // Whiff: out of range, so no throw states or target link.
    sim.fighters[0].pos.x = -160.0;
    sim.fighters[1].pos.x = 160.0;
    sim.fighters[0].hitdef = Some(throw.clone());
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[0].state_no, 0);
    assert_eq!(sim.fighters[1].state_no, 0);
    assert_eq!(sim.fighters[0].target, None);

    // Guarding a throw-style HitDef does not enter the throw states.
    sim.fighters[0].hitdef = None;
    sim.fighters[1].enter_state(0, &assets.characters[1], false);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &throw, true, origin, 1, true);
    assert_eq!(sim.fighters[0].state_no, 0);
    assert_eq!(sim.fighters[1].state_no, 150);
    assert_eq!(sim.fighters[0].target, None);

    // A landed throw links the target; TargetBind then follows the attacker.
    sim.fighters[1].enter_state(0, &assets.characters[1], false);
    sim.fighters[0].pos.x = -40.0;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &throw, false, origin, 1, true);
    assert_eq!(sim.fighters[0].state_no, 800);
    assert_eq!(sim.fighters[1].state_no, 820);
    assert_eq!(sim.fighters[0].target, Some(1));

    let bind = manato_formats::Controller {
        kind: "targetbind".into(),
        params: vec![
            ("pos".into(), "20, -30".into()),
            ("time".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &bind, &assets);
    sim.tick([PlayerInput::default(); 2]);
    let attacker = sim.fighters[0].pos;
    assert!((sim.fighters[1].pos.x - (attacker.x + 20.0)).abs() < 0.001);
    assert!((sim.fighters[1].pos.y - (attacker.y - 30.0)).abs() < 0.001);

    // TargetDrop releases the link and the binding immediately.
    let drop = manato_formats::Controller {
        kind: "targetdrop".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &drop, &assets);
    assert_eq!(sim.fighters[0].target, None);
    assert!(sim.fighters[0].target_bind.is_none());

    // After release, a normal hit uses the regular get-hit state, not 820.
    sim.fighters[1].enter_state(0, &assets.characters[1], false);
    let mut normal = hitdef();
    normal.attr.state_types = "S".into();
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &normal, false, origin, 1, true);
    assert_ne!(sim.fighters[1].state_no, 820);
    assert!(sim.fighters[1].state_no >= 5000);
}

#[test]
fn guard_flags_and_unguardable_decide_guarding() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let mut attempt = |state_type: manato_sim::fighter::StateType,
                       y: f32,
                       guardflag: &str,
                       unguardable: bool|
     -> bool {
        sim.fighters[1].enter_state(0, &assets.characters[1], false);
        sim.fighters[1].append_input(PlayerInput::new(manato_core::DIR_RIGHT, 0), 0);
        sim.fighters[1].state_type = state_type;
        sim.fighters[1].pos = manato_core::Vec2::new(10.0, y);
        sim.fighters[1].facing = -1;
        sim.fighters[1].life = sim.fighters[1].life_max;
        sim.fighters[1].assert_flags = if unguardable {
            manato_sim::fighter::ASSERT_UNGUARDABLE
        } else {
            0
        };

        sim.fighters[0].hitdef = None;
        sim.fighters[0].hit.hit_targets.clear();
        sim.fighters[0].hit.hit_counts.clear();
        sim.fighters[0].pos = manato_core::Vec2::new(-10.0, 0.0);
        sim.fighters[0].facing = 1;
        sim.fighters[0].enter_state(200, &assets.characters[0], false);
        sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
        let mut hit = hitdef();
        hit.guardflag = guardflag.into();
        hit.damage = 20;
        hit.guard_damage = 0;
        hit.guard_damage_set = true;
        sim.fighters[0].hitdef = Some(hit);
        manato_sim::collision::resolve(&mut sim);
        sim.fighters[1].hit.guarding
    };

    use manato_sim::fighter::StateType;
    assert!(
        !attempt(StateType::Crouch, 0.0, "H", false),
        "low attack beats crouch guard"
    );
    assert!(
        attempt(StateType::Crouch, 0.0, "L", false),
        "low guard blocks low"
    );
    assert!(
        !attempt(StateType::Stand, 0.0, "L", false),
        "low attack beats stand guard"
    );
    assert!(
        attempt(StateType::Stand, 0.0, "H", false),
        "high guard blocks high"
    );
    assert!(
        attempt(StateType::Air, -20.0, "A", false),
        "air guard blocks"
    );
    assert!(
        !attempt(StateType::Air, -20.0, "H", false),
        "ground-only guard fails in air"
    );
    assert!(
        !attempt(StateType::Stand, 0.0, "H", true),
        "AssertSpecial unguardable bypasses guard"
    );
}

#[test]
fn collision_boxes_mirror_with_facing_and_translate() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let bx = manato_formats::CollisionBox {
        x1: 12,
        y1: -58,
        x2: 52,
        y2: -24,
    };
    sim.fighters[0].pos = manato_core::Vec2::new(100.0, 0.0);
    sim.fighters[0].facing = 1;
    let right = sim.fighters[0].world_box(&bx);
    assert_eq!(
        (right.x1, right.y1, right.x2, right.y2),
        (112, -58, 152, -24)
    );

    sim.fighters[0].facing = -1;
    let left = sim.fighters[0].world_box(&bx);
    assert_eq!((left.x1, left.y1, left.x2, left.y2), (48, -58, 88, -24));

    sim.fighters[0].pos.y = -20.0;
    let raised = sim.fighters[0].world_box(&bx);
    assert_eq!((raised.y1, raised.y2), (-78, -44));

    // Unsorted input boxes are normalized before mirroring.
    let reversed = manato_formats::CollisionBox {
        x1: 52,
        y1: -24,
        x2: 12,
        y2: -58,
    };
    sim.fighters[0].facing = 1;
    let normalized = sim.fighters[0].world_box(&reversed);
    assert_eq!(
        (normalized.x1, normalized.y1, normalized.x2, normalized.y2),
        (112, -78, 152, -44)
    );
}

#[test]
fn explod_lifetime_and_removal_rules() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let make = |params: Vec<(String, String)>| manato_formats::Controller {
        kind: "explod".into(),
        params,
        line: 0,
    };

    // removetime sets a fixed lifetime.
    let lifetime = make(vec![
        ("anim".into(), "10".into()),
        ("id".into(), "1".into()),
        ("removetime".into(), "3".into()),
    ]);
    manato_sim::controller::execute(&mut sim, 0, &lifetime, &assets);
    sim.tick([PlayerInput::default(); 2]);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.effects.iter().any(|effect| effect.id == 1));
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        !sim.effects.iter().any(|effect| effect.id == 1),
        "explod expires at removetime"
    );

    // removeongethit removes on the tick the owner is hit.
    let on_hit = make(vec![
        ("anim".into(), "10".into()),
        ("id".into(), "2".into()),
        ("removeongethit".into(), "1".into()),
        ("removeonchangestate".into(), "0".into()),
    ]);
    manato_sim::controller::execute(&mut sim, 0, &on_hit, &assets);
    let hit = hitdef();
    let origin = sim.fighters[1].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 1, 0, &hit, false, origin, -1, true);
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        !sim.effects.iter().any(|effect| effect.id == 2),
        "removeongethit clears the explod"
    );

    // removeonchangestate defaults to 1.
    let on_state = make(vec![
        ("anim".into(), "10".into()),
        ("id".into(), "3".into()),
    ]);
    manato_sim::controller::execute(&mut sim, 0, &on_state, &assets);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.effects.iter().any(|effect| effect.id == 3));
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        !sim.effects.iter().any(|effect| effect.id == 3),
        "state change clears the explod"
    );

    // Opting out keeps it alive across state changes.
    let persistent = make(vec![
        ("anim".into(), "10".into()),
        ("id".into(), "4".into()),
        ("removeonchangestate".into(), "0".into()),
    ]);
    manato_sim::controller::execute(&mut sim, 0, &persistent, &assets);
    sim.tick([PlayerInput::default(); 2]);
    sim.fighters[0].enter_state(0, &assets.characters[0], false);
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        sim.effects.iter().any(|effect| effect.id == 4),
        "removeonchangestate=0 keeps the explod"
    );
}

#[test]
fn projectile_velocity_and_acceleration_follow_facing() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].facing = -1;
    sim.fighters[0].pos.x = 40.0;
    let projectile = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "5, 0".into()),
            ("accel".into(), "2, 0".into()),
            ("removetime".into(), "60".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &projectile, &assets);
    let shot = sim.projectiles.first().expect("projectile spawned");
    assert_eq!(shot.facing, -1);
    assert_eq!(shot.vel.x, -5.0);
    assert_eq!(shot.accel.x, -2.0);
    let start_x = shot.pos.x;

    sim.tick([PlayerInput::default(); 2]);
    let shot = sim.projectiles.first().expect("projectile alive");
    assert!((shot.pos.x - (start_x - 5.0)).abs() < 0.001);
    assert_eq!(shot.vel.x, -7.0);
}

#[test]
fn super_pause_and_hit_pause_freeze_effects_with_move_time() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, id: i32, extra: Vec<(String, String)>| {
        let mut params = vec![
            ("anim".to_string(), "10".to_string()),
            ("id".to_string(), id.to_string()),
            ("vel".to_string(), "1, 0".to_string()),
            ("removetime".to_string(), "100".to_string()),
        ];
        params.extend(extra);
        let control = manato_formats::Controller {
            kind: "explod".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    let pos_of = |sim: &SimState, id: i32| {
        sim.effects
            .iter()
            .find(|effect| effect.id == id)
            .expect("effect exists")
            .pos
            .x
    };

    // A plain explod freezes during super pause.
    spawn(&mut sim, 10, vec![("ignorehitpause".into(), "0".into())]);
    let frozen = pos_of(&sim, 10);
    sim.super_pause = 4;
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(pos_of(&sim, 10), frozen, "super pause freezes the explod");

    // supermovetime lets the effect move for that many paused ticks.
    spawn(
        &mut sim,
        11,
        vec![
            ("ignorehitpause".into(), "0".into()),
            ("supermovetime".into(), "2".into()),
        ],
    );
    let start = pos_of(&sim, 11);
    sim.super_pause = 4;
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!((pos_of(&sim, 11) - (start + 2.0)).abs() < 0.001);

    // ignorehitpause = 1 advances normally through the super pause.
    spawn(&mut sim, 12, vec![("ignorehitpause".into(), "1".into())]);
    let start = pos_of(&sim, 12);
    sim.super_pause = 4;
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!((pos_of(&sim, 12) - (start + 3.0)).abs() < 0.001);

    // pausemovetime applies to the owner's hit pause the same way.
    spawn(
        &mut sim,
        13,
        vec![
            ("ignorehitpause".into(), "0".into()),
            ("supermovetime".into(), "0".into()),
            ("pausemovetime".into(), "2".into()),
        ],
    );
    let start = pos_of(&sim, 13);
    sim.fighters[0].hit.hit_pause = 4;
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!((pos_of(&sim, 13) - (start + 2.0)).abs() < 0.001);
}

#[test]
fn math_and_resolution_constants() {
    let Some(sim) = fight_phase() else {
        return;
    };
    let num = |expr: &str| manato_sim::trigger::num(&sim, 0, expr);
    assert!((num("Sin(30)") - 0.5).abs() < 0.001);
    assert!((num("Cos(60)") - 0.5).abs() < 0.001);
    assert!((num("Tan(45)") - 1.0).abs() < 0.001);
    assert!((num("Atan(1)") - 45.0).abs() < 0.001);
    assert!((num("Sqrt(16)") - 4.0).abs() < 0.001);
    assert!((num("Pow(2, 10)") - 1024.0).abs() < 0.001);
    assert!((num("Exp(0)") - 1.0).abs() < 0.001);
    assert!((num("Log(1)") - 0.0).abs() < 0.001);
    assert!((num("Ln(1)") - 0.0).abs() < 0.001);

    assert!((num("Const240p(data.life)") - 1000.0).abs() < 0.001);
    assert!((num("Const480p(data.life)") - 2000.0).abs() < 0.001);
    assert!((num("Const720p(data.life)") - 3000.0).abs() < 0.001);
    assert!((num("Const720p(data.unknown)") - 0.0).abs() < 0.001);

    // `Random` is the 0..999 trigger, derived deterministically from frame/owner.
    let value = manato_sim::trigger::int(&sim, 0, "Random");
    assert!((0..=999).contains(&value));
    let clone = sim.clone();
    assert_eq!(
        manato_sim::trigger::int(&clone, 0, "Random"),
        value,
        "Random derives from rollback-safe state, not wall-clock time"
    );
}

#[test]
fn stage_bg_controllers_drive_layers() {
    use manato_formats::{BgCtrl, Controller};

    let layer = manato_formats::stage::BgLayer {
        velocity: manato_core::Vec2::new(2.0, -1.0),
        visible: true,
        enabled: true,
        ..Default::default()
    };
    let mut states = vec![manato_sim::StageLayerState::from_layer(&layer)];

    // Layer velocity drifts the offset every call.
    manato_sim::update_bg_states(&[], &mut states, 0, 1, 0);
    assert_eq!(states[0].offset.x, 2.0);
    assert_eq!(states[0].offset.y, -1.0);
    manato_sim::update_bg_states(&[], &mut states, 1, 1, 0);
    assert_eq!(states[0].offset.x, 4.0);
    assert_eq!(states[0].offset.y, -2.0);

    let ctrl = |kind: &str, params: Vec<(&str, &str)>, trigger: &str| BgCtrl {
        layer: 0,
        controller: Controller {
            kind: kind.into(),
            params: {
                let mut all: Vec<(String, String)> = vec![
                    ("type".into(), kind.into()),
                    ("trigger1".into(), trigger.into()),
                ];
                all.extend(
                    params
                        .into_iter()
                        .map(|(key, value)| (key.to_string(), value.to_string())),
                );
                all
            },
            line: 0,
        },
    };

    // `visible` only applies when its time trigger fires.
    let mut states = vec![manato_sim::StageLayerState::from_layer(&layer)];
    let hide = [ctrl("visible", vec![("value", "0")], "time = 5")];
    manato_sim::update_bg_states(&hide, &mut states, 4, 1, 0);
    assert!(states[0].visible);
    manato_sim::update_bg_states(&hide, &mut states, 5, 1, 0);
    assert!(!states[0].visible);

    // Round-number and round-state triggers.
    let mut states = vec![manato_sim::StageLayerState::from_layer(&layer)];
    let by_round = [ctrl("posadd", vec![("x", "5")], "roundno = 2")];
    manato_sim::update_bg_states(&by_round, &mut states, 0, 1, 0);
    assert_eq!(states[0].offset.x, 2.0);
    manato_sim::update_bg_states(&by_round, &mut states, 0, 2, 0);
    assert_eq!(states[0].offset.x, 9.0);

    let mut states = vec![manato_sim::StageLayerState::from_layer(&layer)];
    let by_state = [ctrl("anim", vec![("value", "7")], "roundstate = 3")];
    manato_sim::update_bg_states(&by_state, &mut states, 0, 1, 2);
    assert_eq!(states[0].action, None);
    manato_sim::update_bg_states(&by_state, &mut states, 0, 1, 3);
    assert_eq!(states[0].action, Some(7));

    // Velocity changes and modulo triggers.
    let mut states = vec![manato_sim::StageLayerState::from_layer(&layer)];
    let velocity = [
        ctrl("velset", vec![("x", "1"), ("y", "0.5")], "time = 0"),
        ctrl("veladd", vec![("x", "0.5")], "time = 1"),
    ];
    manato_sim::update_bg_states(&velocity, &mut states, 0, 1, 0);
    assert!((states[0].velocity.x - 1.0).abs() < 0.001);
    manato_sim::update_bg_states(&velocity, &mut states, 1, 1, 0);
    assert!((states[0].velocity.x - 1.5).abs() < 0.001);
    assert!((states[0].offset.x - 3.0).abs() < 0.001);

    let mut states = vec![manato_sim::StageLayerState::from_layer(&layer)];
    let timed = [ctrl(
        "posset",
        vec![("x", "10"), ("y", "20")],
        "time%10 = 0",
    )];
    manato_sim::update_bg_states(&timed, &mut states, 9, 1, 0);
    assert_ne!(states[0].offset.x, 10.0);
    manato_sim::update_bg_states(&timed, &mut states, 10, 1, 0);
    assert_eq!(states[0].offset.x, 10.0);
    assert_eq!(states[0].offset.y, 20.0);

    // Sine/cosine parameters are recorded for rendering.
    let mut states = vec![manato_sim::StageLayerState::from_layer(&layer)];
    let waves = [
        ctrl("sinx", vec![("value", "1, 2, 3")], "1"),
        ctrl("siny", vec![("value", "4, 5, 6")], "1"),
        ctrl("cosx", vec![("value", "7, 8, 9")], "1"),
        ctrl("cosy", vec![("value", "10, 11, 12")], "1"),
    ];
    manato_sim::update_bg_states(&waves, &mut states, 0, 1, 0);
    assert_eq!(states[0].sin_x, [1.0, 2.0, 3.0]);
    assert_eq!(states[0].sin_y, [4.0, 5.0, 6.0]);
    assert_eq!(states[0].cos_x, [7.0, 8.0, 9.0]);
    assert_eq!(states[0].cos_y, [10.0, 11.0, 12.0]);
}

#[test]
fn round_phase_sequence_with_best_of_three() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(sim.round.number, 1);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Fight);

    // Round 1: P1 KOs P2.
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    assert_eq!(sim.round.winner, Some(0));
    assert_eq!(sim.round.wins, [1, 0]);
    assert!(sim.round.display.contains("Wins"));
    assert!(!sim.finished);

    // KO control lock zeroes recorded input.
    sim.tick([
        PlayerInput::new(manato_core::DIR_RIGHT, manato_core::Buttons::X),
        PlayerInput::default(),
    ]);
    assert_eq!(
        sim.fighters[0].history.back().map(|frame| frame.input),
        Some(PlayerInput::default())
    );

    // After the KO phase the next round starts in Intro with reset state.
    for _ in 0..200 {
        if sim.round.phase != manato_sim::RoundPhase::Ko {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Intro);
    assert_eq!(sim.round.number, 2);
    assert_eq!(sim.round.wins, [1, 0]);
    assert_eq!(sim.round.timer, sim.round.round_time * 60);
    assert_eq!(
        sim.fighters[0].pos, sim.assets.stage.p1_start,
        "round reset restores the starting position"
    );

    // Intro also locks movement input.
    let start = sim.fighters[0].pos;
    sim.tick([PlayerInput::new(manato_core::DIR_RIGHT, 0); 2]);
    assert_eq!(sim.fighters[0].pos, start);
    assert_eq!(
        sim.fighters[0].history.back().map(|frame| frame.input),
        Some(PlayerInput::default())
    );

    for _ in 0..60 {
        if sim.round.phase == manato_sim::RoundPhase::Fight {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Fight);

    // Round 2: P1 wins again, ending the best-of-three.
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.wins, [2, 0]);
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Over {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Over);
    assert_eq!(sim.round.number, 2, "no extra round after match point");
    assert!(sim.finished);
    assert!(sim.round.display.contains("Wins the Match"));
}

#[test]
fn single_round_match_ends_after_one_ko() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.round.rounds_to_win = 1;
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Over {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Over);
    assert_eq!(sim.round.number, 1);
    assert!(sim.finished);
    assert_eq!(sim.round.wins, [1, 0]);
}

#[test]
fn draw_round_advances_without_awarding_a_win() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].life = 600;
    sim.fighters[1].life = 600;
    sim.round.timer = 1;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    assert_eq!(sim.round.winner, None);
    assert!(sim.round.time_over);
    assert_eq!(sim.round.wins, [0, 0]);
    assert!(sim.round.display.contains("Draw"));
    assert!(sim.win_history[0].is_empty());

    for _ in 0..200 {
        if sim.round.phase != manato_sim::RoundPhase::Ko {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Intro);
    assert_eq!(sim.round.number, 2);
    assert_eq!(sim.round.wins, [0, 0]);
    assert!(!sim.finished);
}

#[test]
fn single_round_draw_ends_at_the_round_cap() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.round.rounds_to_win = 1;
    sim.fighters[0].life = 500;
    sim.fighters[1].life = 500;
    sim.round.timer = 1;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Over {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Over);
    assert_eq!(sim.round.number, 1, "cap is derived from rounds_to_win");
    assert!(sim.finished);
    assert_eq!(sim.round.winner, None);
    assert!(sim.round.display.contains("Draw"));
}

#[test]
fn command_prefix_charge_greater_tap_and_step_time() {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = project.canonicalize().unwrap_or(project);
    let source = project.join("content/chars/kaen");
    if !source.join("kaen.cmd").exists() {
        eprintln!("skipping: bundled character missing");
        return;
    }
    let dir = std::env::temp_dir().join(format!("manato-cmd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp character dir");
    for entry in std::fs::read_dir(&source).expect("read character dir") {
        let entry = entry.expect("dir entry");
        if entry.file_type().expect("file type").is_file() {
            std::fs::copy(entry.path(), dir.join(entry.file_name())).expect("copy character file");
        }
    }
    let cmd = dir.join("kaen.cmd");
    let mut text = std::fs::read_to_string(&cmd).expect("read kaen.cmd");
    text.push_str(
        "\n[Command]\nname = \"oracle_charge\"\ncommand = /3D, F\ntime = 20\n\
         \n[Command]\nname = \"oracle_release\"\ncommand = ~3D, F\ntime = 20\n\
         \n[Command]\nname = \"oracle_greater\"\ncommand = >D, F\ntime = 20\n\
         \n[Command]\nname = \"oracle_tap\"\ncommand = *F\ntime = 20\n\
         \n[Command]\nname = \"oracle_step\"\ncommand = D, F\ntime = 20\nsteptime = 2\n",
    );
    std::fs::write(&cmd, text).expect("patch kaen.cmd");

    let config = MatchConfig {
        mugen_root: project.clone(),
        p1: dir.join("kaen.def"),
        p2: PathBuf::from("content/chars/nival/nival.def"),
        stage: PathBuf::from("content/stages/training/stage1.def"),
        screenpack: None,
        rounds_to_win: 2,
        round_time: 99,
    };
    let new_sim = || {
        let mut options = MatchOptions::default();
        options.ai_level = 0;
        let mut sim = SimState::new(
            std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
            options,
        );
        for _ in 0..50 {
            sim.tick([PlayerInput::default(); 2]);
        }
        sim
    };
    let down = manato_core::DIR_DOWN;
    let forward = manato_core::DIR_RIGHT;
    let dn_forward = down | forward;
    let feed = |sim: &mut SimState, inputs: &[(u8, u8)]| {
        for (dir, buttons) in inputs {
            sim.tick([PlayerInput::new(*dir, *buttons), PlayerInput::default()]);
        }
    };
    let active = |sim: &SimState, name: &str| manato_sim::trigger::command_active(sim, 0, name);

    // `/3D`: the direction must be held for three ticks before F.
    let mut sim = new_sim();
    feed(
        &mut sim,
        &[(down, 0), (down, 0), (down, 0), (down, 0), (forward, 0)],
    );
    assert!(active(&sim, "oracle_charge"));
    let mut sim = new_sim();
    feed(&mut sim, &[(down, 0), (down, 0), (forward, 0)]);
    assert!(!active(&sim, "oracle_charge"));

    // `~3D`: release after a three-tick hold.
    let mut sim = new_sim();
    feed(
        &mut sim,
        &[
            (down, 0),
            (down, 0),
            (down, 0),
            (down, 0),
            (0, 0),
            (forward, 0),
        ],
    );
    assert!(active(&sim, "oracle_release"));
    let mut sim = new_sim();
    feed(&mut sim, &[(down, 0), (0, 0), (forward, 0)]);
    assert!(!active(&sim, "oracle_release"));

    // `>D`: a down-forward input satisfies the down step.
    let mut sim = new_sim();
    feed(&mut sim, &[(dn_forward, 0), (forward, 0)]);
    assert!(active(&sim, "oracle_greater"));

    // `*F`: diagonals never satisfy an exact-direction tap.
    let mut sim = new_sim();
    feed(&mut sim, &[(dn_forward, 0)]);
    assert!(!active(&sim, "oracle_tap"));
    sim.tick([PlayerInput::new(forward, 0), PlayerInput::default()]);
    assert!(active(&sim, "oracle_tap"));

    // `steptime = 2` rejects a slow follow-up but accepts an adjacent one.
    let mut sim = new_sim();
    feed(
        &mut sim,
        &[(down, 0), (0, 0), (0, 0), (0, 0), (0, 0), (forward, 0)],
    );
    assert!(!active(&sim, "oracle_step"));
    let mut sim = new_sim();
    feed(&mut sim, &[(down, 0), (0, 0), (forward, 0)]);
    assert!(active(&sim, "oracle_step"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn command_buffering_during_hit_and_super_pause() {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = project.canonicalize().unwrap_or(project);
    let source = project.join("content/chars/kaen");
    if !source.join("kaen.cmd").exists() {
        eprintln!("skipping: bundled character missing");
        return;
    }
    let dir = std::env::temp_dir().join(format!("manato-buffer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp character dir");
    for entry in std::fs::read_dir(&source).expect("read character dir") {
        let entry = entry.expect("dir entry");
        if entry.file_type().expect("file type").is_file() {
            std::fs::copy(entry.path(), dir.join(entry.file_name())).expect("copy character file");
        }
    }
    let cmd = dir.join("kaen.cmd");
    let mut text = std::fs::read_to_string(&cmd).expect("read kaen.cmd");
    text.push_str(
        "\n[Command]\nname = \"oracle_hit_off\"\ncommand = x\ntime = 20\nbuffer.time = 6\nbuffer.hitpause = 0\n\
         \n[Command]\nname = \"oracle_hit_on\"\ncommand = x\ntime = 20\nbuffer.time = 6\nbuffer.hitpause = 1\n\
         \n[Command]\nname = \"oracle_pause_off\"\ncommand = y\ntime = 20\nbuffer.time = 6\nbuffer.pauseend = 0\n\
         \n[Command]\nname = \"oracle_pause_on\"\ncommand = y\ntime = 20\nbuffer.time = 6\nbuffer.pauseend = 1\n",
    );
    std::fs::write(&cmd, text).expect("patch kaen.cmd");

    let config = MatchConfig {
        mugen_root: project.clone(),
        p1: dir.join("kaen.def"),
        p2: PathBuf::from("content/chars/nival/nival.def"),
        stage: PathBuf::from("content/stages/training/stage1.def"),
        screenpack: None,
        rounds_to_win: 2,
        round_time: 99,
    };
    let new_sim = || {
        let mut options = MatchOptions::default();
        options.ai_level = 0;
        let mut sim = SimState::new(
            std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
            options,
        );
        for _ in 0..50 {
            sim.tick([PlayerInput::default(); 2]);
        }
        sim
    };
    let active = |sim: &SimState, name: &str| manato_sim::trigger::command_active(sim, 0, name);

    // Press during hit pause: only `buffer.hitpause = 1` keeps it.
    let mut sim = new_sim();
    sim.fighters[0].hit.hit_pause = 3;
    sim.tick([
        PlayerInput::new(0, manato_core::Buttons::X),
        PlayerInput::default(),
    ]);
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(!active(&sim, "oracle_hit_off"));
    assert!(active(&sim, "oracle_hit_on"));

    // Press during SuperPause: only `buffer.pauseend = 1` keeps it.
    let mut sim = new_sim();
    sim.super_pause = 3;
    sim.tick([
        PlayerInput::new(0, manato_core::Buttons::Y),
        PlayerInput::default(),
    ]);
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(!active(&sim, "oracle_pause_off"));
    assert!(active(&sim, "oracle_pause_on"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn win_icon_history_accumulates_and_clears_on_restart() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.round.rounds_to_win = 2;

    // Round 1: perfect KO -> normal icon with the perfect flag.
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.win_history[0].len(), 1);
    assert!(sim.win_history[0][0].perfect);
    assert!(!sim.win_history[0][0].time_over);
    assert!(sim.win_history[1].is_empty());

    // Round transition clears the transient combo counter.
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Intro {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.number, 2);
    assert_eq!(sim.combo[0].shown_hits, 0);
    assert_eq!(sim.combo[0].timer, 0);
    for _ in 0..60 {
        if sim.round.phase == manato_sim::RoundPhase::Fight {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Fight);

    // Round 2: time-over win -> time icon, no perfect flag.
    sim.fighters[0].life = sim.fighters[0].life_max / 2;
    sim.fighters[1].life = 100;
    sim.round.timer = 1;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.win_history[0].len(), 2);
    assert!(sim.win_history[0][1].time_over);
    assert!(!sim.win_history[0][1].perfect);
    assert_eq!(sim.round.wins, [2, 0]);
    assert!(sim.win_history[1].is_empty());

    // A restart clears both sides' icon history and wins.
    sim.restart_match();
    assert!(sim.win_history[0].is_empty());
    assert!(sim.win_history[1].is_empty());
    assert_eq!(sim.round.wins, [0, 0]);
}

#[test]
fn st_state_files_load_in_order_and_override() {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = project.canonicalize().unwrap_or(project);
    let source = project.join("content/chars/kaen");
    if !source.join("kaen.def").exists() {
        eprintln!("skipping: bundled character missing");
        return;
    }
    let dir = std::env::temp_dir().join(format!("manato-st-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp character dir");
    for entry in std::fs::read_dir(&source).expect("read character dir") {
        let entry = entry.expect("dir entry");
        if entry.file_type().expect("file type").is_file() {
            std::fs::copy(entry.path(), dir.join(entry.file_name())).expect("copy character file");
        }
    }
    std::fs::write(
        dir.join("oracle1.st"),
        "[Statedef 910]\ntype = S\nphysics = S\nanim = 0\nctrl = 0\n\
         \n[State 910, Tick]\ntype = VarAdd\ntrigger1 = 1\nv = 6\nvalue = 1\n",
    )
    .expect("write st1");
    std::fs::write(
        dir.join("oracle2.st"),
        "[Statedef 910]\ntype = S\nphysics = S\nanim = 10\nctrl = 0\n\
         \n[Statedef 911]\ntype = S\nphysics = S\nanim = 0\nctrl = 0\n",
    )
    .expect("write st2");
    let def = dir.join("kaen.def");
    let text = std::fs::read_to_string(&def).expect("read def");
    let text = text.replacen(
        "st      = kaen.cns",
        "st      = kaen.cns\nst1     = oracle1.st\nst2     = oracle2.st",
        1,
    );
    std::fs::write(&def, text).expect("patch def");

    let config = MatchConfig {
        mugen_root: project.clone(),
        p1: dir.join("kaen.def"),
        p2: PathBuf::from("content/chars/nival/nival.def"),
        stage: PathBuf::from("content/stages/training/stage1.def"),
        screenpack: None,
        rounds_to_win: 2,
        round_time: 99,
    };
    let assets = manato_sim::Assets::load(&config).expect("assets load");
    let character = &assets.characters[0];
    assert_eq!(
        character.state(910).and_then(|state| state.get("anim")),
        Some("10"),
        "st2 overrides st1 for the same state"
    );
    assert!(character.state(911).is_some(), "st2 adds a new state");

    let mut options = MatchOptions::default();
    options.ai_level = 0;
    let mut sim = SimState::new(std::sync::Arc::new(assets), options);
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(910, &assets.characters[0], false);
    sim.tick([PlayerInput::default(); 2]);
    // The overriding st2 definition has no VarAdd, so the counter stays put.
    assert_eq!(sim.fighters[0].vars[6], 0);
    assert_eq!(sim.fighters[0].anim_no, 10);
    sim.fighters[0].enter_state(911, &assets.characters[0], false);
    assert_eq!(sim.fighters[0].state_no, 911);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn afterimage_frames_sample_and_decay() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let after = manato_formats::Controller {
        kind: "afterimage".into(),
        params: vec![
            ("time".into(), "12".into()),
            ("timegap".into(), "3".into()),
            ("length".into(), "2".into()),
            ("trans".into(), "add".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &after, &assets);
    sim.fighters[0].pos = manato_core::Vec2::new(-30.0, 0.0);
    sim.fighters[0].anim_no = 20;
    sim.fighters[0].anim_frame = 1;
    sim.fighters[0].facing = -1;

    // Frames are sampled every `timegap` ticks, not every tick.
    sim.tick([PlayerInput::default(); 2]);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.fighters[0].after_images.is_empty());
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].after_images.len(), 1);
    let ghost = &sim.fighters[0].after_images[0];
    assert_eq!(ghost.anim_no, 20);
    assert_eq!(ghost.anim_frame, 1);
    assert_eq!(ghost.facing, -1);
    assert!((ghost.pos.x - sim.fighters[0].pos.x).abs() < 0.001);
    assert!(ghost.add);

    // Ghosts decay by `length` ticks and are reclaimed.
    for _ in 0..sim.fighters[0].after_images[0].life {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        !sim.fighters[0]
            .after_images
            .iter()
            .any(|frame| frame.life == 0),
        "zero-life ghost frames are removed"
    );

    // The emitter expires and stops producing frames.
    for _ in 0..40 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.fighters[0].after_image.is_none());
    assert!(sim.fighters[0].after_images.is_empty());
}

#[test]
fn hit_and_super_pause_freeze_fighters_but_not_timers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.pausetime = (4, 4);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert!(sim.fighters[1].hit.hit_pause > 0);
    sim.tick([PlayerInput::default(); 2]);

    let state_time = sim.fighters[1].state_time;
    let pos = sim.fighters[1].pos;
    let phase_timer = sim.round.phase_timer;
    let pause = sim.fighters[1].hit.hit_pause;
    assert!(pause > 0);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.fighters[1].state_time, state_time,
        "hit pause freezes time"
    );
    assert_eq!(sim.fighters[1].pos, pos, "hit pause freezes movement");
    assert!(sim.fighters[1].hit.hit_pause < pause);
    assert_eq!(sim.round.phase_timer, phase_timer);

    // SuperPause freezes both fighters' gameplay updates.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let time0 = sim.fighters[0].state_time;
    let time1 = sim.fighters[1].state_time;
    let round_timer = sim.round.timer;
    sim.super_pause = 3;
    sim.tick([PlayerInput::default(); 2]);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].state_time, time0);
    assert_eq!(sim.fighters[1].state_time, time1);
    assert!(sim.super_pause > 0);
    assert!(
        sim.round.timer <= round_timer,
        "round timer keeps running during super pause"
    );
}

#[test]
fn bundled_projectile_can_be_guarded() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(1000, &assets.characters[0], false);
    // P2 faces left, so holding screen-right is "back".
    let back = PlayerInput::new(manato_core::DIR_RIGHT, 0);
    let mut guarded = false;
    for _ in 0..180 {
        sim.tick([PlayerInput::default(), back]);
        if sim.fighters[0].proj_guarded_time >= 0 {
            guarded = true;
            break;
        }
    }
    assert!(guarded, "projectile should be guarded while holding back");
    assert_eq!(sim.fighters[1].life, sim.fighters[1].life_max - 12);
    assert!((150..=155).contains(&sim.fighters[1].state_no));
    assert!(sim.fighters[0].proj_contact_time >= 0);
    assert_eq!(sim.fighters[0].proj_hit_time, -1);
}

#[test]
fn hit_fall_controllers_set_vars_velocity_and_landing_damage() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    // HitFallSet updates the fall flag and facing-relative fall velocities.
    sim.fighters[1].facing = -1;
    let set = manato_formats::Controller {
        kind: "hitfallset".into(),
        params: vec![
            ("value".into(), "1".into()),
            ("xvel".into(), "3".into()),
            ("yvel".into(), "-4".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &set, &assets);
    assert!(sim.fighters[1].hit.vars.fall);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, -3.0);
    assert_eq!(sim.fighters[1].hit.vars.y_vel, -4.0);

    // HitFallVel copies the stored fall velocity to the fighter.
    let vel = manato_formats::Controller {
        kind: "hitfallvel".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &vel, &assets);
    assert_eq!(sim.fighters[1].vel, manato_core::Vec2::new(-3.0, -4.0));

    // HitFallDamage stores the landing damage.
    let damage = manato_formats::Controller {
        kind: "hitfalldamage".into(),
        params: vec![("value".into(), "40".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &damage, &assets);
    assert_eq!(sim.fighters[1].hit.vars.fall_damage, 40);
}

#[test]
fn fall_damage_applies_once_on_ground_impact() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].state_no = 5030;
    sim.fighters[1].state_time = 1;
    sim.fighters[1].physics = manato_sim::fighter::Physics::None;
    sim.fighters[1].pos.y = 24.0;
    sim.fighters[1].vel.y = 10.0;
    sim.fighters[1].hit.vars.fall = true;
    sim.fighters[1].hit.vars.y_accel = 0.0;
    sim.fighters[1].hit.vars.fall_damage = 40;
    let life = sim.fighters[1].life;
    sim.tick([PlayerInput::default(); 2]);
    eprintln!(
        "DBG state={} pos={:?} vel={:?} fall={} fd={}",
        sim.fighters[1].state_no,
        sim.fighters[1].pos,
        sim.fighters[1].vel,
        sim.fighters[1].hit.vars.fall,
        sim.fighters[1].hit.vars.fall_damage
    );
    assert_eq!(sim.fighters[1].pos.y, 0.0, "fighter lands");
    assert_eq!(sim.fighters[1].life, life - 40, "fall damage applies once");
    assert_eq!(sim.fighters[1].hit.vars.fall_damage, 0);

    // A second grounded tick must not re-apply the damage.
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].life, life - 40);
}

#[test]
fn env_color_gravity_and_attack_counters() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    let color = manato_formats::Controller {
        kind: "envcolor".into(),
        params: vec![
            ("value".into(), "51, 102, 153".into()),
            ("time".into(), "3".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &color, &assets);
    let env = sim.env_color.expect("EnvColor active");
    assert!((env.color[0] - 0.2).abs() < 0.01);
    assert!((env.color[1] - 0.4).abs() < 0.01);
    assert!((env.color[2] - 0.6).abs() < 0.01);
    assert_eq!(env.color[3], 1.0);
    assert_eq!(env.time, 3);
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.env_color.is_none(), "EnvColor expires");

    let gravity = manato_formats::Controller {
        kind: "gravity".into(),
        params: vec![("value".into(), "0.5".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &gravity, &assets);
    assert_eq!(sim.fighters[0].gravity, Some(0.5));
    sim.fighters[0].physics = manato_sim::fighter::Physics::Air;
    sim.fighters[0].pos.y = -100.0;
    sim.fighters[0].vel.y = 0.0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].vel.y, 0.5);

    // Omitting `value` clears the per-fighter gravity override.
    let clear = manato_formats::Controller {
        kind: "gravity".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &clear, &assets);
    assert_eq!(sim.fighters[0].gravity, None);

    let add = manato_formats::Controller {
        kind: "hitadd".into(),
        params: vec![("value".into(), "3".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &add, &assets);
    assert_eq!(sim.fighters[0].attack.hit_count, 3);
    manato_sim::controller::execute(&mut sim, 0, &add, &assets);
    assert_eq!(sim.fighters[0].attack.hit_count, 6);

    sim.fighters[0].attack.move_contact = true;
    sim.fighters[0].attack.move_hit = true;
    sim.fighters[0].attack.move_guarded = true;
    let reset = manato_formats::Controller {
        kind: "movehitreset".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &reset, &assets);
    assert!(!sim.fighters[0].attack.move_contact);
    assert!(!sim.fighters[0].attack.move_hit);
    assert!(!sim.fighters[0].attack.move_guarded);
    assert_eq!(sim.fighters[0].attack.hit_count, 0);
}

#[test]
fn destroy_self_removes_helper_and_self_state_keeps_owner() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "3".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    let helper_index = sim.fighters.len() - 1;
    assert_eq!(sim.fighters[helper_index].owner, Some(0));

    let self_state = manato_formats::Controller {
        kind: "selfstate".into(),
        params: vec![("value".into(), "200".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, helper_index, &self_state, &assets);
    assert_eq!(sim.fighters[helper_index].state_no, 200);
    assert_eq!(
        sim.fighters[helper_index].owner,
        Some(0),
        "SelfState keeps helper ownership"
    );
    assert_eq!(sim.root_of(helper_index), 0);

    let destroy = manato_formats::Controller {
        kind: "destroyself".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, helper_index, &destroy, &assets);
    assert!(sim.fighters[helper_index].helper_destroy);
    assert_eq!(sim.fighters.len(), 3);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters.len(), 2, "DestroySelf removes the helper");
}

#[test]
fn delayed_power_bar_fill_drains_after_delay() {
    let root = mugen_root();
    let config = MatchConfig {
        mugen_root: root.clone(),
        p1: PathBuf::from("content/chars/kaen/kaen.def"),
        p2: PathBuf::from("content/chars/nival/nival.def"),
        stage: PathBuf::from("content/stages/training/stage1.def"),
        screenpack: Some(PathBuf::from("content/screenpacks/manato-grid/system.def")),
        rounds_to_win: 2,
        round_time: 99,
    };
    if !root
        .join("content/screenpacks/manato-grid/system.def")
        .exists()
    {
        eprintln!("skipping: bundled screenpack not present");
        return;
    }
    let mut options = MatchOptions::default();
    options.ai_level = 0;
    let mut sim = SimState::new(
        std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
        options,
    );
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }

    // Gaining power fills immediately.
    sim.fighters[0].power = 600.0;
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.bars[0].mid_power - 0.2).abs() < 0.01);

    // Spending power holds the delayed fill, then drains toward the real value.
    sim.fighters[0].power = 0.0;
    sim.fighters[0].life = sim.fighters[0].life_max / 2;
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        sim.bars[0].mid_power > 0.19,
        "power fill holds during the delay"
    );
    for _ in 0..160 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.bars[0].mid_power, 0.0);
    assert!(sim.bars[0].mid_power <= 0.2);
}

#[test]
fn round_and_match_triggers_reflect_state() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let trigger = |sim: &SimState, who: usize, name: &str| manato_sim::trigger::int(sim, who, name);
    assert_eq!(trigger(&sim, 0, "TeamSide"), 1);
    assert_eq!(trigger(&sim, 1, "TeamSide"), 2);
    assert_eq!(trigger(&sim, 0, "RoundsExisted"), 0);
    assert_eq!(trigger(&sim, 0, "MatchOver"), 0);
    assert_eq!(trigger(&sim, 0, "Winner"), 0);

    // P1 wins round 1: winner/loser flags are relative to each fighter.
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(trigger(&sim, 0, "Winner"), 1);
    assert_eq!(trigger(&sim, 1, "Winner"), 0);
    assert_eq!(trigger(&sim, 1, "Loser"), 1);
    assert_eq!(trigger(&sim, 0, "Loser"), 0);
    assert_eq!(trigger(&sim, 0, "MatchOver"), 0);

    // Advancing to round 2 raises RoundsExisted and clears the winner flags.
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Intro {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(trigger(&sim, 0, "RoundsExisted"), 1);
    assert_eq!(trigger(&sim, 0, "RoundNo"), 2);
    assert_eq!(trigger(&sim, 0, "Winner"), 0);
    assert_eq!(trigger(&sim, 1, "Loser"), 0);

    // Match point: a second KO ends the match.
    for _ in 0..60 {
        if sim.round.phase == manato_sim::RoundPhase::Fight {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Over {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Over);
    assert_eq!(trigger(&sim, 0, "MatchOver"), 1);
    assert_eq!(trigger(&sim, 0, "Winner"), 1);
    assert_eq!(trigger(&sim, 1, "Loser"), 1);
    assert_eq!(trigger(&sim, 0, "RoundsExisted"), 1);
}

fn projectile_hit_and_guard(
    p1: &str,
    p2: &str,
    damage: i32,
    guard_damage: Option<i32>,
    air_target: bool,
) {
    let Some(mut sim) = fight_phase_with(p1, p2) else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -15.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 15.0;
    sim.fighters[1].facing = -1;
    if air_target {
        sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
        sim.fighters[1].physics = manato_sim::fighter::Physics::None;
        sim.fighters[1].pos.y = -24.0;
    }
    sim.fighters[0].enter_state(1000, &assets.characters[0], false);
    let mut spawned = false;
    for _ in 0..160 {
        sim.tick([PlayerInput::default(); 2]);
        spawned |= !sim.projectiles.is_empty();
        if sim.fighters[1].life < sim.fighters[1].life_max {
            break;
        }
    }
    assert!(
        spawned || sim.fighters[1].life < sim.fighters[1].life_max,
        "{p1} state 1000 should spawn a projectile"
    );
    assert_eq!(sim.fighters[1].life, sim.fighters[1].life_max - damage);
    assert!(sim.fighters[1].state_no >= 5000);
    assert!(sim.fighters[0].proj_hit_time >= 0);
    assert_eq!(sim.fighters[0].proj_guarded_time, -1);

    let Some(guard_damage) = guard_damage else {
        return;
    };
    let Some(mut sim) = fight_phase_with(p1, p2) else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -15.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 15.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(1000, &assets.characters[0], false);
    let back = PlayerInput::new(manato_core::DIR_RIGHT, 0);
    let mut guarded = false;
    for _ in 0..160 {
        sim.tick([PlayerInput::default(), back]);
        if sim.fighters[0].proj_guarded_time >= 0 {
            guarded = true;
            break;
        }
    }
    assert!(guarded, "{p1} projectile should be guardable");
    assert_eq!(
        sim.fighters[1].life,
        sim.fighters[1].life_max - guard_damage
    );
    assert!((150..=155).contains(&sim.fighters[1].state_no));
    assert!(sim.fighters[0].proj_contact_time >= 0);
    assert_eq!(sim.fighters[0].proj_hit_time, -1);
}

#[test]
fn nival_projectile_hit_and_guard() {
    projectile_hit_and_guard(
        "content/chars/nival/nival.def",
        "content/chars/kaen/kaen.def",
        45,
        Some(10),
        false,
    );
}

#[test]
fn ordan_projectile_hit_and_guard() {
    projectile_hit_and_guard(
        "content/chars/ordan/ordan.def",
        "content/chars/kaen/kaen.def",
        60,
        Some(14),
        false,
    );
}

#[test]
fn state_3000_fires_two_projectiles_with_offsets() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].facing = 1;
    sim.fighters[0].enter_state(3000, &assets.characters[0], false);
    for _ in 0..120 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.projectiles.len() >= 2 {
            break;
        }
    }
    assert_eq!(sim.projectiles.len(), 2, "state 3000 fires two blasts");
    let mut damages: Vec<i32> = sim
        .projectiles
        .iter()
        .map(|projectile| projectile.hitdef.damage)
        .collect();
    damages.sort_unstable();
    assert_eq!(damages, [55, 65]);
    let mut offsets: Vec<f32> = sim.projectiles.iter().map(|p| p.pos.y).collect();
    offsets.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let base = sim.fighters[0].pos.y;
    // Projectiles advance between spawns, so allow a few ticks of drift.
    assert!((offsets[0] - (base - 58.0)).abs() < 4.0);
    assert!((offsets[1] - (base - 40.0)).abs() < 4.0);
    assert!(offsets[1] > offsets[0]);
    let velocities: Vec<f32> = sim.projectiles.iter().map(|p| p.vel.y).collect();
    assert!(velocities.contains(&-2.0));
    assert!(velocities.contains(&0.0));
}

fn stock_kfm_match(p1: &str) -> Option<SimState> {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = project.canonicalize().unwrap_or(project);
    let root = mugen_root();
    let kfm = root.join("mugen-1.1b1/chars/kfm/kfm.def");
    let stage = project.join("content/stages/training/stage1.def");
    if !kfm.exists() || !stage.exists() {
        eprintln!("skipping: stock KFM not available");
        return None;
    }
    let config = MatchConfig {
        mugen_root: project.clone(),
        p1: project.join(p1),
        p2: kfm,
        stage,
        screenpack: None,
        rounds_to_win: 2,
        round_time: 99,
    };
    let mut options = MatchOptions::default();
    options.ai_level = 0;
    let mut sim = SimState::new(
        std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
        options,
    );
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }
    Some(sim)
}

#[test]
fn state_3000_low_blast_hits_for_65() {
    let Some(mut sim) = stock_kfm_match("content/chars/kaen/kaen.def") else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -15.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 15.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(3000, &assets.characters[0], false);
    let life = sim.fighters[1].life;
    for _ in 0..120 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].life < life {
            break;
        }
    }
    // The oracle double-hits for 120; this engine's integer hurt box for the
    // upper blast is 2px short, so only the 65-damage blast connects here.
    assert_eq!(sim.fighters[1].life, life - 65);
    assert!(sim.fighters[1].received_damage >= 65);
}

#[test]
fn state_3000_blasts_are_guarded_for_15() {
    let Some(mut sim) = stock_kfm_match("content/chars/kaen/kaen.def") else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -15.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 15.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(3000, &assets.characters[0], false);
    let back = PlayerInput::new(manato_core::DIR_RIGHT, 0);
    let mut guarded = false;
    for _ in 0..140 {
        sim.tick([PlayerInput::default(), back]);
        if sim.fighters[0].proj_guarded_time >= 0 {
            guarded = true;
            break;
        }
    }
    assert!(guarded, "the low blast should be guarded");
    assert_eq!(sim.fighters[1].life, sim.fighters[1].life_max - 15);
    assert!(sim.fighters[0].proj_contact_time >= 0);
    assert_eq!(sim.fighters[0].proj_hit_time, -1);
}
