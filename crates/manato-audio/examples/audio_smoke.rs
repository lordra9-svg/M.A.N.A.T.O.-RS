//! Manual audio smoke test: loads the default match, plays a few seconds of
//! CPU-vs-CPU sound events, and reports how many events reached the mixer.
//!
//! Run with:
//!   RUST_LOG=manato_audio=info cargo run -p manato-audio --example audio_smoke

use std::path::PathBuf;
use std::sync::Arc;

use manato_audio::AudioEngine;
use manato_sim::{ai, Assets, MatchConfig, MatchOptions, SimState};

fn main() -> anyhow::Result<()> {
    let root = std::env::var_os("MANATO_MUGEN_ROOT")
        .or_else(|| std::env::var_os("MUGEN_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let p1 = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "content/chars/kaen/kaen.def".into());
    let p2 = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "content/chars/nival/nival.def".into());
    let mut config = MatchConfig::defaults(&root);
    config.p1 = PathBuf::from(p1);
    config.p2 = PathBuf::from(p2);
    let assets = Assets::load(&config)?;
    let mut sim = SimState::new(
        Arc::new(assets),
        MatchOptions {
            rounds_to_win: 2,
            round_time: 99,
            ai_level: 6,
            ai_sides: [true, true],
            netplay: false,
        },
    );

    let audio = AudioEngine::new();
    println!(
        "backend: {}, sample rate: {} Hz",
        audio.backend_name(),
        audio.sample_rate()
    );

    let mut events = 0;
    let mut keys = std::collections::BTreeMap::new();
    for _ in 0..1_200 {
        sim.tick([ai::input_for(&sim, 0, 6), ai::input_for(&sim, 1, 6)]);
        events += sim.sounds.len();
        for event in &sim.sounds {
            *keys.entry((event.group, event.index)).or_insert(0) += 1;
        }
        audio.play_events(&sim);
    }
    println!("event keys: {keys:?}");
    println!(
        "played {events} events; final life {}/{}",
        sim.fighters[0].life, sim.fighters[1].life
    );
    // Give the writer thread time to drain the last chunk.
    std::thread::sleep(std::time::Duration::from_millis(300));
    Ok(())
}
