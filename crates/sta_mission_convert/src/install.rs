//! Converting an install (LUA-15i): the orchestration the command line ([`crate::cli`]) and the
//! game both use. One run is: look at the install ([`Install::open`], or [`Install::open_mod`]
//! for the missions of a mod inside it, #927), open the output folder
//! ([`OutDir`], checked with [`resolve_output`]), then [`convert_install`], which clears what an
//! earlier run wrote, discovers the missions of the install (Armada I or II), converts each one
//! (through the [`Cache`] when given one), writes the result through the verified writer
//! ([`OutDir`]) and reports what happens to a sink as [`Event`]s.
//!
//! What this module does not do: it prints nothing, reads no environment variable besides what
//! [`Env::from_process`] is asked for, never exits the process, and turns every fault into a value
//! (an [`InstallError`], a `String` for a refused folder, an [`Abort`] or a failed mission in the
//! [`Summary`]). It runs on the caller's thread, one mission after the other; a caller that must
//! not block (the game) runs it on a worker thread and answers `cancel` from a flag. It never
//! writes outside the [`OutDir`] and the [`Cache`] folder it is given, and never into the install.
//!
//! A cancelled run stops between two missions: what was written stays, the manifest lists exactly
//! that (so the folder is consistent), and the next full run clears it as it clears any earlier
//! run.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_vfs::{Source, Vfs};

use crate::a1;
use crate::cache::{Cache, Entry, KeyBuilder};
use crate::convert::{self, load_engine};
use crate::disasm::Ins;
use crate::map::Vtables;
use crate::output::{OutDir, WriteError};
use crate::paths::{contains, game_root_above, resolve};
use crate::pyval::py_lower;
use crate::{DecodeError, ErrorKind, Pe, Result};

// ------------------------------------------------------------------ surroundings

/// What a run knows of its surroundings (so tests can control it).
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// The user data folder (`platform::data_dir`): the default output is below it.
    pub data_dir: Option<PathBuf>,
    /// The checkout the output must not go into.
    pub repo_root: Option<PathBuf>,
    /// Build folders inside the checkout, which stay allowed (for tests).
    pub target_dirs: Vec<PathBuf>,
}

impl Env {
    /// The process' environment: `STA64_DATA_DIR`, `XDG_DATA_HOME`, ... as `data_dir` reads them,
    /// the checkout this crate was built in (when it still exists) and its `target` folder and
    /// `CARGO_TARGET_DIR`.
    pub fn from_process() -> Env {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        let repo_root = resolve(&repo).ok().filter(|r| r.is_dir());
        let mut target_dirs: Vec<PathBuf> = repo_root.iter().map(|r| r.join("target")).collect();
        target_dirs.extend(get("CARGO_TARGET_DIR").map(PathBuf::from));
        Env { data_dir: data_dir(&get, std::env::consts::OS), repo_root, target_dirs }
    }
}

pub use sta_core::install_key::slug;

/// `key(install, mod)`: the folder key of an install, `<slug of the folder name>-<hash of the
/// resolved path>`, plus `+` and `slug(mod)` for a mod. One rule for the game and the converter,
/// [`sta_core::install_key`] (LUA-15j); the Python tool's rule (the slug alone) is superseded.
pub fn key(install: &Path, modname: Option<&str>) -> String {
    sta_core::install_key::key(install, modname)
}

/// `data_dir(env, system)`: `platform::data_dir`, the user data folder of STA64. `get` reads an
/// environment variable (empty counts as unset), `system` is `linux`, `macos` or `windows` (the
/// Python's `sys.platform` names `darwin` and `win32` are accepted too).
pub fn data_dir(get: &dyn Fn(&str) -> Option<String>, system: &str) -> Option<PathBuf> {
    let var = |k: &str| get(k).filter(|v| !v.is_empty());
    if let Some(d) = var("STA64_DATA_DIR") {
        return Some(PathBuf::from(d));
    }
    let win = system.starts_with("win");
    let home = var("HOME").or_else(|| var("USERPROFILE")).map(PathBuf::from);
    let mut base = var("XDG_DATA_HOME").map(PathBuf::from);
    if base.is_none() && win {
        base = var("LOCALAPPDATA").map(PathBuf::from);
    }
    if base.is_none()
        && let Some(h) = &home
    {
        base = Some(if system == "darwin" || system == "macos" {
            h.join("Library").join("Application Support")
        } else if win {
            h.join("AppData").join("Local")
        } else {
            h.join(".local").join("share")
        });
    }
    base.map(|b| b.join("sta64"))
}

/// `os.path.realpath` ([`resolve`]); a path that cannot be resolved is made absolute only.
pub fn realpath(p: &Path) -> PathBuf {
    resolve(p).unwrap_or_else(|_| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()))
}

/// Why `dest` (a resolved folder the tool would write into) is refused, or `None`: derived from
/// game files, so never in an install or any game folder, never containing an install, never in
/// the checkout (its build folders excepted).
pub fn refuse(dest: &Path, installs: &[&Path], env: &Env) -> Option<String> {
    if let Some(repo) = env.repo_root.as_ref().and_then(|r| resolve(r).ok())
        && contains(&repo, dest)
        && !env.target_dirs.iter().filter_map(|b| resolve(b).ok()).any(|b| contains(&b, dest))
    {
        return Some("is inside the repository: output is derived from game files".to_owned());
    }
    for r in installs {
        if contains(r, dest) {
            return Some(format!("is inside the install {}: installs are read-only", r.display()));
        }
        if contains(dest, r) {
            return Some(format!("contains the install {}: refused", r.display()));
        }
    }
    game_root_above(dest).map(|g| format!("is at or inside the game folder {}: refused", g.display()))
}

pub(crate) fn names(dir: &Path) -> std::io::Result<Vec<String>> {
    Ok(std::fs::read_dir(dir)?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
}

/// `child(d, name)`: `d/name` for a folder spelled in any case, or `None` (of several spellings,
/// the first in code-point order).
pub fn child(d: &Path, name: &str) -> Option<PathBuf> {
    let mut v = names(d).ok()?;
    v.sort();
    v.into_iter().find(|n| py_lower(n) == py_lower(name) && d.join(n).is_dir()).map(|n| d.join(n))
}

/// An Armada I root: `Armada.exe` and no `Armada2.exe`.
pub fn is_armada1(install: &Path) -> bool {
    let n: Vec<String> = names(install).unwrap_or_default().iter().map(|x| py_lower(x)).collect();
    n.iter().any(|x| x == "armada.exe") && !n.iter().any(|x| x == "armada2.exe")
}

/// The output folder a run checked: `given` is the folder as the caller wrote it, the second
/// the resolved one. Refuses (with the text the command line prints after `output folder <given>`)
/// what [`refuse`] refuses, and a folder that cannot be resolved.
pub fn resolve_output(given: &Path, install: &Install, env: &Env) -> std::result::Result<PathBuf, String> {
    let res = resolve(given)
        .map_err(|m| format!("output folder {} cannot be resolved: {m}", given.display()))?;
    match refuse(&res, &install.read_only_roots(), env) {
        Some(why) => Err(format!("output folder {} {why}", given.display())),
        None => Ok(res),
    }
}

/// The cache folder `dir` (as given), checked the same way and against the resolved output folder
/// `out_res`: it may not contain it or lie inside it.
pub fn open_cache(dir: &Path, out_res: &Path, install: &Install, env: &Env) -> std::result::Result<Cache, String> {
    let res = resolve(dir).map_err(|m| format!("cache folder {} cannot be resolved: {m}", dir.display()))?;
    if let Some(why) = refuse(&res, &install.read_only_roots(), env) {
        return Err(format!("cache folder {} {why}", dir.display()));
    }
    if contains(out_res, &res) || contains(&res, out_res) {
        return Err("the cache folder and the output folder may not contain one another".to_owned());
    }
    Ok(Cache::new(res))
}

// ------------------------------------------------------------------ the install

/// Which game family an install is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Game {
    /// `Armada.exe` and no `Armada2.exe`: mission DLLs per map.
    Armada1,
    /// `Armada2.exe`: `.dsl` and `.drl` mission DLLs.
    Armada2,
}

/// An install the converter can read missions from: the base game's own ([`Install::open`]), or
/// those of one mod inside it ([`Install::open_mod`]).
#[derive(Debug, Clone)]
pub struct Install {
    /// The resolved root.
    pub root: PathBuf,
    pub game: Game,
    /// The base game's own missions folder (never a mod's, D6). Opened with a mod, the install's
    /// data folder: that run finds its missions through the mount, not in one folder.
    pub missions_dir: PathBuf,
    /// The mod this install was opened with, and the mount that resolves its files.
    mounted: Option<Mounted>,
    /// Where the Armada II engine files are read when not at `root` ([`Install::with_engine`]).
    engine_root: Option<PathBuf>,
}

/// An install's mount with its active mod, kept from [`Install::open_mod`] to the discovery.
#[derive(Debug, Clone)]
struct Mounted {
    name: String,
    vfs: Arc<Vfs>,
    /// The folder of every layer of the mount, resolved: the mod named may be a link to a
    /// folder outside the install, and each of them is only read.
    layer_roots: Vec<PathBuf>,
}

/// Why an install cannot be opened with a mod ([`Install::open_mod`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModError {
    /// `path` does not resolve to a folder.
    NotAFolder { path: PathBuf },
    /// The game's mount refuses `root` with the mod `name`; `reason` is the mount's own text
    /// (no such mod, an install that has no mods folder, a parent cycle, ...).
    Mount { path: PathBuf, root: PathBuf, name: String, reason: String },
}

/// Why a folder is not an install the converter can use.
#[derive(Debug, Clone, PartialEq, Eq)]
///
/// Every variant carries the data a caller needs to word it in its own text (the command line
/// formats them itself, see `cli.rs`): `path` is the folder as the caller gave it, `root` the
/// resolved one.
pub enum InstallError {
    /// `path` does not resolve to a folder.
    NotAFolder { path: PathBuf },
    /// The folder `root` cannot be listed; `reason` is the system's text.
    Unreadable { path: PathBuf, root: PathBuf, reason: String },
    /// An engine in `Data/` and Lua missions of its own: nothing to convert.
    FleetOperations { path: PathBuf, root: PathBuf },
    /// Neither `Armada.exe` nor `Armada2.exe` in `root`.
    NotArmada { path: PathBuf, root: PathBuf },
    /// `root` has no missions folder (at its top or in `Data/`).
    NoMissionsFolder { path: PathBuf, root: PathBuf },
}

impl Install {
    /// Looks at `path`: which game it is and where its missions are. Reads folder listings only.
    pub fn open(path: &Path) -> std::result::Result<Install, InstallError> {
        let given = path.to_path_buf();
        let install = match resolve(path) {
            Ok(p) if p.is_dir() => p,
            _ => return Err(InstallError::NotAFolder { path: given }),
        };
        let (p, r) = (given.clone(), install.clone());
        let top = names(&install)
            .map_err(|e| InstallError::Unreadable { path: p, root: r, reason: e.to_string() })?;
        let low: Vec<String> = top.iter().map(|n| py_lower(n)).collect();
        if !low.iter().any(|n| n == "armada.exe") && !low.iter().any(|n| n == "armada2.exe") {
            if let Some(d) = child(&install, "Data")
                && names(&d)
                    .unwrap_or_default()
                    .iter()
                    .any(|n| matches!(py_lower(n).as_str(), "armada2.exe" | "armadal.exe"))
            {
                return Err(InstallError::FleetOperations { path: given, root: install });
            }
            return Err(InstallError::NotArmada { path: given, root: install });
        }
        // Only the base game's own missions folder: never a mod's DLLs under Data/Mods (D6).
        let mdir = child(&install, "missions")
            .or_else(|| child(&child(&install, "Data").unwrap_or_else(|| install.clone()), "missions"))
            .ok_or_else(|| InstallError::NoMissionsFolder { path: given.clone(), root: install.clone() })?;
        let game = if is_armada1(&install) { Game::Armada1 } else { Game::Armada2 };
        Ok(Install { root: install, game, missions_dir: mdir, mounted: None, engine_root: None })
    }

    /// Looks at `path` as the install hosting the mod `name` (#927, D6): the missions to convert
    /// are the ones the game's own mount of that install with that mod shows
    /// ([`Vfs::mount_install`]: the mod, its parent chain, and the base data unless a mod of the
    /// chain is standalone). Accepted wherever that mount succeeds, refused with its reason
    /// otherwise; there is no second rule for where a mod lives. The conversion is of the Armada
    /// II kind (a `.dsl` and `.drl` pair per mission), and its output belongs to this install
    /// with this mod only: key it with [`Install::active_mod`].
    ///
    /// An install that hosts mods has no engine files of the kind the conversion reads at its
    /// root: name a stock Armada II root with [`Install::with_engine`].
    pub fn open_mod(path: &Path, name: &str) -> std::result::Result<Install, ModError> {
        let given = path.to_path_buf();
        let root = match resolve(path) {
            Ok(p) if p.is_dir() => p,
            _ => return Err(ModError::NotAFolder { path: given }),
        };
        let refused = |reason: String| ModError::Mount { path: given.clone(), root: root.clone(), name: name.to_owned(), reason };
        // an empty name would give the base install's key: never a mod
        if name.trim().is_empty() {
            return Err(refused("the mod's name is empty".to_owned()));
        }
        // A name that is a path, and a parent entry that leads out of the mods folder, are the
        // mount's to refuse (it names the mod that asked).
        let vfs = Vfs::mount_install(&root, Some(name)).map_err(|e| refused(e.to_string()))?;
        let mut layer_roots = Vec::new();
        for layer in vfs.layers() {
            match resolve(&layer.root) {
                Ok(p) => layer_roots.push(p),
                Err(e) => return Err(refused(format!("the folder {} of the mount cannot be resolved: {e}", layer.root.display()))),
            }
        }
        let missions_dir = child(&root, "Data").unwrap_or_else(|| root.clone());
        let mounted = Some(Mounted { name: name.to_owned(), vfs: Arc::new(vfs), layer_roots });
        Ok(Install { root, game: Game::Armada2, missions_dir, mounted, engine_root: None })
    }

    /// The same install, reading the Armada II engine files (the executable and its map, which
    /// name the calls of a mission) in the folder `path` instead of at its own root. That folder
    /// is read only, like the install: no output or cache folder may lie in it. An Armada I
    /// conversion reads no engine files and ignores this.
    pub fn with_engine(mut self, path: &Path) -> std::result::Result<Install, InstallError> {
        match resolve(path) {
            Ok(p) if p.is_dir() => {
                self.engine_root = Some(p);
                Ok(self)
            }
            _ => Err(InstallError::NotAFolder { path: path.to_path_buf() }),
        }
    }

    /// The mod this install was opened with ([`Install::open_mod`]), as the caller named it: the
    /// second argument of the folder key ([`key`]) of everything converted from it.
    pub fn active_mod(&self) -> Option<&str> {
        self.mounted.as_ref().map(|m| m.name.as_str())
    }

    /// Where the Armada II engine files are read: the folder given to [`Install::with_engine`],
    /// else the install's root.
    pub fn engine_root(&self) -> &Path {
        self.engine_root.as_deref().unwrap_or(&self.root)
    }

    /// The folders a run only reads, which nothing may be written into: the install, the engine
    /// folder, and with a mod every folder its mount reads (a linked mod folder may lie outside
    /// the install).
    fn read_only_roots(&self) -> Vec<&Path> {
        let layers = self.mounted.iter().flat_map(|m| m.layer_roots.iter().map(PathBuf::as_path));
        std::iter::once(self.root.as_path()).chain(self.engine_root.as_deref()).chain(layers).collect()
    }
}

/// The missions of an install opened with a mod, as its mount resolves them: every
/// `missions/**/<name>S.dsl` the mount shows, with the `<name>R.drl` it shows beside it, in path
/// order; each is written under the stems of the maps the mount shows in `bzn/` that name it. So
/// a child mod converts its parent's missions under the names its own maps give them, and a file
/// one mod of the chain overrides is the overriding one.
///
/// A mission DLL the mount finds inside an archive is not converted (the conversion reads DLL
/// files): it is listed and fails with that reason, after the others.
///
/// INTERIM: a mod that hides its parents' maps still converts its parents' mission DLLs (under
/// their own names where no map it shows names them). Whether the original runs them: not
/// measured.
fn mount_missions(vfs: &Vfs, only: &[String]) -> Vec<Mission> {
    let file = |virtual_path: &str| match vfs.resolve(virtual_path) {
        Some((_, Source::Loose(p))) => Some(Ok(p.clone())),
        Some((layer, _)) => Some(Err(format!(
            "{virtual_path} is inside an archive of the layer {:?}: only a mission DLL that is a file is converted",
            vfs.layers()[layer].name()
        ))),
        None => None,
    };
    let (mut pairs, mut packed) = (Vec::new(), Vec::new());
    for dsl in vfs.list("missions") {
        let Some(base) = dsl.strip_suffix("s.dsl") else { continue };
        let name = base.rsplit('/').next().unwrap_or(base).to_owned();
        let found = file(&dsl).map(|d| (d, file(&format!("{base}r.drl")).transpose()));
        match found {
            Some((Ok(d), Ok(r))) => pairs.push((d, r)),
            Some((Err(why), _)) | Some((_, Err(why))) => packed.push((name, why)),
            None => {}
        }
    }
    let maps: Vec<(String, Vec<u8>)> = vfs
        .list("bzn")
        .into_iter()
        .filter_map(|p| {
            let stem = p.strip_prefix("bzn/")?.strip_suffix(".bzn").filter(|s| !s.contains('/'))?.to_owned();
            let (layer, source) = vfs.resolve(&p)?;
            Some((stem, vfs.read_from(layer, source).ok()?))
        })
        .collect();
    let only_low: Vec<String> = only.iter().map(|x| py_lower(x)).collect();
    let mut missions: Vec<Mission> = convert::plan_resolved(pairs, &maps, only)
        .into_iter()
        .map(|j| Mission { name: j.mission.clone(), targets: j.targets.clone(), work: Work::A2(j) })
        .collect();
    missions.extend(
        packed
            .into_iter()
            .filter(|(name, _)| only_low.is_empty() || only_low.contains(name))
            .map(|(name, why)| Mission { name, targets: Vec::new(), work: Work::Refused(why) }),
    );
    missions
}

// ------------------------------------------------------------------ discovery

/// What a run converts.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Only these missions (Armada II: a mission such as `a2_fed01`; Armada I: a map), compared in
    /// lower case. Empty: every mission, and the run is a full one (it clears what an earlier run
    /// wrote first).
    pub only: Vec<String>,
    /// Armada I: also the DLL pairs no map names.
    pub include_unmapped: bool,
    /// Full runs only: discover the missions (and load the engine data) and ask `cancel` once
    /// before clearing the earlier conversion, so that a cancel at the start or a discovery fault
    /// keeps it (see [`convert_install`]). Off by default: the command line clears first, which its
    /// console output pins. The game sets it.
    pub prepare_first: bool,
    /// Tests only, hidden from the documentation: the converter version the cache keys are made
    /// with, instead of [`crate::CONVERTER_VERSION`] (what a raised version would give: every
    /// entry misses and the missions are converted again). `None` in every real run.
    #[doc(hidden)]
    pub cache_version: Option<u32>,
}

#[derive(Debug, Clone)]
enum Work {
    A2(convert::Job),
    A1(a1::mission::Job),
    /// Found, and not convertible for this reason: the mission fails with it.
    Refused(String),
}

/// One mission of an install.
#[derive(Debug, Clone)]
pub struct Mission {
    /// The name the run reports it under (Armada II: the mission; Armada I: the map).
    pub name: String,
    /// The files written for it, `<stem>.lua` each, in `missions/` of the output folder.
    pub targets: Vec<String>,
    work: Work,
}

/// The missions of an install, ready to convert.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub game: Game,
    pub missions: Vec<Mission>,
    /// Whether this is a full run (no `only`).
    pub full: bool,
    engine: Option<Vtables>,
    engine_digest: Option<Vec<u8>>,
    cache_version: Option<u32>,
    install_root: PathBuf,
}

impl Install {
    /// The missions to convert, in the order the Python tools visit them. Armada II also loads the
    /// engine's call names (`Armada2.exe` and `armada2.map` of the install); that failing is the
    /// error text. Reads folder listings and the engine files, no mission DLL.
    pub fn discover(&self, opts: &Options) -> std::result::Result<Discovered, String> {
        let full = opts.only.is_empty();
        match self.game {
            Game::Armada2 => {
                let engine = load_engine(&[], Some(self.engine_root())).map_err(|e| e.message)?;
                let missions = match &self.mounted {
                    Some(m) => mount_missions(&m.vfs, &opts.only),
                    None => {
                        let mut roots = vec![self.missions_dir.clone()];
                        if let Some(b) = self.missions_dir.parent().and_then(|p| child(p, "bzn")) {
                            roots.push(b);
                        }
                        convert::plan(&roots, &opts.only)
                            .into_iter()
                            .map(|j| Mission { name: j.mission.clone(), targets: j.targets.clone(), work: Work::A2(j) })
                            .collect()
                    }
                };
                Ok(Discovered {
                    game: self.game,
                    missions,
                    full,
                    engine: Some(engine),
                    engine_digest: engine_digest(self.engine_root()),
                    cache_version: opts.cache_version,
                    install_root: self.root.clone(),
                })
            }
            Game::Armada1 => {
                let missions = a1::mission::plan(&self.root, opts.include_unmapped, &opts.only)
                    .map_err(|e| e.message)?
                    .into_iter()
                    .map(|j| Mission { name: j.map.clone(), targets: vec![j.map.clone()], work: Work::A1(j) })
                    .collect();
                Ok(Discovered {
                    game: self.game,
                    missions,
                    full,
                    engine: None,
                    engine_digest: None,
                    cache_version: opts.cache_version,
                    install_root: self.root.clone(),
                })
            }
        }
    }
}

impl Discovered {
    /// The install these missions are from.
    pub fn install_root(&self) -> &Path {
        &self.install_root
    }
}

// ------------------------------------------------------------------ the run

/// The listing of a DLL, as the lifters read it: given the file's path and its parsed image (the
/// file was read once already; the Python runs `objdump`). The converter's own decoder is
/// `|_, pe| disasm::disassemble(pe)`.
pub type Decoder<'a> = dyn FnMut(&Path, &Pe) -> Result<Vec<Ins>> + 'a;

/// What happens in a run, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A file of an earlier conversion by the Python tool was renamed to `<name>.old` before the run
    /// began (`old` and `new` are names in `missions/`). Only the clearing at the start reports
    /// this as an event; later moves are in [`Summary::moved_late`].
    MovedAside { old: String, new: String },
    /// The missions are known: how many will be tried.
    Planned { total: usize },
    /// A mission is about to be converted (`index` counts from 0).
    Started { index: usize, total: usize, mission: String },
    /// A line the converter prints for a converted mission (a cached mission replays its lines).
    Console(String),
    /// A mission or one of its files failed; the run goes on. `mission` is always the mission's
    /// name ([`Mission::name`], the key of [`MissionResult::mission`]). `name` is what the command
    /// line prints after `FAIL `: the mission for a conversion failure, the file stem (one of
    /// [`Mission::targets`]) for a file that would overwrite one the tool did not write. `message`
    /// is what follows `FAIL <name>: `.
    Failed { mission: String, name: String, message: String },
    /// A file of this mission was left as it is because the player edited it since the tool wrote it
    /// (only for an [`crate::output::Edited::Keep`] folder). `name` is the file stem.
    KeptEdited { mission: String, name: String },
    /// A mission is done (converted, from the cache or failed).
    Done { index: usize, total: usize, mission: String },
}

/// How many functions of one role (a kind of function of a mission: a rule's condition, an
/// action, ...) a conversion decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoleCount {
    /// Decoded completely.
    pub complete: usize,
    /// Decoded in part (Armada I; an Armada II function decodes completely or not at all).
    pub partial: usize,
    /// All functions of the role, decoded or not.
    pub total: usize,
}

/// What a conversion decoded of one mission, as numbers: the content of the mission's console
/// line, which is written from this (so the two cannot differ).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Counts {
    /// The rules of the mission.
    pub rules: usize,
    /// Armada II: the rules whose condition decoded. Armada I counts conditions as the role
    /// `condition` and has no number of its own here.
    pub rule_conditions_decoded: Option<usize>,
    /// By role name.
    pub roles: std::collections::BTreeMap<String, RoleCount>,
}

impl Counts {
    /// Whether every function of every role decoded completely (and, for Armada II, every
    /// rule's condition).
    pub fn complete(&self) -> bool {
        self.roles.values().all(|r| r.complete == r.total && r.partial == 0) && self.rule_conditions_decoded.is_none_or(|d| d == self.rules)
    }

    fn of_a2(r: &convert::Report) -> Counts {
        let roles = r.functions.iter().map(|(k, v)| (k.clone(), RoleCount { complete: v.1, partial: 0, total: v.0 })).collect();
        Counts { rules: r.rules, rule_conditions_decoded: Some(r.rule_conditions_decoded), roles }
    }

    fn of_a1(r: &a1::Report) -> Counts {
        let n = |v: i64| usize::try_from(v).unwrap_or(0);
        let roles = r.functions.iter().map(|(k, v)| (k.clone(), RoleCount { complete: n(v[1]), partial: n(v[2]), total: n(v[0]) })).collect();
        Counts { rules: r.rules, rule_conditions_decoded: None, roles }
    }
}

/// What became of one mission.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MissionResult {
    pub mission: String,
    /// What the conversion decoded, when this run converted the mission itself. `None` for a
    /// mission that failed and for one that came from the cache (the cache keeps the script
    /// and the console lines, not the numbers).
    pub counts: Option<Counts>,
    /// The converter (or the cache) produced the Lua.
    pub converted: bool,
    /// It came from the cache.
    pub cached: bool,
    /// The files written, `<stem>.lua`.
    pub written: Vec<String>,
    /// Why it failed (the conversion, or a file that would overwrite one the tool did not write).
    pub failures: Vec<String>,
    /// The files kept because the player edited them (`<stem>.lua`, see [`Event::KeptEdited`]).
    pub kept: Vec<String>,
}

impl MissionResult {
    pub fn failed(&self) -> bool {
        !self.failures.is_empty()
    }
}

/// Where a run stopped on a fault that ends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Abort {
    pub stage: Stage,
    pub message: String,
}

/// The part of a run an [`Abort`] happened in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Clearing what an earlier run wrote: nothing else was done.
    Clear,
    /// Discovering or converting: what was written before stays.
    Convert,
}

/// What a run did.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    /// Every mission tried, in order (one that was cancelled before it began is not here).
    pub missions: Vec<MissionResult>,
    /// The number of missions the run planned (0 when it never got to plan).
    pub total: usize,
    /// The run was stopped between two missions (or, with [`Options::prepare_first`], before any).
    pub cancelled: bool,
    /// The earlier conversion was cleared (a full run reached the clearing; it may have failed).
    pub cleared: bool,
    pub aborted: Option<Abort>,
    /// The manifest could not be saved at the end.
    pub finish_error: Option<String>,
    /// Files of the Python tool moved aside while writing, after the start: `(old, new)`.
    pub moved_late: Vec<(String, String)>,
}

impl Summary {
    /// Whether any mission failed, or the run was aborted.
    pub fn any_failed(&self) -> bool {
        self.aborted.is_some() || self.missions.iter().any(MissionResult::failed)
    }

    /// How many missions failed.
    pub fn failed_missions(&self) -> usize {
        self.missions.iter().filter(|m| m.failed()).count()
    }

    /// How many missions produced a file.
    pub fn converted_missions(&self) -> usize {
        self.missions.iter().filter(|m| m.converted && !m.written.is_empty()).count()
    }

    /// Ran to the end: not aborted, not cancelled, manifest saved.
    pub fn completed(&self) -> bool {
        self.aborted.is_none() && !self.cancelled && self.finish_error.is_none()
    }
}

/// The Python exception class a fault stands for (`type(e).__name__`).
pub(crate) fn class_name(e: &DecodeError) -> &'static str {
    match (e.kind, e.class) {
        (ErrorKind::Decode, _) => "DecodeError",
        (ErrorKind::Io, _) => "OSError",
        (_, Some(c)) => c,
        (_, None) => "Exception",
    }
}

fn read_or_none(p: &Path) -> Option<Vec<u8>> {
    std::fs::read(p).ok()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// SHA-256 of the engine executable and map an Armada II run names calls from: the files of the
/// install, found by lower-case name, which the engine lookup reads.
fn engine_digest(install: &Path) -> Option<Vec<u8>> {
    use sha2::{Digest, Sha256};
    let mut v = names(install).ok()?;
    v.sort();
    let mut h = Sha256::new();
    for want in ["armada2.exe", "armada2.map"] {
        let n = v.iter().rev().find(|n| py_lower(n) == want)?;
        let b = std::fs::read(install.join(n)).ok()?;
        h.update((b.len() as u64).to_le_bytes());
        h.update(b);
    }
    Some(h.finalize().to_vec())
}

/// The cached entry for `key`, if the cache is on and has one (and marks it used).
fn lookup(cache: Option<&Cache>, key: Option<&String>) -> Option<Entry> {
    let (c, k) = cache.zip(key)?;
    let e = c.get(k)?;
    c.touch(k);
    Some(e)
}

fn store(cache: Option<&Cache>, key: Option<&String>, e: &Entry) {
    if let Some((c, k)) = cache.zip(key) {
        let _ = c.put(k, e);
    }
}

/// The bytes of the DLLs of one mission, read once for the cache key and handed to the parser, so
/// that no file is read twice.
#[derive(Default)]
struct Loaded(Vec<(PathBuf, Vec<u8>)>);

impl Loaded {
    /// The image and the listing of the DLL at `p`: from the bytes read for the key when there
    /// are some, else from the file.
    fn load(&mut self, p: &Path, decoder: &mut Decoder<'_>) -> Result<(Pe, Vec<Ins>)> {
        let bytes = self.0.iter_mut().find(|(q, _)| q == p).map(|(_, b)| std::mem::take(b)).filter(|b| !b.is_empty());
        let pe = match bytes {
            Some(b) => Pe::from_bytes(b)?,
            None => Pe::open(p)?,
        };
        let ins = decoder(p, &pe)?;
        Ok((pe, ins))
    }
}

/// The cache key of an Armada II mission: its two DLLs' names and bytes and the engine digest.
fn key_builder(kind: &str, version: Option<u32>) -> KeyBuilder {
    match version {
        Some(v) => KeyBuilder::with_version(kind, v),
        None => KeyBuilder::new(kind),
    }
}

fn a2_key(job: &convert::Job, digest: &[u8], version: Option<u32>) -> Option<(String, Loaded)> {
    let dsl = read_or_none(&job.dsl)?;
    let drl = match &job.drl {
        Some(p) => Some(read_or_none(p)?),
        None => None,
    };
    let drl_base = job.drl.as_ref().map(|p| file_name(p));
    let key = key_builder("a2", version)
        .part(file_name(&job.dsl).as_bytes())
        .part(drl_base.as_deref().unwrap_or("").as_bytes())
        .part(&[u8::from(drl.is_some())])
        .part(&dsl)
        .part(drl.as_deref().unwrap_or(&[]))
        .part(digest)
        .finish();
    let mut files = vec![(job.dsl.clone(), dsl)];
    if let (Some(p), Some(b)) = (&job.drl, drl) {
        files.push((p.clone(), b));
    }
    Some((key, Loaded(files)))
}

/// The cache key of an Armada I mission and the bytes read for it.
fn a1_key(map: &str, dsl: &Path, drl: &Path, version: Option<u32>) -> Option<(String, Loaded)> {
    let (d, r) = (read_or_none(dsl)?, read_or_none(drl)?);
    let key = key_builder("a1", version)
        .part(map.as_bytes())
        .part(file_name(dsl).as_bytes())
        .part(file_name(drl).as_bytes())
        .part(&d)
        .part(&r)
        .finish();
    Some((key, Loaded(vec![(dsl.to_path_buf(), d), (drl.to_path_buf(), r)])))
}

/// The cache key of an Armada I mission: its map, the two DLLs' names and bytes (and the build
/// identity, see [`crate::cache::BUILD_ID`]); `None` when a DLL cannot be read.
pub fn a1_cache_key(map: &str, dsl: &Path, drl: &Path) -> Option<String> {
    a1_key(map, dsl, drl, None).map(|(k, _)| k)
}

/// The cache key of an Armada II mission as a run computes it: the two DLLs' names and bytes,
/// the engine files in `engine_root` ([`Install::engine_root`]) and the build identity; `None`
/// when a file cannot be read or that folder has no engine files.
pub fn a2_cache_key(dsl: &Path, drl: Option<&Path>, engine_root: &Path) -> Option<String> {
    let job = convert::Job { mission: String::new(), dsl: dsl.to_path_buf(), drl: drl.map(Path::to_path_buf), targets: Vec::new() };
    a2_key(&job, &engine_digest(engine_root)?, None).map(|(k, _)| k)
}

/// The console line of a converted Armada II mission.
fn a2_line(mission: &str, c: &Counts) -> String {
    let tot: usize = c.roles.values().map(|v| v.total).sum();
    let dec: usize = c.roles.values().map(|v| v.complete).sum();
    let parts: Vec<String> = c.roles.iter().map(|(k, v)| format!("{k} {}/{}", v.complete, v.total)).collect();
    format!(
        "  {mission}: {dec}/{tot} functions decoded {}; rules {}/{}",
        parts.join(", "),
        c.rule_conditions_decoded.unwrap_or(0),
        c.rules
    )
}

/// The console lines of a converted Armada I mission.
fn a1_lines(map: &str, r: &a1::Report, c: &Counts) -> Vec<String> {
    use std::collections::BTreeMap;
    let parts: Vec<String> = c.roles.iter().map(|(k, v)| format!("{k} {}+{}p/{}", v.complete, v.partial, v.total)).collect();
    let mut l = vec![format!("  {map} ({}): rules {}, {}", r.mission, c.rules, parts.join(", "))];
    let sorted: BTreeMap<&String, &i64> = r.registration_calls.iter().collect();
    for (k, v) in sorted {
        l.push(format!("    registration: {k} x{v} in the rule DLL's set-up code is not converted"));
    }
    for e in &r.registration_errors {
        l.push(format!("    registration: {e}"));
    }
    l
}

/// Converts the missions of an install into `out` and tells `sink` what happens.
///
/// A full run (no `only`) first removes what an earlier run of this tool wrote (see
/// [`OutDir::wipe`]); then, for every mission in order: `cancel` is asked (true stops the run here),
/// the mission is looked up in the cache (when one is given; its key is the DLL bytes, the engine
/// files and the build) or converted, and its files are written. A mission that does not convert is
/// reported and the run goes on; a fault that ends the run ([`Abort`]) leaves what was written
/// before it. The manifest is saved at the end whatever happened, and a full run that completed
/// prunes the cache ([`Cache::prune`]). Each DLL is read once.
///
/// # What a stop leaves behind
///
/// By default (the command line's order, which its console output pins) a full run clears the earlier
/// conversion *first*: a cancel before the first mission, or a discovery fault (the Armada II engine
/// files cannot be loaded), then leaves the folder without the earlier missions. With
/// [`Options::prepare_first`] everything that can fail without writing (discovery, loading the
/// engine data, the first cancel check) happens before anything is cleared, so a cancel at the start
/// or such a fault leaves the earlier conversion untouched (and [`Summary::cleared`] false). A cancel
/// later leaves what the missions before it wrote and nothing of the earlier set (a full run replaces
/// it); the manifest always lists exactly what is on disk. A cancel that arrives between the clearing
/// and the first mission still stops there, after the clearing.
///
/// # Panics
///
/// This function does not catch panics (a lifter fault on hostile bytes would be one). A caller on a
/// worker thread must treat a thread that ended by a panic (a `join` error, a closed channel without
/// a [`Summary`]) as an aborted run: what was written before stays, and the manifest may lack the
/// last file's final state, which the next full run repairs.
pub fn convert_install(
    install: &Install,
    opts: &Options,
    out: &mut OutDir,
    cache: Option<&Cache>,
    decoder: &mut Decoder<'_>,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(&Event),
) -> Summary {
    let mut s = Summary::default();
    let mut found = None;
    if opts.prepare_first {
        match install.discover(opts) {
            Ok(d) => found = Some(d),
            Err(message) => {
                s.aborted = Some(Abort { stage: Stage::Convert, message });
                return s;
            }
        }
        if cancel() {
            s.cancelled = true;
            s.total = found.as_ref().map_or(0, |d| d.missions.len());
            return s;
        }
    }
    if opts.only.is_empty() {
        s.cleared = true;
        if let Err(message) = out.wipe() {
            s.aborted = Some(Abort { stage: Stage::Clear, message });
            return s;
        }
    }
    for (old, new) in out.take_moved() {
        sink(&Event::MovedAside { old, new });
    }
    let found = match found {
        Some(d) => Ok(d),
        None => install.discover(opts),
    };
    match found {
        Ok(d) => run_missions(&d, out, cache, decoder, cancel, sink, &mut s),
        Err(message) => s.aborted = Some(Abort { stage: Stage::Convert, message }),
    }
    match out.finish() {
        Ok(()) => {
            if let Some(c) = cache
                && s.completed()
                && opts.only.is_empty()
            {
                c.prune(std::time::Duration::from_secs(30 * 24 * 3600));
            }
        }
        Err(m) => s.finish_error = Some(m),
    }
    s.moved_late = out.take_moved();
    s
}

fn run_missions(
    d: &Discovered,
    out: &mut OutDir,
    cache: Option<&Cache>,
    decoder: &mut Decoder<'_>,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(&Event),
    s: &mut Summary,
) {
    let total = d.missions.len();
    s.total = total;
    sink(&Event::Planned { total });
    for (index, m) in d.missions.iter().enumerate() {
        if cancel() {
            s.cancelled = true;
            return;
        }
        sink(&Event::Started { index, total, mission: m.name.clone() });
        let mut r = MissionResult { mission: m.name.clone(), ..MissionResult::default() };
        let step = convert_mission(d, m, out, cache, decoder, sink, &mut r);
        if let Err(message) = step {
            // the mission that ended the run is in the results, with the fault as its failure
            r.failures.push(message.clone());
            s.missions.push(r);
            s.aborted = Some(Abort { stage: Stage::Convert, message });
            return;
        }
        s.missions.push(r);
        sink(&Event::Done { index, total, mission: m.name.clone() });
    }
}

fn fail(sink: &mut dyn FnMut(&Event), r: &mut MissionResult, name: &str, message: String) {
    sink(&Event::Failed { mission: r.mission.clone(), name: name.to_owned(), message: message.clone() });
    r.failures.push(message);
}

/// One mission: key, cache, conversion, files. `Err` is a fault that ends the run.
fn convert_mission(
    d: &Discovered,
    m: &Mission,
    out: &mut OutDir,
    cache: Option<&Cache>,
    decoder: &mut Decoder<'_>,
    sink: &mut dyn FnMut(&Event),
    r: &mut MissionResult,
) -> std::result::Result<(), String> {
    // what the output depends on; the bytes read for it are used again by the parser
    let (ckey, mut loaded) = match &m.work {
        Work::A2(job) => cache
            .zip(d.engine_digest.as_ref())
            .and_then(|(_, dg)| a2_key(job, dg, d.cache_version))
            .map_or((None, Loaded::default()), |(k, l)| (Some(k), l)),
        Work::Refused(_) => (None, Loaded::default()),
        Work::A1(job) => cache
            .and_then(|_| a1_key(&job.map, &job.dsl, &job.drl, d.cache_version))
            .map_or((None, Loaded::default()), |(k, l)| (Some(k), l)),
    };
    let entry = match lookup(cache, ckey.as_ref()) {
        Some(e) => {
            r.cached = true;
            Some(e)
        }
        None => match &m.work {
            Work::A2(job) => {
                let engine = d.engine.as_ref().expect("an Armada II run has its engine");
                match convert::convert_one_with(job, engine, &mut |p| loaded.load(p, decoder)) {
                    Ok(convert::Outcome::Converted(c)) => {
                        let counts = Counts::of_a2(&c.report);
                        let e = Entry::Converted { source: c.source.clone().into_bytes(), console: vec![a2_line(&c.mission, &counts)] };
                        r.counts = Some(counts);
                        store(cache, ckey.as_ref(), &e);
                        Some(e)
                    }
                    Ok(convert::Outcome::Failed(msg)) => {
                        fail(sink, r, &m.name, msg);
                        None
                    }
                    // every other fault aborts the run; what was written stays
                    Err(e) => return Err(format!("{}: {}: {}", m.name, class_name(&e), e.message)),
                }
            }
            Work::Refused(why) => {
                fail(sink, r, &m.name, why.clone());
                None
            }
            Work::A1(job) => {
                // any fault of one mission fails that mission only
                match a1::mission::convert_one_with(job, &mut |p| loaded.load(p, decoder)) {
                    Ok((src, rep)) => {
                        let counts = Counts::of_a1(&rep);
                        let e = Entry::Converted { source: src.into_bytes(), console: a1_lines(&job.map, &rep, &counts) };
                        r.counts = Some(counts);
                        store(cache, ckey.as_ref(), &e);
                        Some(e)
                    }
                    Err(e) => {
                        fail(sink, r, &m.name, format!("{}: {}", class_name(&e), e.message));
                        None
                    }
                }
            }
        },
    };
    if let Some(Entry::Converted { source, console }) = entry {
        r.converted = true;
        for l in &console {
            sink(&Event::Console(l.clone()));
        }
        // one script per map that runs the mission (the resolver finds it by the map's stem); an
        // Armada II mission no map names keeps its own name
        for stem in &m.targets {
            let file = format!("{stem}.lua");
            match out.write(&file, &source) {
                Ok(()) => r.written.push(file),
                Err(WriteError::Collision(msg)) => fail(sink, r, stem, msg),
                Err(WriteError::Kept) => {
                    sink(&Event::KeptEdited { mission: r.mission.clone(), name: stem.clone() });
                    r.kept.push(file);
                }
                Err(WriteError::Fatal(msg)) => return Err(msg),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod counts_tests {
    use super::*;

    fn role(complete: usize, partial: usize, total: usize) -> RoleCount {
        RoleCount { complete, partial, total }
    }

    /// The counts of an Armada II mission are its report's numbers, and its console line is
    /// written from them (invented role names and numbers).
    #[test]
    fn armada_two_counts_and_the_line_written_from_them() {
        let mut report = convert::Report { mission: "inv01".into(), rules: 7, rule_conditions_decoded: 6, ..convert::Report::default() };
        report.functions.insert("zeta".into(), (4, 3));
        report.functions.insert("alpha".into(), (2, 2));
        let c = Counts::of_a2(&report);
        assert_eq!((c.rules, c.rule_conditions_decoded), (7, Some(6)));
        assert_eq!(c.roles.iter().map(|(k, v)| (k.as_str(), *v)).collect::<Vec<_>>(), [("alpha", role(2, 0, 2)), ("zeta", role(3, 0, 4))]);
        assert_eq!(a2_line("inv01", &c), "  inv01: 5/6 functions decoded alpha 2/2, zeta 3/4; rules 6/7");
        assert!(!c.complete(), "a function and a rule's condition are missing");
        report.functions.insert("zeta".into(), (4, 4));
        assert!(!Counts::of_a2(&report).complete(), "a rule's condition is missing");
        report.rule_conditions_decoded = 7;
        assert!(Counts::of_a2(&report).complete());
    }

    /// The same for Armada I, whose roles also count functions decoded in part.
    #[test]
    fn armada_one_counts_and_the_line_written_from_them() {
        let mut report = a1::Report { mission: "inv".into(), rules: 3, ..a1::Report::default() };
        // in order of first use, not of name: the line and the counts go by name
        report.functions.insert("step".into(), [5, 4, 1, 0]);
        report.functions.insert("condition".into(), [3, 3, 0, 0]);
        let c = Counts::of_a1(&report);
        assert_eq!((c.rules, c.rule_conditions_decoded), (3, None));
        assert_eq!(c.roles.iter().map(|(k, v)| (k.as_str(), *v)).collect::<Vec<_>>(), [("condition", role(3, 0, 3)), ("step", role(4, 1, 5))]);
        assert_eq!(a1_lines("map1", &report, &c), ["  map1 (inv): rules 3, condition 3+0p/3, step 4+1p/5"]);
        assert!(!c.complete(), "one function decoded only in part");
        report.functions.insert("step".into(), [5, 5, 0, 0]);
        assert!(Counts::of_a1(&report).complete());
        // a mission with nothing to count is complete, and a negative number never becomes a count
        assert!(Counts::default().complete());
        report.functions.insert("step".into(), [5, -1, -1, 0]);
        assert_eq!(Counts::of_a1(&report).roles["step"], role(0, 0, 5));
    }
}
