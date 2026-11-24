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
            ai_sides: [false, false],
            netplay: true,
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
        if a.session().confirmed_frame() >= target
            && a.session().confirmed_frame() == b.session().confirmed_frame()
            && a.sim().frame == b.sim().frame
        {
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

/// Deterministic UDP proxy: adds latency/jitter and drops a periodic packet.
/// Packets from A are re-sent from the B-facing socket so the peers still see
/// their configured remote addresses.
fn spawn_proxy(
    a_side: std::net::UdpSocket,
    b_side: std::net::UdpSocket,
    a_addr: SocketAddr,
    b_addr: SocketAddr,
) -> (
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    std::thread::JoinHandle<()>,
) {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};

    a_side.set_nonblocking(true).expect("a nonblocking");
    b_side.set_nonblocking(true).expect("b nonblocking");
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let handle = std::thread::spawn(move || {
        let mut queue: VecDeque<(Instant, bool, Vec<u8>)> = VecDeque::new();
        let mut buffer = [0u8; 4096];
        let mut count = 0u32;
        while !stop_thread.load(Ordering::Relaxed) {
            for (socket, from_b) in [(&a_side, false), (&b_side, true)] {
                loop {
                    match socket.recv_from(&mut buffer) {
                        Ok((size, _)) => {
                            count += 1;
                            // Keep the initial handshake intact, then drop a
                            // deterministic packet every 17th.
                            if count > 16 && count % 17 == 0 {
                                continue;
                            }
                            let delay = Duration::from_millis(12 + u64::from(count % 4) * 6);
                            queue.push_back((
                                Instant::now() + delay,
                                from_b,
                                buffer[..size].to_vec(),
                            ));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                }
            }
            let now = Instant::now();
            let mut remaining = VecDeque::new();
            while let Some((due, from_b, data)) = queue.pop_front() {
                if due <= now {
                    // from_b means this packet came from B; deliver it to A.
                    let target = if from_b { a_addr } else { b_addr };
                    let socket = if from_b { &a_side } else { &b_side };
                    let _ = socket.send_to(&data, target);
                } else {
                    remaining.push_back((due, from_b, data));
                }
            }
            queue = remaining;
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    (stop, handle)
}

#[test]
fn p2p_latency_jitter_and_loss_converges() {
    let Some(sim_a) = load() else {
        eprintln!("skipping: content not present");
        return;
    };
    let sim_b = load().expect("second sim");
    let base = 43_000 + (std::process::id() as u16 % 400) * 2;
    let Ok(socket_a) = UdpNonBlockingSocket::bind_to_port(base) else {
        eprintln!("skipping: port {base} unavailable");
        return;
    };
    let Ok(socket_b) = UdpNonBlockingSocket::bind_to_port(base + 1) else {
        eprintln!("skipping: port {} unavailable", base + 1);
        return;
    };
    let Ok(proxy_a) = std::net::UdpSocket::bind(("127.0.0.1", base + 2)) else {
        eprintln!("skipping: proxy port unavailable");
        return;
    };
    let Ok(proxy_b) = std::net::UdpSocket::bind(("127.0.0.1", base + 3)) else {
        eprintln!("skipping: proxy port unavailable");
        return;
    };
    let a_addr: SocketAddr = ([127, 0, 0, 1], base).into();
    let b_addr: SocketAddr = ([127, 0, 0, 1], base + 1).into();
    let proxy_a_addr: SocketAddr = ([127, 0, 0, 1], base + 2).into();
    let proxy_b_addr: SocketAddr = ([127, 0, 0, 1], base + 3).into();
    let (stop, proxy_thread) = spawn_proxy(proxy_a, proxy_b, a_addr, b_addr);

    let mut a = Netplay::new(sim_a, 0, socket_a, proxy_a_addr).expect("session A");
    let mut b = Netplay::new(sim_b, 1, socket_b, proxy_b_addr).expect("session B");

    let deadline = Instant::now() + Duration::from_secs(30);
    let target = 90;
    while Instant::now() < deadline {
        a.poll();
        b.poll();
        let _ = a.step(PlayerInput::new(DIR_RIGHT, 0));
        let _ = b.step(PlayerInput::new(DIR_LEFT, 0));
        if a.session().confirmed_frame() >= target
            && a.session().confirmed_frame() == b.session().confirmed_frame()
            && a.sim().frame == b.sim().frame
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = proxy_thread.join();

    assert!(
        a.session().confirmed_frame() >= target,
        "session A stalled at {} under latency/jitter/loss",
        a.session().confirmed_frame()
    );
    assert!(
        b.session().confirmed_frame() >= target,
        "session B stalled at {} under latency/jitter/loss",
        b.session().confirmed_frame()
    );
    assert_eq!(
        a.sim().checksum(),
        b.sim().checksum(),
        "peers diverged under simulated network conditions"
    );
}
