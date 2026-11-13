//! Performance runs of the install's map view (TOOL-18; docs/TESTBENCH.md, "Performance runs").
//!
//! `--headless --install-content --scene <ini> --perf <out.json>` draws the scene's map with
//! the map view's own renderers into an offscreen image (no window, no display server, no
//! audio), unpaced (no vsync, no sleep between frames), and measures it:
//!
//! - **frame time**, the wall time between two frames, with the GPU waited for at the end
//!   of each (so a GPU-bound frame is not hidden by the queue running ahead; `--perf-no-sync`
//!   leaves it out), and **main CPU time**, the main world's schedule alone;
//! - **GPU time per render pass** from Bevy's render diagnostics (wgpu timestamp queries,
//!   where the adapter has them; the report says when it has not);
//! - **CPU time per system and per schedule** (`--perf-systems`, needs a build with the
//!   `perf-trace` feature: it adds up Bevy's own tracing spans);
//! - **counts**: entities, visible meshes, visible rocks, whether the shadow pass is on.
//!
//! Cost is attributed **by difference**: `--perf-variants base,no-rocks,no-nebula+no-rocks`
//! runs each variant (a `+` list of [`TOGGLES`]) for `--perf-reps` rounds, interleaved
//! (base, a, b, base, a, b, ...), each round `--perf-warmup` frames to settle and
//! `--perf-frames` measured, so a load that changes during the run touches every variant
//! alike. The orchestration over installs, profiles and sizes is `sta-bench perf`.
//!
//! What is not in the picture: the HUD and the menus (a headless app has none), the sound,
//! and a window's presentation (vsync, the compositor).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use bevy::app::{AppExit, PluginGroupBuilder};
use bevy::diagnostic::DiagnosticsStore;
use bevy::log::{BoxedLayer, LogPlugin};
use bevy::prelude::*;

use crate::app::LaunchConfig;

/// What a variant can switch, by name: classes of things hidden, and quality settings
/// ([`crate::render::quality`]).
pub const TOGGLES: &[(&str, &str)] = &[
    ("no-rocks", "asteroid fields hidden"),
    ("no-planets", "planets hidden"),
    ("no-nebula", "nebulae hidden (gas volumes and billboards)"),
    ("no-sky", "sky, starfield and background objects hidden"),
    ("no-shadows", "the key-light shadow pass off"),
    ("no-fog", "fog and shroud layers off"),
    ("std-hulls", "hulls in Bevy's PBR instead of the hull shader"),
    ("msaa-off", "no multisampling on the map's cameras"),
    ("msaa-2", "2x multisampling"),
    ("no-prepass", "the map camera's depth prepass off (the nebula march reads it)"),
    ("neb-steps8", "nebula march with 8 steps instead of 16"),
    ("freeze-rocks", "rock materials not rewritten when the camera moves (lighting goes stale: measurement only)"),
    ("freeze-hulls", "hull materials not rewritten when the camera moves (measurement only)"),
];

/// The `--perf*` options.
#[derive(Debug, Clone, PartialEq)]
pub struct PerfOpts {
    pub out: Option<PathBuf>,
    pub frames: u32,
    pub warmup: u32,
    pub reps: u32,
    pub variants: Vec<String>,
    /// `still` (the scene's camera) or `sweep` (it swings about the point it looks at).
    pub camera: String,
    pub systems: bool,
    pub sync: bool,
}

impl Default for PerfOpts {
    fn default() -> Self {
        PerfOpts { out: None, frames: 300, warmup: 90, reps: 3, variants: vec!["base".into()], camera: "still".into(), systems: false, sync: true }
    }
}

impl PerfOpts {
    pub fn set(&mut self, flag: &str, v: Option<&str>) -> Result<(), String> {
        let num = |v: Option<&str>| v.and_then(|v| v.parse::<u32>().ok()).filter(|n| *n > 0).ok_or_else(|| format!("{flag} (needs a number)"));
        match flag {
            "--perf" => self.out = v.map(PathBuf::from),
            "--perf-frames" => self.frames = num(v)?,
            "--perf-warmup" => self.warmup = num(v)?,
            "--perf-reps" => self.reps = num(v)?,
            "--perf-variants" => {
                self.variants = v.unwrap_or("").split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
                if self.variants.is_empty() {
                    return Err("--perf-variants (needs at least one, e.g. base)".into());
                }
            }
            "--perf-camera" => match v {
                Some(c @ ("still" | "sweep")) => self.camera = c.into(),
                _ => return Err("--perf-camera (still or sweep)".into()),
            },
            "--perf-systems" => self.systems = true,
            "--perf-no-sync" => self.sync = false,
            _ => return Err(flag.into()),
        }
        Ok(())
    }
}

/// The names in a variant (`a+b`), each a toggle or `base`.
fn parse_variant(v: &str) -> Result<Vec<&str>, String> {
    let names: Vec<&str> = v.split('+').map(str::trim).filter(|n| *n != "base" && !n.is_empty()).collect();
    for n in &names {
        if !TOGGLES.iter().any(|(t, _)| t == n) {
            return Err(format!("unknown perf toggle {n:?} (known: {})", TOGGLES.iter().map(|(t, _)| *t).collect::<Vec<_>>().join(", ")));
        }
    }
    Ok(names)
}

// ---- CPU time per system, from Bevy's tracing spans -------------------------------------

/// Totals per span: nanoseconds and calls, by kind and name.
type Totals = BTreeMap<(&'static str, String), (u64, u64)>;

static TOTALS: Mutex<Totals> = Mutex::new(BTreeMap::new());
static RECORDING: AtomicBool = AtomicBool::new(false);
/// Nanoseconds the render thread spent waiting for the GPU since the last frame was counted.
static GPU_WAIT_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The log plugin of the headless app: with `--perf-systems`, one more layer that adds up
/// the time spent in each system's and schedule's span (it needs the `perf-trace` build).
pub fn log_plugin(cfg: &LaunchConfig, group: impl PluginGroup) -> PluginGroupBuilder {
    let builder = group.build();
    if cfg.perf.as_ref().is_some_and(|p| p.systems) {
        builder.set(LogPlugin { custom_layer: |_| Some(Box::new(span_layer::SpanTimes) as BoxedLayer), ..default() })
    } else {
        builder
    }
}

mod span_layer {
    use super::*;
    use bevy::log::tracing::span::{Attributes, Id};
    use bevy::log::tracing::{Subscriber, field::Field, field::Visit};
    use bevy::log::tracing_subscriber::Layer;
    use bevy::log::tracing_subscriber::layer::Context;
    use bevy::log::tracing_subscriber::registry::LookupSpan;

    pub struct SpanTimes;

    struct Named(&'static str, String);
    struct Entered(Instant);

    struct NameOf(String);
    impl Visit for NameOf {
        fn record_str(&mut self, f: &Field, v: &str) {
            if f.name() == "name" {
                self.0 = v.to_owned();
            }
        }
        fn record_debug(&mut self, f: &Field, v: &dyn std::fmt::Debug) {
            if f.name() == "name" && self.0.is_empty() {
                self.0 = format!("{v:?}");
            }
        }
    }

    impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for SpanTimes {
        fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
            let kind = match attrs.metadata().name() {
                "system" => "system",
                "system_commands" => "commands",
                "schedule" => "schedule",
                _ => return,
            };
            let mut n = NameOf(String::new());
            attrs.record(&mut n);
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(Named(kind, n.0));
            }
        }
        fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
            if !RECORDING.load(Ordering::Relaxed) {
                return;
            }
            if let Some(span) = ctx.span(id) {
                let mut ext = span.extensions_mut();
                if ext.get_mut::<Named>().is_some() {
                    ext.replace(Entered(Instant::now()));
                }
            }
        }
        fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
            if !RECORDING.load(Ordering::Relaxed) {
                return;
            }
            if let Some(span) = ctx.span(id) {
                let mut ext = span.extensions_mut();
                let Some(Entered(t)) = ext.remove::<Entered>() else { return };
                let Some((kind, name)) = ext.get_mut::<Named>().map(|n| (n.0, n.1.clone())) else { return };
                let ns = t.elapsed().as_nanos() as u64;
                if let Ok(mut m) = TOTALS.lock() {
                    let e = m.entry((kind, name)).or_default();
                    e.0 += ns;
                    e.1 += 1;
                }
            }
        }
    }
}

// ---- the run ---------------------------------------------------------------------------

/// One round of one variant.
#[derive(Default)]
struct Round {
    frame_ms: Vec<f64>,
    main_ms: Vec<f64>,
    /// Per frame: the render thread's wait for the GPU, milliseconds.
    wait_ms: Vec<f64>,
    /// GPU milliseconds per pass: (sum, samples).
    gpu: BTreeMap<String, (f64, u32)>,
    counts: Counts,
    /// Per span: milliseconds and calls per frame.
    systems: BTreeMap<String, (f64, f64)>,
}

#[derive(Default, Clone, Copy)]
struct Counts {
    entities: usize,
    visible_meshes: usize,
    rocks_visible: usize,
    shadow_active: bool,
}

#[derive(Resource)]
struct PerfRun {
    opts: PerfOpts,
    /// The variant of each round, in run order.
    plan: Vec<usize>,
    variants: Vec<(String, Vec<String>)>,
    at: usize,
    left_warm: u32,
    measuring: bool,
    applied: Option<usize>,
    round: Round,
    done: Vec<(usize, Round)>,
    started: Instant,
    last: Option<Instant>,
    main_start: Option<Instant>,
    seen_gpu: BTreeMap<String, Instant>,
    load_start: String,
    procs_start: Vec<(String, usize)>,
    scenery: Vec<String>,
    frame_no: u64,
}

pub(super) fn add(app: &mut App, opts: &PerfOpts, cfg: &LaunchConfig) {
    if opts.out.is_none() || cfg.unknown.iter().any(|u| u.starts_with("--perf")) {
        eprintln!("--perf needs an output path and valid --perf-* options: {}", cfg.unknown.join("; "));
        std::process::exit(2);
    }
    let mut variants = Vec::new();
    for v in &opts.variants {
        match parse_variant(v) {
            Ok(t) => variants.push((v.clone(), t.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        }
    }
    if opts.systems && !cfg!(feature = "perf-trace") {
        eprintln!("--perf-systems: this build has no span tracing (build sta64 with `--features perf-trace`); per-system times will be empty");
    }
    let plan = (0..opts.reps).flat_map(|_| 0..variants.len()).collect();
    app.insert_resource(PerfRun {
        opts: opts.clone(),
        plan,
        variants,
        at: 0,
        left_warm: opts.warmup,
        measuring: false,
        applied: None,
        round: Round::default(),
        done: Vec::new(),
        started: Instant::now(),
        last: None,
        main_start: None,
        seen_gpu: BTreeMap::new(),
        load_start: loadavg(),
        procs_start: other_processes(),
        scenery: Vec::new(),
        frame_no: 0,
    });
    app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
    if opts.sync
        && let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp)
    {
        render.add_systems(bevy::render::Render, wait_for_gpu.in_set(bevy::render::RenderSystems::Cleanup));
    }
    app.add_systems(First, |mut run: ResMut<PerfRun>| run.main_start = Some(Instant::now()))
        .add_systems(Update, apply_variant)
        .add_systems(PostUpdate, sweep.after(super::pair_view::place_camera).before(TransformSystems::Propagate))
        .add_systems(Last, drive);
}

/// The end of the render world's frame: wait until the GPU has finished it, so the frame
/// time is the slower of CPU and GPU, not the CPU's alone.
fn wait_for_gpu(device: Res<bevy::render::renderer::RenderDevice>) {
    let t = Instant::now();
    let _ = device.poll(bevy::render::render_resource::PollType::wait_indefinitely());
    GPU_WAIT_NS.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
}

fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg").map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

/// Other processes that compete for the machine: other `sta64` runs, compilers and the like,
/// by command name, from /proc.
fn other_processes() -> Vec<(String, usize)> {
    let me = std::process::id().to_string();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n == me || !n.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if let Ok(c) = std::fs::read_to_string(e.path().join("comm")) {
                let c = c.trim().to_owned();
                if ["sta64", "sta-bench", "rustc", "cargo", "cc1plus", "ld", "mold", "lld", "ffmpeg", "blender", "wineserver", "wine64"].contains(&c.as_str()) || c.starts_with("a2test") {
                    *counts.entry(c).or_default() += 1;
                }
            }
        }
    }
    counts.into_iter().collect()
}

/// Switch the scene to the current variant: classes hidden, settings set. Runs on a change.
#[allow(clippy::type_complexity)]
fn apply_variant(
    mut run: ResMut<PerfRun>,
    mut classes: Query<(
        &mut Visibility,
        Option<&crate::map::AsteroidField>,
        Option<&crate::render::nebula::NebulaField>,
        Option<&crate::render::nebula::NebulaBoards>,
        Option<&crate::map::Sky>,
        Option<&crate::render::planet::PlanetBody>,
    ), Or<(
        With<crate::map::AsteroidField>,
        With<crate::render::nebula::NebulaField>,
        With<crate::render::nebula::NebulaBoards>,
        With<crate::map::Sky>,
        With<crate::render::planet::PlanetBody>,
    )>>,
    fog: Option<ResMut<crate::render::fog::FogSettings>>,
    hulls: Option<ResMut<crate::render::hulls::HullLighting>>,
    quality: Option<ResMut<crate::render::quality::RenderQuality>>,
    switches: Option<ResMut<crate::render::quality::PerfSwitches>>,
) {
    // Every frame, not only on a change: the map spawns its objects over the first frames.
    let want = run.plan.get(run.at).copied();
    let Some(i) = want else { return };
    let toggles = run.variants[i].1.clone();
    let on = |t: &str| toggles.iter().any(|x| x == t);
    for (mut vis, field, neb, boards, sky, planet) in &mut classes {
        let hide = (field.is_some() && on("no-rocks"))
            || ((neb.is_some() || boards.is_some()) && on("no-nebula"))
            || (sky.is_some() && on("no-sky"))
            || (planet.is_some() && on("no-planets"));
        let want = if hide { Visibility::Hidden } else { Visibility::Inherited };
        if *vis != want {
            *vis = want;
        }
    }
    if let Some(mut f) = fog {
        let want = crate::render::fog::FogSettings { fog: !on("no-fog"), shroud: !on("no-fog") };
        if *f != want {
            *f = want;
        }
    }
    if let Some(mut h) = hulls {
        let want = !on("std-hulls");
        if h.on != want {
            h.on = want;
        }
    }
    if let Some(mut q) = quality {
        let d = crate::render::quality::RenderQuality::default();
        let want = crate::render::quality::RenderQuality {
            shadows: !on("no-shadows"),
            msaa: if on("msaa-off") { 1 } else if on("msaa-2") { 2 } else { d.msaa },
            depth_prepass: !on("no-prepass"),
            nebula_steps: if on("neb-steps8") { 8 } else { d.nebula_steps },
        };
        if *q != want {
            *q = want;
        }
    }
    if let Some(mut w) = switches {
        let want = crate::render::quality::PerfSwitches { freeze_rock_materials: on("freeze-rocks"), freeze_hull_materials: on("freeze-hulls") };
        if *w != want {
            *w = want;
        }
    }
    run.applied = want;
}

/// `--perf-camera sweep`: the camera swings about the ground point it looks at (every frame
/// from the scene's pose, so it does not drift), as a player panning does: what is keyed to
/// the camera's movement (rock materials, shadow fit, culling) is exercised.
#[allow(clippy::type_complexity)]
fn sweep(run: Res<PerfRun>, mut cameras: Query<(&mut Transform, &Camera), (With<Camera3d>, Without<crate::map::BackgroundCamera>)>) {
    if run.opts.camera != "sweep" {
        return;
    }
    let Some((mut tf, _)) = cameras.iter_mut().filter(|(_, c)| c.is_active).max_by_key(|(_, c)| c.order) else { return };
    let fwd = tf.forward().as_vec3();
    if fwd.y > -0.05 {
        return;
    }
    let pivot = tf.translation + fwd * (-tf.translation.y / fwd.y);
    // Phase from the frames of the round, so every round of every variant swings alike.
    let angle = 0.25 * (run.round.frame_ms.len() as f32 * 0.013).sin();
    let turn = Quat::from_rotation_y(angle);
    tf.translation = pivot + turn * (tf.translation - pivot);
    tf.rotation = turn * tf.rotation;
}

fn median(sorted: &[f64]) -> f64 {
    match sorted.len() {
        0 => 0.0,
        n if n % 2 == 1 => sorted[n / 2],
        n => (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0,
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[(((sorted.len() - 1) as f64) * p).round() as usize]
}

fn stats(v: &[f64]) -> String {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let mean = if s.is_empty() { 0.0 } else { s.iter().sum::<f64>() / s.len() as f64 };
    json::Obj::new()
        .num("mean", mean)
        .num("p50", percentile(&s, 0.5))
        .num("p95", percentile(&s, 0.95))
        .num("max", s.last().copied().unwrap_or(0.0))
        .num("min", s.first().copied().unwrap_or(0.0))
        .done()
}

/// The frame's driver: count, measure, advance the plan, write the report at the end.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive(
    mut run: ResMut<PerfRun>,
    diagnostics: Res<DiagnosticsStore>,
    visible: Query<(&ViewVisibility, Option<&crate::render::rock::RockPart>), With<Mesh3d>>,
    all: Query<Entity>,
    shadow_cam: Query<&Camera, With<crate::render::shadows::ShadowCamera>>,
    scenery: Query<(&GlobalTransform, Option<&crate::map::AsteroidField>, Option<&crate::render::nebula::NebulaField>, Option<&crate::render::nebula::NebulaBoards>, Option<&crate::render::planet::PlanetBody>)>,
    mut exit: MessageWriter<AppExit>,
    adapter: Option<Res<bevy::render::renderer::RenderAdapterInfo>>,
    device: Option<Res<bevy::render::renderer::RenderDevice>>,
) {
    run.frame_no += 1;
    let now = Instant::now();
    let main_ms = run.main_start.map(|t| now.duration_since(t).as_secs_f64() * 1e3);
    let dt = run.last.map(|t| now.duration_since(t).as_secs_f64() * 1e3);
    run.last = Some(now);
    let Some(&variant) = run.plan.get(run.at) else { return };
    if run.frame_no == 1 {
        return;
    }
    if !run.measuring {
        if run.left_warm > 0 {
            run.left_warm -= 1;
            return;
        }
        run.measuring = true;
        run.round = Round::default();
        if run.opts.systems {
            if let Ok(mut t) = TOTALS.lock() {
                t.clear();
            }
            RECORDING.store(true, Ordering::Relaxed);
        }
        run.seen_gpu.clear();
        GPU_WAIT_NS.store(0, Ordering::Relaxed);
        return;
    }
    if let (Some(dt), Some(m)) = (dt, main_ms) {
        run.round.frame_ms.push(dt);
        run.round.main_ms.push(m);
        run.round.wait_ms.push(GPU_WAIT_NS.swap(0, Ordering::Relaxed) as f64 / 1e6);
    }
    // GPU passes: each new measurement once.
    let mut fresh = Vec::new();
    for d in diagnostics.iter() {
        let path = d.path().as_str();
        if let Some(pass) = path.strip_suffix("/elapsed_gpu")
            && let Some(m) = d.measurement()
            && run.seen_gpu.get(pass) != Some(&m.time)
        {
            fresh.push((pass.trim_start_matches("render/").to_owned(), m.time, m.value));
        }
    }
    for (pass, t, v) in fresh {
        run.seen_gpu.insert(pass.clone(), t);
        let e = run.round.gpu.entry(pass).or_default();
        e.0 += v;
        e.1 += 1;
    }
    if run.round.frame_ms.len() < run.opts.frames as usize {
        return;
    }
    // The round is done: counts, systems.
    let mut counts = Counts { entities: all.iter().count(), shadow_active: shadow_cam.iter().any(|c| c.is_active), ..default() };
    for (vis, rock) in &visible {
        if vis.get() {
            counts.visible_meshes += 1;
            if rock.is_some() {
                counts.rocks_visible += 1;
            }
        }
    }
    run.round.counts = counts;
    if run.opts.systems {
        RECORDING.store(false, Ordering::Relaxed);
        let frames = run.round.frame_ms.len().max(1) as f64;
        let mut out = BTreeMap::new();
        if let Ok(t) = TOTALS.lock() {
            for ((kind, name), (ns, calls)) in t.iter() {
                out.insert(format!("{kind}: {name}"), (*ns as f64 / 1e6 / frames, *calls as f64 / frames));
            }
        }
        run.round.systems = out;
    }
    if run.scenery.is_empty() {
        let mut list = Vec::new();
        for (gt, field, neb, boards, planet) in &scenery {
            let kind = if field.is_some() {
                "asteroid field"
            } else if neb.is_some() {
                "nebula gas field"
            } else if boards.is_some() {
                "nebula billboards"
            } else if planet.is_some() {
                "planet"
            } else {
                continue;
            };
            let p = gt.translation();
            list.push(format!("{kind} at {:.0} {:.0} {:.0}", p.x, p.y, p.z));
        }
        run.scenery = list;
    }
    let round = std::mem::take(&mut run.round);
    run.done.push((variant, round));
    run.at += 1;
    run.measuring = false;
    run.left_warm = (run.opts.warmup / 3).max(15);
    if run.at >= run.plan.len() {
        let name = adapter.map(|a| format!("{} ({:?})", a.name, a.backend)).unwrap_or_default();
        let ts = device.is_some_and(|d| d.features().contains(bevy::render::settings::WgpuFeatures::TIMESTAMP_QUERY));
        match write_report(&run, &name, ts) {
            Ok(p) => {
                println!("perf report {}", p.display());
                exit.write(AppExit::Success);
            }
            Err(e) => {
                eprintln!("perf report: {e}");
                exit.write(AppExit::from_code(1));
            }
        }
    }
}

fn write_report(run: &PerfRun, adapter: &str, timestamps: bool) -> Result<PathBuf, String> {
    use json::{Arr, Obj};
    let out = run.opts.out.clone().ok_or("--perf needs an output path")?;
    let mut variants = Arr::new();
    for (vi, (name, toggles)) in run.variants.iter().enumerate() {
        let rounds: Vec<&Round> = run.done.iter().filter(|(v, _)| *v == vi).map(|(_, r)| r).collect();
        let mut reps = Arr::new();
        let mut means = Vec::new();
        for r in &rounds {
            means.push(r.frame_ms.iter().sum::<f64>() / r.frame_ms.len().max(1) as f64);
            let mut gpu = Obj::new();
            let mut gpu_total = 0.0;
            for (pass, (sum, n)) in &r.gpu {
                let ms = sum / f64::from((*n).max(1));
                gpu = gpu.num(pass, ms);
                // Pass spans can nest by path; the total adds the top-level ones only.
                if !pass.contains('/') {
                    gpu_total += ms;
                }
            }
            let mut systems = Obj::new();
            let mut sorted: Vec<_> = r.systems.iter().collect();
            sorted.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
            for (n, (ms, calls)) in sorted.into_iter().take(40) {
                systems = systems.raw(n, Obj::new().num("ms_per_frame", *ms).num("calls_per_frame", *calls).done());
            }
            reps.push(
                Obj::new()
                    .raw("frame_ms", stats(&r.frame_ms))
                    .raw("main_cpu_ms", stats(&r.main_ms))
                    .raw("gpu_wait_ms", stats(&r.wait_ms))
                    .raw("gpu_pass_ms", gpu.done())
                    .num("gpu_top_level_total_ms", gpu_total)
                    .raw("systems", systems.done())
                    .raw(
                        "counts",
                        Obj::new()
                            .int("entities", r.counts.entities as u64)
                            .int("visible_meshes", r.counts.visible_meshes as u64)
                                            .int("rocks_visible", r.counts.rocks_visible as u64)
                            .boolean("shadow_pass_active", r.counts.shadow_active)
                            .done(),
                    )
                    .done(),
            );
        }
        means.sort_by(f64::total_cmp);
        let med = median(&means);
        variants.push(
            Obj::new()
                .str("name", name)
                .raw("toggles", Arr::strs(toggles).done())
                .num("median_of_means_ms", med)
                .num("min_of_means_ms", means.first().copied().unwrap_or(0.0))
                .num("max_of_means_ms", means.last().copied().unwrap_or(0.0))
                .num("fps_median", if med > 0.0 { 1000.0 / med } else { 0.0 })
                .raw("reps", reps.done())
                .done(),
        );
    }
    let procs = |v: &[(String, usize)]| {
        let mut o = Obj::new();
        for (n, c) in v {
            o = o.int(n, *c as u64);
        }
        o.done()
    };
    let doc = Obj::new()
        .str("adapter", adapter)
        .num("elapsed_s", run.started.elapsed().as_secs_f64())
        .int("frames_per_round", u64::from(run.opts.frames))
        .int("warmup_frames", u64::from(run.opts.warmup))
        .int("reps", u64::from(run.opts.reps))
        .str("camera", &run.opts.camera)
        .boolean("gpu_synced_each_frame", run.opts.sync)
        .boolean("timestamp_queries_available", timestamps)
        .str("load_avg_start", &run.load_start)
        .str("load_avg_end", &loadavg())
        .raw("competing_processes_start", procs(&run.procs_start))
        .raw("competing_processes_end", procs(&other_processes()))
        .raw("scenery", Arr::strs(&run.scenery).done())
        .raw("variants", variants.done())
        .done();
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&out, doc).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(out)
}

/// A tiny JSON writer (the report is flat and small; no serializer dependency).
mod json {
    pub struct Obj(Vec<String>);
    pub struct Arr(Vec<String>);

    fn quote(s: &str) -> String {
        let mut o = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                c => o.push(c),
            }
        }
        o.push('"');
        o
    }

    impl Obj {
        pub fn new() -> Obj {
            Obj(Vec::new())
        }
        pub fn raw(mut self, k: &str, v: String) -> Obj {
            self.0.push(format!("{}:{v}", quote(k)));
            self
        }
        pub fn num(self, k: &str, v: f64) -> Obj {
            self.raw(k, if v.is_finite() { format!("{v:.4}") } else { "0".into() })
        }
        pub fn int(self, k: &str, v: u64) -> Obj {
            self.raw(k, v.to_string())
        }
        pub fn str(self, k: &str, v: &str) -> Obj {
            self.raw(k, quote(v))
        }
        pub fn boolean(self, k: &str, v: bool) -> Obj {
            self.raw(k, v.to_string())
        }
        pub fn done(self) -> String {
            format!("{{{}}}", self.0.join(","))
        }
    }

    impl Arr {
        pub fn new() -> Arr {
            Arr(Vec::new())
        }
        pub fn push(&mut self, v: String) {
            self.0.push(v);
        }
        pub fn strs(v: &[String]) -> Arr {
            Arr(v.iter().map(|s| quote(s)).collect())
        }
        pub fn done(self) -> String {
            format!("[{}]", self.0.join(","))
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn the_writer_escapes() {
            let s = super::Obj::new().str("a\"b", "x\ny").num("n", 1.5).done();
            assert_eq!(s, "{\"a\\\"b\":\"x\\ny\",\"n\":1.5000}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_are_plus_lists_of_known_toggles() {
        assert_eq!(parse_variant("base").unwrap(), Vec::<&str>::new());
        assert_eq!(parse_variant("no-rocks+no-nebula").unwrap(), ["no-rocks", "no-nebula"]);
        assert!(parse_variant("no-rocks+fast").is_err());
    }

    #[test]
    fn options_parse_and_reject_garbage() {
        let mut o = PerfOpts::default();
        o.set("--perf-frames", Some("120")).unwrap();
        o.set("--perf-variants", Some("base, no-rocks")).unwrap();
        o.set("--perf-no-sync", None).unwrap();
        assert_eq!((o.frames, o.variants.len(), o.sync), (120, 2, false));
        assert!(o.set("--perf-frames", Some("many")).is_err());
        assert!(o.set("--perf-camera", Some("spin")).is_err());
    }

    #[test]
    fn the_median_of_an_even_count_is_the_midpoint() {
        assert_eq!(median(&[1.0, 3.0]), 2.0);
        assert_eq!(median(&[1.0, 2.0, 9.0]), 2.0);
        assert_eq!(median(&[]), 0.0);
        let mut o = PerfOpts::default();
        assert!(o.set("--perf-variants", Some(",")).is_err());
    }

    #[test]
    fn percentiles_of_a_sorted_run() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&v, 0.5), 51.0);
        assert_eq!(percentile(&v, 0.95), 95.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }
}
