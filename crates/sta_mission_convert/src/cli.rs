//! The command line (LUA-15f, plan steps S7 and S9a): a port of `tools/convert_install_missions.py`
//! with the `main` of `tools/dsl_to_lua.py` (Armada II) and `tools/a1_to_lua.py` (Armada I) that
//! the wrapper runs, so that one command converts an install the way the Python pair does:
//!
//! ```text
//! sta_mission_convert --install <game root> [--out <folder>] [--only <mission>]...
//!                     [--include-unmapped] [--keep-foreign] [--cache | --cache-dir <folder>]
//! ```
//!
//! Same arguments (`--install`, `--out`, `--only`), same console lines in the same order, same
//! exit codes, same output files and folder layout (`<out>/missions/<stem>.lua`), same behaviour
//! on failure: a mission that does not convert (`DecodeError`, `Unsupported` for Armada II; any
//! fault for Armada I) prints `  FAIL ...`, counts, and the run goes on; any other fault of an
//! Armada II run aborts it with the earlier files already written (the wrapper then reports the
//! warning when some scripts exist, and exits 1 when none do).
//!
//! # Differences from the Python, on purpose
//!
//! - **Where it writes and deletes** ([`crate::output`], [`crate::paths`]): every path is resolved
//!   first (symbolic links followed, `..` removed after the existing prefix is resolved) and
//!   compared on that form; the output folder may not be, contain or lie inside an install or
//!   anything that looks like a game folder, nor lie inside the checkout the binary was built in
//!   (build folders excepted); a full run deletes only the files its manifest lists (the Python
//!   deletes every `*.lua` in `<out>/missions`); an output folder that holds `.lua` files the tool
//!   did not write is refused (`--keep-foreign` goes on and leaves them alone; a mission whose file
//!   would collide with one is a `FAIL` line, the run goes on and the exit code says so; a `.lua`
//!   file with the Python tool's header line is renamed to `<name>.old`, never deleted, one output
//!   line per file and a count in the summary); symbolic links at
//!   the folder, at `missions/` or at a file to replace are refused; every file is written to a
//!   temporary name and renamed into place.
//! - `--include-unmapped` (Armada I) is the option of `a1_to_lua.py`, offered because the wrapper
//!   cannot pass it; `--keep-foreign`, `--cache` and `--cache-dir` are new ([`crate::cache`]: a
//!   mission whose inputs and build are unchanged is not converted again, its console lines and file
//!   are replayed; failures are not cached);
//! - the listing of a DLL comes from the Rust decoder (the caller's closure), not from `objdump`;
//! - the Python's tracebacks are one line of text on stderr (a message, exit code 1);
//! - `--help` prints this tool's own help; an option value that looks like an option (`--out -x`)
//!   is an error as in argparse; paths are kept as the operating system has them (no lossy
//!   conversion), only `--only` names must be UTF-8;
//! - the text of the failure for a truncated PE file (a `struct.error` message in the Python) is
//!   the PE reader's own;
//! - the child's own `--out` check (`drl.refuse_output`) is not repeated: the wrapper's check,
//!   which runs first and covers the same folders, is the one that speaks.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::a1;
use crate::cache::{Cache, Entry, KeyBuilder};
use crate::convert::{self, load_engine, Outcome};
use crate::disasm::Ins;
use crate::output::{OutDir, WriteError};
use crate::paths::{contains, game_root_above, resolve};
use crate::pyval::py_lower;
use crate::{DecodeError, ErrorKind, Result};

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
        let repo_root = resolve(&repo).ok().filter(|r| r.is_dir());
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
    pub install: PathBuf,
    pub out: Option<PathBuf>,
    pub only: Vec<String>,
    pub include_unmapped: bool,
    pub keep_foreign: bool,
    pub cache: bool,
    pub cache_dir: Option<PathBuf>,
}

const USAGE: &str = "usage: sta_mission_convert [-h] --install INSTALL [--out OUT] [--only ONLY] \
[--include-unmapped] [--keep-foreign] [--cache] [--cache-dir CACHE_DIR]";

const HELP: &str = "\nConvert an install's own compiled missions (Armada I and II) to Lua.\n\n\
options:\n\
  -h, --help           show this help and exit\n\
  --install INSTALL    the game install root (read-only)\n\
  --out OUT            output folder instead of the data directory's; the Lua goes to OUT/missions,\n\
                       a manifest of what was written to OUT/.sta_mission_convert.manifest\n\
  --only ONLY          one mission, e.g. a2_fed01 (Armada I: a map, e.g. fed1); repeatable\n\
  --include-unmapped   Armada I: also the DLL pairs no map names\n\
  --keep-foreign       go on in an output folder that holds .lua files this tool did not write,\n\
                       leaving them untouched (default: refuse)\n\
  --cache              keep converted missions in the data directory and reuse them\n\
  --cache-dir DIR      the same, in DIR\n\n\
A full run (no --only) deletes only the files this tool wrote before (its manifest lists them), not\n\
every .lua file in the folder as the Python tool does; a .lua file whose first line is the Python\n\
tool's header (an earlier conversion by it, maybe edited since) is renamed to <name>.old, never\n\
deleted, and listed in the output. The output folder may not be, contain or lie inside a game folder, and symbolic links in it are refused.";

const OPTIONS: [&str; 8] =
    ["--help", "--install", "--out", "--only", "--include-unmapped", "--keep-foreign", "--cache", "--cache-dir"];

enum Parsed {
    Help,
    Run(Args),
}

fn looks_negative_number(s: &str) -> bool {
    let t = s.strip_prefix('-').unwrap_or(s);
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || c == '.') && t.chars().any(|c| c.is_ascii_digit())
}

/// argparse's behaviour for these options: `--name value`, `--name=value`, an unambiguous prefix,
/// and a value that looks like an option is no value.
fn parse_args(args: &[OsString]) -> std::result::Result<Parsed, String> {
    let mut a = Args::default();
    let mut install: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let raw = args[i].to_string_lossy().into_owned();
        i += 1;
        if raw == "-h" {
            return Ok(Parsed::Help);
        }
        // `--name=value`: split on the first `=` of the lossy text, the value from the original
        let (name, inline): (String, Option<OsString>) = match raw.split_once('=') {
            Some((n, _)) if n.starts_with("--") => {
                let owned = args[i - 1].clone();
                let bytes_prefix = n.len() + 1;
                #[cfg(unix)]
                let v = {
                    use std::os::unix::ffi::{OsStrExt, OsStringExt};
                    OsString::from_vec(owned.as_bytes()[bytes_prefix..].to_vec())
                };
                #[cfg(not(unix))]
                let v = OsString::from(owned.to_string_lossy()[bytes_prefix..].to_owned());
                (n.to_owned(), Some(v))
            }
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
        let flag = matches!(opt, "--help" | "--include-unmapped" | "--keep-foreign" | "--cache");
        if flag && inline.is_some() {
            return Err(format!("argument {opt}: ignored explicit argument"));
        }
        let mut value = || -> std::result::Result<OsString, String> {
            if let Some(v) = inline.clone() {
                return Ok(v);
            }
            match args.get(i) {
                Some(v) => {
                    let t = v.to_string_lossy();
                    if t.starts_with('-') && t.len() > 1 && !looks_negative_number(&t) {
                        return Err(format!("argument {opt}: expected one argument"));
                    }
                    i += 1;
                    Ok(v.clone())
                }
                None => Err(format!("argument {opt}: expected one argument")),
            }
        };
        match opt {
            "--help" => return Ok(Parsed::Help),
            "--install" => install = Some(PathBuf::from(value()?)),
            "--out" => a.out = Some(PathBuf::from(value()?)),
            "--only" => {
                let v = value()?;
                a.only.push(
                    v.into_string().map_err(|_| "argument --only: the mission name is not valid UTF-8".to_owned())?,
                );
            }
            "--cache-dir" => a.cache_dir = Some(PathBuf::from(value()?)),
            "--include-unmapped" => a.include_unmapped = true,
            "--keep-foreign" => a.keep_foreign = true,
            "--cache" => a.cache = true,
            _ => unreachable!("listed option"),
        }
    }
    a.install = install.ok_or_else(|| "the following arguments are required: --install".to_owned())?;
    Ok(Parsed::Run(a))
}

// ------------------------------------------------------------------ the run

fn say(io: &mut Io<'_>, s: &str) {
    let _ = writeln!(io.out, "{s}");
}

fn die(io: &mut Io<'_>, s: &str) -> i32 {
    let _ = writeln!(io.err, "{s}");
    1
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
pub fn run<S: AsRef<OsStr>>(args: &[S], env: &Env, io: &mut Io<'_>, listing: &mut ListingFn<'_>) -> i32 {
    let args: Vec<OsString> = args.iter().map(|s| s.as_ref().to_os_string()).collect();
    let a = match parse_args(&args) {
        Ok(Parsed::Run(a)) => a,
        Ok(Parsed::Help) => {
            say(io, &format!("{USAGE}\n{HELP}"));
            return 0;
        }
        Err(m) => {
            let _ = writeln!(io.err, "{USAGE}\nsta_mission_convert: error: {m}");
            return 2;
        }
    };
    let shown = a.install.display().to_string();
    let install = match resolve(&a.install) {
        Ok(p) if p.is_dir() => p,
        _ => return die(io, &format!("--install {shown}: not a folder")),
    };
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
                    "--install {shown}: a Fleet Operations style install (engine in Data/): it ships Lua missions, \
                     there are no mission DLLs to convert"
                ),
            );
        }
        return die(
            io,
            &format!("--install {shown}: neither an Armada I root (Armada.exe) nor an Armada II root (Armada2.exe)"),
        );
    }
    // Only the base game's own missions folder: never a mod's DLLs under Data/Mods (D6).
    let mdir = child(&install, "missions")
        .or_else(|| child(&child(&install, "Data").unwrap_or_else(|| install.clone()), "missions"));
    let Some(mdir) = mdir else {
        return die(io, &format!("--install {shown}: no missions folder"));
    };
    let given_out: PathBuf = match &a.out {
        Some(o) => o.clone(),
        None => match &env.data_dir {
            Some(base) => base.join("converted").join(key(&install, None)),
            None => return die(io, "no user data folder known: set STA64_DATA_DIR or pass --out"),
        },
    };
    let out_res = match resolve(&given_out) {
        Ok(p) => p,
        Err(m) => return die(io, &format!("output folder {} cannot be resolved: {m}", given_out.display())),
    };
    if let Some(why) = refuse(&out_res, &[&install], env) {
        return die(io, &format!("output folder {} {why}", given_out.display()));
    }
    let cache = match (&a.cache_dir, a.cache) {
        (Some(d), _) => Some(d.clone()),
        (None, true) => match &env.data_dir {
            Some(base) => Some(base.join("convert-cache").join(key(&install, None))),
            None => return die(io, "no user data folder known: set STA64_DATA_DIR or pass --cache-dir"),
        },
        (None, false) => None,
    };
    let cache = match cache {
        Some(d) => {
            let res = match resolve(&d) {
                Ok(p) => p,
                Err(m) => return die(io, &format!("cache folder {} cannot be resolved: {m}", d.display())),
            };
            if let Some(why) = refuse(&res, &[&install], env) {
                return die(io, &format!("cache folder {} {why}", d.display()));
            }
            if contains(&out_res, &res) || contains(&res, &out_res) {
                return die(io, "the cache folder and the output folder may not contain one another");
            }
            Some(Cache::new(res))
        }
        None => None,
    };
    let a1 = is_armada1(&install);
    if a.include_unmapped && !a1 {
        return die(io, &format!("--install {shown}: --include-unmapped is for Armada I installs"));
    }
    let mut outdir = match OutDir::inspect(&given_out, out_res, a.keep_foreign) {
        Ok(o) => o,
        Err(m) => return die(io, &m),
    };
    say(io, &format!("converting {} -> {}", install.display(), given_out.display()));
    if a.only.is_empty()
        && let Err(m) = outdir.wipe()
    {
        // a full re-run replaces the previous conversion: what this tool wrote before goes
        return die(io, &m);
    }
    let mut moved_total = report_moved(io, &mut outdir);
    let (code, completed) = if a1 {
        child_a1(&install, &a, &mut outdir, cache.as_ref(), io, listing)
    } else {
        child_a2(&install, &mdir, &a, &mut outdir, cache.as_ref(), io, listing)
    };
    if let Err(m) = outdir.finish() {
        return die(io, &m);
    }
    if let Some(c) = &cache
        && completed
        && a.only.is_empty()
    {
        c.prune(std::time::Duration::from_secs(30 * 24 * 3600));
    }
    // only the files this tool wrote count, not a player's .lua files beside them
    let n = outdir.count_ours();
    if code != 0 && n == 0 {
        return code;
    }
    if code != 0 {
        say(io, &format!("warning: some missions failed to convert (see FAIL lines above); the {n} others are usable"));
    }
    moved_total += report_moved(io, &mut outdir);
    if moved_total > 0 {
        say(io, &format!("{moved_total} file(s) of the Python tool moved aside (renamed to <name>.old, not deleted)"));
    }
    say(io, &format!("{n} mission scripts in {}", given_out.display()));
    say(io, "Start the game as usual; those missions are listed as experimental.");
    0
}

/// One line per file moved aside since the last call; how many.
fn report_moved(io: &mut Io<'_>, out: &mut OutDir) -> usize {
    let moved = out.take_moved();
    for (old, new) in &moved {
        say(io, &format!("moved aside (written by the Python tool, maybe edited): missions/{old} -> missions/{new}"));
    }
    moved.len()
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

/// `dsl_to_lua.py main` for one install: the console lines and the files, as it prints and writes
/// them. Returns the child's exit code and whether it ran to the end (not aborted).
fn child_a2(
    install: &Path,
    mdir: &Path,
    a: &Args,
    out: &mut OutDir,
    cache: Option<&Cache>,
    io: &mut Io<'_>,
    listing: &mut ListingFn<'_>,
) -> (i32, bool) {
    let mut roots = vec![mdir.to_path_buf()];
    if let Some(b) = mdir.parent().and_then(|p| child(p, "bzn")) {
        roots.push(b);
    }
    let engine = match load_engine(&roots, Some(install)) {
        Ok(e) => e,
        Err(e) => return (die(io, &e.message), false),
    };
    let digest = engine_digest(install);
    let mut failed = 0;
    for job in convert::plan(&roots, &a.only) {
        let dsl_base = file_name(&job.dsl);
        let drl_base = job.drl.as_ref().map(|p| file_name(p));
        // the cache key: what the output depends on
        let ckey = cache.zip(digest.as_ref()).and_then(|(_, d)| {
            let dsl = read_or_none(&job.dsl)?;
            let drl = match &job.drl {
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
        let entry = match lookup(cache, ckey.as_ref()) {
            Some(e) => Some(e),
            None => match convert::convert_one(&job, &engine, listing) {
                Ok(Outcome::Converted(c)) => {
                    let e = Entry::Converted { source: c.source.clone().into_bytes(), console: vec![a2_line(&c)] };
                    store(cache, ckey.as_ref(), &e);
                    Some(e)
                }
                Ok(Outcome::Failed(m)) => {
                    failed += 1;
                    say(io, &format!("  FAIL {}: {m}", job.mission));
                    None
                }
                // every other fault aborts the run; what was written stays
                Err(e) => {
                    return (die(io, &format!("{}: {}: {}", job.mission, class_name(&e), e.message)), false);
                }
            },
        };
        if let Some(Entry::Converted { source, console }) = entry {
            for l in &console {
                say(io, l);
            }
            // one script per map that runs the mission (the resolver finds it by the map's stem);
            // a mission no map names keeps its own name
            for stem in &job.targets {
                match out.write(&format!("{stem}.lua"), &source) {
                    Ok(()) => {}
                    Err(WriteError::Collision(m)) => {
                        failed += 1;
                        say(io, &format!("  FAIL {stem}: {m}"));
                    }
                    Err(WriteError::Fatal(m)) => return (die(io, &m), false),
                }
            }
        }
    }
    (i32::from(failed != 0), true)
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
    out: &mut OutDir,
    cache: Option<&Cache>,
    io: &mut Io<'_>,
    listing: &mut ListingFn<'_>,
) -> (i32, bool) {
    let jobs = match a1::mission::plan(install, a.include_unmapped, &a.only) {
        Ok(j) => j,
        Err(e) => return (die(io, &e.message), false),
    };
    let mut failed = 0;
    for job in jobs {
        let ckey = cache.and_then(|_| a1_cache_key(&job.map, &job.dsl, &job.drl));
        let entry = match lookup(cache, ckey.as_ref()) {
            Some(e) => Some(e),
            // any fault of one mission fails that mission only
            None => match a1::mission::convert_one(&job, listing) {
                Ok((src, rep)) => {
                    let e = Entry::Converted { source: src.into_bytes(), console: a1_lines(&job.map, &rep) };
                    store(cache, ckey.as_ref(), &e);
                    Some(e)
                }
                Err(e) => {
                    failed += 1;
                    say(io, &format!("  FAIL {}: {}: {}", job.map, class_name(&e), e.message));
                    None
                }
            },
        };
        if let Some(Entry::Converted { source, console }) = entry {
            for l in &console {
                say(io, l);
            }
            match out.write(&format!("{}.lua", job.map), &source) {
                Ok(()) => {}
                Err(WriteError::Collision(m)) => {
                    failed += 1;
                    say(io, &format!("  FAIL {}: {m}", job.map));
                }
                Err(WriteError::Fatal(m)) => return (die(io, &m), false),
            }
        }
    }
    (i32::from(failed != 0), true)
}

/// The cache key of an Armada I mission: its map, the two DLLs' names and bytes (and the build
/// identity, see [`crate::cache::BUILD_ID`]); `None` when a DLL cannot be read.
pub fn a1_cache_key(map: &str, dsl: &Path, drl: &Path) -> Option<String> {
    Some(
        KeyBuilder::new("a1")
            .part(map.as_bytes())
            .part(file_name(dsl).as_bytes())
            .part(file_name(drl).as_bytes())
            .part(&read_or_none(dsl)?)
            .part(&read_or_none(drl)?)
            .finish(),
    )
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
