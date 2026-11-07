use std::path::PathBuf;

use manato_core::PlayerInput;
use manato_sim::{ai, MatchConfig, MatchOptions, SimState};

fn mugen_root() -> PathBuf {
    std::env::var("MANATO_MUGEN_ROOT")
        .or_else(|_| std::env::var("MUGEN_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("mugen"))
}

fn load(p1: &str, p2: &str) -> Option<SimState> {
    let root = mugen_root();
    let config = MatchConfig {
        mugen_root: root.clone(),
        p1: PathBuf::from(p1),
        p2: PathBuf::from(p2),
        stage: PathBuf::from("content/stages/stage1.def"),
        rounds_to_win: 2,
        round_time: 99,
    };
    if !config.p1.exists() || !config.p2.exists() {
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
