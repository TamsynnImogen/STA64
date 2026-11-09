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

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::platform::cell::SyncCell;
use bevy::prelude::*;
use bevy::time::Real;
use sta_bench::parse::{BenchCommand, SceneFile, parse_command};
use sta_bench::run::{Bench, RunConfig};
use sta_sim::Tick;

use crate::app::{Control, GameSet, LaunchConfig, configure_sets};
use crate::clock::AnimClock;
use crate::sim_link::SimLinkPlugin;

/// Build the scene named by `--scene` on the bench's fake content.
fn build(cfg: &LaunchConfig) -> Result<Bench, String> {
    let path = cfg.scene.as_deref().ok_or("--sim-only and --headless need --scene <ini>")?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let scene = SceneFile::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
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
    println!("{}", digest_line(&mut bench));
    0
}

#[derive(Resource)]
struct BenchRes(SyncCell<Bench>);

#[derive(Resource)]
struct Channel(SyncCell<Receiver<String>>);

#[derive(Resource)]
struct TickBudget(Tick);

/// `--headless`: a Bevy app without a window; returns the process exit code.
pub fn run_headless(cfg: &LaunchConfig) -> i32 {
    if cfg.frames.is_empty() {
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
    let mut app = App::new();
    let runner = ScheduleRunnerPlugin::run_loop(Duration::from_millis(1));
    if cfg.frames.is_empty() {
        app.add_plugins(MinimalPlugins.set(runner));
    } else {
        // Frames: the renderer without a window (an offscreen image target).
        use bevy::render::RenderPlugin;
        use bevy::window::ExitCondition;
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
                .disable::<bevy::winit::WinitPlugin>(),
        )
        .add_plugins(runner);
    }
    configure_sets(&mut app);
    app.insert_resource(AnimClock::new(cfg.clock_mode(), 0))
        .add_plugins(SimLinkPlugin)
        .insert_resource(BenchRes(SyncCell::new(bench)))
        .insert_resource(TickBudget(cfg.ticks.unwrap_or(0).max(cfg.frames.iter().copied().max().unwrap_or(0))))
        .add_systems(Update, bench_frame.in_set(GameSet::Bench));
    if matches!(cfg.control, Some(Control::Stdio)) {
        app.insert_resource(Channel(SyncCell::new(spawn_stdin())));
    }
    if !cfg.frames.is_empty() {
        frames::add(&mut app, cfg, scene_camera);
    }
    match app.run() {
        AppExit::Success => 0,
        AppExit::Error(c) => i32::from(c.get()),
    }
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
    }
    clock.end_frame();
    let frames_done = capture.as_ref().is_none_or(|c| c.done());
    if !open && frames_done && (bench.tick() >= budget.0 || bench.paused()) {
        println!("{}", digest_line(bench));
        exit.write(AppExit::Success);
    }
}
