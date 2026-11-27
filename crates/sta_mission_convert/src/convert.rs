//! The mission converter (LUA-15h): `lift_mission`, `convert`, `mission_report`, `find_missions`,
//! `map_stems` and the engine lookup and loop of `main` of `tools/dsl_to_lua.py`, without the
//! command line (argument parsing, console output, writing files and the output-folder checks
//! belong to LUA-15f).
//!
//! [`convert`] is the library function: from a mission DLL's image and listing, the rule DLL's
//! (when there is one) and the engine names, the Lua text of one mission, or the failure the
//! Python reports for it. [`convert_install`] is `main`'s loop over the mission DLLs of the
//! install folders, in memory: which files it would write (one per map that runs the mission) and
//! which missions failed.
//!
//! # Errors
//!
//! Python's `main` catches `DecodeError` and `drl.Unsupported` per mission (the mission is
//! counted as failed and no file is written); every other exception aborts the run. Here a
//! [`Failure::Unsupported`] or a [`Failure::Error`] of kind [`ErrorKind::Decode`] is a failed
//! mission, any other [`DecodeError`] is an abort (an `Err` of [`convert_install`]).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use crate::call::{check_engine, mission_lifter, OBJECT_CLASSES};
use crate::disasm::Ins;
use crate::drl::{self, Decoded, Failure};
use crate::dsl::Dsl;
use crate::emit::{render_function, Namer};
use crate::format::lua_str;
use crate::lift::Lifted;
use crate::map::{self, Vtables};
use crate::msvc::plain_name;
use crate::prelude::{ENGINE, PRELUDE};
use crate::pyval::py_lower;
use crate::{tool_token, DecodeError, ErrorKind, Pe, Result};

/// `lift_mission(dsl)`: every function a mission reaches, from its exports (the address to the
/// lifted function, in the order they were lifted) and the role of each (`ScriptExtendInterface`,
/// `ScriptInit`, `action`, `step`, `helper`).
pub fn lift_mission(d: &Dsl<'_>) -> Result<(IndexMap<i64, Lifted>, HashMap<i64, String>)> {
    let mut lf = mission_lifter(d);
    let mut out: IndexMap<i64, Lifted> = IndexMap::new();
    let mut roots: Vec<(String, i64)> = Vec::new();
    for (n, a) in &d.exports {
        let a = i64::from(*a);
        if d.section_of(a).as_deref() != Some(".text") {
            continue;
        }
        roots.push((plain_name(n), d.entry(a)?));
    }
    let mut roles: HashMap<i64, String> = HashMap::new();
    for (name, a) in &roots {
        if name == "ScriptExtendInterface" || name == "ScriptInit" {
            roles.insert(*a, name.clone());
        } else {
            roles.insert(*a, "action".to_owned());
        }
    }
    roots.sort(); // `sorted(roots)`: tuples of (name, address)
    let mut todo: VecDeque<i64> = roots.iter().map(|(_, a)| *a).collect();
    while let Some(a) = todo.pop_front() {
        if out.contains_key(&a) {
            continue;
        }
        let f = lf.lift(a)?;
        let (steps, calls) = (f.steps.clone(), f.calls.clone());
        out.insert(a, f);
        for s in steps {
            roles.entry(s).or_insert_with(|| "step".to_owned());
            todo.push_back(s);
        }
        for c in calls {
            roles.entry(c).or_insert_with(|| "helper".to_owned());
            todo.push_back(c);
        }
    }
    Ok((out, roles))
}

/// The conversion report of one mission (`mission_report`; the JSON of the command line is not
/// reproduced, this is its content).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub mission: String,
    /// Role to (total, decoded).
    pub functions: BTreeMap<String, (usize, usize)>,
    /// `(function, role, error)` of every function that was not decoded, in lifting order.
    pub errors: Vec<(String, String, String)>,
    pub loops: usize,
    /// Engine call name to count, over the decoded functions, sorted by name.
    pub engine_calls: BTreeMap<String, i64>,
    pub rules: usize,
    pub rule_conditions_decoded: usize,
}

/// `mission_report(mission, funcs, roles, nm, rules)`.
pub fn mission_report(
    mission: &str,
    funcs: &IndexMap<i64, Lifted>,
    roles: &HashMap<i64, String>,
    nm: &Namer<'_, '_>,
    rules: Option<&Decoded>,
) -> Report {
    let mut r = Report { mission: mission.to_owned(), ..Report::default() };
    for (a, f) in funcs {
        let role = roles.get(a).map_or("helper", String::as_str);
        let e = r.functions.entry(role.to_owned()).or_insert((0, 0));
        e.0 += 1;
        if let Some(err) = &f.error {
            r.errors.push((nm.fn_name(*a), role.to_owned(), err.clone()));
        } else {
            e.1 += 1;
            for (k, v) in &f.engine_calls {
                *r.engine_calls.entry(k.clone()).or_insert(0) += v;
            }
        }
    }
    r.loops = funcs.values().filter(|f| f.loops != 0).count();
    if let Some(rules) = rules {
        r.rules = rules.rules.len();
        r.rule_conditions_decoded = rules.rules.iter().filter(|x| x.condition_lua.is_some()).count();
    }
    r
}

/// `INTERNAL.match(n)`: the `ScriptInterface` methods the converted script implements itself.
fn is_internal(n: &str) -> bool {
    n.starts_with("ScriptCreate_")
        || n.starts_with("ScriptGet_")
        || n.starts_with("ScriptDestroy_")
        || ["Add", "Remove"].iter().any(|a| {
            ["To", "From"].iter().any(|b| ["Load", "Save"].iter().any(|c| n == format!("{a}{b}{c}")))
        })
}

/// One converted mission.
#[derive(Debug, Clone)]
pub struct Converted {
    /// The mission name (the DLL's name without `S.dsl`).
    pub mission: String,
    /// The Lua text.
    pub source: String,
    pub report: Report,
}

/// The mission name of a DLL file name: `re.sub(r"[sS]\.dsl$", "", base)`.
pub fn mission_name(dsl_base: &str) -> String {
    match dsl_base.strip_suffix("s.dsl").or_else(|| dsl_base.strip_suffix("S.dsl")) {
        Some(m) => m.to_owned(),
        None => dsl_base.to_owned(),
    }
}

/// The text of the file `convert` writes, after the lifting: everything from the header line
/// to the engine block (`convert` of the Python after `lift_mission` and the rule decode).
/// `funcs` is changed the way the Python changes its `Lifted` objects: a function whose
/// rendering raised a `DecodeError` gets the error `render: <text>` and is written as a stub.
pub fn emit_mission(
    d: &Dsl<'_>,
    funcs: &mut IndexMap<i64, Lifted>,
    roles: &HashMap<i64, String>,
    rules: Option<&Decoded>,
    mission: &str,
    header: &str,
) -> Result<(String, Report)> {
    let nm = Namer::new(d, funcs, roles);
    let mut l: Vec<String> = Vec::new();
    let mut w = |s: String| l.push(s);
    w(header.to_owned());
    w("-- Generated on the user's machine from the user's install; not for redistribution.".to_owned());
    w("-- Engine calls go to A2 by the ScriptInterface method's name; unbound or failing".to_owned());
    w("-- calls are counted in Mission.missing / Mission.failed and answer nil.".to_owned());
    w(String::new());
    w("Mission_ = {".to_owned());
    w(format!("    name = {},", lua_str(mission)));
    w("    vars = {}, state = {}, objects = {}, globals = {}, timers = {}, scripts = {},".to_owned());
    w("    rules = {}, affected = {}, satisfied = {}, fired = {}, fireCount = {}, ran = {},".to_owned());
    w("    calls = {}, missing = {}, failed = {}, notDecoded = {}, stepErrors = {},".to_owned());
    w("    conditionErrors = {}, engineVars = {}, engineWritten = {}, engineReads = {}, stepCount = 0, callFailed = false,"
        .to_owned());
    w("    problems = {}, problemOrder = {}, blocked = {}, invalid = {}, unresolved = {},".to_owned());
    w("}".to_owned());
    w(PRELUDE.to_owned());
    // handles and arrays
    let mut data: Vec<(&i64, &crate::dsl::DataItem)> = d.data.iter().collect();
    data.sort_by_key(|(a, _)| **a);
    for (_, it) in &data {
        if matches!(it.kind, "intarray" | "floatarray" | "Vector3") {
            w(format!("O.{} = {{}}", it.name));
        } else if matches!(it.kind, "int" | "float") {
            // exported scalars are zero before ScriptInit runs (a rule can read a handle an
            // earlier script has not yet written)
            w(format!("O.{} = {}", it.name, if it.kind == "int" { "0" } else { "0.0" }));
        }
    }
    w(String::new());
    let mut statics: Vec<(i64, &'static str)> =
        d.static_refs.borrow().iter().filter(|(_, k)| matches!(**k, "word" | "str")).map(|(a, k)| (*a, *k)).collect();
    statics.sort_by_key(|(a, _)| *a);
    if !statics.is_empty() {
        w("-- Globals of the DLL that it does not export and that the code writes (or hands out the".to_owned());
        w("-- address of): slots of G, started from the DLL image's words; read-only ones are inlined.".to_owned());
        for (a, k) in &statics {
            let v = if *k == "str" {
                match d.cstr(*a) {
                    Ok(s) => lua_str(&s),
                    Err(e) if e.kind == ErrorKind::Decode => "''".to_owned(),
                    Err(e) => return Err(e),
                }
            } else {
                let x = d.dword(*a).unwrap_or(0);
                (if x >= 1 << 31 { x - (1 << 32) } else { x }).to_string()
            };
            w(format!("G[{}] = {v}", crate::format::py_hex(*a)));
        }
        w(String::new());
    }
    w("-- ---------------------------------------------------------------- functions".to_owned());
    const RANKED: [&str; 5] = ["ScriptExtendInterface", "ScriptInit", "action", "step", "helper"];
    let mut order: Vec<i64> = funcs.keys().copied().collect();
    let rank = |a: &i64| -> usize {
        let role = roles.get(a).map_or("helper", String::as_str);
        RANKED.iter().position(|r| *r == role).unwrap_or(9)
    };
    order.sort_by_key(|a| (rank(a), *a));
    for a in order {
        let name = nm.fn_name(a);
        let role = roles.get(&a).map_or("helper", String::as_str).to_owned();
        {
            let f = &funcs[&a];
            let tail = f.error.as_ref().map_or(String::new(), |e| format!("  (not decoded: {e})"));
            w(format!("-- {role}: {name}{tail}"));
        }
        let rendered = match render_function(&funcs[&a], &nm, &name) {
            Ok(lines) => lines,
            Err(e) if e.kind == ErrorKind::Decode => {
                funcs.get_mut(&a).expect("listed function").error = Some(format!("render: {}", e.message));
                render_function(&funcs[&a], &nm, &name)?
            }
            Err(e) => return Err(e),
        };
        l_extend(&mut w, rendered);
        w(String::new());
    }
    if let Some(rules) = rules {
        w("-- ---------------------------------------------------------------- rules".to_owned());
        w(format!("-- {} rules from {}, in creation order.", rules.rules.len(), rules.drl));
        for r in &rules.rules {
            w("Mission.rules[#Mission.rules + 1] = {".to_owned());
            w(format!("    name = {},", lua_str(&r.name)));
            w(format!("    action = {},", lua_str(&r.action)));
            let deps: Vec<String> = r.deps.iter().map(|x| lua_str(x)).collect();
            w(format!("    deps = {{ {} }},", deps.join(", ")));
            match &r.condition_lua {
                Some(c) => w(format!("    when = function(S, O) return {c} end,")),
                None => {
                    w(format!("    -- condition not decoded: {}", r.error.clone().unwrap_or_default()));
                    w("    when = function(S, O) return false end,".to_owned());
                }
            }
            w("}".to_owned());
        }
        w(String::new());
        // the rule conditions use drl_to_lua's guard name
    }
    let report = mission_report(mission, funcs, roles, &nm, rules);
    let mut names: Vec<String> = report.engine_calls.keys().cloned().collect();
    if let Some(rules) = rules {
        for c in &rules.calls {
            if !names.contains(c) {
                names.push(c.clone());
            }
        }
    }
    names.retain(|n| !is_internal(n));
    names.sort();
    w("-- Engine calls the converted code makes (bodies and rule conditions).".to_owned());
    let quoted: Vec<String> = names.iter().map(|n| lua_str(n)).collect();
    w(format!("Mission.engineNames = {{ {} }}", quoted.join(", ")));
    // Functions the converter could not decode (each is a stub that records it ran): listed so
    // nothing is dropped silently; `Mission.notDecoded` counts the ones that ran.
    let mut und: Vec<(String, String)> =
        funcs.iter().filter_map(|(a, f)| f.error.as_ref().map(|e| (nm.fn_name(*a), e.clone()))).collect();
    und.sort();
    let items: Vec<String> = und.iter().map(|(n, e)| format!("[{}] = {}", lua_str(n), lua_str(e))).collect();
    w(format!("Mission.undecoded = {{ {} }}", items.join(", ")));
    w(ENGINE.to_owned());
    let src = l.join("\n") + "\n";
    Ok((src, report))
}

fn l_extend(w: &mut impl FnMut(String), lines: Vec<String>) {
    for x in lines {
        w(x);
    }
}

/// Opens the rule DLL of a mission: its image and its listing, in that order.
pub type RuleSource<'a> = dyn FnMut() -> Result<(Pe, Vec<Ins>)> + 'a;

/// `convert(dsl_path, drl_path, engine)` over images and listings. `dsl_base` and `drl_base` are
/// the files' base names (the header names them).
pub fn convert(
    dsl: &Pe,
    dsl_listing: Vec<Ins>,
    dsl_base: &str,
    drl: Option<(&str, &mut RuleSource<'_>)>,
    engine: &Vtables,
) -> std::result::Result<Converted, Failure> {
    let exports = dsl.exports()?;
    let d = Dsl::new(dsl, engine.clone(), dsl_listing, exports)?;
    let (mut funcs, roles) = lift_mission(&d)?;
    // the rule DLL is opened (image, then listing) only now, as `drl.decode` does after the lift
    let (rules, drl_base) = match drl {
        Some((base, open)) => {
            let (pe, ins) = open()?;
            (Some(drl::decode(pe, ins, base, Some(engine))?), Some(base))
        }
        None => (None, None),
    };
    let mission = mission_name(dsl_base);
    let mut header = format!("-- {mission}: converted by {} from {dsl_base}", tool_token());
    if let Some(b) = drl_base {
        header.push_str(&format!(" and {b}"));
    }
    header.push('.');
    let (source, report) = emit_mission(&d, &mut funcs, &roles, rules.as_ref(), &mission, &header)?;
    Ok(Converted { mission, source, report })
}

// ------------------------------------------------------------------ the files of an install

/// `(directories, files)` of `dir` as `os.walk` sees them: a symbolic link to a directory is a
/// directory (listed, never walked), anything else is a file. An unreadable directory is empty.
fn list_dir(dir: &Path) -> (Vec<(String, PathBuf, bool)>, Vec<String>) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (dirs, files);
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        let is_link = e.file_type().is_ok_and(|t| t.is_symlink());
        if path.is_dir() {
            dirs.push((name, path, is_link));
        } else {
            files.push(name);
        }
    }
    (dirs, files)
}

fn walk_missions(dir: &Path, out: &mut Vec<(String, Option<String>)>) {
    let (dirs, files) = list_dir(dir);
    // `low = {f.lower(): f for f in files}`: a later file replaces an earlier one
    let mut low: HashMap<String, &String> = HashMap::new();
    for f in &files {
        low.insert(py_lower(f), f);
    }
    for f in &files {
        if py_lower(f).ends_with("s.dsl") {
            let stem: String = {
                let n = f.chars().count();
                f.chars().take(n - 5).collect()
            };
            let drlf = low.get(&py_lower(&format!("{stem}r.drl")));
            out.push((dir.join(f).to_string_lossy().into_owned(), drlf.map(|x| dir.join(x).to_string_lossy().into_owned())));
        }
    }
    for (_, p, is_link) in dirs {
        if !is_link {
            walk_missions(&p, out);
        }
    }
}

/// `find_missions(root)`: the `(S.dsl, R.drl or None)` pairs under `root`, sorted by path.
pub fn find_missions(root: &Path) -> Vec<(PathBuf, Option<PathBuf>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    walk_missions(root, &mut out);
    out.sort();
    out.into_iter().map(|(a, b)| (PathBuf::from(a), b.map(PathBuf::from))).collect()
}

/// The matches of `rb"([A-Za-z0-9_]+)[sS]\.[dD][sS][lL]"` in `data`: the group, in order.
fn dll_names(data: &[u8]) -> Vec<String> {
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut out = Vec::new();
    let mut p = 0usize;
    while p < data.len() {
        if !word(data[p]) {
            p += 1;
            continue;
        }
        let mut j = p;
        while j < data.len() && word(data[j]) {
            j += 1;
        }
        // the run [p, j) can only match as group + `[sS]` at its end, then `.dsl`
        let tail_ok = j + 4 <= data.len()
            && data[j] == b'.'
            && data[j + 1].eq_ignore_ascii_case(&b'd')
            && data[j + 2].eq_ignore_ascii_case(&b's')
            && data[j + 3].eq_ignore_ascii_case(&b'l');
        if tail_ok && j - p >= 2 && data[j - 1].eq_ignore_ascii_case(&b's') {
            out.push(data[p..j - 1].iter().map(|&b| char::from(b)).collect::<String>());
            p = j + 4;
        } else {
            p = j;
        }
    }
    out
}

/// `os.path.basename(p)`: the text after the last separator (so `""` for a path that ends in
/// one, and `.` for `x/.`), not `Path::file_name`, which skips those.
fn py_basename(p: &str) -> String {
    let sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    match p.rfind(sep) {
        Some(i) => p[i + 1..].to_owned(),
        None => p.to_owned(),
    }
}

fn walk_stems(dir: &Path, out: &mut IndexMap<String, Vec<String>>) {
    let (mut dirs, mut files) = list_dir(dir);
    dirs.sort_by(|a, b| a.0.cmp(&b.0)); // `dirs.sort()`
    let base = py_basename(&dir.to_string_lossy());
    if py_lower(&base) == "bzn" {
        dirs.clear();
        files.sort();
        for f in &files {
            if !py_lower(f).ends_with(".bzn") {
                continue;
            }
            let Ok(data) = std::fs::read(dir.join(f)) else { continue };
            let n = f.chars().count();
            let stem = py_lower(&f.chars().take(n - 4).collect::<String>());
            for m in dll_names(&data) {
                let stems = out.entry(py_lower(&m)).or_default();
                if !stems.contains(&stem) {
                    stems.push(stem.clone());
                }
            }
        }
    }
    for (_, p, is_link) in dirs {
        if !is_link {
            walk_stems(&p, out);
        }
    }
}

/// `map_stems(roots)`: `{mission (lower case): [BZN stems]}` from the maps of the installs: a map
/// names the mission DLL it runs (`<mission>S.dsl`) in its script section, and the mission
/// resolver looks a script up by the map's stem, which is not always the DLL's name.
pub fn map_stems(roots: &[PathBuf]) -> IndexMap<String, Vec<String>> {
    let mut out = IndexMap::new();
    for r in roots {
        walk_stems(r, &mut out);
    }
    out
}

/// The engine lookup of `main`: the vtables of the engine classes of the first root (the
/// `engine_root`, else each install in turn) whose `Armada2.exe` and `armada2.map` give names,
/// checked with both `check_engine`s. An unreadable or undecodable root is skipped like the
/// Python's `except (OSError, DecodeError)`; a mismatch of the slots the converter relies on, or
/// no names at all, is the Python's `sys.exit` and an `Err` of kind `Decode`.
pub fn load_engine(installs: &[PathBuf], engine_root: Option<&Path>) -> Result<Vtables> {
    let candidates: Vec<&Path> = match engine_root {
        Some(r) => vec![r],
        None => installs.iter().map(PathBuf::as_path).collect(),
    };
    let classes: Vec<&str> = drl::ENGINE_CLASSES.iter().chain(OBJECT_CLASSES.iter()).copied().collect();
    let mut engine: Option<Vtables> = None;
    for root in candidates {
        engine = match map::engine_vtables(root, "armada2.exe", "armada2.map", &classes) {
            Ok(v) => v,
            Err(e) if e.kind != ErrorKind::Raw => None,
            Err(e) => return Err(e),
        };
        if engine.as_ref().is_some_and(|v| !v.is_empty()) {
            break;
        }
    }
    let Some(engine) = engine.filter(|v| !v.is_empty()) else {
        return Err(DecodeError::new("no Armada2.exe + armada2.map found: pass --engine <Armada II root>"));
    };
    let mut bad = map::check_engine(&engine, &drl::ENGINE_WANT, true);
    bad.extend(check_engine(&engine));
    if !bad.is_empty() {
        return Err(DecodeError::new(format!(
            "engine slots differ from the ones this converter was read from:\n  {}",
            bad.join("\n  ")
        )));
    }
    Ok(engine)
}

/// One mission DLL `main` converts: its name, the files and the maps (stems) whose script it is.
#[derive(Debug, Clone)]
pub struct Job {
    /// The mission name (the DLL's name without `S.dsl`).
    pub mission: String,
    pub dsl: PathBuf,
    pub drl: Option<PathBuf>,
    /// The file stems to write: one per map that runs the mission, or the mission's own
    /// lower-case name.
    pub targets: Vec<String>,
}

/// The loop of `main` over the install folders `roots` (the mission folders), without converting:
/// the mission DLLs in the order the Python visits them (sorted by path, then per root) that
/// `only` (compared in lower case) selects, each with the stems it is written under.
pub fn plan(roots: &[PathBuf], only: &[String]) -> Vec<Job> {
    let stems = map_stems(roots);
    let only: Vec<String> = only.iter().map(|x| py_lower(x)).collect();
    let mut jobs = Vec::new();
    for root in roots {
        for (dsl, drl) in find_missions(root) {
            let dsl_base = dsl.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let mission = mission_name(&dsl_base);
            if !only.is_empty() && !only.contains(&py_lower(&mission)) {
                continue;
            }
            let targets = match stems.get(&py_lower(&mission)).filter(|s| !s.is_empty()) {
                Some(s) => s.clone(),
                None => vec![py_lower(&mission)],
            };
            jobs.push(Job { mission, dsl, drl, targets });
        }
    }
    jobs
}

/// What converting one mission gave: the file, or the failure `main` prints as `FAIL`.
#[derive(Debug, Clone)]
pub enum Outcome {
    Converted(Converted),
    /// `Unsupported` or a `DecodeError`: the mission is counted as failed, the run goes on.
    Failed(String),
}

/// Converts one planned mission. `listing` gives the instruction listing of a DLL (the Python runs
/// `objdump`); it is called after the file's image was read, as the Python's `Dsl` and `Drl`
/// constructors do. Any fault that is not a failed mission is the Python's abort (an `Err`).
pub fn convert_one(
    job: &Job,
    engine: &Vtables,
    listing: &mut dyn FnMut(&Path) -> Result<Vec<Ins>>,
) -> Result<Outcome> {
    convert_one_with(job, engine, &mut |p| {
        let pe = Pe::open(p)?;
        let ins = listing(p)?;
        Ok((pe, ins))
    })
}

/// Loads a DLL: its image and its listing, from the file at the path.
pub type LoadFn<'a> = dyn FnMut(&Path) -> Result<(Pe, Vec<Ins>)> + 'a;

/// [`convert_one`] with the loading of a DLL in the caller's hands: `load` gives the image and the
/// listing of the file at the path (so a caller that has the bytes already need not read the file
/// again). It is called for the mission DLL first and for the rule DLL later, as the Python opens
/// them.
pub fn convert_one_with(job: &Job, engine: &Vtables, load: &mut LoadFn<'_>) -> Result<Outcome> {
    let dsl_base = job.dsl.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let drl_base: Option<String> =
        job.drl.as_ref().map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let result = (|| -> std::result::Result<Converted, Failure> {
        let (pe, ins) = load(&job.dsl)?;
        let mut open = || -> Result<(Pe, Vec<Ins>)> {
            let p = job.drl.as_ref().expect("a rule DLL");
            load(p)
        };
        let drl = drl_base.as_ref().map(|base| (base.as_str(), &mut open as &mut RuleSource<'_>));
        convert(&pe, ins, &dsl_base, drl, engine)
    })();
    match result {
        Ok(c) => Ok(Outcome::Converted(c)),
        Err(Failure::Unsupported(m)) => Ok(Outcome::Failed(m)),
        Err(Failure::Error(e)) if e.kind == ErrorKind::Decode => Ok(Outcome::Failed(e.message)),
        Err(Failure::Error(e)) => Err(e),
    }
}

/// What [`convert_install`] found.
#[derive(Debug, Default)]
pub struct InstallRun {
    /// The files `main` writes, by name (`<map stem>.lua`): a later file of the same name replaces
    /// an earlier one, as the Python overwrites.
    pub files: BTreeMap<String, Vec<u8>>,
    /// The mission DLLs converted, in order, with their reports.
    pub converted: Vec<Report>,
    /// `(mission, error text)` of the missions that failed, in order.
    pub failed: Vec<(String, String)>,
}

/// The loop of `main` over the install folders `roots` (the mission folders): [`plan`], then
/// [`convert_one`] per mission, in memory.
pub fn convert_install(
    roots: &[PathBuf],
    engine: &Vtables,
    only: &[String],
    listing: &mut dyn FnMut(&Path) -> Result<Vec<Ins>>,
) -> Result<InstallRun> {
    let mut run = InstallRun::default();
    for job in plan(roots, only) {
        match convert_one(&job, engine, listing)? {
            Outcome::Converted(c) => {
                for stem in &job.targets {
                    run.files.insert(format!("{stem}.lua"), c.source.clone().into_bytes());
                }
                run.converted.push(c.report);
            }
            Outcome::Failed(m) => run.failed.push((job.mission, m)),
        }
    }
    Ok(run)
}
