//! The text forms the Rust tests print for a `Dsl` and its functions, equal to what the Python
//! drivers (`lift_common.py`) print: shared by `tests/dsl_lift.rs` (authored cases) and
//! `tests/dsl_lift_parity.rs` (installed DLLs). Contains no game data.

#![allow(dead_code)]

use std::collections::HashSet;

use sta_mission_convert::dsl::Dsl;
use sta_mission_convert::pyval::{py_int, py_isspace, Py};
use sta_mission_convert::{tup, Result};

pub fn repr_opt_str(s: Option<&str>) -> String {
    match s {
        Some(s) => Py::from(s).repr(),
        None => "None".to_owned(),
    }
}

pub fn repr_bool(b: bool) -> &'static str {
    if b {
        "True"
    } else {
        "False"
    }
}

/// `lift_common.kind_facts`.
pub fn kind_facts(d: &Dsl<'_>, a: i64) -> Result<String> {
    Ok(format!(
        "{} {} {} {}",
        repr_opt_str(d.helper_kind(a)?),
        repr_opt_str(d.vector_kind(a)?),
        repr_bool(d.is_chkstk(a)),
        repr_bool(d.is_ftol(a)?)
    ))
}

/// `lift_common.dsl_facts`: four lines.
pub fn dsl_facts(d: &Dsl<'_>) -> Vec<String> {
    let mut th: Vec<(i64, i64)> = d.thunks.iter().map(|(a, t)| (*a, *t)).collect();
    th.sort_unstable();
    let th = Py::List(th.into_iter().map(|(a, t)| tup![a, t]).collect());
    let chk = d.chkesp.map_or(Py::None, Py::Int);
    let data = Py::List(
        d.data
            .iter()
            .map(|(a, i)| tup![*a, i.name.as_str(), i.kind, i.size, i.mangled.as_str()])
            .collect(),
    );
    let cn = Py::List(d.code_names.iter().map(|(a, n)| tup![*a, n.as_str()]).collect());
    vec![
        format!("thunks {}", th.repr()),
        format!("chkesp {}", chk.repr()),
        format!("data {}", data.repr()),
        format!("code_names {}", cn.repr()),
    ]
}

/// `lift_common.static_refs_line`.
pub fn static_refs_line(d: &Dsl<'_>) -> String {
    let items: Vec<Py> = d.static_refs.borrow().iter().map(|(a, k)| tup![*a, *k]).collect();
    format!("static_refs {}", Py::List(items).repr())
}

/// `re.fullmatch(r"call\s+0x([0-9a-f]+)", t)`: the target.
fn call_target(t: &str) -> Option<i64> {
    let r = t.strip_prefix("call")?;
    let r = r.trim_start_matches(py_isspace);
    if r.len() == t.len() - 4 {
        return None; // no whitespace after `call`
    }
    let h = r.strip_prefix("0x")?;
    if h.is_empty() || !h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    py_int(h, 16).ok()
}

/// `lift_common.function_list`: every exported code entry (thunks followed) in export order,
/// then every direct call target of the listing not yet listed, in listing order.
pub fn function_list(d: &Dsl<'_>) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    let mut seen: HashSet<i64> = HashSet::new();
    for a in d.exports.values() {
        if d.section_of(i64::from(*a)).as_deref() == Some(".text") {
            let e = d.entry(i64::from(*a))?;
            if seen.insert(e) {
                out.push(e);
            }
        }
    }
    for (_, t) in &d.ins {
        if let Some(tgt) = call_target(t) {
            let tgt = d.thunks.get(&tgt).copied().unwrap_or(tgt);
            if d.at.contains_key(&tgt) && seen.insert(tgt) {
                out.push(tgt);
            }
        }
    }
    Ok(out)
}
