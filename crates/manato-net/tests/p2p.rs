use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ggrs::UdpNonBlockingSocket;
use manato_core::{PlayerInput, DIR_LEFT, DIR_RIGHT};
use manato_net::Netplay;
use manato_sim::{Assets, MatchConfig, MatchOptions, SimState};

fn root() -> PathBuf {
    std::env::var_os("MANATO_MUGEN_ROOT")
        .or_else(|| std::env::var_os("MUGEN_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn load() -> Option<SimState> {
    let root = root();
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
        return None;
    }
    Some(SimState::new(
        Arc::new(Assets::load(&config).expect("assets load")),
        MatchOptions {
            rounds_to_win: 2,
            round_time: 99,
            ai_level: 0,
        },
    ))
}

#[test]
fn p2p_loopback_converges() {
    let Some(sim_a) = load() else {
        eprintln!("skipping: content not present");
        return;
    };
    let sim_b = load().expect("second sim");

    // Ports are derived from the process id to avoid collisions with other
    // test runs; if they are unavailable, skip rather than fail CI.
    let base = 41_000 + (std::process::id() as u16 % 400) * 2;
    let Ok(socket_a) = UdpNonBlockingSocket::bind_to_port(base) else {
        eprintln!("skipping: port {base} unavailable");
        return;
    };
    let Ok(socket_b) = UdpNonBlockingSocket::bind_to_port(base + 1) else {
        eprintln!("skipping: port {} unavailable", base + 1);
        return;
    };
    let addr_a: SocketAddr = ([127, 0, 0, 1], base).into();
    let addr_b: SocketAddr = ([127, 0, 0, 1], base + 1).into();

    let mut a = Netplay::new(sim_a, 0, socket_a, addr_b).expect("session A");
    let mut b = Netplay::new(sim_b, 1, socket_b, addr_a).expect("session B");

    let deadline = Instant::now() + Duration::from_secs(20);
    let target = 120;
    while Instant::now() < deadline {
        a.poll();
        b.poll();
        let _ = a.step(PlayerInput::new(DIR_RIGHT, 0));
        let _ = b.step(PlayerInput::new(DIR_LEFT, 0));
        if a.session().confirmed_frame() >= target && b.session().confirmed_frame() >= target {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    assert!(
        a.session().confirmed_frame() >= target,
        "session A stalled at {}",
        a.session().confirmed_frame()
    );
    assert!(
        b.session().confirmed_frame() >= target,
        "session B stalled at {}",
        b.session().confirmed_frame()
    );
    assert_eq!(
        a.sim().checksum(),
        b.sim().checksum(),
        "peers diverged after {target} confirmed frames"
    );
}
