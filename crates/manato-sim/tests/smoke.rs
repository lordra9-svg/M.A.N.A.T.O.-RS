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
    assert!(
        sim.fighters[1].life < sim.fighters[1].life_max,
        "projectile did not hit the opponent"
    );
    assert!(
        sim.fighters[0].proj_contact_time >= 0,
        "projectile contact timer should start at the hit tick"
    );
    assert!(
        sim.fighters[0].proj_hit_time >= 0 || sim.fighters[0].proj_guarded_time >= 0,
        "projectile hit/guard timer should be set"
    );
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
        options,
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
