//! The test bench inside `sta64` (docs/TESTBENCH.md, "Architecture").
//!
//! `--scene <ini>` builds the scene through the tier-1 runner's [`Bench`]
//! (scene -> `GameSetup` -> `Sim`), so a sim-only run and an in-process test
//! agree digest for digest. Content is the bench's fake content for now.
//!
//! - `--sim-only`: no Bevy `App` at all, just the sim loop. Prints the final
//!   `query` answer (`tick N objects M digest HHHH`) and exits.
//! - `--headless`: a windowless Bevy app (`MinimalPlugins`); `AnimClock`
//!   decides how many ticks a frame runs; the command channel is a system in
//!   [`GameSet::Bench`].
//! - `--control stdio`: one `BenchCommand` per line on stdin
//!   (`sta_bench::parse`), one answer line per command on stdout.

pub mod frames;
pub mod marks;
mod pair_view;
pub mod perf;

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::platform::cell::SyncCell;
use bevy::prelude::*;
use bevy::time::Real;
use sta_bench::parse::{BenchCommand, SceneFile, parse_command};
use sta_bench::run::{Bench, RunConfig};
use sta_sim::command::LocalRouter;
use sta_sim::{PlayerId, Tick, TickOutput, WorldSource, WorldView};

use crate::app::{Control, GameSet, LaunchConfig, configure_sets};
use crate::clock::AnimClock;
use crate::sim_link::{BenchDriven, ContentRes, SimLink, SimLinkPlugin, SimMsg};

/// Build the scene named by `--scene` on the bench's fake content.
fn build(cfg: &LaunchConfig) -> Result<Bench, String> {
    let path = cfg.scene.as_deref().ok_or("--sim-only and --headless need --scene <ini>")?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let scene = SceneFile::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if cfg.install_content {
        return Bench::build(Arc::new(pair_view::install_config(cfg, &scene)?), &scene);
    }
    Bench::build(Arc::new(RunConfig::fake()), &scene)
}

/// Run one command line against the bench; blank and comment lines answer
/// nothing.
fn run_line(bench: &mut Bench, line: &str) -> Option<String> {
    match parse_command(line) {
        Ok(None) => None,
        Ok(Some(c)) => Some(bench.exec_timed(&c)),
        Err(e) => Some(format!("! {e}")),
    }
}

/// The line a run ends on: what the `query` command answers.
fn digest_line(bench: &mut Bench) -> String {
    bench.exec(&BenchCommand::Query)
}

/// `--sim-only`; returns the process exit code.
pub fn run_sim_only(cfg: &LaunchConfig) -> i32 {
    let mut bench = match build(cfg) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    if matches!(cfg.control, Some(Control::Stdio)) {
        // Commands until stdin closes, then the `--ticks` budget.
        for line in std::io::stdin().lines().map_while(Result::ok) {
            if let Some(a) = run_line(&mut bench, &line) {
                println!("{a}");
            }
        }
    }
    let target = cfg.ticks.unwrap_or(0);
    while bench.tick() < target && bench.advance() {}
    marks::write_if_requested(&bench);
    println!("{}", digest_line(&mut bench));
    0
}

/// The link's source while the bench steps the sim: never asked to step.
struct Idle(Arc<WorldView>);

impl WorldSource for Idle {
    fn step(&mut self, _: &[sta_sim::command::Command]) -> TickOutput {
        TickOutput { view: self.0.clone(), events: Vec::new() }
    }
    fn view(&self) -> Arc<WorldView> {
        self.0.clone()
    }
}

#[derive(Resource)]
struct BenchRes(SyncCell<Bench>);

#[derive(Resource)]
struct Channel(SyncCell<Receiver<String>>);

#[derive(Resource)]
struct TickBudget(Tick);

/// Whether the headless app draws (frames or a performance run) or is the sim alone.
fn renders(cfg: &LaunchConfig) -> bool {
    !cfg.frames.is_empty() || cfg.perf.is_some()
}

/// `--headless`: a Bevy app without a window; returns the process exit code.
pub fn run_headless(cfg: &LaunchConfig) -> i32 {
    if !renders(cfg) {
        return run_headless_inner(cfg);
    }
    // No adapter panics inside plugin setup: report it as a skippable
    // condition, not a crash.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_headless_inner(cfg))) {
        Ok(code) => code,
        Err(e) => {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default();
            if msg.contains("Unable to find a GPU") {
                eprintln!("no GPU adapter: {msg}");
                frames::NO_ADAPTER
            } else {
                eprintln!("headless render failed: {msg}");
                101
            }
        }
    }
}

fn run_headless_inner(cfg: &LaunchConfig) -> i32 {
    let bench = match build(cfg) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let scene_camera = scene_camera(cfg);
    let content = bench.cfg.content.clone();
    let first_view = bench.view().clone();
    let fog = scene_fog(cfg, &bench);
    let mut app = App::new();
    // A performance run is not paced: no wait between frames.
    let runner = ScheduleRunnerPlugin::run_loop(if cfg.perf.is_some() { Duration::ZERO } else { Duration::from_millis(1) });
    if !renders(cfg) {
        app.add_plugins(MinimalPlugins.set(runner));
    } else {
        // The audio device is never opened: a bench frame has no sound (bevy_audio is not compiled in).
        // Frames: the renderer without a window (an offscreen image target).
        use bevy::render::RenderPlugin;
        use bevy::window::ExitCondition;
        app.add_plugins(
            perf::log_plugin(cfg, DefaultPlugins)
                .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
                .set(ImagePlugin::default_linear())
                .disable::<bevy::winit::WinitPlugin>(),
        )
        .add_plugins(runner);
        if let Some(info) = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>()
            && frames::is_software_adapter(info.device_type, &info.name)
        {
            eprintln!("no GPU adapter: only a software adapter is available ({})", info.name);
            return frames::NO_ADAPTER;
        }
    }
    configure_sets(&mut app);
    app.insert_resource(AnimClock::new(cfg.clock_mode(), 0))
        .add_plugins(SimLinkPlugin)
        .insert_resource(BenchRes(SyncCell::new(bench)))
        .insert_resource(TickBudget(if cfg.perf.is_some() { Tick::MAX } else { cfg.ticks.unwrap_or(0).max(cfg.frames.iter().copied().max().unwrap_or(0)) }))
        .add_systems(Update, bench_frame.in_set(GameSet::Bench));
    if matches!(cfg.control, Some(Control::Stdio)) {
        app.insert_resource(Channel(SyncCell::new(spawn_stdin())));
    }
    if cfg.install_content && renders(cfg) {
        // TOOL-17: the install's content, drawn by the client's own renderers.
        if let Err(e) = pair_view::add(&mut app, cfg, scene_camera) {
            eprintln!("{e}");
            return 2;
        }
        if let Some(opts) = &cfg.perf {
            // TOOL-18: a seat's fog (`Fog=1`), the run's driver and its measurements.
            if let Some((player, bounds)) = fog {
                app.insert_resource(crate::render::units::LocalPlayer(Some(player))).insert_resource(bounds);
            }
            perf::add(&mut app, opts, cfg);
        }
    } else if cfg.perf.is_some() {
        eprintln!("--perf needs --install-content (and --install-root): it measures the install's map view");
        return 2;
    } else if !cfg.frames.is_empty() {
        frames::add(&mut app, cfg, scene_camera);
        // The effects read the sim through a link; this one is fed the bench's views
        // and events as they happen (`bench_frame`).
        app.insert_resource(ContentRes(content))
            .insert_resource(SimLink::new(Box::new(Idle(first_view)), Box::new(LocalRouter::new()), PlayerId(0)))
            .insert_resource(BenchDriven)
            .add_plugins(crate::render::fx::FxMaterialsPlugin)
            .add_plugins((
                crate::render::fx::weapons::WeaponFxPlugin,
                crate::render::fx::impacts::ImpactFxPlugin,
                crate::render::fx::specials::SpecialsFxPlugin,
            ));
        app.add_plugins(crate::render::post::PostFxPlugin::from_cfg(cfg));
        if crate::frontend::loading::frame_requested() {
            // UI-08: the loading screen over the frame (`STA64_UI_LOADING=1`).
            frames::add_loading(&mut app, cfg);
        } else if crate::ui::hud::frame::requested() {
            frames::add_hud(&mut app, cfg);
        } else if let Some(screen) = crate::frontend::screens::frame::requested() {
            frames::add_screen(&mut app, cfg, screen);
        } else if crate::ui::radar::frame_overlay_requested() {
            frames::add_radar(&mut app, cfg);
        }
        // `--install-root` (and `--mod`) mount the files the frame reads: the mod's own
        // Gshroud / Gfog art, its ordnance recipes (RND-28) and, with
        // `STA64_DEV_SHADER_SHADOW=1`, its shader shadows.
        let mounted = cfg.install_root.as_ref().map(|root| sta_vfs::Vfs::mount_install(root, cfg.active_mod.as_deref()));
        if let Some(Err(e)) = &mounted
            && cfg.active_mod.is_some()
        {
            // The install root defaults to the executable's folder, which is no install;
            // a requested mod that does not mount is worth saying (the frame draws stock).
            eprintln!("bench: --mod {:?} not mounted, drawing without its files: {e}", cfg.active_mod);
        }
        if let Some(Ok(vfs)) = mounted {
            // The systems that read the mounted files also read the sprite tables and
            // the texture and part caches (the map view and the viewer make these too).
            app.init_resource::<crate::render::textures::TextureCache>()
                .init_resource::<crate::render::model_scene::PartCache>();
            app.insert_resource(crate::render::sprites::SpriteTables(Arc::new(sta_assets::load_sprite_table(&vfs))));
            app.insert_resource(crate::viewer::Mounted(vfs));
        }
        app.add_plugins(crate::render::shader_shadow::ShaderShadowPlugin);
        // `Fog=1` in the scene draws the fog and shroud as the first seat sees the map.
        if let Some((player, bounds)) = fog {
            app.insert_resource(crate::render::units::LocalPlayer(Some(player)))
                .insert_resource(bounds)
                .add_plugins(crate::render::fog::FogPlugin);
        }
    }
    match app.run() {
        AppExit::Success => 0,
        AppExit::Error(c) => i32::from(c.get()),
    }
}

/// The scene's `Fog=1`: whose eyes the frames are drawn through (the first seat) and the
/// map's corners (the scene's `Map=`, or the empty map).
fn scene_fog(cfg: &LaunchConfig, bench: &Bench) -> Option<(PlayerId, crate::render::fog::FogBounds)> {
    let text = std::fs::read_to_string(cfg.scene.as_deref()?).ok()?;
    let scene = SceneFile::parse(&text).ok()?;
    if scene.scene.switch(sta_bench::parse::System::Fog) != Some(true) {
        return None;
    }
    let map = match &scene.scene.map {
        Some(name) => bench.cfg.maps.get(name).cloned().unwrap_or_else(sta_content::MapDef::empty),
        None => sta_content::MapDef::empty(),
    };
    let (lo, hi) = map.bounds;
    let player = bench.view().players.first()?.id;
    let bounds = crate::render::fog::FogBounds { min: Vec2::new(lo.x.to_f32(), lo.z.to_f32()), max: Vec2::new(hi.x.to_f32(), hi.z.to_f32()) };
    Some((player, bounds))
}

/// The scene's `Camera=` command, for the frame view.
fn scene_camera(cfg: &LaunchConfig) -> Option<BenchCommand> {
    let text = std::fs::read_to_string(cfg.scene.as_deref()?).ok()?;
    SceneFile::parse(&text).ok()?.scene.camera.map(|t| t.command)
}

fn spawn_stdin() -> Receiver<String> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// Per frame: the channel's commands, then the ticks the clock allows, then
/// exit once the budget is reached and the channel (if any) has closed.
#[allow(clippy::too_many_arguments)]
fn bench_frame(
    time: Res<Time<Real>>,
    mut clock: ResMut<AnimClock>,
    mut bench: ResMut<BenchRes>,
    budget: Res<TickBudget>,
    mut channel: Option<ResMut<Channel>>,
    mut capture: Option<ResMut<frames::Capture>>,
    mut view: Option<ResMut<frames::View>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
    mut link: Option<ResMut<SimLink>>,
    mut out: MessageWriter<SimMsg>,
    mut sent: Local<usize>,
) {
    let bench = bench.0.get();
    let mut open = false;
    if let Some(ch) = channel.as_mut() {
        open = true;
        loop {
            match ch.0.get().try_recv() {
                Ok(line) => {
                    if let Some(v) = view.as_mut() {
                        v.observe(&line);
                    }
                    if let Some(a) = run_line(bench, &line) {
                        println!("{a}");
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    open = false;
                    break;
                }
            }
        }
    }
    // A frame being captured holds the sim; otherwise it caps the ticks run.
    let (mut limit, mut held) = (Tick::MAX, false);
    if let Some(c) = capture.as_mut() {
        match c.gate(bench.tick(), &mut commands) {
            frames::Gate::Run(t) => limit = t,
            frames::Gate::Hold => held = true,
        }
    }
    if held {
        return;
    }
    clock.set_paused(bench.paused());
    let due = clock.begin_frame(time.delta_secs_f64());
    for _ in 0..due {
        if bench.tick() >= limit || (bench.tick() >= budget.0 && !open) || !bench.advance() {
            break;
        }
        clock.ticked();
        // The effects' view of the tick just run: its world and its events.
        if let Some(link) = link.as_mut() {
            link.feed(bench.view().clone());
            out.write_batch(bench.events[*sent..].iter().map(|(_, e)| SimMsg(e.clone())));
            *sent = bench.events.len();
        }
    }
    clock.end_frame();
    let frames_done = capture.as_ref().is_none_or(|c| c.done());
    if !open && frames_done && (bench.tick() >= budget.0 || bench.paused()) {
        marks::write_if_requested(bench);
        println!("{}", digest_line(bench));
        exit.write(AppExit::Success);
    }
}
