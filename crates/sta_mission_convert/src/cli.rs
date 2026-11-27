//! The command line (LUA-15f, plan steps S7 and S9a): a port of `tools/convert_install_missions.py`
//! with the `main` of `tools/dsl_to_lua.py` (Armada II) and `tools/a1_to_lua.py` (Armada I) that
//! the wrapper runs, so that one command converts an install the way the Python pair does:
//!
//! ```text
//! sta_mission_convert --install <game root> [--out <folder>] [--only <mission>]...
//!                     [--include-unmapped] [--cache | --cache-dir <folder>]
//! ```
//!
//! Same arguments (`--install`, `--out`, `--only`), same console lines in the same order, same
//! exit codes, same output files and folder layout (`<out>/missions/<stem>.lua`), same behaviour
//! on failure: a mission that does not convert (`DecodeError`, `Unsupported` for Armada II; any
//! fault for Armada I) prints `  FAIL ...`, counts, and the run goes on; any other fault of an
//! Armada II run aborts it with the earlier files already written (the wrapper then reports the
//! warning when some scripts exist, and exits 1 when none do). Differences, on purpose:
//!
//! - `--include-unmapped` (Armada I) is the option of `a1_to_lua.py`, offered here because the
//!   wrapper has no way to pass it;
//! - `--cache` and `--cache-dir` use [`crate::cache`]: a mission whose inputs and converter
//!   version are unchanged is not converted again, its console lines and file are replayed;
//! - the listing of a DLL comes from the Rust decoder (the caller's closure), not from `objdump`;
//! - the Python's tracebacks are one line of text on stderr (a message, exit code 1);
//! - the repository check refuses an output folder inside the checkout this binary was built in
//!   (build folders excepted), as the Python refuses the checkout it lives in; a binary built
//!   elsewhere has no checkout to protect (`Env::repo_root`), the install check always applies;
//! - the child's own `--out` check (`drl.refuse_output`) is not repeated: the wrapper's check,
//!   which runs first and covers the same folders, is the one that speaks.
//!
//! Nothing is written outside the output folder (and the cache folder, when one is asked for),
//! and neither may lie inside the install (or the repository).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::a1::{self, A1Dll};
use crate::cache::{Cache, Entry, KeyBuilder};
use crate::convert::{self, convert, load_engine, RuleSource};
use crate::disasm::Ins;
use crate::drl::Failure;
use crate::pyval::py_lower;
use crate::{DecodeError, ErrorKind, Pe, Result};

/// The listing of a DLL, as the lifters read it (the Python runs `objdump`).
pub type ListingFn<'a> = dyn FnMut(&Path) -> Result<Vec<Ins>> + 'a;

/// Standard output and standard error of a run.
pub struct Io<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

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
        let repo_root = std::fs::canonicalize(repo).ok();
        let mut target_dirs: Vec<PathBuf> = repo_root.iter().map(|r| r.join("target")).collect();
        target_dirs.extend(get("CARGO_TARGET_DIR").map(PathBuf::from));
        Env { data_dir: data_dir(&get, std::env::consts::OS), repo_root, target_dirs }
    }
}

/// `slug`: lower-case, every run of other characters one `-`, trimmed; `install` when nothing is
/// left (`sta_mission::converted::key` uses the same rule).
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in s.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            out.push(c);
            gap = false;
        } else {
            gap = true;
        }
    }
    if out.is_empty() {
        "install".to_owned()
    } else {
        out
    }
}

/// `key(install, mod)`: `slug(install folder name)`, plus `+` and `slug(mod)` for a mod.
pub fn key(install: &Path, modname: Option<&str>) -> String {
    let real = realpath(install);
    let base = real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let k = slug(&base);
    match modname.filter(|m| !m.is_empty()) {
        Some(m) => format!("{k}+{}", slug(m)),
        None => k,
    }
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

/// `os.path.realpath`: symbolic links resolved as far as the path exists, the rest appended.
pub fn realpath(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = abs.clone();
    loop {
        if let Ok(mut c) = std::fs::canonicalize(&cur) {
            for x in rest.iter().rev() {
                c.push(x);
            }
            return c;
        }
        match (cur.file_name().map(std::ffi::OsStr::to_owned), cur.parent().map(Path::to_path_buf)) {
            (Some(n), Some(par)) => {
                rest.push(n);
                cur = par;
            }
            _ => return abs,
        }
    }
}

/// `inside(path, root)`: `path` is `root` or below it, by real path.
pub fn inside(path: &Path, root: &Path) -> bool {
    realpath(path).starts_with(realpath(root))
}

/// Why `out` is refused, or `None`: derived from game files, so never in the checkout (its build
/// folders excepted) or an install.
pub fn refuse(out: &Path, installs: &[&Path], env: &Env) -> Option<String> {
    if let Some(repo) = &env.repo_root
        && inside(out, repo)
        && !env.target_dirs.iter().any(|b| inside(out, b))
    {
        return Some("is inside the repository: output is derived from game files".to_owned());
    }
    installs
        .iter()
        .find(|r| inside(out, r))
        .map(|r| format!("is inside the install {}: installs are read-only", r.display()))
}

fn names(dir: &Path) -> std::io::Result<Vec<String>> {
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

// ------------------------------------------------------------------ arguments

/// The arguments of a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    pub install: String,
    pub out: Option<String>,
    pub only: Vec<String>,
    pub include_unmapped: bool,
    pub cache: bool,
    pub cache_dir: Option<String>,
}

const USAGE: &str = "usage: sta_mission_convert [-h] --install INSTALL [--out OUT] [--only ONLY] \
[--include-unmapped] [--cache] [--cache-dir CACHE_DIR]";

const OPTIONS: [&str; 7] = ["--help", "--install", "--out", "--only", "--include-unmapped", "--cache", "--cache-dir"];

enum Parsed {
    Help,
    Run(Args),
}

/// argparse's behaviour for these options: `--name value`, `--name=value`, an unambiguous prefix.
fn parse_args(args: &[String]) -> std::result::Result<Parsed, String> {
    let mut a = Args::default();
    let mut install: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let raw = &args[i];
        i += 1;
        if raw == "-h" {
            return Ok(Parsed::Help);
        }
        let (name, inline) = match raw.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_owned(), Some(v.to_owned())),
            _ => (raw.clone(), None),
        };
        if !name.starts_with("--") {
            return Err(format!("unrecognized arguments: {raw}"));
        }
        let exact = OPTIONS.iter().find(|o| **o == name);
        let hits: Vec<&&str> = OPTIONS.iter().filter(|o| o.starts_with(name.as_str())).collect();
        let opt = match (exact, hits.as_slice()) {
            (Some(o), _) => *o,
            (None, [one]) => **one,
            (None, []) => return Err(format!("unrecognized arguments: {raw}")),
            (None, many) => {
                let list: Vec<&str> = many.iter().map(|o| **o).collect();
                return Err(format!("ambiguous option: {name} could match {}", list.join(", ")));
            }
        };
        let flag = matches!(opt, "--help" | "--include-unmapped" | "--cache");
        let mut value = || -> std::result::Result<String, String> {
            if let Some(v) = inline.clone() {
                return Ok(v);
            }
            let v = args.get(i).filter(|v| !v.starts_with("--")).cloned();
            match v {
                Some(v) => {
                    i += 1;
                    Ok(v)
                }
                None => Err(format!("argument {opt}: expected one argument")),
            }
        };
        if flag && inline.is_some() {
            return Err(format!("argument {opt}: ignored explicit argument"));
        }
        match opt {
            "--help" => return Ok(Parsed::Help),
            "--install" => install = Some(value()?),
            "--out" => a.out = Some(value()?),
            "--only" => a.only.push(value()?),
            "--cache-dir" => a.cache_dir = Some(value()?),
            "--include-unmapped" => a.include_unmapped = true,
            "--cache" => a.cache = true,
            _ => unreachable!("listed option"),
        }
    }
    a.install = install.ok_or_else(|| "the following arguments are required: --install".to_owned())?;
    Ok(Parsed::Run(a))
}

// ------------------------------------------------------------------ the run

/// What one mission's conversion gave: the lines and file to write, or the failure text.
type Outcome = Entry;

fn say(io: &mut Io<'_>, s: &str) {
    let _ = writeln!(io.out, "{s}");
}

fn die(io: &mut Io<'_>, s: &str) -> i32 {
    let _ = writeln!(io.err, "{s}");
    1
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The Python exception class a fault stands for (`type(e).__name__`).
fn class_name(e: &DecodeError) -> &'static str {
    match (e.kind, e.class) {
        (ErrorKind::Decode, _) => "DecodeError",
        (ErrorKind::Io, _) => "OSError",
        (_, Some(c)) => c,
        (_, None) => "Exception",
    }
}

/// `main` of `convert_install_missions.py`: the exit code of the run. `listing` gives the listing
/// of a DLL. Output goes to `io`; files go into the output folder only.
pub fn run(args: &[String], env: &Env, io: &mut Io<'_>, listing: &mut ListingFn<'_>) -> i32 {
    let a = match parse_args(args) {
        Ok(Parsed::Run(a)) => a,
        Ok(Parsed::Help) => {
            say(io, USAGE);
            return 0;
        }
        Err(m) => {
            let _ = writeln!(io.err, "{USAGE}\nsta_mission_convert: error: {m}");
            return 2;
        }
    };
    let install = realpath(Path::new(&a.install));
    if !install.is_dir() {
        return die(io, &format!("--install {}: not a folder", a.install));
    }
    let top = match names(&install) {
        Ok(n) => n,
        Err(e) => return die(io, &format!("{}: {e}", install.display())),
    };
    let low: Vec<String> = top.iter().map(|n| py_lower(n)).collect();
    if !low.iter().any(|n| n == "armada.exe") && !low.iter().any(|n| n == "armada2.exe") {
        let data = child(&install, "Data");
        if let Some(d) = data
            && names(&d).unwrap_or_default().iter().any(|n| matches!(py_lower(n).as_str(), "armada2.exe" | "armadal.exe"))
        {
            return die(
                io,
                &format!(
                    "--install {}: a Fleet Operations style install (engine in Data/): it ships Lua missions, \
                     there are no mission DLLs to convert",
                    a.install
                ),
            );
        }
        return die(
            io,
            &format!("--install {}: neither an Armada I root (Armada.exe) nor an Armada II root (Armada2.exe)", a.install),
        );
    }
    // Only the base game's own missions folder: never a mod's DLLs under Data/Mods (D6).
    let mdir = child(&install, "missions")
        .or_else(|| child(&child(&install, "Data").unwrap_or_else(|| install.clone()), "missions"));
    let Some(mdir) = mdir else {
        return die(io, &format!("--install {}: no missions folder", a.install));
    };
    let out: PathBuf = match &a.out {
        Some(o) => PathBuf::from(o),
        None => match &env.data_dir {
            Some(base) => base.join("converted").join(key(&install, None)),
            None => return die(io, "no user data folder known: set STA64_DATA_DIR or pass --out"),
        },
    };
    if let Some(why) = refuse(&out, &[&install], env) {
        return die(io, &format!("output folder {} {why}", out.display()));
    }
    let cache = match (&a.cache_dir, a.cache) {
        (Some(d), _) => Some(Cache::new(d)),
        (None, true) => match &env.data_dir {
            Some(base) => Some(Cache::new(base.join("convert-cache").join(key(&install, None)))),
            None => return die(io, "no user data folder known: set STA64_DATA_DIR or pass --cache-dir"),
        },
        (None, false) => None,
    };
    if let Some(c) = &cache
        && let Some(why) = refuse(c.dir(), &[&install], env)
    {
        return die(io, &format!("cache folder {} {why}", c.dir().display()));
    }
    let a1 = is_armada1(&install);
    if a.include_unmapped && !a1 {
        return die(io, &format!("--install {}: --include-unmapped is for Armada I installs", a.install));
    }
    say(io, &format!("converting {} -> {}", install.display(), out.display()));
    let outm = out.join("missions");
    if outm.is_dir() && a.only.is_empty() {
        // a full re-run replaces the previous conversion
        match names(&outm) {
            Ok(v) => {
                for f in v.into_iter().filter(|f| f.ends_with(".lua")) {
                    if let Err(e) = std::fs::remove_file(outm.join(&f)) {
                        return die(io, &format!("{}: {e}", outm.join(&f).display()));
                    }
                }
            }
            Err(e) => return die(io, &format!("{}: {e}", outm.display())),
        }
    }
    let code = if a1 {
        child_a1(&install, &a, &out, cache.as_ref(), io, listing)
    } else {
        child_a2(&install, &mdir, &a, &out, cache.as_ref(), io, listing)
    };
    let n = if outm.is_dir() {
        names(&outm).map(|v| v.iter().filter(|f| f.ends_with(".lua")).count()).unwrap_or(0)
    } else {
        0
    };
    if code != 0 && n == 0 {
        return code;
    }
    if code != 0 {
        say(io, &format!("warning: some missions failed to convert (see FAIL lines above); the {n} others are usable"));
    }
    say(io, &format!("{n} mission scripts in {}", out.display()));
    say(io, "Start the game as usual; those missions are listed as experimental.");
    0
}

/// Writes `<out>/missions/<name>`.
fn write_file(outm: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(outm)?;
    std::fs::write(outm.join(name), bytes)
}

fn read_or_none(p: &Path) -> Option<Vec<u8>> {
    std::fs::read(p).ok()
}

/// SHA-256 (hex) of the engine executable and map an Armada II run names calls from: the files
/// of the install, found by lower-case name, which the engine lookup reads.
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

/// `dsl_to_lua.py main` for one install: the console lines and the files, as it prints and writes
/// them. Returns the child's exit code.
fn child_a2(
    install: &Path,
    mdir: &Path,
    a: &Args,
    out: &Path,
    cache: Option<&Cache>,
    io: &mut Io<'_>,
    listing: &mut ListingFn<'_>,
) -> i32 {
    let mut roots = vec![mdir.to_path_buf()];
    if let Some(b) = mdir.parent().and_then(|p| child(p, "bzn")) {
        roots.push(b);
    }
    let engine = match load_engine(&roots, Some(install)) {
        Ok(e) => e,
        Err(e) => return die(io, &e.message),
    };
    let digest = engine_digest(install);
    let stems = convert::map_stems(&roots);
    let only: Vec<String> = a.only.iter().map(|x| py_lower(x)).collect();
    let outm = realpath(out).join("missions");
    let mut failed = 0;
    for root in &roots {
        for (dsl_path, drl_path) in convert::find_missions(root) {
            let dsl_base = file_name(&dsl_path);
            let mission = convert::mission_name(&dsl_base);
            if !only.is_empty() && !only.contains(&py_lower(&mission)) {
                continue;
            }
            let drl_base = drl_path.as_ref().map(|p| file_name(p));
            // the cache key: what the output depends on
            let ckey = cache.zip(digest.as_ref()).and_then(|(_, d)| {
                let dsl = read_or_none(&dsl_path)?;
                let drl = match &drl_path {
                    Some(p) => Some(read_or_none(p)?),
                    None => None,
                };
                Some(
                    KeyBuilder::new("a2")
                        .part(dsl_base.as_bytes())
                        .part(drl_base.as_deref().unwrap_or("").as_bytes())
                        .part(&[u8::from(drl.is_some())])
                        .part(&dsl)
                        .part(drl.as_deref().unwrap_or(&[]))
                        .part(d)
                        .finish(),
                )
            });
            let hit = cache.zip(ckey.as_ref()).and_then(|(c, k)| c.get(k));
            let outcome: Outcome = match hit {
                Some(e) => e,
                None => {
                    let result = (|| -> std::result::Result<convert::Converted, Failure> {
                        let pe = Pe::open(&dsl_path)?;
                        let ins = listing(&dsl_path)?;
                        let mut open = || -> Result<(Pe, Vec<Ins>)> {
                            let p = drl_path.as_ref().expect("a rule DLL");
                            let rpe = Pe::open(p)?;
                            let rins = listing(p)?;
                            Ok((rpe, rins))
                        };
                        let drl = drl_base.as_ref().map(|b| (b.as_str(), &mut open as &mut RuleSource<'_>));
                        convert(&pe, ins, &dsl_base, drl, &engine)
                    })();
                    let e = match result {
                        Ok(c) => Entry::Converted {
                            source: c.source.clone().into_bytes(),
                            console: vec![a2_line(&c)],
                        },
                        Err(Failure::Unsupported(m)) => Entry::Failed { message: m },
                        Err(Failure::Error(e)) if e.kind == ErrorKind::Decode => Entry::Failed { message: e.message },
                        // every other fault aborts the run; what was written stays
                        Err(Failure::Error(e)) => return die(io, &format!("{mission}: {}: {}", class_name(&e), e.message)),
                    };
                    if let Some((c, k)) = cache.zip(ckey.as_ref()) {
                        let _ = c.put(k, &e);
                    }
                    e
                }
            };
            match outcome {
                Entry::Failed { message } => {
                    failed += 1;
                    say(io, &format!("  FAIL {mission}: {message}"));
                }
                Entry::Converted { source, console } => {
                    for l in &console {
                        say(io, l);
                    }
                    // one script per map that runs the mission (the resolver finds it by the
                    // map's stem); a mission no map names keeps its own name
                    let targets = match stems.get(&py_lower(&mission)).filter(|s| !s.is_empty()) {
                        Some(s) => s.clone(),
                        None => vec![py_lower(&mission)],
                    };
                    for stem in targets {
                        if let Err(e) = write_file(&outm, &format!("{stem}.lua"), &source) {
                            return die(io, &format!("{}: {e}", outm.display()));
                        }
                    }
                }
            }
        }
    }
    i32::from(failed != 0)
}

/// The console line of a converted Armada II mission.
fn a2_line(c: &convert::Converted) -> String {
    let r = &c.report;
    let tot: usize = r.functions.values().map(|v| v.0).sum();
    let dec: usize = r.functions.values().map(|v| v.1).sum();
    let parts: Vec<String> = r.functions.iter().map(|(k, v)| format!("{k} {}/{}", v.1, v.0)).collect();
    format!(
        "  {}: {dec}/{tot} functions decoded {}; rules {}/{}",
        c.mission,
        parts.join(", "),
        r.rule_conditions_decoded,
        r.rules
    )
}

/// `a1_to_lua.py main` for one install.
fn child_a1(
    install: &Path,
    a: &Args,
    out: &Path,
    cache: Option<&Cache>,
    io: &mut Io<'_>,
    listing: &mut ListingFn<'_>,
) -> i32 {
    let found = match a1::find_missions(install, a.include_unmapped) {
        Ok(f) => f,
        Err(e) => return die(io, &e.message),
    };
    let only: Vec<String> = a.only.iter().map(|x| py_lower(x)).collect();
    let outm = realpath(out).join("missions");
    let mut failed = 0;
    for (map, dsl_path, drl_path) in found {
        if !only.is_empty() && !only.contains(&map) {
            continue;
        }
        let ckey = cache.and_then(|_| {
            Some(
                KeyBuilder::new("a1")
                    .part(map.as_bytes())
                    .part(file_name(&dsl_path).as_bytes())
                    .part(file_name(&drl_path).as_bytes())
                    .part(&read_or_none(&dsl_path)?)
                    .part(&read_or_none(&drl_path)?)
                    .finish(),
            )
        });
        let hit = cache.zip(ckey.as_ref()).and_then(|(c, k)| c.get(k));
        let outcome: Outcome = match hit {
            Some(e) => e,
            None => {
                // any fault of one mission fails that mission only
                let mut one = || -> Result<(String, a1::Report)> {
                    let spe = Pe::open(&dsl_path)?;
                    let sins = listing(&dsl_path)?;
                    let sex = spe.exports()?;
                    let s = A1Dll::new(&file_name(&dsl_path), &spe, sins, sex, None)?;
                    let rpe = Pe::open(&drl_path)?;
                    let rins = listing(&drl_path)?;
                    let rex = rpe.exports()?;
                    let r = A1Dll::new(&file_name(&drl_path), &rpe, rins, rex, None)?;
                    let (_, src, rep) = a1::convert_dlls(&s, Some(&r), Some(&map))?;
                    Ok((src, rep))
                };
                let e = match one() {
                    Ok((src, rep)) => Entry::Converted { source: src.into_bytes(), console: a1_lines(&map, &rep) },
                    Err(e) => Entry::Failed { message: format!("{}: {}", class_name(&e), e.message) },
                };
                if let Some((c, k)) = cache.zip(ckey.as_ref()) {
                    let _ = c.put(k, &e);
                }
                e
            }
        };
        match outcome {
            Entry::Failed { message } => {
                failed += 1;
                say(io, &format!("  FAIL {map}: {message}"));
            }
            Entry::Converted { source, console } => {
                for l in &console {
                    say(io, l);
                }
                if let Err(e) = write_file(&outm, &format!("{map}.lua"), &source) {
                    return die(io, &format!("{}: {e}", outm.display()));
                }
            }
        }
    }
    i32::from(failed != 0)
}

/// The console lines of a converted Armada I mission.
fn a1_lines(map: &str, r: &a1::Report) -> Vec<String> {
    let mut fr: Vec<(&String, &[i64; 4])> = r.functions.iter().collect();
    fr.sort_by(|x, y| x.0.cmp(y.0));
    let parts: Vec<String> = fr.iter().map(|(k, v)| format!("{k} {}+{}p/{}", v[1], v[2], v[0])).collect();
    let mut l = vec![format!("  {map} ({}): rules {}, {}", r.mission, r.rules, parts.join(", "))];
    let sorted: BTreeMap<&String, &i64> = r.registration_calls.iter().collect();
    for (k, v) in sorted {
        l.push(format!("    registration: {k} x{v} in the rule DLL's set-up code is not converted"));
    }
    for e in &r.registration_errors {
        l.push(format!("    registration: {e}"));
    }
    l
}
