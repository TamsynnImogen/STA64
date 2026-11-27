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
//! The work itself is [`crate::install`] (LUA-15i): this module reads the arguments, checks the
//! folders, prints what the run reports and chooses the exit code; the game uses the same library
//! seam.
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
//! - a run in which any mission failed or was skipped exits 1 even when other scripts were written
//!   (it still prints the warning and the summary); the Python wrapper exits 0 there;
//! - the listing of a DLL comes from the Rust decoder (the caller's closure), not from `objdump`;
//! - the Python's tracebacks are one line of text on stderr (a message, exit code 1);
//! - `--help` prints this tool's own help; an option value that looks like an option (`--out -x`)
//!   is an error as in argparse; paths are kept as the operating system has them (no lossy
//!   conversion), only `--only` names must be UTF-8;
//! - the text of the failure for a truncated PE file (a `struct.error` message in the Python) is
//!   the PE reader's own;
//! - the child's own `--out` check (`drl.refuse_output`) is not repeated: the wrapper's check,
//!   which runs first and covers the same folders, is the one that speaks.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::disasm::Ins;
use crate::install::{self, Event, Install, Options, Stage};
use crate::output::OutDir;
use crate::Result;

pub use crate::install::{a1_cache_key, child, data_dir, is_armada1, key, realpath, refuse, slug, Env};

/// The listing of a DLL, as the lifters read it (the Python runs `objdump`).
pub type ListingFn<'a> = dyn FnMut(&Path) -> Result<Vec<Ins>> + 'a;

/// Standard output and standard error of a run.
pub struct Io<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
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
deleted, and listed in the output. The output folder may not be, contain or lie inside a game folder, and symbolic links in it are refused.\n\n\
Exit code: 0 when every mission converted; 1 when any mission failed or was skipped, even if other\n\
scripts were written (the Python wrapper exits 0 then, after its warning).";

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

/// `main` of `convert_install_missions.py`: the exit code of the run. `listing` gives the listing
/// of a DLL from its path. Output goes to `io`; files go into the output folder only. (The binary
/// uses [`run_with`], whose decoder is handed the parsed file, so no DLL is read twice.)
pub fn run<S: AsRef<OsStr>>(args: &[S], env: &Env, io: &mut Io<'_>, listing: &mut ListingFn<'_>) -> i32 {
    run_with(args, env, io, &mut |p, _| listing(p))
}

/// [`run`] with the decoder of [`install::Decoder`]: given the path and the parsed image.
pub fn run_with<S: AsRef<OsStr>>(
    args: &[S],
    env: &Env,
    io: &mut Io<'_>,
    decoder: &mut install::Decoder<'_>,
) -> i32 {
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
    let inst = match Install::open(&a.install) {
        Ok(i) => i,
        Err(e) => return die(io, &e.describe(&shown)),
    };
    let given_out: PathBuf = match &a.out {
        Some(o) => o.clone(),
        None => match &env.data_dir {
            Some(base) => base.join("converted").join(key(&inst.root, None)),
            None => return die(io, "no user data folder known: set STA64_DATA_DIR or pass --out"),
        },
    };
    let out_res = match install::resolve_output(&given_out, &inst, env) {
        Ok(p) => p,
        Err(m) => return die(io, &m),
    };
    let cache_dir = match (&a.cache_dir, a.cache) {
        (Some(d), _) => Some(d.clone()),
        (None, true) => match &env.data_dir {
            Some(base) => Some(base.join("convert-cache").join(key(&inst.root, None))),
            None => return die(io, "no user data folder known: set STA64_DATA_DIR or pass --cache-dir"),
        },
        (None, false) => None,
    };
    let cache = match cache_dir {
        Some(d) => match install::open_cache(&d, &out_res, &inst, env) {
            Ok(c) => Some(c),
            Err(m) => return die(io, &m),
        },
        None => None,
    };
    if a.include_unmapped && inst.game != install::Game::Armada1 {
        return die(io, &format!("--install {shown}: --include-unmapped is for Armada I installs"));
    }
    let mut outdir = match OutDir::inspect(&given_out, out_res, a.keep_foreign) {
        Ok(o) => o,
        Err(m) => return die(io, &m),
    };
    say(io, &format!("converting {} -> {}", inst.root.display(), given_out.display()));
    let opts = Options { only: a.only.clone(), include_unmapped: a.include_unmapped };
    let mut moved_total = 0;
    let summary = {
        let mut sink = |e: &Event| match e {
            Event::MovedAside { old, new } => {
                moved_total += 1;
                say(io, &moved_line(old, new));
            }
            Event::Console(l) => say(io, l),
            Event::Failed { name, message } => say(io, &format!("  FAIL {name}: {message}")),
            Event::Planned { .. } | Event::Started { .. } | Event::Done { .. } => {}
        };
        install::convert_install(&inst, &opts, &mut outdir, cache.as_ref(), decoder, &|| false, &mut sink)
    };
    if let Some(ab) = &summary.aborted {
        die(io, &ab.message);
        if ab.stage == Stage::Clear {
            return 1;
        }
    }
    if let Some(m) = &summary.finish_error {
        return die(io, m);
    }
    // only the files this tool wrote count, not a player's .lua files beside them
    let n = outdir.count_ours();
    let code = i32::from(summary.any_failed());
    if code != 0 && n == 0 {
        return code;
    }
    if code != 0 {
        say(io, &format!("warning: some missions failed to convert (see FAIL lines above); the {n} others are usable"));
    }
    for (old, new) in &summary.moved_late {
        say(io, &moved_line(old, new));
    }
    moved_total += summary.moved_late.len();
    if moved_total > 0 {
        say(io, &format!("{moved_total} file(s) of the Python tool moved aside (renamed to <name>.old, not deleted)"));
    }
    say(io, &format!("{n} mission scripts in {}", given_out.display()));
    say(io, "Start the game as usual; those missions are listed as experimental.");
    // The Python wrapper exits 0 here. A run in which any mission failed or was skipped exits 1
    // (a declared difference), so that a script can tell.
    code
}

fn moved_line(old: &str, new: &str) -> String {
    format!("moved aside (written by the Python tool, maybe edited): missions/{old} -> missions/{new}")
}
