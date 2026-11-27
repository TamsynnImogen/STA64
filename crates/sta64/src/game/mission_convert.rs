//! The core of the in-game mission conversion (LUA-15j, umbrella #637; no UI).
//!
//! The game converts the original games' mission DLLs to Lua on the player's machine, from the
//! player's own install into the user data folder, with the converter library
//! (`sta_mission_convert::install`, LUA-15i). This module is the part that needs no screen:
//!
//! * [`decide`] says whether a run is allowed and where it reads and writes ([`Plan`]), or why
//!   not ([`Refusal`]): base game only (ADR 0004 D6: a mod's chain is never converted), some mission
//!   DLL in the install, an output and cache folder the converter accepts (never inside an install,
//!   a game folder or the checkout).
//! * A message [`StartConversion`] starts a run (`forced` ignores the cache and the stamp), a
//!   message [`CancelConversion`] asks the running one to stop between two missions. The run is one
//!   `std::thread` that owns the output folder writer, the cache and the decoder and calls
//!   `convert_install` with `prepare_first` (a cancel at the start, or a discovery fault, leaves the
//!   earlier conversion in place). No Bevy type enters the thread; its [`Event`]s travel through a
//!   channel and a system drains at most [`MAX_EVENTS_PER_FRAME`] of them per frame into the
//!   [`ConversionState`] resource.
//! * A run ends with a [`ConversionFinished`] message carrying its [`Outcome`]: what the front end
//!   (LUA-15k) listens for to run the campaign load again ([`Outcome::changed`]). A worker thread that
//!   dies (a panic in the lifter on hostile bytes) is reported as an aborted run.
//! * A *stamp* file beside the output skips a run whose inputs (the build, the names and sizes of the
//!   mission DLLs, the engine files and the map files) are the ones of the last complete run and
//!   whose output still has its manifest. The stamp only skips: the cache stays the authority for
//!   what a run converts, and it is removed when a run starts and written only when one completes, so a
//!   cancelled run never leaves a stamp that matches.
//! * A *lock* file beside the output (an advisory lock held by the run, released by the system if the
//!   process dies) keeps two game processes from converting one install at once: the second finds the
//!   lock taken and ends with [`Outcome::Busy`], touching nothing. Where locking is not available
//!   (some network file systems) the run goes on unlocked, which is the command line's limit.
//!
//! DLLs are only read as bytes, never loaded or run (D2); converted Lua runs in the same restricted
//! state as every script (D13). Nothing here is wired to a screen; whether a run also starts
//! by itself is the front end's decision (LUA-15k).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use bevy::prelude::*;
use sta_mission_convert::cache::{BUILD_ID, Cache};
use sta_mission_convert::install::{self, Env, Event as ConvEvent, Install, InstallError, Options, Summary};
use sta_mission_convert::output::{MANIFEST, OutDir};

/// How many worker messages one frame takes at most; the rest wait for the next frame.
pub const MAX_EVENTS_PER_FRAME: usize = 64;

// ------------------------------------------------------------------ deciding

/// A run that is allowed: where it reads, what it writes and what guards it.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The install, opened (the base game's own missions folder, never a mod's).
    pub install: Install,
    /// `<slug>-<hash>`: the folder key of the install ([`sta_core::install_key`]).
    pub key: String,
    /// `<data>/converted/<key>`: where the game mounts converted missions (the folder as written).
    pub out_dir: PathBuf,
    /// The same folder, resolved and checked.
    pub out_res: PathBuf,
    /// `<data>/convert-cache/<key>`.
    pub cache_dir: PathBuf,
    /// `<data>/converted/<key>.stamp`: the stamp of the last complete run.
    pub stamp_path: PathBuf,
    /// `<data>/converted/<key>.lock`: the lock of a running conversion.
    pub lock_path: PathBuf,
    env: Env,
}

/// Why no run is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A mod is active (D6): the conversion covers the base game, which stays mounted.
    ModActive(String),
    /// No user data folder is known.
    NoDataDir,
    /// The folder is not an install the converter can read missions from.
    Install(InstallError),
    /// The install has no mission DLL: nothing to convert.
    NoMissionDlls,
    /// The output folder is refused (inside an install, a game folder or the checkout, or it
    /// cannot be resolved); the text names the folder and why.
    Output(String),
    /// The cache folder is refused.
    Cache(String),
}

impl Refusal {
    /// A refusal the player is told about; the others are the normal "nothing to do here".
    pub fn is_fault(&self) -> bool {
        matches!(self, Refusal::Output(_) | Refusal::Cache(_) | Refusal::NoDataDir | Refusal::Install(_))
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::ModActive(m) => write!(f, "the mod {m} is active: only the base game's missions are converted"),
            Refusal::NoDataDir => f.write_str("no user data folder is known to keep converted missions in"),
            Refusal::Install(e) => write!(f, "this folder has no missions to convert: {}", install_text(e)),
            Refusal::NoMissionDlls => f.write_str("the install has no mission DLLs to convert"),
            Refusal::Output(m) | Refusal::Cache(m) => f.write_str(m),
        }
    }
}

fn install_text(e: &InstallError) -> String {
    match e {
        InstallError::NotAFolder { path } => format!("{} is not a folder", path.display()),
        InstallError::Unreadable { root, reason, .. } => format!("{}: {reason}", root.display()),
        InstallError::FleetOperations { .. } => "a Fleet Operations style install ships Lua missions".to_owned(),
        InstallError::NotArmada { root, .. } => format!("{} is neither an Armada I nor an Armada II install", root.display()),
        InstallError::NoMissionsFolder { root, .. } => format!("{} has no missions folder", root.display()),
    }
}

/// Whether a run may be made for `install_root` with `active_mod` active, writing below `data_dir`,
/// and where. Reads folder listings only. Whether a *particular* mission still needs its script is
/// the campaign list's knowledge (`Campaigns::awaits_conversion`); this answers whether a run is
/// possible and allowed.
pub fn decide(install_root: &Path, active_mod: Option<&str>, data_dir: Option<&Path>) -> Result<Plan, Refusal> {
    decide_with(install_root, active_mod, data_dir, &Env::from_process())
}

/// [`decide`] with the surroundings given (the checkout the output must stay out of, ...).
pub fn decide_with(install_root: &Path, active_mod: Option<&str>, data_dir: Option<&Path>, env: &Env) -> Result<Plan, Refusal> {
    if let Some(m) = active_mod.filter(|m| !m.is_empty()) {
        return Err(Refusal::ModActive(m.to_owned()));
    }
    let data = data_dir.ok_or(Refusal::NoDataDir)?;
    let install = Install::open(install_root).map_err(Refusal::Install)?;
    if sta_mission_convert::convert::find_missions(&install.missions_dir).is_empty() {
        return Err(Refusal::NoMissionDlls);
    }
    let out_dir = sta_mission::converted::dir_in(data, &install.root, None);
    let out_res = install::resolve_output(&out_dir, &install, env).map_err(Refusal::Output)?;
    let cache_dir = sta_mission::converted::cache_dir_in(data, &install.root);
    install::open_cache(&cache_dir, &out_res, &install, env).map_err(Refusal::Cache)?;
    let key = sta_core::install_key::key(&install.root, None);
    let beside = |ext: &str| data.join("converted").join(format!("{key}.{ext}"));
    Ok(Plan {
        stamp_path: beside("stamp"),
        lock_path: beside("lock"),
        install,
        key: key.clone(),
        out_dir,
        out_res,
        cache_dir,
        env: env.clone(),
    })
}

// ------------------------------------------------------------------ the stamp

fn sorted_files(dir: &Path, keep: &dyn Fn(&str) -> bool) -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            let meta = e.metadata().ok()?;
            (meta.is_file() && keep(&name)).then_some((name, meta.len()))
        })
        .collect();
    v.sort();
    v
}

/// What a run's result depends on, as far as a listing tells: the build, then the names (relative
/// to the missions folder) and sizes of the mission DLLs, the engine files and the map files.
fn stamp_text(plan: &Plan) -> String {
    let mut s = format!("sta64-convert-stamp 1\nbuild {BUILD_ID}\nkey {}\n", plan.key);
    let missions = &plan.install.missions_dir;
    for (dsl, drl) in sta_mission_convert::convert::find_missions(missions) {
        for p in std::iter::once(dsl).chain(drl) {
            let rel = p.strip_prefix(missions).unwrap_or(&p).to_string_lossy().replace('\\', "/").to_ascii_lowercase();
            let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            s.push_str(&format!("dll {rel} {len}\n"));
        }
    }
    for (n, len) in sorted_files(&plan.install.root, &|n| n == "armada2.exe" || n == "armada2.map") {
        s.push_str(&format!("engine {n} {len}\n"));
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    for base in [missions.parent(), Some(plan.install.root.as_path())].into_iter().flatten() {
        if let Some(b) = install::child(base, "bzn")
            && !dirs.contains(&b)
        {
            dirs.push(b);
        }
    }
    for d in dirs {
        for (n, len) in sorted_files(&d, &|n| n.ends_with(".bzn")) {
            s.push_str(&format!("map {n} {len}\n"));
        }
    }
    s
}

/// The stamp file of a complete run that wrote `written` scripts: the inputs, then the count.
fn stamp_file(plan: &Plan, written: usize) -> String {
    format!("{}written {written}\n", stamp_text(plan))
}

/// Whether the last complete run's inputs are today's and its output is still there: a run that
/// wrote scripts needs its manifest (the player may have deleted the folder), a run that wrote
/// none (every mission failed) has nothing to lose.
fn stamp_matches(plan: &Plan) -> bool {
    let Ok(text) = std::fs::read_to_string(&plan.stamp_path) else { return false };
    let Some(rest) = text.strip_prefix(&stamp_text(plan)) else { return false };
    match rest.strip_prefix("written ").and_then(|n| n.trim().parse::<usize>().ok()) {
        Some(0) => true,
        Some(_) => plan.out_res.join(MANIFEST).is_file(),
        None => false,
    }
}

// ------------------------------------------------------------------ the lock

enum Lock {
    /// Held until dropped.
    Held(#[allow(dead_code)] std::fs::File),
    /// Locking is not available here: the run goes on without.
    Unlocked,
    /// Another process holds it.
    Busy,
}

fn take_lock(path: &Path) -> Lock {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(file) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path) else {
        return Lock::Unlocked;
    };
    match file.try_lock() {
        Ok(()) => Lock::Held(file),
        Err(std::fs::TryLockError::WouldBlock) => Lock::Busy,
        Err(std::fs::TryLockError::Error(_)) => Lock::Unlocked,
    }
}

// ------------------------------------------------------------------ the worker

/// The conversion itself, as the worker calls it: `install::convert_install` with the decoder of the
/// converter. A resource so that tests can put a fake in its place.
pub type ConverterFn = dyn Fn(&Install, &Options, &mut OutDir, Option<&Cache>, &dyn Fn() -> bool, &mut dyn FnMut(&ConvEvent)) -> Summary
    + Send
    + Sync;

fn real_converter(
    install: &Install,
    opts: &Options,
    out: &mut OutDir,
    cache: Option<&Cache>,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(&ConvEvent),
) -> Summary {
    let mut decoder = |_: &Path, pe: &sta_mission_convert::Pe| sta_mission_convert::disasm::disassemble(pe);
    install::convert_install(install, opts, out, cache, &mut decoder, cancel, sink)
}

enum Done {
    Summary(Box<Summary>),
    Skipped,
    Busy,
    Aborted(String),
}

enum Msg {
    Event(ConvEvent),
    Done(Done),
}

fn work(plan: Plan, forced: bool, cancel: Arc<AtomicBool>, tx: Sender<Msg>, converter: Arc<ConverterFn>) {
    let done = run(&plan, forced, &cancel, &tx, &*converter);
    let _ = tx.send(Msg::Done(done));
}

fn run(plan: &Plan, forced: bool, cancel: &AtomicBool, tx: &Sender<Msg>, converter: &ConverterFn) -> Done {
    let _lock = match take_lock(&plan.lock_path) {
        Lock::Busy => return Done::Busy,
        other => other,
    };
    if !forced && stamp_matches(plan) {
        return Done::Skipped;
    }
    // From here the folder changes: no stamp stays that could skip after a stop.
    let _ = std::fs::remove_file(&plan.stamp_path);
    // A forced run goes without the cache; a cache folder that cannot be opened is converted around.
    let cache = if forced { None } else { install::open_cache(&plan.cache_dir, &plan.out_res, &plan.install, &plan.env).ok() };
    let mut out = match OutDir::inspect(&plan.out_dir, plan.out_res.clone(), false) {
        Ok(o) => o,
        Err(m) => return Done::Aborted(m),
    };
    let opts = Options { prepare_first: true, ..Options::default() };
    let stop = || cancel.load(Ordering::Relaxed);
    // A game that no longer listens (it is closing) stops the run.
    let mut sink = |e: &ConvEvent| {
        if tx.send(Msg::Event(e.clone())).is_err() {
            cancel.store(true, Ordering::Relaxed);
        }
    };
    let summary = converter(&plan.install, &opts, &mut out, cache.as_ref(), &stop, &mut sink);
    if summary.completed() {
        let written = summary.missions.iter().map(|m| m.written.len()).sum();
        let _ = std::fs::write(&plan.stamp_path, stamp_file(plan, written));
    }
    Done::Summary(Box::new(summary))
}

// ------------------------------------------------------------------ what the game sees

/// Start a run for the install, for the base game (`active_mod` is passed so that a mod is refused
/// here, D6, whatever the sender knows). `forced`: ignore the cache and the stamp (convert again).
/// Ignored while a run is going.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct StartConversion {
    pub install_root: PathBuf,
    pub active_mod: Option<String>,
    pub forced: bool,
}

/// Stop the running conversion between two missions (what was written stays).
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelConversion;

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Ran to the end (some missions may have failed: see [`ConversionState::failures`]).
    Completed { total: usize, converted: usize, cached: usize, failed: usize },
    /// Stopped by [`CancelConversion`]; what the missions before the stop wrote stays.
    Cancelled { converted: usize },
    /// Stopped by a fault (or the worker thread died); the text says why.
    Aborted(String),
    /// The stamp matched: nothing was done.
    Skipped,
    /// Another game process is converting this install: nothing was done.
    Busy,
    /// [`decide`] said no: nothing was done.
    Refused(Refusal),
}

impl Outcome {
    /// Whether the output folder may be different now, so the campaigns should be loaded again.
    pub fn changed(&self) -> bool {
        matches!(self, Outcome::Completed { .. } | Outcome::Cancelled { .. } | Outcome::Aborted(_))
    }
}

/// A run ended (also one that was refused, skipped or found the install busy). The front end runs
/// the campaign load again when [`Outcome::changed`].
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct ConversionFinished {
    pub outcome: Outcome,
}

/// What the run in progress (or the last one) has reported so far.
#[derive(Resource, Debug, Default, Clone)]
pub struct ConversionState {
    pub running: bool,
    /// Missions planned (0 until the plan is known).
    pub total: usize,
    /// Missions finished.
    pub done: usize,
    /// The mission being converted.
    pub current: Option<String>,
    /// `(mission, message)` of every failed mission, in order.
    pub failures: Vec<(String, String)>,
    /// Files of the Python tool moved aside at the start: `(old, new)`.
    pub moved_aside: Vec<(String, String)>,
    /// Worker messages taken so far in this run.
    pub events_seen: usize,
    /// How the last run ended; `None` before the first run and while one is going.
    pub outcome: Option<Outcome>,
    /// The converter's own summary of the last run, when it got that far.
    pub summary: Option<Summary>,
    /// Runs started since the game began.
    pub runs: u32,
}

struct Worker {
    rx: Mutex<Receiver<Msg>>,
    handle: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
}

/// The run in progress.
#[derive(Resource, Default)]
pub struct ActiveRun(Option<Worker>);

impl ActiveRun {
    pub fn is_running(&self) -> bool {
        self.0.is_some()
    }
}

/// The surroundings of a run: where the user data lives and which converter runs. The default is the
/// process' own (`platform::data_dir`, the converter's decoder); tests replace it.
#[derive(Resource, Clone)]
pub struct ConvertContext {
    pub data_dir: Option<PathBuf>,
    pub env: Env,
    pub converter: Arc<ConverterFn>,
}

impl Default for ConvertContext {
    fn default() -> Self {
        ConvertContext { data_dir: crate::platform::data_dir(), env: Env::from_process(), converter: Arc::new(real_converter) }
    }
}

fn outcome_of(s: &Summary) -> Outcome {
    if let Some(a) = &s.aborted {
        Outcome::Aborted(a.message.clone())
    } else if s.cancelled {
        Outcome::Cancelled { converted: s.converted_missions() }
    } else if let Some(m) = &s.finish_error {
        Outcome::Aborted(format!("the manifest could not be saved: {m}"))
    } else {
        Outcome::Completed {
            total: s.total,
            converted: s.converted_missions(),
            cached: s.missions.iter().filter(|m| m.cached).count(),
            failed: s.failed_missions(),
        }
    }
}

fn finish(state: &mut ConversionState, fin: &mut MessageWriter<ConversionFinished>, outcome: Outcome) {
    state.running = false;
    state.current = None;
    state.outcome = Some(outcome.clone());
    info!("mission conversion: {outcome:?}");
    fin.write(ConversionFinished { outcome });
}

fn start(
    mut requests: MessageReader<StartConversion>,
    ctx: Res<ConvertContext>,
    mut state: ResMut<ConversionState>,
    mut active: ResMut<ActiveRun>,
    mut fin: MessageWriter<ConversionFinished>,
) {
    for req in requests.read() {
        if active.is_running() {
            continue;
        }
        let plan = match decide_with(&req.install_root, req.active_mod.as_deref(), ctx.data_dir.as_deref(), &ctx.env) {
            Ok(p) => p,
            Err(r) => {
                *state = ConversionState { runs: state.runs, ..ConversionState::default() };
                finish(&mut state, &mut fin, Outcome::Refused(r));
                continue;
            }
        };
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (flag, converter, forced) = (cancel.clone(), ctx.converter.clone(), req.forced);
        let spawned = std::thread::Builder::new().name("mission-convert".into()).spawn(move || work(plan, forced, flag, tx, converter));
        *state = ConversionState { runs: state.runs + 1, running: true, ..ConversionState::default() };
        match spawned {
            Ok(handle) => active.0 = Some(Worker { rx: Mutex::new(rx), handle: Some(handle), cancel }),
            Err(e) => finish(&mut state, &mut fin, Outcome::Aborted(format!("the conversion thread could not be started: {e}"))),
        }
    }
}

fn cancel(mut requests: MessageReader<CancelConversion>, active: Res<ActiveRun>) {
    if requests.read().next().is_some()
        && let Some(w) = &active.0
    {
        w.cancel.store(true, Ordering::Relaxed);
    }
}

/// Takes up to [`MAX_EVENTS_PER_FRAME`] worker messages into the state; a worker that is gone
/// without having said it is done is an aborted run.
fn drain(mut state: ResMut<ConversionState>, mut active: ResMut<ActiveRun>, mut fin: MessageWriter<ConversionFinished>) {
    let Some(worker) = &mut active.0 else { return };
    let mut ended: Option<Outcome> = None;
    let mut summary: Option<Summary> = None;
    {
        let rx = match worker.rx.lock() {
            Ok(r) => r,
            Err(p) => p.into_inner(),
        };
        for _ in 0..MAX_EVENTS_PER_FRAME {
            match rx.try_recv() {
                Ok(Msg::Event(e)) => {
                    state.events_seen += 1;
                    apply(&mut state, e);
                }
                Ok(Msg::Done(d)) => {
                    state.events_seen += 1;
                    ended = Some(match d {
                        Done::Summary(s) => {
                            let o = outcome_of(&s);
                            summary = Some(*s);
                            o
                        }
                        Done::Skipped => Outcome::Skipped,
                        Done::Busy => Outcome::Busy,
                        Done::Aborted(m) => Outcome::Aborted(m),
                    });
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // The thread is gone without a result: it panicked (a lifter fault on hostile
                    // bytes). What was written stays; the next full run repairs the manifest.
                    let why = match worker.handle.take().map(JoinHandle::join) {
                        Some(Err(p)) => panic_text(p.as_ref()),
                        _ => "it ended without a result".to_owned(),
                    };
                    ended = Some(Outcome::Aborted(format!("the conversion stopped unexpectedly: {why}")));
                    break;
                }
            }
        }
    }
    if let Some(o) = ended {
        active.0 = None;
        state.summary = summary;
        finish(&mut state, &mut fin, o);
    }
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| (*s).to_owned()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "a panic".to_owned())
}

fn apply(state: &mut ConversionState, e: ConvEvent) {
    match e {
        ConvEvent::MovedAside { old, new } => state.moved_aside.push((old, new)),
        ConvEvent::Planned { total } => state.total = total,
        ConvEvent::Started { mission, .. } => state.current = Some(mission),
        ConvEvent::Console(_) => {}
        ConvEvent::Failed { mission, message, .. } => state.failures.push((mission, message)),
        ConvEvent::Done { index, .. } => state.done = index + 1,
    }
}

pub struct MissionConvertPlugin;

impl Plugin for MissionConvertPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<StartConversion>()
            .add_message::<CancelConversion>()
            .add_message::<ConversionFinished>()
            .init_resource::<ConversionState>()
            .init_resource::<ActiveRun>()
            .init_resource::<ConvertContext>()
            .add_systems(Update, (start, cancel, drain).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc::sync_channel;

    use sta_mission_convert::cache::Entry;

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(name: &str) -> Tmp {
            static N: AtomicUsize = AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!("sta64-mc-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn put(p: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    /// An Armada I root of invented files: the executable, one pair of junk DLLs and a map naming it.
    fn install(root: &Path, dll_bytes: &[u8]) {
        put(&root.join("Armada.exe"), b"");
        put(&root.join("missions/x1S.dsl"), dll_bytes);
        put(&root.join("missions/x1R.drl"), dll_bytes);
        put(&root.join("bzn/fed1.bzn"), b"xx x1R.drl yy");
    }

    fn env() -> Env {
        Env::default()
    }

    // ---- decide

    #[test]
    fn decide_table() {
        let t = Tmp::new("decide");
        let (root, data) = (t.0.join("Star Trek Armada"), t.0.join("data"));
        install(&root, b"junk");
        // the base game with DLLs and a data folder outside the install
        let plan = decide_with(&root, None, Some(&data), &env()).unwrap();
        assert_eq!(plan.out_dir, sta_mission::converted::dir_in(&data, &root, None), "the folder the game mounts");
        assert_eq!(plan.cache_dir, sta_mission::converted::cache_dir_in(&data, &root));
        assert_eq!(plan.stamp_path, data.join("converted").join(format!("{}.stamp", plan.key)));
        assert_eq!(plan.lock_path, data.join("converted").join(format!("{}.lock", plan.key)));
        assert!(plan.key.starts_with("star-trek-armada-"), "{}", plan.key);
        assert_eq!(plan.install.root, install::realpath(&root));
        // an empty mod name is no mod
        assert!(decide_with(&root, Some(""), Some(&data), &env()).is_ok());
        // a mod is active: base game only (D6)
        assert_eq!(decide_with(&root, Some("Fleet Ops"), Some(&data), &env()).unwrap_err(), Refusal::ModActive("Fleet Ops".into()));
        // no data folder
        assert_eq!(decide_with(&root, None, None, &env()).unwrap_err(), Refusal::NoDataDir);
        // nothing to convert: an install without mission DLLs
        let bare = t.0.join("Bare");
        put(&bare.join("Armada.exe"), b"");
        std::fs::create_dir_all(bare.join("missions")).unwrap();
        put(&bare.join("missions/own.lua"), b"-- authored\n");
        assert_eq!(decide_with(&bare, None, Some(&data), &env()).unwrap_err(), Refusal::NoMissionDlls);
        // a folder that is no install, and one that does not exist
        let other = t.0.join("Other");
        std::fs::create_dir_all(&other).unwrap();
        assert!(matches!(decide_with(&other, None, Some(&data), &env()), Err(Refusal::Install(InstallError::NotArmada { .. }))));
        let gone = t.0.join("gone");
        assert!(matches!(decide_with(&gone, None, Some(&data), &env()), Err(Refusal::Install(InstallError::NotAFolder { .. }))));
        // a data folder inside the install (a portable STA64_DATA_DIR): refused with the reason
        let inside = root.join("userdata");
        let r = decide_with(&root, None, Some(&inside), &env()).unwrap_err();
        assert!(matches!(&r, Refusal::Output(m) if m.contains("inside the install")), "{r:?}");
        assert!(r.is_fault() && !Refusal::NoMissionDlls.is_fault() && !Refusal::ModActive("x".into()).is_fault());
        // the checkout is refused
        let repo = t.0.join("checkout");
        std::fs::create_dir_all(&repo).unwrap();
        let env = Env { repo_root: Some(repo.clone()), ..Env::default() };
        let r = decide_with(&root, None, Some(&repo.join("data")), &env).unwrap_err();
        assert!(matches!(&r, Refusal::Output(m) if m.contains("repository")), "{r:?}");
        // every refusal has a text
        for r in [Refusal::NoDataDir, Refusal::NoMissionDlls, Refusal::ModActive("m".into())] {
            assert!(!r.to_string().is_empty());
        }
    }

    // ---- the worker through an App

    type Fake = dyn Fn(&Install, &Options, &mut OutDir, Option<&Cache>, &dyn Fn() -> bool, &mut dyn FnMut(&ConvEvent)) -> Summary
        + Send
        + Sync;

    /// A closure as the converter (spelled out so that its argument types are inferred).
    fn fake<F>(f: F) -> Arc<Fake>
    where
        F: Fn(&Install, &Options, &mut OutDir, Option<&Cache>, &dyn Fn() -> bool, &mut dyn FnMut(&ConvEvent)) -> Summary
            + Send
            + Sync
            + 'static,
    {
        Arc::new(f)
    }

    #[derive(Resource, Default)]
    struct Seen(Vec<ConversionFinished>);

    fn collect(mut r: MessageReader<ConversionFinished>, mut seen: ResMut<Seen>) {
        seen.0.extend(r.read().cloned());
    }

    fn app(data: &Path, converter: Arc<Fake>) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(MissionConvertPlugin)
            .init_resource::<Seen>()
            .add_systems(Update, collect.after(drain));
        app.insert_resource(ConvertContext { data_dir: Some(data.to_path_buf()), env: Env::default(), converter });
        app
    }

    fn request(app: &mut App, root: &Path, forced: bool) {
        app.world_mut().write_message(StartConversion { install_root: root.to_path_buf(), active_mod: None, forced });
    }

    /// Updates until the run is over (a bound on the loop, not on time).
    fn finish_run(app: &mut App) {
        for _ in 0..2_000_000 {
            app.update();
            if !app.world().resource::<ConversionState>().running {
                return;
            }
            std::thread::yield_now();
        }
        panic!("the run did not end");
    }

    fn state(app: &App) -> ConversionState {
        app.world().resource::<ConversionState>().clone()
    }

    fn seen(app: &App) -> Vec<Outcome> {
        app.world().resource::<Seen>().0.iter().map(|f| f.outcome.clone()).collect()
    }

    /// A converter that reports `n` missions, converts none and saves the manifest.
    fn walks(n: usize) -> Arc<Fake> {
        fake(move |_i, _o, out, _c, cancel, sink| {
            let mut s = Summary::default();
            sink(&ConvEvent::Planned { total: n });
            s.total = n;
            for index in 0..n {
                if cancel() {
                    s.cancelled = true;
                    break;
                }
                let mission = format!("m{index}");
                sink(&ConvEvent::Started { index, total: n, mission: mission.clone() });
                if index == 1 {
                    sink(&ConvEvent::Failed { mission: mission.clone(), name: mission.clone(), message: "no".into() });
                }
                sink(&ConvEvent::Done { index, total: n, mission });
            }
            let _ = out.finish();
            s
        })
    }

    #[test]
    fn events_arrive_in_order_and_the_run_ends_with_one_message() {
        let t = Tmp::new("order");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let mut app = app(&data, walks(3));
        request(&mut app, &root, false);
        app.update();
        assert!(state(&app).running || state(&app).outcome.is_some());
        finish_run(&mut app);
        let s = state(&app);
        assert_eq!((s.total, s.done, s.running), (3, 3, false));
        assert_eq!(s.failures, vec![("m1".to_owned(), "no".to_owned())]);
        assert_eq!(s.events_seen, 1 + 3 + 3 + 1 + 1, "Planned, 3 Started, 3 Done, 1 Failed and the result");
        assert_eq!(s.runs, 1);
        assert_eq!(seen(&app), vec![Outcome::Completed { total: 3, converted: 0, cached: 0, failed: 0 }]);
        assert!(seen(&app)[0].changed());
        assert!(!app.world().resource::<ActiveRun>().is_running());
    }

    #[test]
    fn a_refusal_is_a_message_and_starts_nothing() {
        let t = Tmp::new("refuse");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let mut app = app(&data, walks(1));
        app.world_mut().write_message(StartConversion { install_root: root.clone(), active_mod: Some("Mod".into()), forced: false });
        app.update();
        assert_eq!(seen(&app), vec![Outcome::Refused(Refusal::ModActive("Mod".into()))]);
        assert!(!state(&app).running && !app.world().resource::<ActiveRun>().is_running());
        assert!(!Outcome::Refused(Refusal::NoDataDir).changed());
        assert!(!data.exists(), "a refusal writes nothing");
    }

    #[test]
    fn cancel_is_read_by_the_worker_between_missions() {
        let t = Tmp::new("cancel");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let (gate_tx, gate_rx) = sync_channel::<()>(0);
        let gate = Mutex::new(gate_rx);
        let conv = fake(move |_i, _o, out, _c, cancel, sink| {
            let mut s = Summary { total: 2, ..Summary::default() };
            sink(&ConvEvent::Planned { total: 2 });
            sink(&ConvEvent::Started { index: 0, total: 2, mission: "first".into() });
            // the test cancels while the first mission is "running"
            gate.lock().unwrap().recv().unwrap();
            sink(&ConvEvent::Done { index: 0, total: 2, mission: "first".into() });
            s.cancelled = cancel();
            let _ = out.finish();
            s
        });
        let mut app = app(&data, conv);
        request(&mut app, &root, false);
        app.update();
        app.world_mut().write_message(CancelConversion);
        app.update();
        gate_tx.send(()).unwrap();
        finish_run(&mut app);
        assert_eq!(seen(&app), vec![Outcome::Cancelled { converted: 0 }]);
        assert!(seen(&app)[0].changed());
        assert!(state(&app).summary.unwrap().cancelled);
        // a cancel with nothing running does nothing
        app.world_mut().write_message(CancelConversion);
        app.update();
        assert_eq!(seen(&app).len(), 1);
    }

    #[test]
    fn an_abort_is_reported_with_its_reason() {
        let t = Tmp::new("abort");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let conv = fake(|_i, _o, _out, _c, _cancel, _sink| Summary {
            aborted: Some(install::Abort { stage: install::Stage::Convert, message: "no write permission".into() }),
            ..Summary::default()
        });
        let mut app = app(&data, conv);
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(seen(&app), vec![Outcome::Aborted("no write permission".into())]);
        assert!(state(&app).summary.is_some());
        // an output folder that holds a foreign script is refused before the converter is called
        put(&data.join("converted").join(&decide_with(&root, None, Some(&data), &env()).unwrap().key).join("missions/mine.lua"), b"-- mine\n");
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert!(matches!(&seen(&app)[1], Outcome::Aborted(m) if m.contains("did not write")), "{:?}", seen(&app));
    }

    #[test]
    fn a_panicking_converter_is_an_aborted_run() {
        let t = Tmp::new("panic");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let conv = fake(|_i, _o, _out, _c, _cancel, sink| {
            sink(&ConvEvent::Planned { total: 1 });
            panic!("lifter fault on hostile bytes");
        });
        let mut app = app(&data, conv);
        request(&mut app, &root, false);
        finish_run(&mut app);
        let o = seen(&app);
        assert!(matches!(&o[0], Outcome::Aborted(m) if m.contains("stopped unexpectedly") && m.contains("hostile bytes")), "{o:?}");
        assert!(o[0].changed());
        assert!(!app.world().resource::<ActiveRun>().is_running());
        // no stamp after a dead worker; a new run is possible
        let plan = decide_with(&root, None, Some(&data), &env()).unwrap();
        assert!(!plan.stamp_path.exists());
        let conv2: Arc<Fake> = walks(1);
        app.insert_resource(ConvertContext { data_dir: Some(data.clone()), env: Env::default(), converter: conv2 });
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert!(matches!(seen(&app)[1], Outcome::Completed { total: 1, .. }));
        assert_eq!(state(&app).runs, 2);
    }

    #[test]
    fn a_frame_takes_at_most_the_bound() {
        let t = Tmp::new("bound");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let n = MAX_EVENTS_PER_FRAME * 3;
        let sent = Arc::new(AtomicBool::new(false));
        let flag = sent.clone();
        let conv = fake(move |_i, _o, out, _c, _cancel, sink| {
            for k in 0..n {
                sink(&ConvEvent::Console(format!("line {k}")));
            }
            let _ = out.finish();
            flag.store(true, Ordering::SeqCst);
            Summary::default()
        });
        let mut app = app(&data, conv);
        request(&mut app, &root, false);
        // the start system runs in this update; the drain finds nothing yet or little: wait for the
        // worker to have queued everything, then count what one frame takes
        app.update();
        while !sent.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        let before = state(&app).events_seen;
        app.update();
        let after = state(&app).events_seen;
        assert!(after - before <= MAX_EVENTS_PER_FRAME, "{before} -> {after}");
        assert!(state(&app).running, "{n} events are more than one frame takes");
        finish_run(&mut app);
        assert_eq!(state(&app).events_seen, n + 1, "every event arrives, then the result");
    }

    // ---- the stamp and the lock

    /// A converter that counts its calls and writes one script (the manifest with it).
    fn counting(calls: Arc<AtomicUsize>) -> Arc<Fake> {
        fake(move |_i, _o, out, _c, _cancel, _sink| {
            calls.fetch_add(1, Ordering::SeqCst);
            out.write("fed1.lua", b"-- authored\n").unwrap();
            let _ = out.finish();
            let one = install::MissionResult {
                mission: "fed1".into(),
                converted: true,
                written: vec!["fed1.lua".into()],
                ..install::MissionResult::default()
            };
            Summary { total: 1, missions: vec![one], ..Summary::default() }
        })
    }

    #[test]
    fn the_stamp_skips_a_matching_run_and_only_a_matching_one() {
        let t = Tmp::new("stamp");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut app = app(&data, counting(calls.clone()));
        let plan = decide_with(&root, None, Some(&data), &env()).unwrap();
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(plan.stamp_path.is_file(), "a complete run leaves its stamp");
        // the same inputs: skipped, the converter is not called
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(seen(&app)[1], Outcome::Skipped);
        assert!(!seen(&app)[1].changed());
        // a forced run ignores the stamp
        request(&mut app, &root, true);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // a DLL of another size: the stamp does not match, the run is made
        put(&root.join("missions/x1S.dsl"), b"junk, longer now");
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        // matching again
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        // a map file of another size is another input
        put(&root.join("bzn/fed1.bzn"), b"xx x1R.drl yy and more");
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        // the output folder gone (the player deleted it): the stamp does not skip
        std::fs::remove_dir_all(&plan.out_res).unwrap();
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 5);
        // an incomplete run (cancelled) leaves no stamp that could skip the repair
        let cancelled = fake(|_i, _o, out, _c, _cancel, _sink| {
            let _ = out.finish();
            Summary { cancelled: true, ..Summary::default() }
        });
        app.insert_resource(ConvertContext { data_dir: Some(data.clone()), env: Env::default(), converter: cancelled });
        request(&mut app, &root, true);
        finish_run(&mut app);
        assert!(!plan.stamp_path.exists());
        assert_eq!(seen(&app).last(), Some(&Outcome::Cancelled { converted: 0 }));
    }

    #[test]
    fn a_taken_lock_means_busy_and_nothing_is_touched() {
        let t = Tmp::new("lock");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut app = app(&data, counting(calls.clone()));
        let plan = decide_with(&root, None, Some(&data), &env()).unwrap();
        // another game process holds the lock of this install
        std::fs::create_dir_all(plan.lock_path.parent().unwrap()).unwrap();
        let other = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&plan.lock_path).unwrap();
        other.try_lock().unwrap();
        request(&mut app, &root, true);
        finish_run(&mut app);
        assert_eq!(seen(&app), vec![Outcome::Busy]);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!plan.out_res.exists(), "the output folder was not touched");
        // released: the run goes
        drop(other);
        request(&mut app, &root, true);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(matches!(seen(&app)[1], Outcome::Completed { .. }));
    }

    // ---- prepare_first with the real converter

    #[test]
    fn a_cancel_at_once_leaves_the_earlier_conversion_in_place() {
        let t = Tmp::new("prepare");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let plan = decide_with(&root, None, Some(&data), &env()).unwrap();
        // an earlier conversion: the cache answers for the junk DLLs, so the first run writes fed1.lua
        let key = install::a1_cache_key("fed1", &root.join("missions/x1S.dsl"), &root.join("missions/x1R.drl")).unwrap();
        Cache::new(&plan.cache_dir)
            .put(&key, &Entry::Converted { source: b"-- earlier\n".to_vec(), console: vec!["  fed1: ok".into()] })
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let stop_at_once: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        let stop = stop_at_once.clone();
        // the real converter, but the cancel flag is already up on the second run
        let conv = fake(move |i, o, out, c, cancel, sink| {
            counter.fetch_add(1, Ordering::SeqCst);
            if stop.load(Ordering::SeqCst) {
                real_converter(i, o, out, c, &|| true, sink)
            } else {
                real_converter(i, o, out, c, cancel, sink)
            }
        });
        let mut app = app(&data, conv);
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(seen(&app), vec![Outcome::Completed { total: 1, converted: 1, cached: 1, failed: 0 }]);
        let lua = plan.out_res.join("missions/fed1.lua");
        assert_eq!(std::fs::read(&lua).unwrap(), b"-- earlier\n");
        // the second run is forced (no stamp skip) and cancelled before its first mission
        stop_at_once.store(true, Ordering::SeqCst);
        request(&mut app, &root, true);
        finish_run(&mut app);
        assert_eq!(seen(&app)[1], Outcome::Cancelled { converted: 0 });
        assert!(!state(&app).summary.unwrap().cleared, "prepare_first: nothing was cleared");
        assert_eq!(std::fs::read(&lua).unwrap(), b"-- earlier\n", "the earlier conversion stays");
        assert!(!plan.stamp_path.exists(), "the stopped run leaves no stamp");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_real_converter_fails_junk_missions_as_values() {
        let t = Tmp::new("junk");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(MissionConvertPlugin).init_resource::<Seen>().add_systems(Update, collect.after(drain));
        app.insert_resource(ConvertContext { data_dir: Some(data), env: Env::default(), converter: Arc::new(real_converter) });
        request(&mut app, &root, false);
        finish_run(&mut app);
        let s = state(&app);
        assert_eq!(s.failures.len(), 1, "{:?}", s.failures);
        assert!(matches!(seen(&app)[0], Outcome::Completed { total: 1, converted: 0, failed: 1, .. }), "{:?}", seen(&app));
    }
}
