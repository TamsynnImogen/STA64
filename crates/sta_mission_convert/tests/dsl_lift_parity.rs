//! Install-gated (LUA-15d): the lifter core's intermediate form for every function of every
//! installed Armada II mission DLL equals the original `tools/dsl_to_lua.py`'s.
//!
//! The test needs `STA64_TEST_A2_STOCK_INSTALL` (the Armada II install root, read only),
//! `python3` and GNU `objdump` 2.47 on the path, and skips with a message otherwise. For each
//! `.dsl` under the install it
//!
//! 1. runs `objdump -d -M intel --no-show-raw-insn` on it into a temp directory (the listing is
//!    the lifters' input, `drl_to_lua.disassemble`'s),
//! 2. runs `python3 -I dump_install.py` (a copy in the temp directory, outside the repository)
//!    with that listing: the Python's `Dsl` and `Lifter` print, per function, the intermediate
//!    form in the canonical text of `Lifted::canonical`,
//! 3. lifts the same functions with the Rust `Dsl` and `Lifter` over the same listing, and
//! 4. compares the two texts function by function.
//!
//! Both sides run the call model of `lift::call_model` (the Python's `Lifter.call` is LUA-15h's
//! port; its output is compared by that row's parity test).
//!
//! The functions are the exported code entries and every direct call target of the listing
//! (`function_list` in `tests/dsl_lift/common.rs`).
//!
//! The failure message carries counts and the index of the first differing function, never
//! content. The detail of the first differing functions (both texts) goes to
//! `target/scratch/lift-parity.txt`: it is derived from game files, git-ignored, and must not be
//! committed, attached or pasted anywhere. All temp files, which hold game-derived text, are
//! removed at the end.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use sta_mission_convert::disasm::Ins;
use sta_mission_convert::dsl::Dsl;
use sta_mission_convert::format::py_hex;
use sta_mission_convert::lift::Lifter;
use sta_mission_convert::map::Vtables;
use sta_mission_convert::pyval::Py;
use sta_mission_convert::{ErrorKind, Pe};

#[path = "dsl_lift/common.rs"]
mod common;

const VAR: &str = "STA64_TEST_A2_STOCK_INSTALL";

/// Upper bound, in percent, of the compared functions that may stop with an error (equal on both
/// sides). The stand-in call model makes many engine-heavy functions stop early (about 55 percent
/// in the run the bound was set from), so the bound sits above that with room for a few percent
/// of drift; a change that makes most functions stop, which would leave most of the lifter
/// untested, trips it.
const ERROR_SHARE_BOUND_PERCENT: usize = 70;

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                walk(&p, depth - 1, out);
            }
        } else {
            out.push(p);
        }
    }
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// `drl_to_lua.disassemble`'s parse: `^\s*([0-9a-f]+):\t(.*)$` per line, the text stripped.
fn parse_listing(text: &str) -> Vec<Ins> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let l = line.trim_start_matches(sta_mission_convert::pyval::py_isspace);
        let digits = l.bytes().take_while(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')).count();
        if digits == 0 {
            continue;
        }
        let Some(rest) = l[digits..].strip_prefix(":\t") else {
            continue;
        };
        if let Ok(a) = u32::from_str_radix(&l[..digits], 16) {
            out.push((a, sta_mission_convert::pyval::py_strip(rest).to_owned()));
        }
    }
    out
}

/// The text `dump_install.py` prints for the same DLL.
fn rust_dump(path: &Path, ins: Vec<Ins>) -> String {
    let mut out: Vec<String> = Vec::new();
    let pe = match Pe::open(path) {
        Ok(pe) => pe,
        Err(e) if e.kind == ErrorKind::Decode => {
            return format!("new error {}\n", Py::from(e.message).repr());
        }
        Err(e) => panic!("a mission DLL does not parse: {e}"),
    };
    let exports = pe.exports().expect("the export table reads");
    let d = match Dsl::new(&pe, Vtables::new(), ins, exports) {
        Ok(d) => d,
        Err(e) if e.kind == ErrorKind::Decode => {
            return format!("new error {}\n", Py::from(e.message).repr());
        }
        Err(e) => panic!("Dsl::new: {e}"),
    };
    out.extend(common::dsl_facts(&d));
    let funcs = common::function_list(&d).expect("the function list");
    out.push(format!("functions {}", Py::List(funcs.iter().map(|a| Py::Int(*a)).collect()).repr()));
    let mut lf = Lifter::new(&d);
    for a in &funcs {
        let kinds = match common::kind_facts(&d, *a) {
            Ok(k) => k,
            Err(e) => {
                assert_eq!(e.kind, ErrorKind::Raw);
                out.push("raw".to_owned());
                break;
            }
        };
        out.push(format!("facts {} {kinds}", py_hex(*a)));
        match lf.lift(*a) {
            Ok(l) => out.push(l.canonical().trim_end_matches('\n').to_owned()),
            Err(e) => {
                assert_eq!(e.kind, ErrorKind::Raw);
                out.push("raw".to_owned());
                break;
            }
        }
    }
    out.push(common::static_refs_line(&d));
    out.join("\n") + "\n"
}

/// The text cut at its `facts` lines and its closing `static_refs`/`raw` line: the header, then one
/// piece per function, then the tail.
fn chunks(text: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    for l in text.split_inclusive('\n') {
        if l.starts_with("facts ") || l.starts_with("static_refs ") || l == "raw\n" {
            out.push(String::new());
        }
        out.last_mut().expect("one chunk").push_str(l);
    }
    out
}

#[derive(Default)]
struct Outcome {
    /// Functions in the Python's output.
    functions: usize,
    /// Functions whose text differs (or is missing on one side).
    differing: usize,
    /// Header and tail pieces that differ (the Dsl facts, the function list, `static_refs`).
    other_differing: usize,
    /// Index of the first differing function, 0-based.
    first: Option<usize>,
    /// The first differing pieces, both texts (game-derived: scratch file only).
    detail: String,
    failed: Option<String>,
    // What the equal functions really covered (printed at run time only):
    /// Equal functions that end in an error on both sides (the error texts are equal).
    both_error: usize,
    /// Blocks and statements of the equal functions.
    blocks: usize,
    statements: usize,
    /// A side printed `new error` (the `Dsl` could not be built): nothing was lifted.
    new_error: bool,
    /// Pieces that are `raw` on both sides (a hard Python exception: the later functions are skipped).
    raw_pieces: usize,
    /// Functions in the function list.
    listed: usize,
    /// Of those, the ones neither side lifted (after a `raw`) and the ones only one side lifted.
    skipped: usize,
    skipped_one_side: usize,
}

/// The number of items of the `functions [..]` line.
fn listed_functions(text: &str) -> usize {
    let Some(l) = text.lines().find(|l| l.starts_with("functions ")) else { return 0 };
    if l == "functions []" {
        0
    } else {
        l.matches(", ").count() + 1
    }
}

fn compare(py: &str, rs: &str) -> Outcome {
    let (a, b) = (chunks(py), chunks(rs));
    let mut o = Outcome::default();
    let n = a.len().max(b.len());
    o.new_error = py.starts_with("new error") || rs.starts_with("new error");
    o.listed = listed_functions(py).max(listed_functions(rs));
    let lifted = |c: &[String]| c.iter().filter(|t| t.starts_with("facts ")).count();
    let (lp, lr) = (lifted(&a), lifted(&b));
    o.skipped_one_side = lp.abs_diff(lr);
    o.skipped = o.listed.saturating_sub(lp.max(lr));
    o.raw_pieces = a.iter().zip(&b).filter(|(x, y)| x.as_str() == "raw\n" && y.as_str() == "raw\n").count();
    let mut fi = 0usize; // function index
    for k in 0..n {
        let (x, y) = (a.get(k), b.get(k));
        let is_fn = x.or(y).is_some_and(|t| t.starts_with("facts "));
        let same = x == y;
        if is_fn {
            o.functions += 1;
            if same {
                let t = x.map_or("", String::as_str);
                if !t.contains("\n error None\n") {
                    o.both_error += 1;
                }
                o.blocks += t.lines().filter(|l| l.starts_with(" block ")).count();
                o.statements += t.lines().filter(|l| l.starts_with("  (")).count();
            }
            if !same {
                o.differing += 1;
                o.first.get_or_insert(fi);
            }
            fi += 1;
        } else if !same {
            o.other_differing += 1;
        }
        if !same && o.detail.len() < 200_000 {
            o.detail.push_str(&format!(
                "---- piece {k} (function {}): python\n{}\n---- rust\n{}\n",
                if is_fn { (fi - 1).to_string() } else { "-".into() },
                x.map_or("<missing>", String::as_str),
                y.map_or("<missing>", String::as_str)
            ));
        }
    }
    o
}

fn process(index: usize, dll: &Path, tmp: &Path, tools: &Path) -> Outcome {
    let listing = tmp.join(format!("{index}.lst"));
    let pyout = tmp.join(format!("{index}.py.txt"));
    let fail = |m: &str| Outcome { failed: Some(m.to_owned()), ..Outcome::default() };
    let od = Command::new("objdump").args(["-d", "-M", "intel", "--no-show-raw-insn"]).arg(dll).output();
    let Ok(od) = od else { return fail("objdump could not run") };
    if !od.status.success() {
        return fail("objdump failed");
    }
    if std::fs::write(&listing, &od.stdout).is_err() {
        return fail("the listing could not be written");
    }
    let py = Command::new("python3")
        .arg("-I")
        .arg(tmp.join("dump_install.py"))
        .arg(tools)
        .arg(dll)
        .arg(&listing)
        .arg(&pyout)
        .current_dir(tmp)
        .output();
    match py {
        Ok(o) if o.status.success() => {}
        _ => return fail("the Python driver failed"),
    }
    let py_text = match std::fs::read_to_string(&pyout) {
        Ok(t) => t,
        Err(_) => return fail("the Python output is missing"),
    };
    let ins = parse_listing(&String::from_utf8_lossy(&od.stdout));
    let rs_text = rust_dump(dll, ins);
    compare(&py_text, &rs_text)
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a2_mission_lifter_intermediate_equals_the_originals() {
    let Ok(root) = std::env::var(VAR) else {
        eprintln!("{VAR} not set: skipped");
        return;
    };
    let ver = Command::new("objdump").arg("--version").output();
    let ok_objdump = ver
        .as_ref()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next().map(|l| l.trim().ends_with("2.47")))
        .unwrap_or(false);
    if !ok_objdump {
        eprintln!("objdump 2.47 not found: skipped");
        return;
    }
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 not found: skipped");
        return;
    }
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tools = here.join("../../tools");
    let mut files = Vec::new();
    walk(Path::new(&root), 3, &mut files);
    let mut dlls: Vec<PathBuf> = files.into_iter().filter(|p| has_ext(p, "dsl")).collect();
    dlls.sort_by_key(|p| p.to_string_lossy().into_owned());
    assert!(!dlls.is_empty(), "no mission DLL under the install");

    let tmp = TempDir(std::env::temp_dir().join(format!("sta_lift_parity_{}", std::process::id())));
    std::fs::create_dir_all(&tmp.0).expect("temp dir");
    for f in ["lift_common.py", "dump_install.py"] {
        std::fs::copy(here.join("tests/dsl_lift").join(f), tmp.0.join(f)).expect("copy a driver");
    }

    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Outcome>>> = Mutex::new((0..dlls.len()).map(|_| None).collect());
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get().min(4));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= dlls.len() {
                    break;
                }
                let o = process(i, &dlls[i], &tmp.0, &tools);
                results.lock().expect("results").get_mut(i).expect("slot").replace(o);
            });
        }
    });
    let results: Vec<Outcome> = results.into_inner().expect("results").into_iter().map(|o| o.expect("done")).collect();

    let functions: usize = results.iter().map(|o| o.functions).sum();
    let differing: usize = results.iter().map(|o| o.differing).sum();
    let other: usize = results.iter().map(|o| o.other_differing).sum();
    let failed: Vec<usize> = results.iter().enumerate().filter(|(_, o)| o.failed.is_some()).map(|(i, _)| i).collect();
    let first = results.iter().enumerate().find_map(|(i, o)| o.first.map(|f| (i, f)));
    eprintln!(
        "lifter parity: {} DLLs, {functions} functions compared, {differing} differing, {other} other pieces differing, {} DLLs not run",
        dlls.len(),
        failed.len()
    );
    // What the equal functions really covered. Printed only, never committed.
    let sum = |f: fn(&Outcome) -> usize| -> usize { results.iter().map(f).sum() };
    let both_error = sum(|o| o.both_error);
    eprintln!(
        "  coverage: {both_error} functions end in an error on both sides (equal texts); {} blocks and {} statements compared; \
         {} DLLs where a side could not build the Dsl; {} raw pieces on both sides; {} functions listed, {} skipped on both sides, {} on one side only",
        sum(|o| o.blocks),
        sum(|o| o.statements),
        results.iter().filter(|o| o.new_error).count(),
        sum(|o| o.raw_pieces),
        sum(|o| o.listed),
        sum(|o| o.skipped),
        sum(|o| o.skipped_one_side),
    );
    assert_eq!(sum(|o| o.skipped_one_side), 0, "a function was lifted on one side only");
    // The call model makes engine-heavy functions stop early, so some share of both-error functions
    // is expected; if most of the compared functions stop with an error, the comparison covers too
    // little of the lifter. The bound is a plausibility check, not a measured value.
    assert!(
        both_error * 100 <= functions * ERROR_SHARE_BOUND_PERCENT,
        "implausibly many functions end in an error on both sides ({both_error} of {functions})"
    );
    if differing > 0 || other > 0 || !failed.is_empty() {
        let scratch = here.join("../../target/scratch");
        let detail: String = results
            .iter()
            .enumerate()
            .filter(|(_, o)| !o.detail.is_empty() || o.failed.is_some())
            .take(3)
            .map(|(i, o)| format!("==== DLL {i}: {}\n{}", o.failed.as_deref().unwrap_or(""), o.detail))
            .collect();
        let wrote = std::fs::create_dir_all(&scratch).is_ok() && std::fs::write(scratch.join("lift-parity.txt"), detail).is_ok();
        panic!(
            "lifter parity: {} DLLs, {functions} functions compared, {differing} differing, {other} other pieces differing, \
             DLLs not run (index): {failed:?}; first differing (DLL index, function index): {first:?}; detail {}",
            dlls.len(),
            if wrote { "in target/scratch/lift-parity.txt" } else { "could not be written" }
        );
    }
    assert!(functions > 0, "no function compared");
}
