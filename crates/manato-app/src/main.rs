//! M.A.N.A.T.O.-RS command line runner and window.

#![cfg_attr(not(feature = "gamepad"), allow(dead_code))]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use manato_audio::AudioEngine;
use manato_core::{Buttons, PlayerInput, DIR_DOWN, DIR_LEFT, DIR_RIGHT, DIR_UP};
use manato_formats::screenpack::{ArcadeOrderMode, Roster, RosterEntry, Screenpack};
use manato_formats::SffFile;
use manato_render::frontend::{OptionsView, SelectEntry, SelectView, TitleView};
use manato_render::{wgpu, Renderer, GAME_HEIGHT, GAME_WIDTH};
use manato_sim::arcade::{ArcadeConfig, ArcadePhase, ArcadeSession};
use manato_sim::{ai, Assets, MatchConfig, MatchOptions, SimState};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

const TICK: Duration = Duration::from_nanos(16_666_667);

#[derive(Debug, Clone)]
struct Options {
    root: PathBuf,
    p1: String,
    p2: String,
    stage: String,
    screenpack: Option<String>,
    ai_level: i32,
    rounds: u32,
    round_time: i32,
    frames: Option<u32>,
    synctest: Option<u32>,
    screenshot: Option<PathBuf>,
    storyboard: Option<String>,
    spectate: Option<String>,
    broadcast: Option<String>,
    discover: Option<String>,
    stun: Option<String>,
    matchmake: Option<String>,
    matchmake_play: Option<String>,
    scale: u32,
    menu: bool,
    menu_screen: String,
    p1_keys: KeyBindings,
    p2_keys: KeyBindings,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            root: std::env::var_os("MUGEN_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".")),
            p1: "content/chars/kaen/kaen.def".into(),
            p2: "content/chars/nival/nival.def".into(),
            stage: "content/stages/training/stage1.def".into(),
            screenpack: None,
            ai_level: 6,
            rounds: 2,
            round_time: 99,
            frames: None,
            synctest: None,
            screenshot: None,
            storyboard: None,
            spectate: None,
            broadcast: None,
            discover: None,
            stun: None,
            matchmake: None,
            matchmake_play: None,
            scale: 3,
            menu: false,
            menu_screen: "title".into(),
            p1_keys: KeyBindings::default(),
            p2_keys: KeyBindings::player_two(),
        }
    }
}

/// Keyboard bindings for one player.
#[derive(Debug, Clone, Copy)]
struct KeyBindings {
    up: KeyCode,
    down: KeyCode,
    left: KeyCode,
    right: KeyCode,
    buttons: [KeyCode; 6],
    start: KeyCode,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            up: KeyCode::KeyW,
            down: KeyCode::KeyS,
            left: KeyCode::KeyA,
            right: KeyCode::KeyD,
            buttons: [
                KeyCode::KeyJ,
                KeyCode::KeyK,
                KeyCode::KeyL,
                KeyCode::KeyU,
                KeyCode::KeyI,
                KeyCode::KeyO,
            ],
            start: KeyCode::Enter,
        }
    }
}

impl KeyBindings {
    fn player_two() -> Self {
        Self {
            up: KeyCode::ArrowUp,
            down: KeyCode::ArrowDown,
            left: KeyCode::ArrowLeft,
            right: KeyCode::ArrowRight,
            buttons: [
                KeyCode::Numpad1,
                KeyCode::Numpad2,
                KeyCode::Numpad3,
                KeyCode::Numpad4,
                KeyCode::Numpad5,
                KeyCode::Numpad6,
            ],
            start: KeyCode::Numpad0,
        }
    }

    fn apply(&mut self, file: &KeyFile) {
        let assign = |slot: &mut KeyCode, name: &Option<String>, label: &str| {
            if let Some(name) = name {
                match parse_key(name) {
                    Some(code) => *slot = code,
                    None => log::warn!("unknown key {name:?} for {label}"),
                }
            }
        };
        assign(&mut self.up, &file.up, "up");
        assign(&mut self.down, &file.down, "down");
        assign(&mut self.left, &file.left, "left");
        assign(&mut self.right, &file.right, "right");
        assign(&mut self.start, &file.start, "start");
        for (index, (label, name)) in [
            ("x", &file.x),
            ("y", &file.y),
            ("z", &file.z),
            ("a", &file.a),
            ("b", &file.b),
            ("c", &file.c),
        ]
        .into_iter()
        .enumerate()
        {
            assign(&mut self.buttons[index], name, label);
        }
    }
}

#[derive(Debug, Default, serde::Deserialize)]
struct KeyFile {
    up: Option<String>,
    down: Option<String>,
    left: Option<String>,
    right: Option<String>,
    x: Option<String>,
    y: Option<String>,
    z: Option<String>,
    a: Option<String>,
    b: Option<String>,
    c: Option<String>,
    start: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct InputFile {
    p1: Option<KeyFile>,
    p2: Option<KeyFile>,
}

fn parse_key(name: &str) -> Option<KeyCode> {
    const LETTERS: [KeyCode; 26] = [
        KeyCode::KeyA,
        KeyCode::KeyB,
        KeyCode::KeyC,
        KeyCode::KeyD,
        KeyCode::KeyE,
        KeyCode::KeyF,
        KeyCode::KeyG,
        KeyCode::KeyH,
        KeyCode::KeyI,
        KeyCode::KeyJ,
        KeyCode::KeyK,
        KeyCode::KeyL,
        KeyCode::KeyM,
        KeyCode::KeyN,
        KeyCode::KeyO,
        KeyCode::KeyP,
        KeyCode::KeyQ,
        KeyCode::KeyR,
        KeyCode::KeyS,
        KeyCode::KeyT,
        KeyCode::KeyU,
        KeyCode::KeyV,
        KeyCode::KeyW,
        KeyCode::KeyX,
        KeyCode::KeyY,
        KeyCode::KeyZ,
    ];
    const DIGITS: [KeyCode; 10] = [
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    const NUMPAD: [KeyCode; 10] = [
        KeyCode::Numpad0,
        KeyCode::Numpad1,
        KeyCode::Numpad2,
        KeyCode::Numpad3,
        KeyCode::Numpad4,
        KeyCode::Numpad5,
        KeyCode::Numpad6,
        KeyCode::Numpad7,
        KeyCode::Numpad8,
        KeyCode::Numpad9,
    ];
    let name = name.trim();
    let lower = name.to_ascii_lowercase();
    if lower.len() == 1 {
        let ch = lower.chars().next().unwrap();
        if ch.is_ascii_alphabetic() {
            return Some(LETTERS[(ch as u8 - b'a') as usize]);
        }
        if let Some(digit) = ch.to_digit(10) {
            return Some(DIGITS[digit as usize]);
        }
    }
    if let Some(rest) = lower.strip_prefix("numpad") {
        return rest
            .parse::<usize>()
            .ok()
            .and_then(|index| NUMPAD.get(index).copied());
    }
    Some(match lower.as_str() {
        "up" | "arrowup" => KeyCode::ArrowUp,
        "down" | "arrowdown" => KeyCode::ArrowDown,
        "left" | "arrowleft" => KeyCode::ArrowLeft,
        "right" | "arrowright" => KeyCode::ArrowRight,
        "enter" | "return" => KeyCode::Enter,
        "space" => KeyCode::Space,
        "escape" | "esc" => KeyCode::Escape,
        "tab" => KeyCode::Tab,
        "shift" | "lshift" => KeyCode::ShiftLeft,
        "rshift" => KeyCode::ShiftRight,
        "ctrl" | "control" | "lctrl" => KeyCode::ControlLeft,
        "rctrl" => KeyCode::ControlRight,
        "alt" | "lalt" => KeyCode::AltLeft,
        "ralt" => KeyCode::AltRight,
        "backspace" => KeyCode::Backspace,
        "comma" => KeyCode::Comma,
        "period" | "dot" => KeyCode::Period,
        "slash" => KeyCode::Slash,
        "semicolon" => KeyCode::Semicolon,
        "quote" => KeyCode::Quote,
        "minus" => KeyCode::Minus,
        "equal" => KeyCode::Equal,
        "backslash" => KeyCode::Backslash,
        "grave" => KeyCode::Backquote,
        "bracketleft" | "leftbracket" => KeyCode::BracketLeft,
        "bracketright" | "rightbracket" => KeyCode::BracketRight,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "insert" => KeyCode::Insert,
        "delete" | "del" => KeyCode::Delete,
        _ => return None,
    })
}

/// Optional TOML config file. Command-line arguments override these values.
#[derive(Debug, Default, serde::Deserialize)]
struct FileConfig {
    root: Option<PathBuf>,
    p1: Option<String>,
    p2: Option<String>,
    stage: Option<String>,
    screenpack: Option<String>,
    ai: Option<i32>,
    rounds: Option<u32>,
    round_time: Option<i32>,
    scale: Option<u32>,
    input: Option<InputFile>,
}

impl Options {
    fn parse() -> anyhow::Result<Self> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut opts = Options::default();

        // Load the config file first so explicit CLI flags win.
        if let Some(pos) = args.iter().position(|arg| arg == "--config") {
            let path = args
                .get(pos + 1)
                .with_context(|| "missing value for --config")?;
            opts.apply_file(Path::new(path))?;
        }

        let value = |i: usize| -> anyhow::Result<&str> {
            args.get(i + 1)
                .map(String::as_str)
                .with_context(|| format!("missing value for {}", args[i]))
        };
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--config" => i += 2,
                "--root" => {
                    opts.root = PathBuf::from(value(i)?);
                    i += 2;
                }
                "--p1" => {
                    opts.p1 = value(i)?.to_owned();
                    i += 2;
                }
                "--p2" => {
                    opts.p2 = value(i)?.to_owned();
                    i += 2;
                }
                "--stage" => {
                    opts.stage = value(i)?.to_owned();
                    i += 2;
                }
                "--screenpack" => {
                    opts.screenpack = Some(value(i)?.to_owned());
                    i += 2;
                }
                "--menu" | "--frontend" => {
                    opts.menu = true;
                    i += 1;
                }
                "--screen" => {
                    opts.menu_screen = value(i)?.to_ascii_lowercase();
                    i += 2;
                }
                "--ai" => {
                    opts.ai_level = value(i)?.parse()?;
                    i += 2;
                }
                "--rounds" => {
                    opts.rounds = value(i)?.parse()?;
                    i += 2;
                }
                "--round-time" => {
                    opts.round_time = value(i)?.parse()?;
                    i += 2;
                }
                "--frames" => {
                    opts.frames = Some(value(i)?.parse()?);
                    i += 2;
                }
                "--synctest" => {
                    opts.synctest = Some(value(i)?.parse()?);
                    i += 2;
                }
                "--screenshot" => {
                    opts.screenshot = Some(PathBuf::from(value(i)?));
                    i += 2;
                }
                "--storyboard" => {
                    opts.storyboard = Some(value(i)?.to_owned());
                    i += 2;
                }
                "--spectate" => {
                    opts.spectate = Some(value(i)?.to_owned());
                    i += 2;
                }
                "--broadcast" => {
                    opts.broadcast = Some(value(i)?.to_owned());
                    i += 2;
                }
                "--discover" => {
                    opts.discover = Some(value(i)?.to_owned());
                    i += 2;
                }
                "--stun" => {
                    opts.stun = Some(value(i)?.to_owned());
                    i += 2;
                }
                "--matchmake" => {
                    opts.matchmake = Some(value(i)?.to_owned());
                    i += 2;
                }
                "--matchmake-play" => {
                    opts.matchmake_play = Some(value(i)?.to_owned());
                    i += 2;
                }
                "--scale" => {
                    opts.scale = value(i)?.parse::<u32>()?.max(1);
                    i += 2;
                }
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                other => anyhow::bail!("unknown argument {other}; try --help"),
            }
        }
        Ok(opts)
    }

    fn apply_file(&mut self, path: &Path) -> anyhow::Result<()> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let file: FileConfig = toml::from_str(&text)
            .with_context(|| format!("failed to parse config {}", path.display()))?;
        if let Some(root) = file.root {
            self.root = if root.is_absolute() {
                root
            } else {
                path.parent().unwrap_or(Path::new(".")).join(root)
            };
        }
        if let Some(p1) = file.p1 {
            self.p1 = p1;
        }
        if let Some(p2) = file.p2 {
            self.p2 = p2;
        }
        if let Some(stage) = file.stage {
            self.stage = stage;
        }
        if let Some(screenpack) = file.screenpack {
            self.screenpack = Some(screenpack);
        }
        if let Some(ai) = file.ai {
            self.ai_level = ai;
        }
        if let Some(rounds) = file.rounds {
            self.rounds = rounds;
        }
        if let Some(round_time) = file.round_time {
            self.round_time = round_time;
        }
        if let Some(scale) = file.scale {
            self.scale = scale.max(1);
        }
        if let Some(input) = file.input {
            if let Some(p1) = &input.p1 {
                self.p1_keys.apply(p1);
            }
            if let Some(p2) = &input.p2 {
                self.p2_keys.apply(p2);
            }
        }
        Ok(())
    }

    fn config(&self) -> MatchConfig {
        // `MatchConfig::resolve` applies `mugen_root`; keep these paths
        // relative here so they are not joined twice.
        MatchConfig {
            mugen_root: self.root.clone(),
            p1: PathBuf::from(&self.p1),
            p2: PathBuf::from(&self.p2),
            stage: PathBuf::from(&self.stage),
            screenpack: self.screenpack.as_ref().map(PathBuf::from),
            rounds_to_win: self.rounds,
            round_time: self.round_time,
        }
    }
}

fn print_help() {
    print!(
        r#"M.A.N.A.T.O.-RS - independent M.U.G.E.N reimplementation

Usage: manato [OPTIONS]

Content:
  --config <toml>     Load defaults from a TOML config file
  --root <dir>        Content root (default $MUGEN_ROOT or the repo root)
  --p1 <def>          Player 1 character definition
  --p2 <def>          Player 2 character definition
  --stage <def>       Stage definition
  --screenpack <def>  Screenpack system.def; with --menu defaults to the
                      bundled Manato Grid (use "none" for built-in text/HUD)
  --ai <level>        CPU level for slot 2, 0 = human (default 6)
  --rounds <n>        Rounds needed to win (default 2)
  --round-time <s>    Round timer in seconds (default 99)

Modes:
  --menu              Start at the title screen (front end)
  --screen <name>     Headless front-end screen: title, select, options
  --frames <n>        Run n ticks headless, then exit
  --synctest <n>      Run n ticks through GGRS rollback verification
  --screenshot <png>  Save the final frame (headless)
  --storyboard <def>  Play a storyboard headless (with --frames)
  --spectate <addr>   Watch a host streaming snapshots (host:port)
  --broadcast <addr>  Stream match snapshots to spectators (bind addr)
  --discover <addr>   Query LAN lobby discovery at addr and print results
  --stun <addr>       Report the public UDP address via a STUN server
  --matchmake <addr>  Discover lobbies, list them, and spectate the best one
  --matchmake-play <addr>  Discover, join the best lobby as a player, and ready
  --scale <n>         Window integer scale (default 3)

Controls (window mode):
  P1: WASD move, J/K/L = X/Y/Z, U/I/O = A/B/C
  P2: arrows move, numpad 1/2/3 = X/Y/Z, 4/5/6 = A/B/C
  Pads: d-pad/left stick, South/X/Y/Z = X/Y/Z, West/A/B/C = A/B/C, Start
  R restart match, F12 screenshot, Esc quit
"#
    );
}

fn run_discovery(target: &str) -> anyhow::Result<()> {
    let target: std::net::SocketAddr = target.parse()?;
    let mut browser = manato_net::discovery::DiscoveryBrowser::new()?;
    browser.query(target)?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(1) {
        browser.poll()?;
        std::thread::sleep(Duration::from_millis(10));
    }
    if browser.lobbies().is_empty() {
        println!("no lobbies found at {target}");
    }
    for lobby in browser.lobbies() {
        println!(
            "{} @ {} players={} spectators={} phase={:?}",
            lobby.name, lobby.address, lobby.players, lobby.spectators, lobby.phase
        );
    }
    Ok(())
}

fn run_matchmake_play(target: &str, opts: &Options) -> anyhow::Result<()> {
    let target: std::net::SocketAddr = target.parse()?;
    let lobbies = manato_net::discovery::Matchmaker::discover(target, Duration::from_secs(1))?;
    for lobby in &lobbies {
        println!(
            "{} @ {} players={} spectators={} phase={:?}",
            lobby.name, lobby.address, lobby.players, lobby.spectators, lobby.phase
        );
    }
    let Some(best) = manato_net::discovery::best_lobby(&lobbies) else {
        println!("no joinable lobbies");
        return Ok(());
    };
    let mut client = manato_net::lobby::LobbyClient::connect(best.address, "player", false)?;
    println!("joining {} @ {}", best.name, best.address);

    // Reserve a local port for the GGRS socket, then announce it.
    let local_port = {
        let probe = std::net::UdpSocket::bind("0.0.0.0:0")?;
        probe.local_addr()?.port()
    };
    let started = Instant::now();
    let mut ready_sent = false;
    let mut handoff = None;
    while started.elapsed() < Duration::from_secs(15) {
        client.poll()?;
        if client.id.is_some() && !ready_sent {
            client.announce_netplay(local_port);
            client.set_ready(true);
            ready_sent = true;
        }
        if let Some(value) = client.match_handoff() {
            handoff = Some(value);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let Some(handoff) = handoff else {
        println!("{}", client.status_text());
        println!("lobby did not start a match within 15s");
        return Ok(());
    };

    // Auto-enter: build a GGRS session from the handoff and run a bounded
    // headless loop with AI local inputs.
    println!(
        "entering netplay: local player {} remote {}",
        handoff.local_player, handoff.remote
    );
    let sim = load_match(opts)?;
    let bind = std::net::SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, local_port));
    // Best-effort NAT hole punching on the same local port the GGRS session
    // will use. Both peers must be in this flow for it to succeed.
    let mut puncher = manato_net::nat::NatPuncher::bind(bind, handoff.remote)?;
    let punched = puncher.punch(Duration::from_secs(2))?;
    println!(
        "nat punch {} (sent={} received={})",
        if punched { "open" } else { "timed out" },
        puncher.packets_sent,
        puncher.packets_received
    );
    drop(puncher);
    let mut netplay = manato_net::Netplay::from_handoff(sim, handoff, bind)?;
    let mut stepped = 0u32;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) && !netplay.sim().finished {
        let input = ai::input_for(netplay.sim(), handoff.local_player, opts.ai_level.max(1));
        if netplay.step(input)? {
            stepped += 1;
        }
        if netplay.sim().finished {
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    println!(
        "netplay stopped: frames={stepped} checksum={:016x} finished={}",
        netplay.sim().checksum(),
        netplay.sim().finished
    );
    Ok(())
}

fn run_stun(server: &str) -> anyhow::Result<()> {
    let server: std::net::SocketAddr = server.parse()?;
    match manato_net::discovery::stun_public_address(server, Duration::from_secs(2))? {
        Some(address) => println!("public address: {address}"),
        None => println!("STUN server {server} did not answer with a mapped address"),
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let mut opts = Options::parse()?;

    if let Some(target) = opts.matchmake.clone() {
        let target: std::net::SocketAddr = target.parse()?;
        let lobbies = manato_net::discovery::Matchmaker::discover(target, Duration::from_secs(1))?;
        if lobbies.is_empty() {
            println!("no lobbies found at {target}");
            return Ok(());
        }
        for lobby in &lobbies {
            println!(
                "{} @ {} players={} spectators={} phase={:?}",
                lobby.name, lobby.address, lobby.players, lobby.spectators, lobby.phase
            );
        }
        match manato_net::discovery::best_lobby(&lobbies) {
            Some(best) => {
                println!("joining {} @ {}", best.name, best.address);
                if opts.spectate.is_none() {
                    opts.spectate = Some(best.address.to_string());
                }
            }
            None => {
                println!("no joinable lobbies");
                return Ok(());
            }
        }
    }

    if let Some(target) = &opts.matchmake_play {
        return run_matchmake_play(target, &opts);
    }

    if let Some(target) = &opts.discover {
        return run_discovery(target);
    }
    if let Some(server) = &opts.stun {
        return run_stun(server);
    }

    if let Some(frames) = opts.frames {
        return run_headless(&opts, frames);
    }
    if let Some(frames) = opts.synctest {
        return run_synctest(&opts, frames);
    }
    run_window(opts)
}

fn load_match(opts: &Options) -> anyhow::Result<SimState> {
    let config = opts.config();
    let assets = Assets::load(&config)
        .with_context(|| format!("failed to load content from {}", opts.root.display()))?;
    Ok(SimState::new(
        Arc::new(assets),
        MatchOptions {
            rounds_to_win: opts.rounds,
            round_time: opts.round_time,
            ai_level: opts.ai_level,
            ai_sides: [opts.ai_level > 0; 2],
        },
    ))
}

fn headless_inputs(sim: &SimState, ai_level: i32) -> [PlayerInput; 2] {
    if ai_level > 0 {
        [
            ai::input_for(sim, 0, ai_level),
            ai::input_for(sim, 1, ai_level),
        ]
    } else {
        [PlayerInput::default(); 2]
    }
}

fn run_headless(opts: &Options, frames: u32) -> anyhow::Result<()> {
    if let Some(storyboard) = &opts.storyboard {
        return run_storyboard_headless(opts, frames, storyboard);
    }
    if opts.menu {
        return run_frontend_headless(opts, frames);
    }
    let mut sim = load_match(opts)?;
    let mut renderer = Renderer::new_offscreen()?;
    renderer.load_assets(&sim);

    for _ in 0..frames {
        sim.tick(headless_inputs(&sim, opts.ai_level));
    }
    renderer.render(&sim);
    if let Some(path) = &opts.screenshot {
        renderer.screenshot(path)?;
        println!("screenshot written to {}", path.display());
    }
    println!(
        "frame={} phase={:?} life={}/{} checksum={:016x}",
        sim.frame,
        sim.round.phase,
        sim.fighters[0].life,
        sim.fighters[1].life,
        sim.checksum()
    );
    Ok(())
}

fn run_storyboard_headless(
    opts: &Options,
    frames: u32,
    storyboard_path: &str,
) -> anyhow::Result<()> {
    let path = if Path::new(storyboard_path).is_absolute() {
        PathBuf::from(storyboard_path)
    } else {
        opts.root.join(storyboard_path)
    };
    let storyboard = manato_formats::storyboard::Storyboard::load(&path)
        .with_context(|| format!("failed to load storyboard {}", path.display()))?;
    let mut player = manato_formats::storyboard::StoryboardPlayer::new(&storyboard);
    let mut renderer = Renderer::new_offscreen()?;
    renderer.load_storyboard(&storyboard);
    let audio = AudioEngine::new();
    let mut music_scene = usize::MAX;
    for _ in 0..frames {
        player.tick(&storyboard);
        for (group, index, volume, pan) in player.take_sounds() {
            audio.play_bank(
                &storyboard.sounds,
                group,
                index,
                (volume / 100.0).clamp(0.0, 1.0),
                pan.clamp(-1.0, 1.0),
            );
        }
        if player.scene != music_scene {
            music_scene = player.scene;
            match storyboard
                .scenes
                .get(player.scene)
                .and_then(|scene| scene.bgm.as_ref())
            {
                Some(bgm) => {
                    match manato_audio::Music::load(&bgm.path, bgm.loop_start, bgm.loop_end) {
                        Ok(music) => audio.play_music_with(
                            &music,
                            (bgm.volume as f32 / 100.0).clamp(0.0, 1.0),
                            bgm.loop_music,
                            0,
                            bgm.freq_mul,
                            bgm.loop_count,
                        ),
                        Err(error) => {
                            log::warn!("storyboard BGM {} failed: {error:#}", bgm.path.display())
                        }
                    }
                }
                None => audio.stop_music(),
            }
        }
    }
    renderer.render_storyboard(&storyboard, &player);
    if let Some(path) = &opts.screenshot {
        renderer.screenshot(path)?;
        println!("screenshot written to {}", path.display());
    }
    println!(
        "storyboard={} scene={} time={} done={}",
        storyboard.name, player.scene, player.time, player.done
    );
    Ok(())
}

fn run_frontend_headless(opts: &Options, frames: u32) -> anyhow::Result<()> {
    let mut frontend = Frontend::load(opts)?;
    frontend.frame = frames;
    match opts.menu_screen.as_str() {
        "select" => {
            frontend.screen = FrontendScreen::Select;
            frontend.p1 = 0;
            frontend.p2 = 1.min(frontend.roster.len().saturating_sub(1));
        }
        "options" => frontend.screen = FrontendScreen::Options,
        _ => frontend.screen = FrontendScreen::Title,
    }
    for time in 0..frames as i32 {
        match frontend.screen {
            FrontendScreen::Select => {
                if let Some(bg) = &frontend.screenpack.select_bg {
                    manato_sim::update_bg_states(
                        &bg.bg_ctrls,
                        &mut frontend.select_layers,
                        time,
                        1,
                        0,
                    );
                }
            }
            FrontendScreen::Title | FrontendScreen::Options | FrontendScreen::Scores => {
                if let Some(bg) = &frontend.screenpack.title_bg {
                    manato_sim::update_bg_states(
                        &bg.bg_ctrls,
                        &mut frontend.title_layers,
                        time,
                        1,
                        0,
                    );
                }
            }
            FrontendScreen::Fight | FrontendScreen::Continue => {}
        }
    }
    let mut renderer = Renderer::new_offscreen()?;
    renderer.load_frontend(Some(&frontend.screenpack), &frontend.faces);
    match frontend.screen {
        FrontendScreen::Select => {
            renderer.render_select(
                &frontend.screenpack,
                &frontend.select_layers,
                &frontend.select_view(),
            );
        }
        FrontendScreen::Options => {
            renderer.render_options(
                &frontend.screenpack,
                &frontend.title_layers,
                &frontend.options_view(),
            );
        }
        _ => renderer.render_title(
            &frontend.screenpack,
            &frontend.title_layers,
            &frontend.title_view(),
        ),
    }
    if let Some(path) = &opts.screenshot {
        renderer.screenshot(path)?;
        println!("screenshot written to {}", path.display());
    }
    println!(
        "frontend screen={:?} frame={}",
        frontend.screen, frontend.frame
    );
    Ok(())
}

fn run_synctest(opts: &Options, frames: u32) -> anyhow::Result<()> {
    let sim = load_match(opts)?;
    let mut session = manato_net::SyncTest::new(sim, opts.ai_level.max(6), 2)?;
    for _ in 0..frames {
        let input = ai::input_for(session.sim(), 0, opts.ai_level.max(6));
        session.step(input)?;
    }
    println!(
        "synctest ok: frames={} checksum={:016x} life={}/{}",
        frames,
        session.sim().checksum(),
        session.sim().fighters[0].life,
        session.sim().fighters[1].life
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Arcade,
    Versus,
    Training,
    Survival,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrontendScreen {
    Title,
    Select,
    Options,
    Fight,
    Continue,
    Scores,
}

/// Maximum selectable partners per side from the options screen.
const MAX_PARTNERS: usize = 3;

struct Frontend {
    screen: FrontendScreen,
    screenpack: Screenpack,
    roster: Vec<RosterEntry>,
    /// Character sprite files aligned with `roster`, used for select portraits.
    faces: Vec<SffFile>,
    names: Vec<SelectEntry>,
    select_timer: Option<i32>,
    stages: Vec<String>,
    stage_index: usize,
    menu: Vec<(String, String)>,
    menu_index: usize,
    mode: Mode,
    p1: usize,
    p2: usize,
    p1_done: bool,
    p2_done: bool,
    page: i32,
    frame: u32,
    options_index: usize,
    options_rounds: u32,
    options_round_time: i32,
    options_ai: i32,
    options_sound: u32,
    options_music: u32,
    options_fullscreen: bool,
    team_modes: [manato_sim::TeamMode; 2],
    partners: [[Option<usize>; MAX_PARTNERS]; 2],
    result_timer: i32,
    result_resolved: bool,
    arcade: Option<ArcadeSession>,
    arcade_special: Option<String>,
    arcade_final_boss: Option<String>,
    arcade_order: ArcadeOrderMode,
    scores: Vec<u64>,
    title_layers: Vec<manato_sim::StageLayerState>,
    select_layers: Vec<manato_sim::StageLayerState>,
}

const MENU_ORDER: [&str; 6] = [
    "arcade", "survival", "versus", "training", "options", "quit",
];

fn fallback_screenpack() -> Screenpack {
    use manato_core::Vec2;
    use manato_formats::screenpack::{MenuItemDef, SelectDef, TextDef, TitleDef};

    let labels = [
        ("arcade", "ARCADE"),
        ("survival", "SURVIVAL"),
        ("versus", "VERSUS"),
        ("training", "TRAINING"),
        ("scores", "SCORES"),
        ("options", "OPTIONS"),
        ("quit", "QUIT"),
    ];
    let title = TitleDef {
        menu_pos: Vec2::new(160.0, 108.0),
        item_spacing: Vec2::new(0.0, 14.0),
        item_scale: Vec2::new(1.0, 1.0),
        active_scale: Vec2::new(1.0, 1.0),
        item_color: [220, 224, 236, 255],
        active_color: [255, 214, 92, 255],
        items: labels
            .iter()
            .map(|(key, label)| MenuItemDef {
                key: (*key).into(),
                label: (*label).into(),
            })
            .collect(),
        ..TitleDef::default()
    };
    let name = TextDef {
        align: 1,
        offset: Vec2::new(40.0, 8.0),
        ..TextDef::default()
    };
    let select = SelectDef {
        rows: 2,
        columns: 3,
        pos: Vec2::new(24.0, 84.0),
        cell_size: Vec2::new(80.0, 24.0),
        cell_spacing: Vec2::new(8.0, 10.0),
        cell_name: name,
        ..SelectDef::default()
    };
    Screenpack {
        name: "Built-in".into(),
        localcoord: (320.0, 240.0),
        title,
        select,
        ..Screenpack::default()
    }
}

impl Frontend {
    fn load(opts: &Options) -> anyhow::Result<Self> {
        let explicit = match opts.screenpack.as_deref() {
            Some(value) if value.eq_ignore_ascii_case("none") => Some(None),
            Some(value) => Some(Some(value)),
            None => None,
        };
        let screenpack = match explicit {
            Some(Some(path)) => {
                let path = if Path::new(path).is_absolute() {
                    PathBuf::from(path)
                } else {
                    opts.root.join(path)
                };
                Screenpack::load(&path)
                    .with_context(|| format!("failed to load screenpack {}", path.display()))?
            }
            Some(None) => fallback_screenpack(),
            None => {
                // `--menu` uses the bundled screenpack by default; other
                // content roots fall back to the built-in text screens.
                let bundled = opts.root.join("content/screenpacks/manato-grid/system.def");
                if bundled.exists() {
                    Screenpack::load(&bundled).with_context(|| {
                        format!("failed to load screenpack {}", bundled.display())
                    })?
                } else {
                    fallback_screenpack()
                }
            }
        };
        let roster = match &screenpack.select_def_path {
            Some(path) => Roster::load(path).unwrap_or_else(|error| {
                log::warn!("could not load {}: {error:#}", path.display());
                Roster::default()
            }),
            None => Roster {
                entries: [opts.p1.clone(), opts.p2.clone()]
                    .into_iter()
                    .map(|def| RosterEntry {
                        def,
                        stage: None,
                        music: None,
                        random: false,
                        single: false,
                    })
                    .collect(),
                stages: Vec::new(),
                arcade: Default::default(),
            },
        };
        let names = roster
            .entries
            .iter()
            .map(|entry| {
                if entry.random {
                    SelectEntry {
                        name: "RANDOM".into(),
                        display_name: "RANDOM SELECT".into(),
                        random: true,
                    }
                } else {
                    let def = if Path::new(&entry.def).is_absolute() {
                        PathBuf::from(&entry.def)
                    } else {
                        opts.root.join(&entry.def)
                    };
                    let stem = def
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "UNKNOWN".into());
                    let display_name = manato_formats::CharacterDef::load(&def)
                        .map(|def| def.display_name)
                        .unwrap_or_else(|_| stem.clone());
                    let name = stem;
                    SelectEntry {
                        name: name.to_uppercase(),
                        display_name: display_name.to_uppercase(),
                        random: false,
                    }
                }
            })
            .collect();
        let faces = roster
            .entries
            .iter()
            .map(|entry| {
                if entry.random {
                    return SffFile::default();
                }
                let def = if Path::new(&entry.def).is_absolute() {
                    PathBuf::from(&entry.def)
                } else {
                    opts.root.join(&entry.def)
                };
                let Ok(character) = manato_formats::CharacterDef::load(&def) else {
                    return SffFile::default();
                };
                character
                    .files
                    .sprite
                    .as_ref()
                    .and_then(|sprite| SffFile::load(sprite).ok())
                    .unwrap_or_default()
            })
            .collect();
        let select_timer =
            (screenpack.select.timer_count > 0).then(|| screenpack.select.timer_count.max(1) * 60);
        let menu = MENU_ORDER
            .iter()
            .filter_map(|key| {
                screenpack
                    .title
                    .items
                    .iter()
                    .find(|item| item.key == *key)
                    .map(|item| (item.key.clone(), item.label.clone()))
            })
            .collect();
        let arcade_special = roster.arcade.special.clone();
        let arcade_final_boss = roster.arcade.final_boss.clone();
        let arcade_order = roster.arcade.order;
        let mut stages = roster.stages.clone();
        if !stages.iter().any(|stage| stage == &opts.stage) {
            stages.push(opts.stage.clone());
        }
        let p2_start = if roster.entries.len() > 1 { 1 } else { 0 };
        let title_layers = screen_layer_states(&screenpack.title_bg);
        let select_layers = screen_layer_states(&screenpack.select_bg);
        Ok(Self {
            screen: FrontendScreen::Title,
            screenpack,
            roster: roster.entries,
            faces,
            names,
            select_timer,
            stages,
            stage_index: 0,
            menu,
            menu_index: 0,
            mode: Mode::Arcade,
            p1: 0,
            p2: p2_start,
            p1_done: false,
            p2_done: false,
            page: 0,
            frame: 0,
            options_index: 0,
            options_rounds: opts.rounds,
            options_round_time: opts.round_time,
            options_ai: opts.ai_level,
            options_sound: 100,
            options_music: 100,
            options_fullscreen: false,
            team_modes: [manato_sim::TeamMode::Single; 2],
            partners: [
                [Some(p2_start), Some(0), Some(0)],
                [Some(0), Some(p2_start), Some(0)],
            ],
            result_timer: 0,
            result_resolved: false,
            arcade: None,
            arcade_special,
            arcade_final_boss,
            arcade_order,
            scores: manato_sim::arcade::load_score_table(&opts.root.join("manato_highscore.dat")),
            title_layers,
            select_layers,
        })
    }

    fn title_view(&self) -> TitleView {
        TitleView {
            selected: self.menu_index,
            frame: self.frame,
        }
    }

    fn select_view(&self) -> SelectView {
        SelectView {
            p1: self.p1,
            p2: self.p2,
            page: self.page,
            p1_done: self.p1_done,
            p2_done: self.p2_done,
            frame: self.frame,
            timer: self.select_timer,
            names: self.names.clone(),
        }
    }

    fn options_view(&self) -> OptionsView {
        let yes_no = |value: bool| if value { "YES" } else { "NO" }.to_string();
        let mut items = vec![
            ("ROUNDS".into(), self.options_rounds.to_string()),
            ("ROUND TIME".into(), self.options_round_time.to_string()),
            ("CPU LEVEL".into(), self.options_ai.to_string()),
            ("SOUND VOLUME".into(), self.options_sound.to_string()),
            ("MUSIC VOLUME".into(), self.options_music.to_string()),
            ("FULLSCREEN".into(), yes_no(self.options_fullscreen)),
            ("P1 TEAM".into(), team_name(self.team_modes[0])),
            ("P2 TEAM".into(), team_name(self.team_modes[1])),
        ];
        for (side, label) in ["P1", "P2"].into_iter().enumerate() {
            for slot in 0..MAX_PARTNERS {
                let suffix = if slot == 0 {
                    String::new()
                } else {
                    format!(" {}", slot + 1)
                };
                let value = self
                    .names
                    .get(self.partners[side][slot].unwrap_or(0))
                    .map(|entry| entry.name.clone())
                    .unwrap_or_else(|| "--".into());
                items.push((format!("{label} PARTNER{suffix}"), value));
            }
        }
        items.push(("BACK".into(), String::new()));
        OptionsView {
            selected: self.options_index,
            frame: self.frame,
            items,
        }
    }

    fn per_page(&self) -> usize {
        (self.screenpack.select.rows.max(1) * self.screenpack.select.columns.max(1)) as usize
    }

    fn move_menu(&mut self, delta: i32) {
        let count = self.menu.len();
        if count == 0 {
            return;
        }
        let next = self.menu_index as i32 + delta;
        self.menu_index = next.rem_euclid(count as i32) as usize;
    }

    fn cycle_stage(&mut self, delta: i32) {
        if self.stages.is_empty() {
            return;
        }
        let next = self.stage_index as i32 + delta;
        self.stage_index = next.rem_euclid(self.stages.len() as i32) as usize;
    }

    fn cycle_team(&mut self, side: usize, delta: i32) {
        if side >= 2 {
            return;
        }
        let modes = [
            manato_sim::TeamMode::Single,
            manato_sim::TeamMode::Turns,
            manato_sim::TeamMode::Tag,
            manato_sim::TeamMode::Simul,
        ];
        let current = modes
            .iter()
            .position(|mode| *mode == self.team_modes[side])
            .unwrap_or(0) as i32;
        let next = (current + delta).rem_euclid(modes.len() as i32);
        self.team_modes[side] = modes[next as usize];
    }

    fn cycle_partner(&mut self, side: usize, slot: usize, delta: i32) {
        if side >= 2 || slot >= MAX_PARTNERS || self.roster.is_empty() {
            return;
        }
        let len = self.roster.len() as i32;
        let mut next = self.partners[side][slot].unwrap_or(0) as i32;
        // Single-mode-only characters cannot be team partners.
        for _ in 0..len {
            next = (next + delta).rem_euclid(len);
            if !self.roster[next as usize].single {
                break;
            }
        }
        self.partners[side][slot] = Some(next as usize);
    }

    /// Team modes after applying select.def single-only rules for the chosen
    /// leaders. A single-only leader forces Single mode for its side.
    fn effective_team_modes(&self) -> [manato_sim::TeamMode; 2] {
        let mut modes = self.team_modes;
        for (side, mode) in modes.iter_mut().enumerate() {
            let cell = if side == 0 { self.p1 } else { self.p2 };
            if chosen_entry(self, cell).is_some_and(|entry| entry.single) {
                *mode = manato_sim::TeamMode::Single;
            }
        }
        modes
    }

    fn move_cursor(&mut self, side: usize, dx: i32, dy: i32) {
        let columns = self.screenpack.select.columns.max(1);
        let count = self.roster.len().max(1) as i32;
        let mut cell = if side == 0 { self.p1 } else { self.p2 } as i32;
        let mut column = cell % columns;
        let mut row = cell / columns;
        if self.screenpack.select.wrapping {
            column = (column + dx).rem_euclid(columns);
            row += dy;
        } else {
            let next = column + dx;
            if (0..columns).contains(&next) {
                column = next;
            }
            row = (row + dy).max(0);
        }
        cell = (row * columns + column).clamp(0, count - 1);
        if side == 0 {
            self.p1 = cell as usize;
        } else {
            self.p2 = cell as usize;
        }
        let page_size = self.per_page() as i32;
        self.page = cell / page_size;
    }
}

fn screen_layer_states(
    bg: &Option<manato_formats::screenpack::ScreenBg>,
) -> Vec<manato_sim::StageLayerState> {
    bg.as_ref()
        .map(|bg| {
            bg.layers
                .iter()
                .map(manato_sim::StageLayerState::from_layer)
                .collect()
        })
        .unwrap_or_default()
}

fn team_name(mode: manato_sim::TeamMode) -> String {
    match mode {
        manato_sim::TeamMode::Single => "SINGLE",
        manato_sim::TeamMode::Turns => "TURNS",
        manato_sim::TeamMode::Tag => "TAG",
        manato_sim::TeamMode::Simul => "SIMUL",
    }
    .into()
}

fn roster_index_by_def(roster: &[RosterEntry], def: &str) -> Option<usize> {
    let normalize = |value: &str| {
        value
            .trim()
            .trim_matches('"')
            .replace('\\', "/")
            .to_ascii_lowercase()
    };
    let wanted = normalize(def);
    roster
        .iter()
        .position(|entry| !entry.random && normalize(&entry.def) == wanted)
}

fn chosen_entry(frontend: &Frontend, cell: usize) -> Option<RosterEntry> {
    let entry = frontend.roster.get(cell).cloned()?;
    if !entry.random {
        return Some(entry);
    }
    let choices: Vec<&RosterEntry> = frontend.roster.iter().filter(|e| !e.random).collect();
    if choices.is_empty() {
        return Some(entry);
    }
    let pick = (frontend.frame as usize) % choices.len();
    Some(choices[pick].clone())
}

fn next_entry(frontend: &Frontend, cell: usize) -> Option<RosterEntry> {
    let len = frontend.roster.len();
    if len == 0 {
        return None;
    }
    for offset in 1..=len {
        let index = (cell + offset) % len;
        if !frontend.roster[index].random {
            return Some(frontend.roster[index].clone());
        }
    }
    None
}

/// Runs sandboxed storyboard Lua once per simulation tick and applies the
/// resulting overlay transform and scene jump to the simulation.
#[derive(Default)]
struct StoryboardScripts {
    runtimes: HashMap<usize, Option<manato_formats::storyboard_lua::StoryboardLua>>,
}

impl StoryboardScripts {
    fn tick(&mut self, overlays: &mut [manato_sim::StoryboardOverlay]) {
        let mut live = HashSet::new();
        for overlay in overlays {
            let key = Arc::as_ptr(&overlay.storyboard) as usize;
            live.insert(key);
            let Some(source) = overlay.storyboard.lua_source.as_deref() else {
                continue;
            };
            let runtime = self.runtimes.entry(key).or_insert_with(|| {
                match manato_formats::storyboard_lua::StoryboardLua::load(source, "storyboard.lua")
                {
                    Ok(script) => Some(script),
                    Err(error) => {
                        log::warn!("storyboard lua failed: {error:#}");
                        None
                    }
                }
            });
            let Some(script) = runtime else {
                continue;
            };
            let mut ctx = manato_formats::storyboard_lua::StoryboardScriptContext::new(
                overlay.player.scene,
                overlay.player.time,
                overlay.pos.x,
                overlay.pos.y,
            );
            ctx.scale_x = overlay.scale.x;
            ctx.scale_y = overlay.scale.y;
            ctx.facing = overlay.facing;
            ctx.alpha = overlay.script_alpha;
            match script.update(&mut ctx) {
                Ok(()) => {
                    overlay.pos = manato_core::Vec2::new(ctx.x, ctx.y);
                    overlay.scale = manato_core::Vec2::new(ctx.scale_x, ctx.scale_y);
                    overlay.facing = ctx.facing;
                    overlay.script_alpha = ctx.alpha.clamp(0.0, 1.0);
                    if ctx.done {
                        overlay.player.done = true;
                    }
                    if let Some(jump) = ctx.jump {
                        if !overlay.player.jump_to(&overlay.storyboard, &jump) {
                            log::warn!("storyboard lua jump to unknown scene {jump}");
                        }
                    }
                }
                Err(error) => log::warn!("storyboard lua update failed: {error:#}"),
            }
        }
        self.runtimes.retain(|key, _| live.contains(key));
    }
}

struct App {
    opts: Options,
    sim: Option<SimState>,
    frontend: Option<Frontend>,
    renderer: Option<Renderer>,
    audio: AudioEngine,
    instance: wgpu::Instance,
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    surface_config: Option<wgpu::SurfaceConfiguration>,
    keys: HashSet<KeyCode>,
    #[cfg(feature = "gamepad")]
    gamepads: Option<gilrs::Gilrs>,
    pad_nav: [bool; 6],
    last_tick: Instant,
    accumulator: Duration,
    music_override: Option<PathBuf>,
    spectator: Option<manato_net::lobby::SpectatorClient>,
    broadcaster: Option<manato_net::lobby::SnapshotBroadcaster>,
    storyboard_scripts: StoryboardScripts,
}

fn run_window(opts: Options) -> anyhow::Result<()> {
    let sim = if opts.menu {
        None
    } else {
        Some(load_match(&opts)?)
    };
    let frontend = if opts.menu {
        Some(Frontend::load(&opts)?)
    } else {
        None
    };
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let spectator = opts
        .spectate
        .as_deref()
        .map(
            |addr| -> anyhow::Result<manato_net::lobby::SpectatorClient> {
                let client = manato_net::lobby::SpectatorClient::connect(addr.parse()?)?;
                log::info!("spectating host {addr} from {}", client.local_addr()?);
                Ok(client)
            },
        )
        .transpose()?;
    let broadcaster = opts
        .broadcast
        .as_deref()
        .map(
            |addr| -> anyhow::Result<manato_net::lobby::SnapshotBroadcaster> {
                let broadcaster = manato_net::lobby::SnapshotBroadcaster::bind(addr.parse()?)?;
                log::info!("broadcasting at {}", broadcaster.local_addr()?);
                Ok(broadcaster)
            },
        )
        .transpose()?;
    let audio = AudioEngine::new();
    log::info!(
        "audio backend: {} ({} Hz)",
        audio.backend_name(),
        audio.sample_rate()
    );
    #[cfg(feature = "gamepad")]
    let gamepads = match gilrs::Gilrs::new() {
        Ok(gilrs) => {
            let count = gilrs.gamepads().count();
            if count > 0 {
                log::info!("gamepad support: {count} controller(s)");
            }
            Some(gilrs)
        }
        Err(error) => {
            log::warn!("gamepad support unavailable: {error}");
            None
        }
    };
    let mut app = App {
        opts,
        sim,
        frontend,
        renderer: None,
        audio,
        instance,
        window: None,
        surface: None,
        surface_config: None,
        keys: HashSet::new(),
        #[cfg(feature = "gamepad")]
        gamepads,
        pad_nav: [false; 6],
        last_tick: Instant::now(),
        accumulator: Duration::ZERO,
        music_override: None,
        spectator,
        broadcaster,
        storyboard_scripts: StoryboardScripts::default(),
    };
    if let Some(sim) = app.sim.as_ref() {
        app.play_stage_music(sim);
    }
    event_loop.run_app(&mut app)?;
    Ok(())
}

impl App {
    fn play_stage_music(&self, sim: &SimState) {
        let stage = &sim.assets.stage;
        let path = self.music_override.as_ref().or(stage.music.path.as_ref());
        let Some(path) = path else {
            return;
        };
        match manato_audio::Music::load(path, stage.music.loop_start, stage.music.loop_end) {
            Ok(music) => {
                let volume = (stage.music.volume as f32 / 100.0
                    * stage.music.ratio.clamp(0.0, 1.0))
                .clamp(0.0, 1.0);
                self.audio.play_music(&music, volume);
                log::info!("playing stage music {}", path.display());
            }
            Err(error) => log::warn!("could not load stage music {}: {error:#}", path.display()),
        }
    }

    fn high_score_path(&self) -> PathBuf {
        self.opts.root.join("manato_highscore.dat")
    }

    fn start_match(&mut self) {
        let high_score_path = self.high_score_path();
        if let Some(frontend) = self.frontend.as_mut() {
            let single_player = matches!(frontend.mode, Mode::Arcade | Mode::Survival);
            let needs_session = single_player
                && frontend.arcade.as_ref().is_none_or(|session| {
                    matches!(session.phase, ArcadePhase::Complete | ArcadePhase::GameOver)
                });
            if needs_session {
                let p1 = frontend.p1;
                let random_flags: Vec<bool> =
                    frontend.roster.iter().map(|entry| entry.random).collect();
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_nanos() as u64)
                    .unwrap_or(1);
                let special = frontend
                    .arcade_special
                    .as_deref()
                    .and_then(|def| roster_index_by_def(&frontend.roster, def));
                let boss = frontend
                    .arcade_final_boss
                    .as_deref()
                    .and_then(|def| roster_index_by_def(&frontend.roster, def));
                let shuffle = frontend.arcade_order == ArcadeOrderMode::Random;
                let ladder = manato_sim::arcade::build_ladder_ordered(
                    &random_flags,
                    p1,
                    seed,
                    special,
                    boss,
                    shuffle,
                );
                let survival = frontend.mode == Mode::Survival;
                let config = ArcadeConfig {
                    credits: if survival { 1 } else { 3 },
                    survival,
                    high_score: manato_sim::arcade::load_score_table(&high_score_path)
                        .first()
                        .copied()
                        .unwrap_or(0),
                    ..ArcadeConfig::default()
                };
                frontend.arcade = Some(ArcadeSession::new(ladder, config));
            }
        }
        let Some(frontend) = self.frontend.as_ref() else {
            return;
        };
        let Some(p1) = chosen_entry(frontend, frontend.p1) else {
            log::error!("roster is empty");
            return;
        };
        let p2 = if frontend.mode == Mode::Versus {
            chosen_entry(frontend, frontend.p2).unwrap_or_else(|| p1.clone())
        } else if matches!(frontend.mode, Mode::Arcade | Mode::Survival) {
            frontend
                .arcade
                .as_ref()
                .and_then(|session| session.current_opponent())
                .and_then(|index| chosen_entry(frontend, index))
                .unwrap_or_else(|| next_entry(frontend, frontend.p1).unwrap_or_else(|| p1.clone()))
        } else {
            next_entry(frontend, frontend.p1).unwrap_or_else(|| p1.clone())
        };
        let chosen_stage = frontend
            .stages
            .get(frontend.stage_index)
            .cloned()
            .unwrap_or_else(|| self.opts.stage.clone());
        let single_player = matches!(frontend.mode, Mode::Arcade | Mode::Survival);
        let music_override = if single_player {
            p2.music.as_ref().map(|music| {
                if Path::new(music).is_absolute() {
                    PathBuf::from(music)
                } else {
                    self.opts.root.join(music)
                }
            })
        } else {
            None
        };
        let stage = if single_player {
            p2.stage
                .clone()
                .or_else(|| p1.stage.clone())
                .unwrap_or(chosen_stage)
        } else {
            p1.stage
                .clone()
                .or_else(|| p2.stage.clone())
                .unwrap_or(chosen_stage)
        };
        let mode = frontend.mode;
        let rounds = frontend.options_rounds;
        let round_time = frontend.options_round_time;
        let ai = frontend.options_ai;
        let screenpack = self.opts.screenpack.as_ref().map(PathBuf::from);
        let config = MatchConfig {
            mugen_root: self.opts.root.clone(),
            p1: PathBuf::from(&p1.def),
            p2: PathBuf::from(&p2.def),
            stage: PathBuf::from(stage),
            screenpack,
            rounds_to_win: rounds,
            round_time,
        };
        let team_modes = frontend.effective_team_modes();
        let partner_entries: [Vec<Option<RosterEntry>>; 2] = [0, 1].map(|side| {
            frontend.partners[side]
                .iter()
                .map(|index| index.and_then(|index| chosen_entry(frontend, index)))
                .collect()
        });
        let mut assets = match Assets::load(&config) {
            Ok(assets) => assets,
            Err(error) => {
                log::error!("failed to start match: {error:#}");
                return;
            }
        };
        // Load selected partners as extra characters for team modes.
        let mut partner_ids: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
        for side in 0..2 {
            if team_modes[side] == manato_sim::TeamMode::Single {
                continue;
            }
            for entry in partner_entries[side].iter().flatten() {
                if entry.single {
                    continue;
                }
                let path = if Path::new(&entry.def).is_absolute() {
                    PathBuf::from(&entry.def)
                } else {
                    self.opts.root.join(&entry.def)
                };
                match manato_formats::Character::load(&path, &assets.data_dirs) {
                    Ok(character) => {
                        partner_ids[side].push(assets.characters.len());
                        assets.characters.push(character);
                    }
                    Err(error) => {
                        log::warn!("failed to load partner {}: {error:#}", path.display());
                    }
                }
            }
        }
        let mut sim = SimState::new(
            Arc::new(assets),
            MatchOptions {
                rounds_to_win: rounds,
                round_time,
                ai_level: if mode == Mode::Versus { 0 } else { ai.max(1) },
                ai_sides: [false, mode != Mode::Versus],
            },
        );
        if team_modes != [manato_sim::TeamMode::Single; 2] {
            sim.team_modes = team_modes;
            for (side, partners) in partner_ids.iter().enumerate() {
                if partners.is_empty() {
                    continue;
                }
                let mut roster = vec![sim.fighters[side].char_id];
                roster.extend_from_slice(partners);
                sim.assign_team(side, &roster);
            }
            sim.restart_match();
        }
        self.music_override = music_override;
        self.play_stage_music(&sim);
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.load_assets(&sim);
        }
        self.sim = Some(sim);
        if let Some(frontend) = self.frontend.as_mut() {
            frontend.screen = FrontendScreen::Fight;
            frontend.result_timer = 0;
            frontend.result_resolved = false;
        }
    }

    fn return_to_title(&mut self) {
        let high_score_path = self.high_score_path();
        self.sim = None;
        self.audio.stop_music();
        if let Some(frontend) = self.frontend.as_mut() {
            frontend.screen = FrontendScreen::Title;
            frontend.p1_done = false;
            frontend.p2_done = false;
            frontend.result_timer = 0;
            frontend.result_resolved = false;
            if let Some(session) = &frontend.arcade {
                manato_sim::arcade::insert_score(&mut frontend.scores, session.high_score);
                let _ = manato_sim::arcade::save_score_table(&high_score_path, &frontend.scores);
            }
            frontend.arcade = None;
            frontend.frame = 0;
        }
        if let (Some(renderer), Some(frontend)) = (self.renderer.as_mut(), self.frontend.as_ref()) {
            renderer.load_frontend(Some(&frontend.screenpack), &frontend.faces);
        }
    }

    fn activate_menu(&mut self, event_loop: Option<&ActiveEventLoop>) {
        let Some(frontend) = self.frontend.as_ref() else {
            return;
        };
        let Some((key, _)) = frontend.menu.get(frontend.menu_index).cloned() else {
            return;
        };
        let first = 0;
        let second = if frontend.roster.len() > 1 { 1 } else { 0 };
        match key.as_str() {
            "arcade" | "survival" | "versus" | "training" => {
                if let Some(frontend) = self.frontend.as_mut() {
                    frontend.mode = match key.as_str() {
                        "versus" => Mode::Versus,
                        "training" => Mode::Training,
                        "survival" => Mode::Survival,
                        _ => Mode::Arcade,
                    };
                    frontend.screen = FrontendScreen::Select;
                    frontend.p1 = first;
                    frontend.p2 = second;
                    frontend.p1_done = false;
                    frontend.p2_done = false;
                    frontend.page = 0;
                    frontend.select_timer = (frontend.screenpack.select.timer_count > 0)
                        .then(|| frontend.screenpack.select.timer_count.max(1) * 60);
                }
            }
            "scores" => {
                let path = self.high_score_path();
                if let Some(frontend) = self.frontend.as_mut() {
                    frontend.scores = manato_sim::arcade::load_score_table(&path);
                    frontend.screen = FrontendScreen::Scores;
                }
            }
            "options" => {
                if let Some(frontend) = self.frontend.as_mut() {
                    frontend.screen = FrontendScreen::Options;
                    frontend.options_index = 0;
                }
            }
            "quit" => {
                if let Some(event_loop) = event_loop {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn confirm_side(&mut self, side: usize) {
        let Some(frontend) = self.frontend.as_ref() else {
            return;
        };
        if frontend.mode != Mode::Versus {
            self.start_match();
            return;
        }
        if let Some(frontend) = self.frontend.as_mut() {
            if side == 0 {
                frontend.p1_done = true;
            } else {
                frontend.p2_done = true;
            }
        }
        if self
            .frontend
            .as_ref()
            .is_some_and(|f| f.p1_done && f.p2_done)
        {
            self.start_match();
        }
    }

    fn adjust_option(&mut self, delta: i32) {
        let Some(frontend) = self.frontend.as_mut() else {
            return;
        };
        match frontend.options_index {
            0 => {
                frontend.options_rounds =
                    (frontend.options_rounds as i32 + delta).clamp(1, 5) as u32
            }
            1 => {
                frontend.options_round_time =
                    (frontend.options_round_time + delta * 10).clamp(30, 99)
            }
            2 => frontend.options_ai = (frontend.options_ai + delta).clamp(0, 8),
            3 => {
                frontend.options_sound =
                    (frontend.options_sound as i32 + delta * 10).clamp(0, 100) as u32
            }
            4 => {
                frontend.options_music =
                    (frontend.options_music as i32 + delta * 10).clamp(0, 100) as u32
            }
            5 => frontend.options_fullscreen = !frontend.options_fullscreen,
            6 => frontend.cycle_team(0, delta),
            7 => frontend.cycle_team(1, delta),
            index @ 8..=13 => {
                let offset = index - 8;
                frontend.cycle_partner(offset / MAX_PARTNERS, offset % MAX_PARTNERS, delta);
            }
            _ => {}
        }
        if frontend.options_index == 3 {
            self.audio.set_volume(frontend.options_sound as f32 / 100.0);
        }
    }

    fn frontend_key(&mut self, code: KeyCode, event_loop: Option<&ActiveEventLoop>) {
        let Some(screen) = self.frontend.as_ref().map(|f| f.screen) else {
            return;
        };
        match screen {
            FrontendScreen::Title => match code {
                KeyCode::KeyW | KeyCode::ArrowUp => {
                    if let Some(frontend) = self.frontend.as_mut() {
                        frontend.move_menu(-1);
                    }
                }
                KeyCode::KeyS | KeyCode::ArrowDown => {
                    if let Some(frontend) = self.frontend.as_mut() {
                        frontend.move_menu(1);
                    }
                }
                KeyCode::Enter | KeyCode::Space | KeyCode::KeyJ => self.activate_menu(event_loop),
                KeyCode::Escape => {
                    if let Some(event_loop) = event_loop {
                        event_loop.exit();
                    }
                }
                _ => {}
            },
            FrontendScreen::Select => {
                let versus = self
                    .frontend
                    .as_ref()
                    .is_some_and(|f| f.mode == Mode::Versus);
                if let Some(frontend) = self.frontend.as_mut() {
                    if code == KeyCode::Escape {
                        frontend.screen = FrontendScreen::Title;
                        return;
                    }
                }
                match code {
                    KeyCode::KeyQ | KeyCode::PageUp => {
                        if let Some(frontend) = self.frontend.as_mut() {
                            frontend.cycle_stage(-1);
                        }
                    }
                    KeyCode::KeyE | KeyCode::PageDown => {
                        if let Some(frontend) = self.frontend.as_mut() {
                            frontend.cycle_stage(1);
                        }
                    }
                    _ => {}
                }
                if versus {
                    match code {
                        KeyCode::KeyW => self.move_cursor(0, 0, -1),
                        KeyCode::KeyS => self.move_cursor(0, 0, 1),
                        KeyCode::KeyA => self.move_cursor(0, -1, 0),
                        KeyCode::KeyD => self.move_cursor(0, 1, 0),
                        KeyCode::Enter | KeyCode::KeyJ => self.confirm_side(0),
                        KeyCode::ArrowUp | KeyCode::Numpad8 => self.move_cursor(1, 0, -1),
                        KeyCode::ArrowDown | KeyCode::Numpad2 => self.move_cursor(1, 0, 1),
                        KeyCode::ArrowLeft | KeyCode::Numpad4 => self.move_cursor(1, -1, 0),
                        KeyCode::ArrowRight | KeyCode::Numpad6 => self.move_cursor(1, 1, 0),
                        KeyCode::Numpad1 => self.confirm_side(1),
                        _ => {}
                    }
                } else {
                    match code {
                        KeyCode::KeyW | KeyCode::ArrowUp => self.move_cursor(0, 0, -1),
                        KeyCode::KeyS | KeyCode::ArrowDown => self.move_cursor(0, 0, 1),
                        KeyCode::KeyA | KeyCode::ArrowLeft => self.move_cursor(0, -1, 0),
                        KeyCode::KeyD | KeyCode::ArrowRight => self.move_cursor(0, 1, 0),
                        KeyCode::Enter | KeyCode::KeyJ => self.confirm_side(0),
                        _ => {}
                    }
                }
            }
            FrontendScreen::Continue => match code {
                KeyCode::Enter | KeyCode::Space | KeyCode::KeyJ => {
                    let resumed = self
                        .frontend
                        .as_mut()
                        .and_then(|frontend| frontend.arcade.as_mut())
                        .is_some_and(|session| session.use_credit());
                    if resumed {
                        self.start_match();
                    }
                }
                KeyCode::Escape => self.return_to_title(),
                _ => {}
            },
            FrontendScreen::Scores => match code {
                KeyCode::Enter | KeyCode::Space | KeyCode::KeyJ | KeyCode::Escape => {
                    if let Some(frontend) = self.frontend.as_mut() {
                        frontend.screen = FrontendScreen::Title;
                    }
                }
                _ => {}
            },
            FrontendScreen::Options => {
                let count = self
                    .frontend
                    .as_ref()
                    .map(|f| f.options_view().items.len())
                    .unwrap_or(0);
                match code {
                    KeyCode::KeyW | KeyCode::ArrowUp => {
                        if let Some(frontend) = self.frontend.as_mut() {
                            frontend.options_index =
                                (frontend.options_index + count.saturating_sub(1)) % count.max(1);
                        }
                    }
                    KeyCode::KeyS | KeyCode::ArrowDown => {
                        if let Some(frontend) = self.frontend.as_mut() {
                            frontend.options_index = (frontend.options_index + 1) % count.max(1);
                        }
                    }
                    KeyCode::KeyA | KeyCode::ArrowLeft => self.adjust_option(-1),
                    KeyCode::KeyD | KeyCode::ArrowRight => self.adjust_option(1),
                    KeyCode::Enter | KeyCode::Space | KeyCode::KeyJ => {
                        if self
                            .frontend
                            .as_ref()
                            .is_some_and(|f| f.options_index == 8 + 2 * MAX_PARTNERS)
                        {
                            if let Some(frontend) = self.frontend.as_mut() {
                                frontend.screen = FrontendScreen::Title;
                            }
                        } else {
                            self.adjust_option(1)
                        }
                    }
                    KeyCode::Escape => {
                        if let Some(frontend) = self.frontend.as_mut() {
                            frontend.screen = FrontendScreen::Title;
                        }
                    }
                    _ => {}
                }
            }
            FrontendScreen::Fight => {}
        }
    }

    fn move_cursor(&mut self, side: usize, dx: i32, dy: i32) {
        if let Some(frontend) = self.frontend.as_mut() {
            frontend.move_cursor(side, dx, dy);
        }
    }

    fn apply_fullscreen(&self) {
        let Some(window) = &self.window else {
            return;
        };
        let fullscreen = self
            .frontend
            .as_ref()
            .is_some_and(|frontend| frontend.options_fullscreen);
        window.set_fullscreen(fullscreen.then(|| Fullscreen::Borderless(None)));
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let attrs = Window::default_attributes()
            .with_title("M.A.N.A.T.O.-RS")
            .with_inner_size(winit::dpi::LogicalSize::new(
                GAME_WIDTH * self.opts.scale,
                GAME_HEIGHT * self.opts.scale,
            ));
        let window = Arc::new(event_loop.create_window(attrs)?);
        let surface = self.instance.create_surface(window.clone())?;

        // Surface capabilities need an adapter that can present to this
        // surface. Request a probe adapter first, pick a non-sRGB format, then
        // let the renderer create its own device.
        let probe =
            pollster::block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            }))
            .context("no surface-compatible GPU adapter")?;
        let caps = surface.get_capabilities(&probe);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        let mut renderer = Renderer::new_for_surface(self.instance.clone(), &surface, format)?;
        if let Some(sim) = &self.sim {
            renderer.load_assets(sim);
        } else if let Some(frontend) = &self.frontend {
            renderer.load_frontend(Some(&frontend.screenpack), &frontend.faces);
        }
        surface.configure(&renderer.device, &config);
        log::info!(
            "window ready: {}x{} {:?}, {} atlas pages",
            config.width,
            config.height,
            format,
            renderer.atlas_pages()
        );
        self.renderer = Some(renderer);
        self.surface = Some(surface);
        self.surface_config = Some(config);
        self.window = Some(window);
        self.apply_fullscreen();
        Ok(())
    }

    #[cfg(feature = "gamepad")]
    fn poll_pad_menu(&mut self) {
        if self
            .frontend
            .as_ref()
            .is_some_and(|frontend| frontend.screen == FrontendScreen::Fight)
        {
            return;
        }
        let Some(gilrs) = &self.gamepads else {
            return;
        };
        let Some((_, pad)) = gilrs.gamepads().next() else {
            return;
        };
        let current = pad_nav(pad_input(&pad));
        let previous = self.pad_nav;
        self.pad_nav = current;
        for key in nav_edges(previous, current) {
            self.frontend_key(key, None);
        }
    }

    #[cfg(not(feature = "gamepad"))]
    fn poll_pad_menu(&mut self) {}

    fn gamepad_inputs(&self) -> [PlayerInput; 2] {
        #[cfg(feature = "gamepad")]
        {
            let mut pads = [PlayerInput::default(); 2];
            let Some(gilrs) = &self.gamepads else {
                return pads;
            };
            for (index, (_, pad)) in gilrs.gamepads().enumerate().take(2) {
                pads[index] = pad_input(&pad);
            }
            pads
        }
        #[cfg(not(feature = "gamepad"))]
        {
            [PlayerInput::default(); 2]
        }
    }

    fn step(&mut self) {
        if let Some(spectator) = self.spectator.as_mut() {
            if let Some(sim) = self.sim.as_mut() {
                if let Err(error) = spectator.poll(sim) {
                    log::warn!("spectator receive failed: {error:#}");
                }
            }
            return;
        }
        self.poll_pad_menu();
        if let Some(frontend) = self.frontend.as_mut() {
            if frontend.screen == FrontendScreen::Continue {
                let ended = frontend.arcade.as_mut().is_none_or(|session| {
                    session.tick();
                    session.phase == ArcadePhase::GameOver
                });
                if ended {
                    self.return_to_title();
                    return;
                }
            }
            if frontend.screen != FrontendScreen::Fight {
                frontend.frame = frontend.frame.wrapping_add(1);
                let time = frontend.frame as i32;
                match frontend.screen {
                    FrontendScreen::Title | FrontendScreen::Options | FrontendScreen::Scores => {
                        if let Some(bg) = &frontend.screenpack.title_bg {
                            manato_sim::update_bg_states(
                                &bg.bg_ctrls,
                                &mut frontend.title_layers,
                                time,
                                1,
                                0,
                            );
                        }
                    }
                    FrontendScreen::Select => {
                        if let Some(bg) = &frontend.screenpack.select_bg {
                            manato_sim::update_bg_states(
                                &bg.bg_ctrls,
                                &mut frontend.select_layers,
                                time,
                                1,
                                0,
                            );
                        }
                    }
                    FrontendScreen::Fight | FrontendScreen::Continue => {}
                }
                if frontend.screen == FrontendScreen::Select {
                    if let Some(timer) = frontend.select_timer.as_mut() {
                        *timer = (*timer - 1).max(0);
                    }
                    if frontend.select_timer == Some(0) {
                        self.start_match();
                    }
                }
                return;
            }
        }
        let versus = self
            .frontend
            .as_ref()
            .is_some_and(|f| f.mode == Mode::Versus);
        let pads = self.gamepad_inputs();
        let p1 = merge_input(player_input(&self.keys, &self.opts.p1_keys), pads[0]);
        let Some(sim) = self.sim.as_mut() else {
            return;
        };
        let p2 = if self.frontend.is_some() && !versus {
            ai::input_for(sim, 1, self.opts.ai_level.max(1))
        } else if self.opts.ai_level > 0 && self.frontend.is_none() {
            ai::input_for(sim, 1, self.opts.ai_level)
        } else {
            merge_input(player_input(&self.keys, &self.opts.p2_keys), pads[1])
        };
        sim.tick([p1, p2]);
        self.storyboard_scripts.tick(&mut sim.storyboards);
        if let Some(broadcaster) = self.broadcaster.as_mut() {
            if let Err(error) = broadcaster.poll() {
                log::warn!("broadcast join poll failed: {error}");
            }
            if let Err(error) = broadcaster.broadcast(sim, sim.frame as i32) {
                log::warn!("broadcast failed: {error:#}");
            }
        }
        self.audio.play_events(sim);
        if sim.bgm.active {
            if let Some(status) = self.audio.bgm_status() {
                sim.bgm.position_ms = status.position_ms;
                sim.bgm.length_ms = status.length_ms;
                sim.bgm.sample_rate = status.sample_rate;
            }
        }
        let finished = sim.finished;
        let winner = sim.round.winner;
        let time_left = sim.round.timer;
        let life_left = sim.fighters[0].life;
        let perfect = sim.fighters[0].life >= sim.fighters[0].life_max;
        if finished {
            if let Some(frontend) = self.frontend.as_mut() {
                frontend.result_timer += 1;
            }
        }
        let resolve = finished
            && self.frontend.as_ref().is_some_and(|f| {
                f.mode != Mode::Training && f.result_timer >= 180 && !f.result_resolved
            });
        if resolve {
            let arcade = self
                .frontend
                .as_ref()
                .is_some_and(|f| matches!(f.mode, Mode::Arcade | Mode::Survival));
            if arcade {
                let phase = {
                    let frontend = self.frontend.as_mut().expect("frontend");
                    frontend.result_resolved = true;
                    let Some(session) = frontend.arcade.as_mut() else {
                        return;
                    };
                    if winner == Some(0) {
                        session.win(time_left, perfect, life_left);
                    } else {
                        session.lose();
                    }
                    session.phase
                };
                match phase {
                    ArcadePhase::Continue => {
                        self.sim = None;
                        if let Some(frontend) = self.frontend.as_mut() {
                            frontend.screen = FrontendScreen::Continue;
                        }
                    }
                    ArcadePhase::Fight => {
                        let opponent = self
                            .frontend
                            .as_ref()
                            .and_then(|f| f.arcade.as_ref())
                            .and_then(|session| session.current_opponent());
                        if let (Some(frontend), Some(opponent)) = (self.frontend.as_mut(), opponent)
                        {
                            frontend.p2 = opponent;
                        }
                        self.start_match();
                    }
                    ArcadePhase::Complete | ArcadePhase::GameOver => self.return_to_title(),
                }
            } else {
                self.return_to_title();
            }
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        if let (Some(config), Some(surface)) = (self.surface_config.as_mut(), self.surface.as_ref())
        {
            config.width = width.max(1);
            config.height = height.max(1);
            surface.configure(&renderer.device, config);
        }
    }

    fn draw(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let Some(config) = self.surface_config.as_ref() else {
            return;
        };
        let frame = match surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                surface.configure(&renderer.device, config);
                return;
            }
            Err(e) => {
                log::warn!("surface error: {e}");
                return;
            }
        };
        let status = self
            .spectator
            .as_ref()
            .map(|spectator| spectator.status_text())
            .or_else(|| {
                self.broadcaster.as_ref().map(|broadcaster| {
                    format!(
                        "BROADCASTING · {} spectator(s)",
                        broadcaster.spectator_count()
                    )
                })
            });
        renderer.set_status_line(status);
        if let Some(sim) = &self.sim {
            renderer.render(sim);
        } else if let Some(frontend) = &self.frontend {
            match frontend.screen {
                FrontendScreen::Title => {
                    renderer.render_title(
                        &frontend.screenpack,
                        &frontend.title_layers,
                        &frontend.title_view(),
                    );
                }
                FrontendScreen::Options => {
                    renderer.render_options(
                        &frontend.screenpack,
                        &frontend.title_layers,
                        &frontend.options_view(),
                    );
                }
                FrontendScreen::Select => {
                    renderer.render_select(
                        &frontend.screenpack,
                        &frontend.select_layers,
                        &frontend.select_view(),
                    );
                }
                FrontendScreen::Scores => {
                    renderer.render_scores(&frontend.scores);
                }
                FrontendScreen::Continue => {
                    if let Some(session) = &frontend.arcade {
                        renderer.render_continue(
                            session.credits,
                            session.continue_seconds_left(),
                            session.score,
                        );
                    }
                }
                FrontendScreen::Fight => {}
            }
        }
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        renderer.blit(&view, config.width, config.height);
        frame.present();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(e) = self.create_window(event_loop) {
            log::error!("failed to create window: {e:#}");
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => self.resize(size.width, size.height),
            WindowEvent::Focused(false) => self.keys.clear(),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        repeat,
                        ..
                    },
                ..
            } => {
                if repeat {
                    return;
                }
                match state {
                    ElementState::Pressed => {
                        if self
                            .frontend
                            .as_ref()
                            .is_some_and(|f| f.screen != FrontendScreen::Fight)
                        {
                            self.frontend_key(code, Some(event_loop));
                            return;
                        }
                        if code == KeyCode::KeyR {
                            if let Some(sim) = self.sim.as_mut() {
                                sim.restart_match();
                            }
                        } else if code == KeyCode::Escape {
                            if self.frontend.is_some() {
                                self.return_to_title();
                            } else {
                                event_loop.exit();
                            }
                        } else if code == KeyCode::F12 {
                            if let Some(sim) = self.sim.as_ref() {
                                if let Some(renderer) = self.renderer.as_mut() {
                                    renderer.render(sim);
                                    let path = format!("manato-{:06}.png", sim.frame);
                                    if let Err(e) = renderer.screenshot(Path::new(&path)) {
                                        log::warn!("screenshot failed: {e:#}");
                                    } else {
                                        log::info!("screenshot written to {path}");
                                    }
                                }
                            }
                        }
                        self.keys.insert(code);
                    }
                    ElementState::Released => {
                        self.keys.remove(&code);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                self.accumulator += now.duration_since(self.last_tick);
                self.last_tick = now;
                let mut steps = 0;
                while self.accumulator >= TICK && steps < 5 {
                    self.step();
                    self.accumulator -= TICK;
                    steps += 1;
                }
                self.draw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

fn dir_bits(
    keys: &HashSet<KeyCode>,
    up: KeyCode,
    down: KeyCode,
    left: KeyCode,
    right: KeyCode,
) -> u8 {
    let mut dir = 0;
    if keys.contains(&up) {
        dir |= DIR_UP;
    }
    if keys.contains(&down) {
        dir |= DIR_DOWN;
    }
    if keys.contains(&left) {
        dir |= DIR_LEFT;
    }
    if keys.contains(&right) {
        dir |= DIR_RIGHT;
    }
    dir
}

fn button_bits(keys: &HashSet<KeyCode>, b: [KeyCode; 6], start: KeyCode) -> u8 {
    let mut buttons = 0;
    for (i, code) in b.iter().enumerate() {
        if keys.contains(code) {
            buttons |= match i {
                0 => Buttons::X,
                1 => Buttons::Y,
                2 => Buttons::Z,
                3 => Buttons::A,
                4 => Buttons::B,
                _ => Buttons::C,
            };
        }
    }
    if keys.contains(&start) {
        buttons |= Buttons::START;
    }
    buttons
}

fn merge_input(a: PlayerInput, b: PlayerInput) -> PlayerInput {
    PlayerInput::new(a.dir | b.dir, a.buttons | b.buttons)
}

/// Converts a gamepad's per-frame state into engine input.
///
/// Mapping: South/East/North -> X/Y/Z, West/LeftTrigger/RightTrigger -> A/B/C,
/// Start -> START. D-pad and left stick both move, with a 0.5 dead zone.
fn pad_input_from(dpad: [bool; 4], stick: (f32, f32), buttons: [bool; 7]) -> PlayerInput {
    let [up, down, left, right] = dpad;
    let (x, y) = stick;
    let mut dir = 0;
    if up || y > 0.5 {
        dir |= DIR_UP;
    }
    if down || y < -0.5 {
        dir |= DIR_DOWN;
    }
    if left || x < -0.5 {
        dir |= DIR_LEFT;
    }
    if right || x > 0.5 {
        dir |= DIR_RIGHT;
    }
    let [south, east, north, west, l1, r1, start] = buttons;
    let mut bits = 0;
    if south {
        bits |= Buttons::X;
    }
    if east {
        bits |= Buttons::Y;
    }
    if north {
        bits |= Buttons::Z;
    }
    if west {
        bits |= Buttons::A;
    }
    if l1 {
        bits |= Buttons::B;
    }
    if r1 {
        bits |= Buttons::C;
    }
    if start {
        bits |= Buttons::START;
    }
    PlayerInput::new(dir, bits)
}

#[cfg(feature = "gamepad")]
fn pad_input(pad: &gilrs::Gamepad<'_>) -> PlayerInput {
    let dpad = [
        pad.is_pressed(gilrs::Button::DPadUp),
        pad.is_pressed(gilrs::Button::DPadDown),
        pad.is_pressed(gilrs::Button::DPadLeft),
        pad.is_pressed(gilrs::Button::DPadRight),
    ];
    let stick = (
        pad.value(gilrs::Axis::LeftStickX),
        pad.value(gilrs::Axis::LeftStickY),
    );
    let buttons = [
        pad.is_pressed(gilrs::Button::South),
        pad.is_pressed(gilrs::Button::East),
        pad.is_pressed(gilrs::Button::North),
        pad.is_pressed(gilrs::Button::West),
        pad.is_pressed(gilrs::Button::LeftTrigger),
        pad.is_pressed(gilrs::Button::RightTrigger),
        pad.is_pressed(gilrs::Button::Start),
    ];
    pad_input_from(dpad, stick, buttons)
}

/// Menu navigation bits derived from a pad frame.
fn pad_nav(input: PlayerInput) -> [bool; 6] {
    [
        input.dir & DIR_UP != 0,
        input.dir & DIR_DOWN != 0,
        input.dir & DIR_LEFT != 0,
        input.dir & DIR_RIGHT != 0,
        input.buttons & (Buttons::START | Buttons::X) != 0,
        input.buttons & (Buttons::A | Buttons::Y) != 0,
    ]
}

/// Rising edges only, so held directions move the cursor once.
fn nav_edges(previous: [bool; 6], current: [bool; 6]) -> Vec<KeyCode> {
    let keys = [
        KeyCode::KeyW,
        KeyCode::KeyS,
        KeyCode::KeyA,
        KeyCode::KeyD,
        KeyCode::Enter,
        KeyCode::Escape,
    ];
    current
        .iter()
        .zip(previous.iter())
        .zip(keys.iter())
        .filter(|((now, before), _)| **now && !**before)
        .map(|(_, key)| *key)
        .collect()
}

fn player_input(keys: &HashSet<KeyCode>, bindings: &KeyBindings) -> PlayerInput {
    PlayerInput::new(
        dir_bits(
            keys,
            bindings.up,
            bindings.down,
            bindings.left,
            bindings.right,
        ),
        button_bits(keys, bindings.buttons, bindings.start),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]
    use super::*;

    #[test]
    fn gamepad_maps_buttons_and_dpad() {
        let input = pad_input_from(
            [true, false, false, true],
            (0.0, 0.0),
            [true, false, true, false, false, false, true],
        );
        assert_eq!(input.dir, DIR_UP | DIR_RIGHT);
        assert_eq!(input.buttons, Buttons::X | Buttons::Z | Buttons::START);

        let kicks = pad_input_from(
            [false; 4],
            (0.0, 0.0),
            [false, false, false, true, true, true, false],
        );
        assert_eq!(kicks.buttons, Buttons::A | Buttons::B | Buttons::C);
    }

    #[test]
    fn gamepad_stick_uses_dead_zone() {
        let neutral = pad_input_from([false; 4], (0.4, -0.3), [false; 7]);
        assert_eq!(neutral.dir, 0);
        let diagonal = pad_input_from([false; 4], (0.9, -0.9), [false; 7]);
        assert_eq!(diagonal.dir, DIR_RIGHT | DIR_DOWN);
    }

    fn test_frontend() -> Frontend {
        let mut screenpack = Screenpack::default();
        screenpack.title.items = vec![
            manato_formats::screenpack::MenuItemDef {
                key: "arcade".into(),
                label: "ARCADE".into(),
            },
            manato_formats::screenpack::MenuItemDef {
                key: "versus".into(),
                label: "VERSUS".into(),
            },
        ];
        screenpack.select.rows = 1;
        screenpack.select.columns = 3;
        let entry = RosterEntry {
            def: "kaen.def".into(),
            stage: None,
            music: None,
            random: false,
            single: false,
        };
        Frontend {
            screen: FrontendScreen::Title,
            screenpack,
            roster: vec![
                entry.clone(),
                entry.clone(),
                entry.clone(),
                entry.clone(),
                entry,
            ],
            faces: vec![],
            names: vec![],
            select_timer: Some(60),
            stages: vec!["a.def".into(), "b.def".into()],
            stage_index: 0,
            menu: vec![
                ("arcade".into(), "ARCADE".into()),
                ("versus".into(), "VERSUS".into()),
            ],
            menu_index: 0,
            mode: Mode::Arcade,
            p1: 0,
            p2: 1,
            p1_done: false,
            p2_done: false,
            page: 0,
            frame: 0,
            options_index: 0,
            options_rounds: 2,
            options_round_time: 99,
            options_ai: 6,
            options_sound: 100,
            options_music: 100,
            options_fullscreen: false,
            team_modes: [manato_sim::TeamMode::Single; 2],
            partners: [[None; MAX_PARTNERS]; 2],
            result_timer: 0,
            result_resolved: false,
            arcade: None,
            arcade_special: None,
            arcade_final_boss: None,
            arcade_order: ArcadeOrderMode::Roster,
            scores: vec![],
            title_layers: vec![],
            select_layers: vec![],
        }
    }

    #[test]
    fn storyboard_lua_applies_transform_and_scene_jump() {
        let storyboard = manato_formats::storyboard::Storyboard {
            path: std::path::PathBuf::from("test.def"),
            name: "test".into(),
            author: String::new(),
            start_scene: 0,
            sprites: manato_formats::SffFile::default(),
            animations: HashMap::new(),
            scenes: vec![
                manato_formats::storyboard::StoryboardScene {
                    name: "Scene 0".into(),
                    ..Default::default()
                },
                manato_formats::storyboard::StoryboardScene {
                    name: "Scene 1".into(),
                    ..Default::default()
                },
            ],
            font_files: std::collections::BTreeMap::new(),
            sounds: manato_formats::SoundBank::default(),
            lua_source: Some(
                "function update(ctx)\n\
                   ctx.x = ctx.x + 10\n\
                   ctx.alpha = 0.5\n\
                   if ctx.time >= 2 then ctx.jump = 'Scene 1' end\n\
                 end\n"
                    .into(),
            ),
        };
        let player = manato_formats::storyboard::StoryboardPlayer::new(&storyboard);
        let mut overlays = vec![manato_sim::StoryboardOverlay {
            storyboard: Arc::new(storyboard),
            player,
            owner: 0,
            id: 0,
            pos: manato_core::Vec2::new(5.0, 6.0),
            scale: manato_core::Vec2::new(1.0, 1.0),
            facing: 1,
            screen_space: true,
            bind_target: None,
            bind_time: 0,
            script_alpha: 1.0,
        }];
        let mut scripts = StoryboardScripts::default();
        scripts.tick(&mut overlays);
        assert_eq!(overlays[0].pos.x, 15.0);
        assert!((overlays[0].script_alpha - 0.5).abs() < f32::EPSILON);
        assert_eq!(overlays[0].player.scene, 0);

        overlays[0].player.time = 2;
        scripts.tick(&mut overlays);
        assert_eq!(overlays[0].player.scene, 1);
        assert_eq!(overlays[0].player.time, 0);
    }

    #[test]
    fn title_menu_wraps() {
        let mut frontend = test_frontend();
        frontend.move_menu(1);
        assert_eq!(frontend.menu_index, 1);
        frontend.move_menu(1);
        assert_eq!(frontend.menu_index, 0);
        frontend.move_menu(-1);
        assert_eq!(frontend.menu_index, 1);
    }

    #[test]
    fn options_cycle_team_modes_and_partners() {
        let mut frontend = test_frontend();
        assert_eq!(frontend.team_modes[0], manato_sim::TeamMode::Single);
        frontend.cycle_team(0, 1);
        assert_eq!(frontend.team_modes[0], manato_sim::TeamMode::Turns);
        frontend.cycle_team(0, 1);
        frontend.cycle_team(0, 1);
        assert_eq!(frontend.team_modes[0], manato_sim::TeamMode::Simul);
        frontend.cycle_team(0, 1);
        assert_eq!(frontend.team_modes[0], manato_sim::TeamMode::Single);
        frontend.cycle_team(0, -1);
        assert_eq!(frontend.team_modes[0], manato_sim::TeamMode::Simul);

        for slot in 0..MAX_PARTNERS {
            let first = frontend.partners[0][slot];
            frontend.cycle_partner(0, slot, 1);
            assert_ne!(frontend.partners[0][slot], first);
            let changed = frontend.partners[0][slot];
            for _ in 0..frontend.roster.len() {
                frontend.cycle_partner(0, slot, 1);
            }
            assert_eq!(
                frontend.partners[0][slot], changed,
                "partner slot {slot} cycling wraps"
            );
        }
    }

    #[test]
    fn options_expose_all_partner_slots() {
        let frontend = test_frontend();
        let view = frontend.options_view();
        assert_eq!(view.items.len(), 8 + 2 * MAX_PARTNERS + 1);
        assert_eq!(view.items[8].0, "P1 PARTNER");
        assert_eq!(view.items[9].0, "P1 PARTNER 2");
        assert_eq!(view.items[10].0, "P1 PARTNER 3");
        assert_eq!(view.items[11].0, "P2 PARTNER");
        assert_eq!(view.items[12].0, "P2 PARTNER 2");
        assert_eq!(view.items[13].0, "P2 PARTNER 3");
        assert_eq!(view.items[14].0, "BACK");
    }

    #[test]
    fn single_only_roster_entries_force_single_mode_and_skip_partners() {
        let mut frontend = test_frontend();
        frontend.roster[1].single = true;
        frontend.partners[0][0] = Some(0);
        frontend.cycle_partner(0, 0, 1);
        assert_eq!(
            frontend.partners[0][0],
            Some(2),
            "single-only partner skipped"
        );
        frontend.partners[0][1] = Some(0);
        frontend.cycle_partner(0, 1, 1);
        assert_eq!(
            frontend.partners[0][1],
            Some(2),
            "single-only second partner skipped"
        );

        frontend.p1 = 1;
        frontend.p2 = 0;
        frontend.team_modes = [manato_sim::TeamMode::Simul, manato_sim::TeamMode::Tag];
        assert_eq!(
            frontend.effective_team_modes(),
            [manato_sim::TeamMode::Single, manato_sim::TeamMode::Tag]
        );
        frontend.p2 = 1;
        assert_eq!(
            frontend.effective_team_modes(),
            [manato_sim::TeamMode::Single; 2]
        );
    }

    #[test]
    fn select_cursor_moves_and_pages() {
        let mut frontend = test_frontend();
        frontend.move_cursor(0, 1, 0);
        assert_eq!(frontend.p1, 1);
        frontend.move_cursor(0, 1, 0);
        assert_eq!(frontend.p1, 2);
        // A one-row grid does not wrap horizontally.
        frontend.move_cursor(0, 1, 0);
        assert_eq!(frontend.p1, 2);
        frontend.move_cursor(0, 0, 1);
        assert_eq!(frontend.p1, 4);
        assert_eq!(frontend.page, 1);
        frontend.move_cursor(0, 0, -1);
        assert_eq!(frontend.p1, 1);
        assert_eq!(frontend.page, 0);
    }

    #[test]
    fn stage_selection_wraps() {
        let mut frontend = test_frontend();
        frontend.cycle_stage(1);
        assert_eq!(frontend.stage_index, 1);
        frontend.cycle_stage(1);
        assert_eq!(frontend.stage_index, 0);
        frontend.cycle_stage(-1);
        assert_eq!(frontend.stage_index, 1);
    }

    #[test]
    fn random_roster_slots_resolve_deterministically() {
        let mut frontend = test_frontend();
        frontend.roster[0].random = true;
        frontend.frame = 7;
        let first = chosen_entry(&frontend, 0).expect("entry");
        let second = chosen_entry(&frontend, 0).expect("entry");
        assert_eq!(first.def, second.def);
        assert!(!first.random);
    }

    #[test]
    fn pad_menu_edges_fire_once() {
        let current = [true, false, false, false, true, false];
        let keys = nav_edges([false; 6], current);
        assert_eq!(keys, vec![KeyCode::KeyW, KeyCode::Enter]);
        assert!(nav_edges(current, current).is_empty());
        let released = nav_edges(current, [false; 6]);
        assert!(released.is_empty());
    }

    #[test]
    fn key_parsing_and_overrides() {
        assert_eq!(parse_key("W"), Some(KeyCode::KeyW));
        assert_eq!(parse_key("arrowup"), Some(KeyCode::ArrowUp));
        assert_eq!(parse_key("numpad3"), Some(KeyCode::Numpad3));
        assert_eq!(parse_key("not-a-key"), None);

        let mut bindings = KeyBindings::default();
        let file = KeyFile {
            up: Some("arrowup".into()),
            x: Some("l".into()),
            ..Default::default()
        };
        bindings.apply(&file);
        assert_eq!(bindings.up, KeyCode::ArrowUp);
        assert_eq!(bindings.buttons[0], KeyCode::KeyL);
        assert_eq!(bindings.down, KeyCode::KeyS);
    }

    #[test]
    fn keyboard_and_gamepad_merge() {
        let keyboard = PlayerInput::new(DIR_LEFT, Buttons::X);
        let pad = PlayerInput::new(DIR_RIGHT, Buttons::Y);
        let merged = merge_input(keyboard, pad);
        assert_eq!(merged.dir, DIR_LEFT | DIR_RIGHT);
        assert_eq!(merged.buttons, Buttons::X | Buttons::Y);
    }
}
