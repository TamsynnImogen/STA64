//! The core of the in-game mission conversion (LUA-15j, umbrella #637; no UI).
//!
//! The game converts the original games' mission DLLs to Lua on the player's machine, from the
//! player's own install into the user data folder, with the converter library
//! (`sta_mission_convert::install`, LUA-15i). This module is the part that needs no screen:
//!
//! * [`decide`] says whether a run is allowed and where it reads and writes ([`Plan`]), or why
//!   not ([`Refusal`]): for a plain stock install its own missions (some mission DLL in it); for the
//!   mod family's own base game nothing (its missions are scripts already, and the stock campaign
//!   at the install's root is not converted under it); with a mod active that mod's compiled
//!   missions, from the install and that mod only (ADR 0004 D6: the output and the cache follow the
//!   install and the mod they were made for); an output and cache folder the converter accepts
//!   (never inside an install, a game folder or the checkout).
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
//! * A *stamp* file beside the output skips a run whose inputs (the build, the names and content digests of the
//!   mission DLLs, the engine files and the map files) are the ones of the last complete run and
//!   whose output is whole (every script the run wrote is there with the content the manifest
//!   lists). The check runs on the worker thread, never in a frame. The stamp only skips: the cache stays the authority for
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
use sta_mission_convert::output::{Edited, OutDir};

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
    /// The mounted game is the mod family's own base game and no mod is active: its missions are
    /// scripts already, and the stock campaign found at the install's root is not converted under it
    /// (ADR 0004 D6: a conversion layer follows the install and the mod it was made for).
    ModFamilyBase,
    /// No user data folder is known.
    NoDataDir,
    /// The folder is not an install the converter can read missions from.
    Install(InstallError),
    /// The active mod cannot be opened with this install (the mount's own reason).
    Mod(String),
    /// A mod's missions need the engine files and none are in reach; the text says what is missing
    /// and where to name a folder that has them.
    NoEngine(String),
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
        matches!(self, Refusal::Output(_) | Refusal::Cache(_) | Refusal::NoDataDir | Refusal::Install(_) | Refusal::Mod(_) | Refusal::NoEngine(_))
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::ModFamilyBase => f.write_str("this is the mod family's own base game: its missions are scripts already, and the stock campaign at the install's root is not converted under it"),
            Refusal::NoDataDir => f.write_str("no user data folder is known to keep converted missions in"),
            Refusal::Install(e) => write!(f, "this folder has no missions to convert: {}", install_text(e)),
            Refusal::Mod(m) => write!(f, "the active mod cannot be opened with this install: {m}"),
            Refusal::NoMissionDlls => f.write_str("the install has no mission DLLs to convert"),
            Refusal::NoEngine(m) | Refusal::Output(m) | Refusal::Cache(m) => f.write_str(m),
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
    decide_for(install_root, active_mod, data_dir, env, None)
}

/// [`decide_with`] and the stock Armada II folder the player named (`stock_folder` of the settings file),
/// where a mod's missions find their engine files when the install's own root has none.
pub fn decide_for(install_root: &Path, active_mod: Option<&str>, data_dir: Option<&Path>, env: &Env, stock_folder: Option<&Path>) -> Result<Plan, Refusal> {
    let data = data_dir.ok_or(Refusal::NoDataDir)?;
    match active_mod.filter(|m| !m.is_empty()) {
        Some(name) => decide_mod(install_root, name, data, env, stock_folder),
        None => decide_base(install_root, data, env),
    }
}

/// The base game of an install: a plain stock install is offered its own missions; the mod family's
/// own base game is not (its missions are scripts, and the stock campaign at the root of a hand-made
/// install is not converted under it).
fn decide_base(install_root: &Path, data: &Path, env: &Env) -> Result<Plan, Refusal> {
    let install = Install::open(install_root).map_err(Refusal::Install)?;
    if sta_mission_convert::convert::find_missions(&install.missions_dir).is_empty() {
        return Err(Refusal::NoMissionDlls);
    }
    if mod_family_base(&install.root) {
        return Err(Refusal::ModFamilyBase);
    }
    plan_for(install, None, data, env)
}

/// Whether the install, mounted with no mod, is the mod family's own base game (Roots, Fleet Operations
/// 3.x): the profile the mount detects. A mount that fails or detects another profile is not.
pub fn mod_family_base(install_root: &Path) -> bool {
    sta_vfs::Vfs::mount_install(install_root, None).is_ok_and(|v| is_family_base(sta_content::Profile::detect(&v)))
}

/// The profiles of the mod family's own base games (ADR 0004 D5).
pub fn is_family_base(profile: sta_content::Profile) -> bool {
    matches!(profile, sta_content::Profile::Fo3 | sta_content::Profile::Fo4)
}

/// The active mod's compiled missions, from this install and that mod only (D6). The engine files
/// the Armada II conversion names its calls from are looked for at the install's own root first (a
/// hand-made install has the stock pair there); only when it is missing there is the player's stock
/// folder looked at; when neither has them the refusal says what is missing and where to name a folder.
fn decide_mod(install_root: &Path, name: &str, data: &Path, env: &Env, stock_folder: Option<&Path>) -> Result<Plan, Refusal> {
    let install = Install::open_mod(install_root, name).map_err(|e| {
        Refusal::Mod(match e {
            install::ModError::NotAFolder { path } => format!("{} is not a folder", path.display()),
            install::ModError::Mount { reason, .. } => reason,
        })
    })?;
    let install = if has_engine_files(&install.root) {
        install
    } else {
        let missing = |why: String| Refusal::NoEngine(format!("the mod {name}'s missions need the engine files ({ENGINE_FILES}) and {why}"));
        match stock_folder {
            None => {
                return Err(missing(format!(
                    "they are not in {}: set `stock_folder` in the settings file ({}) to a stock Armada II folder that has them",
                    install.root.display(),
                    crate::ui::settings::FILE_NAME
                )));
            }
            Some(f) if !has_engine_files(f) => {
                return Err(missing(format!("the `stock_folder` of the settings file ({}) does not have them", f.display())));
            }
            Some(f) => install.with_engine(f).map_err(|e| missing(format!("`stock_folder` cannot be used: {}", install_text(&e))))?,
        }
    };
    plan_for(install, Some(name), data, env)
}

/// The engine files an Armada II conversion reads, as the player is told about them.
const ENGINE_FILES: &str = "Armada2.exe and armada2.map";

/// Whether `dir` holds the engine pair (the names in any case).
fn has_engine_files(dir: &Path) -> bool {
    sorted_files(dir, &|n| n == "armada2.exe" || n == "armada2.map").len() == 2
}

/// The folders of a run for `install` (and `active_mod`), checked.
fn plan_for(install: Install, active_mod: Option<&str>, data: &Path, env: &Env) -> Result<Plan, Refusal> {
    let out_dir = sta_mission::converted::dir_in(data, &install.root, active_mod);
    let out_res = install::resolve_output(&out_dir, &install, env).map_err(Refusal::Output)?;
    let cache_dir = sta_mission::converted::cache_dir_for(data, &install.root, active_mod);
    install::open_cache(&cache_dir, &out_res, &install, env).map_err(Refusal::Cache)?;
    let key = sta_core::install_key::key(&install.root, active_mod);
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

/// The regular files of `dir` that `keep` (called with the lower-cased name) accepts, sorted by name.
fn sorted_files(dir: &Path, keep: &dyn Fn(&str) -> bool) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            (e.metadata().ok()?.is_file() && keep(&name)).then(|| (name, e.path()))
        })
        .collect();
    v.sort();
    v
}

/// The content digest of an input (the converter's own hash, streamed block by block, so memory
/// stays bounded); a file that cannot be read has a digest of its own that no content has.
fn digest(p: &Path) -> String {
    sta_mission_convert::cache::file_digest(p).unwrap_or_else(|_| "unreadable".to_owned())
}

/// What a run's result depends on: the build, then the names (relative to the missions folder) and
/// the content digests of the mission DLLs, the engine files and the map files. Every input is read
/// once; this runs on the worker thread, never in a frame.
fn stamp_text(plan: &Plan) -> String {
    let mut s = format!("sta64-convert-stamp 2\nbuild {BUILD_ID}\nkey {}\n", plan.key);
    let missions = &plan.install.missions_dir;
    for (dsl, drl) in sta_mission_convert::convert::find_missions(missions) {
        for p in std::iter::once(dsl).chain(drl) {
            let rel = p.strip_prefix(missions).unwrap_or(&p).to_string_lossy().replace('\\', "/").to_ascii_lowercase();
            s.push_str(&format!("dll {rel} {}\n", digest(&p)));
        }
    }
    for (n, p) in sorted_files(&plan.install.root, &|n| n == "armada2.exe" || n == "armada2.map") {
        s.push_str(&format!("engine {n} {}\n", digest(&p)));
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
        for (n, p) in sorted_files(&d, &|n| n.ends_with(".bzn")) {
            s.push_str(&format!("map {n} {}\n", digest(&p)));
        }
    }
    s
}

/// Whether the last complete run's inputs (`inputs`, [`stamp_text`] as of now) are the stamp's and
/// its output is whole: each script the run wrote or kept (the stamp lists their names) is there,
/// either as the tool wrote it or as an edit of it (a run keeps edits). A script the player deleted
/// is missing, whichever others are present. A run that wrote none (every mission failed) has
/// nothing to lose.
fn stamp_matches(plan: &Plan, inputs: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(&plan.stamp_path) else { return false };
    let Some(rest) = text.strip_prefix(inputs) else { return false };
    let names: Option<Vec<&str>> = rest.lines().map(|l| l.strip_prefix("script ")).collect();
    let Some(names) = names else { return false };
    names.is_empty()
        || OutDir::inspect_with(&plan.out_dir, plan.out_res.clone(), false, Edited::Keep).is_ok_and(|o| names.iter().all(|n| o.holds(n)))
}

/// Writes the stamp through a temporary file in its folder and a rename, so a half-written stamp
/// is never read and a link at the stamp's name is replaced, not written through.
fn write_stamp(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let r = f.write_all(text.as_bytes()).and_then(|()| f.flush()).and_then(|()| {
        drop(f);
        std::fs::rename(&tmp, path)
    });
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
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

fn work(plan: Plan, mode: Mode, cancel: Arc<AtomicBool>, tx: Sender<Msg>, converter: Arc<ConverterFn>) {
    let done = run(&plan, mode, &cancel, &tx, &*converter);
    let _ = tx.send(Msg::Done(done));
}

/// What a request asks of the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mode {
    /// Ignore the cache and the stamp.
    forced: bool,
    /// Move the scripts the player edited aside and write the tool's in their place.
    replace_edited: bool,
}

fn run(plan: &Plan, mode: Mode, cancel: &AtomicBool, tx: &Sender<Msg>, converter: &ConverterFn) -> Done {
    let Mode { forced, replace_edited } = mode;
    let _lock = match take_lock(&plan.lock_path) {
        Lock::Busy => return Done::Busy,
        other => other,
    };
    // The inputs as they are before the run: a file changed during it is another stamp next time. The
    // stamp covers the base game's missions, which lie in one folder; a mod's missions come through the
    // install's mount, so a mod's run has no stamp and leans on the cache, which skips every unchanged
    // mission (its key covers the DLLs, the engine files and the converter build).
    let inputs = plan.install.active_mod().is_none().then(|| stamp_text(plan));
    if !forced && inputs.as_deref().is_some_and(|i| stamp_matches(plan, i)) {
        return Done::Skipped;
    }
    // From here the folder changes: no stamp stays that could skip after a stop.
    let _ = std::fs::remove_file(&plan.stamp_path);
    // A forced run goes without the cache; a cache folder that cannot be opened is converted around.
    let cache = if forced { None } else { install::open_cache(&plan.cache_dir, &plan.out_res, &plan.install, &plan.env).ok() };
    // A script the player edited is kept and reported by every run, forced or not; only an explicit
    // replacement moves it aside. A script the tool never wrote is never touched: the folder is
    // refused (no keep_foreign).
    let policy = if replace_edited { Edited::Replace } else { Edited::Keep };
    let mut out = match OutDir::inspect_with(&plan.out_dir, plan.out_res.clone(), false, policy) {
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
    if summary.completed()
        && let Some(inputs) = &inputs
    {
        // the scripts the folder must hold: those written and those kept for the player's edit
        let expected: std::collections::BTreeSet<&String> = summary.missions.iter().flat_map(|m| m.written.iter().chain(&m.kept)).collect();
        let lines: String = expected.iter().map(|n| format!("script {n}\n")).collect();
        let _ = write_stamp(&plan.stamp_path, &format!("{inputs}{lines}"));
    }
    Done::Summary(Box::new(summary))
}

// ------------------------------------------------------------------ what the game sees

/// Start a run for the install: for its base game (`active_mod` none), or for the missions of the
/// active mod (D6: the output and the cache follow the install and the mod). `forced`: ignore the
/// cache and the stamp (convert again); the scripts the player edited are kept all the same.
/// `replace_edited`: move them aside (as `<name>.old`) and write the tool's own in their place; only
/// an explicit request says so. Answered with [`Outcome::AlreadyRunning`] while a run is going.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct StartConversion {
    pub install_root: PathBuf,
    pub active_mod: Option<String>,
    pub forced: bool,
    pub replace_edited: bool,
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
    /// A [`StartConversion`] arrived while this game's own run was going: it is answered with this
    /// and dropped; the run goes on and ends with its own message. [`ConversionState`] is not
    /// touched by it.
    AlreadyRunning,
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
    /// Script stems the run left alone because the player edited them since.
    pub kept_edited: Vec<String>,
    /// The run was asked to replace the scripts the player edited (they are moved aside).
    pub replacing: bool,
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
    settings: Option<Res<crate::frontend::screens::Settings>>,
    mut state: ResMut<ConversionState>,
    mut active: ResMut<ActiveRun>,
    mut fin: MessageWriter<ConversionFinished>,
) {
    for req in requests.read() {
        if active.is_running() {
            fin.write(ConversionFinished { outcome: Outcome::AlreadyRunning });
            continue;
        }
        let stock = settings.as_ref().map(|s| s.stock_folder.trim()).filter(|f| !f.is_empty()).map(PathBuf::from);
        let plan = match decide_for(&req.install_root, req.active_mod.as_deref(), ctx.data_dir.as_deref(), &ctx.env, stock.as_deref()) {
            Ok(p) => p,
            Err(r) => {
                *state = ConversionState { runs: state.runs, ..ConversionState::default() };
                finish(&mut state, &mut fin, Outcome::Refused(r));
                continue;
            }
        };
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (flag, converter) = (cancel.clone(), ctx.converter.clone());
        let mode = Mode { forced: req.forced, replace_edited: req.replace_edited };
        let spawned = std::thread::Builder::new().name("mission-convert".into()).spawn(move || work(plan, mode, flag, tx, converter));
        *state = ConversionState { runs: state.runs + 1, running: true, replacing: req.replace_edited, ..ConversionState::default() };
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
        ConvEvent::KeptEdited { name, .. } => state.kept_edited.push(name),
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
        // a mod the install does not have: refused with the mount's reason
        assert!(matches!(decide_with(&root, Some("Fleet Ops"), Some(&data), &env()), Err(Refusal::Mod(_))));
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
        assert!(r.is_fault() && !Refusal::NoMissionDlls.is_fault() && !Refusal::ModFamilyBase.is_fault() && Refusal::Mod("x".into()).is_fault() && Refusal::NoEngine("x".into()).is_fault());
        // the checkout is refused
        let repo = t.0.join("checkout");
        std::fs::create_dir_all(&repo).unwrap();
        let env = Env { repo_root: Some(repo.clone()), ..Env::default() };
        let r = decide_with(&root, None, Some(&repo.join("data")), &env).unwrap_err();
        assert!(matches!(&r, Refusal::Output(m) if m.contains("repository")), "{r:?}");
        // every refusal has a text
        for r in [Refusal::NoDataDir, Refusal::NoMissionDlls, Refusal::ModFamilyBase, Refusal::Mod("m".into()), Refusal::NoEngine("m".into())] {
            assert!(!r.to_string().is_empty());
        }
    }

    // ---- #1407: the offer follows the install and the mod

    /// A hand-made install of the mod family: a complete stock install at the root (executable, mission DLLs) whose
    /// mount detects the mod family's profile (an explicit profile in the data folder's `info.ini`).
    fn hand_made(root: &Path) {
        install(root, b"junk");
        put(&root.join("Data/info.ini"), b"[Mod]\nProfile=fo4\n");
    }

    #[test]
    fn the_mod_familys_base_game_is_offered_nothing_and_a_plain_stock_install_is_unchanged() {
        let t = Tmp::new("family");
        let data = t.0.join("data");
        let (stock, made) = (t.0.join("Stock"), t.0.join("Made"));
        install(&stock, b"junk");
        hand_made(&made);
        // (a) the engine pair and the stock missions at the root do not by themselves make an offer
        put(&made.join("Armada2.exe"), b"");
        put(&made.join("armada2.map"), b"");
        assert_eq!(decide_with(&made, None, Some(&data), &env()).unwrap_err(), Refusal::ModFamilyBase);
        assert!(!Refusal::ModFamilyBase.is_fault(), "the normal nothing to do, not a fault");
        // (c) a plain stock install: as before
        assert!(decide_with(&stock, None, Some(&data), &env()).is_ok());
        // Fleet Operations 3.x is a base game of the family too; Armada II's own profile is not
        put(&made.join("Data/info.ini"), b"[Mod]\nProfile=fo3\n");
        assert_eq!(decide_with(&made, None, Some(&data), &env()).unwrap_err(), Refusal::ModFamilyBase);
        put(&made.join("Data/info.ini"), b"[Mod]\nProfile=sta2classic\n");
        assert!(decide_with(&made, None, Some(&data), &env()).is_ok(), "an Armada II profile is a plain stock install");
        // nothing was written
        assert!(!data.exists());
    }

    /// An install hosting the mod `M`, with the engine pair at its root when `engine`.
    fn with_mod(root: &Path, engine: bool) {
        put(&root.join("Data/Mods/M/info.ini"), b"[Mod]\nName=M\n");
        if engine {
            put(&root.join("Armada2.exe"), b"");
            put(&root.join("ARMADA2.MAP"), b"");
        }
    }

    #[test]
    fn a_mods_missions_are_offered_keyed_by_the_install_and_the_mod_with_the_engine_pair_at_the_root() {
        let t = Tmp::new("modplan");
        let (root, data) = (t.0.join("Made"), t.0.join("data"));
        hand_made(&root);
        with_mod(&root, true);
        let base = decide_with(&root, None, Some(&data), &env()).unwrap_err();
        assert_eq!(base, Refusal::ModFamilyBase, "the base game of the same install is not offered");
        // (b) the mod is: the engine files are looked for at the install's root first
        let plan = decide_with(&root, Some("M"), Some(&data), &env()).unwrap();
        assert_eq!(plan.install.active_mod(), Some("M"));
        assert_eq!(plan.install.engine_root(), install::realpath(&root));
        assert_eq!(plan.out_dir, sta_mission::converted::dir_in(&data, &root, Some("M")), "the folder the game mounts for this mod");
        assert_eq!(plan.cache_dir, sta_mission::converted::cache_dir_for(&data, &root, Some("M")));
        assert!(plan.key.ends_with("+m"), "{}", plan.key);
        assert_ne!(plan.out_dir, sta_mission::converted::dir_in(&data, &root, None), "never the base game's folder");
        assert_eq!(plan.stamp_path, data.join("converted").join(format!("{}.stamp", plan.key)));
        // the mod's own key for another mod is another folder, and the same mod in another case is the mount's to accept
        assert_ne!(sta_mission::converted::dir_in(&data, &root, Some("Other")), plan.out_dir);
        // the mount's reason when the install does not have the mod
        let r = decide_with(&root, Some("Gone"), Some(&data), &env()).unwrap_err();
        assert!(matches!(&r, Refusal::Mod(m) if !m.is_empty()) && r.is_fault(), "{r:?}");
        // a mod name that is a path is the mount's to refuse
        assert!(matches!(decide_with(&root, Some("../x"), Some(&data), &env()), Err(Refusal::Mod(_))));
        // no data folder
        assert_eq!(decide_with(&root, Some("M"), None, &env()).unwrap_err(), Refusal::NoDataDir);
    }

    #[test]
    fn without_the_engine_pair_at_the_root_a_mod_needs_the_stock_folder_setting_and_the_refusal_says_what_is_missing() {
        let t = Tmp::new("modengine");
        let (root, data, stock, empty) = (t.0.join("Made"), t.0.join("data"), t.0.join("Stock"), t.0.join("Empty"));
        with_mod(&root, false);
        put(&stock.join("Armada2.exe"), b"");
        put(&stock.join("armada2.map"), b"");
        std::fs::create_dir_all(&empty).unwrap();
        // neither: the text names the files, the install's folder and the setting
        let r = decide_for(&root, Some("M"), Some(&data), &env(), None).unwrap_err();
        let Refusal::NoEngine(m) = &r else { panic!("{r:?}") };
        assert!(m.contains("Armada2.exe") && m.contains("armada2.map") && m.contains("stock_folder") && m.contains("settings.cfg"), "{m}");
        assert!(m.contains(&install::realpath(&root).display().to_string()), "{m}");
        assert!(r.is_fault(), "the player is told");
        // a stock folder that lacks them: the text says that folder does not have them
        let r = decide_for(&root, Some("M"), Some(&data), &env(), Some(&empty)).unwrap_err();
        assert!(matches!(&r, Refusal::NoEngine(m) if m.contains("does not have them") && m.contains("Empty")), "{r:?}");
        // a stock folder that has them: the plan reads the engine files there and writes under the mod's key
        let plan = decide_for(&root, Some("M"), Some(&data), &env(), Some(&stock)).unwrap();
        assert_eq!(plan.install.engine_root(), install::realpath(&stock));
        assert_eq!(plan.out_dir, sta_mission::converted::dir_in(&data, &root, Some("M")));
        // the install's own root wins over the setting
        with_mod(&root, true);
        let plan = decide_for(&root, Some("M"), Some(&data), &env(), Some(&stock)).unwrap();
        assert_eq!(plan.install.engine_root(), install::realpath(&root));
        // no output or cache below the stock folder the run only reads
        let inside = stock.join("userdata");
        let r = decide_for(&root, Some("M"), Some(&inside), &env(), Some(&stock)).unwrap_err();
        assert!(matches!(r, Refusal::Output(_)), "{r:?}");
    }

    /// The setting reaches the run: a mod's run through the app reads the engine files in the folder the
    /// `stock_folder` setting names, and a game with no setting says what is missing and runs nothing.
    #[test]
    fn the_stock_folder_setting_reaches_a_mods_run() {
        let t = Tmp::new("modrun");
        let (root, data, stock) = (t.0.join("Made"), t.0.join("data"), t.0.join("Stock"));
        with_mod(&root, false);
        put(&stock.join("Armada2.exe"), b"");
        put(&stock.join("armada2.map"), b"");
        let engine = Arc::new(Mutex::new(None::<PathBuf>));
        let seen_engine = engine.clone();
        let converter = fake(move |i, _o, out, _c, _cancel, _sink| {
            *seen_engine.lock().unwrap() = Some(i.engine_root().to_path_buf());
            let _ = out.finish();
            Summary::default()
        });
        let mut app = app(&data, converter.clone());
        app.world_mut().write_message(StartConversion { install_root: root.clone(), active_mod: Some("M".into()), forced: false, replace_edited: false });
        app.update();
        assert!(matches!(&seen(&app)[..], [Outcome::Refused(Refusal::NoEngine(_))]), "{:?}", seen(&app));
        assert!(engine.lock().unwrap().is_none(), "nothing ran");
        let mut settings = crate::frontend::screens::Settings::default();
        settings.stock_folder = stock.display().to_string();
        app.insert_resource(settings);
        app.world_mut().write_message(StartConversion { install_root: root.clone(), active_mod: Some("M".into()), forced: false, replace_edited: false });
        finish_run(&mut app);
        assert_eq!(engine.lock().unwrap().clone(), Some(install::realpath(&stock)));
        assert!(!data.join("converted").join(format!("{}.stamp", sta_core::install_key::key(&root, Some("M")))).exists(), "a mod's run keeps no stamp");
    }

    /// With `STA64_TEST_ROOTS_INSTALL` (an install of the mod family) and optionally `STA64_TEST_ROOTS_MOD`
    /// (a mod of it with compiled missions): the base game is offered nothing, and the mod's run goes through the
    /// game's own start path (the hook's message, the real converter) into a temporary data folder and converts
    /// every mission of the mod. Writes only below the temporary folder, never into the install or the player's
    /// data folder (`data_dir` is given to the context). Structure only; skips without the variables.
    #[test]
    fn installed_the_family_base_is_offered_nothing_and_the_mods_missions_convert_under_its_own_key() {
        let Ok(root) = std::env::var("STA64_TEST_ROOTS_INSTALL") else {
            eprintln!("SKIP: set STA64_TEST_ROOTS_INSTALL to an install of the mod family");
            return;
        };
        let root = PathBuf::from(root);
        let t = Tmp::new("installed");
        let data = t.0.join("data");
        let base = decide_with(&root, None, Some(&data), &env());
        assert!(matches!(base, Err(Refusal::ModFamilyBase)), "the base game of a mod family install is not offered a conversion: {base:?}");
        let Ok(name) = std::env::var("STA64_TEST_ROOTS_MOD") else {
            eprintln!("SKIP the mod run: set STA64_TEST_ROOTS_MOD to a mod of the install with compiled missions");
            assert!(!data.exists(), "deciding writes nothing");
            return;
        };
        let mut app = app(&data, Arc::new(real_converter));
        app.world_mut().write_message(StartConversion { install_root: root.clone(), active_mod: Some(name.clone()), forced: false, replace_edited: false });
        finish_run(&mut app);
        let outcome = seen(&app).pop().expect("the run ends with a message");
        match outcome {
            Outcome::Completed { total, converted, failed, .. } => {
                assert!(total > 0, "the mod has compiled missions");
                assert_eq!((converted, failed), (total, 0), "every mission of the mod converts");
            }
            Outcome::Refused(Refusal::NoEngine(m)) => panic!("the install has no engine pair and no stock folder is set: {m}"),
            other => panic!("{other:?}"),
        }
        let key = sta_core::install_key::key(&root, Some(&name));
        assert!(data.join("converted").join(&key).join("missions").is_dir(), "written under the mod's key");
        assert!(!data.join("converted").join(sta_core::install_key::key(&root, None)).exists(), "nothing for the base game");
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
        app.world_mut().write_message(StartConversion { install_root: root.to_path_buf(), active_mod: None, forced, replace_edited: false });
    }

    fn request_replacing(app: &mut App, root: &Path) {
        app.world_mut().write_message(StartConversion { install_root: root.to_path_buf(), active_mod: None, forced: true, replace_edited: true });
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
        app.world_mut().write_message(StartConversion { install_root: root.clone(), active_mod: Some("Mod".into()), forced: false, replace_edited: false });
        app.update();
        assert!(matches!(&seen(&app)[..], [Outcome::Refused(Refusal::Mod(_))]), "{:?}", seen(&app));
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
        fake(move |_i, _o, out, _c, _cancel, sink| {
            calls.fetch_add(1, Ordering::SeqCst);
            let mut one = install::MissionResult { mission: "fed1".into(), converted: true, ..install::MissionResult::default() };
            match out.write("fed1.lua", b"-- authored\n") {
                Ok(()) => one.written.push("fed1.lua".into()),
                Err(sta_mission_convert::output::WriteError::Kept) => {
                    sink(&ConvEvent::KeptEdited { mission: "fed1".into(), name: "fed1".into() });
                    one.kept.push("fed1.lua".into());
                }
                Err(e) => panic!("{e}"),
            }
            let _ = out.finish();
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
        // a DLL patched to the same size is another input: the content is compared, not the size
        let dll = root.join("missions/x1S.dsl");
        let before = std::fs::read(&dll).unwrap();
        let mut patched = before.clone();
        patched[0] ^= 1;
        assert_eq!(patched.len(), before.len());
        put(&dll, &patched);
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 6);
        // the same bytes written again are the same input
        put(&dll, &patched);
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 6);
        assert_eq!(seen(&app).last(), Some(&Outcome::Skipped));
        // so is a map patched to the same size
        let map = root.join("bzn/fed1.bzn");
        let mut m = std::fs::read(&map).unwrap();
        m[0] ^= 1;
        put(&map, &m);
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 7);
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
    fn a_script_that_is_gone_or_changed_is_not_skipped_over() {
        let t = Tmp::new("damage");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut app = app(&data, counting(calls.clone()));
        let plan = decide_with(&root, None, Some(&data), &env()).unwrap();
        let lua = plan.out_res.join("missions/fed1.lua");
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // a script the player deleted: the stamp does not skip, the run writes it again
        std::fs::remove_file(&lua).unwrap();
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(std::fs::read(&lua).unwrap(), b"-- authored\n");
        // a script the player edited stays: the stamp counts it as present, so nothing runs
        std::fs::write(&lua, b"-- edited\n").unwrap();
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(seen(&app).last(), Some(&Outcome::Skipped));
        assert_eq!(std::fs::read(&lua).unwrap(), b"-- edited\n");
        // a run that is made for another reason keeps it and says so
        put(&root.join("missions/x1S.dsl"), b"junk, changed");
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(state(&app).kept_edited, vec!["fed1".to_owned()]);
        assert_eq!(std::fs::read(&lua).unwrap(), b"-- edited\n", "the edit is kept");
        // "convert again" (forced: no cache, no stamp) keeps the edit too, after a cancelled run as well
        let cancelled_run = fake(|_i, _o, out, _c, _cancel, _sink| {
            let _ = out.finish();
            Summary { cancelled: true, ..Summary::default() }
        });
        app.insert_resource(ConvertContext { data_dir: Some(data.clone()), env: Env::default(), converter: cancelled_run });
        request(&mut app, &root, true);
        finish_run(&mut app);
        assert_eq!(seen(&app).last(), Some(&Outcome::Cancelled { converted: 0 }));
        app.insert_resource(ConvertContext { data_dir: Some(data.clone()), env: Env::default(), converter: counting(calls.clone()) });
        request(&mut app, &root, true);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 4, "a forced run is made");
        assert_eq!(state(&app).kept_edited, vec!["fed1".to_owned()]);
        assert_eq!(std::fs::read(&lua).unwrap(), b"-- edited\n", "forcing is not replacing");
        assert!(!state(&app).replacing);
        // only the explicit request moves it aside and writes the tool's own
        request_replacing(&mut app, &root);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 5);
        assert!(state(&app).replacing && state(&app).kept_edited.is_empty());
        assert_eq!(std::fs::read(&lua).unwrap(), b"-- authored\n");
        assert_eq!(std::fs::read(plan.out_res.join("missions/fed1.lua.old")).unwrap(), b"-- edited\n", "the edit is aside, not lost");
        // a file the tool never wrote is never touched: the folder is refused, whatever the run
        put(&plan.out_res.join("missions/mine.lua"), b"-- mine\n");
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert!(matches!(seen(&app).last(), Some(Outcome::Aborted(m)) if m.contains("did not write")), "{:?}", seen(&app).last());
        request(&mut app, &root, true);
        finish_run(&mut app);
        assert!(matches!(seen(&app).last(), Some(Outcome::Aborted(m)) if m.contains("did not write")));
        request_replacing(&mut app, &root);
        finish_run(&mut app);
        assert!(matches!(seen(&app).last(), Some(Outcome::Aborted(m)) if m.contains("did not write")));
        assert_eq!(std::fs::read(plan.out_res.join("missions/mine.lua")).unwrap(), b"-- mine\n");
        assert_eq!(calls.load(Ordering::SeqCst), 5);
    }

    /// A converter that writes `fed1.lua` and `fed2.lua`, keeping an edited one.
    fn two_scripts(calls: Arc<AtomicUsize>) -> Arc<ConverterFn> {
        fake(move |_i, _o, out, _c, _cancel, sink| {
            calls.fetch_add(1, Ordering::SeqCst);
            let mut r = install::MissionResult { mission: "fed".into(), converted: true, ..install::MissionResult::default() };
            for n in ["fed1", "fed2"] {
                match out.write(&format!("{n}.lua"), format!("-- {n}\n").as_bytes()) {
                    Ok(()) => r.written.push(format!("{n}.lua")),
                    Err(sta_mission_convert::output::WriteError::Kept) => {
                        sink(&ConvEvent::KeptEdited { mission: "fed".into(), name: n.into() });
                        r.kept.push(format!("{n}.lua"));
                    }
                    Err(e) => panic!("{e}"),
                }
            }
            let _ = out.finish();
            Summary { total: 1, missions: vec![r], ..Summary::default() }
        })
    }

    #[test]
    fn a_deleted_script_is_restored_at_the_next_start_and_the_edit_beside_it_is_kept() {
        let t = Tmp::new("restore");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut app = app(&data, two_scripts(calls.clone()));
        let plan = decide_with(&root, None, Some(&data), &env()).unwrap();
        let (one, two) = (plan.out_res.join("missions/fed1.lua"), plan.out_res.join("missions/fed2.lua"));
        request(&mut app, &root, false);
        finish_run(&mut app);
        std::fs::write(&one, b"-- edited\n").unwrap();
        // a run that keeps the edit (another input changed) leaves a stamp that lists both scripts
        put(&root.join("missions/x1S.dsl"), b"junk, changed");
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(state(&app).kept_edited, vec!["fed1".to_owned()]);
        // nothing is missing: the next start skips
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(seen(&app).last(), Some(&Outcome::Skipped));
        // the player deletes the script the tool wrote: the edit in its place does not stand in for it
        std::fs::remove_file(&two).unwrap();
        request(&mut app, &root, false);
        finish_run(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 3, "a run is made");
        assert_eq!(std::fs::read(&two).unwrap(), b"-- fed2\n", "the deleted script is back");
        assert_eq!(std::fs::read(&one).unwrap(), b"-- edited\n", "the edit is kept");
    }

    #[test]
    fn a_start_while_a_run_is_going_is_answered_and_the_run_goes_on() {
        let t = Tmp::new("twice");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        install(&root, b"junk");
        let (gate_tx, gate_rx) = sync_channel::<()>(0);
        let gate = Mutex::new(gate_rx);
        let conv = fake(move |_i, _o, out, _c, _cancel, sink| {
            sink(&ConvEvent::Planned { total: 1 });
            gate.lock().unwrap().recv().unwrap();
            let _ = out.finish();
            Summary { total: 1, ..Summary::default() }
        });
        let mut app = app(&data, conv);
        request(&mut app, &root, false);
        app.update();
        assert!(state(&app).running);
        request(&mut app, &root, true);
        app.update();
        assert_eq!(seen(&app), vec![Outcome::AlreadyRunning], "the second request is answered");
        assert!(!seen(&app)[0].changed());
        assert!(state(&app).running, "the run in flight is untouched");
        gate_tx.send(()).unwrap();
        finish_run(&mut app);
        assert_eq!(seen(&app).len(), 2);
        assert!(matches!(seen(&app)[1], Outcome::Completed { total: 1, .. }));
        assert_eq!(state(&app).runs, 1, "no second run was started");
    }

    #[cfg(unix)]
    #[test]
    fn the_stamp_is_written_through_a_temporary_file_and_replaces_a_link() {
        let t = Tmp::new("stamplink");
        let (victim, stamp) = (t.0.join("victim.txt"), t.0.join("k.stamp"));
        put(&victim, b"precious");
        std::os::unix::fs::symlink(&victim, &stamp).unwrap();
        write_stamp(&stamp, "stamp text\n").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious", "nothing is written through the link");
        assert!(!std::fs::symlink_metadata(&stamp).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(&stamp).unwrap(), b"stamp text\n");
        // no temporary file stays, and a second write replaces the first
        write_stamp(&stamp, "again\n").unwrap();
        assert_eq!(std::fs::read(&stamp).unwrap(), b"again\n");
        let names: Vec<_> = std::fs::read_dir(&t.0).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    /// Measures what the stamp's content check costs on an authored folder (not part of the normal
    /// run: it writes about 115 MB). `cargo test -p sta64 --lib stamp_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "measurement, writes about 115 MB"]
    fn stamp_cost_on_a_folder_of_realistic_size() {
        let t = Tmp::new("cost");
        let (root, data) = (t.0.join("Armada"), t.0.join("data"));
        put(&root.join("Armada2.exe"), &vec![7u8; 5 << 20]);
        put(&root.join("Armada2.map"), &vec![9u8; 2 << 20]);
        put(&root.join("Armada.exe"), b"");
        for i in 0..40 {
            put(&root.join(format!("missions/m{i}S.dsl")), &vec![(i % 251) as u8; 100 << 10]);
            put(&root.join(format!("missions/m{i}R.drl")), &vec![(i % 241) as u8; 100 << 10]);
        }
        for i in 0..100 {
            put(&root.join(format!("bzn/map{i}.bzn")), &vec![(i % 239) as u8; 1 << 20]);
        }
        let plan = decide_with(&root, None, Some(&data), &env()).unwrap();
        let t0 = std::time::Instant::now();
        let text = stamp_text(&plan);
        eprintln!("stamp_text over about 115 MB in 241 files: {:?} ({} lines)", t0.elapsed(), text.lines().count());
        let t1 = std::time::Instant::now();
        let again = stamp_text(&plan);
        eprintln!("second time (file cache warm): {:?}", t1.elapsed());
        assert_eq!(text, again);
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
