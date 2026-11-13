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
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = project.canonicalize().unwrap_or(project);
    // Bundled fixtures live in the repository. Always prefer them so a stock
    // `MANATO_MUGEN_ROOT` neither skips these tests nor runs them against a
    // stale copy of the bundled content under the stock root.
    let locate = |path: &str| {
        let bundled = project.join(path);
        if bundled.exists() {
            bundled
        } else {
            root.join(path)
        }
    };
    let p1_path = locate(p1);
    let p2_path = locate(p2);
    let stage_path = locate("content/stages/training/stage1.def");
    if !p1_path.exists() || !p2_path.exists() || !stage_path.exists() {
        eprintln!("skipping: content not present at {}", root.display());
        return None;
    }
    let config = MatchConfig {
        mugen_root: root.clone(),
        p1: p1_path,
        p2: p2_path,
        stage: stage_path,
        screenpack: None,
        rounds_to_win: 2,
        round_time: 99,
    };
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
    sim.fighters[1].hit_overrides[0] = Some(manato_sim::fighter::HitOverride {
        attr: "S, NA".into(),
        state: 0,
        time: 100,
        keepstate: false,
        forceguard: false,
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
    // Blend-only `add` ignores `alpha`; use `addalpha` for opacity.
    assert_eq!(sim.fighters[0].trans_alpha, 1.0);
    let add_alpha = manato_formats::Controller {
        kind: "trans".into(),
        params: vec![
            ("type".into(), "Trans".into()),
            ("trans".into(), "addalpha".into()),
            ("alpha".into(), "128".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &add_alpha, &assets);
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
    // Standard content uses a negative ground velocity, which pushes the
    // defender away from a right-facing attacker (screen +X).
    push.ground_velocity = manato_core::Vec2::new(-4.0, 0.0);
    push.ground_cornerpush = -8.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &push, false, origin, 1, true);
    // Corner push is world-space, so it cancels the +4 pushback.
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
fn falling_hitflag_requires_f() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -1.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 5.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));

    // Falling defender: the bundled fall action has no hurt boxes, so use the
    // idle animation with the fall flag set.
    let mark_falling = |sim: &mut SimState| {
        sim.fighters[1].enter_state(0, &assets.characters[1], false);
        sim.fighters[1].set_anim(0, &assets.characters[1], None);
        sim.fighters[1].state_no = 5030;
        sim.fighters[1].hit.vars.fall = true;
    };

    let mut hit = hitdef();
    hit.hitflag = "MA".into();
    hit.damage = 10;
    hit.pausetime = (0, 0);
    sim.fighters[0].hitdef = Some(hit.clone());
    let life = sim.fighters[1].life;
    mark_falling(&mut sim);
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(
        sim.fighters[1].life, life,
        "F is required to hit a falling opponent"
    );

    // With F the same attack connects.
    hit.hitflag = "MAF".into();
    sim.fighters[0].hitdef = Some(hit);
    sim.fighters[0].hit.hit_counts.clear();
    sim.fighters[0].hit.hit_targets.clear();
    mark_falling(&mut sim);
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, life - 10);

    // A grounded, non-falling opponent is not gated by F.
    sim.fighters[0].hit.hit_counts.clear();
    sim.fighters[0].hit.hit_targets.clear();
    sim.fighters[1].enter_state(0, &assets.characters[1], false);
    sim.fighters[1].set_anim(0, &assets.characters[1], None);
    sim.fighters[1].hit.vars.fall = false;
    sim.fighters[1].state_no = 0;
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, life - 20);
}

#[test]
fn numhits_limits_persistent_hitdef_in_resolve() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -1.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 5.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    let mut hit = hitdef();
    hit.numhits = 2;
    hit.damage = 10;
    hit.pausetime = (0, 0);
    sim.fighters[0].hitdef = Some(hit);

    // Bundled get-hit actions have no Clsn2 boxes, so restore the idle animation
    // between resolutions to give the next hit a hurt box to connect with.
    let reset_defender = |sim: &mut SimState, assets: &manato_sim::Assets| {
        sim.fighters[1].enter_state(0, &assets.characters[1], false);
        sim.fighters[1].set_anim(0, &assets.characters[1], None);
    };

    let life = sim.fighters[1].life;
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, life - 10, "first hit connects");
    reset_defender(&mut sim, &assets);
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, life - 20, "second hit connects");
    reset_defender(&mut sim, &assets);
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(
        sim.fighters[1].life,
        life - 20,
        "numhits stops further hits in this activation"
    );

    // A new activation resets the budget.
    sim.fighters[0].hit.hit_counts.clear();
    sim.fighters[0].hit.hit_targets.clear();
    reset_defender(&mut sim, &assets);
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, life - 30);
}

#[test]
fn changeanim_elem_selects_start_frame() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let anim = |elem: Option<&str>| {
        let mut params = vec![("value".to_string(), "200".to_string())];
        if let Some(elem) = elem {
            params.push(("elem".to_string(), elem.to_string()));
        }
        manato_formats::Controller {
            kind: "changeanim".into(),
            params,
            line: 0,
        }
    };

    // Animation 200 frame 0 lasts 3 ticks.
    manato_sim::controller::execute(&mut sim, 0, &anim(Some("2")), &assets);
    assert_eq!(sim.fighters[0].anim_no, 200);
    assert_eq!(sim.fighters[0].anim_frame, 1);
    assert_eq!(sim.fighters[0].anim_time, 3);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElem"), 2);

    // Element 0 clamps to the first frame.
    manato_sim::controller::execute(&mut sim, 0, &anim(Some("0")), &assets);
    assert_eq!(sim.fighters[0].anim_frame, 0);
    assert_eq!(sim.fighters[0].anim_time, 0);

    // Out-of-range elements clamp to the last frame (durations 3+4=7).
    manato_sim::controller::execute(&mut sim, 0, &anim(Some("99")), &assets);
    assert_eq!(sim.fighters[0].anim_frame, 2);
    assert_eq!(sim.fighters[0].anim_time, 7);

    // Omitting elem starts at the first frame.
    manato_sim::controller::execute(&mut sim, 0, &anim(None), &assets);
    assert_eq!(sim.fighters[0].anim_frame, 0);
    assert_eq!(sim.fighters[0].anim_time, 0);
}

#[test]
fn stage_autoturn_faces_opponent_when_idle() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    // Cross the fighters so each faces away from the other.
    sim.fighters[0].pos.x = 30.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = -30.0;
    sim.fighters[1].facing = -1;
    for fighter in &mut sim.fighters {
        fighter.ctrl = true;
        fighter.move_type = manato_sim::fighter::MoveType::Idle;
        fighter.state_type = manato_sim::fighter::StateType::Stand;
    }
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].facing, -1, "P1 turns toward P2");
    assert_eq!(sim.fighters[1].facing, 1, "P2 turns toward P1");

    // An attacking fighter does not auto-turn.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].pos.x = 30.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = -30.0;
    sim.fighters[0].ctrl = false;
    sim.fighters[0].move_type = manato_sim::fighter::MoveType::Attack;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].facing, 1, "attacking fighters keep facing");

    // AssertSpecial(noautoturn) suppresses the turn.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let state = assets.characters[0]
            .states
            .get_mut(&-1)
            .expect("statedef -1");
        state.controllers.push(manato_formats::Controller {
            kind: "assertspecial".into(),
            params: vec![
                ("type".into(), "AssertSpecial".into()),
                ("noautoturn".into(), "1".into()),
            ],
            line: 0,
        });
    }
    sim.fighters[0].pos.x = 30.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = -30.0;
    sim.fighters[0].ctrl = true;
    sim.fighters[0].move_type = manato_sim::fighter::MoveType::Idle;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.fighters[0].facing, 1,
        "noautoturn keeps the original facing"
    );
}

#[test]
fn noko_flag_prevents_ko_classification() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    // Re-assert NoKO every tick from the helper state so zero life is ignored.
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let state = assets.characters[1]
            .states
            .get_mut(&-1)
            .expect("statedef -1");
        state.controllers.push(manato_formats::Controller {
            kind: "assertspecial".into(),
            params: vec![
                ("type".into(), "AssertSpecial".into()),
                ("noko".into(), "1".into()),
            ],
            line: 0,
        });
    }
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.round.phase,
        manato_sim::RoundPhase::Fight,
        "NoKO keeps the round in Fight despite zero life"
    );
    assert_eq!(sim.fighters[1].life, 0);
    assert_eq!(sim.round.winner, None);

    // Removing the assertion allows the KO to register.
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let state = assets.characters[1]
            .states
            .get_mut(&-1)
            .expect("statedef -1");
        state.controllers.retain(|c| c.kind != "assertspecial");
    }
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    assert_eq!(sim.round.winner, Some(0));
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
    // Clear the round-start state-change marker so `removeonchangestate`
    // does not remove the explod on its first tick.
    sim.tick([PlayerInput::default(); 2]);
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
            ("shadow".into(), "1".into()),
            ("ownpal".into(), "1".into()),
            ("ontop".into(), "0".into()),
            ("sprpriority".into(), "7".into()),
            ("vfacing".into(), "-1".into()),
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
    assert!(effect.shadow);
    assert!(effect.own_pal);
    assert!(effect.behind, "ontop = 0 draws behind");
    assert_eq!(effect.sprite_priority, 7);
    assert!(effect.flip_v, "vfacing = -1 flips vertically");

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
fn zoom_controller_honors_stage_and_camera_bounds() {
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
    loaded.stage.camera.bound_left = -200.0;
    loaded.stage.camera.bound_right = 200.0;
    loaded.stage.left_bound = -150.0;
    loaded.stage.right_bound = 150.0;
    let mut options = MatchOptions::default();
    options.ai_level = 0;
    let mut sim = SimState::new(std::sync::Arc::new(loaded), options);
    let assets = sim.assets.clone();

    // scale 2 => half-width 80, so camera bounds clamp x to +-120.
    let zoom = |params: Vec<(String, String)>| manato_formats::Controller {
        kind: "zoom".into(),
        params,
        line: 0,
    };
    let camera_bound = zoom(vec![
        ("pos".into(), "400, 0".into()),
        ("scale".into(), "2".into()),
        ("time".into(), "20".into()),
        ("lag".into(), "1".into()),
        ("camerabound".into(), "1".into()),
    ]);
    manato_sim::controller::execute(&mut sim, 0, &camera_bound, &assets);
    for _ in 0..12 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        (sim.camera.x - 120.0).abs() < 0.5,
        "camerabound should clamp x to {} (got {})",
        120.0,
        sim.camera.x
    );

    sim.camera.x = 0.0;
    sim.camera.scale = 2.0;
    let stage_bound = zoom(vec![
        ("pos".into(), "400, 0".into()),
        ("scale".into(), "2".into()),
        ("time".into(), "20".into()),
        ("lag".into(), "1".into()),
        ("stagebound".into(), "1".into()),
    ]);
    manato_sim::controller::execute(&mut sim, 0, &stage_bound, &assets);
    for _ in 0..12 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        (sim.camera.x - 70.0).abs() < 0.5,
        "stagebound should clamp x to {} (got {})",
        70.0,
        sim.camera.x
    );
}

#[test]
fn clsn_overlap_checks_collision_boxes_between_players() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].pos = manato_core::Vec2::new(0.0, 0.0);
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos = manato_core::Vec2::new(0.0, 0.0);
    sim.fighters[1].facing = -1;
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ClsnOverlap(clsn2, 1, clsn2)"),
        1
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "ClsnOverlap(clsn2, 0, clsn2)"),
        1
    );

    sim.fighters[1].pos.x = 400.0;
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ClsnOverlap(clsn2, 1, clsn2)"),
        0
    );

    // Clsn1 of a standing animation has no attack boxes.
    sim.fighters[1].pos.x = 0.0;
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ClsnOverlap(clsn1, 1, clsn2)"),
        0
    );

    // Invalid or self targets are false.
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ClsnOverlap(clsn2, 99, clsn2)"),
        0
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ClsnOverlap(clsn2, 0, clsn2)"),
        0
    );
}

#[test]
fn env_shake_var_reads_active_shake() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "envshake".into(),
        params: vec![
            ("time".into(), "30".into()),
            ("freq".into(), "45".into()),
            ("ampl".into(), "-8".into()),
            ("dir".into(), "10".into()),
            ("phase".into(), "20".into()),
            ("diradd".into(), "3".into()),
            ("decay".into(), "1.5".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);

    assert_eq!(manato_sim::trigger::int(&sim, 0, "EnvShakeVar(time)"), 30);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "EnvShakeVar(freq)"), 45.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "EnvShakeVar(ampl)"), 8.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "EnvShakeVar(dir)"), 10.0);
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "EnvShakeVar(phase)"),
        20.0
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "EnvShakeVar(diradd)"),
        3.0
    );
    assert_eq!(manato_sim::trigger::num(&sim, 0, "EnvShakeVar(decay)"), 1.5);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "EnvShakeVar(unknown)"), 0);

    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "EnvShakeVar(time)"), 29);
}

#[test]
fn palfx_var_reads_effective_effect() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    // Identity values apply when no effect is active; `mul` uses 0..256 units.
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(time)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(add.r)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(mul.r)"), 256);

    let control = manato_formats::Controller {
        kind: "palfx".into(),
        params: vec![
            ("add".into(), "10, 20, 30".into()),
            ("mul".into(), "128, 64, 32".into()),
            ("sinadd".into(), "5, 0, 0, 4".into()),
            ("time".into(), "40".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);

    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(time)"), 40);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(add.r)"), 10);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(add.g)"), 20);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(add.b)"), 30);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(mul.r)"), 128);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(mul.g)"), 64);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(mul.b)"), 32);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(unknown)"), 0);

    // The reported add includes the active `sinadd` oscillation.
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.fighters[0].pal_add[0] - 15.0).abs() < 0.01);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(add.r)"), 15);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(time)"), 39);

    let off = manato_formats::Controller {
        kind: "palfx".into(),
        params: vec![("time".into(), "0".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &off, &assets);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(time)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(add.r)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalfxVar(mul.r)"), 256);
}

#[test]
fn transform_clsn_scales_collision_boxes_per_tick() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos = manato_core::Vec2::ZERO;
    sim.fighters[0].facing = 1;
    let box1 = manato_formats::CollisionBox {
        x1: -10,
        y1: -20,
        x2: 10,
        y2: 0,
    };
    let control = |params: Vec<(String, String)>| manato_formats::Controller {
        kind: "transformclsn".into(),
        params,
        line: 0,
    };
    manato_sim::controller::execute(
        &mut sim,
        0,
        &control(vec![
            ("group".into(), "2".into()),
            ("scale".into(), "2, 1".into()),
        ]),
        &assets,
    );
    let world = sim.fighters[0].world_clsn_box(&box1, 1, 0);
    assert_eq!((world.x1, world.y1, world.x2, world.y2), (-20, -20, 20, 0));
    // Clsn1 is a separate group.
    let untouched = sim.fighters[0].world_clsn_box(&box1, 0, 0);
    assert_eq!(
        (untouched.x1, untouched.y1, untouched.x2, untouched.y2),
        (-10, -20, 10, 0)
    );

    // Index-specific transforms stack on top of the all-boxes entry.
    manato_sim::controller::execute(
        &mut sim,
        0,
        &control(vec![
            ("group".into(), "2".into()),
            ("index".into(), "0".into()),
            ("scale".into(), "3, 2".into()),
        ]),
        &assets,
    );
    let first = sim.fighters[0].world_clsn_box(&box1, 1, 0);
    assert_eq!((first.x1, first.y1, first.x2, first.y2), (-60, -40, 60, 0));
    let second = sim.fighters[0].world_clsn_box(&box1, 1, 1);
    assert_eq!(
        (second.x1, second.y1, second.x2, second.y2),
        (-20, -20, 20, 0)
    );

    // Transforms are per-tick.
    sim.tick([PlayerInput::default(); 2]);
    let reset = sim.fighters[0].world_clsn_box(&box1, 1, 0);
    assert_eq!((reset.x1, reset.y1, reset.x2, reset.y2), (-10, -20, 10, 0));

    // group = 0 clears existing transforms immediately.
    manato_sim::controller::execute(
        &mut sim,
        0,
        &control(vec![
            ("group".into(), "-1".into()),
            ("scale".into(), "2, 2".into()),
        ]),
        &assets,
    );
    assert!(!sim.fighters[0].clsn_transforms[0].is_empty());
    assert!(!sim.fighters[0].clsn_transforms[1].is_empty());
    manato_sim::controller::execute(
        &mut sim,
        0,
        &control(vec![("group".into(), "0".into())]),
        &assets,
    );
    assert!(sim.fighters[0].clsn_transforms[0].is_empty());
    assert!(sim.fighters[0].clsn_transforms[1].is_empty());
}

#[test]
fn transform_sprite_sets_per_tick_render_transform() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "transformsprite".into(),
        params: vec![
            ("xshear".into(), "0.4".into()),
            ("focallength".into(), "512".into()),
            ("projection".into(), "perspective2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let fighter = &sim.fighters[0];
    assert_eq!(fighter.transform_x_shear, 0.4);
    assert_eq!(fighter.transform_focal_length, 512.0);
    assert_eq!(
        fighter.transform_projection,
        manato_sim::ProjectionMode::Perspective2
    );

    sim.tick([PlayerInput::default(); 2]);
    let fighter = &sim.fighters[0];
    assert_eq!(fighter.transform_x_shear, 0.0, "per-tick override");
    assert_eq!(fighter.transform_focal_length, 2048.0);
    assert_eq!(
        fighter.transform_projection,
        manato_sim::ProjectionMode::Orthographic
    );
}

#[test]
fn modify_stage_var_updates_camera_and_bounds() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "modifystagevar".into(),
        params: vec![
            ("camera.boundleft".into(), "-50".into()),
            ("camera.boundright".into(), "50".into()),
            ("camera.boundhigh".into(), "-40".into()),
            ("camera.boundlow".into(), "-5".into()),
            ("camera.verticalfollow".into(), "0.5".into()),
            ("camera.floortension".into(), "12".into()),
            ("camera.tension".into(), "80".into()),
            ("camera.cuthigh".into(), "-20".into()),
            ("camera.cutlow".into(), "-2".into()),
            ("camera.startzoom".into(), "0.8".into()),
            ("camera.zoomout".into(), "0.5".into()),
            ("camera.zoomin".into(), "2".into()),
            ("camera.zoomindelay".into(), "7".into()),
            ("camera.zoominspeed".into(), "0.25".into()),
            ("camera.zoomoutspeed".into(), "0.75".into()),
            ("playerinfo.leftbound".into(), "-80".into()),
            ("playerinfo.rightbound".into(), "80".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);

    let camera = &sim.stage_camera;
    assert_eq!(camera.bound_left, -50.0);
    assert_eq!(camera.bound_right, 50.0);
    assert_eq!(camera.bound_high, -40.0);
    assert_eq!(camera.bound_low, -5.0);
    assert_eq!(camera.vertical_follow, 0.5);
    assert_eq!(camera.floor_tension, 12.0);
    assert_eq!(camera.tension, 80.0);
    assert_eq!(camera.cut_high, -20.0);
    assert_eq!(camera.cut_low, -2.0);
    assert_eq!(camera.start_zoom, 0.8);
    assert_eq!(camera.zoom_out, 0.5);
    assert_eq!(camera.zoom_in, 2.0);
    assert_eq!(camera.zoom_in_delay, 7);
    assert_eq!(camera.zoom_in_speed, 0.25);
    assert_eq!(camera.zoom_out_speed, 0.75);
    assert_eq!(sim.stage_left_bound, -80.0);
    assert_eq!(sim.stage_right_bound, 80.0);

    // The new playable bounds feed the stage-edge trigger.
    sim.fighters[0].facing = 1;
    sim.fighters[0].pos.x = 0.0;
    sim.camera.x = 0.0;
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "StageFrontEdgeDist"),
        80.0
    );

    // A restart restores the stage's declared values.
    sim.restart_match();
    assert_eq!(sim.stage_left_bound, assets.stage.left_bound);
    assert_eq!(sim.stage_right_bound, assets.stage.right_bound);
    assert_eq!(sim.stage_camera.bound_left, assets.stage.camera.bound_left);
    assert_eq!(
        sim.stage_camera.zoom_in_delay,
        assets.stage.camera.zoom_in_delay
    );
}

#[test]
fn explod_bind_time_rebinds_matching_explods() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = manato_formats::Controller {
        kind: "explod".into(),
        params: vec![
            ("anim".into(), "10".into()),
            ("id".into(), "5".into()),
            ("pos".into(), "0, 0".into()),
            ("bindtime".into(), "0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &spawn, &assets);
    assert_eq!(sim.effects.len(), 1);
    assert!(!sim.effects[0].bound, "bindtime = 0 is not bound");

    let bind = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "explodbindtime".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    bind(
        &mut sim,
        vec![("id".into(), "5".into()), ("time".into(), "3".into())],
    );
    assert_eq!(sim.effects[0].bind_time, 3);
    assert!(sim.effects[0].bound);

    // The explod follows the owner while bound.
    sim.fighters[0].pos = manato_core::Vec2::new(40.0, -10.0);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.effects[0].bind_time, 2);
    let owner_pos = sim.fighters[0].pos;
    assert!((sim.effects[0].pos.x - owner_pos.x).abs() < 0.001);
    assert!((sim.effects[0].pos.y - owner_pos.y).abs() < 0.001);

    // A non-matching id leaves the explod untouched.
    bind(
        &mut sim,
        vec![("id".into(), "9".into()), ("time".into(), "5".into())],
    );
    assert_eq!(sim.effects[0].bind_time, 2);

    // `value` is the documented fallback for `time`; id < 0 matches all.
    bind(&mut sim, vec![("value".into(), "1".into())]);
    assert_eq!(sim.effects[0].bind_time, 1);
    sim.tick([PlayerInput::default(); 2]);
    assert!(!sim.effects[0].bound, "bind expires at zero");
}

#[test]
fn sprite_var_reports_current_frame_sprite() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].set_anim(0, &sim.assets.characters[0], None);
    let expected = {
        let char = &sim.assets.characters[0];
        let frame = sim.fighters[0]
            .current_frame(char)
            .expect("current animation frame");
        let sprite = char.sprites.get(frame.group, frame.index).expect("sprite");
        (
            sprite.group,
            sprite.index,
            sprite.width,
            sprite.height,
            sprite.axis_x,
            sprite.axis_y,
        )
    };

    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "SpriteVar(group)"),
        expected.0
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "SpriteVar(image)"),
        expected.1
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "SpriteVar(width)"),
        expected.2 as f32
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "SpriteVar(height)"),
        expected.3 as f32
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "SpriteVar(xoffset)"),
        expected.4 as f32
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "SpriteVar(yoffset)"),
        expected.5 as f32
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "SpriteVar(unknown)"), 0);
}

#[test]
fn last_player_id_tracks_latest_creation() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(manato_sim::trigger::int(&sim, 0, "LastPlayerID"), 1);
    let assets = sim.assets.clone();
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "4".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "LastPlayerID"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 2, "LastPlayerID"), 2);

    sim.restart_match();
    assert_eq!(manato_sim::trigger::int(&sim, 0, "LastPlayerID"), 1);
}

#[test]
fn index_and_hit_overridden_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Index"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "Index"), 1);
    let assets = sim.assets.clone();
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "4".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    assert_eq!(manato_sim::trigger::int(&sim, 2, "Index"), 2);
    sim.fighters[1].active = false;
    assert_eq!(
        manato_sim::trigger::int(&sim, 2, "Index"),
        1,
        "inactive fighters are skipped"
    );

    // HitOverride sets HitOverridden on the tick it intercepts the hit.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos = manato_core::Vec2::new(-10.0, 0.0);
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos = manato_core::Vec2::new(10.0, 0.0);
    sim.fighters[1].facing = -1;
    sim.fighters[1].hit_overrides[0] = Some(manato_sim::fighter::HitOverride {
        attr: "S, NA".into(),
        state: 0,
        time: 100,
        keepstate: false,
        forceguard: false,
    });
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    sim.fighters[0].hitdef = Some(hitdef());
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitOverridden"), 0);
    sim.fighters[1].hit_overridden = false;
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitOverridden"), 1);
    assert_eq!(sim.fighters[1].life, sim.fighters[1].life_max);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "HitOverridden"),
        0,
        "the flag is per-tick"
    );
}

#[test]
fn version_and_helper_index_exist_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MugenVersion(major)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MugenVersion(minor)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "IkemenVersion(major)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "IkemenVersion(minor)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "IkemenVersion(patch)"), 0);

    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        assets.characters[0].def.mugen_version = "1.1".into();
        assets.characters[1].def.mugen_version = "14.10.1999".into();
        assets.characters[0]
            .def
            .info
            .insert("ikemenversion".into(), "0.99.3".into());
    }
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MugenVersion(major)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MugenVersion(minor)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "MugenVersion(major)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "MugenVersion(minor)"), 5);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "IkemenVersion(major)"), 0);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "IkemenVersion(minor)"),
        99
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "IkemenVersion(patch)"), 3);

    // HelperIndexExist: index 0 is the fighter itself; helpers are counted in
    // creation order for that root player.
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HelperIndexExist(0)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HelperIndexExist(1)"), 0);
    let assets = sim.assets.clone();
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "4".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    drop(assets);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HelperIndexExist(1)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HelperIndexExist(2)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HelperIndexExist(1)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 2, "HelperIndexExist(0)"), 1);
}

#[test]
fn explod_var_reports_active_effect_fields() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "explod".into(),
        params: vec![
            ("anim".into(), "10".into()),
            ("id".into(), "3".into()),
            ("pos".into(), "10, 20".into()),
            ("vel".into(), "2, -1".into()),
            ("accel".into(), "0.5, 0".into()),
            ("scale".into(), "1.5, 0.75".into()),
            ("angle".into(), "30".into()),
            ("xangle".into(), "15".into()),
            ("yangle".into(), "10".into()),
            ("xshear".into(), "0.25".into()),
            ("sprpriority".into(), "7".into()),
            ("removetime".into(), "40".into()),
            ("pausemovetime".into(), "5".into()),
            ("ignorehitpause".into(), "1".into()),
            ("ontop".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    assert_eq!(sim.effects.len(), 1);
    let effect = sim.effects[0].clone();

    let num = |sim: &SimState, expr: &str| manato_sim::trigger::num(sim, 0, expr);
    assert_eq!(num(&sim, "ExplodVar(3, 0, pos.x)"), effect.pos.x);
    assert_eq!(num(&sim, "ExplodVar(3, 0, pos.y)"), effect.pos.y);
    assert_eq!(num(&sim, "ExplodVar(3, 0, vel.x)"), 2.0);
    assert_eq!(num(&sim, "ExplodVar(3, 0, vel.y)"), -1.0);
    assert_eq!(num(&sim, "ExplodVar(3, 0, accel.x)"), 0.5);
    assert_eq!(num(&sim, "ExplodVar(3, 0, scale.x)"), 1.5);
    assert_eq!(num(&sim, "ExplodVar(3, 0, scale.y)"), 0.75);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ExplodVar(3, 0, anim)"),
        10
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ExplodVar(3, 0, animelem)"),
        1
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ExplodVar(3, 0, animelemtime)"),
        0
    );
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "ExplodVar(3, 0, animprefix)").as_str(),
        "s"
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ExplodVar(3, 0, id)"), 3);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ExplodVar(3, 0, layerno)"),
        1
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ExplodVar(3, 0, sprpriority)"),
        7
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ExplodVar(3, 0, time)"),
        0
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ExplodVar(3, 0, removetime)"),
        40
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ExplodVar(3, 0, pausemovetime)"),
        5
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ExplodVar(3, 0, ignorehitpause)"),
        1
    );
    assert_eq!(num(&sim, "ExplodVar(3, 0, angle)"), 30.0);
    assert_eq!(num(&sim, "ExplodVar(3, 0, angle.x)"), 15.0);
    assert_eq!(num(&sim, "ExplodVar(3, 0, angle.y)"), 10.0);
    assert_eq!(num(&sim, "ExplodVar(3, 0, xshear)"), 0.25);

    assert_eq!(manato_sim::trigger::int(&sim, 0, "ExplodVar(-1, 0, id)"), 3);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "ExplodVar(-1, 0, id)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ExplodVar(99, 0, id)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ExplodVar(3, 9, id)"), 0);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ExplodVar(3, 0, unknown)"),
        0
    );
}

#[test]
fn helper_var_reports_helper_metadata() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, id: &str, helper_type: &str, keyctrl: &str| {
        let control = manato_formats::Controller {
            kind: "helper".into(),
            params: vec![
                ("stateno".into(), "0".into()),
                ("id".into(), id.into()),
                ("removetime".into(), "-1".into()),
                ("helpertype".into(), helper_type.into()),
                ("keyctrl".into(), keyctrl.into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    // Helpers report their own metadata through their slot.
    spawn(&mut sim, "9", "normal", "1");
    assert_eq!(sim.fighters.len(), 3);
    assert_eq!(manato_sim::trigger::int(&sim, 2, "HelperVar(id)"), 9);
    assert_eq!(
        manato_sim::trigger::int(&sim, 2, "HelperVar(helpertype)"),
        1
    );
    assert_eq!(manato_sim::trigger::int(&sim, 2, "HelperVar(keyctrl)"), 1);

    spawn(&mut sim, "11", "player", "0");
    assert_eq!(sim.fighters.len(), 4);
    assert_eq!(manato_sim::trigger::int(&sim, 3, "HelperVar(id)"), 11);
    assert_eq!(
        manato_sim::trigger::int(&sim, 3, "HelperVar(helpertype)"),
        2
    );
    assert_eq!(manato_sim::trigger::int(&sim, 3, "HelperVar(keyctrl)"), 0);

    // Players are not helpers; unknown fields return 0.
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HelperVar(id)"), 0);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "HelperVar(helpertype)"),
        0
    );
    assert_eq!(manato_sim::trigger::int(&sim, 2, "HelperVar(unknown)"), 0);
}

#[test]
fn proj_var_reports_active_projectile_fields() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].facing = 1;
    let spawn = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "3, -2".into()),
            ("accel".into(), "1, 0".into()),
            ("id".into(), "7".into()),
            ("projremovetime".into(), "50".into()),
            ("projscale".into(), "2, 0.5".into()),
            ("projangle".into(), "15".into()),
            ("projxshear".into(), "0.25".into()),
            ("projlayerno".into(), "1".into()),
            ("projsprpriority".into(), "4".into()),
            ("projstagebound".into(), "30".into()),
            ("projedgebound".into(), "20".into()),
            ("projshadow".into(), "10, 20, 30".into()),
            ("projmisstime".into(), "6".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &spawn, &assets);
    assert_eq!(sim.projectiles.len(), 1);
    let projectile = sim.projectiles[0].clone();

    let num = |sim: &SimState, expr: &str| manato_sim::trigger::num(sim, 0, expr);
    assert_eq!(num(&sim, "ProjVar(7, 0, pos.x)"), projectile.pos.x);
    assert_eq!(num(&sim, "ProjVar(7, 0, pos.y)"), projectile.pos.y);
    assert_eq!(num(&sim, "ProjVar(7, 0, vel.x)"), 3.0);
    assert_eq!(num(&sim, "ProjVar(7, 0, vel.y)"), -2.0);
    assert_eq!(num(&sim, "ProjVar(7, 0, accel.x)"), 1.0);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(7, 0, anim)"),
        1500
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(7, 0, animelem)"),
        1
    );
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "ProjVar(7, 0, animprefix)").as_str(),
        "s"
    );
    assert_eq!(num(&sim, "ProjVar(7, 0, facing)"), 1.0);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(7, 0, projid)"),
        7
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(7, 0, projlayerno)"),
        1
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(7, 0, projsprpriority)"),
        4
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(7, 0, projremovetime)"),
        50
    );
    assert_eq!(num(&sim, "ProjVar(7, 0, projstagebound)"), 30.0);
    assert_eq!(num(&sim, "ProjVar(7, 0, projedgebound)"), 20.0);
    assert_eq!(num(&sim, "ProjVar(7, 0, scale.x)"), 2.0);
    assert_eq!(num(&sim, "ProjVar(7, 0, scale.y)"), 0.5);
    assert_eq!(num(&sim, "ProjVar(7, 0, angle)"), 15.0);
    assert_eq!(num(&sim, "ProjVar(7, 0, xshear)"), 0.25);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjVar(7, 0, time)"), 0);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(7, 0, shadow.r)"),
        10
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(7, 0, shadow.g)"),
        20
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(7, 0, shadow.b)"),
        30
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(7, 0, projmisstime)"),
        6
    );

    // `id < 0` matches any projectile; missing id/index and unknown fields
    // yield 0.
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(-1, 0, projid)"),
        7
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(99, 0, projid)"),
        0
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(7, 5, projid)"),
        0
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "ProjVar(7, 0, unknown)"),
        0
    );
}

#[test]
fn attack_defence_and_draw_angle_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PowerMax"), 3000);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "GuardCount"), 0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "Attack"), 100.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "Defence"), 100.0);

    sim.fighters[0].attack.guard_count = 3;
    sim.fighters[0].attack_mul = 1.2;
    sim.fighters[0].defence_mul = 0.8;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "GuardCount"), 3);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "Attack"), 120.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "AttackMul"), 1.2);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "Defence"), 80.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "DefenceMul"), 80.0);

    // Angle only reports while AngleDraw is active.
    sim.fighters[0].draw_angle = 30.0;
    assert_eq!(manato_sim::trigger::num(&sim, 0, "Angle"), 0.0);
    sim.fighters[0].angle_draw = true;
    assert_eq!(manato_sim::trigger::num(&sim, 0, "Angle"), 30.0);
    sim.fighters[0].angle_draw_value = Some(-20.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "Angle"), -20.0);
}

#[test]
fn fight_time_counts_match_ticks_across_rounds() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "FightTime"),
        sim.frame as i32
    );
    let start = manato_sim::trigger::int(&sim, 0, "FightTime");
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(manato_sim::trigger::int(&sim, 0, "FightTime"), start + 3);

    // Time keeps running through the KO phase and the next round's intro.
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    for _ in 0..130 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.number, 2);
    let between_rounds = manato_sim::trigger::int(&sim, 0, "FightTime");
    assert!(
        between_rounds > start + 3,
        "FightTime is not reset between rounds"
    );

    // Restarting the match clears the counter.
    sim.restart_match();
    assert_eq!(manato_sim::trigger::int(&sim, 0, "FightTime"), 0);
}

#[test]
fn round_and_random_range_expressions() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let num = |sim: &SimState, expr: &str| manato_sim::trigger::num(sim, 0, expr);
    let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
    assert!(close(num(&sim, "Round(2.5, 0)"), 3.0));
    assert!(close(num(&sim, "Round(2.4, 0)"), 2.0));
    assert!(close(num(&sim, "Round(1.2345, 2)"), 1.23));
    assert!(close(num(&sim, "Round(-1.5, 0)"), -1.0));
    assert!(close(num(&sim, "Round(-1.6, 0)"), -2.0));

    assert_eq!(manato_sim::trigger::int(&sim, 0, "RandomRange(5, 5)"), 5);
    for frame in 0..20 {
        sim.frame = frame;
        for (lo, hi) in [(0, 1), (2, 10), (10, 2), (-4, 4)] {
            let (min, max) = (lo.min(hi), lo.max(hi));
            let value = manato_sim::trigger::int(&sim, 0, &format!("RandomRange({lo}, {hi})"));
            assert!(
                (min..=max).contains(&value),
                "RandomRange({lo}, {hi}) = {value} outside {min}..={max}"
            );
        }
    }

    // The function is deterministic for a given frame and fighter.
    sim.frame = 7;
    let first = manato_sim::trigger::int(&sim, 0, "RandomRange(100, 200)");
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "RandomRange(100, 200)"),
        first
    );
    let clone = sim.clone();
    assert_eq!(
        manato_sim::trigger::int(&clone, 1, "RandomRange(100, 200)"),
        manato_sim::trigger::int(&sim, 1, "RandomRange(100, 200)")
    );
}

#[test]
fn consecutive_wins_counts_match_sweeps() {
    // A 1-0 sweep increments the winner's streak.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ConsecutiveWins"), 0);
    sim.round.rounds_to_win = 1;
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    for _ in 0..130 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Over);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ConsecutiveWins"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "ConsecutiveWins"), 0);
    sim.fighters[0].owner = Some(1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ConsecutiveWins"), 0);

    // A 2-1 match is not a sweep, so the streak stays zero.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.round.rounds_to_win = 2;
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    for _ in 0..130 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.number, 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ConsecutiveWins"), 0);
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Fight {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Fight);
    sim.fighters[0].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    for _ in 0..130 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.number, 3);
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Fight {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Fight);
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    for _ in 0..130 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Over);
    assert_eq!(sim.round.wins, [2, 1]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ConsecutiveWins"), 0);
}

#[test]
fn win_and_decisive_round_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Win"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "WinKO"), 0);

    sim.round.phase = manato_sim::RoundPhase::Ko;
    sim.round.winner = Some(0);
    sim.round.time_over = false;
    sim.win_history[0].push(manato_sim::WinRecord {
        time_over: false,
        perfect: true,
        clutch: false,
    });
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Win"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "WinKO"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "WinTime"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "WinPerfect"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "WinClutch"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "Win"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "Lose"), 1);

    // Time-over win with a clutch (non-perfect) record.
    sim.round.time_over = true;
    sim.win_history[0].pop();
    sim.win_history[0].push(manato_sim::WinRecord {
        time_over: true,
        perfect: false,
        clutch: true,
    });
    assert_eq!(manato_sim::trigger::int(&sim, 0, "WinTime"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "WinKO"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "WinPerfect"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "WinClutch"), 1);

    // Helpers never report a match win.
    sim.fighters[0].owner = Some(1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Win"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "WinTime"), 0);

    // DecisiveRound: both sides at one win from the match.
    sim.fighters[0].owner = None;
    sim.round.rounds_to_win = 2;
    sim.round.wins = [1, 0];
    assert_eq!(manato_sim::trigger::int(&sim, 0, "DecisiveRound"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "DecisiveRound"), 0);
    sim.round.wins = [1, 1];
    assert_eq!(manato_sim::trigger::int(&sim, 0, "DecisiveRound"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "DecisiveRound"), 1);
}

#[test]
fn anim_elem_no_reads_present_and_wrapped_elements() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let anim =
        manato_formats::air::parse_air_str("[Begin Action 9200]\n9000,1, 0,0, 4\n9000,1, 0,0, 6\n")
            .into_iter()
            .next()
            .expect("animation");
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        assets.characters[0].animations.insert(9200, anim);
    }
    let char = &sim.assets.characters[0];
    sim.fighters[0].set_anim(9200, char, None);
    sim.fighters[0].anim_time = 4;
    sim.fighters[0].anim_clock = 4;
    sim.fighters[0].anim_frame = 1;

    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElemNo(0)"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElemNo(-1)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElemNo(-4)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElemNo(-5)"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElemNo(1)"), 2);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "AnimElemNo(6)"),
        1,
        "past the end wraps to the loop start"
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElemNo(15)"), 2);
}

#[test]
fn stage_bg_var_reads_layer_state_and_animation() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let anim =
        manato_formats::air::parse_air_str("[Begin Action 9100]\n9000,1, 0,0, 4\n9000,1, 0,0, 6\n")
            .into_iter()
            .next()
            .expect("animation");
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        assets.stage.animations.insert(9100, anim);
    }
    sim.stage_layers[0].action = Some(9100);

    let field = |sim: &SimState, id: i32, index: i32, name: &str| {
        manato_sim::trigger::eval_str(sim, 0, &format!("StageBGVar({id}, {index}, {name})"))
    };
    sim.bg_time = 0.0;
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "StageBGVar(0, 0, actionno)"),
        9100
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "StageBGVar(0, 0, animelem)"),
        1
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "StageBGVar(0, 0, animelemtime)"),
        0
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "StageBGVar(0, 0, animtime)"),
        -10
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "StageBGVar(0, 0, animloopcount)"),
        0
    );

    sim.bg_time = 4.0;
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "StageBGVar(0, 0, animelem)"),
        2
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "StageBGVar(0, 0, animelemtime)"),
        0
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "StageBGVar(0, 0, animtime)"),
        -6
    );

    sim.bg_time = 10.0;
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "StageBGVar(0, 0, animelem)"),
        1
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "StageBGVar(0, 0, animelemtime)"),
        0
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "StageBGVar(0, 0, animtime)"),
        -10
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "StageBGVar(0, 0, animloopcount)"),
        1
    );

    sim.bg_time = 0.0;
    assert_eq!(field(&sim, 0, 0, "delta.x").as_f32(), 1.0);
    assert_eq!(field(&sim, 0, 0, "delta.y").as_f32(), 1.0);
    assert_eq!(field(&sim, 0, 1, "start.y").as_f32(), -46.0);
    assert_eq!(field(&sim, 0, 1, "tile.y").as_i32(), 0);
    assert_eq!(field(&sim, 0, 1, "layerno").as_i32(), 0);
    assert_eq!(field(&sim, 0, 1, "id").as_i32(), 0);

    // No matching id/index or unknown field yields 0.
    assert_eq!(field(&sim, 99, 0, "delta.x").as_i32(), 0);
    assert_eq!(field(&sim, 0, 99, "delta.x").as_i32(), 0);
    assert_eq!(field(&sim, 0, 0, "unknown").as_i32(), 0);
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
            ai_sides: [false, false],
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
fn get_hitvar_hitshaketime_tracks_the_defender_freeze() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.pausetime = (3, 5);
    let origin = sim.fighters[0].pos;
    let facing = sim.fighters[0].facing;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, facing, true);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(hitshaketime)"),
        5
    );
    // The value is case-insensitive and counts down once per tick, stopping
    // at zero instead of going negative.
    for expected in [4, 3, 2, 1, 0] {
        sim.tick([PlayerInput::default(); 2]);
        assert_eq!(
            manato_sim::trigger::int(&sim, 1, "GetHitVar(HitShakeTime)"),
            expected
        );
    }
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitShakeOver"), 1);

    // A guarded contact observes the same defender shake timer.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.pausetime = (2, 6);
    hit.guard_damage = 1;
    hit.guard_damage_set = true;
    let origin = sim.fighters[0].pos;
    let facing = sim.fighters[0].facing;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, facing, true);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(hitshaketime)"),
        6
    );
}

#[test]
fn get_hitvar_fallcount_counts_and_resets_per_combo() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let character = &mut assets.characters[1];
        for number in [5070, 5100] {
            character.states.insert(
                number,
                manato_formats::StateDef {
                    number,
                    params: vec![
                        ("type".into(), "A".into()),
                        ("movetype".into(), "H".into()),
                        ("physics".into(), "N".into()),
                        ("anim".into(), "0".into()),
                    ],
                    controllers: Vec::new(),
                    line: 0,
                    source: String::new(),
                },
            );
        }
    }
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fallcount)"), 0);

    let character = &sim.assets.characters[1];
    sim.fighters[1].enter_state(5070, character, false);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fallcount)"), 1);
    sim.fighters[1].enter_state(5100, character, false);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fallcount)"), 2);

    // The count persists while the defender remains in hitstun.
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fallcount)"), 2);

    // Leaving hitstun ends the combo and clears the counter.
    sim.fighters[1].move_type = manato_sim::fighter::MoveType::Idle;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fallcount)"), 0);
}

#[test]
fn get_hitvar_fall_and_liedown_recovertimes() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let mut hit = hitdef();
    hit.fall = true;
    hit.fall_recovertime = 7;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(fall.recovertime)"),
        7
    );
    // Unguarded hits seed the liedown recovery time from data.liedown.time.
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(recovertime)"),
        60
    );

    // The liedown counter ticks down once per tick in state 5110.
    sim.fighters[1].state_no = 5110;
    sim.fighters[1].state_time = 1;
    sim.fighters[1].move_type = manato_sim::fighter::MoveType::Hit;
    for expected in [59, 58] {
        sim.tick([PlayerInput::default(); 2]);
        assert_eq!(
            manato_sim::trigger::int(&sim, 1, "GetHitVar(recovertime)"),
            expected
        );
    }

    // A guarded contact does not reset the existing liedown recovery time.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].hit.vars.down_recovertime = 12;
    let mut guarded = hitdef();
    guarded.guard_damage = 1;
    guarded.guard_damage_set = true;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &guarded, true, origin, 1, true);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(recovertime)"),
        12
    );

    // `fall.recovertime` defaults to 4 when the HitDef omits it.
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![("attr".into(), "S, NA".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    assert_eq!(
        sim.fighters[0]
            .hitdef
            .as_ref()
            .expect("hitdef")
            .fall_recovertime,
        4
    );
}

#[test]
fn hitdef_fall_kill_and_envshake_fields_apply_on_landing() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "10".into()),
            ("fall".into(), "1".into()),
            ("fall.damage".into(), "100".into()),
            ("fall.kill".into(), "0".into()),
            ("fall.envshake.time".into(), "10".into()),
            ("fall.envshake.freq".into(), "120".into()),
            ("fall.envshake.ampl".into(), "-6".into()),
            ("fall.envshake.phase".into(), "20".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert!(!parsed.fall_kill);
    assert_eq!(parsed.fall_envshake_time, 10);
    assert_eq!(parsed.fall_envshake_freq, 120.0);
    assert_eq!(parsed.fall_envshake_ampl, -6);
    assert_eq!(parsed.fall_envshake_phase, 20.0);

    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fall.kill)"), 0);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(fall.envshake.time)"),
        10
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 1, "GetHitVar(fall.envshake.freq)"),
        120.0
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(fall.envshake.ampl)"),
        -6
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 1, "GetHitVar(fall.envshake.phase)"),
        20.0
    );

    // Landing applies fall.damage without KO when fall.kill = 0, and starts
    // the stored fall envshake.
    sim.fighters[1].state_no = 5040;
    sim.fighters[1].state_time = 1;
    sim.fighters[1].physics = manato_sim::fighter::Physics::None;
    sim.fighters[1].pos = manato_core::Vec2::new(20.0, 24.0);
    sim.fighters[1].vel = manato_core::Vec2::new(0.0, 10.0);
    sim.fighters[1].life = 50;
    sim.fighters[1].hit.vars.fall_damage = 100;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].life, 1, "fall.kill = 0 prevents a KO");
    assert_eq!(sim.env_shake.initial_time, 10);
    assert_eq!(sim.env_shake.time, 9, "one tick was consumed on landing");
    assert_eq!(sim.env_shake.freq, 120.0);
    assert_eq!(sim.env_shake.ampl, -6.0);
    assert_eq!(sim.env_shake.phase, 20.0);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(fall.envshake.time)"),
        0
    );

    // Defaults: kill enabled, time 0, freq 60, ampl -4, phase derived.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let defaults = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![("attr".into(), "S, NA".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &defaults, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert!(parsed.fall_kill);
    assert_eq!(parsed.fall_envshake_time, 0);
    assert_eq!(parsed.fall_envshake_freq, 60.0);
    assert_eq!(parsed.fall_envshake_ampl, -4);
    assert!(parsed.fall_envshake_phase.is_nan());

    // Landing with fall.kill = 1 can KO.
    let mut kill = parsed;
    kill.fall = true;
    kill.fall_damage = 100;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &kill, false, origin, 1, true);
    sim.fighters[1].state_no = 5040;
    sim.fighters[1].state_time = 1;
    sim.fighters[1].physics = manato_sim::fighter::Physics::None;
    sim.fighters[1].pos = manato_core::Vec2::new(20.0, 24.0);
    sim.fighters[1].vel = manato_core::Vec2::new(0.0, 10.0);
    sim.fighters[1].life = 50;
    sim.fighters[1].hit.vars.fall_damage = 100;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].life, 0, "fall.kill = 1 allows a KO");
}

#[test]
fn hitdef_kill_and_guard_kill_control_ko() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "100, 100".into()),
            ("kill".into(), "0".into()),
            ("guard.kill".into(), "0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert!(!parsed.kill);
    assert!(!parsed.guard_kill);

    // A lethal unguarded hit is clamped at 1 life when kill = 0.
    sim.fighters[1].life = 20;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, 1);

    // A lethal guarded hit is clamped at 1 life when guard.kill = 0.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    sim.fighters[1].life = 20;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, true, origin, 1, true);
    assert_eq!(sim.fighters[1].life, 1);

    // Enabling the corresponding kill flag allows the same hit to KO.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let lethal = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "100, 100".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &lethal, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert!(parsed.kill);
    assert!(parsed.guard_kill);
    sim.fighters[1].life = 20;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, 0);

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    manato_sim::controller::execute(&mut sim, 0, &lethal, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    sim.fighters[1].life = 20;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, true, origin, 1, true);
    assert_eq!(sim.fighters[1].life, 0);
}

#[test]
fn hitdef_air_fall_defaults_and_applies_to_air_hits() {
    let parse = |sim: &mut SimState, assets: &manato_sim::Assets, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };

    // air.fall inherits fall when omitted.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let hit = parse(
        &mut sim,
        &assets,
        vec![("attr".into(), "S, NA".into()), ("fall".into(), "1".into())],
    );
    assert!(hit.fall);
    assert!(hit.air_fall);
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos.y = -20.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fall)"), 1);
    assert_eq!(sim.fighters[1].state_no, 5020);

    // An explicit air.fall overrides fall for airborne hits only.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let hit = parse(
        &mut sim,
        &assets,
        vec![
            ("attr".into(), "S, NA".into()),
            ("fall".into(), "1".into()),
            ("air.fall".into(), "0".into()),
        ],
    );
    assert!(hit.fall);
    assert!(!hit.air_fall);
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos.y = -20.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fall)"), 0);

    // air.fall alone does not make a grounded hit fall.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let hit = parse(
        &mut sim,
        &assets,
        vec![
            ("attr".into(), "S, NA".into()),
            ("air.fall".into(), "1".into()),
        ],
    );
    assert!(!hit.fall);
    assert!(hit.air_fall);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fall)"), 0);

    // A grounded hit still uses fall.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let hit = parse(
        &mut sim,
        &assets,
        vec![("attr".into(), "S, NA".into()), ("fall".into(), "1".into())],
    );
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fall)"), 1);
}

#[test]
fn hitdef_forcestand_defaults_and_forces_standing_reaction() {
    let parse = |sim: &mut SimState, assets: &manato_sim::Assets, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let rising = parse(
        &mut sim,
        &assets,
        vec![
            ("attr".into(), "S, NA".into()),
            ("ground.velocity".into(), "-4, -6".into()),
        ],
    );
    assert!(
        rising.forcestand,
        "non-zero ground Y velocity defaults to 1"
    );

    let flat = parse(
        &mut sim,
        &assets,
        vec![
            ("attr".into(), "S, NA".into()),
            ("ground.velocity".into(), "-4, 0".into()),
        ],
    );
    assert!(!flat.forcestand);

    let explicit_off = parse(
        &mut sim,
        &assets,
        vec![
            ("attr".into(), "S, NA".into()),
            ("ground.velocity".into(), "-4, -6".into()),
            ("forcestand".into(), "0".into()),
        ],
    );
    assert!(!explicit_off.forcestand);

    let explicit_on = parse(
        &mut sim,
        &assets,
        vec![
            ("attr".into(), "S, NA".into()),
            ("ground.velocity".into(), "-4, 0".into()),
            ("forcestand".into(), "1".into()),
        ],
    );
    assert!(explicit_on.forcestand);

    // A crouching defender uses the standing hit state when forced.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.forcestand = true;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Crouch;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 5000);
    assert_eq!(
        sim.fighters[1].state_type,
        manato_sim::fighter::StateType::Stand
    );

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.forcestand = false;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Crouch;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 5010);

    // Airborne defenders ignore forcestand.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.forcestand = true;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos.y = -20.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 5020);
    assert_eq!(
        sim.fighters[1].state_type,
        manato_sim::fighter::StateType::Air
    );
}

#[test]
fn hitdef_forcenofall_clears_existing_fall() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let default = parse(&mut sim, vec![("attr".into(), "S, NA".into())]);
    assert!(!default.forcenofall);
    let explicit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("forcenofall".into(), "1".into()),
        ],
    );
    assert!(explicit.forcenofall);

    // A non-fall hit preserves an existing fall state.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].hit.vars.fall = true;
    let mut hit = hitdef();
    hit.forcenofall = false;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fall)"), 1);

    // forcenofall clears it when this hit is not itself a fall.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].hit.vars.fall = true;
    let mut hit = hitdef();
    hit.forcenofall = true;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fall)"), 0);

    // A fall hit ignores forcenofall.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].hit.vars.fall = true;
    let mut hit = hitdef();
    hit.fall = true;
    hit.forcenofall = true;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fall)"), 1);

    // Ending hitstun clears the persistent fall state for the next combo.
    sim.fighters[1].move_type = manato_sim::fighter::MoveType::Idle;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(fall)"), 0);
}

#[test]
fn hitdef_down_velocity_falls_back_to_air_and_hits_liedown() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let inherited = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("air.velocity".into(), "-3, -4".into()),
        ],
    );
    assert_eq!(inherited.down_velocity, manato_core::Vec2::new(-3.0, -4.0));
    let explicit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("air.velocity".into(), "-3, -4".into()),
            ("down.velocity".into(), "-2, 0".into()),
        ],
    );
    assert_eq!(explicit.down_velocity, manato_core::Vec2::new(-2.0, 0.0));
    drop(assets);

    // Inject the downed hit state so entry is observable in bundled content.
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let character = &mut assets.characters[1];
        character.states.insert(
            5080,
            manato_formats::StateDef {
                number: 5080,
                params: vec![
                    ("type".into(), "L".into()),
                    ("movetype".into(), "H".into()),
                    ("physics".into(), "N".into()),
                    ("anim".into(), "0".into()),
                ],
                controllers: Vec::new(),
                line: 0,
                source: String::new(),
            },
        );
    }

    // A downed hit uses down.velocity and enters state 5080.
    let mut hit = hitdef();
    hit.down_velocity = manato_core::Vec2::new(-5.0, 0.0);
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Liedown;
    sim.fighters[1].state_no = 5100;
    sim.fighters[1].pos.y = 0.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 5080);
    assert_eq!(
        sim.fighters[1].hit.vars.x_vel, 5.0,
        "down.velocity mirrored"
    );
    assert_eq!(sim.fighters[1].hit.vars.y_vel, 0.0);

    // Hitting a downed opponent requires the D hitflag.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let character = &mut assets.characters[1];
        character.states.insert(
            5080,
            manato_formats::StateDef {
                number: 5080,
                params: vec![
                    ("type".into(), "L".into()),
                    ("movetype".into(), "H".into()),
                    ("physics".into(), "N".into()),
                    ("anim".into(), "0".into()),
                ],
                controllers: Vec::new(),
                line: 0,
                source: String::new(),
            },
        );
    }
    let assets = sim.assets.clone();
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Liedown;
    sim.fighters[1].state_no = 5100;
    sim.fighters[1].pos = manato_core::Vec2::new(10.0, 0.0);
    sim.fighters[1].facing = -1;
    sim.fighters[0].pos = manato_core::Vec2::new(-10.0, 0.0);
    sim.fighters[0].facing = 1;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    let mut hit = hitdef();
    hit.hitflag = "MAF".into();
    hit.down_velocity = manato_core::Vec2::new(-5.0, 0.0);
    sim.fighters[0].hitdef = Some(hit);
    let life = sim.fighters[1].life;
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(
        sim.fighters[1].life, life,
        "D flag is required for downed hits"
    );

    sim.fighters[0].hit.hit_targets.clear();
    sim.fighters[0].hit.hit_counts.clear();
    if let Some(hit) = sim.fighters[0].hitdef.as_mut() {
        hit.hitflag = "MAFD".into();
    }
    manato_sim::collision::resolve(&mut sim);
    assert!(sim.fighters[1].life < life);
    assert_eq!(sim.fighters[1].state_no, 5080);
}

#[test]
fn hitdef_down_cornerpush_defaults_and_applies() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let inherited = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("ground.cornerpush.veloff".into(), "-8".into()),
        ],
    );
    assert_eq!(inherited.down_cornerpush, -8.0);
    let explicit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("ground.cornerpush.veloff".into(), "-8".into()),
            ("down.cornerpush.veloff".into(), "-2".into()),
        ],
    );
    assert_eq!(explicit.down_cornerpush, -2.0);

    // A grounded downed hit near the corner uses down.cornerpush, not the
    // ground value.
    let mut hit = hitdef();
    hit.down_velocity = manato_core::Vec2::new(-5.0, 0.0);
    hit.ground_cornerpush = -8.0;
    hit.down_cornerpush = -2.0;
    let right = sim.assets.stage.right_bound;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Liedown;
    sim.fighters[1].state_no = 5100;
    sim.fighters[1].pos = manato_core::Vec2::new(right, 0.0);
    sim.fighters[1].facing = -1;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, 3.0, "5 + (-2) down push");
}

#[test]
fn hitdef_envshake_applies_on_successful_hits() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("envshake.time".into(), "8".into()),
            ("envshake.freq".into(), "120".into()),
            ("envshake.ampl".into(), "-6".into()),
            ("envshake.phase".into(), "45".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert_eq!(parsed.envshake_time, 8);
    assert_eq!(parsed.envshake_freq, 120.0);
    assert_eq!(parsed.envshake_ampl, -6);
    assert_eq!(parsed.envshake_phase, 45.0);

    // A successful unguarded heavy hit uses the explicit HitDef shake,
    // overriding the generic heavy-hit camera kick.
    let mut hit = hitdef();
    hit.damage = 100;
    hit.envshake_time = 8;
    hit.envshake_freq = 120.0;
    hit.envshake_ampl = -6;
    hit.envshake_phase = 45.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.env_shake.time, 8);
    assert_eq!(sim.env_shake.freq, 120.0);
    assert_eq!(sim.env_shake.ampl, -6.0);
    assert_eq!(sim.env_shake.phase, 45.0);
    assert_eq!(sim.env_shake.decay, 1.0);

    // Guarded contacts also count as successful hits for envshake.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.envshake_time = 8;
    hit.envshake_freq = 120.0;
    hit.envshake_ampl = -6;
    hit.envshake_phase = 45.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.env_shake.time, 8);

    // The omitted phase follows the EnvShake frequency rule.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.envshake_time = 5;
    hit.envshake_freq = 100.0;
    hit.envshake_phase = f32::NAN;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.env_shake.phase, 90.0);

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.envshake_time = 5;
    hit.envshake_freq = 50.0;
    hit.envshake_phase = f32::NAN;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.env_shake.phase, 0.0);

    // time = 0 leaves the shake alone.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.damage = 10;
    hit.envshake_time = 0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.env_shake.time, 0);
}

#[test]
fn hitdef_palfx_applies_to_target() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let default = parse(&mut sim, vec![("attr".into(), "S, NA".into())]);
    assert_eq!(default.palfx_time, 0);
    assert_eq!(default.palfx_mul, [1.0; 3]);
    assert_eq!(default.palfx_add, [0.0; 3]);

    let parsed = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("palfx.time".into(), "6".into()),
            ("palfx.mul".into(), "128, 256, 64".into()),
            ("palfx.add".into(), "32, 0, 8".into()),
        ],
    );
    assert_eq!(parsed.palfx_time, 6);
    assert_eq!(parsed.palfx_mul, [0.5, 1.0, 0.25]);
    assert_eq!(parsed.palfx_add, [32.0, 0.0, 8.0]);

    // Successful unguarded hit tints the defender.
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(sim.fighters[1].pal_time, 6);
    assert_eq!(sim.fighters[1].pal_mul, [0.5, 1.0, 0.25]);
    assert_eq!(sim.fighters[1].pal_add, [32.0, 0.0, 8.0]);
    assert_eq!(sim.fighters[1].pal_base_add, [32.0, 0.0, 8.0]);
    assert_eq!(sim.fighters[1].pal_alpha, 1.0);
    // `PalfxVar` reports the same effect in content units.
    assert_eq!(manato_sim::trigger::int(&sim, 1, "PalfxVar(mul.r)"), 128);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "PalfxVar(mul.g)"), 256);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "PalfxVar(add.r)"), 32);

    // Guarded contacts also apply the target palette effect.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("palfx.time".into(), "6".into()),
            ("palfx.mul".into(), "128, 256, 64".into()),
            ("palfx.add".into(), "32, 0, 8".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, true, origin, 1, true);
    assert_eq!(sim.fighters[1].pal_time, 6);
    assert_eq!(sim.fighters[1].pal_mul, [0.5, 1.0, 0.25]);

    // palfx.time = 0 leaves an existing effect untouched.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].pal_time = 3;
    sim.fighters[1].pal_mul = [0.9, 0.9, 0.9];
    let mut hit = hitdef();
    hit.palfx_time = 0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].pal_time, 3);
    assert_eq!(sim.fighters[1].pal_mul, [0.9, 0.9, 0.9]);
}

#[test]
fn hitdef_mindist_maxdist_clamp_post_hit_position() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let default = parse(&mut sim, vec![("attr".into(), "S, NA".into())]);
    assert!(default.mindist.x.is_nan());
    assert!(default.mindist.y.is_nan());
    assert!(default.maxdist.x.is_nan());
    assert!(default.maxdist.y.is_nan());

    let parsed = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("mindist".into(), "100".into()),
            ("maxdist".into(), "200, 10".into()),
        ],
    );
    assert_eq!(parsed.mindist.x, 100.0);
    assert!(parsed.mindist.y.is_nan());
    assert_eq!(parsed.maxdist, manato_core::Vec2::new(200.0, 10.0));

    // Min X and min Y pull the defender into range.
    let mut hit = hitdef();
    hit.mindist = manato_core::Vec2::new(100.0, -20.0);
    hit.maxdist = manato_core::Vec2::new(200.0, 10.0);
    sim.fighters[1].pos = manato_core::Vec2::new(50.0, -50.0);
    let origin = manato_core::Vec2::new(0.0, 0.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].pos, manato_core::Vec2::new(100.0, -20.0));

    // Max X and max Y constrain the far side.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.mindist = manato_core::Vec2::new(f32::NAN, f32::NAN);
    hit.maxdist = manato_core::Vec2::new(200.0, 10.0);
    sim.fighters[1].pos = manato_core::Vec2::new(300.0, 50.0);
    let origin = manato_core::Vec2::new(0.0, 0.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].pos, manato_core::Vec2::new(200.0, 10.0));

    // Facing left mirrors the X bounds.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.mindist = manato_core::Vec2::new(30.0, f32::NAN);
    hit.maxdist = manato_core::Vec2::new(f32::NAN, f32::NAN);
    sim.fighters[1].pos = manato_core::Vec2::new(90.0, 0.0);
    let origin = manato_core::Vec2::new(100.0, 0.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, -1, true);
    assert_eq!(sim.fighters[1].pos.x, 70.0);

    // Guarded grounded hits apply X but not the Y bounds.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.mindist = manato_core::Vec2::new(100.0, -20.0);
    hit.maxdist = manato_core::Vec2::new(f32::NAN, f32::NAN);
    sim.fighters[1].pos = manato_core::Vec2::new(50.0, 0.0);
    let origin = manato_core::Vec2::new(0.0, 0.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.fighters[1].pos, manato_core::Vec2::new(100.0, 0.0));

    // Omitted bounds leave the position unchanged.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let hit = hitdef();
    sim.fighters[1].pos = manato_core::Vec2::new(37.0, -12.0);
    let origin = manato_core::Vec2::new(0.0, 0.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].pos, manato_core::Vec2::new(37.0, -12.0));
}

#[test]
fn hitdef_snap_places_defender_relative_to_attacker() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let parsed = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("snap".into(), "50, -20".into()),
        ],
    );
    assert_eq!(parsed.snap, manato_core::Vec2::new(50.0, -20.0));
    assert_eq!(parsed.mindist, manato_core::Vec2::new(50.0, -20.0));
    assert_eq!(parsed.maxdist, manato_core::Vec2::new(50.0, -20.0));

    // Right-facing snap positions the defender exactly relative to P1.
    sim.fighters[1].pos = manato_core::Vec2::new(200.0, 50.0);
    let origin = manato_core::Vec2::new(0.0, 0.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(sim.fighters[1].pos, manato_core::Vec2::new(50.0, -20.0));

    // Left-facing snap mirrors X.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("snap".into(), "50, -20".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    sim.fighters[1].pos = manato_core::Vec2::new(0.0, 0.0);
    let origin = manato_core::Vec2::new(100.0, 0.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, -1, true);
    assert_eq!(sim.fighters[1].pos.x, 50.0);

    // Guarded grounded hits apply the X snap but not Y.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    sim.fighters[1].pos = manato_core::Vec2::new(200.0, 0.0);
    let origin = manato_core::Vec2::new(0.0, 0.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, true, origin, 1, true);
    assert_eq!(sim.fighters[1].pos, manato_core::Vec2::new(50.0, 0.0));

    // snap overrides explicit min/max on the same axis.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("maxdist".into(), "200, 0".into()),
            ("snap".into(), "40".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    sim.fighters[1].pos = manato_core::Vec2::new(200.0, 0.0);
    let origin = manato_core::Vec2::new(0.0, 0.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(sim.fighters[1].pos.x, 40.0);
}

#[test]
fn hitdef_guard_pausetime_defaults_and_applies() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let inherited = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("pausetime".into(), "3, 5".into()),
        ],
    );
    assert_eq!(inherited.guard_pausetime, (3, 5));

    let explicit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("pausetime".into(), "3, 5".into()),
            ("guard.pausetime".into(), "7, 9".into()),
        ],
    );
    assert_eq!(explicit.guard_pausetime, (7, 9));

    // Unguarded contacts use pausetime.
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &explicit, false, origin, 1, true);
    assert_eq!(sim.fighters[0].hit.hit_pause, 3);
    assert_eq!(sim.fighters[0].hit.hit_pause_time, 3);
    assert_eq!(sim.fighters[1].hit.hit_pause, 5);
    assert_eq!(sim.fighters[1].hit.hit_shake_timer, 5);

    // Guarded contacts use guard.pausetime for both players.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("pausetime".into(), "3, 5".into()),
            ("guard.pausetime".into(), "7, 9".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, true, origin, 1, true);
    assert_eq!(sim.fighters[0].hit.hit_pause, 7);
    assert_eq!(sim.fighters[0].hit.hit_pause_time, 7);
    assert_eq!(sim.fighters[1].hit.hit_pause, 9);
    assert_eq!(sim.fighters[1].hit.hit_shake_timer, 9);
}

#[test]
fn hitdef_facing_params_apply_to_normal_hits() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let default = parse(&mut sim, vec![("attr".into(), "S, NA".into())]);
    assert_eq!(default.p1getp2facing, 0);
    let parsed = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("p1getp2facing".into(), "1".into()),
        ],
    );
    assert_eq!(parsed.p1getp2facing, 1);

    // p1getp2facing = 1 copies P2's facing.
    let mut hit = hitdef();
    hit.p1getp2facing = 1;
    sim.fighters[0].facing = 1;
    sim.fighters[1].facing = -1;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[0].facing, -1);

    // -1 takes the opposite.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.p1getp2facing = -1;
    sim.fighters[0].facing = 1;
    sim.fighters[1].facing = -1;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[0].facing, 1);

    // p1getp2facing overrides p1facing.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.p1facing = 1;
    hit.p1getp2facing = 1;
    sim.fighters[0].facing = 1;
    sim.fighters[1].facing = -1;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[0].facing, -1);

    // p1facing and p2facing apply to normal hits, not only throws.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.p1facing = 1;
    hit.p2facing = -1;
    sim.fighters[0].facing = -1;
    sim.fighters[1].facing = 1;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[0].facing, 1);
    assert_eq!(sim.fighters[1].facing, -1);

    // Guarded contacts do not change facing.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.p1getp2facing = 1;
    sim.fighters[0].facing = 1;
    sim.fighters[1].facing = -1;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.fighters[0].facing, 1);
    assert_eq!(sim.fighters[1].facing, -1);
}

#[test]
fn hitdef_affectteam_filters_melee_and_projectile_hits() {
    use manato_sim::hitdef::TeamFilter;

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, value: &str| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params: vec![
                ("attr".into(), "S, NA".into()),
                ("affectteam".into(), value.into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef").affect_team
    };
    assert_eq!(parse(&mut sim, "E"), TeamFilter::Enemy);
    assert_eq!(parse(&mut sim, "b"), TeamFilter::Both);
    assert_eq!(parse(&mut sim, "F"), TeamFilter::Friendly);

    let melee = |sim: &mut SimState, team: TeamFilter| -> bool {
        sim.fighters[1].enter_state(0, &assets.characters[1], false);
        sim.fighters[1].life = sim.fighters[1].life_max;
        sim.fighters[1].pos = manato_core::Vec2::new(10.0, 0.0);
        sim.fighters[1].facing = -1;
        sim.fighters[0].hitdef = None;
        sim.fighters[0].hit.hit_targets.clear();
        sim.fighters[0].hit.hit_counts.clear();
        sim.fighters[0].pos = manato_core::Vec2::new(-10.0, 0.0);
        sim.fighters[0].facing = 1;
        sim.fighters[0].enter_state(200, &assets.characters[0], false);
        sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
        let mut hit = hitdef();
        hit.damage = 10;
        hit.affect_team = team;
        sim.fighters[0].hitdef = Some(hit);
        manato_sim::collision::resolve(sim);
        sim.fighters[1].life < sim.fighters[1].life_max
    };
    assert!(melee(&mut sim, TeamFilter::Enemy));
    assert!(melee(&mut sim, TeamFilter::Both));
    assert!(!melee(&mut sim, TeamFilter::Friendly));

    let projectile = |sim: &mut SimState, team: &str| -> bool {
        sim.projectiles.clear();
        sim.fighters[1].enter_state(0, &assets.characters[1], false);
        sim.fighters[1].life = sim.fighters[1].life_max;
        let control = manato_formats::Controller {
            kind: "projectile".into(),
            params: vec![
                ("projanim".into(), "1500".into()),
                ("velocity".into(), "0, 0".into()),
                ("removetime".into(), "60".into()),
                ("damage".into(), "10".into()),
                ("affectteam".into(), team.into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        if let Some(shot) = sim.projectiles.last_mut() {
            shot.pos = sim.fighters[1].pos;
        }
        manato_sim::collision::resolve_projectiles(sim);
        sim.fighters[1].life < sim.fighters[1].life_max
    };
    assert!(projectile(&mut sim, "E"));
    assert!(projectile(&mut sim, "B"));
    assert!(!projectile(&mut sim, "F"));
}

#[test]
fn hitdef_chainid_and_nochainid_gate_hits() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let default = parse(&mut sim, vec![("attr".into(), "S, NA".into())]);
    assert_eq!(default.nochainid, [-1, -1]);
    let parsed = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("chainid".into(), "5".into()),
            ("nochainid".into(), "3, 4".into()),
        ],
    );
    assert_eq!(parsed.chainid, 5);
    assert_eq!(parsed.nochainid, [3, 4]);
    let single = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("nochainid".into(), "3".into()),
        ],
    );
    assert_eq!(single.nochainid, [3, -1]);

    let melee = |sim: &mut SimState,
                 chainid: i32,
                 nochainid: [i32; 2],
                 hit_id: i32,
                 in_hit: bool|
     -> bool {
        sim.fighters[1].enter_state(0, &assets.characters[1], false);
        sim.fighters[1].life = sim.fighters[1].life_max;
        sim.fighters[1].hit.vars.hit_id = hit_id;
        sim.fighters[1].move_type = if in_hit {
            manato_sim::fighter::MoveType::Hit
        } else {
            manato_sim::fighter::MoveType::Idle
        };
        sim.fighters[1].pos = manato_core::Vec2::new(10.0, 0.0);
        sim.fighters[1].facing = -1;
        sim.fighters[0].hitdef = None;
        sim.fighters[0].hit.hit_targets.clear();
        sim.fighters[0].hit.hit_counts.clear();
        sim.fighters[0].pos = manato_core::Vec2::new(-10.0, 0.0);
        sim.fighters[0].facing = 1;
        sim.fighters[0].enter_state(200, &assets.characters[0], false);
        sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
        let mut hit = hitdef();
        hit.damage = 10;
        hit.chainid = chainid;
        hit.nochainid = nochainid;
        sim.fighters[0].hitdef = Some(hit);
        manato_sim::collision::resolve(sim);
        sim.fighters[1].life < sim.fighters[1].life_max
    };
    assert!(melee(&mut sim, -1, [-1, -1], 0, false));
    assert!(melee(&mut sim, 5, [-1, -1], 5, false));
    assert!(!melee(&mut sim, 5, [-1, -1], 6, false));
    assert!(!melee(&mut sim, -1, [5, -1], 5, true));
    assert!(melee(&mut sim, -1, [5, -1], 5, false));
    assert!(melee(&mut sim, -1, [5, -1], 6, true));

    let projectile =
        |sim: &mut SimState, chainid: i32, nochainid: &str, hit_id: i32, in_hit: bool| -> bool {
            sim.projectiles.clear();
            sim.fighters[1].enter_state(0, &assets.characters[1], false);
            sim.fighters[1].life = sim.fighters[1].life_max;
            sim.fighters[1].hit.vars.hit_id = hit_id;
            sim.fighters[1].move_type = if in_hit {
                manato_sim::fighter::MoveType::Hit
            } else {
                manato_sim::fighter::MoveType::Idle
            };
            let control = manato_formats::Controller {
                kind: "projectile".into(),
                params: vec![
                    ("projanim".into(), "1500".into()),
                    ("velocity".into(), "0, 0".into()),
                    ("removetime".into(), "60".into()),
                    ("damage".into(), "10".into()),
                    ("chainid".into(), chainid.to_string()),
                    ("nochainid".into(), nochainid.into()),
                ],
                line: 0,
            };
            manato_sim::controller::execute(sim, 0, &control, &assets);
            if let Some(shot) = sim.projectiles.last_mut() {
                shot.pos = sim.fighters[1].pos;
            }
            manato_sim::collision::resolve_projectiles(sim);
            sim.fighters[1].life < sim.fighters[1].life_max
        };
    assert!(projectile(&mut sim, 5, "-1", 5, false));
    assert!(!projectile(&mut sim, 5, "-1", 6, false));
    assert!(!projectile(&mut sim, -1, "5", 5, true));
    assert!(projectile(&mut sim, -1, "5", 5, false));
}

#[test]
fn hitdef_down_bounce_rebounds_once_on_ground_impact() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let default = parse(&mut sim, vec![("attr".into(), "S, NA".into())]);
    assert!(!default.down_bounce);
    let parsed = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("down.bounce".into(), "1".into()),
        ],
    );
    assert!(parsed.down_bounce);

    let mut hit = hitdef();
    hit.down_velocity = manato_core::Vec2::new(0.0, -8.0);
    hit.fall_xvel = 3.0;
    hit.fall_yvel = -5.0;
    hit.down_bounce = true;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Liedown;
    sim.fighters[1].state_no = 5100;
    sim.fighters[1].pos = manato_core::Vec2::new(20.0, 0.0);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert!(sim.fighters[1].hit.down_bounce);
    assert_eq!(sim.fighters[1].hit.vars.fall_x_vel, Some(-3.0));
    assert_eq!(sim.fighters[1].hit.vars.fall_y_vel, Some(-5.0));

    sim.fighters[1].state_no = 5030;
    sim.fighters[1].state_time = 1;
    sim.fighters[1].physics = manato_sim::fighter::Physics::Air;
    sim.fighters[1].move_type = manato_sim::fighter::MoveType::Hit;
    sim.fighters[1].pos.y = 25.0;
    sim.fighters[1].vel = manato_core::Vec2::new(0.0, 10.0);
    sim.fighters[0].hitdef = None;
    sim.fighters[0].hit.hit_targets.clear();
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].vel, manato_core::Vec2::new(-3.0, -5.0));
    assert!(!sim.fighters[1].hit.down_bounce);

    // down.bounce = 0 clears the fall rebound velocities.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.down_velocity = manato_core::Vec2::new(0.0, -8.0);
    hit.fall_xvel = 3.0;
    hit.fall_yvel = -5.0;
    hit.down_bounce = false;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Liedown;
    sim.fighters[1].state_no = 5100;
    sim.fighters[1].pos = manato_core::Vec2::new(20.0, 0.0);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert!(!sim.fighters[1].hit.down_bounce);
    assert_eq!(sim.fighters[1].hit.vars.fall_x_vel, None);
    assert_eq!(sim.fighters[1].hit.vars.fall_y_vel, Some(0.0));
    sim.fighters[1].state_no = 5030;
    sim.fighters[1].state_time = 1;
    sim.fighters[1].physics = manato_sim::fighter::Physics::Air;
    sim.fighters[1].pos.y = 25.0;
    sim.fighters[1].vel = manato_core::Vec2::new(0.0, 10.0);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].vel.y, 0.0);

    // down.bounce is ignored when down.velocity has no Y component.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.down_velocity = manato_core::Vec2::new(-4.0, 0.0);
    hit.down_bounce = true;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Liedown;
    sim.fighters[1].state_no = 5100;
    sim.fighters[1].pos = manato_core::Vec2::new(20.0, 0.0);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert!(!sim.fighters[1].hit.down_bounce);
}

#[test]
fn hitdef_down_hittime_applies_to_grounded_downed_hits() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let default = parse(&mut sim, vec![("attr".into(), "S, NA".into())]);
    assert_eq!(default.down_hittime, 20);
    let parsed = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("down.hittime".into(), "12".into()),
        ],
    );
    assert_eq!(parsed.down_hittime, 12);

    let mut hit = hitdef();
    hit.down_velocity = manato_core::Vec2::new(0.0, 0.0);
    hit.down_hittime = 12;
    hit.ground_hittime = 4;
    hit.ground_slidetime = 3;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Liedown;
    sim.fighters[1].state_no = 5100;
    sim.fighters[1].pos = manato_core::Vec2::new(20.0, 0.0);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.hittime, 12);
    assert_eq!(sim.fighters[1].hit.vars.slidetime, 3);
    assert_eq!(sim.fighters[1].hit.vars.ctrltime, 12);

    // Launched downed hits ignore down.hittime.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.down_velocity = manato_core::Vec2::new(0.0, -8.0);
    hit.down_hittime = 12;
    hit.ground_hittime = 4;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Liedown;
    sim.fighters[1].state_no = 5100;
    sim.fighters[1].pos = manato_core::Vec2::new(20.0, 0.0);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.hittime, 4);
    assert_eq!(sim.fighters[1].hit.vars.ctrltime, 4);
}

#[test]
fn hitdef_p2getp1state_controls_p2_state_bank() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        for (index, state_type) in [(0usize, "A"), (1usize, "L")] {
            assets.characters[index].states.insert(
                1234,
                manato_formats::StateDef {
                    number: 1234,
                    params: vec![
                        ("type".into(), state_type.into()),
                        ("movetype".into(), "H".into()),
                        ("physics".into(), "N".into()),
                        ("anim".into(), "0".into()),
                    ],
                    controllers: Vec::new(),
                    line: 0,
                    source: String::new(),
                },
            );
        }
    }
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let default = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("p2stateno".into(), "1234".into()),
        ],
    );
    assert!(default.p2getp1state);
    let explicit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("p2stateno".into(), "1234".into()),
            ("p2getp1state".into(), "0".into()),
        ],
    );
    assert!(!explicit.p2getp1state);

    // Default uses the attacker's state definition and animation bank.
    let mut throw = hitdef();
    throw.p1stateno = 800;
    throw.p2stateno = 1234;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &throw, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 1234);
    assert_eq!(
        sim.fighters[1].state_type,
        manato_sim::fighter::StateType::Air
    );
    assert_eq!(sim.fighters[1].anim_char_id, Some(0));

    // p2getp1state = 0 keeps the defender's own state data.
    throw.p2getp1state = false;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &throw, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 1234);
    assert_eq!(
        sim.fighters[1].state_type,
        manato_sim::fighter::StateType::Liedown
    );
    assert_eq!(sim.fighters[1].anim_char_id, None);
}

#[test]
fn p2getp1state_runs_attacker_state_controllers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        assets.characters[0].states.insert(
            4321,
            manato_formats::StateDef {
                number: 4321,
                params: vec![
                    ("type".into(), "S".into()),
                    ("movetype".into(), "H".into()),
                    ("physics".into(), "N".into()),
                    ("anim".into(), "0".into()),
                ],
                controllers: vec![manato_formats::Controller {
                    kind: "varset".into(),
                    params: vec![("v".into(), "0".into()), ("value".into(), "7".into())],
                    line: 0,
                }],
                line: 0,
                source: String::new(),
            },
        );
    }
    let mut throw = hitdef();
    throw.p1stateno = 800;
    throw.p2stateno = 4321;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &throw, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_char_id, Some(0));
    sim.fighters[0].hitdef = None;
    sim.fighters[0].hit.hit_targets.clear();
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].vars[0], 7);

    // With p2getp1state = 0 the attacker-only state has no controllers to run.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        assets.characters[0].states.insert(
            4321,
            manato_formats::StateDef {
                number: 4321,
                params: vec![
                    ("type".into(), "S".into()),
                    ("movetype".into(), "H".into()),
                    ("physics".into(), "N".into()),
                    ("anim".into(), "0".into()),
                ],
                controllers: vec![manato_formats::Controller {
                    kind: "varset".into(),
                    params: vec![("v".into(), "0".into()), ("value".into(), "7".into())],
                    line: 0,
                }],
                line: 0,
                source: String::new(),
            },
        );
    }
    let mut throw = hitdef();
    throw.p1stateno = 800;
    throw.p2stateno = 4321;
    throw.p2getp1state = false;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &throw, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_char_id, None);
    sim.fighters[0].hitdef = None;
    sim.fighters[0].hit.hit_targets.clear();
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].vars[0], 0);
}

#[test]
fn p2_state_override_clears_on_selfstate_and_get_hit() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        assets.characters[1].states.insert(
            5555,
            manato_formats::StateDef {
                number: 5555,
                params: vec![
                    ("type".into(), "C".into()),
                    ("movetype".into(), "I".into()),
                    ("physics".into(), "C".into()),
                    ("anim".into(), "0".into()),
                ],
                controllers: Vec::new(),
                line: 0,
                source: String::new(),
            },
        );
    }
    let assets = sim.assets.clone();
    sim.fighters[1].state_char_id = Some(0);
    sim.fighters[1].anim_char_id = Some(0);
    let self_state = manato_formats::Controller {
        kind: "selfstate".into(),
        params: vec![("value".into(), "5555".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &self_state, &assets);
    assert_eq!(sim.fighters[1].state_no, 5555);
    assert_eq!(sim.fighters[1].state_char_id, None);
    assert_eq!(sim.fighters[1].anim_char_id, None);
    assert_eq!(
        sim.fighters[1].state_type,
        manato_sim::fighter::StateType::Crouch
    );

    // A normal get-hit also returns the defender to its own state data.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].state_char_id = Some(0);
    sim.fighters[1].anim_char_id = Some(0);
    let hit = hitdef();
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_char_id, None);
    assert_eq!(sim.fighters[1].anim_char_id, None);
    assert!(sim.fighters[1].state_no >= 5000);
}

#[test]
fn hitdef_down_recover_and_recovertime_semantics() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let default = parse(&mut sim, vec![("attr".into(), "S, NA".into())]);
    assert!(default.down_recover);
    assert_eq!(default.down_recovertime, -1);
    let parsed = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("down.recover".into(), "0".into()),
            ("down.recovertime".into(), "15".into()),
        ],
    );
    assert!(!parsed.down_recover);
    assert_eq!(parsed.down_recovertime, 15);

    // Defaults seed from data.liedown.time and expose both GetHitVars.
    let mut hit = hitdef();
    hit.damage = 10;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(recovertime)"),
        60
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(down.recovertime)"),
        60
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(down.recover)"),
        1
    );

    // A downed defender cannot have recovery time increased by a later hit.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Liedown;
    sim.fighters[1].state_no = 5100;
    sim.fighters[1].hit.vars.down_recovertime = 10;
    let mut hit = hitdef();
    hit.down_recovertime = 20;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.down_recovertime, 10);
    hit.down_recovertime = 5;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.down_recovertime, 5);

    // Buttons accelerate the countdown only when down.recover is enabled.
    let recover = |sim: &mut SimState, recover: bool| {
        sim.fighters[1].state_no = 5110;
        sim.fighters[1].state_time = 1;
        sim.fighters[1].move_type = manato_sim::fighter::MoveType::Hit;
        sim.fighters[1].hit.vars.down_recover = recover;
        sim.fighters[1].hit.vars.down_recovertime = 10;
        sim.tick([
            PlayerInput::default(),
            PlayerInput::new(0, manato_core::Buttons::X),
        ]);
        sim.fighters[1].hit.vars.down_recovertime
    };
    assert_eq!(recover(&mut sim, true), 8);
    assert_eq!(recover(&mut sim, false), 9);
}

#[test]
fn liedown_recovery_enters_getup_state_at_zero() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        assets.characters[1].states.insert(
            5120,
            manato_formats::StateDef {
                number: 5120,
                params: vec![
                    ("type".into(), "L".into()),
                    ("movetype".into(), "I".into()),
                    ("physics".into(), "N".into()),
                    ("anim".into(), "0".into()),
                ],
                controllers: Vec::new(),
                line: 0,
                source: String::new(),
            },
        );
    }
    sim.fighters[1].state_no = 5110;
    sim.fighters[1].state_time = 1;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Liedown;
    sim.fighters[1].move_type = manato_sim::fighter::MoveType::Hit;
    sim.fighters[1].hit.vars.down_recovertime = 1;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].hit.vars.down_recovertime, 0);
    assert_eq!(sim.fighters[1].state_no, 5120);

    // With no 5120 state available, the fighter stays in 5110.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].state_no = 5110;
    sim.fighters[1].state_time = 1;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Liedown;
    sim.fighters[1].move_type = manato_sim::fighter::MoveType::Hit;
    sim.fighters[1].hit.vars.down_recovertime = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].state_no, 5110);
}

#[test]
fn hitdef_getpower_and_givepower_apply_on_hit_and_guard() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let default = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "100".into()),
        ],
    );
    assert_eq!(default.getpower, (70, 35));
    assert_eq!(default.givepower, (60, 30));

    let parsed = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "100".into()),
            ("getpower".into(), "20, 5".into()),
            ("givepower".into(), "10, 4".into()),
        ],
    );
    assert_eq!(parsed.getpower, (20, 5));
    assert_eq!(parsed.givepower, (10, 4));

    // Unguarded hit uses the primary values.
    sim.fighters[0].power = 0.0;
    sim.fighters[1].power = 0.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(sim.fighters[0].power, 20.0);
    assert_eq!(sim.fighters[1].power, 10.0);

    // Guarded hit uses the guard values.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "100".into()),
            ("getpower".into(), "20, 5".into()),
            ("givepower".into(), "10, 4".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    sim.fighters[0].power = 0.0;
    sim.fighters[1].power = 0.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, true, origin, 1, true);
    assert_eq!(sim.fighters[0].power, 5.0);
    assert_eq!(sim.fighters[1].power, 4.0);

    // Omitted values derive from damage with 0.7/0.6 multipliers.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "100".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    sim.fighters[0].power = 0.0;
    sim.fighters[1].power = 0.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(sim.fighters[0].power, 70.0);
    assert_eq!(sim.fighters[1].power, 60.0);
}

#[test]
fn guard_hits_drain_guard_points() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].guard_max = 100;
    sim.fighters[1].guard_points = 100;
    let mut hit = hitdef();
    hit.guard_damage = 5;
    hit.guard_damage_set = true;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.fighters[1].guard_points, 95);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.fighters[1].guard_points, 90);

    // Unguarded hits do not consume guard points.
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].guard_points, 90);
}

#[test]
fn hitdef_forcecrouch_forces_crouching_reaction() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let default = parse(&mut sim, vec![("attr".into(), "S, NA".into())]);
    assert!(!default.forcecrouch);
    let parsed = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("forcecrouch".into(), "1".into()),
        ],
    );
    assert!(parsed.forcecrouch);

    let mut hit = hitdef();
    hit.forcecrouch = true;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Stand;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 5010);
    assert_eq!(
        sim.fighters[1].state_type,
        manato_sim::fighter::StateType::Crouch
    );

    // Airborne defenders ignore forcecrouch.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.forcecrouch = true;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos.y = -20.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 5020);

    // forcestand takes precedence for a crouching defender.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.forcestand = true;
    hit.forcecrouch = true;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Crouch;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 5000);

    // forcecrouch applies to a standing defender when both are set.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.forcestand = true;
    hit.forcecrouch = true;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Stand;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 5010);
}

#[test]
fn guard_crush_triggers_and_blocks_guarding() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].guard_max = 100;
    sim.fighters[1].guard_points = 5;
    let mut hit = hitdef();
    hit.guard_damage = 10;
    hit.guard_damage_set = true;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.fighters[1].guard_points, 0);
    assert_eq!(sim.fighters[1].guard_crush_timer, 20);
    assert!(!sim.fighters[1].hit.guarding);
    assert_eq!(sim.fighters[1].state_no, 5000);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GuardBreak"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GuardPoints"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GuardPointsMax"), 100);

    // The no-guard window restores guard points when it expires.
    sim.fighters[1].guard_crush_timer = 1;
    sim.fighters[1].guard_points = 0;
    sim.fighters[0].hitdef = None;
    sim.fighters[0].hit.hit_targets.clear();
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].guard_crush_timer, 0);
    assert_eq!(sim.fighters[1].guard_points, 100);

    // While the crush timer is active, held-back input cannot guard.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[1].guard_max = 100;
    sim.fighters[1].guard_points = 100;
    sim.fighters[1].guard_crush_timer = 20;
    sim.fighters[1].enter_state(0, &assets.characters[1], false);
    sim.fighters[1].append_input(PlayerInput::new(manato_core::DIR_RIGHT, 0), 0);
    sim.fighters[1].pos = manato_core::Vec2::new(10.0, 0.0);
    sim.fighters[1].facing = -1;
    sim.fighters[0].pos = manato_core::Vec2::new(-10.0, 0.0);
    sim.fighters[0].facing = 1;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    let mut hit = hitdef();
    hit.damage = 10;
    hit.guardflag = "MA".into();
    sim.fighters[0].hitdef = Some(hit);
    let life = sim.fighters[1].life;
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, life - 10);
    assert!(!sim.fighters[1].hit.guarding);
}

#[test]
fn guard_points_regenerate_after_cooldown() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].guard_max = 100;
    sim.fighters[1].guard_points = 90;
    sim.fighters[1].guard_cooldown = 2;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].guard_cooldown, 1);
    assert_eq!(sim.fighters[1].guard_points, 90);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].guard_cooldown, 0);
    assert_eq!(sim.fighters[1].guard_points, 92);

    // Regeneration caps at the maximum.
    sim.fighters[1].guard_points = 99;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].guard_points, 100);

    // A guard-crush window suppresses regeneration.
    sim.fighters[1].guard_points = 50;
    sim.fighters[1].guard_crush_timer = 5;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].guard_crush_timer, 4);
    assert_eq!(sim.fighters[1].guard_points, 50);

    // Guarded hits start the 60-tick cooldown.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].guard_max = 100;
    sim.fighters[1].guard_points = 100;
    let mut hit = hitdef();
    hit.guard_damage = 5;
    hit.guard_damage_set = true;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.fighters[1].guard_cooldown, 60);
}

#[test]
fn guard_point_controllers_add_and_set() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[1].guard_max = 100;
    sim.fighters[1].guard_points = 50;
    let run = |sim: &mut SimState, kind: &str, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: kind.into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 1, &control, &assets);
    };
    run(
        &mut sim,
        "guardpointsset",
        vec![("value".into(), "75".into())],
    );
    assert_eq!(sim.fighters[1].guard_points, 75);
    run(
        &mut sim,
        "guardpointsset",
        vec![("value".into(), "200".into())],
    );
    assert_eq!(sim.fighters[1].guard_points, 100);
    run(
        &mut sim,
        "guardpointsset",
        vec![("value".into(), "-5".into())],
    );
    assert_eq!(sim.fighters[1].guard_points, 0);

    run(
        &mut sim,
        "guardpointsadd",
        vec![("value".into(), "10".into())],
    );
    assert_eq!(sim.fighters[1].guard_points, 10);

    // Non-absolute additions scale by final defence.
    sim.fighters[1].defence_mul = 2.0;
    run(
        &mut sim,
        "guardpointsadd",
        vec![("value".into(), "10".into())],
    );
    assert_eq!(sim.fighters[1].guard_points, 15);
    run(
        &mut sim,
        "guardpointsadd",
        vec![
            ("value".into(), "10".into()),
            ("absolute".into(), "1".into()),
        ],
    );
    assert_eq!(sim.fighters[1].guard_points, 25);

    // No guard meter means no effect.
    sim.fighters[1].guard_max = 0;
    run(
        &mut sim,
        "guardpointsset",
        vec![("value".into(), "50".into())],
    );
    assert_eq!(sim.fighters[1].guard_points, 25);
}

#[test]
fn get_hitvar_guardpoints_reports_live_meter() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].guard_max = 100;
    sim.fighters[1].guard_points = 42;
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(guardpoints)"),
        42
    );
    let mut hit = hitdef();
    hit.guard_damage = 5;
    hit.guard_damage_set = true;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(guardpoints)"),
        37
    );
}

#[test]
fn guard_break_set_controller_toggles_trigger() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "guardbreakset".into(),
        params: vec![("value".into(), "1".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &control, &assets);
    assert!(sim.fighters[1].guard_break);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GuardBreak"), 1);
    let clear = manato_formats::Controller {
        kind: "guardbreakset".into(),
        params: vec![("value".into(), "0".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &clear, &assets);
    assert!(!sim.fighters[1].guard_break);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GuardBreak"), 0);
}

#[test]
fn dizzy_points_deplete_and_trigger_stun() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].dizzy_max = 100;
    sim.fighters[1].dizzy_points = 100;
    let mut hit = hitdef();
    hit.damage = 10;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].dizzy_points, 82);

    sim.fighters[1].dizzy_points = 10;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].dizzy_points, 0);
    assert_eq!(sim.fighters[1].dizzy_timer, 60);
    assert!(sim.fighters[1].dizzy_flag);
    assert!(!sim.fighters[1].ctrl);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "Dizzy"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "DizzyPoints"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "DizzyPointsMax"), 100);

    // Guarded contacts do not consume dizzy points.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].dizzy_max = 100;
    sim.fighters[1].dizzy_points = 100;
    let mut hit = hitdef();
    hit.damage = 10;
    hit.guard_damage = 2;
    hit.guard_damage_set = true;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.fighters[1].dizzy_points, 100);

    // The stun restores points and control after its bounded window.
    sim.fighters[1].dizzy_timer = 1;
    sim.fighters[1].dizzy_points = 0;
    sim.fighters[0].hitdef = None;
    sim.fighters[0].hit.hit_targets.clear();
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].dizzy_timer, 0);
    assert_eq!(sim.fighters[1].dizzy_points, 100);
    assert!(!sim.fighters[1].dizzy_flag);
    assert!(sim.fighters[1].ctrl);

    // Controllers clamp the meter.
    let assets = sim.assets.clone();
    let run = |sim: &mut SimState, kind: &str, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: kind.into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 1, &control, &assets);
    };
    run(
        &mut sim,
        "dizzypointsset",
        vec![("value".into(), "40".into())],
    );
    assert_eq!(sim.fighters[1].dizzy_points, 40);
    run(
        &mut sim,
        "dizzypointsadd",
        vec![("value".into(), "10".into())],
    );
    assert_eq!(sim.fighters[1].dizzy_points, 50);
    run(
        &mut sim,
        "dizzypointsset",
        vec![("value".into(), "500".into())],
    );
    assert_eq!(sim.fighters[1].dizzy_points, 100);
}

#[test]
fn dizzy_set_controller_toggles_trigger() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "dizzyset".into(),
        params: vec![("value".into(), "1".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &control, &assets);
    assert!(sim.fighters[1].dizzy_flag);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "Dizzy"), 1);
    let clear = manato_formats::Controller {
        kind: "dizzyset".into(),
        params: vec![("value".into(), "0".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &clear, &assets);
    assert!(!sim.fighters[1].dizzy_flag);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "Dizzy"), 0);
}

#[test]
fn modify_player_adjusts_meter_maxima() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[1].life = 1000;
    sim.fighters[1].life_max = 1000;
    sim.fighters[1].guard_max = 100;
    sim.fighters[1].guard_points = 100;
    sim.fighters[1].dizzy_max = 100;
    sim.fighters[1].dizzy_points = 100;
    let control = manato_formats::Controller {
        kind: "modifyplayer".into(),
        params: vec![
            ("lifemax".into(), "500".into()),
            ("guardpointsmax".into(), "50".into()),
            ("dizzypointsmax".into(), "25".into()),
            ("attack".into(), "150".into()),
            ("defence".into(), "50".into()),
            ("ailevel".into(), "7".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &control, &assets);
    assert_eq!(sim.fighters[1].life, 500);
    assert_eq!(sim.fighters[1].life_max, 500);
    assert_eq!(sim.fighters[1].guard_max, 50);
    assert_eq!(sim.fighters[1].guard_points, 50);
    assert_eq!(sim.fighters[1].dizzy_max, 25);
    assert_eq!(sim.fighters[1].dizzy_points, 25);
    assert_eq!(sim.fighters[1].attack_mul, 1.5);
    assert_eq!(sim.fighters[1].defence_mul, 0.5);
    assert_eq!(sim.fighters[1].ai_level, 7);

    let zero = manato_formats::Controller {
        kind: "modifyplayer".into(),
        params: vec![
            ("guardpointsmax".into(), "0".into()),
            ("dizzypointsmax".into(), "0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &zero, &assets);
    assert_eq!(sim.fighters[1].guard_max, 0);
    assert_eq!(sim.fighters[1].guard_points, 0);
    assert_eq!(sim.fighters[1].dizzy_max, 0);
    assert_eq!(sim.fighters[1].dizzy_points, 0);
}

#[test]
fn target_guard_and_dizzy_point_add_controllers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].target = Some(1);
    sim.fighters[1].guard_max = 100;
    sim.fighters[1].guard_points = 50;
    sim.fighters[1].dizzy_max = 100;
    sim.fighters[1].dizzy_points = 50;
    let run = |sim: &mut SimState, kind: &str, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: kind.into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    run(
        &mut sim,
        "targetguardpointsadd",
        vec![("value".into(), "10".into())],
    );
    run(
        &mut sim,
        "targetdizzypointsadd",
        vec![("value".into(), "10".into())],
    );
    assert_eq!(sim.fighters[1].guard_points, 60);
    assert_eq!(sim.fighters[1].dizzy_points, 60);

    // Non-absolute additions scale by the target's defence.
    sim.fighters[1].defence_mul = 2.0;
    run(
        &mut sim,
        "targetguardpointsadd",
        vec![("value".into(), "10".into())],
    );
    run(
        &mut sim,
        "targetdizzypointsadd",
        vec![("value".into(), "10".into())],
    );
    assert_eq!(sim.fighters[1].guard_points, 65);
    assert_eq!(sim.fighters[1].dizzy_points, 65);

    // Absolute additions bypass defence and clamp at the maximum.
    run(
        &mut sim,
        "targetguardpointsadd",
        vec![
            ("value".into(), "100".into()),
            ("absolute".into(), "1".into()),
        ],
    );
    run(
        &mut sim,
        "targetdizzypointsadd",
        vec![
            ("value".into(), "100".into()),
            ("absolute".into(), "1".into()),
        ],
    );
    assert_eq!(sim.fighters[1].guard_points, 100);
    assert_eq!(sim.fighters[1].dizzy_points, 100);

    // No bound target leaves the meters unchanged.
    sim.fighters[0].target = None;
    run(
        &mut sim,
        "targetguardpointsadd",
        vec![("value".into(), "-10".into())],
    );
    run(
        &mut sim,
        "targetdizzypointsadd",
        vec![("value".into(), "-10".into())],
    );
    assert_eq!(sim.fighters[1].guard_points, 100);
    assert_eq!(sim.fighters[1].dizzy_points, 100);
}

#[test]
fn target_meter_setters_clamp_values() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].target = Some(1);
    sim.fighters[1].guard_max = 100;
    sim.fighters[1].dizzy_max = 100;
    let run = |sim: &mut SimState, kind: &str, value: &str| {
        let control = manato_formats::Controller {
            kind: kind.into(),
            params: vec![("value".into(), value.into())],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    run(&mut sim, "targetguardpointsset", "75");
    run(&mut sim, "targetdizzypointsset", "75");
    assert_eq!(sim.fighters[1].guard_points, 75);
    assert_eq!(sim.fighters[1].dizzy_points, 75);
    run(&mut sim, "targetguardpointsset", "500");
    run(&mut sim, "targetdizzypointsset", "500");
    assert_eq!(sim.fighters[1].guard_points, 100);
    assert_eq!(sim.fighters[1].dizzy_points, 100);
    run(&mut sim, "targetguardpointsset", "-5");
    run(&mut sim, "targetdizzypointsset", "-5");
    assert_eq!(sim.fighters[1].guard_points, 0);
    assert_eq!(sim.fighters[1].dizzy_points, 0);

    sim.fighters[0].target = None;
    run(&mut sim, "targetguardpointsset", "50");
    run(&mut sim, "targetdizzypointsset", "50");
    assert_eq!(sim.fighters[1].guard_points, 0);
    assert_eq!(sim.fighters[1].dizzy_points, 0);
}

#[test]
fn get_hitvar_dizzypoints_reports_live_meter() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].dizzy_max = 100;
    sim.fighters[1].dizzy_points = 42;
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(dizzypoints)"),
        42
    );
    let mut hit = hitdef();
    hit.damage = 10;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(dizzypoints)"),
        24
    );
}

#[test]
fn hitdef_animtype_air_and_fall_inheritance_and_selection() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };

    // air.animtype inherits animtype.
    let inherited = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("animtype".into(), "Medium".into()),
        ],
    );
    assert_eq!(
        inherited.air_anim_type,
        manato_sim::hitdef::AnimType::Medium
    );
    assert_eq!(inherited.fall_anim_type, manato_sim::hitdef::AnimType::Back);

    // fall.animtype follows Up/DiagUp air types, else Back.
    let up = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("air.animtype".into(), "Up".into()),
        ],
    );
    assert_eq!(up.fall_anim_type, manato_sim::hitdef::AnimType::Up);
    let diag = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("air.animtype".into(), "DiagUp".into()),
        ],
    );
    assert_eq!(diag.fall_anim_type, manato_sim::hitdef::AnimType::DiagUp);
    let explicit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("air.animtype".into(), "Up".into()),
            ("fall.animtype".into(), "Hard".into()),
        ],
    );
    assert_eq!(explicit.fall_anim_type, manato_sim::hitdef::AnimType::Heavy);

    // An airborne defender uses air.animtype.
    let mut hit = hitdef();
    hit.anim_type = manato_sim::hitdef::AnimType::Light;
    hit.air_anim_type = manato_sim::hitdef::AnimType::Heavy;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos.y = -20.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(animtype)"),
        manato_sim::hitdef::AnimType::Heavy.code()
    );

    // A fall hit selects fall.animtype.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.fall = true;
    hit.air_fall = true;
    hit.anim_type = manato_sim::hitdef::AnimType::Light;
    hit.air_anim_type = manato_sim::hitdef::AnimType::Light;
    hit.fall_anim_type = manato_sim::hitdef::AnimType::DiagUp;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(animtype)"),
        manato_sim::hitdef::AnimType::DiagUp.code()
    );

    // A grounded Back/Up type with no Y velocity degrades to Heavy.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.anim_type = manato_sim::hitdef::AnimType::Back;
    hit.air_anim_type = manato_sim::hitdef::AnimType::Back;
    hit.fall = false;
    hit.air_fall = false;
    hit.ground_velocity = manato_core::Vec2::new(-4.0, 0.0);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(animtype)"),
        manato_sim::hitdef::AnimType::Heavy.code()
    );

    // With a Y velocity the ground type is preserved.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.anim_type = manato_sim::hitdef::AnimType::Back;
    hit.air_anim_type = manato_sim::hitdef::AnimType::Back;
    hit.fall = false;
    hit.air_fall = false;
    hit.ground_velocity = manato_core::Vec2::new(-4.0, -6.0);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(animtype)"),
        manato_sim::hitdef::AnimType::Back.code()
    );
}

#[test]
fn hitdef_hitonce_defaults_on_for_throw_attr() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let throw = parse(&mut sim, vec![("attr".into(), "S, NT".into())]);
    assert!(throw.hitonce, "throw attributes default hitonce to 1");
    let normal = parse(&mut sim, vec![("attr".into(), "S, NA".into())]);
    assert!(!normal.hitonce);
    let explicit_off = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NT".into()),
            ("hitonce".into(), "0".into()),
        ],
    );
    assert!(!explicit_off.hitonce, "an explicit value wins");
    let explicit_on = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("hitonce".into(), "1".into()),
        ],
    );
    assert!(explicit_on.hitonce);
}

#[test]
fn get_hitvar_contact_offsets_record_relative_position() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(manato_sim::trigger::num(&sim, 1, "GetHitVar(xoff)"), 0.0);
    assert_eq!(manato_sim::trigger::num(&sim, 1, "GetHitVar(yoff)"), 0.0);
    sim.fighters[0].pos = manato_core::Vec2::new(-10.0, 0.0);
    sim.fighters[1].pos = manato_core::Vec2::new(10.0, -20.0);
    let hit = hitdef();
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::num(&sim, 1, "GetHitVar(xoff)"), 20.0);
    assert_eq!(manato_sim::trigger::num(&sim, 1, "GetHitVar(yoff)"), -20.0);

    // Snap clamps the position first, then the offsets report the result.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.snap = manato_core::Vec2::new(50.0, -20.0);
    hit.mindist = manato_core::Vec2::new(50.0, -20.0);
    hit.maxdist = manato_core::Vec2::new(50.0, -20.0);
    sim.fighters[0].pos = manato_core::Vec2::ZERO;
    sim.fighters[1].pos = manato_core::Vec2::new(200.0, 40.0);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].pos, manato_core::Vec2::new(50.0, -20.0));
    assert_eq!(manato_sim::trigger::num(&sim, 1, "GetHitVar(xoff)"), 50.0);
    assert_eq!(manato_sim::trigger::num(&sim, 1, "GetHitVar(yoff)"), -20.0);

    // Facing left keeps the offset signed relative to the attacker.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("snap".into(), "50, 0".into()),
        ],
        line: 0,
    };
    let assets = sim.assets.clone();
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let hit = sim.fighters[0].hitdef.clone().expect("hitdef");
    sim.fighters[0].pos = manato_core::Vec2::new(100.0, 0.0);
    sim.fighters[1].pos = manato_core::Vec2::ZERO;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, -1, true);
    assert_eq!(sim.fighters[1].pos.x, 50.0);
    assert_eq!(manato_sim::trigger::num(&sim, 1, "GetHitVar(xoff)"), -50.0);
}

#[test]
fn get_hitvar_fall_xvel_and_yvel_aliases() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.fall = true;
    hit.fall_xvel = 3.0;
    hit.fall_yvel = -5.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(
        manato_sim::trigger::num(&sim, 1, "GetHitVar(fall.xvel)"),
        -3.0
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 1, "GetHitVar(fall.yvel)"),
        -5.0
    );
    // The longer HitDef parameter spelling remains available.
    assert_eq!(
        manato_sim::trigger::num(&sim, 1, "GetHitVar(fall.yvelocity)"),
        -5.0
    );
}

#[test]
fn get_hit_var_set_writes_modeled_fields() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[1].guard_max = 100;
    sim.fighters[1].dizzy_max = 100;
    let control = manato_formats::Controller {
        kind: "gethitvarset".into(),
        params: vec![
            ("xvel".into(), "-4.5".into()),
            ("yvel".into(), "-6".into()),
            ("yaccel".into(), "0.7".into()),
            ("hittime".into(), "12".into()),
            ("slidetime".into(), "3".into()),
            ("ctrltime".into(), "4".into()),
            ("damage".into(), "33".into()),
            ("hitcount".into(), "2".into()),
            ("fallcount".into(), "1".into()),
            ("groundtype".into(), "2".into()),
            ("airtype".into(), "3".into()),
            ("animtype".into(), "4".into()),
            ("guarded".into(), "1".into()),
            ("fall".into(), "1".into()),
            ("fall.damage".into(), "15".into()),
            ("fall.kill".into(), "0".into()),
            ("fall.recover".into(), "0".into()),
            ("fall.recovertime".into(), "9".into()),
            ("fall.xvel".into(), "2.5".into()),
            ("fall.yvel".into(), "-5".into()),
            ("fall.envshake.time".into(), "8".into()),
            ("fall.envshake.freq".into(), "90".into()),
            ("fall.envshake.ampl".into(), "-3".into()),
            ("fall.envshake.phase".into(), "45".into()),
            ("down.recover".into(), "0".into()),
            ("down.recovertime".into(), "30".into()),
            ("guardpoints".into(), "40".into()),
            ("dizzypoints".into(), "60".into()),
            ("hitshaketime".into(), "5".into()),
            ("unknown.field".into(), "99".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &control, &assets);
    let f = &sim.fighters[1];
    assert_eq!(f.hit.vars.x_vel, -4.5);
    assert_eq!(f.hit.vars.y_vel, -6.0);
    assert_eq!(f.hit.vars.y_accel, 0.7);
    assert_eq!(f.hit.vars.hittime, 12);
    assert_eq!(f.hit.vars.slidetime, 3);
    assert_eq!(f.hit.vars.ctrltime, 4);
    assert_eq!(f.hit.vars.damage, 33);
    assert_eq!(f.hit.vars.hitcount, 2);
    assert_eq!(f.hit.vars.fallcount, 1);
    assert_eq!(f.hit.vars.ground_type, 2);
    assert_eq!(f.hit.vars.air_type, 3);
    assert_eq!(f.hit.vars.anim_type, 4);
    assert!(f.hit.vars.guarded);
    assert!(f.hit.vars.fall);
    assert_eq!(f.hit.vars.fall_damage, 15);
    assert!(!f.hit.vars.fall_kill);
    assert!(!f.hit.vars.fall_recover);
    assert_eq!(f.hit.vars.fall_recovertime, 9);
    assert_eq!(f.hit.vars.fall_x_vel, Some(2.5));
    assert_eq!(f.hit.vars.fall_y_vel, Some(-5.0));
    assert_eq!(f.hit.vars.fall_envshake_time, 8);
    assert_eq!(f.hit.vars.fall_envshake_freq, 90.0);
    assert_eq!(f.hit.vars.fall_envshake_ampl, -3);
    assert_eq!(f.hit.vars.fall_envshake_phase, 45.0);
    assert!(!f.hit.vars.down_recover);
    assert_eq!(f.hit.vars.down_recovertime, 30);
    assert_eq!(f.guard_points, 40);
    assert_eq!(f.dizzy_points, 60);
    assert_eq!(f.hit.hit_shake_timer, 5);

    // Omitted fields keep their previous values.
    let partial = manato_formats::Controller {
        kind: "gethitset_none".into(),
        params: vec![("xvel".into(), "1".into())],
        line: 0,
    };
    let before = sim.fighters[1].hit.vars.damage;
    manato_sim::controller::execute(&mut sim, 1, &partial, &assets);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, -4.5);
    assert_eq!(sim.fighters[1].hit.vars.damage, before);
}

#[test]
fn ikemen_only_anim_and_round_time_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let char = &sim.assets.characters[0];
    let expected_length = char.animation(0).map(|anim| anim.total_time()).unwrap_or(0);
    sim.fighters[0].set_anim(0, char, None);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "AnimLength"),
        expected_length
    );
    sim.fighters[0].anim_looped = 3;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimLoopCount"), 3);
    sim.fighters[0].sprite_priority = 7;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "SprPriority"), 7);

    sim.round.round_time = 60;
    sim.round.timer = 30 * manato_core::TICKS_PER_SECOND as i32;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "TimeRemaining"), 30);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "TimeTotal"), 60);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "TimeElapsed"), 30);
    sim.round.timer = 0;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "TimeRemaining"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "TimeElapsed"), 60);
}

#[test]
fn round_time_controllers_set_and_clamp_ticks() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.round.round_time = 60;
    sim.round.timer = 10 * manato_core::TICKS_PER_SECOND as i32;
    let run = |sim: &mut SimState, kind: &str, value: &str| {
        let control = manato_formats::Controller {
            kind: kind.into(),
            params: vec![("value".into(), value.into())],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    run(&mut sim, "roundtimeset", "1800");
    assert_eq!(sim.round.timer, 1800);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "RoundTime"), 30);
    run(&mut sim, "roundtimeadd", "300");
    assert_eq!(sim.round.timer, 2100);
    run(&mut sim, "roundtimeadd", "-999999");
    assert_eq!(sim.round.timer, 0);
    run(&mut sim, "roundtimeset", "999999");
    assert_eq!(sim.round.timer, 60 * manato_core::TICKS_PER_SECOND as i32);
}

#[test]
fn prev_state_and_move_type_triggers_track_last_change() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let set = |sim: &mut SimState, control: &manato_formats::Controller| {
        manato_sim::controller::execute(sim, 0, control, &assets);
    };
    let crouch = manato_formats::Controller {
        kind: "statetypeset".into(),
        params: vec![
            ("statetype".into(), "C".into()),
            ("movetype".into(), "A".into()),
        ],
        line: 0,
    };
    set(&mut sim, &crouch);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "StateType = C"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PrevStateType = S"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveType = A"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PrevMoveType = I"), 1);

    let air = manato_formats::Controller {
        kind: "statetypeset".into(),
        params: vec![
            ("statetype".into(), "A".into()),
            ("movetype".into(), "H".into()),
        ],
        line: 0,
    };
    set(&mut sim, &air);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "StateType = A"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PrevStateType = C"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveType = H"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PrevMoveType = A"), 1);

    // Entering a state records the type that was active beforehand.
    sim.fighters[0].state_type = manato_sim::fighter::StateType::Liedown;
    sim.fighters[0].move_type = manato_sim::fighter::MoveType::Idle;
    sim.fighters[0].enter_state(0, &assets.characters[0], false);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PrevStateType = L"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "StateType = S"), 1);
}

#[test]
fn stage_edge_distance_triggers_clamp_to_stage_bounds() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.stage_left_bound = -100.0;
    sim.stage_right_bound = 100.0;
    sim.camera.x = 0.0;
    sim.fighters[0].facing = 1;
    sim.fighters[0].pos.x = 0.0;
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "StageFrontEdgeDist"),
        100.0
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "StageBackEdgeDist"),
        100.0
    );
    assert_eq!(manato_sim::trigger::num(&sim, 0, "StageBackEdge"), 100.0);

    // Facing left swaps which stage bound is in front.
    sim.fighters[0].facing = -1;
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "StageFrontEdgeDist"),
        100.0
    );
    sim.fighters[0].pos.x = 50.0;
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "StageFrontEdgeDist"),
        150.0
    );
    assert_eq!(manato_sim::trigger::num(&sim, 0, "StageBackEdgeDist"), 50.0);

    // The camera screen edge wins when it is inside the stage bound.
    sim.fighters[0].facing = 1;
    sim.fighters[0].pos.x = 0.0;
    sim.stage_right_bound = 500.0;
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "StageFrontEdgeDist"),
        manato_core::GAME_WIDTH * 0.5
    );
}

#[test]
fn is_asserted_and_combo_count_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "IsAsserted(invisible)"),
        0
    );
    let control = manato_formats::Controller {
        kind: "assertspecial".into(),
        params: vec![
            ("invisible".into(), "1".into()),
            ("nobardisplay".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "IsAsserted(invisible)"),
        1
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "IsAsserted(nobardisplay)"),
        1
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "IsAsserted(noshadow)"), 0);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "IsAsserted(unknownflag)"),
        0
    );

    sim.combo[0].hits = 4;
    sim.fighters[0].side = 0;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ComboCount"), 4);
    sim.fighters[1].side = 1;
    sim.combo[1].hits = 2;
    assert_eq!(manato_sim::trigger::int(&sim, 1, "ComboCount"), 2);

    // Helpers report no combo count, matching Ikemen's teamside handling.
    sim.fighters[0].owner = Some(1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ComboCount"), 0);
}

#[test]
fn statedef_persist_flags_control_state_entry_resets() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let character = &mut assets.characters[0];
        for (number, params) in [
            (
                7100,
                vec![
                    ("type".into(), "S".into()),
                    ("movetype".into(), "I".into()),
                    ("physics".into(), "N".into()),
                    ("anim".into(), "0".into()),
                    ("hitdefpersist".into(), "1".into()),
                ],
            ),
            (
                7101,
                vec![
                    ("type".into(), "S".into()),
                    ("movetype".into(), "I".into()),
                    ("physics".into(), "N".into()),
                    ("anim".into(), "0".into()),
                    ("movehitpersist".into(), "1".into()),
                    ("hitcountpersist".into(), "1".into()),
                ],
            ),
        ] {
            character.states.insert(
                number,
                manato_formats::StateDef {
                    number,
                    params,
                    controllers: Vec::new(),
                    line: 0,
                    source: String::new(),
                },
            );
        }
    }
    let assets = sim.assets.clone();
    let fighter = &mut sim.fighters[0];
    fighter.hitdef = Some(hitdef());
    fighter.hit.hit_targets = vec![1];
    fighter.hit.hit_counts = vec![(1, 3)];
    fighter.attack.move_contact = true;
    fighter.attack.move_hit = true;
    fighter.attack.move_guarded = true;
    fighter.attack.move_reversed = true;
    fighter.attack.hit_count = 4;
    fighter.attack.uniq_hit_count = 5;
    fighter.attack.guard_count = 6;

    // hitdefpersist keeps the active HitDef and its per-activation targets.
    fighter.enter_state(7100, &assets.characters[0], false);
    assert!(fighter.hitdef.is_some());
    assert_eq!(fighter.hit.hit_targets, vec![1]);
    assert_eq!(fighter.hit.hit_counts, vec![(1, 3)]);
    assert!(!fighter.attack.move_hit);
    assert_eq!(fighter.attack.hit_count, 0);

    fighter.attack.move_contact = true;
    fighter.attack.move_hit = true;
    fighter.attack.move_guarded = true;
    fighter.attack.move_reversed = true;
    fighter.attack.hit_count = 4;
    fighter.attack.uniq_hit_count = 5;
    fighter.attack.guard_count = 6;

    // movehitpersist/hitcountpersist keep Move* flags and hit counters, while
    // the HitDef itself is cleared.
    fighter.enter_state(7101, &assets.characters[0], false);
    assert!(fighter.hitdef.is_none());
    assert!(fighter.hit.hit_targets.is_empty());
    assert!(fighter.attack.move_contact);
    assert!(fighter.attack.move_hit);
    assert!(fighter.attack.move_guarded);
    assert!(fighter.attack.move_reversed);
    assert_eq!(fighter.attack.hit_count, 4);
    assert_eq!(fighter.attack.uniq_hit_count, 5);
    assert_eq!(fighter.attack.guard_count, 6);

    // Defaults clear everything again.
    fighter.enter_state(0, &assets.characters[0], false);
    assert!(fighter.hitdef.is_none());
    assert!(!fighter.attack.move_hit);
    assert_eq!(fighter.attack.hit_count, 0);
}

#[test]
fn victory_quote_controller_and_selection() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        assets.characters[0].quotes = vec![(0, "First quote.".into()), (2, "Third quote.".into())];
    }
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "victoryquote".into(),
        params: vec![("value".into(), "2".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    drop(assets);
    assert_eq!(sim.fighters[0].win_quote, 2);

    assert_eq!(sim.victory_quote_text(), None, "quotes only show at Over");
    sim.round.phase = manato_sim::RoundPhase::Over;
    sim.round.winner = Some(0);
    assert_eq!(sim.victory_quote_text(), Some("Third quote."));

    // An unknown index falls back to the first available quote.
    sim.fighters[0].win_quote = 7;
    assert_eq!(sim.victory_quote_text(), Some("First quote."));

    // A character without quotes yields nothing.
    sim.fighters[0].win_quote = 0;
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        assets.characters[0].quotes.clear();
    }
    assert_eq!(sim.victory_quote_text(), None);
}

#[test]
fn get_hitvar_hittime_counts_down_after_shake_to_minus_one() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.ground_hittime = 3;
    hit.pausetime = (0, 0);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(hittime)"), 3);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitOver"), 0);

    for expected in [2, 1, 0, -1, -1] {
        sim.tick([PlayerInput::default(); 2]);
        assert_eq!(
            manato_sim::trigger::int(&sim, 1, "GetHitVar(hittime)"),
            expected
        );
        assert_eq!(
            manato_sim::trigger::int(&sim, 1, "HitOver"),
            i32::from(expected < 0)
        );
    }

    // A hit shake delays the countdown until it expires.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.ground_hittime = 4;
    hit.pausetime = (0, 2);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(hittime)"), 4);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(hittime)"),
        4,
        "hit pause/shake keeps hittime frozen"
    );
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(hittime)"), 4);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(hittime)"), 3);
}

#[test]
fn score_controllers_and_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].side = 0;
    sim.fighters[1].side = 1;
    let run = |sim: &mut SimState, who: usize, kind: &str, value: &str| {
        let control = manato_formats::Controller {
            kind: kind.into(),
            params: vec![("value".into(), value.into())],
            line: 0,
        };
        manato_sim::controller::execute(sim, who, &control, &assets);
    };
    run(&mut sim, 0, "scoreadd", "25.5");
    assert_eq!(sim.score[0], 25.5);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "Score"), 25.5);
    sim.score_rounds[0] = 100.0;
    assert_eq!(manato_sim::trigger::num(&sim, 0, "ScoreTotal"), 125.5);

    // TargetScoreAdd credits the bound target's side.
    sim.fighters[0].target = Some(1);
    run(&mut sim, 0, "targetscoreadd", "7");
    assert_eq!(sim.score[1], 7.0);
    assert_eq!(manato_sim::trigger::num(&sim, 1, "Score"), 7.0);

    // Helpers do not hold score.
    sim.fighters[0].owner = Some(1);
    run(&mut sim, 0, "scoreadd", "10");
    assert_eq!(sim.score[0], 25.5);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "Score"), 0.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "ScoreTotal"), 0.0);
}

#[test]
fn red_life_model_controllers_and_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[1].life_max = 1000;
    sim.fighters[1].life = 1000;
    sim.fighters[1].red_life = 1000;
    let mut hit = hitdef();
    hit.damage = 100;
    hit.pausetime = (0, 0);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, 900);
    assert_eq!(sim.fighters[1].red_life, 925, "75% of damage is red loss");
    assert_eq!(manato_sim::trigger::int(&sim, 1, "RedLife"), 925);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(redlife)"), 925);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].red_life, 850);

    let run = |sim: &mut SimState, who: usize, kind: &str, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: kind.into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, who, &control, &assets);
    };
    run(
        &mut sim,
        1,
        "redlifeadd",
        vec![
            ("value".into(), "20".into()),
            ("absolute".into(), "1".into()),
        ],
    );
    assert_eq!(sim.fighters[1].red_life, 870);
    run(
        &mut sim,
        1,
        "redlifeset",
        vec![("value".into(), "500".into())],
    );
    assert_eq!(sim.fighters[1].red_life, 800, "red clamps at life");
    run(
        &mut sim,
        1,
        "redlifeset",
        vec![("value".into(), "2000".into())],
    );
    assert_eq!(sim.fighters[1].red_life, 1000, "red clamps at life max");

    // Non-absolute additions scale by defence.
    sim.fighters[1].red_life = 800;
    sim.fighters[1].defence_mul = 2.0;
    run(
        &mut sim,
        1,
        "redlifeadd",
        vec![("value".into(), "10".into())],
    );
    assert_eq!(sim.fighters[1].red_life, 805);

    // TargetRedLifeAdd applies to the bound target.
    sim.fighters[0].target = Some(1);
    run(
        &mut sim,
        0,
        "targetredlifeadd",
        vec![
            ("value".into(), "15".into()),
            ("absolute".into(), "1".into()),
        ],
    );
    assert_eq!(sim.fighters[1].red_life, 820);
}

#[test]
fn received_and_self_state_exist_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.damage = 20;
    hit.numhits = 2;
    hit.pausetime = (0, 0);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "ReceivedHits"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "ReceivedDamage"), 20);

    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "SelfStateNoExist(200)"),
        1
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "SelfStateNoExist(999999)"),
        0
    );
}

#[test]
fn map_controllers_and_trigger() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let run = |sim: &mut SimState, kind: &str, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: kind.into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    run(
        &mut sim,
        "mapset",
        vec![
            ("map".into(), "\"timer\"".into()),
            ("value".into(), "5".into()),
        ],
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Map(timer)"), 5);
    run(
        &mut sim,
        "mapadd",
        vec![("map".into(), "timer".into()), ("value".into(), "3".into())],
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Map(timer)"), 8);

    // `map(name) = value` form.
    run(&mut sim, "mapset", vec![("map(score)".into(), "10".into())]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Map(score)"), 10);

    // Float promotion on add.
    run(
        &mut sim,
        "mapadd",
        vec![
            ("map".into(), "timer".into()),
            ("value".into(), "1.5".into()),
        ],
    );
    assert_eq!(manato_sim::trigger::num(&sim, 0, "Map(timer)"), 9.5);

    // String values round-trip.
    run(
        &mut sim,
        "mapset",
        vec![
            ("map".into(), "greeting".into()),
            ("value".into(), "\"hello\"".into()),
        ],
    );
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "Map(greeting)").as_str(),
        "hello"
    );

    // MapReset with an exclusion keeps matching substring keys only.
    run(&mut sim, "mapreset", vec![("exclude".into(), "ti".into())]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Map(timer)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Map(score)"), 10);
    run(&mut sim, "mapreset", Vec::new());
    assert_eq!(sim.fighters[0].map.len(), 0);
}

#[test]
fn relative_map_controllers_write_parent_root_and_team() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let run = |sim: &mut SimState, kind: &str, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: kind.into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    sim.fighters[0].parent = Some(1);
    sim.fighters[0].root = Some(1);
    run(
        &mut sim,
        "parentmapset",
        vec![
            ("map".into(), "shared".into()),
            ("value".into(), "4".into()),
        ],
    );
    assert_eq!(
        sim.fighters[1].map.get("shared").map(|v| v.as_i32()),
        Some(4)
    );
    assert!(!sim.fighters[0].map.contains_key("shared"));
    run(
        &mut sim,
        "rootmapadd",
        vec![
            ("map".into(), "shared".into()),
            ("value".into(), "2".into()),
        ],
    );
    assert_eq!(
        sim.fighters[1].map.get("shared").map(|v| v.as_i32()),
        Some(6)
    );

    // Team maps apply to every player on the same side, not helpers.
    sim.fighters[0].side = 0;
    sim.fighters[1].side = 0;
    run(
        &mut sim,
        "teammapset",
        vec![("map(team)".into(), "9".into())],
    );
    assert_eq!(sim.fighters[0].map.get("team").map(|v| v.as_i32()), Some(9));
    assert_eq!(sim.fighters[1].map.get("team").map(|v| v.as_i32()), Some(9));

    // Helpers are not player-side map targets.
    sim.fighters[1].owner = Some(0);
    run(
        &mut sim,
        "teammapadd",
        vec![("map".into(), "team".into()), ("value".into(), "1".into())],
    );
    assert_eq!(
        sim.fighters[0].map.get("team").map(|v| v.as_i32()),
        Some(10)
    );
    assert_eq!(sim.fighters[1].map.get("team").map(|v| v.as_i32()), Some(9));
}

#[test]
fn player_count_and_existence_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumPlayer"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PlayerNo"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "PlayerNo"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PlayerIDExist(1)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PlayerIDExist(99)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PlayerIndexExist(1)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PlayerIndexExist(99)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PlayerNoExist(1)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PlayerNoExist(2)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PlayerNoExist(3)"), 0);

    // Helpers count towards NumPlayer and inherit the owner's PlayerNo.
    let assets = sim.assets.clone();
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "5".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    assert_eq!(sim.fighters.len(), 3);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumPlayer"), 3);
    assert_eq!(manato_sim::trigger::int(&sim, 2, "PlayerNo"), 1);
}

#[test]
fn prev_anim_tracks_the_last_animation_change() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let char = &sim.assets.characters[0];
    assert!(char.animation(200).is_some());
    assert!(char.animation(210).is_some());
    sim.fighters[0].set_anim(200, char, None);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PrevAnim"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Anim"), 200);

    sim.fighters[0].set_anim(210, char, None);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PrevAnim"), 200);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Anim"), 210);

    // ChangeAnim goes through the same setter.
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "changeanim".into(),
        params: vec![("value".into(), "200".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PrevAnim"), 210);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Anim"), 200);
}

#[test]
fn height_controller_sets_per_tick_offsets() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "height".into(),
        params: vec![("value".into(), "30, 20".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    assert_eq!(
        sim.fighters[0].height_player,
        manato_core::Vec2::new(30.0, 20.0)
    );
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.fighters[0].height_player,
        manato_core::Vec2::ZERO,
        "Height is a per-tick override"
    );
}

#[test]
fn ikemen_math_and_localcoord_expressions() {
    use std::f32::consts::PI;

    let Some(sim) = fight_phase() else {
        return;
    };
    let num = |expr: &str| manato_sim::trigger::num(&sim, 0, expr);
    let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
    assert!(close(num(&format!("Sin({})", PI / 2.0)), 1.0));
    assert!(close(num(&format!("Cos({})", PI)), -1.0));
    assert!(close(num("Atan(1)"), PI / 4.0));
    assert!(close(num("Atan2(1, 0)"), PI / 2.0));
    assert!(close(num("Asin(1)"), PI / 2.0));
    assert!(close(num("Acos(0)"), PI / 2.0));
    assert!(close(num("Rad(180)"), PI));
    assert!(close(num(&format!("Deg({})", PI)), 180.0));
    assert!(close(num("Clamp(5, 0, 3)"), 3.0));
    assert!(close(num("Lerp(0, 10, 0.5)"), 5.0));
    assert!(close(num("Lerp(0, 10, 2)"), 10.0));
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Sign(-4)"), -1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Sign(0)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Sign(9)"), 1);

    assert_eq!(manato_sim::trigger::int(&sim, 0, "LocalCoordX"), 320);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "LocalCoordY"), 240);
    assert!(close(
        manato_sim::trigger::num(&sim, 0, "Const1080p(movement.yaccel)"),
        0.44 * 4.5
    ));
}

#[test]
fn stage_time_tracks_ticks_since_round_start() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let start = manato_sim::trigger::int(&sim, 0, "StageTime");
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(manato_sim::trigger::int(&sim, 0, "StageTime"), start + 3);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "StageTime"), start + 3);
}

#[test]
fn camera_controller_switches_view_and_targets() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(sim.camera.view, manato_sim::CameraView::Fighting);
    let assets = sim.assets.clone();
    let run = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "camera".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    run(
        &mut sim,
        vec![
            ("view".into(), "free".into()),
            ("pos".into(), "1000, 0".into()),
        ],
    );
    assert_eq!(sim.camera.view, manato_sim::CameraView::Free);
    assert_eq!(sim.camera.free_pos, manato_core::Vec2::new(1000.0, 0.0));
    let before = sim.camera.x;
    for _ in 0..30 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.camera.x > before, "free view moves toward pos");

    run(
        &mut sim,
        vec![
            ("view".into(), "follow".into()),
            ("followid".into(), "1".into()),
        ],
    );
    assert_eq!(sim.camera.view, manato_sim::CameraView::Follow);
    assert_eq!(sim.camera.follow, Some(1));
    sim.fighters[1].pos.x = 120.0;
    for _ in 0..60 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.camera.x > 40.0, "follow view tracks the target");

    run(&mut sim, vec![("view".into(), "fighting".into())]);
    assert_eq!(sim.camera.view, manato_sim::CameraView::Fighting);
    assert_eq!(sim.camera.follow, None);

    // Follow mode defaults to the controller's owner.
    run(&mut sim, vec![("view".into(), "follow".into())]);
    assert_eq!(sim.camera.follow, Some(0));
}

#[test]
fn modify_hit_def_updates_active_fields() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let base = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "20".into()),
            ("guardflag".into(), "MA".into()),
            ("ground.velocity".into(), "-4, 0".into()),
            ("air.velocity".into(), "-5, -6".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &base, &assets);
    let modify = manato_formats::Controller {
        kind: "modifyhitdef".into(),
        params: vec![
            ("damage".into(), "50".into()),
            ("guardflag".into(), "L".into()),
            ("ground.velocity".into(), "-9, 0".into()),
            ("numhits".into(), "3".into()),
            ("id".into(), "7".into()),
            ("pausetime".into(), "3, 5".into()),
            ("guard.pausetime".into(), "1, 2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    let hit = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert_eq!(hit.damage, 50);
    assert_eq!(hit.guardflag, "L");
    assert_eq!(hit.ground_velocity, manato_core::Vec2::new(-9.0, 0.0));
    assert_eq!(hit.air_velocity, manato_core::Vec2::new(-5.0, -6.0));
    assert_eq!(hit.numhits, 3);
    assert_eq!(hit.id, 7);
    assert_eq!(hit.pausetime, (3, 5));
    assert_eq!(hit.guard_pausetime, (1, 2));
    assert_eq!(hit.hitflag, "MAF", "unmodified fields stay");

    // Without an active HitDef the controller is a no-op.
    sim.fighters[0].hitdef = None;
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    assert!(sim.fighters[0].hitdef.is_none());
}

#[test]
fn team_membership_triggers_for_single_member_teams() {
    let Some(sim) = fight_phase() else {
        return;
    };
    assert_eq!(manato_sim::trigger::int(&sim, 0, "TeamSize"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "TeamSize"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "TeamLeader"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "TeamLeader"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MemberNo"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "MemberNo"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Standby"), 0);
}

#[test]
fn modify_reversal_def_updates_attr_and_guardflags() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let reversal = manato_formats::Controller {
        kind: "reversaldef".into(),
        params: vec![
            ("reversal.attr".into(), "S, NA".into()),
            ("reversal.guardflag".into(), "A".into()),
            ("damage".into(), "15".into()),
            ("time".into(), "5".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &reversal, &assets);
    let parsed = sim.fighters[1].reversal.clone().expect("reversal");
    assert_eq!(parsed.attr.state_types, "S");
    assert_eq!(parsed.attr.class, "NA");
    assert_eq!(parsed.reversal_guardflag, "A");

    let modify = manato_formats::Controller {
        kind: "modifyreversaldef".into(),
        params: vec![
            ("reversal.attr".into(), "S, SA".into()),
            ("reversal.guardflag".into(), "MA".into()),
            ("reversal.guardflag.not".into(), "L".into()),
            ("damage".into(), "25".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &modify, &assets);
    let parsed = sim.fighters[1].reversal.clone().expect("reversal");
    assert_eq!(parsed.attr.class, "SA");
    assert_eq!(parsed.reversal_guardflag, "MA");
    assert_eq!(parsed.reversal_guardflag_not, "L");
    assert_eq!(parsed.damage, 25);

    // Guardflag gating: no intersection falls through to a normal hit.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -10.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 10.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    sim.fighters[0].hitdef = Some(hitdef());
    let mut rev = hitdef();
    rev.attr = manato_sim::hitdef::HitAttr::parse("S, NA");
    rev.reversal_guardflag = "L".into();
    sim.fighters[1].reversal = Some(rev.clone());
    sim.fighters[1].reversal_time = 5;
    let p2_life = sim.fighters[1].life;
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, p2_life - 20);

    // Matching guardflag reverses; guardflag.not can veto it.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -10.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 10.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    sim.fighters[0].hitdef = Some(hitdef());
    rev.reversal_guardflag = "MA".into();
    sim.fighters[1].reversal = Some(rev.clone());
    sim.fighters[1].reversal_time = 5;
    let p2_life = sim.fighters[1].life;
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, p2_life, "matching guardflag reverses");

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -10.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 10.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    sim.fighters[0].hitdef = Some(hitdef());
    rev.reversal_guardflag_not = "M".into();
    sim.fighters[1].reversal = Some(rev);
    sim.fighters[1].reversal_time = 5;
    let p2_life = sim.fighters[1].life;
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, p2_life - 20, "guardflag.not vetoes");
}

#[test]
fn hit_def_var_reports_active_hitdef_fields() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitDefVar(hitdamage)"), 0);
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "33".into()),
            ("guard.damage".into(), "7".into()),
            ("guardflag".into(), "L".into()),
            ("pausetime".into(), "3, 5".into()),
            ("sparkno".into(), "12".into()),
            ("sparkxy".into(), "-8, -40".into()),
            ("hitsound".into(), "5, 1".into()),
            ("id".into(), "9".into()),
            ("priority".into(), "2, Miss".into()),
            ("yaccel".into(), "0.7".into()),
            ("ground.velocity".into(), "-6, -2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "HitDefVar(hitdamage)"),
        33
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "HitDefVar(guarddamage)"),
        7
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "HitDefVar(guardflag) = L"),
        1
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitDefVar(pausetime)"), 3);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitDefVar(shaketime)"), 5);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitDefVar(sparkno)"), 12);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "HitDefVar(sparkx)"), -8.0);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "HitDefVar(hitsound.group)"),
        5
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitDefVar(id)"), 9);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitDefVar(priority)"), 2);
    assert!((manato_sim::trigger::num(&sim, 0, "HitDefVar(yaccel)") - 0.7).abs() < 1e-4);
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "HitDefVar(ground.velocity.x)"),
        -6.0
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitDefVar(unknown)"), 0);
}

#[test]
fn modify_shadow_sets_per_tick_overrides() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "modifyshadow".into(),
        params: vec![
            ("intensity".into(), "128".into()),
            ("color".into(), "10, 20, 30".into()),
            ("offset".into(), "4, -2".into()),
            ("xscale".into(), "1.5".into()),
            ("yscale".into(), "0.5".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let fighter = &sim.fighters[0];
    assert_eq!(fighter.shadow_intensity, Some(128));
    assert_eq!(fighter.shadow_color, Some([10, 20, 30]));
    assert_eq!(fighter.shadow_offset, manato_core::Vec2::new(4.0, -2.0));
    assert_eq!(fighter.shadow_xscale, Some(1.5));
    assert_eq!(fighter.shadow_yscale, Some(0.5));

    sim.tick([PlayerInput::default(); 2]);
    let fighter = &sim.fighters[0];
    assert_eq!(fighter.shadow_intensity, None, "per-tick override");
    assert_eq!(fighter.shadow_color, None);
    assert_eq!(fighter.shadow_offset, manato_core::Vec2::ZERO);
    assert_eq!(fighter.shadow_xscale, None);
    assert_eq!(fighter.shadow_yscale, None);
}

#[test]
fn character_metadata_and_stage_bg_triggers() {
    let Some(sim) = fight_phase() else {
        return;
    };
    let character = sim.character(0);
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "AuthorName").as_str(),
        character.def.author
    );
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "DisplayName").as_str(),
        character.display_name()
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "NumStageBG"),
        sim.stage_layers.len() as i32
    );
    assert!(manato_sim::trigger::int(&sim, 0, "NumStageBG") > 0);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "NumStageBG(-1)"),
        sim.stage_layers.len() as i32
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "NumStageBG(0)"),
        sim.assets
            .stage
            .layers
            .iter()
            .filter(|layer| layer.id == 0)
            .count() as i32
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumStageBG(77)"), 0);
}

#[test]
fn modify_reflection_sets_per_tick_overrides() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "modifyreflection".into(),
        params: vec![
            ("intensity".into(), "96".into()),
            ("color".into(), "30, 40, 50".into()),
            ("offset".into(), "2, -3".into()),
            ("xscale".into(), "0.75".into()),
            ("yscale".into(), "1.25".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let fighter = &sim.fighters[0];
    assert_eq!(fighter.reflection_intensity, Some(96));
    assert_eq!(fighter.reflection_color, Some([30, 40, 50]));
    assert_eq!(fighter.reflection_offset, manato_core::Vec2::new(2.0, -3.0));
    assert_eq!(fighter.reflection_xscale, Some(0.75));
    assert_eq!(fighter.reflection_yscale, Some(1.25));

    sim.tick([PlayerInput::default(); 2]);
    let fighter = &sim.fighters[0];
    assert_eq!(fighter.reflection_intensity, None, "per-tick override");
    assert_eq!(fighter.reflection_color, None);
    assert_eq!(fighter.reflection_offset, manato_core::Vec2::ZERO);
    assert_eq!(fighter.reflection_xscale, None);
    assert_eq!(fighter.reflection_yscale, None);
}

#[test]
fn angle_controllers_track_persistent_and_per_tick_draw_state() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let run = |sim: &mut SimState, kind: &str, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: kind.into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    run(&mut sim, "angleset", vec![("value".into(), "30".into())]);
    assert_eq!(sim.fighters[0].draw_angle, 30.0);
    run(&mut sim, "angleadd", vec![("value".into(), "15".into())]);
    assert_eq!(sim.fighters[0].draw_angle, 45.0);
    run(&mut sim, "anglemul", vec![("value".into(), "2".into())]);
    assert_eq!(sim.fighters[0].draw_angle, 90.0);

    run(
        &mut sim,
        "angledraw",
        vec![("scale".into(), "2, 0.5".into())],
    );
    assert!(sim.fighters[0].angle_draw);
    assert_eq!(sim.fighters[0].angle_draw_value, None);
    assert_eq!(
        sim.fighters[0].angle_draw_scale,
        Some(manato_core::Vec2::new(2.0, 0.5))
    );
    run(&mut sim, "angledraw", vec![("value".into(), "-20".into())]);
    assert_eq!(sim.fighters[0].angle_draw_value, Some(-20.0));

    sim.tick([PlayerInput::default(); 2]);
    assert!(!sim.fighters[0].angle_draw, "per-tick activation");
    assert_eq!(sim.fighters[0].angle_draw_value, None);
    assert_eq!(sim.fighters[0].angle_draw_scale, None);
    assert_eq!(sim.fighters[0].draw_angle, 90.0, "angle persists");
}

#[test]
fn attack_dist_updates_active_guard_distance() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let hitdef = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("guard.dist".into(), "120".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &hitdef, &assets);
    assert_eq!(sim.fighters[0].hitdef.as_ref().unwrap().guard_dist, 120);

    let attack_dist = manato_formats::Controller {
        kind: "attackdist".into(),
        params: vec![("value".into(), "45".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &attack_dist, &assets);
    assert_eq!(sim.fighters[0].hitdef.as_ref().unwrap().guard_dist, 45);

    // No active HitDef means no change and no panic.
    sim.fighters[0].hitdef = None;
    manato_sim::controller::execute(&mut sim, 0, &attack_dist, &assets);
    assert!(sim.fighters[0].hitdef.is_none());
}

#[test]
fn hit_pause_time_reports_self_pause_only() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.pausetime = (10, 11);
    let origin = sim.fighters[0].pos;
    let facing = sim.fighters[0].facing;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, facing, true);

    // Oracle `kfm_crouch_attack`: the attacker's state-400 HitDef has
    // pausetime 10,11 and reports HitPauseTime 10 on contact, counting down
    // to 0. The defender stays frozen by its own hit shake but reports 0.
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitPauseTime"), 10);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitPauseTime"), 0);
    assert_eq!(sim.fighters[0].hit.hit_pause, 10);
    assert_eq!(sim.fighters[1].hit.hit_pause, 11);

    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitPauseTime"), 9);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitPauseTime"), 0);

    for _ in 0..9 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitPauseTime"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitPauseTime"), 0);
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
        (
            "nohardcodedkeys",
            manato_sim::fighter::ASSERT_NOHARDCODEDKEYS,
        ),
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
        scale: manato_core::Vec2::new(1.0, 1.0),
        angle: 0.0,
        x_shear: 0.0,
        projection: manato_sim::ProjectionMode::Orthographic,
        focal_length: 1024.0,
        time: 0,
        remove_time: 5,
        anim_loop_time: 4,
        remove_on_hits_exhausted: true,
        priority: 0,
        priority_points: 0,
        shadow: [0; 3],
        reflection: 0,
        miss_time: 0,
        hit_anim: -1,
        rem_anim: -1,
        cancel_anim: -1,
        layer_no: 0,
        sprite_priority: 3,
        edge_bound: 40.0,
        stage_bound: 40.0,
        height_bound: (-240.0, 1.0),
        depth_bound: f32::MAX,
        out_of_bounds: false,
        status: manato_sim::projectile::ProjectileStatus::Active,
        removal_ticks: 0,
        rem_velocity: manato_core::Vec2::ZERO,
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

    // Re-binding restarts the duration. Face the target again because stage
    // autoturn flipped the idle attacker while unbound.
    sim.fighters[0].facing = 1;
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
            ("projanim".into(), "10".into()),
            ("accel".into(), "1, -2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    let projectile = &sim.projectiles[0];
    assert_eq!(projectile.vel.x, 0.0);
    assert_eq!(projectile.remove_time, 3);
    assert_eq!(projectile.hits, 2);
    assert_eq!(projectile.hitdef.damage, 40);
    assert_eq!(projectile.anim, 10);
    assert_eq!(projectile.accel, manato_core::Vec2::new(1.0, -2.0));

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
            ("accel".into(), "0.5, 0.25".into()),
            ("trans".into(), "addalpha".into()),
            ("alpha".into(), "128, 64".into()),
            ("angle".into(), "15".into()),
            ("xangle".into(), "5".into()),
            ("yangle".into(), "6".into()),
            ("xshear".into(), "0.125".into()),
            ("projection".into(), "perspective".into()),
            ("focallength".into(), "512".into()),
            ("removetime".into(), "9".into()),
            ("sprpriority".into(), "7".into()),
            ("ontop".into(), "0".into()),
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
    assert!((effect.accel.x - 0.5).abs() < 0.001);
    assert!((effect.accel.y - 0.25).abs() < 0.001);
    assert_eq!(effect.blend, manato_sim::EffectBlend::Add);
    assert!(effect.transparent);
    assert!((effect.alpha - 0.25).abs() < 0.001, "alpha[1]/256");
    assert_eq!(effect.angle, 15.0);
    assert_eq!(effect.x_angle, 5.0);
    assert_eq!(effect.y_angle, 6.0);
    assert_eq!(effect.x_shear, 0.125);
    assert_eq!(effect.projection, manato_sim::ProjectionMode::Perspective);
    assert_eq!(effect.focal_length, 512.0);
    assert_eq!(effect.duration, 9);
    assert!(!effect.remove_on_anim_end);
    assert_eq!(effect.sprite_priority, 7);
    assert!(effect.behind, "ontop = 0 renders behind");

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
         \n[State 900, Paused]\ntype = VarAdd\ntrigger1 = 1\nv = 2\nvalue = 100\npersistent = 1\nignorehitpause = 1\n\
         \n[Statedef 901]\ntype = S\nphysics = S\nanim = 0\nctrl = 0\n\
         \n[State 901, Periodic]\ntype = VarAdd\ntrigger1 = 1\nv = 3\nvalue = 1\npersistent = 3\n",
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

    // persistent = 3 runs on the first matching tick, then every third tick.
    sim.fighters[0].enter_state(901, &assets.characters[0], false);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].vars[3], 1);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].vars[3], 1);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].vars[3], 1);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].vars[3], 2, "second run after three ticks");
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.fighters[0].vars[3], 3);

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
fn hold_pseudo_commands_follow_facing_and_diagonals() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let holds = |sim: &SimState| {
        (
            manato_sim::trigger::int(sim, 0, "Command = \"holdfwd\""),
            manato_sim::trigger::int(sim, 0, "Command = \"holdback\""),
            manato_sim::trigger::int(sim, 0, "Command = \"holdup\""),
            manato_sim::trigger::int(sim, 0, "Command = \"holddown\""),
        )
    };
    let hold = |sim: &mut SimState, facing: i32, dir: u8| {
        sim.fighters[0].facing = facing;
        sim.fighters[0].append_input(PlayerInput::new(dir, 0), 0);
    };

    hold(&mut sim, 1, manato_core::DIR_RIGHT);
    assert_eq!(holds(&sim), (1, 0, 0, 0));
    hold(&mut sim, 1, manato_core::DIR_LEFT);
    assert_eq!(holds(&sim), (0, 1, 0, 0));
    hold(&mut sim, 1, manato_core::DIR_UP);
    assert_eq!(holds(&sim), (0, 0, 1, 0));
    hold(&mut sim, 1, manato_core::DIR_DOWN);
    assert_eq!(holds(&sim), (0, 0, 0, 1));

    // Diagonals satisfy both component holds.
    hold(&mut sim, 1, manato_core::DIR_UP | manato_core::DIR_RIGHT);
    assert_eq!(holds(&sim), (1, 0, 1, 0));
    hold(&mut sim, 1, manato_core::DIR_DOWN | manato_core::DIR_RIGHT);
    assert_eq!(holds(&sim), (1, 0, 0, 1));
    hold(&mut sim, 1, manato_core::DIR_DOWN | manato_core::DIR_LEFT);
    assert_eq!(holds(&sim), (0, 1, 0, 1));

    // Facing -1 swaps forward/back but not up/down.
    hold(&mut sim, -1, manato_core::DIR_LEFT);
    assert_eq!(holds(&sim), (1, 0, 0, 0));
    hold(&mut sim, -1, manato_core::DIR_RIGHT);
    assert_eq!(holds(&sim), (0, 1, 0, 0));
    hold(&mut sim, -1, manato_core::DIR_UP | manato_core::DIR_LEFT);
    assert_eq!(holds(&sim), (1, 0, 1, 0));

    // Neutral releases every hold, and the negated command form follows.
    hold(&mut sim, 1, 0);
    assert_eq!(holds(&sim), (0, 0, 0, 0));
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "Command != \"holdfwd\""),
        1
    );
    hold(&mut sim, 1, manato_core::DIR_RIGHT);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "Command != \"holdfwd\""),
        0
    );
}

#[test]
fn team_mode_and_home_team_follow_side() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, assets: &manato_sim::Assets, owner: usize| {
        let control = manato_formats::Controller {
            kind: "helper".into(),
            params: vec![
                ("stateno".into(), "0".into()),
                ("removetime".into(), "-1".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, owner, &control, assets);
        sim.fighters.len() - 1
    };

    // The engine models single play for both sides: no simul/turns teams.
    for who in [0, 1] {
        assert_eq!(manato_sim::trigger::int(&sim, who, "TeamMode = single"), 1);
        assert_eq!(manato_sim::trigger::int(&sim, who, "TeamMode = Single"), 1);
        assert_eq!(manato_sim::trigger::int(&sim, who, "TeamMode = simul"), 0);
        assert_eq!(manato_sim::trigger::int(&sim, who, "TeamMode = turns"), 0);
        assert_eq!(manato_sim::trigger::int(&sim, who, "TeamMode != turns"), 1);
        assert_eq!(
            manato_sim::trigger::eval_str(&sim, who, "TeamMode").as_str(),
            "single"
        );
    }

    // Versus: P1's side is the home team; P2's is not. Helpers inherit the
    // side of their owner.
    assert_eq!(manato_sim::trigger::int(&sim, 0, "IsHomeTeam"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "IsHomeTeam"), 0);
    let p1_helper = spawn(&mut sim, &assets, 0);
    let p2_helper = spawn(&mut sim, &assets, 1);
    assert_eq!(sim.fighters[p1_helper].side, 0);
    assert_eq!(sim.fighters[p2_helper].side, 1);
    assert_eq!(manato_sim::trigger::int(&sim, p1_helper, "IsHomeTeam"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, p2_helper, "IsHomeTeam"), 0);
}

#[test]
fn ai_level_reports_per_slot_and_helpers_inherit() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let base_assets = sim.assets.clone();

    // Only P2 is AI-driven at level 6; P1 is human and reports 0.
    let mut options = MatchOptions::default();
    options.ai_level = 6;
    options.ai_sides = [false, true];
    sim = SimState::new(base_assets.clone(), options);

    assert_eq!(manato_sim::trigger::int(&sim, 0, "AILevel"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "AILevel"), 6);

    let controller = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &controller, &base_assets);
    let human_helper = sim.fighters.len() - 1;
    manato_sim::controller::execute(&mut sim, 1, &controller, &base_assets);
    let cpu_helper = sim.fighters.len() - 1;
    assert_eq!(manato_sim::trigger::int(&sim, human_helper, "AILevel"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, cpu_helper, "AILevel"), 6);

    // Disabling AI at a nonzero level still reports 0 for every slot.
    let mut options = MatchOptions::default();
    options.ai_level = 8;
    options.ai_sides = [false, false];
    let sim = SimState::new(base_assets, options);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AILevel"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "AILevel"), 0);
}

#[test]
fn parent_exist_tracks_helper_ancestry() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, assets: &manato_sim::Assets, owner: usize| {
        let control = manato_formats::Controller {
            kind: "helper".into(),
            params: vec![
                ("stateno".into(), "0".into()),
                ("removetime".into(), "-1".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, owner, &control, assets);
        sim.fighters.len() - 1
    };

    // Root players have no parent.
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ParentExist"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "ParentExist"), 0);

    let helper = spawn(&mut sim, &assets, 0);
    let nested = spawn(&mut sim, &assets, helper);
    assert_eq!(manato_sim::trigger::int(&sim, helper, "ParentExist"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, nested, "ParentExist"), 1);

    // Redirects follow the parent/root chain.
    assert_eq!(
        manato_sim::trigger::int(&sim, helper, "Parent, ParentExist"),
        0
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, nested, "Parent, ParentExist"),
        1
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, nested, "Root, ParentExist"),
        0
    );
    // The nested helper still resolves the opposing player, not its owner.
    assert_eq!(
        manato_sim::trigger::int(&sim, nested, "Enemy, ParentExist"),
        0
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, nested, "Enemy, StateNo"),
        sim.fighters[1].state_no
    );
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
    // `time = 1` normalizes to 0, which disables emission but keeps the
    // state object so it can be revived.
    assert_eq!(
        sim.fighters[0]
            .after_image
            .as_ref()
            .expect("still present")
            .time,
        0
    );
    let ghosts = sim.fighters[0].after_images.len();
    sim.tick([PlayerInput::default(); 2]);
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        sim.fighters[0].after_images.len() <= ghosts,
        "disabled emitter adds no ghosts"
    );

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
    assert_eq!(sim.fighters[0].pal_mul, [0.5, 0.5, 0.5]);
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
    // Trigonometry uses radians (`sin(pi/2)` in the M.U.G.E.N reference).
    assert!((num("Sin(0.5235988)") - 0.5).abs() < 0.001);
    assert!((num("Cos(1.0471976)") - 0.5).abs() < 0.001);
    assert!((num("Tan(0.7853982)") - 1.0).abs() < 0.001);
    assert!((num("Atan(1)") - std::f32::consts::FRAC_PI_4).abs() < 0.001);
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
    // Keep the opponent on the side the fighter faces so stage autoturn does
    // not override the synthetic facing used here.
    sim.fighters[1].pos = manato_core::Vec2::new(-140.0, 0.0);
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
    // The emitter stops (timer 0) but the state object persists, matching
    // Ikemen's reusable `aimg`.
    assert_eq!(sim.fighters[0].after_image.as_ref().expect("state").time, 0);
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
    assert_eq!(sim.fighters[1].hit.vars.fall_x_vel, Some(-3.0));
    assert_eq!(sim.fighters[1].hit.vars.fall_y_vel, Some(-4.0));

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
    manato_sim::collision::resolve_projectiles(&mut sim);
    eprintln!(
        "DBG direct life {} status {:?} hits {}",
        sim.fighters[1].life,
        sim.projectiles.first().map(|p| p.status),
        sim.projectiles.first().map(|p| p.hits).unwrap_or(-1)
    );
    sim.tick([PlayerInput::default(); 2]);
    eprintln!(
        "DBG tick life {} status {:?}",
        sim.fighters[1].life,
        sim.projectiles.first().map(|p| p.status)
    );
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
    // The oracle double-hits for 120. Here the upper blast's bottom edge lands
    // at y=-94 while KFM's tallest idle hurt box starts at y=-93, so strict
    // integer box intersection misses by 1px and only the low blast connects.
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

#[test]
fn modify_projectile_targets_newest_or_all_matching_ids() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(3000, &assets.characters[0], false);
    for _ in 0..120 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.projectiles.len() >= 2 {
            break;
        }
    }
    assert_eq!(sim.projectiles.len(), 2);
    assert!(sim.projectiles.iter().all(|projectile| projectile.id == 0));
    let newest = sim.projectiles.last().unwrap().hitdef.damage;

    // Without an id only the newest projectile changes.
    let newest_only = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![("velocity".into(), "0, 0".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &newest_only, &assets);
    assert_eq!(sim.projectiles[0].vel.x, 6.0);
    assert_eq!(sim.projectiles[1].vel.x, 0.0);
    assert_eq!(sim.projectiles[1].hitdef.damage, newest);

    // An explicit matching id updates every projectile in the group.
    let all = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("id".into(), "0".into()),
            ("velocity".into(), "1, 0".into()),
            ("damage".into(), "20".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &all, &assets);
    for projectile in &sim.projectiles {
        assert_eq!(projectile.vel.x, 1.0);
        assert_eq!(projectile.hitdef.damage, 20);
    }
}

#[test]
fn nival_super_pauses_then_hits_for_90() {
    let Some(mut sim) = stock_kfm_match("content/chars/nival/nival.def") else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -15.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 15.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(3000, &assets.characters[0], false);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.super_pause > 0, "state 3000 opens with SuperPause");

    let life = sim.fighters[1].life;
    let mut contact_tick = None;
    for tick in 0..140 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].life < life {
            contact_tick = Some(tick + 2);
            break;
        }
    }
    let contact_tick = contact_tick.expect("projectile connects");
    // The 30-tick SuperPause delays contact vs the ~6-tick plain projectile.
    assert!(
        contact_tick >= 30,
        "contact waited for the pause, got {contact_tick}"
    );
    assert_eq!(sim.fighters[1].life, life - 90);
    assert!(sim.fighters[1].received_damage >= 90);
    assert!(sim.fighters[0].proj_hit_time >= 0);
}

#[test]
fn nival_super_guard_takes_20() {
    let Some(mut sim) = stock_kfm_match("content/chars/nival/nival.def") else {
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
    for _ in 0..160 {
        sim.tick([PlayerInput::default(), back]);
        if sim.fighters[0].proj_guarded_time >= 0 {
            guarded = true;
            break;
        }
    }
    assert!(guarded, "super projectile should be guardable");
    assert_eq!(sim.fighters[1].life, sim.fighters[1].life_max - 20);
    assert!(sim.fighters[0].proj_contact_time >= 0);
    assert_eq!(sim.fighters[0].proj_hit_time, -1);
}

#[test]
fn projectile_guard_uses_crouch_and_air_guard_states() {
    // Crouch guard: back + down.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -15.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 15.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(1000, &assets.characters[0], false);
    let crouch_back = PlayerInput::new(manato_core::DIR_RIGHT | manato_core::DIR_DOWN, 0);
    let mut guarded = false;
    for _ in 0..180 {
        sim.tick([PlayerInput::default(), crouch_back]);
        if sim.fighters[0].proj_guarded_time >= 0 {
            guarded = true;
            break;
        }
    }
    assert!(guarded, "crouch guard should block the projectile");
    assert_eq!(sim.fighters[1].life, sim.fighters[1].life_max - 12);
    assert!(
        (150..=155).contains(&sim.fighters[1].state_no),
        "crouch guard state, got {}",
        sim.fighters[1].state_no
    );

    // Air guard: back while airborne and in guard range.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -15.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 15.0;
    sim.fighters[1].facing = -1;
    sim.fighters[1].enter_state(50, &assets.characters[1], false);
    sim.fighters[1].pos.y = -24.0;
    sim.fighters[1].vel = manato_core::Vec2::ZERO;
    sim.fighters[1].gravity = Some(0.0);
    sim.fighters[0].enter_state(1000, &assets.characters[0], false);
    let back = PlayerInput::new(manato_core::DIR_RIGHT, 0);
    let mut guarded = false;
    for _ in 0..180 {
        sim.tick([PlayerInput::default(), back]);
        if sim.fighters[0].proj_guarded_time >= 0 {
            guarded = true;
            break;
        }
    }
    assert!(guarded, "air guard should block the projectile");
    assert_eq!(sim.fighters[1].life, sim.fighters[1].life_max - 12);
    assert_eq!(sim.fighters[1].state_no, 154);
}

#[test]
fn p2stateno_overrides_get_hit_state_without_p1stateno() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.p2stateno = 820;
    hit.damage = 30;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 820);
    assert_eq!(sim.fighters[1].life, sim.fighters[1].life_max - 30);
    assert!(!sim.fighters[1].ctrl);

    // Guarded hits never use p2stateno.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 150);
    assert!(sim.fighters[1].hit.guarding);

    // With p1stateno the throw path still wins and does not apply damage.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut throw = hit;
    throw.p1stateno = 800;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &throw, false, origin, 1, true);
    assert_eq!(sim.fighters[0].state_no, 800);
    assert_eq!(sim.fighters[1].state_no, 820);
    assert_eq!(sim.fighters[1].life, sim.fighters[1].life_max);
    assert_eq!(sim.fighters[0].target, Some(1));
}

#[test]
fn hit_override_matches_attributes_and_expires() {
    // Controller parsing: attr is stored uppercase with its target state/time.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitoverride".into(),
        params: vec![
            ("attr".into(), "s, na".into()),
            ("stateno".into(), "820".into()),
            ("time".into(), "3".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &control, &assets);
    let override_ = sim.fighters[1].hit_overrides[0]
        .clone()
        .expect("override set");
    assert_eq!(override_.attr, "S, NA");
    assert_eq!(override_.state, 820);
    assert_eq!(override_.time, 3);

    // Non-matching class falls through to a normal hit.
    let mut hit = hitdef();
    hit.attr.state_types = "S".into();
    hit.attr.class = "SA".into();
    let origin = sim.fighters[0].pos;
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - hit.damage);
    assert!(sim.fighters[1].state_no >= 5000);

    // Matching override redirects without damage.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let origin = sim.fighters[0].pos;
    let mut match_hit = hitdef();
    match_hit.attr.state_types = "S".into();
    match_hit.attr.class = "NA".into();
    sim.fighters[1].hit_overrides[0] = Some(manato_sim::fighter::HitOverride {
        attr: "SCA, NA".into(),
        state: 820,
        time: 50,
        keepstate: false,
        forceguard: false,
    });
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &match_hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life, "override prevents damage");
    assert_eq!(sim.fighters[1].state_no, 820);

    // The override window expires after its `time` ticks.
    sim.fighters[1].hit_overrides[0] = Some(manato_sim::fighter::HitOverride {
        attr: "SCA, NA".into(),
        state: 820,
        time: 1,
        keepstate: false,
        forceguard: false,
    });
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        sim.fighters[1]
            .hit_overrides
            .iter()
            .all(|slot| slot.is_none()),
        "override expires after its window"
    );
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &match_hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - match_hit.damage);

    // Projectiles honor HitOverride too (attr S, SP for Kaen's fireball).
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[1].hit_overrides[0] = Some(manato_sim::fighter::HitOverride {
        attr: "SCA, SP".into(),
        state: 820,
        time: 200,
        keepstate: false,
        forceguard: false,
    });
    sim.fighters[0].enter_state(1000, &assets.characters[0], false);
    let life = sim.fighters[1].life;
    for _ in 0..180 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].state_no == 820 {
            break;
        }
    }
    assert_eq!(
        sim.fighters[1].life, life,
        "projectile override prevents damage"
    );
    assert_eq!(sim.fighters[1].state_no, 820);

    // keepstate negates the hit without changing state; time=-1 is infinite.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let keep = manato_formats::Controller {
        kind: "hitoverride".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("stateno".into(), "820".into()),
            ("time".into(), "-1".into()),
            ("keepstate".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &keep, &assets);
    let override_ = sim.fighters[1].hit_overrides[0].clone().expect("override");
    assert_eq!(override_.time, -1);
    assert!(override_.keepstate);
    let life = sim.fighters[1].life;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &match_hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life);
    assert_eq!(
        sim.fighters[1].state_no, 0,
        "keepstate keeps the current state"
    );
    for _ in 0..120 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1]
            .hit_overrides
            .iter()
            .all(|slot| slot.is_none())
        {
            break;
        }
    }
    assert!(
        sim.fighters[1]
            .hit_overrides
            .iter()
            .any(|slot| slot.is_some()),
        "time=-1 never expires"
    );

    // M.U.G.E.N normalizes time 0 and values below -1 to 1.
    for (raw, expected) in [("0", 1), ("-2", 1)] {
        let control = manato_formats::Controller {
            kind: "hitoverride".into(),
            params: vec![
                ("attr".into(), "S, NA".into()),
                ("stateno".into(), "820".into()),
                ("time".into(), raw.into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(&mut sim, 1, &control, &assets);
        assert_eq!(
            sim.fighters[1].hit_overrides[0]
                .as_ref()
                .expect("override")
                .time,
            expected
        );
    }
}

#[test]
fn ordan_slam_sends_defender_to_820_for_190() {
    let Some(mut sim) = fight_phase_with(
        "content/chars/ordan/ordan.def",
        "content/chars/kaen/kaen.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -5.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 5.0;
    sim.fighters[1].facing = -1;
    // Keep the close range: pushbox separation would otherwise open a 32px gap.
    sim.fighters[0].player_push = false;
    sim.fighters[1].player_push = false;
    sim.fighters[0].enter_state(3000, &assets.characters[0], false);
    eprintln!(
        "PRE p1={:?} p2={:?}",
        sim.fighters[0].pos, sim.fighters[1].pos
    );
    sim.tick([PlayerInput::default(); 2]);
    eprintln!(
        "POST p1={:?} p2={:?}",
        sim.fighters[0].pos, sim.fighters[1].pos
    );
    assert!(sim.super_pause > 0, "slam opens with SuperPause");

    let life = sim.fighters[1].life;
    let mut contacted = false;
    for _ in 0..160 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[1].state_no == 820 || sim.fighters[1].life < life {
            contacted = true;
            break;
        }
    }
    assert!(contacted, "slam should connect");
    assert_eq!(sim.fighters[1].state_no, 820, "p2stateno without p1stateno");
    assert_eq!(sim.fighters[1].life, life - 190);
    assert!(!sim.fighters[1].ctrl);

    // `guardflag` is empty, so holding back must not block the slam.
    let Some(mut sim) = fight_phase_with(
        "content/chars/ordan/ordan.def",
        "content/chars/kaen/kaen.def",
    ) else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -5.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 5.0;
    sim.fighters[1].facing = -1;
    // Keep the close range: pushbox separation would otherwise open a 32px gap.
    sim.fighters[0].player_push = false;
    sim.fighters[1].player_push = false;
    sim.fighters[0].enter_state(3000, &assets.characters[0], false);
    let back = PlayerInput::new(manato_core::DIR_LEFT, 0);
    let life = sim.fighters[1].life;
    for _ in 0..160 {
        sim.tick([PlayerInput::default(), back]);
        if sim.fighters[1].state_no == 820 || sim.fighters[1].life < life {
            break;
        }
    }
    assert_eq!(sim.fighters[1].state_no, 820);
    assert_eq!(sim.fighters[1].life, life - 190);
    assert!(!sim.fighters[1].hit.guarding);
}

#[test]
fn defence_and_attack_multipliers_scale_damage() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let defence = manato_formats::Controller {
        kind: "defencemulset".into(),
        params: vec![("value".into(), "0.5".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &defence, &assets);
    assert_eq!(sim.fighters[1].defence_mul, 0.5);

    let hit = hitdef();
    let origin = sim.fighters[0].pos;
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - 10, "defence halves damage");

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let attack = manato_formats::Controller {
        kind: "attackmulset".into(),
        params: vec![("value".into(), "2".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &attack, &assets);
    assert_eq!(sim.fighters[0].attack_mul, 2.0);
    let origin = sim.fighters[0].pos;
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - 40, "attack doubles damage");

    // Both multipliers compose (2 * 0.5 = 1).
    let defence = manato_formats::Controller {
        kind: "defencemulset".into(),
        params: vec![("value".into(), "0.5".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &defence, &assets);
    let origin = sim.fighters[0].pos;
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - 20);
}

#[test]
fn fall_env_shake_alias_sets_shake() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    // FallEnvShake consumes the values stored by the last unguarded hit.
    sim.fighters[0].hit.vars.fall_envshake_time = 12;
    sim.fighters[0].hit.vars.fall_envshake_freq = 120.0;
    sim.fighters[0].hit.vars.fall_envshake_ampl = -6;
    sim.fighters[0].hit.vars.fall_envshake_phase = 0.5;
    let shake = manato_formats::Controller {
        kind: "fallenvshake".into(),
        params: Vec::new(),
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &shake, &assets);
    assert_eq!(sim.env_shake.time, 12);
    assert!((sim.env_shake.freq - 120.0).abs() < 0.001);
    assert!((sim.env_shake.ampl - -6.0).abs() < 0.001);
    assert!((sim.env_shake.phase - 0.5).abs() < 0.001);
    assert_eq!(sim.fighters[0].hit.vars.fall_envshake_time, 0);

    // With no stored time the controller is a no-op.
    let before = sim.env_shake;
    manato_sim::controller::execute(&mut sim, 0, &shake, &assets);
    assert_eq!(sim.env_shake, before);
}

#[test]
fn hit_override_slots_are_independent() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let slot2 = manato_formats::Controller {
        kind: "hitoverride".into(),
        params: vec![
            ("attr".into(), "S, SA".into()),
            ("stateno".into(), "820".into()),
            ("slot".into(), "2".into()),
            ("time".into(), "5".into()),
            ("keepstate".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &slot2, &assets);
    assert!(sim.fighters[1].hit_overrides[0].is_none());
    let stored = sim.fighters[1].hit_overrides[2].clone().expect("slot 2");
    assert_eq!(stored.attr, "S, SA");
    assert_eq!(stored.time, 5);
    assert!(stored.keepstate);

    // Slot 0 and slot 2 match different attack classes independently.
    sim.fighters[1].hit_overrides[0] = Some(manato_sim::fighter::HitOverride {
        attr: "SCA, NA".into(),
        state: 820,
        time: 1,
        keepstate: false,
        forceguard: false,
    });
    let mut na = hitdef();
    na.attr.state_types = "S".into();
    na.attr.class = "NA".into();
    let mut special = na.clone();
    special.attr.class = "SA".into();

    let origin = sim.fighters[0].pos;
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &special, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life, "slot 2 negates the special");
    assert_eq!(sim.fighters[1].state_no, 0, "keepstate keeps state");

    sim.tick([PlayerInput::default(); 2]);
    assert!(
        sim.fighters[1].hit_overrides[0].is_none(),
        "slot 0 expired after one tick"
    );
    assert!(
        sim.fighters[1].hit_overrides[2].is_some(),
        "slot 2 outlives slot 0"
    );

    // Slot 0 is gone, so a normal attack now connects.
    let origin = sim.fighters[0].pos;
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &na, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - na.damage);
}

#[test]
fn projectile_postype_controls_spawn_anchor() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos = manato_core::Vec2::new(-40.0, 0.0);
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos = manato_core::Vec2::new(60.0, -10.0);
    let spawn = |sim: &mut SimState, postype: Option<&str>, pos: &str| {
        let mut params = vec![
            ("projanim".to_string(), "1500".to_string()),
            ("velocity".to_string(), "0, 0".to_string()),
            ("pos".to_string(), pos.to_string()),
            ("removetime".to_string(), "30".to_string()),
        ];
        if let Some(postype) = postype {
            params.push(("postype".to_string(), postype.to_string()));
        }
        let control = manato_formats::Controller {
            kind: "projectile".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.projectiles.pop().expect("projectile spawned").pos
    };

    // Default is relative to the owner.
    let p1 = spawn(&mut sim, None, "0, 0");
    assert!((p1.x - -40.0).abs() < 0.001);
    assert!(p1.y.abs() < 0.001);

    // `p2` anchors to the opponent, with facing-relative x and plain y offset.
    let p2 = spawn(&mut sim, Some("p2"), "5, -3");
    assert!((p2.x - 65.0).abs() < 0.001);
    assert!((p2.y - -13.0).abs() < 0.001);

    // `front`/`back` anchor to the screen edges (Ikemen helperPos semantics).
    sim.camera.x = 0.0;
    let front = spawn(&mut sim, Some("front"), "0, 0");
    assert!((front.x - 160.0).abs() < 0.001);
    let back = spawn(&mut sim, Some("back"), "0, 0");
    assert!((back.x - -160.0).abs() < 0.001);

    // Facing flips which edge is "front" vs "back".
    sim.fighters[0].facing = -1;
    let front_left = spawn(&mut sim, Some("front"), "0, 0");
    assert!((front_left.x - -160.0).abs() < 0.001);
    let back_right = spawn(&mut sim, Some("back"), "0, 0");
    assert!((back_right.x - 160.0).abs() < 0.001);
    sim.fighters[0].facing = 1;

    // Screen-edge anchors follow the camera.
    sim.camera.x = 50.0;
    let left = spawn(&mut sim, Some("left"), "0, 0");
    assert!((left.x - -110.0).abs() < 0.001);
    let right = spawn(&mut sim, Some("right"), "0, 0");
    assert!((right.x - 210.0).abs() < 0.001);
}

#[test]
fn hit_override_slot_clamp_and_forceguard() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    for (raw_slot, expected) in [("9", 7usize), ("-4", 0)] {
        let control = manato_formats::Controller {
            kind: "hitoverride".into(),
            params: vec![
                ("attr".into(), "S, NA".into()),
                ("stateno".into(), "820".into()),
                ("slot".into(), raw_slot.into()),
                ("time".into(), "2".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(&mut sim, 1, &control, &assets);
        assert!(
            sim.fighters[1].hit_overrides[expected].is_some(),
            "slot {raw_slot} should clamp to {expected}"
        );
    }

    // forceguard turns the overridden hit into a guard instead of a negation.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let force = manato_formats::Controller {
        kind: "hitoverride".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("stateno".into(), "820".into()),
            ("time".into(), "10".into()),
            ("forceguard".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &force, &assets);
    let override_ = sim.fighters[1].hit_overrides[0].clone().expect("override");
    assert!(override_.forceguard);

    let hit = hitdef();
    let origin = sim.fighters[0].pos;
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert!(sim.fighters[1].hit.guarding, "forceguard produces a guard");
    assert_eq!(sim.fighters[1].state_no, 150);
    assert_eq!(sim.fighters[1].life, life - hit.guard_damage);
    assert_ne!(sim.fighters[1].state_no, 820);

    // The melee resolve path also honors forceguard.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[1].hit_overrides[0] = Some(manato_sim::fighter::HitOverride {
        attr: "S, NA".into(),
        state: 820,
        time: 10,
        keepstate: false,
        forceguard: true,
    });
    sim.fighters[0].pos.x = -10.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 10.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    sim.fighters[0].hitdef = Some(hitdef());
    manato_sim::collision::resolve(&mut sim);
    assert!(sim.fighters[1].hit.guarding);
    assert_eq!(sim.fighters[1].state_no, 150);
}

#[test]
fn hitdefattr_inguarddist_and_canrecover_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let hitdef_controller = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "20".into()),
            ("hitflag".into(), "MAF".into()),
            ("guardflag".into(), "MA".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &hitdef_controller, &assets);
    assert!(sim.fighters[0].hitdef.is_some());
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitDefAttr = S, NA"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitDefAttr = S, SA"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitDefAttr = SCA, NA"), 1);

    // InGuardDist requires a nearby attacking opponent.
    sim.fighters[0].pos.x = 0.0;
    sim.fighters[1].pos.x = 50.0;
    sim.fighters[1].hitdef = Some(manato_sim::hitdef::HitDef::default());
    assert_eq!(manato_sim::trigger::int(&sim, 0, "InGuardDist"), 1);
    sim.fighters[1].hitdef = None;
    sim.fighters[1].move_type = manato_sim::fighter::MoveType::Attack;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "InGuardDist"), 1);
    sim.fighters[1].move_type = manato_sim::fighter::MoveType::Idle;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "InGuardDist"), 0);
    sim.fighters[1].hitdef = Some(manato_sim::hitdef::HitDef::default());
    sim.fighters[1].pos.x = 200.0;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "InGuardDist"), 0);

    // CanRecover needs a recoverable fall state.
    sim.fighters[1].state_no = 5030;
    sim.fighters[1].hit.vars.fall_recover = true;
    assert_eq!(manato_sim::trigger::int(&sim, 1, "CanRecover"), 1);
    sim.fighters[1].hit.vars.fall_recover = false;
    assert_eq!(manato_sim::trigger::int(&sim, 1, "CanRecover"), 0);
    sim.fighters[1].hit.vars.fall_recover = true;
    sim.fighters[1].state_no = 5000;
    assert_eq!(manato_sim::trigger::int(&sim, 1, "CanRecover"), 0);
}

#[test]
fn hitdef_guard_dist_defaults_and_limits_guarding() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    // Melee HitDefs default guard.dist from [Size] attack.dist.
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let hit = parse(&mut sim, vec![("attr".into(), "S, NA".into())]);
    assert_eq!(hit.guard_dist, 160);

    let hit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("guard.dist".into(), "0".into()),
        ],
    );
    assert_eq!(hit.guard_dist, 0);

    let hit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("guard.dist".into(), "50, 10".into()),
        ],
    );
    assert_eq!(hit.guard_dist, 50);

    // InGuardDist uses the opponent HitDef's guard.dist.
    sim.fighters[0].pos.x = 0.0;
    sim.fighters[1].pos.x = 60.0;
    let mut threat = hit;
    threat.guard_dist = 50;
    sim.fighters[1].hitdef = Some(threat);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "InGuardDist"), 0);
    if let Some(threat) = sim.fighters[1].hitdef.as_mut() {
        threat.guard_dist = 61;
    }
    assert_eq!(manato_sim::trigger::int(&sim, 0, "InGuardDist"), 1);

    // Projectile HitDefs default from [Size] proj.attack.dist and also
    // participate in InGuardDist.
    sim.fighters[1].hitdef = None;
    sim.fighters[1].move_type = manato_sim::fighter::MoveType::Idle;
    let projectile = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "0, 0".into()),
            ("removetime".into(), "60".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &projectile, &assets);
    let shot = sim.projectiles.last().expect("projectile");
    assert_eq!(shot.hitdef.guard_dist, 90);
    let shot_x = sim.projectiles.last_mut().expect("projectile");
    shot_x.pos.x = 50.0;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "InGuardDist"), 1);
    sim.projectiles.last_mut().expect("projectile").pos.x = 100.0;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "InGuardDist"), 0);

    // A zero guard distance prevents a held-back guard at contact range.
    let attempt = |sim: &mut SimState, guard_dist: i32| -> bool {
        sim.fighters[1].enter_state(0, &assets.characters[1], false);
        sim.fighters[1].append_input(PlayerInput::new(manato_core::DIR_RIGHT, 0), 0);
        sim.fighters[1].pos = manato_core::Vec2::new(10.0, 0.0);
        sim.fighters[1].facing = -1;
        sim.fighters[0].hitdef = None;
        sim.fighters[0].hit.hit_targets.clear();
        sim.fighters[0].hit.hit_counts.clear();
        sim.fighters[0].pos = manato_core::Vec2::new(-10.0, 0.0);
        sim.fighters[0].facing = 1;
        sim.fighters[0].enter_state(200, &assets.characters[0], false);
        sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
        let mut hit = hitdef();
        hit.guard_dist = guard_dist;
        sim.fighters[0].hitdef = Some(hit);
        manato_sim::collision::resolve(sim);
        sim.fighters[1].hit.guarding
    };
    assert!(!attempt(&mut sim, 0), "guard.dist = 0 rejects contact");
    assert!(!attempt(&mut sim, 20), "strict distance is not inclusive");
    assert!(attempt(&mut sim, 21), "distance beyond dx allows guard");
}

#[test]
fn round_reset_carries_power_and_removes_helpers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "5".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    assert_eq!(sim.fighters.len(), 3);
    sim.fighters[0].power = 500.0;
    sim.fighters[1].power = 700.0;

    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.wins, [1, 0]);
    // Winner gains 200 power for the round win.
    assert!((sim.fighters[0].power - 700.0).abs() < 0.001);

    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Intro {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.number, 2);
    assert_eq!(sim.fighters.len(), 2, "helpers are removed between rounds");
    assert!(
        (sim.fighters[0].power - 700.0).abs() < 0.001,
        "power carries"
    );
    assert!(
        (sim.fighters[1].power - 700.0).abs() < 0.001,
        "power carries"
    );
    assert_eq!(sim.fighters[1].life, sim.fighters[1].life_max);
    assert_eq!(sim.fighters[1].state_no, 0);
}

#[test]
fn animation_time_loop_and_invalid_elem_edges() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(0, &assets.characters[0], false);
    // Idle animation 0 is 48 ticks across 6 frames of 8 ticks each.
    sim.fighters[0].anim_time = 47;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimTime"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElem"), 6);

    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElem"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimTime"), 47);
    assert_eq!(sim.fighters[0].anim_looped, 1);

    // Out-of-range elements report the documented sentinel.
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "AnimElemTime(0)"),
        -1_000_000
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "AnimElemTime(99)"),
        -1_000_000
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "AnimElemTime(1)"), 1);
}

#[test]
fn round_timer_freezes_outside_fight() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.round.timer = 500;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.timer, 499, "Fight decrements the timer");

    // KO freezes the timer while the KO phase runs.
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    let ko_timer = sim.round.timer;
    for _ in 0..10 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.timer, ko_timer, "KO does not tick the timer");

    // Intro also holds the reset timer until Fight starts.
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Intro {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Intro);
    assert_eq!(sim.round.timer, sim.round.round_time * 60);
    for _ in 0..10 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.timer, sim.round.round_time * 60);

    for _ in 0..60 {
        if sim.round.phase == manato_sim::RoundPhase::Fight {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Fight);
    let fight_timer = sim.round.timer;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.timer, fight_timer - 1);
}

#[test]
fn hit_vel_set_sprpriority_offset_and_power_controllers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    // HitVelSet copies the stored get-hit velocity on the requested axes.
    sim.fighters[1].hit.vars.x_vel = 3.0;
    sim.fighters[1].hit.vars.y_vel = -4.0;
    sim.fighters[1].vel = manato_core::Vec2::new(9.0, 9.0);
    let hit_vel = manato_formats::Controller {
        kind: "hitvelset".into(),
        params: vec![("x".into(), "1".into()), ("y".into(), "1".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &hit_vel, &assets);
    assert_eq!(sim.fighters[1].vel, manato_core::Vec2::new(3.0, -4.0));

    // Only the selected axis changes.
    sim.fighters[1].vel = manato_core::Vec2::new(9.0, 9.0);
    let x_only = manato_formats::Controller {
        kind: "hitvelset".into(),
        params: vec![("x".into(), "1".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &x_only, &assets);
    assert_eq!(sim.fighters[1].vel, manato_core::Vec2::new(3.0, 9.0));

    // Presentation controllers.
    let priority = manato_formats::Controller {
        kind: "sprpriority".into(),
        params: vec![("value".into(), "4".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &priority, &assets);
    assert_eq!(sim.fighters[0].sprite_priority, 4);

    let offset = manato_formats::Controller {
        kind: "offset".into(),
        params: vec![("x".into(), "5".into()), ("y".into(), "-6".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &offset, &assets);
    assert_eq!(
        sim.fighters[0].draw_offset,
        manato_core::Vec2::new(5.0, -6.0)
    );

    // Power add/set respect the power cap.
    sim.fighters[0].power = 100.0;
    let add = manato_formats::Controller {
        kind: "poweradd".into(),
        params: vec![("value".into(), "250".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &add, &assets);
    assert_eq!(sim.fighters[0].power, 350.0);
    let set = manato_formats::Controller {
        kind: "powerset".into(),
        params: vec![("value".into(), "99999".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &set, &assets);
    assert_eq!(sim.fighters[0].power, manato_sim::MAX_POWER);

    // VarAdd accumulates on the selected int variable.
    sim.fighters[0].vars[2] = 1;
    let var_add = manato_formats::Controller {
        kind: "varadd".into(),
        params: vec![("v".into(), "2".into()), ("value".into(), "5".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &var_add, &assets);
    manato_sim::controller::execute(&mut sim, 0, &var_add, &assets);
    assert_eq!(sim.fighters[0].vars[2], 11);
}

#[test]
fn hit_counts_and_get_hit_var_fields() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.damage = 10;
    hit.fall_damage = 25;
    hit.ground_type = manato_sim::hitdef::GroundType::Low;
    hit.air_type = manato_sim::hitdef::GroundType::High;
    hit.anim_type = manato_sim::hitdef::AnimType::Medium;
    hit.numhits = 3;
    let origin = sim.fighters[0].pos;
    for _ in 0..3 {
        manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    }
    assert_eq!(sim.fighters[0].attack.hit_count, 3);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitCount"), 3);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "UniqHitCount"), 3);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(hitcount)"), 3);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(groundtype)"),
        2
    );
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(airtype)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(animtype)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(damage)"), 10);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(fall.damage)"),
        25
    );
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(guarded)"), 0);
    assert_eq!(sim.fighters[1].received_damage, 30);
}

#[test]
fn posfreeze_velmul_and_helper_variable_binds() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    // PosFreeze holds position for the requested ticks, then movement resumes.
    sim.fighters[0].pos = manato_core::Vec2::new(0.0, 0.0);
    sim.fighters[0].vel = manato_core::Vec2::new(5.0, 0.0);
    sim.fighters[0].physics = manato_sim::fighter::Physics::None;
    let freeze = manato_formats::Controller {
        kind: "posfreeze".into(),
        params: vec![("value".into(), "3".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &freeze, &assets);
    sim.tick([PlayerInput::default(); 2]);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].pos.x, 0.0, "position frozen for two ticks");
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].pos.x, 5.0, "movement resumes after freeze");

    // VelMul scales both velocity axes.
    sim.fighters[0].vel = manato_core::Vec2::new(8.0, -4.0);
    let mul = manato_formats::Controller {
        kind: "velmul".into(),
        params: vec![("x".into(), "0.5".into()), ("y".into(), "0.25".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &mul, &assets);
    assert_eq!(sim.fighters[0].vel, manato_core::Vec2::new(4.0, -1.0));

    // BindToTarget places a helper relative to the target with facing.
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "4".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    let helper_index = sim.fighters.len() - 1;
    sim.fighters[0].target = Some(1);
    sim.fighters[helper_index].target = Some(1);
    sim.fighters[0].pos = manato_core::Vec2::new(-40.0, 0.0);
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos = manato_core::Vec2::new(30.0, -5.0);
    sim.fighters[1].facing = -1;
    let bind = manato_formats::Controller {
        kind: "bindtotarget".into(),
        params: vec![("pos".into(), "10, -20".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, helper_index, &bind, &assets);
    assert_eq!(
        sim.fighters[helper_index].pos,
        manato_core::Vec2::new(20.0, -25.0)
    );

    // RootVarAdd from a helper updates the root's variable.
    let root_add = manato_formats::Controller {
        kind: "rootvaradd".into(),
        params: vec![("v".into(), "3".into()), ("value".into(), "7".into())],
        line: 0,
    };
    sim.fighters[0].vars[3] = 5;
    manato_sim::controller::execute(&mut sim, helper_index, &root_add, &assets);
    assert_eq!(sim.fighters[0].vars[3], 12);

    // ParentVarAdd does the same for the direct parent.
    let parent_add = manato_formats::Controller {
        kind: "parentvaradd".into(),
        params: vec![("v".into(), "4".into()), ("value".into(), "2".into())],
        line: 0,
    };
    sim.fighters[0].vars[4] = 1;
    manato_sim::controller::execute(&mut sim, helper_index, &parent_add, &assets);
    assert_eq!(sim.fighters[0].vars[4], 3);
}

#[test]
fn combo_counter_refreshes_on_new_hits() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let hit = hitdef();
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.combo[0].shown_hits, 1);
    let first_timer = sim.combo[0].timer;
    for _ in 0..5 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.combo[0].timer < first_timer);

    // Two more hits refresh the display timer and accumulate damage.
    for _ in 0..2 {
        manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    }
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.combo[0].shown_hits, 3);
    assert_eq!(sim.combo[0].damage, 60);
    assert_eq!(sim.combo[0].timer, manato_sim::COMBO_DISPLAY_TICKS - 1);

    // Recovering clears combo damage while the displayed count lingers.
    for _ in 0..180 {
        if sim.fighters[1].received_hits == 0 {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.fighters[1].received_hits, 0);
    assert_eq!(sim.combo[0].damage, 0);
    assert_eq!(sim.combo[0].shown_hits, 3);
}

#[test]
fn round_display_strings_match_flow() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let winner_name = sim.assets.characters[0].display_name().to_string();
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.display, format!("{winner_name} Wins"));

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let winner_name = sim.assets.characters[0].display_name().to_string();
    sim.fighters[0].life = 800;
    sim.fighters[1].life = 400;
    sim.round.timer = 1;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.display, format!("Time Over - {winner_name} Wins"));

    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].life = 500;
    sim.fighters[1].life = 500;
    sim.round.timer = 1;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.display, "Time Over - Draw");
}

#[test]
fn hit_fall_controller_interaction_lands_with_damage() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[1].state_no = 5030;
    sim.fighters[1].state_time = 1;
    sim.fighters[1].physics = manato_sim::fighter::Physics::None;
    sim.fighters[1].facing = -1;
    sim.fighters[1].pos.y = 24.0;
    sim.fighters[1].vel = manato_core::Vec2::ZERO;
    sim.fighters[1].hit.vars.y_accel = 0.0;

    let set = manato_formats::Controller {
        kind: "hitfallset".into(),
        params: vec![
            ("value".into(), "1".into()),
            ("xvel".into(), "2".into()),
            ("yvel".into(), "6".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &set, &assets);
    assert!(sim.fighters[1].hit.vars.fall);
    assert_eq!(sim.fighters[1].hit.vars.fall_x_vel, Some(-2.0));
    assert_eq!(sim.fighters[1].hit.vars.fall_y_vel, Some(6.0));

    let vel = manato_formats::Controller {
        kind: "hitfallvel".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &vel, &assets);
    assert_eq!(sim.fighters[1].vel, manato_core::Vec2::new(-2.0, 6.0));

    let damage = manato_formats::Controller {
        kind: "hitfalldamage".into(),
        params: vec![("value".into(), "33".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &damage, &assets);
    assert_eq!(sim.fighters[1].hit.vars.fall_damage, 33);

    let life = sim.fighters[1].life;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].pos.y, 0.0);
    assert_eq!(sim.fighters[1].vel.y, 0.0);
    assert_eq!(sim.fighters[1].life, life - 33);
    assert_eq!(sim.fighters[1].hit.vars.fall_damage, 0);

    // HitFallSet(0) clears the fall flag without touching velocity.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[1].hit.vars.fall = true;
    sim.fighters[1].hit.vars.fall_y_vel = Some(4.0);
    let clear = manato_formats::Controller {
        kind: "hitfallset".into(),
        params: vec![("value".into(), "0".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &clear, &assets);
    assert!(!sim.fighters[1].hit.vars.fall);
    assert_eq!(sim.fighters[1].hit.vars.fall_y_vel, Some(4.0));
}

#[test]
fn life_add_and_set_kill_semantics() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = |kind: &str, value: i32, kill: i32| manato_formats::Controller {
        kind: kind.into(),
        params: vec![
            ("value".into(), value.to_string()),
            ("kill".into(), kill.to_string()),
        ],
        line: 0,
    };

    // kill = 0 keeps at least 1 HP.
    sim.fighters[0].life = 20;
    manato_sim::controller::execute(&mut sim, 0, &control("lifeadd", -50, 0), &assets);
    assert_eq!(sim.fighters[0].life, 1);
    manato_sim::controller::execute(&mut sim, 0, &control("lifeset", 0, 0), &assets);
    assert_eq!(sim.fighters[0].life, 1);
    manato_sim::controller::execute(&mut sim, 0, &control("lifeset", -10, 0), &assets);
    assert_eq!(sim.fighters[0].life, 1);

    // kill = 1 (the default) can reach zero.
    manato_sim::controller::execute(&mut sim, 0, &control("lifeadd", -50, 1), &assets);
    assert_eq!(sim.fighters[0].life, 0);
    manato_sim::controller::execute(&mut sim, 0, &control("lifeset", 0, 1), &assets);
    assert_eq!(sim.fighters[0].life, 0);

    // Healing and setting clamp to the maximum.
    manato_sim::controller::execute(&mut sim, 0, &control("lifeadd", 99999, 0), &assets);
    assert_eq!(sim.fighters[0].life, sim.fighters[0].life_max);
    manato_sim::controller::execute(&mut sim, 0, &control("lifeset", 99999, 0), &assets);
    assert_eq!(sim.fighters[0].life, sim.fighters[0].life_max);
}

#[test]
fn target_vel_add_and_bind_variants() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    // TargetVelAdd uses the target's facing for the X component.
    sim.fighters[0].target = Some(1);
    sim.fighters[1].facing = -1;
    sim.fighters[1].vel = manato_core::Vec2::new(1.0, 1.0);
    let vel_add = manato_formats::Controller {
        kind: "targetveladd".into(),
        params: vec![("x".into(), "2".into()), ("y".into(), "-3".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &vel_add, &assets);
    assert_eq!(sim.fighters[1].vel, manato_core::Vec2::new(-1.0, -2.0));

    // BindToParent places the helper relative to the parent's facing.
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "6".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    let helper_index = sim.fighters.len() - 1;
    sim.fighters[0].pos = manato_core::Vec2::new(-20.0, 0.0);
    sim.fighters[0].facing = 1;
    let bind = manato_formats::Controller {
        kind: "bindtoparent".into(),
        params: vec![("pos".into(), "5, -10".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, helper_index, &bind, &assets);
    assert_eq!(
        sim.fighters[helper_index].pos,
        manato_core::Vec2::new(-15.0, -10.0)
    );

    sim.fighters[0].facing = -1;
    manato_sim::controller::execute(&mut sim, helper_index, &bind, &assets);
    assert_eq!(
        sim.fighters[helper_index].pos,
        manato_core::Vec2::new(-25.0, -10.0),
        "facing mirrors the bind offset"
    );

    // BindToRoot resolves through the helper's root (still fighter 0).
    sim.fighters[0].pos = manato_core::Vec2::new(40.0, 0.0);
    sim.fighters[0].facing = 1;
    let root_bind = manato_formats::Controller {
        kind: "bindtoroot".into(),
        params: vec![("pos".into(), "-8, -4".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, helper_index, &root_bind, &assets);
    assert_eq!(
        sim.fighters[helper_index].pos,
        manato_core::Vec2::new(32.0, -4.0)
    );
    assert_eq!(sim.root_of(helper_index), 0);
    assert_eq!(sim.parent_of(helper_index), 0);
}

#[test]
fn target_life_power_composition_and_filter_timers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].target = Some(1);

    // TargetLifeAdd accumulates and honors kill = 0.
    let life_add = |value: i32, kill: i32| manato_formats::Controller {
        kind: "targetlifeadd".into(),
        params: vec![
            ("value".into(), value.to_string()),
            ("kill".into(), kill.to_string()),
        ],
        line: 0,
    };
    let life = sim.fighters[1].life;
    manato_sim::controller::execute(&mut sim, 0, &life_add(-50, 0), &assets);
    manato_sim::controller::execute(&mut sim, 0, &life_add(-50, 0), &assets);
    assert_eq!(sim.fighters[1].life, life - 100);
    sim.fighters[1].life = 20;
    manato_sim::controller::execute(&mut sim, 0, &life_add(-50, 0), &assets);
    assert_eq!(sim.fighters[1].life, 1);
    manato_sim::controller::execute(&mut sim, 0, &life_add(-50, 1), &assets);
    assert_eq!(sim.fighters[1].life, 0);

    // TargetPowerAdd accumulates and clamps.
    sim.fighters[1].power = 100.0;
    let power_add = manato_formats::Controller {
        kind: "targetpoweradd".into(),
        params: vec![("value".into(), "250".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &power_add, &assets);
    assert_eq!(sim.fighters[1].power, 350.0);
    let power_cap = manato_formats::Controller {
        kind: "targetpoweradd".into(),
        params: vec![("value".into(), "99999".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &power_cap, &assets);
    assert_eq!(sim.fighters[1].power, manato_sim::MAX_POWER);

    // NotHitBy blocks matching attacks until its timer expires.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let hit = hitdef();
    let origin = sim.fighters[0].pos;
    sim.fighters[1].hit.not_hit_by = Some(("SCA, NA".into(), 2));
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life);
    for _ in 0..2 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.fighters[1].hit.not_hit_by.is_none());
    // Force a clean hit after the filter expires.
    sim.fighters[1].state_no = 0;
    sim.fighters[1].ctrl = true;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - hit.damage);

    // HitBy whitelist takes precedence over NotHitBy.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].hit.not_hit_by = Some(("SCA, NA".into(), 50));
    sim.fighters[1].hit.hit_by = Some(("SCA, SA".into(), 50));
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(
        sim.fighters[1].life, life,
        "whitelist rejects normal attack"
    );
}

#[test]
fn playerindex_helperindex_and_stateowner_redirects() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "8".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    let helper_index = sim.fighters.len() - 1;
    sim.fighters[1].life = 777;
    sim.fighters[helper_index].state_no = 200;
    sim.fighters[0].state_no = 50;

    // playerindex(n) selects a player and clamps out-of-range indices.
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "playerindex(1), life"),
        777
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "playerindex(9), life"),
        777
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "playerindex(0), life"),
        sim.fighters[0].life
    );

    // helperindex(n) selects the root's nth helper; invalid indices fall back to self.
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "helperindex(0), stateno"),
        200
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "helperindex(5), stateno"),
        sim.fighters[0].state_no
    );

    // stateowner resolves to the helper's root/owner.
    assert_eq!(
        manato_sim::trigger::int(&sim, helper_index, "stateowner, stateno"),
        sim.fighters[0].state_no
    );
    // Invalid helper redirects fall back to self rather than panicking.
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "helper(99), stateno"),
        sim.fighters[0].state_no
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "enemynear, life"),
        sim.fighters[1].life
    );
}

#[test]
fn numexplod_and_numprojid_filters() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -5.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 5.0;
    sim.fighters[1].facing = -1;
    sim.fighters[1].set_anim(0, &assets.characters[1], None);
    let spawn = |sim: &mut SimState, id: i32| {
        let control = manato_formats::Controller {
            kind: "explod".into(),
            params: vec![
                ("anim".into(), "10".into()),
                ("id".into(), id.to_string()),
                ("removetime".into(), "60".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    spawn(&mut sim, 3);
    spawn(&mut sim, 4);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumExplod"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumExplod(3)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumExplod(4)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumExplod(9)"), 0);

    let remove = manato_formats::Controller {
        kind: "removeexplod".into(),
        params: vec![("id".into(), "3".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &remove, &assets);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumExplod"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumExplod(3)"), 0);

    // Projectile ID filtering counts only the owner's matching projectiles.
    let projectile = |id: i32| manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "0, 0".into()),
            ("removetime".into(), "60".into()),
            ("id".into(), id.to_string()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &projectile(7), &assets);
    manato_sim::controller::execute(&mut sim, 0, &projectile(7), &assets);
    manato_sim::controller::execute(&mut sim, 0, &projectile(8), &assets);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProj"), 3);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProjID(7)"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProjID(8)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProjID(9)"), 0);
}

#[test]
fn hitbyattr_matching_semantics() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].hit.not_hit_by = Some(("SCA, NA".into(), 60));
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitByAttr = SCA, NA"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitByAttr = S"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitByAttr = S, SA"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitByAttr = SCA, SA"), 0);

    // HitBy-only filters also answer the trigger.
    sim.fighters[1].hit.not_hit_by = None;
    sim.fighters[1].hit.hit_by = Some(("SA, SA".into(), 60));
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitByAttr = S, SA"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitByAttr = S, NA"), 0);

    // No filter means no match.
    sim.fighters[1].hit.hit_by = None;
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitByAttr = S, NA"), 0);
}

#[test]
fn hit_vel_and_get_hit_var_ids_and_hit_over_transitions() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.activation = 42;
    hit.id = 42;
    hit.chainid = 7;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);

    let vars = sim.fighters[1].hit.vars.clone();
    eprintln!(
        "vars x={} y={} triggerX={} triggerY={}",
        vars.x_vel,
        vars.y_vel,
        manato_sim::trigger::num(&sim, 1, "HitVel X"),
        manato_sim::trigger::num(&sim, 1, "HitVel Y")
    );
    assert!((manato_sim::trigger::num(&sim, 1, "HitVel X") - vars.x_vel).abs() < 0.001);
    assert!((manato_sim::trigger::num(&sim, 1, "HitVel Y") - vars.y_vel).abs() < 0.001);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(hitid)"), 42);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(chainid)"), 7);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(isbound)"), 0);
    sim.fighters[1].hit.vars.chain_id = 9;
    sim.fighters[1].hit.vars.is_bound = true;
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(chainid)"), 9);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(isbound)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(unknown)"), 0);

    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitOver"), 0);
    let mut shake_seen = false;
    let mut hit_over_seen = false;
    for _ in 0..60 {
        sim.tick([PlayerInput::default(); 2]);
        if manato_sim::trigger::int(&sim, 1, "HitShakeOver") != 0 {
            shake_seen = true;
        }
        if manato_sim::trigger::int(&sim, 1, "HitOver") != 0 {
            hit_over_seen = true;
            break;
        }
    }
    assert!(shake_seen, "HitShakeOver should become true");
    assert!(hit_over_seen, "HitOver should become true after hitstun");
}

#[test]
fn hitdef_id_and_chainid_parse_and_apply() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let default = parse(&mut sim, vec![("attr".into(), "S, NA".into())]);
    assert_eq!(default.id, 0);
    assert_eq!(default.chainid, -1);

    let parsed = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("id".into(), "12".into()),
            ("chainid".into(), "7".into()),
            ("damage".into(), "10".into()),
        ],
    );
    assert_eq!(parsed.id, 12);
    assert_eq!(parsed.chainid, 7);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(hitid)"), 12);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(chainid)"), 7);
}

#[test]
fn p2_opponent_state_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].move_type = manato_sim::fighter::MoveType::Attack;
    sim.fighters[1].ctrl = false;
    sim.fighters[1].state_no = 50;
    sim.fighters[1].prev_state_no = 20;
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "P2StateType").as_str(),
        "A"
    );
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "P2MoveType").as_str(),
        "A"
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "P2Ctrl"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "P2StateNo"), 50);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "P2PrevStateNo"), 20);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "P2Facing"),
        sim.fighters[1].facing
    );

    // From P2's perspective the opponent is P1.
    sim.fighters[0].state_no = 200;
    assert_eq!(manato_sim::trigger::int(&sim, 1, "P2StateNo"), 200);
}

#[test]
fn explod_window_perspective_edges_and_modify() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let explod = manato_formats::Controller {
        kind: "explod".into(),
        params: vec![
            ("anim".into(), "10".into()),
            ("id".into(), "6".into()),
            ("window".into(), "0, 0, 0, 0".into()),
            ("space".into(), "screen".into()),
            ("projection".into(), "perspective".into()),
            ("focallength".into(), "0".into()),
            ("xangle".into(), "20".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &explod, &assets);
    let effect = sim.effects.last().expect("explod");
    assert_eq!(effect.window, None, "all-zero window is ignored");
    assert!(!effect.wants_camera(), "screen space bypasses the camera");
    assert_eq!(effect.focal_length, 1.0, "focal length clamps to 1");
    assert_eq!(effect.projection, manato_sim::ProjectionMode::Perspective);
    assert_eq!(effect.x_angle, 20.0);

    let modify = manato_formats::Controller {
        kind: "modifyexplod".into(),
        params: vec![
            ("id".into(), "6".into()),
            ("projection".into(), "orthographic".into()),
            ("focallength".into(), "512".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    let effect = sim.effects.last().expect("explod");
    assert_eq!(effect.projection, manato_sim::ProjectionMode::Orthographic);
    assert_eq!(effect.focal_length, 512.0);
}

#[test]
fn posset_absolute_and_posadd_facing_edges() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos = manato_core::Vec2::new(10.0, -5.0);
    sim.fighters[0].facing = -1;

    // PosSet is absolute regardless of facing.
    let set = manato_formats::Controller {
        kind: "posset".into(),
        params: vec![("x".into(), "100".into()), ("y".into(), "-20".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &set, &assets);
    assert_eq!(sim.fighters[0].pos, manato_core::Vec2::new(100.0, -20.0));

    // Omitting an axis leaves it unchanged.
    let set_x = manato_formats::Controller {
        kind: "posset".into(),
        params: vec![("x".into(), "40".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &set_x, &assets);
    assert_eq!(sim.fighters[0].pos, manato_core::Vec2::new(40.0, -20.0));

    // PosAdd applies facing to X only.
    let add = manato_formats::Controller {
        kind: "posadd".into(),
        params: vec![("x".into(), "10".into()), ("y".into(), "5".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &add, &assets);
    assert_eq!(sim.fighters[0].pos, manato_core::Vec2::new(30.0, -15.0));

    // Reconciled facing flips the X direction.
    sim.fighters[0].facing = 1;
    manato_sim::controller::execute(&mut sim, 0, &add, &assets);
    assert_eq!(sim.fighters[0].pos, manato_core::Vec2::new(40.0, -10.0));

    // A controller with no axes is a no-op.
    let empty = manato_formats::Controller {
        kind: "posadd".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &empty, &assets);
    assert_eq!(sim.fighters[0].pos, manato_core::Vec2::new(40.0, -10.0));
}

#[test]
fn identity_and_team_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PlayerID"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "PlayerID"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumEnemy"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumPartner"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PalNo"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "PalNo"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "TimeMod"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "P2, PlayerID"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Enemy, NumEnemy"), 1);

    // Team character names: P1/P2 are populated; P3/P4 are empty in 1v1.
    let assets = sim.assets.clone();
    let p1_name = assets.characters[0].display_name().to_string();
    let p2_name = assets.characters[1].display_name().to_string();
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "P1Name").as_str(),
        p1_name
    );
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "P2Name").as_str(),
        p2_name
    );
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "P3Name").as_str(),
        ""
    );
    assert_eq!(
        manato_sim::trigger::eval_str(&sim, 0, "P4Name").as_str(),
        ""
    );

    // Helpers report their owner's player ID.
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "9".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    let helper_index = sim.fighters.len() - 1;
    eprintln!(
        "DBG helper_index={} owner={:?} len={}",
        helper_index,
        sim.fighters[helper_index].owner,
        sim.fighters.len()
    );
    assert_eq!(manato_sim::trigger::int(&sim, helper_index, "PlayerID"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Helper(9), PlayerID"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Helper(9), NumEnemy"), 1);
}

#[test]
fn distance_and_edge_triggers_track_facing_and_position() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.camera.x = 0.0;
    sim.camera.y = 0.0;
    sim.fighters[0].pos = manato_core::Vec2::new(0.0, 0.0);
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos = manato_core::Vec2::new(50.0, -30.0);
    sim.fighters[1].facing = -1;

    assert_eq!(manato_sim::trigger::num(&sim, 0, "P2Dist X"), 50.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "P2Dist Y"), -30.0);
    sim.fighters[0].facing = -1;
    assert_eq!(manato_sim::trigger::num(&sim, 0, "P2Dist X"), -50.0);
    sim.fighters[0].facing = 1;

    let a = &sim.fighters[0];
    let expected_body_x = (50.0 - a.width_front - sim.fighters[1].width_back).max(0.0);
    let expected_body_y = (30.0 - a.height).max(0.0);
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "P2BodyDist X"),
        expected_body_x
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "P2BodyDist Y"),
        expected_body_y
    );

    // Screen distance is the difference after camera mapping.
    let screen_a = sim.screen_pos(sim.fighters[0].pos);
    let screen_b = sim.screen_pos(sim.fighters[1].pos);
    assert!(
        (manato_sim::trigger::num(&sim, 0, "P2ScreenDist X") - (screen_b.x - screen_a.x)).abs()
            < 0.001
    );
    assert!(
        (manato_sim::trigger::num(&sim, 0, "P2ScreenDist Y") - (screen_b.y - screen_a.y)).abs()
            < 0.001
    );

    // Stage edges depend on facing and body width.
    sim.fighters[0].pos.x = 0.0;
    sim.fighters[0].facing = 1;
    assert_eq!(manato_sim::trigger::num(&sim, 0, "FrontEdgeDist"), 160.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "BackEdgeDist"), 160.0);
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "FrontEdgeBodyDist"),
        160.0 - sim.fighters[0].width_front
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "BackEdgeBodyDist"),
        160.0 - sim.fighters[0].width_back
    );
    sim.fighters[0].facing = -1;
    assert_eq!(manato_sim::trigger::num(&sim, 0, "FrontEdgeDist"), 160.0);
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "FrontEdgeBodyDist"),
        160.0 - sim.fighters[0].width_back
    );
}

#[test]
fn varrange_rootvar_targetadd_and_constant_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();

    // VarRangeSet fills an inclusive range regardless of argument order.
    let range = manato_formats::Controller {
        kind: "varrangeset".into(),
        params: vec![
            ("value".into(), "9".into()),
            ("first".into(), "2".into()),
            ("last".into(), "4".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &range, &assets);
    assert_eq!(&sim.fighters[0].vars[2..=4], &[9, 9, 9]);
    let reversed = manato_formats::Controller {
        kind: "varrangeset".into(),
        params: vec![
            ("value".into(), "3".into()),
            ("first".into(), "5".into()),
            ("last".into(), "4".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &reversed, &assets);
    assert_eq!(sim.fighters[0].vars[4], 3);
    assert_eq!(sim.fighters[0].vars[5], 3);
    assert_eq!(sim.fighters[0].vars[1], 0);

    // RootVarSet from a helper writes the root's variable.
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "10".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    let helper_index = sim.fighters.len() - 1;
    let root_var = manato_formats::Controller {
        kind: "rootvarset".into(),
        params: vec![("v".into(), "3".into()), ("value".into(), "77".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, helper_index, &root_var, &assets);
    assert_eq!(sim.fighters[0].vars[3], 77);

    // TargetAdd links the current opponent.
    sim.fighters[0].target = None;
    let add = manato_formats::Controller {
        kind: "targetadd".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &add, &assets);
    assert_eq!(sim.fighters[0].target, Some(1));

    // Constant screen/match triggers.
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "ScreenWidth"),
        manato_core::GAME_WIDTH
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "ScreenHeight"),
        manato_core::GAME_HEIGHT
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "GameWidth"),
        manato_core::GAME_WIDTH
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "GameHeight"),
        manato_core::GAME_HEIGHT
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "TicksPerSecond"), 60);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MatchNo"), 1);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "GameTime"),
        sim.frame as i32
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "RoundTime"),
        sim.round.seconds()
    );
}

#[test]
fn projectile_sound_and_explod_animelem_flags() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, projsnd: Option<&str>| {
        let mut params = vec![
            ("projanim".to_string(), "1500".to_string()),
            ("velocity".to_string(), "0, 0".to_string()),
            ("removetime".to_string(), "30".to_string()),
        ];
        if let Some(projsnd) = projsnd {
            params.push(("projsnd".to_string(), projsnd.to_string()));
        }
        let control = manato_formats::Controller {
            kind: "projectile".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    spawn(&mut sim, Some("5, 2"));
    let sound = sim
        .sounds
        .iter()
        .find(|sound| sound.group == 5 && sound.index == 2)
        .expect("projsnd event");
    assert!(matches!(
        sound.source,
        manato_sim::SoundSource::Character(0)
    ));
    assert_eq!(sim.projectiles.len(), 1);

    spawn(&mut sim, Some("-1, 0"));
    assert_eq!(sim.sounds.len(), 1, "negative group queues no sound");
    spawn(&mut sim, None);
    assert_eq!(sim.sounds.len(), 1);

    // Explod animelem and animfreeze parameters are recorded.
    let explod = manato_formats::Controller {
        kind: "explod".into(),
        params: vec![
            ("anim".into(), "10".into()),
            ("id".into(), "11".into()),
            ("animelem".into(), "3".into()),
            ("animfreeze".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &explod, &assets);
    let effect = sim.effects.last().expect("explod");
    assert_eq!(effect.anim_elem, 3);
    assert!(effect.anim_freeze);

    let plain = manato_formats::Controller {
        kind: "explod".into(),
        params: vec![("anim".into(), "10".into()), ("id".into(), "12".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &plain, &assets);
    let effect = sim.effects.last().expect("explod");
    assert_eq!(effect.anim_elem, 1);
    assert!(!effect.anim_freeze);
}

#[test]
fn get_hit_var_times_and_target_rebind_after_drop() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.ground_hittime = 8;
    hit.ground_slidetime = 5;
    hit.fall_time = 30;
    hit.fall_recover = true;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(slidetime)"), 5);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(ctrltime)"), 8);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(fall.time)"),
        30
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "GetHitVar(fall.recover)"),
        1
    );

    // TargetBind survives until TargetDrop, then can be re-established.
    let assets = sim.assets.clone();
    let bind = manato_formats::Controller {
        kind: "targetbind".into(),
        params: vec![
            ("pos".into(), "12, -18".into()),
            ("time".into(), "5".into()),
        ],
        line: 0,
    };
    sim.fighters[0].target = Some(1);
    sim.fighters[0].pos = manato_core::Vec2::new(-30.0, 0.0);
    sim.fighters[0].facing = 1;
    manato_sim::controller::execute(&mut sim, 0, &bind, &assets);
    sim.tick([PlayerInput::default(); 2]);
    let attacker = sim.fighters[0].pos;
    assert!((sim.fighters[1].pos.x - (attacker.x + 12.0)).abs() < 0.01);
    assert!((sim.fighters[1].pos.y - (attacker.y - 18.0)).abs() < 0.01);

    let drop = manato_formats::Controller {
        kind: "targetdrop".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &drop, &assets);
    assert_eq!(sim.fighters[0].target, None);
    assert!(sim.fighters[0].target_bind.is_none());

    let add = manato_formats::Controller {
        kind: "targetadd".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &add, &assets);
    assert_eq!(sim.fighters[0].target, Some(1));
    sim.fighters[0].pos = manato_core::Vec2::new(40.0, 0.0);
    manato_sim::controller::execute(&mut sim, 0, &bind, &assets);
    sim.tick([PlayerInput::default(); 2]);
    let attacker = sim.fighters[0].pos;
    assert!((sim.fighters[1].pos.x - (attacker.x + 12.0)).abs() < 0.01);
    assert!((sim.fighters[1].pos.y - (attacker.y - 18.0)).abs() < 0.01);
}

#[test]
fn width_override_is_per_tick_and_screenbound_axes_independent() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let width = manato_formats::Controller {
        kind: "width".into(),
        params: vec![("edge".into(), "-25, 35".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &width, &assets);
    assert_eq!(sim.fighters[0].width_override, Some((-25.0, 35.0)));
    assert_eq!(sim.fighters[0].width_back, -25.0);
    assert_eq!(sim.fighters[0].width_front, 35.0);

    // The override does not survive a tick without a re-applying controller.
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.fighters[0].width_override, None,
        "Width resets each tick"
    );
    let char = &assets.characters[0];
    let expected_back = char.constants.get_or("size.ground.back", 15.0);
    let expected_front = char.constants.get_or("size.ground.front", 16.0);
    assert!((sim.fighters[0].width_back - expected_back).abs() < 0.001);
    assert!((sim.fighters[0].width_front - expected_front).abs() < 0.001);

    // ScreenBound axes are independent: disabling X leaves Y set.
    let bounds = manato_formats::Controller {
        kind: "screenbound".into(),
        params: vec![("value".into(), "0, 1".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &bounds, &assets);
    assert_eq!(sim.fighters[0].screen_bound, (false, true));

    // With X disabled, the stage bound no longer clamps horizontal position.
    let stage_left = sim.assets.stage.left_bound;
    sim.fighters[0].pos = manato_core::Vec2::new(stage_left - 500.0, 0.0);
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        sim.fighters[0].pos.x <= stage_left - 400.0,
        "X bound disabled, position {} should be outside the stage",
        sim.fighters[0].pos.x
    );

    // Re-enabling X clamps again.
    let bounds = manato_formats::Controller {
        kind: "screenbound".into(),
        params: vec![("value".into(), "1, 0".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &bounds, &assets);
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        sim.fighters[0].pos.x >= stage_left - 0.001,
        "re-enabled X bound clamps to the stage edge"
    );
}

#[test]
fn win_pose_enters_state_180_and_round_not_over_delays_ko() {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = project.canonicalize().unwrap_or(project);
    let source = project.join("content/chars/kaen");
    if !source.join("kaen.cns").exists() {
        eprintln!("skipping: bundled character missing");
        return;
    }
    let dir = std::env::temp_dir().join(format!("manato-winpose-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp character dir");
    for entry in std::fs::read_dir(&source).expect("read character dir") {
        let entry = entry.expect("dir entry");
        if entry.file_type().expect("file type").is_file() {
            std::fs::copy(entry.path(), dir.join(entry.file_name())).expect("copy character file");
        }
    }
    let states = dir.join("kaen.cns");
    let text = std::fs::read_to_string(&states).expect("read kaen.cns");
    let text = text.replacen(
        "[Statedef -1]",
        "[Statedef -1]\n[State -1, Delay Ko]\ntype = AssertSpecial\ntrigger1 = RoundState = 3\nflag = roundnotover",
        1,
    );
    let text = format!(
        "{text}\n[Statedef 180]\ntype = S\nphysics = S\nanim = 0\nctrl = 0\n\
         \n[State 180, Mark]\ntype = VarSet\ntrigger1 = Time = 0\nv = 7\nvalue = 42\n"
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
    let new_sim = || {
        let mut options = MatchOptions::default();
        options.ai_level = 0;
        SimState::new(
            std::sync::Arc::new(manato_sim::Assets::load(&config).expect("assets load")),
            options,
        )
    };
    let mut sim = new_sim();
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Fight);

    // Winner enters state 180 on the KO tick.
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    assert_eq!(sim.fighters[0].state_no, 180, "winner enters the win pose");
    assert_ne!(sim.fighters[1].state_no, 180, "loser stays in a hit state");
    for _ in 0..5 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.fighters[0].state_no, 180);
    assert_eq!(sim.fighters[0].vars[7], 42, "win pose controller ran");

    // RoundNotOver keeps the Ko phase from advancing even past its timer.
    let mut sim = new_sim();
    for _ in 0..50 {
        sim.tick([PlayerInput::default(); 2]);
    }
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    for _ in 0..200 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(
        sim.round.phase,
        manato_sim::RoundPhase::Ko,
        "RoundNotOver delays the transition"
    );
    assert!(!sim.finished);
    assert_eq!(sim.round.number, 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reversal_def_redirects_matching_hits_and_expires() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -10.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 10.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    sim.fighters[0].hitdef = Some(hitdef());

    let reversal = manato_formats::Controller {
        kind: "reversaldef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "15".into()),
            ("time".into(), "2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &reversal, &assets);
    assert!(sim.fighters[1].reversal.is_some());
    assert_eq!(sim.fighters[1].reversal_time, 2);

    let p1_life = sim.fighters[0].life;
    let p2_life = sim.fighters[1].life;
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, p2_life, "reversal negates the hit");
    assert_eq!(
        sim.fighters[0].life,
        p1_life - 15,
        "attacker is hit instead"
    );
    assert!(sim.fighters[0].state_no >= 5000);

    // The window counts down and then clears.
    sim.tick([PlayerInput::default(); 2]);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.fighters[1].reversal.is_none(), "reversal expires");

    // A non-matching attribute does not reverse.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -10.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 10.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    sim.fighters[0].hitdef = Some(hitdef());
    sim.fighters[1].reversal = Some(hitdef());
    sim.fighters[1].reversal_time = 5;
    let p2_life = sim.fighters[1].life;
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(
        sim.fighters[1].life,
        p2_life - 20,
        "non-matching reversal attr falls through to a normal hit"
    );
}

#[test]
fn hit_and_guard_sounds_and_heavy_hit_shake() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.hitsound = (5, 3);
    hit.guardsound = (6, 4);
    hit.damage = 20;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert!(
        sim.sounds.iter().any(|s| s.group == 5 && s.index == 3),
        "unguarded hit plays hitsound"
    );
    assert!(sim.env_shake.time == 0, "light hit does not shake");

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert!(
        sim.sounds.iter().any(|s| s.group == 6 && s.index == 4),
        "guarded hit plays guardsound"
    );
    assert!(!sim.sounds.iter().any(|s| s.group == 5));

    // Heavy unguarded hits add a short camera shake.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut heavy = hitdef();
    heavy.damage = 100;
    heavy.hitsound = (-1, 0);
    heavy.guardsound = (-1, 0);
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &heavy, false, origin, 1, true);
    assert_eq!(sim.env_shake.time, 6);
    assert!(sim.sounds.is_empty(), "sound group -1 is silent");
}

#[test]
fn air_corner_push_uses_air_offset_and_threshold() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let right = sim.assets.stage.right_bound;
    let mut hit = hitdef();
    hit.ground_velocity = manato_core::Vec2::new(-4.0, 0.0);
    hit.air_velocity = manato_core::Vec2::new(-4.0, -6.0);
    hit.ground_cornerpush = -8.0;
    hit.air_cornerpush = -6.0;
    hit.guard_cornerpush = -8.0;
    hit.airguard_cornerpush = -6.0;
    let origin = sim.fighters[0].pos;

    // Airborne defender inside the 24px threshold uses air.cornerpush.
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos = manato_core::Vec2::new(right - 10.0, -20.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, -2.0, "4 - 6 air push");

    // The same airborne hit outside the threshold gets no push.
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos = manato_core::Vec2::new(right - 30.0, -20.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(
        sim.fighters[1].hit.vars.x_vel, 4.0,
        "outside corner threshold"
    );

    // A grounded defender at the same spot uses ground.cornerpush instead.
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Stand;
    sim.fighters[1].pos = manato_core::Vec2::new(right - 10.0, 0.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, -4.0, "4 - 8 ground push");

    // Guarded hits also receive the corner push.
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Stand;
    sim.fighters[1].pos = manato_core::Vec2::new(right - 10.0, 0.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, -4.0, "4 - 8 guard push");

    // Air guards select airguard.cornerpush.
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos = manato_core::Vec2::new(right - 10.0, -20.0);
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, -6.0, "airguard corner push");
}

#[test]
fn env_shake_advances_and_expires() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let shake = manato_formats::Controller {
        kind: "envshake".into(),
        params: vec![
            ("time".into(), "3".into()),
            ("freq".into(), "90".into()),
            ("ampl".into(), "-8".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &shake, &assets);
    assert_eq!(sim.env_shake.time, 3);
    let phase = sim.env_shake.phase;

    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.env_shake.time, 2);
    assert_eq!(
        sim.env_shake.phase, phase,
        "phase is an offset, not a counter"
    );
    assert!(
        sim.camera.shake.x.abs() > 0.001 || sim.camera.shake.y.abs() > 0.001,
        "active shake moves the camera"
    );

    // Two ticks reach time 0 (still applying the final frame); the next tick
    // observes the expired timer and clears the offset.
    for _ in 0..2 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.env_shake.time, 0);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.camera.shake, manato_core::Vec2::ZERO, "shake clears");
}

#[test]
fn down_and_bounce_fall_states_land_with_one_shot_damage() {
    for state in [5040, 5050] {
        let Some(mut sim) = fight_phase() else {
            return;
        };
        sim.fighters[1].state_no = state;
        sim.fighters[1].state_time = 1;
        sim.fighters[1].physics = manato_sim::fighter::Physics::None;
        sim.fighters[1].pos = manato_core::Vec2::new(20.0, 24.0);
        sim.fighters[1].vel = manato_core::Vec2::new(3.0, 10.0);
        sim.fighters[1].hit.vars.y_accel = 0.0;
        sim.fighters[1].hit.vars.fall = true;
        sim.fighters[1].hit.vars.fall_damage = 25;
        let life = sim.fighters[1].life;

        sim.tick([PlayerInput::default(); 2]);
        assert_eq!(sim.fighters[1].pos.y, 0.0, "state {state} lands at y=0");
        assert_eq!(sim.fighters[1].vel.y, 0.0, "state {state} stops falling");
        assert_eq!(sim.fighters[1].life, life - 25, "state {state} fall damage");
        assert_eq!(sim.fighters[1].hit.vars.fall_damage, 0);

        // A second grounded tick does not re-apply fall damage.
        sim.tick([PlayerInput::default(); 2]);
        assert_eq!(sim.fighters[1].life, life - 25);
        assert_eq!(sim.fighters[1].pos.y, 0.0);
    }
}

#[test]
fn fall_landing_without_damage_only_snaps_position() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].state_no = 5040;
    sim.fighters[1].state_time = 1;
    sim.fighters[1].physics = manato_sim::fighter::Physics::None;
    sim.fighters[1].pos = manato_core::Vec2::new(20.0, 30.0);
    sim.fighters[1].vel = manato_core::Vec2::new(2.0, 4.0);
    sim.fighters[1].hit.vars.y_accel = 0.0;
    sim.fighters[1].hit.vars.fall = true;
    sim.fighters[1].hit.vars.fall_damage = 0;
    let life = sim.fighters[1].life;

    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].pos.y, 0.0);
    assert_eq!(sim.fighters[1].vel.y, 0.0);
    assert_eq!(sim.fighters[1].life, life);
}

#[test]
fn guardflag_m_blocks_stand_and_crouch_but_not_air() {
    let attempt = |state_type: manato_sim::fighter::StateType, y: f32| -> bool {
        let Some(mut sim) = fight_phase() else {
            return false;
        };
        let assets = sim.assets.clone();
        sim.fighters[1].enter_state(0, &assets.characters[1], false);
        sim.fighters[1].append_input(PlayerInput::new(manato_core::DIR_RIGHT, 0), 0);
        sim.fighters[1].state_type = state_type;
        sim.fighters[1].pos = manato_core::Vec2::new(10.0, y);
        sim.fighters[1].facing = -1;
        sim.fighters[1].life = sim.fighters[1].life_max;
        sim.fighters[0].pos = manato_core::Vec2::new(-10.0, 0.0);
        sim.fighters[0].facing = 1;
        sim.fighters[0].enter_state(200, &assets.characters[0], false);
        sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
        let mut hit = hitdef();
        hit.guardflag = "M".into();
        hit.damage = 20;
        sim.fighters[0].hitdef = Some(hit);
        manato_sim::collision::resolve(&mut sim);
        sim.fighters[1].hit.guarding
    };

    use manato_sim::fighter::StateType;
    assert!(attempt(StateType::Stand, 0.0), "M blocks standing");
    assert!(attempt(StateType::Crouch, 0.0), "M blocks crouching");
    assert!(
        !attempt(StateType::Air, -20.0),
        "M does not grant air guard"
    );
}

#[test]
fn air_guard_uses_airguard_velocity_and_guard_hittime() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.airguard_velocity = manato_core::Vec2::new(-6.0, -2.0);
    hit.guard_hittime = 6;
    hit.ground_slidetime = 3;
    hit.guard_slidetime = 3;
    hit.guard_ctrltime = 5;
    hit.airguard_ctrltime = 3;
    hit.guard_damage = 5;
    hit.guard_damage_set = true;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos.y = -20.0;
    sim.fighters[1].facing = -1;
    let life = sim.fighters[1].life;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - 5, "guard damage");
    assert_eq!(
        sim.fighters[1].hit.vars.x_vel, 6.0,
        "airguard velocity mirrored"
    );
    assert_eq!(sim.fighters[1].hit.vars.y_vel, -2.0);
    assert_eq!(sim.fighters[1].hit.vars.hittime, 6);
    assert_eq!(sim.fighters[1].hit.vars.ctrltime, 3);
    assert_eq!(sim.fighters[1].state_no, 154);
}

#[test]
fn move_contact_hit_guarded_triggers_reflect_attacks() {
    let setup = |guarding: bool| -> Option<SimState> {
        let mut sim = fight_phase()?;
        let assets = sim.assets.clone();
        if guarding {
            sim.fighters[1].append_input(PlayerInput::new(manato_core::DIR_RIGHT, 0), 0);
        }
        sim.fighters[1].pos = manato_core::Vec2::new(5.0, 0.0);
        sim.fighters[1].facing = -1;
        sim.fighters[0].pos = manato_core::Vec2::new(-1.0, 0.0);
        sim.fighters[0].facing = 1;
        sim.fighters[0].enter_state(200, &assets.characters[0], false);
        sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
        let mut hit = hitdef();
        hit.pausetime = (0, 0);
        sim.fighters[0].hitdef = Some(hit);
        manato_sim::collision::resolve(&mut sim);
        Some(sim)
    };

    let sim = setup(false).expect("content");
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveContact"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveHit"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveGuarded"), 0);
    // A normal hit does not establish a target link; only throws do.
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumTarget"), 0);

    let sim = setup(true).expect("content");
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveContact"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveHit"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveGuarded"), 1);

    // MoveHitReset clears the contact/hit/guard flags.
    let mut sim = setup(false).expect("content");
    let assets = sim.assets.clone();
    let reset = manato_formats::Controller {
        kind: "movehitreset".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &reset, &assets);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveContact"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveHit"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveGuarded"), 0);
}

#[test]
fn hitdef_parses_guard_velocity_hittime_and_airguard() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "20, 6".into()),
            ("ground.velocity".into(), "-4, 0".into()),
            ("guard.velocity".into(), "-10, 1".into()),
            ("airguard.velocity".into(), "-6, -2".into()),
            ("guard.hittime".into(), "7".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert_eq!(
        parsed.guard_velocity,
        Some(manato_core::Vec2::new(-10.0, 1.0))
    );
    assert_eq!(parsed.airguard_velocity, manato_core::Vec2::new(-6.0, -2.0));
    assert_eq!(parsed.guard_hittime, 7);
    assert_eq!(parsed.guard_damage, 6);
    // Guard flag defaults to MA when omitted.
    assert_eq!(parsed.guardflag, "MA");

    // Supplying guard.velocity drives the guarded reaction directly.
    let origin = sim.fighters[0].pos;
    sim.fighters[1].facing = -1;
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, true, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - 6);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, 10.0, "-10 mirrored");
    assert_eq!(sim.fighters[1].hit.vars.y_vel, 1.0);
    assert_eq!(sim.fighters[1].hit.vars.hittime, 7);

    // Without guard.velocity the ground velocity is used instead.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let fallback = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "20".into()),
            ("ground.velocity".into(), "-8, 0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &fallback, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert_eq!(parsed.guard_velocity, None);
    let origin = sim.fighters[0].pos;
    sim.fighters[1].facing = -1;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, true, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, 8.0, "-8 mirrored");
}

#[test]
fn hitdef_airguard_velocity_defaults_from_air_velocity() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "10, 2".into()),
            ("air.velocity".into(), "-4, -6".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert_eq!(
        parsed.airguard_velocity,
        manato_core::Vec2::new(-6.0, -3.0),
        "default is air.velocity.x * 1.5, air.velocity.y * 0.5"
    );

    // The default is applied directly to an air guard (no extra halving).
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos.y = -20.0;
    sim.fighters[1].facing = -1;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, true, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, 6.0);
    assert_eq!(sim.fighters[1].hit.vars.y_vel, -3.0);

    // An explicit airguard.velocity overrides the derived value.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let explicit = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("air.velocity".into(), "-4, -6".into()),
            ("airguard.velocity".into(), "-8, -1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &explicit, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert_eq!(parsed.airguard_velocity, manato_core::Vec2::new(-8.0, -1.0));
}

#[test]
fn hitdef_parses_ground_air_types_times_and_cornerpush() {
    use manato_sim::hitdef::{AnimType, GroundType};

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "20, 4".into()),
            ("guard.damage".into(), "2".into()),
            ("animtype".into(), "DiagUp".into()),
            ("ground.type".into(), "Low".into()),
            ("air.type".into(), "Trip".into()),
            ("ground.velocity".into(), "-3, 0".into()),
            ("air.velocity".into(), "-5, -6".into()),
            ("ground.hittime".into(), "9".into()),
            ("air.hittime".into(), "21".into()),
            ("ground.slidetime".into(), "6".into()),
            ("guard.hittime".into(), "7".into()),
            ("guard.slidetime".into(), "4".into()),
            ("guard.ctrltime".into(), "5".into()),
            ("airguard.ctrltime".into(), "3".into()),
            ("ground.cornerpush.veloff".into(), "-2.5".into()),
            ("air.cornerpush.veloff".into(), "-3.5".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");

    assert_eq!(parsed.anim_type, AnimType::DiagUp);
    assert_eq!(parsed.ground_type, GroundType::Low);
    assert_eq!(parsed.air_type, GroundType::Trip);
    assert_eq!(parsed.damage, 20);
    assert_eq!(parsed.guard_damage, 2);
    assert!(parsed.guard_damage_set);
    assert_eq!(parsed.ground_velocity, manato_core::Vec2::new(-3.0, 0.0));
    assert_eq!(parsed.air_velocity, manato_core::Vec2::new(-5.0, -6.0));
    assert_eq!(parsed.ground_hittime, 9);
    assert_eq!(parsed.air_hittime, 21);
    assert_eq!(parsed.ground_slidetime, 6);
    assert_eq!(parsed.guard_hittime, 7);
    assert_eq!(parsed.guard_slidetime, 4);
    assert_eq!(parsed.guard_ctrltime, 5);
    assert_eq!(parsed.airguard_ctrltime, 3);
    assert_eq!(parsed.ground_cornerpush, -2.5);
    assert_eq!(parsed.air_cornerpush, -3.5);

    // Ground hit: type codes follow M.U.G.E.N (High/Low/Trip = 1/2/3) and
    // GetHitVar(type) selects the ground type.
    let origin = sim.fighters[0].pos;
    sim.fighters[1].facing = -1;
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - 20);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, 3.0);
    assert_eq!(sim.fighters[1].hit.vars.hittime, 9);
    assert_eq!(sim.fighters[1].hit.vars.slidetime, 6);
    assert_eq!(sim.fighters[1].hit.vars.ctrltime, 9);
    // A grounded DiagUp type with no Y velocity degrades to Hard/Heavy.
    assert_eq!(sim.fighters[1].hit.vars.anim_type, 2);
    assert_eq!(sim.fighters[1].hit.vars.ground_type, 2);
    assert_eq!(sim.fighters[1].hit.vars.air_type, 3);
    assert_eq!(sim.fighters[1].hit.vars.hit_type, 2);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(type)"), 2);

    // Guarded ground hit uses guard.damage, guard.hittime and guard.slidetime.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    let origin = sim.fighters[0].pos;
    sim.fighters[1].facing = -1;
    let life = sim.fighters[1].life;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, true, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - 2);
    assert!(sim.fighters[1].hit.vars.guarded);
    assert_eq!(sim.fighters[1].hit.vars.hittime, 7);
    assert_eq!(sim.fighters[1].hit.vars.slidetime, 4);
    assert_eq!(sim.fighters[1].hit.vars.ctrltime, 5);
    assert_eq!(sim.fighters[1].state_no, 150);

    // Air hit: air velocity and air.hittime apply, and GetHitVar(type)
    // selects the air type.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos.y = -20.0;
    sim.fighters[1].facing = -1;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, 5.0);
    assert_eq!(sim.fighters[1].hit.vars.y_vel, -6.0);
    assert_eq!(sim.fighters[1].hit.vars.hittime, 21);
    assert_eq!(sim.fighters[1].hit.vars.ctrltime, 21);
    assert_eq!(sim.fighters[1].hit.vars.hit_type, 3);
    assert_eq!(sim.fighters[1].state_no, 5020);
}

#[test]
fn hitdef_defaults_match_documented_ground_air_and_guard_times() {
    use manato_sim::hitdef::GroundType;

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let base = vec![("attr".to_string(), "S, NA".to_string())];

    // air.type inherits ground.type; times use the documented defaults.
    let mut params = base.clone();
    params.push(("ground.type".into(), "Low".into()));
    let hit = parse(&mut sim, params);
    assert_eq!(hit.ground_type, GroundType::Low);
    assert_eq!(
        hit.air_type,
        GroundType::Low,
        "air.type inherits ground.type"
    );
    assert_eq!(hit.ground_hittime, 0);
    assert_eq!(hit.ground_slidetime, 0);
    assert_eq!(hit.air_hittime, 20);
    assert_eq!(
        hit.guard_hittime, 0,
        "guard.hittime inherits ground.hittime"
    );
    assert_eq!(
        hit.guard_slidetime, 0,
        "guard.slidetime inherits guard.hittime"
    );
    assert_eq!(
        hit.guard_ctrltime, 0,
        "guard.ctrltime inherits guard.slidetime"
    );
    assert_eq!(
        hit.airguard_ctrltime, 0,
        "airguard.ctrltime inherits guard.ctrltime"
    );

    // An explicit guard.hittime feeds guard.slidetime when the latter is
    // omitted.
    let mut params = base.clone();
    params.push(("guard.hittime".into(), "7".into()));
    let hit = parse(&mut sim, params);
    assert_eq!(hit.guard_hittime, 7);
    assert_eq!(hit.guard_slidetime, 7);
    assert_eq!(hit.guard_ctrltime, 7);
    assert_eq!(hit.airguard_ctrltime, 7);

    // An explicit guard.slidetime wins over the inherited value.
    let mut params = base.clone();
    params.push(("guard.hittime".into(), "7".into()));
    params.push(("guard.slidetime".into(), "3".into()));
    let hit = parse(&mut sim, params);
    assert_eq!(hit.guard_hittime, 7);
    assert_eq!(hit.guard_slidetime, 3);
    assert_eq!(hit.guard_ctrltime, 3);
    assert_eq!(hit.airguard_ctrltime, 3);

    // Explicit guard.ctrltime feeds airguard.ctrltime.
    let mut params = base.clone();
    params.push(("guard.hittime".into(), "7".into()));
    params.push(("guard.slidetime".into(), "3".into()));
    params.push(("guard.ctrltime".into(), "2".into()));
    let hit = parse(&mut sim, params);
    assert_eq!(hit.guard_ctrltime, 2);
    assert_eq!(hit.airguard_ctrltime, 2);

    // Explicit airguard.ctrltime wins.
    let mut params = base;
    params.push(("guard.ctrltime".into(), "2".into()));
    params.push(("airguard.ctrltime".into(), "1".into()));
    let hit = parse(&mut sim, params);
    assert_eq!(hit.guard_ctrltime, 2);
    assert_eq!(hit.airguard_ctrltime, 1);
}

#[test]
fn hitdef_none_type_pauses_without_damage_or_state_change() {
    use manato_sim::hitdef::GroundType;

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "20, 4".into()),
            ("guardflag".into(), "MA".into()),
            ("ground.type".into(), "None".into()),
            ("ground.velocity".into(), "-4".into()),
            ("pausetime".into(), "6, 9".into()),
            ("sparkno".into(), "0".into()),
            ("hitsound".into(), "5, 0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert_eq!(parsed.ground_type, GroundType::None);
    assert_eq!(parsed.air_type, GroundType::None, "air.type inherits None");

    let origin = sim.fighters[0].pos;
    let life = sim.fighters[1].life;
    let state = sim.fighters[1].state_no;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);

    // Pause both sides, but do not damage, move, spark, or change state.
    assert_eq!(sim.fighters[1].life, life);
    assert_eq!(sim.fighters[1].state_no, state);
    assert_eq!(sim.fighters[1].hit.vars.damage, 0);
    assert_eq!(sim.fighters[0].hit.hit_pause, 6);
    assert_eq!(sim.fighters[0].hit.hit_pause_time, 6);
    assert_eq!(sim.fighters[1].hit.hit_pause, 9);
    assert_eq!(sim.fighters[1].hit.hit_shake_timer, 9);
    assert_eq!(sim.fighters[1].hit.hit_pause_time, 0);
    assert!(!sim.fighters[0].attack.move_hit);
    assert!(!sim.fighters[0].attack.move_contact);
    assert!(sim.effects.is_empty(), "no spark for a pause-only hit");
    // The contact is consumed so the same activation does not pause twice.
    assert_eq!(sim.fighters[0].hit.hit_targets, vec![1]);

    // An airborne defender uses air.type; None pauses without a hit state.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "20".into()),
            ("ground.type".into(), "High".into()),
            ("air.type".into(), "None".into()),
            ("air.velocity".into(), "-5, -6".into()),
            ("pausetime".into(), "3, 4".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos.y = -20.0;
    let state = sim.fighters[1].state_no;
    let life = sim.fighters[1].life;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life);
    assert_eq!(sim.fighters[1].state_no, state);
    assert_eq!(sim.fighters[0].hit.hit_pause, 3);
    assert_eq!(sim.fighters[1].hit.hit_pause, 4);
}

#[test]
fn hitdef_cornerpush_defaults_follow_guard_velocity_and_attr() {
    use manato_sim::hitdef::GroundType;

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, params: Vec<(String, String)>| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].hitdef.clone().expect("hitdef")
    };
    let close = |a: f32, b: f32| (a - b).abs() < 1e-5;

    // Ground attacks default to 1.3x the effective guard velocity.
    let hit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("ground.velocity".into(), "-4".into()),
        ],
    );
    assert!(close(hit.ground_cornerpush, -5.2));
    assert_eq!(hit.air_cornerpush, hit.ground_cornerpush);
    assert_eq!(hit.guard_cornerpush, hit.ground_cornerpush);
    assert_eq!(hit.airguard_cornerpush, hit.guard_cornerpush);

    // An explicit guard.velocity wins over ground.velocity.
    let hit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("ground.velocity".into(), "-4".into()),
            ("guard.velocity".into(), "-10, 0".into()),
        ],
    );
    assert!(close(hit.ground_cornerpush, -13.0));

    // Air attacks default to no corner push.
    let hit = parse(
        &mut sim,
        vec![
            ("attr".into(), "A, NA".into()),
            ("ground.velocity".into(), "-4".into()),
        ],
    );
    assert_eq!(hit.ground_cornerpush, 0.0);
    assert_eq!(hit.air_cornerpush, 0.0);
    assert_eq!(hit.guard_cornerpush, 0.0);
    assert_eq!(hit.airguard_cornerpush, 0.0);

    // Explicit values override, and air inherits ground when omitted.
    let hit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("ground.cornerpush.veloff".into(), "-2.5".into()),
        ],
    );
    assert_eq!(hit.ground_cornerpush, -2.5);
    assert_eq!(hit.air_cornerpush, -2.5);
    assert_eq!(hit.guard_cornerpush, -2.5);
    assert_eq!(hit.airguard_cornerpush, -2.5);

    let hit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("ground.cornerpush.veloff".into(), "-2.5".into()),
            ("air.cornerpush.veloff".into(), "-3.5".into()),
            ("guard.cornerpush.veloff".into(), "-1.25".into()),
        ],
    );
    assert_eq!(hit.ground_cornerpush, -2.5);
    assert_eq!(hit.air_cornerpush, -3.5);
    assert_eq!(hit.guard_cornerpush, -1.25);
    assert_eq!(hit.airguard_cornerpush, -1.25);

    let hit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("guard.cornerpush.veloff".into(), "-1.25".into()),
            ("airguard.cornerpush.veloff".into(), "-4.0".into()),
        ],
    );
    assert_eq!(hit.guard_cornerpush, -1.25);
    assert_eq!(hit.airguard_cornerpush, -4.0);

    // `None` parses with the documented code instead of falling back to High.
    let hit = parse(
        &mut sim,
        vec![
            ("attr".into(), "S, NA".into()),
            ("ground.type".into(), "None".into()),
        ],
    );
    assert_eq!(hit.ground_type, GroundType::None);
    assert_eq!(hit.ground_type.code(), 0);
}

#[test]
fn guard_cornerpush_uses_guard_specific_veloff() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "10, 2".into()),
            ("guardflag".into(), "MA".into()),
            ("ground.velocity".into(), "-4".into()),
            ("guard.velocity".into(), "-10, 0".into()),
            ("ground.cornerpush.veloff".into(), "-8".into()),
            ("air.cornerpush.veloff".into(), "-6".into()),
            ("guard.cornerpush.veloff".into(), "-1.5".into()),
            ("airguard.cornerpush.veloff".into(), "-2.5".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");

    // Guarded ground hit near the right corner uses guard.cornerpush rather
    // than ground.cornerpush. Guard velocity -10 is mirrored to +10.
    let right = sim.assets.stage.right_bound;
    sim.fighters[1].pos.x = right;
    sim.fighters[1].facing = -1;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, true, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, 8.5);

    // Air guard uses airguard.cornerpush.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos = manato_core::Vec2::new(right, -20.0);
    sim.fighters[1].facing = -1;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, true, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, -2.5);
}

#[test]
fn hitdef_parses_yaccel_defaults_and_overrides() {
    use manato_sim::hitdef::HitDef;

    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![("attr".into(), "S, NA".into())],
        line: 0,
    };
    let at = |localcoord| HitDef::from_controller_for(&control, 1, localcoord, 160);
    // The documented defaults are .35 at 240p, .7 at 480p, and 1.4 at 720p.
    assert!((at((320.0, 240.0)).yaccel - 0.35).abs() < 1e-6);
    assert!((at((640.0, 480.0)).yaccel - 0.7).abs() < 1e-6);
    assert!((at((1280.0, 720.0)).yaccel - 1.4).abs() < 1e-6);
    assert!((at((320.0, 240.0)).fall_yvel + 4.5).abs() < 1e-6);
    assert!((at((640.0, 480.0)).fall_yvel + 9.0).abs() < 1e-6);
    assert!((at((1280.0, 720.0)).fall_yvel + 18.0).abs() < 1e-6);

    let explicit = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("yaccel".into(), "0.85".into()),
            ("fall.yvelocity".into(), "6".into()),
        ],
        line: 0,
    };
    let hit = HitDef::from_controller_for(&explicit, 1, (640.0, 480.0), 160);
    assert!((hit.yaccel - 0.85).abs() < 1e-6);
    assert!((hit.fall_yvel - 6.0).abs() < 1e-6);
}

#[test]
fn hitdef_yaccel_reports_and_drives_air_hit_physics() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "10".into()),
            ("yaccel".into(), "1.5".into()),
            ("air.velocity".into(), "0, 0".into()),
            ("pausetime".into(), "0, 0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert_eq!(parsed.yaccel, 1.5);

    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos.y = -40.0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &parsed, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::num(&sim, 1, "GetHitVar(yaccel)"), 1.5);

    // The hit reaction uses the HitDef yaccel instead of movement.yaccel.
    sim.fighters[1].state_no = 5030;
    sim.fighters[1].state_time = 1;
    sim.fighters[1].move_type = manato_sim::fighter::MoveType::Hit;
    sim.fighters[1].physics = manato_sim::fighter::Physics::Air;
    sim.fighters[1].vel = manato_core::Vec2::ZERO;
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        (sim.fighters[1].vel.y - 1.5).abs() < 1e-4,
        "expected one 1.5 yaccel step, got {}",
        sim.fighters[1].vel.y
    );
}

#[test]
fn hitdef_yaccel_defaults_from_the_character_localcoord() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![("attr".into(), "S, NA".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let parsed = sim.fighters[0].hitdef.clone().expect("hitdef");
    // Bundled characters use localcoord 320,240.
    assert!((parsed.yaccel - 0.35).abs() < 1e-6);
}

#[test]
fn round_timer_clamps_at_zero_and_stays_frozen_after_over() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.round.rounds_to_win = 1;
    sim.fighters[0].life = 800;
    sim.fighters[1].life = 300;
    sim.round.timer = 1;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    assert_eq!(sim.round.timer, 0, "timer clamps at zero on time over");
    assert!(sim.round.time_over);
    assert_eq!(sim.round.winner, Some(0), "higher life wins the timeout");
    assert_eq!(manato_sim::trigger::int(&sim, 0, "RoundTime"), 0);

    // Stays at zero through the KO phase.
    for _ in 0..100 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.timer, 0);

    // Match end reaches Over without the timer going negative.
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Over {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Over);
    assert!(sim.finished);
    let over_timer = sim.round.timer;
    for _ in 0..30 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.timer, over_timer, "Over never ticks the timer");
    assert!(sim.round.timer >= 0);

    // Rounding boundaries for the seconds display.
    let mut round = sim.round.clone();
    round.timer = 0;
    assert_eq!(round.seconds(), 0);
    round.timer = 1;
    assert_eq!(round.seconds(), 1);
    round.timer = 59;
    assert_eq!(round.seconds(), 1);
    round.timer = 60;
    assert_eq!(round.seconds(), 1);
    round.timer = 61;
    assert_eq!(round.seconds(), 2);
}

#[test]
fn helper_postype_anchors() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos = manato_core::Vec2::new(-40.0, 0.0);
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos = manato_core::Vec2::new(60.0, -10.0);
    sim.camera.x = 0.0;
    let spawn = |sim: &mut SimState, postype: Option<&str>, pos: &str| {
        let mut params = vec![
            ("stateno".to_string(), "0".to_string()),
            ("id".to_string(), "12".to_string()),
            ("removetime".to_string(), "-1".to_string()),
            ("pos".to_string(), pos.to_string()),
        ];
        if let Some(postype) = postype {
            params.push(("postype".to_string(), postype.to_string()));
        }
        let control = manato_formats::Controller {
            kind: "helper".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        let helper = sim.fighters.last().expect("helper spawned");
        helper.pos
    };

    let p1 = spawn(&mut sim, None, "0, 0");
    assert_eq!(p1, manato_core::Vec2::new(-40.0, 0.0));

    let p2 = spawn(&mut sim, Some("p2"), "5, -3");
    assert_eq!(p2, manato_core::Vec2::new(65.0, -13.0));

    let front = spawn(&mut sim, Some("front"), "0, 0");
    assert_eq!(front.x, manato_core::GAME_WIDTH * 0.5);
    let back = spawn(&mut sim, Some("back"), "0, 0");
    assert_eq!(back.x, -manato_core::GAME_WIDTH * 0.5);

    let left = spawn(&mut sim, Some("left"), "0, 0");
    assert_eq!(left.x, -manato_core::GAME_WIDTH * 0.5);
    let right = spawn(&mut sim, Some("right"), "0, 0");
    assert_eq!(right.x, manato_core::GAME_WIDTH * 0.5);
}

#[test]
fn explod_and_gamemakeanim_front_back_use_screen_edges() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos = manato_core::Vec2::new(-40.0, 0.0);
    sim.fighters[0].facing = 1;
    sim.camera.x = 0.0;
    let spawn = |sim: &mut SimState, kind: &str, postype: &str, pos: &str| {
        let control = manato_formats::Controller {
            kind: kind.into(),
            params: vec![
                ("anim".into(), "10".into()),
                ("id".into(), "13".into()),
                ("pos".into(), pos.into()),
                ("postype".into(), postype.into()),
                ("bindtime".into(), "5".into()),
                ("removetime".into(), "60".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.effects.last().expect("effect").clone()
    };

    // Explod front/back anchor to the screen edge and do not bind to the owner.
    let front = spawn(&mut sim, "explod", "front", "10, -5");
    assert_eq!(front.pos, manato_core::Vec2::new(170.0, -5.0));
    assert!(!front.bound, "edge-anchored explod is not bound");
    assert!(front.bind_target.is_none());

    let back = spawn(&mut sim, "explod", "back", "10, -5");
    assert_eq!(back.pos, manato_core::Vec2::new(-150.0, -5.0));

    // Facing flips which edge is front.
    sim.fighters[0].facing = -1;
    let front_left = spawn(&mut sim, "explod", "front", "10, -5");
    assert_eq!(front_left.pos, manato_core::Vec2::new(-170.0, -5.0));
    let back_right = spawn(&mut sim, "explod", "back", "10, -5");
    assert_eq!(back_right.pos, manato_core::Vec2::new(150.0, -5.0));
    sim.fighters[0].facing = 1;

    // GameMakeAnim follows the same edge rule.
    let anim_front = spawn(&mut sim, "gamemakeanim", "front", "0, 0");
    assert_eq!(anim_front.pos.x, 160.0);
    let anim_back = spawn(&mut sim, "gamemakeanim", "back", "0, 0");
    assert_eq!(anim_back.pos.x, -160.0);
}

#[test]
fn explod_bindtime_release_and_edge_unbound_behavior() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos = manato_core::Vec2::new(-40.0, 0.0);
    sim.fighters[0].facing = 1;
    sim.camera.x = 0.0;
    let spawn = |sim: &mut SimState, postype: &str, bindtime: &str| {
        let control = manato_formats::Controller {
            kind: "explod".into(),
            params: vec![
                ("anim".into(), "10".into()),
                ("id".into(), "21".into()),
                ("pos".into(), "10, -5".into()),
                ("postype".into(), postype.into()),
                ("bindtime".into(), bindtime.into()),
                ("removeonchangestate".into(), "0".into()),
                ("vel".into(), "2, 0".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.effects.last().expect("explod").clone()
    };

    // p1 explod binds to the owner, then releases after bindtime.
    let bound = spawn(&mut sim, "p1", "2");
    assert!(bound.bound);
    assert_eq!(bound.bind_time, 2);
    assert_eq!(bound.pos, manato_core::Vec2::new(-30.0, -5.0));
    sim.tick([PlayerInput::default(); 2]);
    let effect = sim.effects.last().expect("bound explod");
    assert!(effect.bound);
    assert_eq!(effect.bind_time, 1);
    sim.fighters[0].pos.x = 10.0;
    sim.tick([PlayerInput::default(); 2]);
    let effect = sim.effects.last().expect("released explod");
    assert!(!effect.bound, "bindtime expired");
    assert_eq!(effect.pos, manato_core::Vec2::new(20.0, -5.0));

    // After release the explod moves by its own velocity.
    sim.fighters[0].pos.x = 100.0;
    sim.tick([PlayerInput::default(); 2]);
    let effect = sim.effects.last().expect("free explod");
    assert!(!effect.bound);
    assert_eq!(effect.pos.x, 22.0, "moves by vel, not with the owner");

    // Edge-anchored explods ignore bindtime and never follow the owner.
    sim.fighters[0].pos = manato_core::Vec2::new(-40.0, 0.0);
    sim.fighters[0].facing = 1;
    sim.fighters[0].ctrl = true;
    sim.fighters[0].move_type = manato_sim::fighter::MoveType::Idle;
    sim.fighters[1].pos = manato_core::Vec2::new(300.0, 0.0);
    sim.camera.x = 0.0;
    let edge = spawn(&mut sim, "front", "5");
    assert!(!edge.bound);
    assert!(edge.bind_target.is_none());
    assert_eq!(edge.pos, manato_core::Vec2::new(170.0, -5.0));
    sim.fighters[0].pos.x = -200.0;
    sim.tick([PlayerInput::default(); 2]);
    let effect = sim
        .effects
        .iter()
        .find(|effect| effect.id == 21 && effect.pos.x > 160.0)
        .expect("edge explod");
    assert!(!effect.bound, "edge explod stays unbound");
    assert_eq!(effect.pos.x, 172.0, "edge explod moves by velocity only");
}

#[test]
fn gamemakeanim_left_right_follow_camera_edges() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].facing = 1;
    sim.camera.x = 50.0;
    let spawn = |sim: &mut SimState, postype: &str, pos: &str| {
        let control = manato_formats::Controller {
            kind: "gamemakeanim".into(),
            params: vec![
                ("anim".into(), "10".into()),
                ("pos".into(), pos.into()),
                ("postype".into(), postype.into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.effects.last().expect("effect").pos
    };
    assert_eq!(spawn(&mut sim, "left", "0, 0").x, -110.0);
    assert_eq!(spawn(&mut sim, "right", "0, 0").x, 210.0);

    // pos.x is facing-mirrored.
    sim.fighters[0].facing = -1;
    assert_eq!(spawn(&mut sim, "left", "10, 0").x, -120.0);
    assert_eq!(spawn(&mut sim, "right", "10, 0").x, 200.0);
}

#[test]
fn projectile_hits_limit_removes_after_final_contact() {
    let setup = |hits: i32| -> Option<SimState> {
        let mut sim = fight_phase()?;
        let assets = sim.assets.clone();
        sim.fighters[0].pos.x = -5.0;
        sim.fighters[0].facing = 1;
        sim.fighters[1].pos.x = 5.0;
        sim.fighters[1].facing = -1;
        let control = manato_formats::Controller {
            kind: "projectile".into(),
            params: vec![
                ("projanim".into(), "1500".into()),
                ("velocity".into(), "0, 0".into()),
                ("removetime".into(), "-1".into()),
                ("damage".into(), "10".into()),
                ("hits".into(), hits.to_string()),
                ("postype".into(), "p1".into()),
                ("pos".into(), "0, 0".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(&mut sim, 0, &control, &assets);
        assert_eq!(sim.projectiles.len(), 1);
        Some(sim)
    };

    let mut sim = setup(1).expect("content");
    let life = sim.fighters[1].life;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].life, life - 10, "projectile hits once");
    assert!(
        sim.projectiles.is_empty(),
        "projectile is removed after its last hit"
    );
    for _ in 0..5 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.fighters[1].life, life - 10, "no repeat damage");

    // hits = 2 survives the first contact with one hit remaining.
    let mut sim = setup(2).expect("content");
    let life = sim.fighters[1].life;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].life, life - 10);
    assert_eq!(sim.projectiles.len(), 1);
    assert_eq!(sim.projectiles[0].hits, 1, "one hit left");
}

#[test]
fn vel_add_set_facing_and_power_clamp() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].facing = -1;
    sim.fighters[0].vel = manato_core::Vec2::new(1.0, 1.0);
    let add = manato_formats::Controller {
        kind: "veladd".into(),
        params: vec![("x".into(), "2".into()), ("y".into(), "-3".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &add, &assets);
    assert_eq!(sim.fighters[0].vel, manato_core::Vec2::new(-1.0, -2.0));

    // VelSet mirrors x and leaves an omitted y unchanged.
    let set = manato_formats::Controller {
        kind: "velset".into(),
        params: vec![("x".into(), "4".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &set, &assets);
    assert_eq!(sim.fighters[0].vel, manato_core::Vec2::new(-4.0, -2.0));

    // Power never goes below zero.
    sim.fighters[0].power = 50.0;
    let power_add = manato_formats::Controller {
        kind: "poweradd".into(),
        params: vec![("value".into(), "-100".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &power_add, &assets);
    assert_eq!(sim.fighters[0].power, 0.0);
    let power_set = manato_formats::Controller {
        kind: "powerset".into(),
        params: vec![("value".into(), "-5".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &power_set, &assets);
    assert_eq!(sim.fighters[0].power, 0.0);
}

#[test]
fn trans_variants_only_use_alpha_for_alpha_blends() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let apply = |sim: &mut SimState, kind: &str, alpha: Option<&str>| {
        let mut params = vec![("trans".to_string(), kind.to_string())];
        if let Some(alpha) = alpha {
            params.push(("alpha".to_string(), alpha.to_string()));
        }
        let control = manato_formats::Controller {
            kind: "trans".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].trans_alpha
    };

    for kind in ["default", "none", "add", "add1", "sub", "sub1"] {
        assert_eq!(
            apply(&mut sim, kind, Some("64")),
            1.0,
            "{kind} stays opaque even with an alpha parameter"
        );
    }
    assert!((apply(&mut sim, "addalpha", Some("64")) - 64.0 / 255.0).abs() < 0.001);
    assert!((apply(&mut sim, "alpha", Some("128")) - 128.0 / 255.0).abs() < 0.001);
    // Pair form uses the destination byte over 256.
    assert!((apply(&mut sim, "addalpha", Some("256, 128")) - 0.5).abs() < 0.001);
    assert!((apply(&mut sim, "alpha", Some("0, 64")) - 0.25).abs() < 0.001);
    assert_eq!(apply(&mut sim, "addalpha", None), 1.0, "no alpha = opaque");
}

#[test]
fn afterimage_time_zero_disables_and_negative_is_infinite() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let after = manato_formats::Controller {
        kind: "afterimage".into(),
        params: vec![
            ("time".into(), "10".into()),
            ("timegap".into(), "2".into()),
            ("length".into(), "2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &after, &assets);

    // AfterImageTime 1 is normalized to 0 and disables the emitter.
    let stop = manato_formats::Controller {
        kind: "afterimagetime".into(),
        params: vec![("time".into(), "1".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &stop, &assets);
    assert_eq!(sim.fighters[0].after_image.as_ref().expect("state").time, 0);

    // Re-enable, then time = 0 also disables emission.
    manato_sim::controller::execute(&mut sim, 0, &after, &assets);
    let zero = manato_formats::Controller {
        kind: "afterimagetime".into(),
        params: vec![("time".into(), "0".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &zero, &assets);
    assert_eq!(sim.fighters[0].after_image.as_ref().expect("state").time, 0);

    // A longer timer resets the sampling counter.
    manato_sim::controller::execute(&mut sim, 0, &after, &assets);
    sim.tick([PlayerInput::default(); 2]);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].after_images.len(), 1);
    let extend = manato_formats::Controller {
        kind: "afterimagetime".into(),
        params: vec![("time".into(), "10".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &extend, &assets);
    let state = sim.fighters[0].after_image.as_ref().expect("active");
    assert_eq!(state.time, 10);
    assert_eq!(state.tick, 0, "counter resets");

    // Negative time keeps emitting without expiring.
    let infinite = manato_formats::Controller {
        kind: "afterimagetime".into(),
        params: vec![("time".into(), "-1".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &infinite, &assets);
    for _ in 0..30 {
        sim.tick([PlayerInput::default(); 2]);
    }
    let state = sim.fighters[0]
        .after_image
        .as_ref()
        .expect("still emitting");
    assert_eq!(state.time, -1);
    assert!(!sim.fighters[0].after_images.is_empty());
}

#[test]
fn numhelper_filters_by_id() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, id: i32| {
        let control = manato_formats::Controller {
            kind: "helper".into(),
            params: vec![
                ("stateno".into(), "0".into()),
                ("id".into(), id.to_string()),
                ("removetime".into(), "-1".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    spawn(&mut sim, 4);
    spawn(&mut sim, 4);
    spawn(&mut sim, 5);

    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumHelper"), 3);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumHelper(4)"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumHelper(5)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumHelper(9)"), 0);

    // A helper sees the same sibling set through its root.
    let helper = sim.fighters.len() - 1;
    assert_eq!(manato_sim::trigger::int(&sim, helper, "NumHelper"), 3);
    assert_eq!(manato_sim::trigger::int(&sim, helper, "NumHelper(4)"), 2);

    // The opponent has no helpers.
    assert_eq!(manato_sim::trigger::int(&sim, 1, "NumHelper"), 0);
}

#[test]
fn hitby_not_hitby_wildcard_filters() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.attr.state_types = "S".into();
    hit.attr.class = "NA".into();
    let origin = sim.fighters[0].pos;
    let life = sim.fighters[1].life;

    // Full wildcard blocks any attack.
    sim.fighters[1].hit.not_hit_by = Some(("*, *".into(), 60));
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life);

    // State wildcard with a specific class still blocks NA.
    sim.fighters[1].hit.not_hit_by = Some(("SCA, *".into(), 60));
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life);

    // Non-matching class with a wildcard state does not block.
    sim.fighters[1].hit.not_hit_by = Some(("*, SA".into(), 60));
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - hit.damage);

    // HitBy whitelist with a wildcard class allows the attack through a
    // permissive NotHitBy.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let life = sim.fighters[1].life;
    sim.fighters[1].hit.not_hit_by = Some(("*, *".into(), 60));
    sim.fighters[1].hit.hit_by = Some(("SCA, *".into(), 60));
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - hit.damage);

    // An empty filter never blocks.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let life = sim.fighters[1].life;
    sim.fighters[1].hit.not_hit_by = Some(("   ".into(), 60));
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - hit.damage);
}

#[test]
fn root_parent_self_redirects_from_helper() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "31".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    let index = sim.fighters.len() - 1;
    sim.fighters[0].state_no = 200;
    sim.fighters[0].vars[2] = 77;
    sim.fighters[1].state_no = 50;
    sim.fighters[index].state_no = 20;

    assert_eq!(manato_sim::trigger::int(&sim, index, "Root, stateno"), 200);
    assert_eq!(manato_sim::trigger::int(&sim, index, "Parent, var(2)"), 77);
    assert_eq!(manato_sim::trigger::int(&sim, index, "Self, stateno"), 20);
    assert_eq!(manato_sim::trigger::int(&sim, index, "Enemy, stateno"), 50);
    assert_eq!(manato_sim::trigger::int(&sim, index, "Root, PlayerID"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, index, "P2, PlayerID"), 1);
}

#[test]
fn helper_relation_triggers_follow_axis_facing_and_ids() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, assets: &manato_sim::Assets, owner: usize, id: i32| {
        let control = manato_formats::Controller {
            kind: "helper".into(),
            params: vec![
                ("stateno".into(), "0".into()),
                ("id".into(), id.to_string()),
                ("removetime".into(), "-1".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, owner, &control, assets);
        sim.fighters.len() - 1
    };
    let helper = spawn(&mut sim, &assets, 0, 31);

    // The root is at (40,-5) facing right; the helper at (30,15) facing left.
    sim.fighters[0].pos = manato_core::Vec2::new(40.0, -5.0);
    sim.fighters[0].facing = 1;
    sim.fighters[helper].pos = manato_core::Vec2::new(30.0, 15.0);
    sim.fighters[helper].facing = -1;

    assert_eq!(manato_sim::trigger::int(&sim, 0, "IsHelper"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, helper, "IsHelper"), 1);
    assert_eq!(sim.fighters[helper].helper_id, 31);
    assert_eq!(manato_sim::trigger::int(&sim, helper, "IsHelper(31)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, helper, "IsHelper(7)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "IsHelper(31)"), 0);

    // X distance is positive when the relation is in front of the child.
    assert_eq!(
        manato_sim::trigger::num(&sim, helper, "ParentDist X"),
        -10.0
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, helper, "ParentDist Y"),
        -20.0
    );
    assert_eq!(manato_sim::trigger::num(&sim, helper, "RootDist X"), -10.0);
    assert_eq!(manato_sim::trigger::num(&sim, helper, "RootDist Y"), -20.0);

    // Facing flips the sign of the X component.
    sim.fighters[helper].facing = 1;
    assert_eq!(manato_sim::trigger::num(&sim, helper, "ParentDist X"), 10.0);
    assert_eq!(manato_sim::trigger::num(&sim, helper, "RootDist X"), 10.0);

    // A KO'd relation reports no position instead of a stale distance.
    sim.fighters[0].life = 0;
    assert_eq!(manato_sim::trigger::num(&sim, helper, "ParentDist X"), 0.0);
    sim.fighters[0].life = sim.fighters[0].life_max;

    // A nested helper measures its parent and its root separately.
    let nested = spawn(&mut sim, &assets, helper, 5);
    sim.fighters[nested].pos = manato_core::Vec2::new(50.0, 30.0);
    sim.fighters[nested].facing = 1;
    assert_eq!(manato_sim::trigger::int(&sim, nested, "IsHelper(5)"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, nested, "IsHelper(31)"), 0);
    assert_eq!(
        manato_sim::trigger::num(&sim, nested, "ParentDist X"),
        -20.0
    );
    assert_eq!(manato_sim::trigger::num(&sim, nested, "RootDist X"), -10.0);

    // No relation and a missing component both yield zero.
    assert_eq!(manato_sim::trigger::num(&sim, 0, "ParentDist X"), 0.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "RootDist Y"), 0.0);
    assert_eq!(manato_sim::trigger::num(&sim, helper, "ParentDist"), 0.0);
}

#[test]
fn changeanim2_reads_another_players_animation_bank() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let p2_name = assets.characters[1].name().to_string();
    let change = |sim: &mut SimState, kind: &str, read: Option<&str>| {
        let mut params = vec![("value".to_string(), "0".to_string())];
        if let Some(read) = read {
            params.push(("readplayerid".to_string(), read.to_string()));
        }
        let control = manato_formats::Controller {
            kind: kind.into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    // readplayerid 1 selects the opponent's animation bank.
    change(&mut sim, "changeanim2", Some("1"));
    assert_eq!(sim.fighters[0].anim_char_id, Some(1));
    assert_eq!(sim.anim_character(0).name(), p2_name);
    let frame0 = assets.characters[1]
        .animation(0)
        .and_then(|anim| anim.frames.first())
        .map(|frame| frame.effective_duration())
        .expect("P2 anim 0");
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "AnimElemTime(2)"),
        -frame0,
        "frame timing comes from the opponent's bank"
    );

    // Out-of-range ids clamp like a player slot.
    change(&mut sim, "changeanim2", Some("9"));
    assert_eq!(sim.fighters[0].anim_char_id, Some(1));

    // Without readplayerid the state owner's bank is used.
    change(&mut sim, "changeanim2", None);
    assert_eq!(sim.fighters[0].anim_char_id, Some(0));

    // Plain ChangeAnim restores the own bank.
    change(&mut sim, "changeanim", None);
    assert_eq!(sim.fighters[0].anim_char_id, None);
    assert_eq!(sim.anim_character(0).name(), assets.characters[0].name());
}

#[test]
fn sysvars_persist_across_rounds_and_pauses() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].sysvars[0] = -3;
    sim.fighters[0].sysvars[3] = 42;
    sim.fighters[0].sysfvars[2] = 1.5;
    sim.fighters[0].sysfvars[4] = -0.25;
    sim.fighters[1].sysvars[3] = 7;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "SysVar(3)"), 42);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "SysVar(0)"), -3);
    assert!((manato_sim::trigger::num(&sim, 0, "SysFVar(2)") - 1.5).abs() < 0.001);
    assert!((manato_sim::trigger::num(&sim, 0, "SysFVar(4)") + 0.25).abs() < 0.001);
    // Out-of-range indices clamp to the last slot.
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "SysVar(99)"),
        sim.fighters[0].sysvars[4]
    );

    // Values remain readable during hit pause and super pause.
    sim.fighters[0].hit.hit_pause = 3;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "SysVar(3)"), 42);
    assert!(sim.fighters[0].hit.hit_pause > 0);
    sim.fighters[0].hit.hit_pause = 0;
    sim.super_pause = 3;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "SysVar(3)"), 42);
    sim.super_pause = 0;

    // Round reset carries sysvars for both players.
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Intro {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.number, 2);
    assert_eq!(sim.fighters[0].sysvars[3], 42);
    assert_eq!(sim.fighters[0].sysvars[0], -3);
    assert!((sim.fighters[0].sysfvars[2] - 1.5).abs() < 0.001);
    assert!((sim.fighters[0].sysfvars[4] + 0.25).abs() < 0.001);
    assert_eq!(sim.fighters[1].sysvars[3], 7);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "SysVar(3)"), 42);

    // A match restart also keeps them (only round/match state resets).
    sim.restart_match();
    assert_eq!(sim.fighters[0].sysvars[3], 42);
    assert_eq!(sim.fighters[1].sysvars[3], 7);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "SysVar(3)"), 7);
}

#[test]
fn const_reads_character_constants() {
    let Some(sim) = fight_phase() else {
        return;
    };
    let num = |expr: &str| manato_sim::trigger::num(&sim, 0, expr);
    assert_eq!(num("Const(data.life)"), 1000.0);
    assert_eq!(num("Const(size.ground.front)"), 16.0);
    assert_eq!(num("Const(size.air.back)"), 12.0);
    assert_eq!(num("Const(size.height)"), 60.0);
    assert_eq!(num("Const(size.head.pos.x)"), -5.0);
    assert_eq!(num("Const(size.head.pos.y)"), -72.0);

    // Scalar velocity constants expand to .x / .y.
    assert_eq!(num("Const(velocity.walk.fwd)"), 2.6);
    assert_eq!(num("Const(velocity.walk.fwd.x)"), 2.6);
    assert_eq!(num("Const(velocity.walk.fwd.y)"), 0.0);
    assert_eq!(num("Const(velocity.walk.back)"), -2.6);
    assert_eq!(num("Const(velocity.run.fwd.x)"), 4.4);
    assert_eq!(num("Const(velocity.run.fwd.y)"), 0.0);
    assert_eq!(num("Const(movement.yaccel)"), 0.44);
    assert_eq!(num("Const(data.airjuggle)"), 15.0);

    // Unknown constants evaluate to 0.
    assert_eq!(num("Const(no.such.constant)"), 0.0);
}

#[test]
fn hitdef_parses_priority_points_and_type() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let parse = |sim: &mut SimState, value: &str| {
        let control = manato_formats::Controller {
            kind: "hitdef".into(),
            params: vec![
                ("attr".into(), "S, NA".into()),
                ("damage".into(), "10".into()),
                ("priority".into(), value.into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        let hitdef = sim.fighters[0].hitdef.clone().expect("hitdef");
        (hitdef.priority, hitdef.priority_type)
    };

    use manato_sim::hitdef::PriorityType;
    assert_eq!(parse(&mut sim, "6, Dodge"), (6, PriorityType::Dodge));
    assert_eq!(parse(&mut sim, "3"), (3, PriorityType::Hit));
    assert_eq!(parse(&mut sim, "2, miss"), (2, PriorityType::Miss));
    assert_eq!(parse(&mut sim, "7, HIT"), (7, PriorityType::Hit));
    // Invalid numbers fall back to 0 without losing the type.
    assert_eq!(parse(&mut sim, "abc, Dodge"), (0, PriorityType::Dodge));
}

#[test]
fn projectile_contact_hit_guarded_trigger_timing() {
    // Unguarded: contact and hit timers start at 0 and advance together.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    assert_eq!(sim.fighters[0].proj_contact_time, -1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjContact"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjHit"), 0);

    sim.fighters[0].enter_state(1000, &assets.characters[0], false);
    for _ in 0..200 {
        sim.tick([PlayerInput::default(); 2]);
        if sim.fighters[0].proj_hit_time == 0 {
            break;
        }
    }
    assert_eq!(sim.fighters[0].proj_contact_time, 0);
    assert_eq!(sim.fighters[0].proj_hit_time, 0);
    assert_eq!(sim.fighters[0].proj_guarded_time, -1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjContactTime"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjHitTime"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjHit"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjGuarded"), 0);
    // The single-hit projectile is gone at the end of its contact tick.
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProj"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProjID(0)"), 0);

    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].proj_contact_time, 1);
    assert_eq!(sim.fighters[0].proj_hit_time, 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjHitTime"), 1);

    // Guarded: contact and guarded timers advance; the hit timer stays -1.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(1000, &assets.characters[0], false);
    let back = PlayerInput::new(manato_core::DIR_RIGHT, 0);
    for _ in 0..200 {
        sim.tick([PlayerInput::default(), back]);
        if sim.fighters[0].proj_guarded_time == 0 {
            break;
        }
    }
    assert_eq!(sim.fighters[0].proj_contact_time, 0);
    assert_eq!(sim.fighters[0].proj_guarded_time, 0);
    assert_eq!(sim.fighters[0].proj_hit_time, -1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjGuarded"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjHit"), 0);
    sim.tick([PlayerInput::default(), back]);
    assert_eq!(sim.fighters[0].proj_contact_time, 1);
    assert_eq!(sim.fighters[0].proj_guarded_time, 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "ProjGuardedTime"), 1);
}

#[test]
fn assertspecial_render_flags_are_per_tick() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let state = assets.characters[0].states.get_mut(&0).expect("statedef 0");
        state.controllers.push(manato_formats::Controller {
            kind: "assertspecial".into(),
            params: vec![
                ("type".into(), "AssertSpecial".into()),
                ("nobg".into(), "1".into()),
                ("nofg".into(), "1".into()),
                ("nobardisplay".into(), "1".into()),
                ("invisible".into(), "1".into()),
                ("intro".into(), "1".into()),
            ],
            line: 0,
        });
    }
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.background_hidden());
    assert!(sim.foreground_hidden());
    assert!(sim.bars_hidden());
    let flags = sim.fighters[0].assert_flags;
    assert_ne!(flags & manato_sim::fighter::ASSERT_INVISIBLE, 0);
    assert_ne!(flags & manato_sim::fighter::ASSERT_INTRO, 0);
    assert_ne!(flags & manato_sim::fighter::ASSERT_NOBG, 0);
    assert_ne!(flags & manato_sim::fighter::ASSERT_NOFG, 0);
    assert_ne!(flags & manato_sim::fighter::ASSERT_NOBARDISPLAY, 0);

    // Assertions are not latched: removing the controller clears them.
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let state = assets.characters[0].states.get_mut(&0).expect("statedef 0");
        state.controllers.retain(|c| c.kind != "assertspecial");
    }
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].assert_flags, 0);
    assert!(!sim.background_hidden());
    assert!(!sim.foreground_hidden());
    assert!(!sim.bars_hidden());
}

#[test]
fn target_bind_mirrors_on_turn_and_target_facing() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].target = Some(1);
    sim.fighters[0].pos = manato_core::Vec2::new(0.0, 0.0);
    sim.fighters[0].facing = 1;
    let bind = manato_formats::Controller {
        kind: "targetbind".into(),
        params: vec![
            ("pos".into(), "10, -20".into()),
            ("time".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &bind, &assets);
    // Pushbox separation can move the attacker, so compare relative offsets.
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.fighters[1].pos,
        manato_core::Vec2::new(sim.fighters[0].pos.x + 10.0, -20.0)
    );

    // Turning the attacker mirrors the stored offset on the next tick.
    sim.fighters[0].facing = -1;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.fighters[1].pos,
        manato_core::Vec2::new(sim.fighters[0].pos.x - 10.0, -20.0),
        "bind offset is facing-relative each tick"
    );
    assert_eq!(sim.fighters[1].facing, 1);

    // TargetFacing +1 matches the attacker, -1 opposes it.
    let face = |sim: &mut SimState, value: i32| {
        let control = manato_formats::Controller {
            kind: "targetfacing".into(),
            params: vec![("value".into(), value.to_string())],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    face(&mut sim, 1);
    assert_eq!(sim.fighters[1].facing, -1);
    face(&mut sim, -1);
    assert_eq!(sim.fighters[1].facing, 1);

    // TargetDrop clears the binding completely.
    let drop = manato_formats::Controller {
        kind: "targetdrop".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &drop, &assets);
    assert!(sim.fighters[0].target_bind.is_none());
    assert_eq!(sim.fighters[0].target, None);
}

#[test]
fn hitdef_fall_velocity_populates_get_hit_var_and_hitfallvel() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "10".into()),
            ("fall".into(), "1".into()),
            ("fall.xvelocity".into(), "3".into()),
            ("fall.yvelocity".into(), "6".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    let hit = sim.fighters[0].hitdef.clone().expect("hitdef");
    sim.fighters[1].facing = -1;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.fall_x_vel, Some(-3.0));
    assert_eq!(sim.fighters[1].hit.vars.fall_y_vel, Some(6.0));
    assert_eq!(
        manato_sim::trigger::num(&sim, 1, "GetHitVar(fall.xvelocity)"),
        -3.0
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 1, "GetHitVar(fall.yvelocity)"),
        6.0
    );

    // HitFallVel copies only the fall velocity, not the hit velocity.
    sim.fighters[1].hit.vars.x_vel = 99.0;
    sim.fighters[1].hit.vars.y_vel = 99.0;
    let vel = manato_formats::Controller {
        kind: "hitfallvel".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &vel, &assets);
    assert_eq!(sim.fighters[1].vel, manato_core::Vec2::new(-3.0, 6.0));

    // Unset fall velocities leave the current velocity untouched.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[1].vel = manato_core::Vec2::new(2.0, -2.0);
    sim.fighters[1].hit.vars.fall_x_vel = None;
    sim.fighters[1].hit.vars.fall_y_vel = None;
    manato_sim::controller::execute(&mut sim, 1, &vel, &assets);
    assert_eq!(sim.fighters[1].vel, manato_core::Vec2::new(2.0, -2.0));
}

#[test]
fn airborne_hit_uses_air_velocity_and_air_hittime() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.air_velocity = manato_core::Vec2::new(-4.0, -6.0);
    hit.air_hittime = 5;
    hit.ground_slidetime = 2;
    hit.pausetime = (0, 0);
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos.y = -20.0;
    sim.fighters[1].facing = -1;
    let life = sim.fighters[1].life;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].life, life - hit.damage);
    assert_eq!(sim.fighters[1].hit.vars.x_vel, 4.0, "air velocity mirrored");
    assert_eq!(sim.fighters[1].hit.vars.y_vel, -6.0);
    assert_eq!(sim.fighters[1].hit.vars.hittime, 5);
    assert_eq!(sim.fighters[1].hit.vars.slidetime, 2);
    assert_eq!(sim.fighters[1].hit.vars.ctrltime, 5);
    assert_eq!(sim.fighters[1].state_no, 5020);
}

#[test]
fn hit_reaction_state_selection_and_throw_sprpriority() {
    use manato_sim::fighter::{Physics, StateType};
    use manato_sim::hitdef::GroundType;

    // Crouching defenders enter 5010.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let hit = hitdef();
    sim.fighters[1].state_type = StateType::Crouch;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].state_no, 5010);

    // Trip and non-trip falls select 5070 / 5030 (inject the bundled-missing
    // common states so entry is observable).
    for (ground, expected) in [(GroundType::Trip, 5070), (GroundType::Low, 5030)] {
        let Some(mut sim) = fight_phase() else {
            return;
        };
        {
            let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
            let character = &mut assets.characters[1];
            character.states.insert(
                expected,
                manato_formats::StateDef {
                    number: expected,
                    params: vec![
                        ("type".into(), "A".into()),
                        ("physics".into(), "N".into()),
                        ("anim".into(), "0".into()),
                    ],
                    controllers: Vec::new(),
                    line: 0,
                    source: String::new(),
                },
            );
        }
        let mut fall = hitdef();
        fall.fall = true;
        fall.ground_type = ground;
        fall.p2stateno = -1;
        let origin = sim.fighters[0].pos;
        sim.fighters[1].state_type = StateType::Stand;
        sim.fighters[1].pos.y = 0.0;
        sim.fighters[1].physics = Physics::Stand;
        manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &fall, false, origin, 1, true);
        assert_eq!(sim.fighters[1].state_no, expected, "ground type {ground:?}");
    }

    // Throw HitDefs apply p1/p2 sprite priorities.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut throw = hitdef();
    throw.p1stateno = 800;
    throw.p2stateno = 820;
    throw.p1sprpriority = 5;
    throw.p1sprpriority_set = true;
    throw.p2sprpriority = -2;
    throw.p1facing = -1;
    throw.p2facing = 1;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &throw, false, origin, 1, true);
    assert_eq!(sim.fighters[0].sprite_priority, 5);
    assert_eq!(sim.fighters[1].sprite_priority, -2);
    assert_eq!(sim.fighters[0].facing, -1);
    assert_eq!(sim.fighters[1].facing, 1);
    assert_eq!(sim.fighters[0].state_no, 800);
    assert_eq!(sim.fighters[1].state_no, 820);
}

#[test]
fn projectile_air_juggle_consumes_and_gates_hits() {
    let setup = |points: i32, no_juggle_check: bool| -> Option<SimState> {
        let mut sim = fight_phase()?;
        let assets = sim.assets.clone();
        sim.fighters[0].pos.x = -5.0;
        sim.fighters[0].facing = 1;
        sim.fighters[1].pos = manato_core::Vec2::new(5.0, -20.0);
        sim.fighters[1].facing = -1;
        sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
        sim.fighters[1].set_anim(0, &assets.characters[1], None);
        sim.fighters[1].juggle_points = points;
        if no_juggle_check {
            sim.fighters[1].assert_flags |= manato_sim::fighter::ASSERT_NOJUGGLECHECK;
        }
        let control = manato_formats::Controller {
            kind: "projectile".into(),
            params: vec![
                ("projanim".into(), "1500".into()),
                ("velocity".into(), "0, 0".into()),
                ("removetime".into(), "-1".into()),
                ("damage".into(), "10".into()),
                ("hits".into(), "1".into()),
                ("air.juggle".into(), "5".into()),
                ("postype".into(), "p1".into()),
                ("pos".into(), "0, 0".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(&mut sim, 0, &control, &assets);
        Some(sim)
    };

    // First hit consumes 5 of 9 juggle points.
    let mut sim = setup(9, false).expect("content");
    let life = sim.fighters[1].life;
    manato_sim::collision::resolve_projectiles(&mut sim);
    assert_eq!(sim.fighters[1].life, life - 10);
    assert_eq!(sim.fighters[1].juggle_points, 4);

    // A repeat hit with only 4 points left misses.
    let assets = sim.assets.clone();
    sim.fighters[1].set_anim(0, &assets.characters[1], None);
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].pos.y = -20.0;
    let control = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "0, 0".into()),
            ("removetime".into(), "-1".into()),
            ("damage".into(), "10".into()),
            ("hits".into(), "1".into()),
            ("air.juggle".into(), "5".into()),
            ("postype".into(), "p1".into()),
            ("pos".into(), "0, 0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    manato_sim::collision::resolve_projectiles(&mut sim);
    assert_eq!(
        sim.fighters[1].life,
        life - 10,
        "insufficient juggle points"
    );
    assert_eq!(sim.fighters[1].juggle_points, 4);

    // NoJuggleCheck bypasses the gate and the consumption.
    let mut sim = setup(0, true).expect("content");
    let life = sim.fighters[1].life;
    manato_sim::collision::resolve_projectiles(&mut sim);
    assert_eq!(sim.fighters[1].life, life - 10);
    assert_eq!(sim.fighters[1].juggle_points, 0);
}

#[test]
fn nojugglecheck_skips_melee_juggle_consumption() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -5.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos = manato_core::Vec2::new(5.0, -20.0);
    sim.fighters[1].facing = -1;
    sim.fighters[1].state_type = manato_sim::fighter::StateType::Air;
    sim.fighters[1].set_anim(0, &assets.characters[1], None);
    sim.fighters[1].juggle_points = 0;
    sim.fighters[1].assert_flags |= manato_sim::fighter::ASSERT_NOJUGGLECHECK;
    sim.fighters[0].juggle = 5;
    sim.fighters[0].hit.hit_targets.push(1);
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    let mut hit = hitdef();
    hit.air_juggle = 5;
    sim.fighters[0].hitdef = Some(hit);
    let life = sim.fighters[1].life;
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, life - 20, "NoJuggleCheck lets it hit");
    assert_eq!(sim.fighters[1].juggle_points, 0, "and skips consumption");
}

#[test]
fn envcolor_zero_and_negative_time_edges() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let apply = |sim: &mut SimState, time: &str| {
        let control = manato_formats::Controller {
            kind: "envcolor".into(),
            params: vec![
                ("value".into(), "255, 128, 0".into()),
                ("time".into(), time.into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    // time = 0 disables the tint entirely.
    apply(&mut sim, "0");
    assert!(sim.env_color.is_none());
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.env_color.is_none());

    // Negative time persists until replaced.
    apply(&mut sim, "-1");
    let color = sim.env_color.expect("permanent tint");
    assert_eq!(color.time, -1);
    assert_eq!(color.color[0], 1.0);
    for _ in 0..10 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.env_color.expect("still active").time, -1);

    // Replacing with a positive time expires normally.
    apply(&mut sim, "2");
    for _ in 0..2 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.env_color.is_none());
}

#[test]
fn explod_and_gamemakeanim_p2_anchor_and_binding() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos = manato_core::Vec2::new(60.0, -10.0);
    sim.fighters[1].facing = -1;
    let spawn = |sim: &mut SimState, kind: &str, bindtime: &str| {
        let control = manato_formats::Controller {
            kind: kind.into(),
            params: vec![
                ("anim".into(), "10".into()),
                ("id".into(), "41".into()),
                ("pos".into(), "5, -3".into()),
                ("postype".into(), "p2".into()),
                ("bindtime".into(), bindtime.into()),
                ("removeonchangestate".into(), "0".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.effects.last().expect("effect").clone()
    };

    // Unbound p2 explod lands at the opponent plus the offset.
    let free = spawn(&mut sim, "explod", "0");
    assert_eq!(free.pos, manato_core::Vec2::new(65.0, -13.0));
    assert!(!free.bound);

    // bindtime > 0 follows the opponent.
    let bound = spawn(&mut sim, "explod", "3");
    assert!(bound.bound);
    assert_eq!(bound.bind_target, Some(1));
    assert_eq!(bound.pos, manato_core::Vec2::new(65.0, -13.0));
    sim.fighters[1].pos.x = 100.0;
    sim.tick([PlayerInput::default(); 2]);
    let effect = sim
        .effects
        .iter()
        .find(|effect| effect.id == 41 && effect.bound)
        .expect("bound p2 explod");
    assert_eq!(effect.pos.x, 105.0);

    // GameMakeAnim honors the same p2 anchor without binding.
    let anim = spawn(&mut sim, "gamemakeanim", "0");
    assert_eq!(anim.pos, manato_core::Vec2::new(105.0, -13.0));
    assert!(!anim.bound);
}

#[test]
fn hitfallset_negative_value_leaves_flag_unchanged() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let set = |sim: &mut SimState, params: Vec<(&str, &str)>| {
        let control = manato_formats::Controller {
            kind: "hitfallset".into(),
            params: params
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            line: 0,
        };
        manato_sim::controller::execute(sim, 1, &control, &assets);
    };

    sim.fighters[1].hit.vars.fall = false;
    set(&mut sim, vec![("value", "-1")]);
    assert!(!sim.fighters[1].hit.vars.fall, "-1 leaves the flag alone");

    sim.fighters[1].hit.vars.fall = true;
    set(&mut sim, vec![("value", "-1")]);
    assert!(sim.fighters[1].hit.vars.fall, "still unchanged");

    // A non-negative value still sets/clears, and velocities apply even
    // without a value parameter.
    set(&mut sim, vec![("value", "0")]);
    assert!(!sim.fighters[1].hit.vars.fall);
    set(&mut sim, vec![("value", "1")]);
    assert!(sim.fighters[1].hit.vars.fall);
    sim.fighters[1].facing = -1;
    set(&mut sim, vec![("xvel", "4"), ("yvel", "-2")]);
    assert!(sim.fighters[1].hit.vars.fall);
    assert_eq!(sim.fighters[1].hit.vars.fall_x_vel, Some(-4.0));
    assert_eq!(sim.fighters[1].hit.vars.fall_y_vel, Some(-2.0));
}

#[test]
fn lethal_fall_damage_ends_the_round() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].state_no = 5030;
    sim.fighters[1].state_time = 1;
    sim.fighters[1].physics = manato_sim::fighter::Physics::None;
    sim.fighters[1].pos = manato_core::Vec2::new(20.0, 24.0);
    sim.fighters[1].vel = manato_core::Vec2::new(0.0, 10.0);
    sim.fighters[1].hit.vars.y_accel = 0.0;
    sim.fighters[1].hit.vars.fall = true;
    sim.fighters[1].hit.vars.fall_damage = 50;
    sim.fighters[1].life = 20;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].life, 0, "life clamps at zero");
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    assert_eq!(sim.round.winner, Some(0));
    assert_eq!(sim.round.wins, [1, 0]);
}

#[test]
fn camera_zoom_bounds_tension_and_helper_ignoring() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let cam = &mut sim.stage_camera;
        cam.zoom_out = 0.5;
        cam.zoom_in = 2.0;
        cam.zoom_in_delay = 0;
        cam.zoom_in_speed = 1.0;
        cam.zoom_out_speed = 1.0;
        cam.tension = 50.0;
        cam.bound_left = -100.0;
        cam.bound_right = 1000.0;
        cam.bound_high = -200.0;
        cam.bound_low = 100.0;
    }
    for fighter in &mut sim.fighters {
        fighter.player_push = false;
        fighter.ctrl = false;
        // Synthetic heights exceed the stage extent; bypass ScreenBound Y.
        fighter.screen_bound = (false, false);
    }
    sim.fighters[0].pos = manato_core::Vec2::new(0.0, 0.0);
    sim.fighters[1].pos = manato_core::Vec2::new(0.0, 0.0);
    sim.tick([PlayerInput::default(); 2]);
    // Close fighters: the wide tension sets the target to the max zoom.
    assert!((sim.camera.target_scale - 2.0).abs() < 0.001);
    assert!(sim.camera.scale > 1.0);

    // Far fighters: target clamps to the minimum zoom and follows down.
    sim.fighters[0].pos.x = -300.0;
    sim.fighters[1].pos.x = 300.0;
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.camera.target_scale - 0.5).abs() < 0.001);
    for _ in 0..20 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!((sim.camera.scale - 0.5).abs() < 0.01);
    // Half-width at 0.5 zoom is 320, so the camera clamps to x=220.
    for _ in 0..40 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        (sim.camera.x - 220.0).abs() < 5.0,
        "camera x clamps inside the bounds, got {}",
        sim.camera.x
    );

    // A helper far away does not widen the framing.
    let assets = sim.assets.clone();
    let before = sim.camera.target_scale;
    let helper = manato_formats::Controller {
        kind: "helper".into(),
        params: vec![
            ("stateno".into(), "0".into()),
            ("id".into(), "50".into()),
            ("removetime".into(), "-1".into()),
            ("pos".into(), "4000, 0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &helper, &assets);
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.camera.target_scale - before).abs() < 0.001);

    // Vertical target clamps to boundhigh; the camera follows toward it.
    sim.fighters[0].pos.y = -300.0;
    for _ in 0..40 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        sim.camera.y < -50.0,
        "camera follows the clamped height, got {}",
        sim.camera.y
    );
}

#[test]
fn camera_vertical_follow_and_floor_tension() {
    // verticalfollow = 1 clamps the target to boundhigh.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let cam = &mut sim.stage_camera;
        cam.vertical_follow = 1.0;
        cam.floor_tension = 0.0;
        cam.bound_high = -76.0;
        cam.bound_low = 0.0;
        cam.zoom_out = 1.0;
        cam.zoom_in = 1.0;
    }
    sim.fighters[0].pos.y = -300.0;
    sim.fighters[1].pos.y = 0.0;
    for _ in 0..40 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        (sim.camera.y + 76.0).abs() < 1.0,
        "camera clamps to boundhigh, got {}",
        sim.camera.y
    );

    // verticalfollow = 0.2 tracks only a fifth of the height.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let cam = &mut sim.stage_camera;
        cam.vertical_follow = 0.2;
        cam.floor_tension = 0.0;
        cam.bound_high = -500.0;
        cam.bound_low = 500.0;
    }
    sim.fighters[0].pos.y = -300.0;
    sim.fighters[1].pos.y = 0.0;
    for _ in 0..40 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        (sim.camera.y + 60.0).abs() < 1.0,
        "0.2 * -300 should target -60, got {}",
        sim.camera.y
    );

    // verticalfollow = 0 pins the camera to 0 regardless of height.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let cam = &mut sim.stage_camera;
        cam.vertical_follow = 0.0;
        cam.floor_tension = 0.0;
        cam.bound_high = -500.0;
        cam.bound_low = 500.0;
    }
    sim.fighters[0].pos.y = -300.0;
    for _ in 0..40 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.camera.y.abs() < 1.0, "follow 0 keeps y at 0");

    // floor tension raises the target: (0 + 100) * 0.5 = 50.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let cam = &mut sim.stage_camera;
        cam.vertical_follow = 0.5;
        cam.floor_tension = 100.0;
        cam.bound_high = -500.0;
        cam.bound_low = 500.0;
    }
    for _ in 0..40 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        (sim.camera.y - 50.0).abs() < 1.0,
        "floor tension target, got {}",
        sim.camera.y
    );
}

#[test]
fn helper_keyctrl_and_helpertype_gate_special_states() {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = project.canonicalize().unwrap_or(project);
    let source = project.join("content/chars/kaen");
    if !source.join("kaen.cns").exists() {
        eprintln!("skipping: bundled character missing");
        return;
    }
    let dir = std::env::temp_dir().join(format!("manato-keyctrl-{}", std::process::id()));
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
    let states = dir.join("kaen.cns");
    let text = std::fs::read_to_string(&states).expect("read kaen.cns");
    let text = text.replacen(
        "[Statedef -1]",
        "[Statedef -1]\n[State -1, Keyctrl Tick]\ntype = VarAdd\ntrigger1 = 1\nv = 5\nvalue = 1",
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
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, keyctrl: i32, player: bool| {
        let mut params = vec![
            ("stateno".to_string(), "0".to_string()),
            ("id".to_string(), "60".to_string()),
            ("removetime".to_string(), "-1".to_string()),
            ("keyctrl".to_string(), keyctrl.to_string()),
        ];
        if player {
            params.push(("helpertype".to_string(), "player".to_string()));
        }
        let control = manato_formats::Controller {
            kind: "helper".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters.len() - 1
    };

    let none = spawn(&mut sim, 0, false);
    let state1 = spawn(&mut sim, 1, false);
    let state2 = spawn(&mut sim, 2, false);
    let state3 = spawn(&mut sim, 4, false);
    let player = spawn(&mut sim, 0, true);
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }

    let vars = |sim: &SimState, who: usize| {
        (
            sim.fighters[who].vars[3],
            sim.fighters[who].vars[4],
            sim.fighters[who].vars[5],
        )
    };
    assert_eq!(vars(&sim, none), (0, 0, 0), "keyctrl 0 runs nothing");
    assert_eq!(
        vars(&sim, state1),
        (0, 0, 3),
        "keyctrl 1 runs state -1 only"
    );
    assert_eq!(
        vars(&sim, state2),
        (0, 3, 0),
        "keyctrl 2 runs state -2 only"
    );
    assert_eq!(
        vars(&sim, state3),
        (3, 0, 0),
        "keyctrl 4 runs state -3 only"
    );
    assert_eq!(
        vars(&sim, player),
        (3, 3, 0),
        "helpertype=player runs -3/-2 but not -1 without keyctrl"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn helper_velocity_is_facing_relative() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, facing: i32, velocity: Option<&str>| {
        sim.fighters[0].facing = facing;
        let mut params = vec![
            ("stateno".to_string(), "0".to_string()),
            ("id".to_string(), "61".to_string()),
            ("removetime".to_string(), "-1".to_string()),
        ];
        if let Some(velocity) = velocity {
            params.push(("velocity".to_string(), velocity.to_string()));
        }
        let control = manato_formats::Controller {
            kind: "helper".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters.last().expect("helper").vel
    };

    assert_eq!(
        spawn(&mut sim, 1, Some("3, -2")),
        manato_core::Vec2::new(3.0, -2.0)
    );
    assert_eq!(
        spawn(&mut sim, -1, Some("3, -2")),
        manato_core::Vec2::new(-3.0, -2.0),
        "velocity X mirrors with the owner facing"
    );
    assert_eq!(
        spawn(&mut sim, 1, None),
        manato_core::Vec2::ZERO,
        "default helper velocity is zero"
    );
}

#[test]
fn helper_size_scale_parameters() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, scale: Option<(&str, &str)>| {
        let mut params = vec![
            ("stateno".to_string(), "0".to_string()),
            ("id".to_string(), "62".to_string()),
            ("removetime".to_string(), "-1".to_string()),
        ];
        if let Some((x, y)) = scale {
            params.push(("size.xscale".to_string(), x.to_string()));
            params.push(("size.yscale".to_string(), y.to_string()));
        }
        let control = manato_formats::Controller {
            kind: "helper".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        let helper = sim.fighters.last().expect("helper");
        (helper.size_xscale, helper.size_yscale)
    };

    assert_eq!(spawn(&mut sim, Some(("2", "1.5"))), (2.0, 1.5));
    assert_eq!(spawn(&mut sim, None), (1.0, 1.0), "default scale is 1");
    // Non-positive values clamp to a tiny positive scale.
    let (x, y) = spawn(&mut sim, Some(("0", "-3")));
    assert!((x - 0.01).abs() < 0.001);
    assert!((y - 0.01).abs() < 0.001);
}

#[test]
fn explod_animelem_offsets_lifetime_and_freeze() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    // Animation 10 lasts 3+4+4 = 11 ticks; elem 3 starts at tick 7.
    let spawn = |sim: &mut SimState, elem: &str, freeze: bool, removetime: Option<&str>| {
        let mut params = vec![
            ("anim".to_string(), "10".to_string()),
            ("id".to_string(), "70".to_string()),
            ("animelem".to_string(), elem.to_string()),
            ("removeonchangestate".to_string(), "0".to_string()),
        ];
        if freeze {
            params.push(("animfreeze".to_string(), "1".to_string()));
        }
        if let Some(removetime) = removetime {
            params.push(("removetime".to_string(), removetime.to_string()));
        }
        let control = manato_formats::Controller {
            kind: "explod".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.effects.last().expect("explod").clone()
    };

    let effect = spawn(&mut sim, "3", false, None);
    assert_eq!(effect.anim_elem, 3);
    assert_eq!(effect.duration, 4, "remaining animation after elem 3");
    assert!(effect.remove_on_anim_end);
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        !sim.effects.is_empty(),
        "still alive before the shifted end"
    );
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.effects.is_empty(), "expires after the remaining frames");

    let effect = spawn(&mut sim, "2", true, Some("5"));
    assert!(effect.anim_freeze);
    assert!(!effect.remove_on_anim_end);
    assert_eq!(effect.duration, 5);
    for _ in 0..4 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(!sim.effects.is_empty());
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.effects.is_empty(), "frozen explod honors removetime");
}

#[test]
fn modify_explod_bindtime_removeongethit_and_ownpal() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = manato_formats::Controller {
        kind: "explod".into(),
        params: vec![
            ("anim".into(), "10".into()),
            ("id".into(), "70".into()),
            ("removetime".into(), "100".into()),
            ("removeonchangestate".into(), "0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &spawn, &assets);
    let modify = manato_formats::Controller {
        kind: "modifyexplod".into(),
        params: vec![
            ("id".into(), "70".into()),
            ("bindtime".into(), "2".into()),
            ("ownpal".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    {
        let effect = sim.effects.last().expect("explod");
        assert!(effect.bound, "bindtime > 0 reattaches to the owner");
        assert_eq!(effect.bind_target, Some(0));
        assert_eq!(effect.bind_time, 2);
        assert!(effect.own_pal);
    }
    sim.fighters[0].pos.x += 40.0;
    sim.tick([PlayerInput::default(); 2]);
    {
        let effect = sim.effects.last().expect("still bound");
        assert_eq!(effect.bind_time, 1);
        assert_eq!(effect.pos.x, sim.fighters[0].pos.x);
    }
    sim.tick([PlayerInput::default(); 2]);
    assert!(!sim.effects.last().expect("released").bound);

    // removeongethit applies to the owner being hit.
    let hit = manato_formats::Controller {
        kind: "modifyexplod".into(),
        params: vec![
            ("id".into(), "70".into()),
            ("removeongethit".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &hit, &assets);
    let strike = hitdef();
    let origin = sim.fighters[1].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 1, 0, &strike, false, origin, -1, true);
    sim.tick([PlayerInput::default(); 2]);
    assert!(!sim.effects.iter().any(|effect| effect.id == 70));
}

#[test]
fn modify_explod_animelem_and_animfreeze() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, id: i32| {
        let control = manato_formats::Controller {
            kind: "explod".into(),
            params: vec![
                ("anim".into(), "10".into()),
                ("id".into(), id.to_string()),
                ("removeonchangestate".into(), "0".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    let modify = |sim: &mut SimState, id: i32, params: Vec<(&str, &str)>| {
        let control = manato_formats::Controller {
            kind: "modifyexplod".into(),
            params: {
                let mut all = vec![("id".to_string(), id.to_string())];
                all.extend(
                    params
                        .into_iter()
                        .map(|(k, v)| (k.to_string(), v.to_string())),
                );
                all
            },
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    spawn(&mut sim, 71);
    modify(&mut sim, 71, vec![("animelem", "3")]);
    {
        let effect = sim.effects.last().expect("explod");
        assert_eq!(effect.anim_elem, 3);
        assert_eq!(effect.time, 0, "restarts from the new element");
        assert_eq!(effect.duration, 4, "11 - 7 frames");
    }
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(!sim.effects.is_empty());
    sim.tick([PlayerInput::default(); 2]);
    assert!(!sim.effects.iter().any(|effect| effect.id == 71));

    spawn(&mut sim, 72);
    modify(&mut sim, 72, vec![("animelem", "2"), ("animfreeze", "1")]);
    {
        let effect = sim.effects.last().expect("explod");
        assert_eq!(effect.anim_elem, 2);
        assert!(effect.anim_freeze);
        assert!(!effect.remove_on_anim_end);
        assert_eq!(effect.duration, -1);
    }
    for _ in 0..20 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        sim.effects.iter().any(|effect| effect.id == 72),
        "frozen explod persists"
    );
}

#[test]
fn explod_permanent_bind_modify_transitions_and_removeongethit() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].facing = 1;
    let spawn = |sim: &mut SimState, id: i32, removeongethit: bool| {
        let mut params = vec![
            ("anim".to_string(), "10".to_string()),
            ("id".to_string(), id.to_string()),
            ("pos".to_string(), "10, -5".to_string()),
            ("bindtime".to_string(), "-1".to_string()),
            ("removeonchangestate".to_string(), "0".to_string()),
            ("removetime".to_string(), "100".to_string()),
        ];
        if removeongethit {
            params.push(("removeongethit".to_string(), "1".to_string()));
        }
        let control = manato_formats::Controller {
            kind: "explod".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    let modify = |sim: &mut SimState, id: i32, bindtime: i32| {
        let control = manato_formats::Controller {
            kind: "modifyexplod".into(),
            params: vec![
                ("id".into(), id.to_string()),
                ("bindtime".into(), bindtime.to_string()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    // spawn bindtime = -1 is a permanent bind.
    spawn(&mut sim, 80, false);
    {
        let effect = sim.effects.last().expect("explod");
        assert!(effect.bound);
        assert_eq!(effect.bind_time, -1);
        assert_eq!(
            effect.pos,
            manato_core::Vec2::new(sim.fighters[0].pos.x + 10.0, -5.0)
        );
    }
    for _ in 0..10 {
        sim.fighters[0].pos.x += 5.0;
        sim.tick([PlayerInput::default(); 2]);
        let effect = sim.effects.last().expect("bound explod");
        assert_eq!(effect.bind_time, -1, "permanent bind never counts down");
        assert_eq!(effect.pos.x, sim.fighters[0].pos.x + 10.0);
    }

    // Modify bindtime = 0 releases it.
    modify(&mut sim, 80, 0);
    {
        let effect = sim.effects.last().expect("explod");
        assert!(!effect.bound);
        assert_eq!(effect.bind_target, None);
    }
    let frozen_x = sim.effects.last().unwrap().pos.x;
    sim.fighters[0].pos.x += 40.0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.effects.last().unwrap().pos.x,
        frozen_x,
        "released explod does not follow the owner"
    );

    // Modify bindtime = -1 rebinds permanently at the current offset.
    modify(&mut sim, 80, -1);
    {
        let effect = sim.effects.last().expect("explod");
        assert!(effect.bound);
        assert_eq!(effect.bind_time, -1);
    }
    sim.fighters[0].pos.x += 20.0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.effects.last().unwrap().pos.x,
        sim.fighters[0].pos.x - 30.0,
        "rebound offset stays fixed relative to the owner"
    );

    // Spawn removeongethit removes on the owner's next hit.
    spawn(&mut sim, 81, true);
    let strike = hitdef();
    let origin = sim.fighters[1].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 1, 0, &strike, false, origin, -1, true);
    sim.tick([PlayerInput::default(); 2]);
    assert!(!sim.effects.iter().any(|effect| effect.id == 81));
    assert!(sim.effects.iter().any(|effect| effect.id == 80));
}

#[test]
fn p2_life_anim_facing_and_numtarget_after_throw() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[1].life = 700;
    sim.fighters[1].anim_no = 20;
    sim.fighters[1].facing = -1;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "P2Life"), 700);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "P2Anim"), 20);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "P2Facing"), -1);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "P2Life"),
        sim.fighters[0].life
    );
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "P2Anim"),
        sim.fighters[0].anim_no
    );

    // A normal hit does not link a target; a throw does.
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumTarget"), 0);
    let mut throw = hitdef();
    throw.p1stateno = 800;
    throw.p2stateno = 820;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &throw, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumTarget"), 1);
    assert_eq!(sim.fighters[0].target, Some(1));
}

#[test]
fn alive_physics_lifemax_and_power_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Alive"), 1);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "LifeMax"),
        sim.fighters[0].life_max
    );
    sim.fighters[0].power = 1234.0;
    assert!((manato_sim::trigger::num(&sim, 0, "Power") - 1234.0).abs() < 0.001);

    use manato_sim::fighter::Physics;
    for (physics, code) in [
        (Physics::Stand, "S"),
        (Physics::Crouch, "C"),
        (Physics::Air, "A"),
        (Physics::None, "N"),
    ] {
        sim.fighters[0].physics = physics;
        assert_eq!(
            manato_sim::trigger::eval_str(&sim, 0, "Physics").as_str(),
            code
        );
    }

    sim.fighters[0].life = 0;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Alive"), 0);
    sim.fighters[0].life = 1;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Alive"), 1);
}

#[test]
fn modify_explod_removetime_negative_restores_animation_expiry() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, id: i32, removetime: &str| {
        let control = manato_formats::Controller {
            kind: "explod".into(),
            params: vec![
                ("anim".into(), "10".into()),
                ("id".into(), id.to_string()),
                ("removetime".into(), removetime.into()),
                ("removeonchangestate".into(), "0".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    let modify = |sim: &mut SimState, id: i32, removetime: &str| {
        let control = manato_formats::Controller {
            kind: "modifyexplod".into(),
            params: vec![
                ("id".into(), id.to_string()),
                ("removetime".into(), removetime.into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    // Fixed lifetime, then -1 restores animation expiry (11 ticks).
    spawn(&mut sim, 90, "100");
    modify(&mut sim, 90, "-1");
    {
        let effect = sim.effects.last().expect("explod");
        assert!(effect.remove_on_anim_end);
        assert_eq!(effect.duration, 11);
    }
    for _ in 0..10 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.effects.iter().any(|effect| effect.id == 90));
    sim.tick([PlayerInput::default(); 2]);
    assert!(!sim.effects.iter().any(|effect| effect.id == 90));

    // With animelem 3 the restored lifetime is the remaining 4 ticks.
    spawn(&mut sim, 91, "100");
    {
        let control = manato_formats::Controller {
            kind: "modifyexplod".into(),
            params: vec![("id".into(), "91".into()), ("animelem".into(), "3".into())],
            line: 0,
        };
        manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    }
    modify(&mut sim, 91, "-1");
    {
        let effect = sim.effects.last().expect("explod");
        assert_eq!(effect.anim_elem, 3);
        assert_eq!(effect.duration, 4);
    }

    // A frozen explod ignores removetime = -1.
    spawn(&mut sim, 92, "100");
    let freeze = manato_formats::Controller {
        kind: "modifyexplod".into(),
        params: vec![
            ("id".into(), "92".into()),
            ("animfreeze".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &freeze, &assets);
    modify(&mut sim, 92, "-1");
    let effect = sim
        .effects
        .iter()
        .find(|effect| effect.id == 92)
        .expect("frozen explod");
    assert!(effect.anim_freeze);
    assert_eq!(effect.duration, -1);
}

#[test]
fn projectile_spawns_get_unique_hit_activation_ids() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -5.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 5.0;
    sim.fighters[1].facing = -1;
    sim.fighters[1].set_anim(0, &assets.characters[1], None);
    let spawn = |sim: &mut SimState, id: i32| {
        let control = manato_formats::Controller {
            kind: "projectile".into(),
            params: vec![
                ("projanim".into(), "1500".into()),
                ("velocity".into(), "0, 0".into()),
                ("removetime".into(), "30".into()),
                ("projid".into(), id.to_string()),
                ("id".into(), "77".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    spawn(&mut sim, 1);
    spawn(&mut sim, 2);
    assert_eq!(sim.projectiles.len(), 2);
    assert_ne!(
        sim.projectiles[0].hitdef.activation, sim.projectiles[1].hitdef.activation,
        "each projectile spawn is a distinct activation"
    );

    // GetHitVar(hitid) reports the projectile HitDef's id after contact.
    manato_sim::collision::resolve_projectiles(&mut sim);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(hitid)"), 77);
}

#[test]
fn hitonce_limits_resolve_to_one_hit_per_activation() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -1.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 5.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    let mut hit = hitdef();
    hit.hitonce = true;
    hit.numhits = 5;
    hit.damage = 10;
    hit.pausetime = (0, 0);
    sim.fighters[0].hitdef = Some(hit);
    let reset = |sim: &mut SimState| {
        sim.fighters[1].enter_state(0, &assets.characters[1], false);
        sim.fighters[1].set_anim(0, &assets.characters[1], None);
    };

    let life = sim.fighters[1].life;
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, life - 10);
    reset(&mut sim);
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(
        sim.fighters[1].life,
        life - 10,
        "hitonce stops subsequent hits in the activation"
    );

    // A new activation clears the target list and allows another hit.
    sim.fighters[0].hit.hit_targets.clear();
    sim.fighters[0].hit.hit_counts.clear();
    reset(&mut sim);
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(sim.fighters[1].life, life - 20);
}

#[test]
fn stateno_prevstateno_and_time_triggers_after_transition() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "StateNo"), 200);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PrevStateNo"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Time"), 0);

    sim.fighters[0].enter_state(210, &assets.characters[0], false);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "StateNo"), 210);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PrevStateNo"), 200);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Time"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "StateTime"), 0);

    // The entry tick keeps Time at 0; the next tick advances it.
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "StateNo"), 210);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "PrevStateNo"), 200);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Time"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "StateTime"), 0);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "Time"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "StateTime"), 1);
}

#[test]
fn width_accepts_value_player_and_edge_forms() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let apply = |sim: &mut SimState, params: Vec<(&str, &str)>| {
        let control = manato_formats::Controller {
            kind: "width".into(),
            params: params
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.fighters[0].width_override
    };

    assert_eq!(
        apply(&mut sim, vec![("value", "-20, 30")]),
        Some((-20.0, 30.0))
    );
    assert_eq!(
        apply(&mut sim, vec![("player", "-5, 8")]),
        Some((-5.0, 8.0))
    );
    assert_eq!(
        apply(
            &mut sim,
            vec![("edge", "1, 2"), ("player", "3, 4"), ("value", "5, 6")]
        ),
        Some((1.0, 2.0)),
        "edge takes precedence"
    );
    // No width parameter leaves the previous override untouched.
    assert_eq!(apply(&mut sim, vec![]), Some((1.0, 2.0)));
}

#[test]
fn changestate_ctrl_and_anim_overrides() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let switch = |sim: &mut SimState, value: i32, extra: Vec<(&str, &str)>| {
        let mut params = vec![("value".to_string(), value.to_string())];
        params.extend(
            extra
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string())),
        );
        let control = manato_formats::Controller {
            kind: "changestate".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    // State 200 declares ctrl = 0 and anim 200.
    switch(&mut sim, 200, vec![]);
    assert_eq!(sim.fighters[0].state_no, 200);
    assert!(!sim.fighters[0].ctrl);
    assert_eq!(sim.fighters[0].anim_no, 200);

    // Explicit ctrl/anim parameters override the statedef.
    switch(&mut sim, 200, vec![("ctrl", "1"), ("anim", "210")]);
    assert_eq!(sim.fighters[0].state_no, 200);
    assert!(sim.fighters[0].ctrl);
    assert_eq!(sim.fighters[0].anim_no, 210);

    // SelfState honors the same overrides.
    let self_state = manato_formats::Controller {
        kind: "selfstate".into(),
        params: vec![
            ("value".into(), "210".into()),
            ("ctrl".into(), "0".into()),
            ("anim".into(), "220".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &self_state, &assets);
    assert_eq!(sim.fighters[0].state_no, 210);
    assert!(!sim.fighters[0].ctrl);
    assert_eq!(sim.fighters[0].anim_no, 220);
}

#[test]
fn makedust_spacing_gate_and_pos2() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].facing = 1;
    let dust = |sim: &mut SimState, params: Vec<(&str, &str)>| {
        let control = manato_formats::Controller {
            kind: "makedust".into(),
            params: params
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    // Emits after the spacing gate, then resets the counter.
    let before = sim.effects.len();
    dust(
        &mut sim,
        vec![("spacing", "3"), ("pos", "1, 2"), ("pos2", "-3, 4")],
    );
    assert_eq!(sim.effects.len(), before + 2, "pos and pos2 spawn");
    let owner = sim.fighters[0].pos;
    let effects: Vec<_> = sim.effects.iter().rev().take(2).collect();
    assert!((effects[0].pos.x - (owner.x - 3.0)).abs() < 0.001);
    assert!((effects[0].pos.y - (owner.y + 4.0)).abs() < 0.001);
    assert!((effects[1].pos.x - (owner.x + 1.0)).abs() < 0.001);
    assert!((effects[1].pos.y - (owner.y + 2.0)).abs() < 0.001);
    for effect in &effects {
        assert_eq!(effect.sprite_priority, i32::MAX);
        assert!(effect.own_pal);
        assert!(effect.behind);
    }
    assert_eq!(sim.fighters[0].make_dust_spacing, 0);

    // Immediate re-execution is blocked by the gate.
    dust(&mut sim, vec![("spacing", "3")]);
    assert_eq!(sim.effects.len(), before + 2);
    sim.tick([PlayerInput::default(); 2]);
    dust(&mut sim, vec![("spacing", "3")]);
    assert_eq!(sim.effects.len(), before + 2, "counter 1 < spacing 3");

    // After two more ticks the gate opens.
    sim.tick([PlayerInput::default(); 2]);
    sim.tick([PlayerInput::default(); 2]);
    dust(&mut sim, vec![("spacing", "3")]);
    assert_eq!(sim.effects.len(), before + 3);

    // spacing = 0 clamps to 1 tick.
    sim.tick([PlayerInput::default(); 2]);
    dust(&mut sim, vec![("spacing", "0")]);
    assert_eq!(sim.effects.len(), before + 4);
}

#[test]
fn afterimage_negative_time_is_infinite() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let start = |sim: &mut SimState, time: i32| {
        let control = manato_formats::Controller {
            kind: "afterimage".into(),
            params: vec![
                ("time".into(), time.to_string()),
                ("timegap".into(), "1".into()),
                ("length".into(), "3".into()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    start(&mut sim, -1);
    assert_eq!(
        sim.fighters[0].after_image.as_ref().expect("state").time,
        -1
    );
    for _ in 0..10 {
        sim.tick([PlayerInput::default(); 2]);
    }
    let state = sim.fighters[0]
        .after_image
        .as_ref()
        .expect("still emitting");
    assert_eq!(state.time, -1, "infinite timer does not count down");
    assert!(!sim.fighters[0].after_images.is_empty());

    // time = 0 produces no ghosts.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    start(&mut sim, 0);
    for _ in 0..5 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(sim.fighters[0].after_images.is_empty());
}

#[test]
fn get_hit_var_priority_reflects_hitdef() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.priority = 6;
    hit.damage = 10;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].hit.vars.priority, 6);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(priority)"), 6);

    let mut guarded = hitdef();
    guarded.priority = 2;
    guarded.damage = 10;
    guarded.guard_damage = 3;
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &guarded, true, origin, 1, true);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(priority)"), 2);
}

#[test]
fn envcolor_under_flag_and_default() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let apply = |sim: &mut SimState, under: Option<&str>| {
        let mut params = vec![
            ("value".to_string(), "10, 20, 30".to_string()),
            ("time".to_string(), "5".to_string()),
        ];
        if let Some(under) = under {
            params.push(("under".to_string(), under.to_string()));
        }
        let control = manato_formats::Controller {
            kind: "envcolor".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.env_color.expect("env color")
    };

    assert!(apply(&mut sim, Some("1")).under);
    assert!(!apply(&mut sim, Some("0")).under);
    assert!(!apply(&mut sim, None).under, "under defaults to false");
}

#[test]
fn explod_trans_blend_variants() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, trans: &str, alpha: Option<&str>| {
        let mut params = vec![
            ("anim".to_string(), "10".to_string()),
            ("id".to_string(), "95".to_string()),
            ("removeonchangestate".to_string(), "0".to_string()),
            ("trans".to_string(), trans.to_string()),
        ];
        if let Some(alpha) = alpha {
            params.push(("alpha".to_string(), alpha.to_string()));
        }
        let control = manato_formats::Controller {
            kind: "explod".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.effects.last().expect("explod").clone()
    };

    let none = spawn(&mut sim, "none", Some("128"));
    assert_eq!(none.blend, manato_sim::EffectBlend::Normal);
    assert_eq!(none.alpha, 1.0);
    assert!(!none.transparent);
    for kind in ["add", "add1"] {
        let effect = spawn(&mut sim, kind, Some("128"));
        assert_eq!(effect.blend, manato_sim::EffectBlend::Add);
        assert_eq!(effect.alpha, 1.0);
        assert!(!effect.transparent);
    }
    for kind in ["sub", "sub1"] {
        let effect = spawn(&mut sim, kind, Some("128"));
        assert_eq!(effect.blend, manato_sim::EffectBlend::Sub);
        assert_eq!(effect.alpha, 1.0);
        assert!(!effect.transparent);
    }
    let alpha = spawn(&mut sim, "alpha", Some("128, 64"));
    assert_eq!(alpha.blend, manato_sim::EffectBlend::Normal);
    assert!(alpha.transparent);
    assert!((alpha.alpha - 0.25).abs() < 0.001);
    let addalpha = spawn(&mut sim, "addalpha", Some("256, 128"));
    assert_eq!(addalpha.blend, manato_sim::EffectBlend::Add);
    assert!((addalpha.alpha - 0.5).abs() < 0.001);
    let default = spawn(&mut sim, "default", None);
    assert_eq!(default.blend, manato_sim::EffectBlend::Normal);
    assert_eq!(default.alpha, 1.0);
    assert!(!default.transparent);
    // addalpha without alpha defaults to fully transparent dest 0.
    let bare = spawn(&mut sim, "addalpha", None);
    assert!(bare.transparent);
    assert_eq!(bare.alpha, 0.0);
}

#[test]
fn palfx_sinadd_oscillates_and_resets() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let palfx = manato_formats::Controller {
        kind: "palfx".into(),
        params: vec![
            ("add".into(), "10, 20, 30".into()),
            ("sinadd".into(), "5, 0, 0, 4".into()),
            ("time".into(), "8".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &palfx, &assets);
    assert_eq!(sim.fighters[0].pal_add, [10.0, 20.0, 30.0]);
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.fighters[0].pal_add[0] - 15.0).abs() < 0.01, "sin peak");
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        (sim.fighters[0].pal_add[0] - 10.0).abs() < 0.01,
        "zero crossing"
    );
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.fighters[0].pal_add[0] - 5.0).abs() < 0.01, "trough");
    for _ in 0..5 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.fighters[0].pal_add, [0.0, 0.0, 0.0]);
    assert_eq!(sim.fighters[0].pal_sin_period, 0);
    assert_eq!(sim.fighters[0].pal_time, 0);

    // Negative period flips the amplitude sign.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let negative = manato_formats::Controller {
        kind: "palfx".into(),
        params: vec![
            ("add".into(), "10, 0, 0".into()),
            ("sinadd".into(), "5, 0, 0, -4".into()),
            ("time".into(), "4".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &negative, &assets);
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.fighters[0].pal_add[0] - 5.0).abs() < 0.01);
}

#[test]
fn hit_and_guard_spark_selection() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.sparkno = 100;
    hit.guard_sparkno = 101;
    hit.sparkxy = manato_core::Vec2::new(5.0, -10.0);
    let origin = manato_core::Vec2::new(0.0, 0.0);
    let before = sim.effects.len();
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    let spark = sim.effects.last().expect("hit spark");
    assert_eq!(spark.anim, 100);
    assert_eq!(spark.source, manato_sim::EffectSource::FightFx);
    assert_eq!(spark.pos, manato_core::Vec2::new(5.0, -10.0));

    let before_guard = sim.effects.len();
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, true, origin, 1, true);
    let spark = sim.effects.last().expect("guard spark");
    assert_eq!(spark.anim, 101);
    assert!(before_guard > before);

    // Default sparkno uses 0; a guard without guard.sparkno reuses sparkno.
    let mut plain = hitdef();
    plain.sparkno = -1;
    plain.guard_sparkno = -1;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &plain, true, origin, 1, true);
    assert_eq!(
        sim.effects.last().expect("default guard spark").anim,
        40,
        "guards default to spark 40"
    );
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &plain, false, origin, 1, true);
    assert_eq!(sim.effects.last().expect("default hit spark").anim, 0);
}

#[test]
fn modify_explod_trans_variants_and_toggles() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = manato_formats::Controller {
        kind: "explod".into(),
        params: vec![
            ("anim".into(), "10".into()),
            ("id".into(), "96".into()),
            ("removeonchangestate".into(), "0".into()),
            ("trans".into(), "addalpha".into()),
            ("alpha".into(), "256, 128".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &spawn, &assets);
    let modify = |sim: &mut SimState, params: Vec<(&str, &str)>| {
        let mut all = vec![("id".to_string(), "96".to_string())];
        all.extend(
            params
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string())),
        );
        let control = manato_formats::Controller {
            kind: "modifyexplod".into(),
            params: all,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
        sim.effects.last().expect("explod").clone()
    };

    let effect = modify(&mut sim, vec![("trans", "none")]);
    assert_eq!(effect.blend, manato_sim::EffectBlend::Normal);
    assert!(!effect.transparent);
    assert_eq!(effect.alpha, 1.0);

    for (kind, blend) in [
        ("add", manato_sim::EffectBlend::Add),
        ("add1", manato_sim::EffectBlend::Add),
        ("sub", manato_sim::EffectBlend::Sub),
        ("sub1", manato_sim::EffectBlend::Sub),
    ] {
        let effect = modify(&mut sim, vec![("trans", kind)]);
        assert_eq!(effect.blend, blend, "{kind}");
        assert!(!effect.transparent);
        assert_eq!(effect.alpha, 1.0);
    }

    let effect = modify(&mut sim, vec![("trans", "alpha"), ("alpha", "128, 64")]);
    assert_eq!(effect.blend, manato_sim::EffectBlend::Normal);
    assert!(effect.transparent);
    assert!((effect.alpha - 0.25).abs() < 0.001);

    let effect = modify(&mut sim, vec![("trans", "addalpha"), ("alpha", "256, 128")]);
    assert_eq!(effect.blend, manato_sim::EffectBlend::Add);
    assert!((effect.alpha - 0.5).abs() < 0.001);

    // ownpal/shadow toggles.
    let effect = modify(&mut sim, vec![("ownpal", "1"), ("shadow", "1")]);
    assert!(effect.own_pal);
    assert!(effect.shadow);
    let effect = modify(&mut sim, vec![("ownpal", "0"), ("shadow", "0")]);
    assert!(!effect.own_pal);
    assert!(!effect.shadow);
}

#[test]
fn palfx_zero_time_disables_and_negative_persists() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let apply = |sim: &mut SimState, time: i32| {
        let control = manato_formats::Controller {
            kind: "palfx".into(),
            params: vec![
                ("add".into(), "50, 60, 70".into()),
                ("time".into(), time.to_string()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    apply(&mut sim, 5);
    assert_eq!(sim.fighters[0].pal_add, [50.0, 60.0, 70.0]);
    apply(&mut sim, 0);
    assert_eq!(sim.fighters[0].pal_add, [0.0, 0.0, 0.0], "time 0 clears");
    assert_eq!(sim.fighters[0].pal_time, 0);
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.fighters[0].pal_add, [0.0, 0.0, 0.0]);

    apply(&mut sim, -1);
    assert_eq!(sim.fighters[0].pal_add, [50.0, 60.0, 70.0]);
    for _ in 0..10 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(
        sim.fighters[0].pal_add,
        [50.0, 60.0, 70.0],
        "negative persists"
    );
    assert_eq!(sim.fighters[0].pal_time, -1);
}

#[test]
fn camera_round_reset_and_zoom_anchor_mapping() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let cam = &mut assets.stage.camera;
        cam.start = manato_core::Vec2::new(25.0, -10.0);
        cam.start_zoom = 0.8;
        cam.zoom_out = 0.5;
        cam.zoom_in = 2.0;
    }
    sim.camera.x = 999.0;
    sim.camera.y = 999.0;
    sim.camera.scale = 2.0;
    sim.camera.target_scale = 2.0;
    sim.camera.zoom_override = None;
    sim.restart_match();
    assert_eq!(sim.camera.x, 25.0, "round reset restores startx");
    assert_eq!(sim.camera.y, -10.0, "round reset restores starty");
    assert_eq!(sim.camera.scale, 0.8, "round reset restores start zoom");
    assert_eq!(sim.camera.target_scale, 0.8);

    // Zoom anchor off: the centre world point stays at the view centre.
    sim.stage_camera.zoom_anchor = false;
    sim.camera.x = 0.0;
    sim.camera.y = -30.0;
    sim.camera.scale = 2.0;
    sim.camera.shake = manato_core::Vec2::ZERO;
    let ground = sim.ground_screen_y();
    let center_y = manato_core::GAME_HEIGHT * 0.5;
    let center_world = manato_core::Vec2::new(0.0, sim.camera.y + center_y - ground);
    let mapped = sim.screen_pos(center_world);
    assert!(
        (mapped.y - center_y).abs() < 0.001,
        "centre anchor stays at view centre, got {}",
        mapped.y
    );
    let ground_at_scale_two = sim.screen_pos(manato_core::Vec2::new(0.0, 0.0)).y;
    let expected_ground = center_y + (0.0 - sim.camera.y + ground - center_y) * 2.0;
    assert!(
        (ground_at_scale_two - expected_ground).abs() < 0.01,
        "centre anchor scales away from the ground, got {ground_at_scale_two}"
    );

    // Zoom anchor on: the ground world line stays on the ground line.
    sim.stage_camera.zoom_anchor = true;
    let mapped = sim.screen_pos(manato_core::Vec2::new(0.0, 0.0));
    let expected = ground + (0.0 - sim.camera.y) * 2.0;
    assert!(
        (mapped.y - expected).abs() < 0.01,
        "ground anchor maps world y=0 relative to camera, got {}",
        mapped.y
    );
    sim.camera.y = 0.0;
    assert!((sim.screen_pos(manato_core::Vec2::new(0.0, 0.0)).y - ground).abs() < 0.001);
}

#[test]
fn pause_movetime_lets_only_the_pauser_act() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    for fighter in &mut sim.fighters {
        fighter.physics = manato_sim::fighter::Physics::None;
        fighter.ctrl = false;
        fighter.vel = manato_core::Vec2::new(2.0, 0.0);
    }
    let pause = manato_formats::Controller {
        kind: "pause".into(),
        params: vec![("time".into(), "5".into()), ("movetime".into(), "3".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &pause, &assets);
    assert_eq!(sim.super_pause, 5);
    assert_eq!(sim.pause_mover, Some(0));
    assert_eq!(sim.pause_movetime, 3);

    let start0 = sim.fighters[0].pos.x;
    let start1 = sim.fighters[1].pos.x;
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!(
        (sim.fighters[0].pos.x - (start0 + 6.0)).abs() < 0.001,
        "pauser moves during movetime"
    );
    assert_eq!(sim.fighters[1].pos.x, start1, "opponent stays frozen");
    assert_eq!(sim.pause_movetime, 0);

    // Remaining pause ticks freeze both.
    for _ in 0..2 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.super_pause, 0);
    assert!((sim.fighters[0].pos.x - (start0 + 6.0)).abs() < 0.001);
    assert_eq!(sim.fighters[1].pos.x, start1);

    // Gameplay resumes after the pause.
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.fighters[0].pos.x > start0 + 6.0);
    assert!(sim.fighters[1].pos.x > start1);

    // movetime greater than time is ignored.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    for fighter in &mut sim.fighters {
        fighter.physics = manato_sim::fighter::Physics::None;
        fighter.vel = manato_core::Vec2::new(2.0, 0.0);
    }
    let over = manato_formats::Controller {
        kind: "pause".into(),
        params: vec![("time".into(), "2".into()), ("movetime".into(), "9".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &over, &assets);
    assert_eq!(sim.pause_movetime, 0);
    let start = sim.fighters[0].pos.x;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.fighters[0].pos.x, start,
        "no movement when movetime > time"
    );
}

#[test]
fn superpause_movetime_lets_only_the_pauser_act() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    for fighter in &mut sim.fighters {
        fighter.physics = manato_sim::fighter::Physics::None;
        fighter.ctrl = false;
        fighter.vel = manato_core::Vec2::new(2.0, 0.0);
    }
    let pause = manato_formats::Controller {
        kind: "superpause".into(),
        params: vec![("time".into(), "5".into()), ("movetime".into(), "3".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &pause, &assets);
    assert_eq!(sim.super_pause, 5);
    assert_eq!(sim.pause_movetime, 3);
    assert_eq!(sim.pause_mover, Some(0));

    let start0 = sim.fighters[0].pos.x;
    let start1 = sim.fighters[1].pos.x;
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert!((sim.fighters[0].pos.x - (start0 + 6.0)).abs() < 0.001);
    assert_eq!(sim.fighters[1].pos.x, start1);
    for _ in 0..2 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.super_pause, 0);
    assert!((sim.fighters[0].pos.x - (start0 + 6.0)).abs() < 0.001);
    assert_eq!(sim.fighters[1].pos.x, start1);

    // movetime > time is ignored.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    for fighter in &mut sim.fighters {
        fighter.physics = manato_sim::fighter::Physics::None;
        fighter.vel = manato_core::Vec2::new(2.0, 0.0);
    }
    let over = manato_formats::Controller {
        kind: "superpause".into(),
        params: vec![("time".into(), "2".into()), ("movetime".into(), "9".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &over, &assets);
    assert_eq!(sim.pause_movetime, 0);
    let start = sim.fighters[0].pos.x;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].pos.x, start);
}

#[test]
fn nohardcodedkeys_disables_builtin_movement() {
    // Control: holding forward enters the walk state.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].enter_state(0, &sim.assets.clone().characters[0], false);
    let start = sim.fighters[0].pos.x;
    sim.tick([
        PlayerInput::new(manato_core::DIR_RIGHT, 0),
        PlayerInput::default(),
    ]);
    assert_eq!(sim.fighters[0].state_no, 20, "hardcoded walk state");
    assert!(sim.fighters[0].pos.x != start);

    // With NoHardcodedKeys asserted every tick, no automatic walk occurs.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    {
        let assets = std::sync::Arc::get_mut(&mut sim.assets).expect("unique assets");
        let state = assets.characters[0]
            .states
            .get_mut(&-1)
            .expect("statedef -1");
        state.controllers.push(manato_formats::Controller {
            kind: "assertspecial".into(),
            params: vec![
                ("type".into(), "AssertSpecial".into()),
                ("nohardcodedkeys".into(), "1".into()),
            ],
            line: 0,
        });
    }
    let assets = sim.assets.clone();
    sim.fighters[0].enter_state(0, &assets.characters[0], false);
    let start = sim.fighters[0].pos.x;
    sim.tick([
        PlayerInput::new(manato_core::DIR_RIGHT, 0),
        PlayerInput::default(),
    ]);
    assert_eq!(sim.fighters[0].state_no, 0, "no built-in walk transition");
    assert_eq!(sim.fighters[0].pos.x, start, "no built-in movement");
    assert_ne!(
        sim.fighters[0].assert_flags & manato_sim::fighter::ASSERT_NOHARDCODEDKEYS,
        0
    );
}

#[test]
fn p2sprpriority_applies_to_hits_and_projectiles() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.p2sprpriority = 4;
    hit.p2sprpriority_set = true;
    sim.fighters[1].sprite_priority = 0;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(sim.fighters[1].sprite_priority, 4);
    assert_eq!(
        sim.fighters[0].sprite_priority, 1,
        "p1 defaults to the documented priority of 1"
    );

    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut own = hitdef();
    own.p1sprpriority = 3;
    own.p1sprpriority_set = true;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &own, false, origin, 1, true);
    assert_eq!(sim.fighters[0].sprite_priority, 3);

    // Projectiles apply the defender's priority.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -5.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 5.0;
    sim.fighters[1].facing = -1;
    sim.fighters[1].set_anim(0, &assets.characters[1], None);
    sim.fighters[1].sprite_priority = 0;
    let projectile = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "0, 0".into()),
            ("removetime".into(), "-1".into()),
            ("damage".into(), "10".into()),
            ("p2sprpriority".into(), "5".into()),
            ("postype".into(), "p1".into()),
            ("pos".into(), "0, 0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &projectile, &assets);
    manato_sim::collision::resolve_projectiles(&mut sim);
    assert_eq!(sim.fighters[1].sprite_priority, 5);

    // Parsing records which priorities were supplied.
    let parsed = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("p1sprpriority".into(), "2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &parsed, &assets);
    let hitdef = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert!(hitdef.p1sprpriority_set);
    assert!(!hitdef.p2sprpriority_set);

    // Omitting the parameter keeps the documented default of 1.
    let default = manato_formats::Controller {
        kind: "hitdef".into(),
        params: vec![("attr".into(), "S, NA".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &default, &assets);
    let hitdef = sim.fighters[0].hitdef.clone().expect("hitdef");
    assert_eq!(hitdef.p1sprpriority, 1);
    assert!(!hitdef.p1sprpriority_set);
    assert_eq!(hitdef.p2sprpriority, 0);
}

#[test]
fn palfx_sinadd_with_mul_and_round_reset() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let palfx = manato_formats::Controller {
        kind: "palfx".into(),
        params: vec![
            ("add".into(), "10, 20, 30".into()),
            ("mul".into(), "128, 64, 32".into()),
            ("sinadd".into(), "5, 0, 0, 4".into()),
            ("time".into(), "10000".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &palfx, &assets);
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.fighters[0].pal_add[0] - 15.0).abs() < 0.01);
    assert_eq!(
        sim.fighters[0].pal_mul,
        [0.5, 0.25, 0.125],
        "mul survives sinadd"
    );
    assert_eq!(sim.fighters[0].pal_sin_period, 4);

    // Round reset restores identity palette state.
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Intro {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.number, 2);
    assert_eq!(sim.fighters[0].pal_add, [0.0, 0.0, 0.0]);
    assert_eq!(sim.fighters[0].pal_base_add, [0.0, 0.0, 0.0]);
    assert_eq!(sim.fighters[0].pal_sinadd, [0.0, 0.0, 0.0]);
    assert_eq!(sim.fighters[0].pal_sin_period, 0);
    assert_eq!(sim.fighters[0].pal_cycle, 0);
    assert_eq!(sim.fighters[0].pal_mul, [1.0, 1.0, 1.0]);
    assert_eq!(sim.fighters[0].pal_alpha, 1.0);
    assert_eq!(sim.fighters[0].pal_time, 0);

    // restart_match also clears an active PalFX.
    sim.fighters[0].pal_add = [9.0, 9.0, 9.0];
    sim.fighters[0].pal_time = 500;
    sim.restart_match();
    assert_eq!(sim.fighters[0].pal_add, [0.0, 0.0, 0.0]);
    assert_eq!(sim.fighters[0].pal_time, 0);
}

#[test]
fn envshake_mul_decay_and_globalnoshadow() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let shake = manato_formats::Controller {
        kind: "envshake".into(),
        params: vec![
            ("time".into(), "5".into()),
            ("ampl".into(), "-8".into()),
            ("freq".into(), "90".into()),
            ("mul".into(), "0.5".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &shake, &assets);
    assert_eq!(sim.env_shake.initial_time, 5);
    assert_eq!(sim.env_shake.mul, 0.5);
    assert_eq!(sim.env_shake.phase, 90.0, "freq >= 90 defaults phase 90");
    // One mul step per full freq cycle (90 deg/tick => cycle every 4 ticks).
    let mut xs = Vec::new();
    for _ in 0..5 {
        sim.tick([PlayerInput::default(); 2]);
        xs.push(sim.camera.shake.y);
    }
    assert!((xs[0] - -8.0).abs() < 0.01, "first frame: {xs:?}");
    assert!(xs[1].abs() < 0.01, "zero crossing: {xs:?}");
    assert!((xs[2] - 8.0).abs() < 0.01, "opposite peak: {xs:?}");
    assert!(xs[3].abs() < 0.01, "second crossing: {xs:?}");
    assert!((xs[4] - -4.0).abs() < 0.01, "one cycle applies mul: {xs:?}");

    // decay scales amplitude by (remaining / initial)^decay.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let decay = manato_formats::Controller {
        kind: "envshake".into(),
        params: vec![
            ("time".into(), "4".into()),
            ("ampl".into(), "-8".into()),
            ("freq".into(), "0".into()),
            ("phase".into(), "90".into()),
            ("decay".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &decay, &assets);
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.camera.shake.y - -8.0).abs() < 0.01);
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.camera.shake.y - -6.0).abs() < 0.01);
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.camera.shake.y - -4.0).abs() < 0.01);

    // dir rotates the shake vector; phase is clamped to [-180, 180].
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let directed = manato_formats::Controller {
        kind: "envshake".into(),
        params: vec![
            ("time".into(), "3".into()),
            ("ampl".into(), "-8".into()),
            ("freq".into(), "0".into()),
            ("phase".into(), "999".into()),
            ("dir".into(), "90".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &directed, &assets);
    assert_eq!(sim.env_shake.phase, 180.0, "phase clamps to 180");
    sim.tick([PlayerInput::default(); 2]);
    // phase 180 => offset 0, so rotate is moot; use phase -90 for a live test.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let directed = manato_formats::Controller {
        kind: "envshake".into(),
        params: vec![
            ("time".into(), "3".into()),
            ("ampl".into(), "-8".into()),
            ("freq".into(), "0".into()),
            ("phase".into(), "-90".into()),
            ("dir".into(), "90".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &directed, &assets);
    sim.tick([PlayerInput::default(); 2]);
    assert!(
        (sim.camera.shake.x - -8.0).abs() < 0.01,
        "dir 90 rotates to -X"
    );
    assert!(sim.camera.shake.y.abs() < 0.01);

    // diradd advances the direction each tick: 0 deg then 90 deg.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let rotating = manato_formats::Controller {
        kind: "envshake".into(),
        params: vec![
            ("time".into(), "3".into()),
            ("ampl".into(), "-8".into()),
            ("freq".into(), "0".into()),
            ("phase".into(), "90".into()),
            ("diradd".into(), "90".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &rotating, &assets);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.camera.shake.x.abs() < 0.01);
    assert!((sim.camera.shake.y - -8.0).abs() < 0.01);
    sim.tick([PlayerInput::default(); 2]);
    assert!((sim.camera.shake.x - 8.0).abs() < 0.01);
    assert!(sim.camera.shake.y.abs() < 0.01);

    // GlobalNoShadow hides all fighter shadows.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    assert!(!sim.shadows_hidden());
    let no_shadow = manato_formats::Controller {
        kind: "assertspecial".into(),
        params: vec![
            ("type".into(), "AssertSpecial".into()),
            ("globalnoshadow".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &no_shadow, &assets);
    assert!(sim.shadows_hidden());
}

#[test]
fn action_banners_are_per_side_clamped_and_cleared_by_round() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = |text: &str, extra: Vec<(&str, &str)>| {
        let mut params = vec![("text".to_string(), text.to_string())];
        params.extend(
            extra
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string())),
        );
        manato_formats::Controller {
            kind: "lifebaraction".into(),
            params,
            line: 0,
        }
    };

    // Side 1 does not disturb side 0 (and side 2 wraps back to 0).
    let first = control("P1", vec![("time", "5")]);
    manato_sim::controller::execute(&mut sim, 0, &first, &assets);
    {
        let mut side1 = control("P2", vec![("displaytime", "4")]);
        side1.params.push(("redirectid".into(), "1".into()));
        // Execute as P2 so the banner lands on side 1.
        manato_sim::controller::execute(&mut sim, 1, &side1, &assets);
    }
    assert_eq!(sim.actions[0].len(), 1);
    assert_eq!(sim.actions[1].len(), 1);
    assert_eq!(sim.actions[0][0].text, "P1");
    assert_eq!(sim.actions[1][0].text, "P2");
    assert_eq!(sim.actions[0][0].time, 5);
    assert_eq!(sim.actions[1][0].time, 4, "displaytime fallback used");

    // Empty text is ignored; non-positive time clamps to one tick.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let empty = control("   ", vec![("time", "3")]);
    manato_sim::controller::execute(&mut sim, 0, &empty, &assets);
    assert!(sim.actions[0].is_empty(), "blank text is dropped");
    let clamped = control("CLAMP", vec![("time", "0")]);
    manato_sim::controller::execute(&mut sim, 0, &clamped, &assets);
    assert_eq!(sim.actions[0][0].time, 1);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.actions[0].is_empty(), "one-tick banner expires");

    // Starting a new round clears both sides' banners.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.push_action(0, "leftover-p1", 100);
    sim.push_action(1, "leftover-p2", 100);
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Intro {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.number, 2);
    assert!(sim.actions[0].is_empty());
    assert!(sim.actions[1].is_empty());
}

#[test]
fn lifebaraction_falls_back_to_screenpack_displaytime() {
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
    let assets = sim.assets.clone();
    let banner = |params: Vec<(&str, &str)>| manato_formats::Controller {
        kind: "lifebaraction".into(),
        params: params
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        line: 0,
    };

    // No time/displaytime: the screenpack [Action] displaytime (90) applies.
    let control = banner(vec![("text", "ROUND")]);
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    assert_eq!(sim.actions[0][0].time, 90);
    assert_eq!(sim.actions[0][0].total, 90);

    // Explicit time wins over displaytime and the screenpack default.
    let control = banner(vec![("text", "TIME"), ("time", "7"), ("displaytime", "30")]);
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    assert_eq!(sim.actions[0].last().unwrap().time, 7);

    // displaytime wins over the screenpack default when time is absent.
    let control = banner(vec![("text", "DISPLAY"), ("displaytime", "12")]);
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    assert_eq!(sim.actions[0].last().unwrap().time, 12);
}

#[test]
fn get_hitvar_velocities_and_hitover_threshold() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.ground_velocity = manato_core::Vec2::new(4.0, -2.0);
    hit.ground_hittime = 4;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    assert_eq!(manato_sim::trigger::num(&sim, 1, "GetHitVar(xvel)"), -4.0);
    assert_eq!(manato_sim::trigger::num(&sim, 1, "GetHitVar(yvel)"), -2.0);
    assert_eq!(manato_sim::trigger::num(&sim, 1, "GetHitVar(yaccel)"), 0.35);
    assert_eq!(manato_sim::trigger::int(&sim, 1, "GetHitVar(hittime)"), 4);

    // HitOver flips once the hit state has lasted `hittime` ticks.
    assert_eq!(manato_sim::trigger::int(&sim, 1, "HitOver"), 0);
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "HitOver"),
        0,
        "still inside hittime (state_time {})",
        sim.fighters[1].state_time
    );
    for i in 0..6 {
        sim.tick([PlayerInput::default(); 2]);
        eprintln!(
            "DBG i={i} st={} time={} hitover={} hittime={}",
            sim.fighters[1].state_no,
            sim.fighters[1].state_time,
            manato_sim::trigger::int(&sim, 1, "HitOver"),
            sim.fighters[1].hit.vars.hittime
        );
    }
}

#[test]
fn projectile_removetime_zero_and_countdown() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].facing = 1;
    let spawn = |sim: &mut SimState, remove_time: i32, id: i32| {
        let control = manato_formats::Controller {
            kind: "projectile".into(),
            params: vec![
                ("projanim".into(), "1500".into()),
                ("velocity".into(), "0, 0".into()),
                ("removetime".into(), remove_time.to_string()),
                ("id".into(), id.to_string()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    // removetime = 0 never spawns.
    spawn(&mut sim, 0, 1);
    assert!(sim.projectiles.is_empty());

    // removetime = 2 lives for two ticks, decrementing each frame.
    spawn(&mut sim, 2, 2);
    assert_eq!(sim.projectiles[0].remove_time, 2);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.projectiles.len(), 1);
    assert_eq!(sim.projectiles[0].remove_time, 1);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.projectiles.is_empty(), "countdown reaches zero");

    // Negative values never time out.
    spawn(&mut sim, -1, 3);
    for _ in 0..30 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.projectiles.len(), 1);
    assert_eq!(sim.projectiles[0].remove_time, -1);
}

#[test]
fn projectile_removetime_minus_two_expires_at_animation_loop_end() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = |sim: &mut SimState, anim: i32, id: i32| {
        let control = manato_formats::Controller {
            kind: "projectile".into(),
            params: vec![
                ("projanim".into(), anim.to_string()),
                ("velocity".into(), "0, 0".into()),
                ("removetime".into(), "-2".into()),
                ("id".into(), id.to_string()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    // Animation 1500 is a single 4-tick frame.
    spawn(&mut sim, 1500, 1);
    assert_eq!(sim.projectiles[0].anim_loop_time, 4);
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
        assert_eq!(sim.projectiles.len(), 1, "alive within the first loop");
    }
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.projectiles.is_empty(), "expires at loop end");

    // A 34-tick animation survives much longer.
    spawn(&mut sim, 3000, 2);
    assert_eq!(sim.projectiles[0].anim_loop_time, 34);
    for _ in 0..33 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.projectiles.len(), 1);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.projectiles.is_empty());

    // Other negative values still never expire.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    spawn(&mut sim, 1500, 3);
    let assets = sim.assets.clone();
    let modify = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("id".into(), "3".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    for _ in 0..10 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.projectiles.len(), 1);
}

#[test]
fn projectile_prefixed_aliases_and_projremove() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let control = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "0, 0".into()),
            ("projremovetime".into(), "2".into()),
            ("projhits".into(), "3".into()),
            ("projid".into(), "9".into()),
            ("projpriority".into(), "4".into()),
            ("projremove".into(), "0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &control, &assets);
    {
        let projectile = &sim.projectiles[0];
        assert_eq!(projectile.remove_time, 2);
        assert_eq!(projectile.hits, 3);
        assert_eq!(projectile.id, 9);
        assert_eq!(projectile.priority, 4);
        assert_eq!(projectile.priority_points, 4);
        assert!(!projectile.remove_on_hits_exhausted);
    }

    let modify = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "9".into()),
            ("projremovetime".into(), "-1".into()),
            ("projhits".into(), "1".into()),
            ("projpriority".into(), "7".into()),
            ("projremove".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    {
        let projectile = &sim.projectiles[0];
        assert_eq!(projectile.remove_time, -1);
        assert_eq!(projectile.hits, 1);
        assert_eq!(projectile.priority, 7);
        assert_eq!(projectile.priority_points, 7);
        assert!(projectile.remove_on_hits_exhausted);
    }
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.projectiles.len(), 1, "one hit remains");

    // Exhausted hits do not remove while projremove = 0.
    sim.projectiles[0].hits = 0;
    let keep = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "9".into()),
            ("projremove".into(), "0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &keep, &assets);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.projectiles.len(), 1, "projremove = 0 keeps it alive");

    let drop = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "9".into()),
            ("projremove".into(), "1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &drop, &assets);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.projectiles.is_empty(), "re-enabling removal clears it");
}

#[test]
fn screenbound_y_axis_clamps_to_stage_extent() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let top = assets.stage.camera.bound_high;
    let bottom = assets.stage.camera.bound_low;
    assert!(top < bottom);
    // Keep X in play so only the Y clamp is under test.
    sim.fighters[0].screen_bound = (false, true);
    sim.fighters[0].physics = manato_sim::fighter::Physics::None;
    sim.fighters[0].vel = manato_core::Vec2::ZERO;
    sim.fighters[0].ctrl = false;

    // Outside the top bound: clamped to the top.
    sim.fighters[0].pos.y = top - 200.0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].pos.y, top);

    // Outside the bottom bound: clamped to the floor.
    sim.fighters[0].pos.y = bottom + 100.0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].pos.y, bottom);

    // Disabling the Y axis leaves the position untouched.
    sim.fighters[0].screen_bound = (false, false);
    sim.fighters[0].pos.y = top - 200.0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.fighters[0].pos.y,
        top - 200.0,
        "ScreenBound Y disabled skips the clamp"
    );

    // Re-enabling clamps again.
    sim.fighters[0].screen_bound = (false, true);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[0].pos.y, top);
}

#[test]
fn projectile_projshadow_parsing_and_modify() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "0, 0".into()),
            ("removetime".into(), "-1".into()),
            ("projid".into(), "11".into()),
            ("projshadow".into(), "32, 64, 128".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &spawn, &assets);
    assert_eq!(sim.projectiles[0].shadow, [32, 64, 128]);

    let modify = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "11".into()),
            ("projshadow".into(), "10, 20".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    assert_eq!(
        sim.projectiles[0].shadow,
        [10, 20, 0],
        "missing components default to zero"
    );

    // No projshadow parameter leaves the stored color untouched.
    let velocity = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "11".into()),
            ("velocity".into(), "1, 0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &velocity, &assets);
    assert_eq!(sim.projectiles[0].shadow, [10, 20, 0]);

    // A projectile spawned without projshadow has no shadow color.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let plain = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &plain, &assets);
    assert_eq!(sim.projectiles[0].shadow, [0, 0, 0]);
}

#[test]
fn hitcount_adds_numhits_once_while_uniq_counts_contacts() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut hit = hitdef();
    hit.numhits = 2;
    hit.damage = 5;
    let origin = sim.fighters[0].pos;
    for _ in 0..3 {
        manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &hit, false, origin, 1, true);
    }
    assert_eq!(
        sim.fighters[0].attack.hit_count, 2,
        "HitCount adds numhits on first contact only"
    );
    assert_eq!(sim.fighters[0].attack.uniq_hit_count, 3);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "HitCount"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "UniqHitCount"), 3);

    // Guarded contacts raise guard_count instead of hit_count.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let mut guarded = hitdef();
    guarded.numhits = 2;
    guarded.guard_damage = 1;
    guarded.guard_damage_set = true;
    let origin = sim.fighters[0].pos;
    manato_sim::collision::apply_hit_at(&mut sim, 0, 1, &guarded, true, origin, 1, true);
    assert_eq!(sim.fighters[0].attack.guard_count, 2);
    assert_eq!(sim.fighters[0].attack.hit_count, 0);
    assert_eq!(sim.fighters[0].attack.uniq_hit_count, 1);

    // HitAdd and MoveHitReset affect both counters.
    let assets = sim.assets.clone();
    let add = manato_formats::Controller {
        kind: "hitadd".into(),
        params: vec![("value".into(), "3".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &add, &assets);
    assert_eq!(sim.fighters[0].attack.hit_count, 3);
    assert_eq!(sim.fighters[0].attack.uniq_hit_count, 4);
    let reset = manato_formats::Controller {
        kind: "movehitreset".into(),
        params: vec![],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &reset, &assets);
    assert_eq!(sim.fighters[0].attack.hit_count, 0);
    assert_eq!(sim.fighters[0].attack.uniq_hit_count, 0);
}

#[test]
fn projectile_projscale_and_projangle_parse_and_modify() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "0, 0".into()),
            ("removetime".into(), "-1".into()),
            ("projid".into(), "12".into()),
            ("projscale".into(), "2, 0.5".into()),
            ("projangle".into(), "45".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &spawn, &assets);
    assert_eq!(sim.projectiles[0].scale, manato_core::Vec2::new(2.0, 0.5));
    assert_eq!(sim.projectiles[0].angle, 45.0);

    let modify = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "12".into()),
            ("projscale".into(), "0.25, 3".into()),
            ("projangle".into(), "-30".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    assert_eq!(sim.projectiles[0].scale, manato_core::Vec2::new(0.25, 3.0));
    assert_eq!(sim.projectiles[0].angle, -30.0);

    // Omitting the parameters leaves the stored transform untouched.
    let velocity = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "12".into()),
            ("velocity".into(), "1, 0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &velocity, &assets);
    assert_eq!(sim.projectiles[0].scale, manato_core::Vec2::new(0.25, 3.0));
    assert_eq!(sim.projectiles[0].angle, -30.0);

    // Defaults are identity scale and zero angle.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let plain = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &plain, &assets);
    assert_eq!(sim.projectiles[0].scale, manato_core::Vec2::new(1.0, 1.0));
    assert_eq!(sim.projectiles[0].angle, 0.0);
}

#[test]
fn projectile_projxshear_projection_and_focallength() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "0, 0".into()),
            ("removetime".into(), "-1".into()),
            ("projid".into(), "13".into()),
            ("projxshear".into(), "0.25".into()),
            ("projprojection".into(), "perspective".into()),
            ("projfocallength".into(), "512".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &spawn, &assets);
    assert_eq!(sim.projectiles[0].x_shear, 0.25);
    assert_eq!(
        sim.projectiles[0].projection,
        manato_sim::ProjectionMode::Perspective
    );
    assert_eq!(sim.projectiles[0].focal_length, 512.0);

    let modify = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "13".into()),
            ("projxshear".into(), "-0.5".into()),
            ("projprojection".into(), "perspective2".into()),
            ("projfocallength".into(), "0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    assert_eq!(sim.projectiles[0].x_shear, -0.5);
    assert_eq!(
        sim.projectiles[0].projection,
        manato_sim::ProjectionMode::Perspective2
    );
    assert_eq!(sim.projectiles[0].focal_length, 1.0, "clamped to >= 1");

    // Omitting the parameters preserves the stored transform.
    let velocity = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "13".into()),
            ("velocity".into(), "1, 0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &velocity, &assets);
    assert_eq!(sim.projectiles[0].x_shear, -0.5);
    assert_eq!(sim.projectiles[0].focal_length, 1.0);

    // Defaults: no shear, orthographic, focal length 1024.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let plain = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &plain, &assets);
    assert_eq!(sim.projectiles[0].x_shear, 0.0);
    assert_eq!(
        sim.projectiles[0].projection,
        manato_sim::ProjectionMode::Orthographic
    );
    assert_eq!(sim.projectiles[0].focal_length, 1024.0);
}

#[test]
fn projectile_reflection_misstime_and_hit_anims() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "0, 0".into()),
            ("removetime".into(), "-1".into()),
            ("projid".into(), "14".into()),
            ("projreflection".into(), "1".into()),
            ("projmisstime".into(), "6".into()),
            ("projhitanim".into(), "20".into()),
            ("projremanim".into(), "30".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &spawn, &assets);
    assert_eq!(sim.projectiles[0].reflection, 1);
    assert_eq!(sim.projectiles[0].miss_time, 6);
    assert_eq!(sim.projectiles[0].hit_anim, 20);
    assert_eq!(sim.projectiles[0].rem_anim, 30);

    let modify = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "14".into()),
            ("projreflection".into(), "-1".into()),
            ("projmisstime".into(), "2".into()),
            ("projhitanim".into(), "21".into()),
            ("projremanim".into(), "31".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    assert_eq!(sim.projectiles[0].reflection, -1);
    assert_eq!(sim.projectiles[0].miss_time, 2);
    assert_eq!(sim.projectiles[0].hit_anim, 21);
    assert_eq!(sim.projectiles[0].rem_anim, 31);

    // Omitting them preserves the stored values.
    let velocity = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "14".into()),
            ("velocity".into(), "1, 0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &velocity, &assets);
    assert_eq!(sim.projectiles[0].hit_anim, 21);
    assert_eq!(sim.projectiles[0].rem_anim, 31);

    // Defaults: no reflection, zero miss timer, keep animations.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let plain = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &plain, &assets);
    assert_eq!(sim.projectiles[0].reflection, 0);
    assert_eq!(sim.projectiles[0].miss_time, 0);
    assert_eq!(sim.projectiles[0].hit_anim, -1);
    assert_eq!(sim.projectiles[0].rem_anim, -1);
}

#[test]
fn projectile_cancelanim_layerno_and_sprpriority() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "0, 0".into()),
            ("removetime".into(), "-1".into()),
            ("projid".into(), "15".into()),
            ("projcancelanim".into(), "40".into()),
            ("projlayerno".into(), "1".into()),
            ("projsprpriority".into(), "7".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &spawn, &assets);
    assert_eq!(sim.projectiles[0].cancel_anim, 40);
    assert_eq!(sim.projectiles[0].layer_no, 1);
    assert_eq!(sim.projectiles[0].sprite_priority, 7);

    let modify = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "15".into()),
            ("projcancelanim".into(), "41".into()),
            ("projlayerno".into(), "0".into()),
            ("projsprpriority".into(), "-2".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    assert_eq!(sim.projectiles[0].cancel_anim, 41);
    assert_eq!(sim.projectiles[0].layer_no, 0);
    assert_eq!(sim.projectiles[0].sprite_priority, -2);

    // Omitting the parameters preserves them.
    let velocity = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "15".into()),
            ("velocity".into(), "1, 0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &velocity, &assets);
    assert_eq!(sim.projectiles[0].cancel_anim, 41);
    assert_eq!(sim.projectiles[0].sprite_priority, -2);

    // Defaults: -1 cancel anim, layer 0, sprite priority 3.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let plain = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &plain, &assets);
    assert_eq!(sim.projectiles[0].cancel_anim, -1);
    assert_eq!(sim.projectiles[0].layer_no, 0);
    assert_eq!(sim.projectiles[0].sprite_priority, 3);
}

#[test]
fn projectile_bounds_parameters_storage_and_defaults() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    let spawn = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "0, 0".into()),
            ("removetime".into(), "-1".into()),
            ("projid".into(), "16".into()),
            ("projedgebound".into(), "80".into()),
            ("projstagebound".into(), "60".into()),
            ("projheightbound".into(), "-300, 5".into()),
            ("projdepthbound".into(), "120".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &spawn, &assets);
    assert_eq!(sim.projectiles[0].edge_bound, 80.0);
    assert_eq!(sim.projectiles[0].stage_bound, 60.0);
    assert_eq!(sim.projectiles[0].height_bound, (-300.0, 5.0));
    assert_eq!(sim.projectiles[0].depth_bound, 120.0);

    let modify = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "16".into()),
            ("projedgebound".into(), "20".into()),
            ("projstagebound".into(), "10".into()),
            ("projheightbound".into(), "-100, 2".into()),
            ("projdepthbound".into(), "7".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    assert_eq!(sim.projectiles[0].edge_bound, 20.0);
    assert_eq!(sim.projectiles[0].stage_bound, 10.0);
    assert_eq!(sim.projectiles[0].height_bound, (-100.0, 2.0));
    assert_eq!(sim.projectiles[0].depth_bound, 7.0);

    // Omitting the parameters preserves them.
    let velocity = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "16".into()),
            ("velocity".into(), "1, 0".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &velocity, &assets);
    assert_eq!(sim.projectiles[0].edge_bound, 20.0);
    assert_eq!(sim.projectiles[0].height_bound, (-100.0, 2.0));

    // Defaults: 40 / 40 / (-240, 1) / infinite depth.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let plain = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("removetime".into(), "-1".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &plain, &assets);
    assert_eq!(sim.projectiles[0].edge_bound, 40.0);
    assert_eq!(sim.projectiles[0].stage_bound, 40.0);
    assert_eq!(sim.projectiles[0].height_bound, (-240.0, 1.0));
    assert_eq!(sim.projectiles[0].depth_bound, f32::MAX);
}

#[test]
fn projectile_bounds_remove_out_of_range_projectiles() {
    let spawn = |sim: &mut SimState, extra: Vec<(&str, &str)>| {
        let assets = sim.assets.clone();
        let mut params = vec![
            ("projanim".to_string(), "1500".to_string()),
            ("velocity".to_string(), "0, 0".to_string()),
            ("removetime".to_string(), "-1".to_string()),
            ("projid".to_string(), "17".to_string()),
        ];
        params.extend(
            extra
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string())),
        );
        let control = manato_formats::Controller {
            kind: "projectile".into(),
            params,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };

    // Screen edge.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.camera.x = 0.0;
    spawn(&mut sim, vec![]);
    sim.projectiles[0].pos.x = 500.0;
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.projectiles.is_empty(), "far past the screen edge");

    // A large edge bound keeps it alive.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.camera.x = 0.0;
    spawn(&mut sim, vec![("projedgebound", "10000")]);
    sim.projectiles[0].pos.x = 500.0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.projectiles.len(), 1);

    // Stage bound when moving outward, with a large edge bound.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.camera.x = 0.0;
    spawn(
        &mut sim,
        vec![
            ("projedgebound", "10000"),
            ("projstagebound", "10"),
            ("velocity", "1, 0"),
        ],
    );
    sim.projectiles[0].pos.x = 400.0;
    sim.projectiles[0].facing = 1;
    sim.projectiles[0].vel = manato_core::Vec2::new(1.0, 0.0);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.projectiles.is_empty(), "past the stage camera bound");

    // Height bound (downward and upward).
    let Some(mut sim) = fight_phase() else {
        return;
    };
    spawn(&mut sim, vec![("velocity", "0, 1")]);
    sim.projectiles[0].pos.y = 100.0;
    sim.projectiles[0].vel = manato_core::Vec2::new(0.0, 1.0);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.projectiles.is_empty(), "below the height bound");

    let Some(mut sim) = fight_phase() else {
        return;
    };
    spawn(
        &mut sim,
        vec![("velocity", "0, -1"), ("projheightbound", "-50, 1")],
    );
    sim.projectiles[0].pos.y = -100.0;
    sim.projectiles[0].vel = manato_core::Vec2::new(0.0, -1.0);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.projectiles.is_empty(), "above the height bound");

    // ModifyProjectile tightening the edge bound removes it next tick.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.camera.x = 0.0;
    spawn(&mut sim, vec![("projedgebound", "10000")]);
    sim.projectiles[0].pos.x = 500.0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.projectiles.len(), 1);
    let assets = sim.assets.clone();
    let modify = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![
            ("projid".into(), "17".into()),
            ("projedgebound".into(), "10".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.projectiles.is_empty());
}

#[test]
fn camera_screen_and_drawgame_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.camera.x = 42.0;
    sim.camera.y = -17.0;
    sim.camera.scale = 1.25;
    assert_eq!(manato_sim::trigger::num(&sim, 0, "CameraPos X"), 42.0);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "CameraPos Y"), -17.0);
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "CameraPos"),
        42.0,
        "default axis is X"
    );
    assert_eq!(manato_sim::trigger::num(&sim, 0, "CameraZoom"), 1.25);

    sim.fighters[0].vel = manato_core::Vec2::new(-1.5, 2.5);
    sim.fighters[0].facing = -1;
    assert_eq!(manato_sim::trigger::num(&sim, 0, "ScreenVel X"), -1.5);
    assert_eq!(manato_sim::trigger::num(&sim, 0, "ScreenVel Y"), 2.5);

    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "GameWidth"),
        manato_core::GAME_WIDTH
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "GameHeight"),
        manato_core::GAME_HEIGHT
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "DrawGame"), 0);

    // A double KO is a draw game; a decisive winner is not.
    sim.round.phase = manato_sim::RoundPhase::Ko;
    sim.round.winner = None;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "DrawGame"), 1);
    sim.round.winner = Some(0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "DrawGame"), 0);
    sim.round.phase = manato_sim::RoundPhase::Fight;
    sim.round.winner = None;
    assert_eq!(manato_sim::trigger::int(&sim, 0, "DrawGame"), 0);
}

#[test]
fn screen_misc_and_match_progress_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "ScreenWidth"),
        manato_core::GAME_WIDTH
    );
    assert_eq!(
        manato_sim::trigger::num(&sim, 0, "ScreenHeight"),
        manato_core::GAME_HEIGHT
    );
    assert_eq!(manato_sim::trigger::int(&sim, 0, "TicksPerSecond"), 60);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "GameTime"),
        sim.frame as i32
    );
    let before = sim.frame;
    sim.tick([PlayerInput::default(); 2]);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        manato_sim::trigger::int(&sim, 0, "GameTime"),
        (before + 2) as i32
    );

    assert_eq!(manato_sim::trigger::int(&sim, 0, "RoundsExisted"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "RoundNo"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MatchOver"), 0);

    // A round transition increments RoundsExisted.
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Intro {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.number, 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "RoundNo"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "RoundsExisted"), 1);

    // MatchOver only flips once the match reaches the Over phase.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.round.rounds_to_win = 1;
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MatchOver"), 0);
    for _ in 0..250 {
        if sim.round.phase == manato_sim::RoundPhase::Over {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Over);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MatchOver"), 1);
    assert!(sim.finished);
}

#[test]
fn roundswon_and_loseko_losetime_triggers() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let trigger = |sim: &SimState, who: usize, name: &str| manato_sim::trigger::int(sim, who, name);
    assert_eq!(trigger(&sim, 0, "RoundsWon"), 0);
    assert_eq!(trigger(&sim, 1, "RoundsWon"), 0);

    // KO win: winner's RoundsWon rises and the loser reports LoseKO.
    sim.fighters[1].life = 0;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    assert_eq!(trigger(&sim, 0, "RoundsWon"), 1);
    assert_eq!(trigger(&sim, 1, "RoundsWon"), 0);
    assert_eq!(trigger(&sim, 1, "LoseKO"), 1);
    assert_eq!(trigger(&sim, 0, "LoseKO"), 0);
    assert_eq!(trigger(&sim, 1, "LoseTime"), 0);

    // Advance to round 2, then win on time instead of KO.
    for _ in 0..200 {
        if sim.round.phase == manato_sim::RoundPhase::Intro {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.number, 2);
    for _ in 0..120 {
        if sim.round.phase == manato_sim::RoundPhase::Fight {
            break;
        }
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Fight);
    sim.fighters[0].life = 800;
    sim.fighters[1].life = 300;
    sim.round.timer = 1;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.round.phase, manato_sim::RoundPhase::Ko);
    assert!(sim.round.time_over);
    assert_eq!(trigger(&sim, 0, "RoundsWon"), 2);
    assert_eq!(trigger(&sim, 1, "RoundsWon"), 0);
    assert_eq!(trigger(&sim, 1, "LoseTime"), 1);
    assert_eq!(trigger(&sim, 1, "LoseKO"), 0, "time-over is not a KO loss");
}

#[test]
fn projectile_hit_and_removal_animations_play_before_removal() {
    // Hit animation: a one-hit projectile plays projhitanim, then vanishes.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -5.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 5.0;
    sim.fighters[1].facing = -1;
    let spawn = |sim: &mut SimState, params: Vec<(&str, &str)>| {
        let mut all = vec![
            ("projanim".to_string(), "1500".to_string()),
            ("velocity".to_string(), "0, 0".to_string()),
            ("removetime".to_string(), "-1".to_string()),
            ("hits".to_string(), "1".to_string()),
            ("damage".to_string(), "20".to_string()),
        ];
        all.extend(
            params
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string())),
        );
        let control = manato_formats::Controller {
            kind: "projectile".into(),
            params: all,
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    spawn(&mut sim, vec![("projhitanim", "5000")]);
    let life = sim.fighters[1].life;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(sim.fighters[1].life, life - 20, "projectile connects once");
    {
        let projectile = sim.projectiles.first().expect("hit animation playing");
        assert_eq!(
            projectile.status,
            manato_sim::projectile::ProjectileStatus::Hit
        );
        assert_eq!(projectile.anim, 5000);
    }
    for _ in 0..14 {
        sim.tick([PlayerInput::default(); 2]);
    }
    assert_eq!(sim.fighters[1].life, life - 20, "no further damage");
    assert_eq!(sim.projectiles.len(), 1);
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.projectiles.is_empty(), "hit animation finished");

    // Removal animation: expired lifetime plays projremanim first.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    sim.fighters[0].pos.x = -5.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 200.0;
    spawn(&mut sim, vec![("projremanim", "1500")]);
    sim.projectiles[0].remove_time = 1;
    sim.tick([PlayerInput::default(); 2]);
    {
        let projectile = sim.projectiles.first().expect("removal animation playing");
        assert_eq!(
            projectile.status,
            manato_sim::projectile::ProjectileStatus::Rem
        );
        assert_eq!(projectile.anim, 1500);
        assert_eq!(projectile.removal_ticks, 4);
    }
    for _ in 0..3 {
        sim.tick([PlayerInput::default(); 2]);
        assert_eq!(sim.projectiles.len(), 1, "within the removal animation");
    }
    sim.tick([PlayerInput::default(); 2]);
    assert!(sim.projectiles.is_empty(), "removal animation finished");
}

#[test]
fn projectile_remvelocity_replaces_velocity_on_removal() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -5.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 200.0;
    let spawn = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "5, 0".into()),
            ("remvelocity".into(), "3, -1".into()),
            ("removetime".into(), "1".into()),
            ("projremanim".into(), "1500".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &spawn, &assets);
    sim.tick([PlayerInput::default(); 2]);
    {
        let projectile = sim.projectiles.first().expect("removal animation");
        assert_eq!(
            projectile.status,
            manato_sim::projectile::ProjectileStatus::Rem
        );
        assert_eq!(projectile.vel, manato_core::Vec2::new(3.0, -1.0));
    }

    // ModifyProjectile can change the removal velocity; `projremvelocity` alias
    // is accepted too.
    let modify = manato_formats::Controller {
        kind: "modifyprojectile".into(),
        params: vec![("projremvelocity".into(), "-2, 4".into())],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &modify, &assets);
    assert_eq!(
        sim.projectiles[0].rem_velocity,
        manato_core::Vec2::new(-2.0, 4.0)
    );

    // Default removal velocity is zero.
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let plain = manato_formats::Controller {
        kind: "projectile".into(),
        params: vec![
            ("projanim".into(), "1500".into()),
            ("velocity".into(), "5, 0".into()),
            ("removetime".into(), "1".into()),
            ("projremanim".into(), "1500".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 0, &plain, &assets);
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.projectiles.first().expect("removal").vel,
        manato_core::Vec2::ZERO
    );
}

#[test]
fn numproj_counts_only_active_projectiles() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 200.0;
    let spawn = |sim: &mut SimState, id: i32| {
        let control = manato_formats::Controller {
            kind: "projectile".into(),
            params: vec![
                ("projanim".into(), "1500".into()),
                ("velocity".into(), "0, 0".into()),
                ("removetime".into(), "-1".into()),
                ("projid".into(), id.to_string()),
            ],
            line: 0,
        };
        manato_sim::controller::execute(sim, 0, &control, &assets);
    };
    spawn(&mut sim, 3);
    spawn(&mut sim, 4);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProj"), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProjID(3)"), 1);

    // Put one projectile into its removal animation; it still exists but is no
    // longer active, so it is not counted.
    sim.projectiles[0].remove_time = 1;
    sim.projectiles[0].rem_anim = 1500;
    sim.tick([PlayerInput::default(); 2]);
    assert_eq!(
        sim.projectiles[0].status,
        manato_sim::projectile::ProjectileStatus::Rem
    );
    assert_eq!(sim.projectiles.len(), 2);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProj"), 1);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProjID(3)"), 0);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "NumProjID(4)"), 1);
}

#[test]
fn move_reversed_trigger_after_reversal_def() {
    let Some(mut sim) = fight_phase() else {
        return;
    };
    let assets = sim.assets.clone();
    sim.fighters[0].pos.x = -10.0;
    sim.fighters[0].facing = 1;
    sim.fighters[1].pos.x = 10.0;
    sim.fighters[1].facing = -1;
    sim.fighters[0].enter_state(200, &assets.characters[0], false);
    sim.fighters[0].set_anim(200, &assets.characters[0], Some(2));
    sim.fighters[0].hitdef = Some(hitdef());
    let reversal = manato_formats::Controller {
        kind: "reversaldef".into(),
        params: vec![
            ("attr".into(), "S, NA".into()),
            ("damage".into(), "5".into()),
            ("time".into(), "5".into()),
        ],
        line: 0,
    };
    manato_sim::controller::execute(&mut sim, 1, &reversal, &assets);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveReversed"), 0);
    manato_sim::collision::resolve(&mut sim);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveReversed"), 1);
    assert_eq!(
        manato_sim::trigger::int(&sim, 1, "MoveReversed"),
        0,
        "the reversing fighter is not marked"
    );

    // Entering a new state clears the flag alongside the rest of the swing.
    sim.fighters[0].enter_state(0, &assets.characters[0], false);
    assert_eq!(manato_sim::trigger::int(&sim, 0, "MoveReversed"), 0);
}
