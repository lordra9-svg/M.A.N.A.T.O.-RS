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
