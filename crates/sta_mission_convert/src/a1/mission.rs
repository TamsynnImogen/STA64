//! The Armada I mission driver (`classify`, `norm`, `lift_mission`, `CondNamer`, `lua_ident`,
//! `emit_function`, `convert`, `mission_report`, `find_missions` and the loop of `main` of
//! `tools/a1_to_lua.py`), without the command line.
//!
//! Rendering is LUA-15h's [`crate::emit`]: the Armada I tool calls `dsl.Namer`,
//! `dsl.render_function` and `dsl.ENGINE` and patches `dsl.render_expr`. Here those are
//! `emit::Namer` (with the `game` and `cond` fields, LUA-15e's edit), `emit::render_function` and
//! `prelude::ENGINE`; the patch is the hook at the top of `emit::render_expr`.
//!
//! # Errors
//!
//! `main` catches every exception per mission (the mission fails and no file is written), so any
//! [`DecodeError`] of [`convert_dlls`] is a failed mission. `emit_function` catches any exception of
//! the renderer and writes `render: {class}: {text}` into the Lua: the class is the fault's
//! [`DecodeError::class`], `DecodeError` for a decode fault, `OSError` for an io fault; a raw fault
//! of STA64's own (class `None`) has no Python counterpart and is written as `Exception`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use crate::disasm::Ins;
use crate::dsl::Dsl;
use crate::emit::{render_function, Namer};
use crate::format::{a1_lua_ident, lua_str, py_hex};
use crate::lift::Lifted;
use crate::msvc::{data_kind, plain_name, VAR_KINDS};
use crate::prelude::{a1_prelude, ENGINE, A1_PRELUDE};
use crate::pyval::py_lower;
use crate::{tool_token, DecodeError, ErrorKind, Pe, Result};

use super::dll::A1Dll;
use super::lift::Resolved;
use super::render::render_failed;

/// `classify(f)`: complete / partial (some call written as `UD`) / not decoded.
pub fn classify(f: &Lifted) -> &'static str {
    if f.error.as_deref().is_some_and(|e| !e.is_empty()) {
        "not"
    } else if !f.a1.undecoded.is_empty() {
        "partial"
    } else {
        "complete"
    }
}

/// `norm(why)`: a decode failure reason without addresses, for the leftover table.
pub fn norm(why: &str) -> String {
    // re.sub(r"0x[0-9a-f]+", "#", why)
    let b = why.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'0' && i + 2 < b.len() + 0 && b[i + 1] == b'x' && matches!(b[i + 2], b'0'..=b'9' | b'a'..=b'f') {
            let mut j = i + 2;
            while j < b.len() && matches!(b[j], b'0'..=b'9' | b'a'..=b'f') {
                j += 1;
            }
            out.push('#');
            i = j;
        } else {
            let ch = why[i..].chars().next().expect("a char");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    // re.sub(r"\(\d+ slots\)", "(n slots)", why)
    let b = out.as_bytes();
    let mut res = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'(' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && out[j..].starts_with(" slots)") {
                res.push_str("(n slots)");
                i = j + " slots)".len();
                continue;
            }
        }
        let ch = out[i..].chars().next().expect("a char");
        res.push(ch);
        i += ch.len_utf8();
    }
    res
}

/// One rule of the rule DLL: the dict `lift_mission` builds.
#[derive(Debug, Clone)]
pub struct RuleOut {
    pub name: String,
    /// The condition's entry address (after `R.entry`).
    pub cond: i64,
    pub act: i64,
    pub deps: Vec<String>,
    pub action: Option<String>,
    pub action_error: Option<String>,
}

/// What `lift_mission` returns, in the Python's order: `(S, R, funcs, roles, rules, conds, rerr,
/// registration_calls)` without the DLLs.
#[derive(Debug, Default)]
pub struct MissionLift {
    pub funcs: IndexMap<i64, Lifted>,
    pub roles: HashMap<i64, String>,
    pub rules: Vec<RuleOut>,
    pub conds: IndexMap<i64, Lifted>,
    pub rerr: Vec<String>,
    pub registration_calls: IndexMap<String, i64>,
}

/// `lift_mission(dsl_path, drl_path)` over built DLLs.
pub fn lift_mission(s: &A1Dll<'_>, r: Option<&A1Dll<'_>>) -> Result<MissionLift> {
    let mut out = MissionLift::default();
    // rule DLL: DllMain and what it calls register the rules
    if let Some(r) = r {
        let mut rl = r.lifter();
        let mut rfuncs: IndexMap<i64, Lifted> = IndexMap::new();
        match r.dllmain {
            None => out.rerr.push("no DllMain found".to_owned()),
            Some(dm) => {
                let mut todo: VecDeque<i64> = VecDeque::from([dm]);
                while let Some(x) = todo.pop_front() {
                    if rfuncs.contains_key(&x) {
                        continue;
                    }
                    let f = rl.lift(x)?;
                    if let Some(e) = &f.error {
                        out.rerr.push(format!("registration {}: {e}", py_hex(x - i64::from(r.dsl.pe.base()))));
                    }
                    todo.extend(f.calls.iter().copied());
                    rfuncs.insert(x, f);
                }
            }
        }
        for ru in rl.a1.rules.clone() {
            out.rules.push(RuleOut { name: ru.name, cond: ru.cond, act: ru.act, deps: ru.deps, action: None, action_error: None });
        }
        // conditions
        for ru in &mut out.rules {
            let c = r.entry(ru.cond)?;
            ru.cond = c;
            if !out.conds.contains_key(&c) {
                let f = rl.lift(c)?;
                out.conds.insert(c, f);
            }
            // the action: a link-table thunk to the mission DLL's exported action
            match rl.a1_resolve(ru.act)? {
                Resolved::Imp(x) => ru.action = Some(plain_name(&x)),
                Resolved::Fn(x) => {
                    let w = rl.lift(x)?;
                    let calls: Vec<&String> = w.a1.imports_called.iter().filter(|s| !s.is_empty()).collect();
                    let ok = w.error.as_deref().is_none_or(str::is_empty);
                    ru.action = if calls.len() == 1 && ok { Some(calls[0].clone()) } else { None };
                    if ru.action.is_none() {
                        ru.action_error = Some(match w.error.as_deref().filter(|e| !e.is_empty()) {
                            Some(e) => e.to_owned(),
                            None => "action wrapper does not call one action".to_owned(),
                        });
                    }
                }
            }
        }
        for (c, f) in &out.conds {
            if !rfuncs.contains_key(c) {
                // `rfuncs.update({c: f ...})`: only for the totals below
                let _ = f;
            }
        }
        let cond_addrs: HashSet<i64> = out.conds.keys().copied().collect();
        for f in rfuncs.values().chain(out.conds.iter().filter(|(c, _)| !rfuncs.contains_key(*c)).map(|(_, f)| f)) {
            if !cond_addrs.contains(&f.addr) {
                for (k, v) in &f.engine_calls {
                    *out.registration_calls.entry(k.clone()).or_insert(0) += v;
                }
            }
        }
    }
    // mission DLL: DllMain (start values, handles) and the actions with their steps
    let mut sl = s.lifter();
    let mut todo: VecDeque<i64> = VecDeque::new();
    if let Some(dm) = s.dllmain {
        out.roles.insert(dm, "init".to_owned());
        todo.push_back(dm);
    }
    for a in s.dsl.exports.values() {
        if s.dsl.section_of(i64::from(*a)).as_deref() == Some(".text") {
            let e = s.entry(i64::from(*a))?;
            out.roles.insert(e, "action".to_owned());
            todo.push_back(e);
        }
    }
    while let Some(a) = todo.pop_front() {
        if out.funcs.contains_key(&a) {
            continue;
        }
        let f = sl.lift(a)?;
        for st in &f.steps {
            out.roles.entry(*st).or_insert_with(|| "step".to_owned());
            todo.push_back(*st);
        }
        for c in &f.calls {
            out.roles.entry(*c).or_insert_with(|| "helper".to_owned());
            todo.push_back(*c);
        }
        out.funcs.insert(a, f);
    }
    Ok(out)
}

/// `emit_function(L, f, nm, name, role)`: the function's Lua, or the stub of a function that is
/// not decoded. A renderer failure of any kind makes the function a stub with
/// `render: {class}: {text}` as its reason (this sets `f.error`, which the report then counts).
pub fn emit_function(lines: &mut Vec<String>, f: &mut Lifted, nm: &Namer<'_, '_>, name: &str, role: &str) {
    let has_err = f.error.as_deref().is_some_and(|e| !e.is_empty());
    lines.push(format!(
        "-- {role}: {name}{}",
        if has_err { format!("  (not decoded: {})", f.error.as_deref().unwrap_or("")) } else { String::new() }
    ));
    if !has_err {
        match render_function(f, nm, name) {
            Ok(l) => {
                lines.extend(l);
                lines.push(String::new());
                return;
            }
            Err(e) => {
                let class = match (e.kind, e.class) {
                    (ErrorKind::Decode, _) => "DecodeError",
                    (ErrorKind::Io, _) => "OSError",
                    (_, Some(c)) => c,
                    (_, None) => "Exception",
                };
                f.error = Some(format!("render: {class}: {}", e.message));
            }
        }
    }
    let calls: BTreeSet<&String> = f.a1.imports_called.iter().filter(|c| !c.is_empty()).collect();
    let calls: Vec<String> = calls.into_iter().cloned().collect();
    lines.extend(render_failed(name, f.error.as_deref().unwrap_or(""), &calls));
    lines.push(String::new());
}

/// `mission_report(...)`: the counts of one converted mission.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub mission: String,
    pub map: Option<String>,
    pub rules: usize,
    pub rules_with_deps: usize,
    pub rule_actions_resolved: usize,
    pub registration_errors: Vec<String>,
    pub registration_calls: IndexMap<String, i64>,
    pub actions: usize,
    /// role to `[total, complete, partial, not]`, in order of first use.
    pub functions: IndexMap<String, [i64; 4]>,
    pub undecoded_calls: IndexMap<String, i64>,
    /// `dict(leftovers.most_common())`.
    pub leftovers: Vec<(String, i64)>,
    pub engine_calls: BTreeMap<String, i64>,
    pub init: bool,
}

impl Report {
    /// A text form with every value as Python's `repr` (for the intermediate parity test).
    pub fn canonical(&self) -> String {
        use crate::pyval::Py;
        let kv = |it: Vec<(String, Py)>| Py::List(it.into_iter().map(|(k, v)| Py::Tuple(vec![Py::from(k), v])).collect()).repr();
        let mut o = String::new();
        let mut line = |k: &str, v: String| {
            o.push_str(&format!("{k} {v}\n"));
        };
        line("mission", Py::from(self.mission.as_str()).repr());
        line("map", self.map.as_deref().map_or(Py::None, Py::from).repr());
        line("rules", self.rules.to_string());
        line("rules_with_deps", self.rules_with_deps.to_string());
        line("rule_actions_resolved", self.rule_actions_resolved.to_string());
        line("registration_errors", Py::List(self.registration_errors.iter().map(Py::from).collect()).repr());
        line("registration_calls", kv(self.registration_calls.iter().map(|(k, v)| (k.clone(), Py::Int(*v))).collect()));
        line("actions", self.actions.to_string());
        line(
            "functions",
            kv(self.functions.iter().map(|(k, v)| (k.clone(), Py::List(v.iter().map(|x| Py::Int(*x)).collect()))).collect()),
        );
        line("undecoded_calls", kv(self.undecoded_calls.iter().map(|(k, v)| (k.clone(), Py::Int(*v))).collect()));
        line("leftovers", kv(self.leftovers.iter().map(|(k, v)| (k.clone(), Py::Int(*v))).collect()));
        line("engine_calls", kv(self.engine_calls.iter().map(|(k, v)| (k.clone(), Py::Int(*v))).collect()));
        line("init", if self.init { "True" } else { "False" }.to_owned());
        o
    }
}

/// `mission_report(mission, map_name, funcs, roles, rules, conds, rerr, regcalls, S)`.
pub fn mission_report(mission: &str, map_name: Option<&str>, ml: &MissionLift, s: &A1Dll<'_>) -> Report {
    let mut by_role: IndexMap<String, [i64; 4]> = IndexMap::new();
    let mut leftovers: IndexMap<String, i64> = IndexMap::new();
    let mut undecoded: IndexMap<String, i64> = IndexMap::new();
    let mut engine: BTreeMap<String, i64> = BTreeMap::new();
    let all = ml
        .funcs
        .iter()
        .map(|(a, f)| (ml.roles.get(a).map_or("helper", String::as_str), f))
        .chain(ml.conds.values().map(|f| ("condition", f)));
    for (role, f) in all {
        let r = by_role.entry(role.to_owned()).or_insert([0; 4]);
        r[0] += 1;
        r[match classify(f) {
            "complete" => 1,
            "partial" => 2,
            _ => 3,
        }] += 1;
        if let Some(e) = f.error.as_deref().filter(|e| !e.is_empty()) {
            *leftovers.entry(norm(e)).or_insert(0) += 1;
        }
        for (name, why) in &f.a1.undecoded {
            *undecoded.entry(name.clone()).or_insert(0) += 1;
            *leftovers.entry(format!("call: {}", norm(why))).or_insert(0) += 1;
        }
        for (k, v) in &f.engine_calls {
            *engine.entry(k.clone()).or_insert(0) += v;
        }
    }
    let actions = ml.funcs.keys().filter(|a| ml.roles.get(*a).map(String::as_str) == Some("action")).count();
    let mut lo: Vec<(String, i64)> = leftovers.into_iter().collect();
    lo.sort_by_key(|x| std::cmp::Reverse(x.1)); // `most_common`: stable, largest first
    Report {
        mission: mission.to_owned(),
        map: map_name.map(str::to_owned),
        rules: ml.rules.len(),
        rules_with_deps: ml.rules.iter().filter(|r| !r.deps.is_empty()).count(),
        rule_actions_resolved: ml.rules.iter().filter(|r| r.action.as_deref().is_some_and(|a| !a.is_empty())).count(),
        registration_errors: ml.rerr.clone(),
        registration_calls: ml.registration_calls.clone(),
        actions,
        functions: by_role,
        undecoded_calls: undecoded,
        leftovers: lo,
        engine_calls: engine,
        init: s.dllmain.is_some_and(|d| d != 0),
    }
}

/// `re.sub(r"[sS]\.dsl$", "", os.path.basename(path))`.
fn mission_name(base: &str) -> String {
    for suf in ["s.dsl", "S.dsl"] {
        if let Some(m) = base.strip_suffix(suf) {
            return m.to_owned();
        }
    }
    base.to_owned()
}

/// `convert(dsl_path, drl_path, map_name)` over built DLLs: `(mission, Lua source, report)`.
pub fn convert_dlls(s: &A1Dll<'_>, r: Option<&A1Dll<'_>>, map_name: Option<&str>) -> Result<(String, String, Report)> {
    let mut ml = lift_mission(s, r)?;
    let base = |p: &str| Path::new(p).file_name().map_or_else(|| p.to_owned(), |n| n.to_string_lossy().into_owned());
    let s_base = base(&s.path);
    let mission = mission_name(&s_base);
    let mut nm = Namer::new(&s.dsl, &ml.funcs, &ml.roles);
    nm.game = crate::lift::Game::A1;
    let mut l: Vec<String> = Vec::new();
    let mut header = format!("-- {}: converted by {} from {s_base}", map_name.unwrap_or(&mission), tool_token());
    if let Some(r) = r {
        header.push_str(&format!(" and {}", base(&r.path)));
    }
    header.push('.');
    l.push(header);
    l.push("-- Generated on the user's machine from the user's install; not for redistribution.".to_owned());
    l.push("-- Engine calls go to A1 by the imported function's name; unbound, failing or".to_owned());
    l.push("-- undecoded calls are counted in Mission.missing / failed / undecoded and answer nil.".to_owned());
    l.push(String::new());
    l.push("Mission_ = {".to_owned());
    l.push(format!("    name = {}, game = 'a1',", lua_str(&mission)));
    l.push("    vars = {}, state = {}, objects = {}, globals = {}, timers = {}, scripts = {},".to_owned());
    l.push("    rules = {}, affected = {}, satisfied = {}, fired = {}, fireCount = {}, ran = {},".to_owned());
    l.push("    calls = {}, missing = {}, failed = {}, notDecoded = {}, stepErrors = {},".to_owned());
    l.push("    conditionErrors = {}, engineVars = {}, engineWritten = {}, engineReads = {}, stepCount = 0, callFailed = false,".to_owned());
    l.push("    problems = {}, problemOrder = {}, blocked = {}, invalid = {}, unresolved = {},".to_owned());
    l.push("    undecoded = {},".to_owned());
    l.push("}".to_owned());
    l.push(a1_prelude()?);
    l.push(A1_PRELUDE.to_owned());
    // variables: the mission DLL's exported objects; the engine's own by import
    let mut data: Vec<(&i64, &crate::dsl::DataItem)> = s.dsl.data.iter().collect();
    data.sort_by_key(|x| *x.0);
    for (_, it) in &data {
        if VAR_KINDS.contains(&it.kind) {
            l.push(format!(
                "BindVar({}, {}, NewVar({}, {}))",
                lua_str(&it.name),
                lua_str(it.kind),
                lua_str(it.kind),
                lua_str(&it.name)
            ));
        }
    }
    let mut engine_vars: BTreeSet<(String, String)> = BTreeSet::new();
    for d in std::iter::once(s).chain(r) {
        for (dll, sym) in d.dsl.pe.imports().values() {
            let k = data_kind(sym);
            if VAR_KINDS.contains(&k) && !py_lower(dll).ends_with(".dsl") {
                engine_vars.insert((plain_name(sym), k.to_owned()));
            }
        }
    }
    for (n, k) in &engine_vars {
        l.push(format!("BindVar({}, {}, EngineVar({}, {}))", lua_str(n), lua_str(k), lua_str(n), lua_str(k)));
    }
    for (_, it) in &data {
        if matches!(it.kind, "intarray" | "floatarray" | "Vector3") {
            l.push(format!("O.{} = {{}}", it.name));
        } else if matches!(it.kind, "int" | "float") {
            l.push(format!("O.{} = {}", it.name, if it.kind == "int" { "0" } else { "0.0" }));
        }
    }
    l.push(String::new());
    l.push("-- ---------------------------------------------------------------- mission DLL".to_owned());
    let rank = ["init", "action", "step", "helper"];
    let mut addrs: Vec<i64> = ml.funcs.keys().copied().collect();
    let key = |a: &i64| -> (usize, i64) {
        let role = ml.roles.get(a).map(String::as_str);
        (role.and_then(|r| rank.iter().position(|x| *x == r)).unwrap_or(9), *a)
    };
    addrs.sort_by_key(key);
    for a in &addrs {
        let role = ml.roles.get(a).cloned().unwrap_or_else(|| "helper".to_owned());
        let name = nm.fn_name(*a);
        let f = ml.funcs.get_mut(a).expect("lifted function");
        emit_function(&mut l, f, &nm, &name, &role);
    }
    if let Some(dm) = s.dllmain.filter(|d| *d != 0) {
        l.push(format!("function M.ScriptInit() M.{}(0, 1, 0) end", nm.fn_name(dm)));
        l.push(String::new());
    }
    // rule conditions
    let mut cnames: HashMap<i64, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    for ru in &ml.rules {
        if !cnames.contains_key(&ru.cond) {
            let base = format!("R_{}", a1_lua_ident(&ru.name));
            let (mut n, mut k) = (base.clone(), 1);
            while used.contains(&n) {
                k += 1;
                n = format!("{base}_{k}");
            }
            used.insert(n.clone());
            cnames.insert(ru.cond, n);
        }
    }
    l.push("-- ---------------------------------------------------------------- rule conditions".to_owned());
    if let Some(r) = r {
        let cn = Namer::cond_namer(&r.dsl, cnames.clone());
        let mut cs: Vec<i64> = ml.conds.keys().copied().collect();
        cs.sort_by(|a, b| cnames.get(a).map_or("", String::as_str).cmp(cnames.get(b).map_or("", String::as_str)));
        for c in cs {
            let name = cn.fn_name(c);
            let f = ml.conds.get_mut(&c).expect("condition");
            emit_function(&mut l, f, &cn, &name, "condition");
        }
    }
    l.push("-- ---------------------------------------------------------------- rules".to_owned());
    l.push(format!(
        "-- {} rules from {}, in creation order.",
        ml.rules.len(),
        r.map_or_else(|| "-".to_owned(), |r| base(&r.path))
    ));
    let mut seen: HashMap<&str, i64> = HashMap::new();
    for ru in &ml.rules {
        let c = seen.entry(ru.name.as_str()).or_insert(0);
        *c += 1;
        let name = if *c == 1 { ru.name.clone() } else { format!("{}#{}", ru.name, *c) };
        l.push("Mission.rules[#Mission.rules + 1] = {".to_owned());
        l.push(format!("    name = {},", lua_str(&name)));
        l.push(format!("    action = {},", lua_str(ru.action.as_deref().filter(|a| !a.is_empty()).unwrap_or("?"))));
        let deps: Vec<String> = ru.deps.iter().map(|x| lua_str(x)).collect();
        l.push(format!("    deps = {{ {} }},", deps.join(", ")));
        l.push(format!("    when = function(S, O) return T(M.{}()) end,", cnames[&ru.cond]));
        l.push("}".to_owned());
    }
    l.push(String::new());
    let report = mission_report(&mission, map_name, &ml, s);
    let names: Vec<String> = report.engine_calls.keys().map(|n| lua_str(n)).collect();
    l.push("-- Engine calls the converted code makes (bodies and rule conditions).".to_owned());
    l.push(format!("Mission.engineNames = {{ {} }}", names.join(", ")));
    l.push(ENGINE.to_owned());
    Ok((mission, l.join("\n") + "\n", report))
}

// ------------------------------------------------------------------ install

/// The matches of `rb"([A-Za-z0-9_]+[rR]\.drl)"` in `data`, lower-cased.
fn drl_names(data: &[u8]) -> Vec<String> {
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
        if j + 4 <= data.len() && &data[j..j + 4] == b".drl" && j - p >= 2 && data[j - 1].eq_ignore_ascii_case(&b'r') {
            let s: String = data[p..j + 4].iter().map(|&b| char::from(b)).collect();
            out.push(py_lower(&s));
            p = j + 4;
        } else {
            p = j;
        }
    }
    out
}

fn sorted_names(dir: &Path) -> Result<Vec<String>> {
    let rd = std::fs::read_dir(dir).map_err(|e| DecodeError::io(format!("{}: {e}", dir.display())))?;
    let mut v: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    Ok(v)
}

/// `os.path.realpath`.
fn realpath(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// `find_missions(root, include_unmapped)`: `(map stem, S.dsl, R.drl)` for every map whose `.bzn`
/// names an Armada I rule DLL present in `missions/`; with `include_unmapped`, the pairs no `.bzn`
/// names follow, keyed by their lower-case DLL stem. Names that differ only in case go in
/// code-point order (the last `missions` folder, the first `bzn`, the last file win).
pub fn find_missions(root: &Path, include_unmapped: bool) -> Result<Vec<(String, PathBuf, PathBuf)>> {
    let top = sorted_names(root)?;
    let mut mdir: Option<PathBuf> = None;
    for d in &top {
        if py_lower(d) == "missions" && root.join(d).is_dir() {
            mdir = Some(root.join(d));
        }
    }
    let bdir = top.iter().find(|d| py_lower(d) == "bzn").map(|d| root.join(d));
    let Some(mdir) = mdir else { return Ok(Vec::new()) };
    let mut files: HashMap<String, String> = HashMap::new();
    for f in sorted_names(&mdir)? {
        files.insert(py_lower(&f), f);
    }
    let mut pairs: BTreeMap<String, (PathBuf, PathBuf)> = BTreeMap::new();
    for f in files.keys() {
        if let Some(stem) = f.strip_suffix("r.drl")
            && let Some(sf) = files.get(&format!("{stem}s.dsl"))
        {
            pairs.insert(f.clone(), (mdir.join(sf), mdir.join(&files[f])));
        }
    }
    let mut out: Vec<(String, PathBuf, PathBuf)> = Vec::new();
    if let Some(bdir) = bdir.filter(|b| b.is_dir()) {
        for b in sorted_names(&bdir)? {
            if !py_lower(&b).ends_with(".bzn") {
                continue;
            }
            let Ok(data) = std::fs::read(bdir.join(&b)) else { continue };
            for key in drl_names(&data) {
                if let Some((d, r)) = pairs.get(&key) {
                    let n = b.chars().count();
                    out.push((py_lower(&b.chars().take(n - 4).collect::<String>()), d.clone(), r.clone()));
                    break;
                }
            }
        }
    }
    if include_unmapped {
        let named: HashSet<PathBuf> = out.iter().map(|o| realpath(&o.2)).collect();
        for (key, (d, r)) in &pairs {
            let stem = key[..key.len() - 5].to_owned();
            if !named.contains(&realpath(r)) && !out.iter().any(|o| o.0 == stem) {
                out.push((stem, d.clone(), r.clone()));
            }
        }
    }
    Ok(out)
}

/// What [`convert_install`] found.
#[derive(Debug, Default)]
pub struct InstallRun {
    /// `missions/<map>.lua` to its bytes.
    pub files: BTreeMap<String, Vec<u8>>,
    pub reports: Vec<Report>,
    /// `(map, error text)` of the missions that failed (no file is written for them).
    pub failed: Vec<(String, String)>,
}

/// The loop of `main`: every mission `find_missions` lists (`only` selects maps, lower-cased), one
/// file per mission; any failure of one mission is recorded and the others go on. `listing` gives
/// the instruction listing of a DLL (the Python runs `objdump`).
pub fn convert_install(
    root: &Path,
    include_unmapped: bool,
    only: &[String],
    listing: &mut dyn FnMut(&Path) -> Result<Vec<Ins>>,
) -> Result<InstallRun> {
    let mut run = InstallRun::default();
    let only: Vec<String> = only.iter().map(|x| py_lower(x)).collect();
    for (map, dsl_path, drl_path) in find_missions(root, include_unmapped)? {
        if !only.is_empty() && !only.contains(&map) {
            continue;
        }
        let mut one = || -> Result<(String, String, Report)> {
            let spe = Pe::open(&dsl_path)?;
            let sins = listing(&dsl_path)?;
            let sex = spe.exports()?;
            let sname = dsl_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let s = A1Dll::new(&sname, &spe, sins, sex, None)?;
            let rpe = Pe::open(&drl_path)?;
            let rins = listing(&drl_path)?;
            let rex = rpe.exports()?;
            let rname = drl_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let r = A1Dll::new(&rname, &rpe, rins, rex, None)?;
            convert_dlls(&s, Some(&r), Some(&map))
        };
        match one() {
            Ok((_, src, rep)) => {
                run.files.insert(format!("missions/{map}.lua"), src.into_bytes());
                run.reports.push(rep);
            }
            Err(e) => run.failed.push((map, e.message)),
        }
    }
    Ok(run)
}

/// Keeps the `Dsl` import used for the documentation links.
#[allow(dead_code)]
type _D<'a> = Dsl<'a>;
