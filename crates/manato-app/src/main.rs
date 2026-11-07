//! M.A.N.A.T.O.-RS command line runner and window.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use manato_audio::AudioEngine;
use manato_core::{Buttons, PlayerInput, DIR_DOWN, DIR_LEFT, DIR_RIGHT, DIR_UP};
use manato_render::{wgpu, Renderer, GAME_HEIGHT, GAME_WIDTH};
use manato_sim::{ai, Assets, MatchConfig, MatchOptions, SimState};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const TICK: Duration = Duration::from_nanos(16_666_667);

#[derive(Debug, Clone)]
struct Options {
    root: PathBuf,
    p1: String,
    p2: String,
    stage: String,
    ai_level: i32,
    rounds: u32,
    round_time: i32,
    frames: Option<u32>,
    synctest: Option<u32>,
    screenshot: Option<PathBuf>,
    scale: u32,
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
            ai_level: 6,
            rounds: 2,
            round_time: 99,
            frames: None,
            synctest: None,
            screenshot: None,
            scale: 3,
        }
    }
}

/// Optional TOML config file. Command-line arguments override these values.
#[derive(Debug, Default, serde::Deserialize)]
struct FileConfig {
    root: Option<PathBuf>,
    p1: Option<String>,
    p2: Option<String>,
    stage: Option<String>,
    ai: Option<i32>,
    rounds: Option<u32>,
    round_time: Option<i32>,
    scale: Option<u32>,
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
  --ai <level>        CPU level for slot 2, 0 = human (default 6)
  --rounds <n>        Rounds needed to win (default 2)
  --round-time <s>    Round timer in seconds (default 99)

Modes:
  --frames <n>        Run n ticks headless, then exit
  --synctest <n>      Run n ticks through GGRS rollback verification
  --screenshot <png>  Save the final frame (headless)
  --scale <n>         Window integer scale (default 3)

Controls (window mode):
  P1: WASD move, J/K/L = X/Y/Z, U/I/O = A/B/C
  P2: arrows move, numpad 1/2/3 = X/Y/Z, 4/5/6 = A/B/C
  F12 screenshot, Esc quit
"#
    );
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let opts = Options::parse()?;

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

struct App {
    opts: Options,
    sim: SimState,
    renderer: Option<Renderer>,
    audio: AudioEngine,
    instance: wgpu::Instance,
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    surface_config: Option<wgpu::SurfaceConfiguration>,
    keys: HashSet<KeyCode>,
    last_tick: Instant,
    accumulator: Duration,
}

fn run_window(opts: Options) -> anyhow::Result<()> {
    let sim = load_match(&opts)?;
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let audio = AudioEngine::new();
    log::info!(
        "audio backend: {} ({} Hz)",
        audio.backend_name(),
        audio.sample_rate()
    );
    if let Some(path) = &sim.assets.stage.music.path {
        match manato_audio::Music::load(
            path,
            sim.assets.stage.music.loop_start,
            sim.assets.stage.music.loop_end,
        ) {
            Ok(music) => {
                let volume = (sim.assets.stage.music.volume as f32 / 100.0
                    * sim.assets.stage.music.ratio.clamp(0.0, 1.0))
                .clamp(0.0, 1.0);
                audio.play_music(&music, volume);
                log::info!("playing stage music {}", path.display());
            }
            Err(error) => log::warn!("could not load stage music {}: {error:#}", path.display()),
        }
    }
    let mut app = App {
        opts,
        sim,
        renderer: None,
        audio,
        instance,
        window: None,
        surface: None,
        surface_config: None,
        keys: HashSet::new(),
        last_tick: Instant::now(),
        accumulator: Duration::ZERO,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

impl App {
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
        renderer.load_assets(&self.sim);
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
        Ok(())
    }

    fn step(&mut self) {
        let p1 = p1_input(&self.keys);
        let p2 = if self.opts.ai_level > 0 {
            ai::input_for(&self.sim, 1, self.opts.ai_level)
        } else {
            p2_input(&self.keys)
        };
        self.sim.tick([p1, p2]);
        self.audio.play_events(&self.sim);
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
        renderer.render(&self.sim);
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
                        if code == KeyCode::Escape {
                            event_loop.exit();
                        } else if code == KeyCode::F12 {
                            if let Some(renderer) = self.renderer.as_mut() {
                                renderer.render(&self.sim);
                                let path = format!("manato-{:06}.png", self.sim.frame);
                                if let Err(e) = renderer.screenshot(Path::new(&path)) {
                                    log::warn!("screenshot failed: {e:#}");
                                } else {
                                    log::info!("screenshot written to {path}");
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

fn p1_input(keys: &HashSet<KeyCode>) -> PlayerInput {
    PlayerInput::new(
        dir_bits(
            keys,
            KeyCode::KeyW,
            KeyCode::KeyS,
            KeyCode::KeyA,
            KeyCode::KeyD,
        ),
        button_bits(
            keys,
            [
                KeyCode::KeyJ,
                KeyCode::KeyK,
                KeyCode::KeyL,
                KeyCode::KeyU,
                KeyCode::KeyI,
                KeyCode::KeyO,
            ],
            KeyCode::Enter,
        ),
    )
}

fn p2_input(keys: &HashSet<KeyCode>) -> PlayerInput {
    PlayerInput::new(
        dir_bits(
            keys,
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
        ),
        button_bits(
            keys,
            [
                KeyCode::Numpad1,
                KeyCode::Numpad2,
                KeyCode::Numpad3,
                KeyCode::Numpad4,
                KeyCode::Numpad5,
                KeyCode::Numpad6,
            ],
            KeyCode::Numpad0,
        ),
    )
}
