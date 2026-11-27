//! The call handling of the mission lifter (LUA-15h): `Lifter.call` of `tools/dsl_to_lua.py` and
//! the methods it calls (`set_lv`, `vector_call`, `known_call`, `typed`, `by_pointer`, `special`),
//! ported line for line onto the lifter core ([`crate::lift`]).
//!
//! [`call`] is the function the lifter's `call` instruction hands to; [`mission_lifter`] builds a
//! [`Lifter`] that uses it (a plain [`Lifter::new`] keeps the stand-in [`crate::lift::call_model`]
//! that LUA-15d's parity test pins).
//!
//! The engine constants below are the ones the Python file carries (slot numbers and class names
//! of the engine's script interface, read from the executable, static; checked against the user's
//! executable by [`check_engine`]).
//!
//! Errors follow the core: a fault the Python raises as `DecodeError` is a [`DecodeError::new`]
//! (it stops the function), a Python exception of its own is a [`DecodeError::python`].

use crate::algebra::lv_value;
use crate::dsl::{pat, Dsl};
use crate::format::py_hex;
use crate::lift::{float_of_bits, reads_location, Flow, Lifter, State};
use crate::map::{ExpectedSlot, Vtables};
use crate::msvc::{msvc_params, msvc_ret, plain_name, push_count};
use crate::pyval::{py_int, Py};
use crate::{tup, DecodeError, ErrorKind, Result};

/// `CHARACTER_NEW_SCRIPT`: `character::new_script(char *)` returns the new script.
pub const CHARACTER_NEW_SCRIPT: i64 = 1;
/// `SCRIPT_ADD_STEP`: `script::add_step(int (*)(script *))`.
pub const SCRIPT_ADD_STEP: i64 = 0;
/// `IIIE_TRASH_RULE`: `IIIE::trash_rule(char *)` / `(rule *)`.
pub const IIIE_TRASH_RULE: [i64; 2] = [2, 3];
/// `OBJECT_CLASSES`: the object classes whose methods the lifter names.
pub const OBJECT_CLASSES: [&str; 3] = ["SiegeFleet", "MoveFleet", "AttackFleet"];
/// `OUT_INT_PARAMS`: `ScriptInterface` methods whose last `int &` parameter is an out parameter.
pub const OUT_INT_PARAMS: [&str; 1] = ["RecruitSpecialForces"];

/// The `want` table of `check_engine` of `dsl_to_lua.py`.
pub const ENGINE_WANT: [ExpectedSlot<'static>; 6] = [
    ExpectedSlot { class: "character", slot: CHARACTER_NEW_SCRIPT as usize, prefix: "?new_script@character@@" },
    ExpectedSlot { class: "timer_int", slot: 9, prefix: "?start@timer_int@@" },
    ExpectedSlot { class: "timer_int", slot: 10, prefix: "?stop@timer_int@@" },
    ExpectedSlot { class: "hot_int", slot: 7, prefix: "??4hot_int@@" },
    ExpectedSlot { class: "hot_float", slot: 7, prefix: "??4hot_float@@" },
    ExpectedSlot { class: "IIIE", slot: 2, prefix: "?trash_rule@IIIE@@" },
];

/// `check_engine(vt)` of `dsl_to_lua.py`: the slots this converter relies on, against the user's
/// executable (one message per slot that differs; no `found` tail).
pub fn check_engine(vt: &Vtables) -> Vec<String> {
    crate::map::check_engine(vt, &ENGINE_WANT, false)
}

fn dec<T>(msg: String) -> Result<T> {
    Err(DecodeError::new(msg))
}

fn py_err<T>(class: &'static str, msg: impl Into<String>) -> Result<T> {
    Err(DecodeError::python(class, msg))
}

fn index_err<T>() -> Result<T> {
    py_err("IndexError", "list index out of range")
}

/// `x[0]` as a string, `""` for a head that is not one.
fn tg(v: &Py) -> Result<&str> {
    Ok(v.tag()?.unwrap_or(""))
}

/// `f"{x and x[0]}"`: `x` itself when it is false (`None`), else its head.
fn and_head(v: &Py) -> Result<String> {
    if v.truthy() {
        Ok(v.item(0)?.to_str())
    } else {
        Ok(v.to_str())
    }
}

/// `x and x[0] == name`-style guard: `x` is a non-empty tuple whose head is `name`.
fn truthy_tag(v: &Py, name: &str) -> Result<bool> {
    Ok(v.truthy() && v.tag_is(name)?)
}

fn int_of(v: &Py, op: &str) -> Result<i64> {
    match v.as_int() {
        Some(n) => Ok(n),
        None => py_err("TypeError", format!("unsupported operand type(s) for {op}: '{}' and 'int'", v.type_name())),
    }
}

/// `next(x for x in d.data.values() if x[0] == name)[1]`: the kind of the exported datum named
/// `name` (a raw `StopIteration` when there is none).
fn data_kind_of(d: &Dsl<'_>, name: &Py) -> Result<&'static str> {
    d.data
        .values()
        .find(|v| Py::Str(v.name.clone()) == *name)
        .map(|v| v.kind)
        .ok_or_else(|| DecodeError::python("StopIteration", ""))
}

/// `names[slot]` of a list for a Python integer index that may be negative: `None` when
/// `slot >= len(names)` is the caller's test, so the index is in range from above here.
fn name_at(names: &[String], slot: i64) -> Result<&str> {
    let len = names.len() as i64;
    let j = if slot < 0 { slot + len } else { slot };
    if j < 0 || j >= len {
        return index_err();
    }
    Ok(names[j as usize].as_str())
}

/// `names[slot] if slot < len(names) else None` (`Dsl.si`, `Dsl.var_slot`): Python list indexing,
/// so a negative slot counts from the end and one below the start is an `IndexError`.
fn name_or_none(names: &[String], slot: i64) -> Result<Option<&str>> {
    if slot < names.len() as i64 { name_at(names, slot).map(Some) } else { Ok(None) }
}

/// `re.fullmatch(r"PAV(\w+)@@", ret or "")`: the class name.
fn pav_class(ret: Option<&str>) -> Option<&str> {
    let inner = ret?.strip_prefix("PAV")?.strip_suffix("@@")?;
    (!inner.is_empty() && inner.chars().all(|c| c.is_alphanumeric() || c == '_')).then_some(inner)
}

/// A [`Lifter`] that runs the full call handling of the Python (not the stand-in model).
pub fn mission_lifter<'d, 'a>(d: &'d Dsl<'a>) -> Lifter<'d, 'a> {
    let mut lf = Lifter::new(d);
    lf.call_fn = call;
    lf
}

/// `ops` as `re.fullmatch(r"0x([0-9a-f]+)", ops)`: the digits.
fn direct_target(ops: &str) -> Option<&str> {
    ops.strip_prefix("0x").filter(|h| !h.is_empty() && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// `Lifter.call`: the effect of a `call` instruction.
pub fn call(lf: &mut Lifter<'_, '_>, st: &mut State, stmts: &mut Vec<Py>, a: i64, ops: &str, i: usize) -> Result<Flow> {
    let d = lf.d;
    if let Some(h) = direct_target(ops) {
        let tgt = py_int(h, 16)?;
        if Some(tgt) == d.chkesp {
            return Ok(Flow::Next);
        }
        let body = d.entry(tgt)?;
        if d.is_chkstk(body) {
            st.body = false; // `mov eax,N; call _chkstk`: a large frame's prologue
            return Ok(Flow::Next);
        }
        if let Some(kind) = d.helper_kind(body)? {
            return known_call(lf, st, stmts, a, kind, i);
        }
        if let Some(vk) = d.vector_kind(body)? {
            return vector_call(lf, st, stmts, a, vk);
        }
        let ftol = d.is_ftol(body)?;
        if !ftol {
            // an internal function may write any global, variable or handed-out slot
            let esc = st.escaped.clone();
            lf.snap(
                st,
                stmts,
                &move |e| {
                    Ok(e.tag_in(&["obj", "elem", "vfield", "glob", "var", "locidx"])?
                        || (e.tag_is("loc")? && esc.contains(e)))
                },
                Py::None,
            )?;
        }
        if ftol {
            let v = lf.fpop(st, a)?;
            st.r.insert("eax".to_owned(), tup!["ftol", v]);
            st.r.insert("edx".to_owned(), Py::None);
            return Ok(Flow::Next);
        }
        let nxt = d.ins.get(i + 1).map_or("", |x| x.1.as_str());
        let args: Vec<Py> = if let Some(h) = pat::add_esp(nxt) {
            let n = py_int(h, 16)? / 4;
            let have = st.args.len() as i64;
            if n > have {
                return dec(format!("internal call with {n} arguments, {have} pushed at {}", py_hex(a)));
            }
            // the following `add esp` drops them
            st.args[(have - n) as usize..].iter().rev().cloned().collect()
        } else {
            Vec::new()
        };
        if !lf.cur.calls.contains(&body) {
            lf.cur.calls.push(body);
        }
        let t = lf.new_tmp();
        let mut checked = Vec::with_capacity(args.len());
        for x in &args {
            checked.push(lf.arg(x, a)?);
        }
        stmts.push(tup!["set", &t, tup!["fcall", body, Py::List(checked)]]);
        lf.clobber(st);
        st.r.insert("eax".to_owned(), t.clone());
        st.fret = t;
        return Ok(Flow::Next);
    }
    let tgt = lf.read(st, ops, a)?;
    // an engine call may write through a slot address the function handed out
    let esc = st.escaped.clone();
    lf.snap(st, stmts, &move |e| Ok(e.tag_is("loc")? && esc.contains(e)), Py::None)?;
    if !tgt.truthy() || !tgt.tag_is("method")? {
        return dec(format!("indirect call `{ops}` ({}) at {}", and_head(&tgt)?, py_hex(a)));
    }
    let obj = tgt.item(1)?.clone();
    let slot = int_of(tgt.item(2)?, "<")?;
    let okind = tg(&obj)?.to_owned();
    if okind == "si" {
        return si_call(lf, st, stmts, a, slot);
    }
    if okind == "varptr" {
        return var_call(lf, st, stmts, a, &obj, slot);
    }
    if okind == "world" {
        if slot == CHARACTER_NEW_SCRIPT {
            let nm = lf.take(st, 1, a)?.remove(0);
            let t = lf.new_tmp();
            stmts.push(tup!["set", &t, tup!["newscript", lf.arg(&nm, a)?]]);
            lf.clobber(st);
            st.r.insert("eax".to_owned(), tup!["script", &t]);
            return Ok(Flow::Next);
        }
        if slot == crate::drl::CHARACTER_GET_IIIE {
            lf.clobber(st);
            st.r.insert("eax".to_owned(), tup!["iiie"]);
            return Ok(Flow::Next);
        }
    }
    if okind == "objref" {
        return objref_call(lf, st, stmts, a, &obj, slot);
    }
    if okind == "iiie" && IIIE_TRASH_RULE.contains(&slot) {
        let nm = lf.take(st, 1, a)?.remove(0);
        stmts.push(tup!["trashrule", lf.arg(&nm, a)?]);
        lf.clobber(st);
        return Ok(Flow::Next);
    }
    if okind == "script" && slot == SCRIPT_ADD_STEP {
        let func = lf.take(st, 1, a)?.remove(0);
        if !truthy_tag(&func, "code")? {
            return dec(format!("add_step with {} at {}", func.to_str(), py_hex(a)));
        }
        let body = d.entry(int_of(func.item(1)?, "-")?)?;
        lf.cur.steps.push(body);
        stmts.push(tup!["addstep", obj.item(1)?, body]);
        lf.clobber(st);
        return Ok(Flow::Next);
    }
    dec(format!("call on {okind} slot {slot} at {}", py_hex(a)))
}

/// The `obj[0] == "si"` branch of `Lifter.call`: a `ScriptInterface` method.
fn si_call(lf: &mut Lifter<'_, '_>, st: &mut State, stmts: &mut Vec<Py>, a: i64, slot: i64) -> Result<Flow> {
    let d = lf.d;
    let Some(name) = name_or_none(d.engine.get("ScriptInterfaceImp").map_or(&[], |v| v.as_slice()), slot)? else {
        return dec(format!("ScriptInterface slot {slot} unknown at {}", py_hex(a)));
    };
    let plain = plain_name(name);
    let Some(params) = msvc_params(name) else {
        return dec(format!("cannot read the signature of {plain} at {}", py_hex(a)));
    };
    let ret = msvc_ret(name)?;
    let hidden = i64::from(ret.as_deref().is_some_and(|r| r.starts_with("?A")));
    let n: i64 = params.iter().map(|p| push_count(p)).sum::<i64>() + hidden;
    let mut raw = lf.take(st, n, a)?;
    let mut slot_out: Option<Py> = None;
    if hidden != 0 {
        let first = raw.first().cloned().ok_or_else(|| DecodeError::python("IndexError", "list index out of range"))?;
        raw.remove(0);
        let ok = first.truthy() && first.tag_is("addr")? && first.item(1)?.tag_is("loc")?;
        if !ok {
            return dec(format!("{plain}: struct return into {} at {}", first.to_str(), py_hex(a)));
        }
        slot_out = Some(first);
    }
    let mut outlv: Option<Py> = None;
    if OUT_INT_PARAMS.contains(&plain.as_str())
        && params.last().is_some_and(|p| p == "AH")
        && raw.last().ok_or_else(|| DecodeError::python("IndexError", "list index out of range"))?.truthy()
        && raw.last().is_some_and(|x| matches!(x.tag(), Ok(Some("addr"))))
    {
        // `int &out`: the engine writes the handle through it
        let lv = raw.last().expect("checked").item(1)?.clone();
        raw.pop();
        let loaded = lf.load(st, &lv, a)?; // the binding echoes it back when nothing was recruited
        raw.push(loaded);
        outlv = Some(lv);
    }
    let args = typed(lf, &raw, &params, &plain, a, Some(&mut *st))?;
    *lf.cur.engine_calls.entry(plain.clone()).or_insert(0) += 1;
    let call_e = tup!["call", &plain, Py::List(args.clone())];
    if let Some(outlv) = outlv {
        let call_e = tup!["callout", &plain, Py::List(args)];
        let t = lf.new_tmp();
        stmts.push(tup!["set", &t, call_e]);
        let test = reads_location(&outlv)?;
        lf.snap(st, stmts, &*test, Py::None)?;
        stmts.push(tup!["set", &outlv, tup!["outval", &outlv]]);
        st.mem.shift_remove(&outlv);
        lf.clobber(st);
        st.r.insert("eax".to_owned(), t);
        return Ok(Flow::Next);
    }
    let special = special(&plain, &args, ret.as_deref())?;
    if ret.as_deref() == Some("X") {
        stmts.push(special.unwrap_or_else(|| tup!["eval", call_e]));
        lf.clobber(st);
        return Ok(Flow::Next);
    }
    let t = lf.new_tmp();
    stmts.push(tup!["set", &t, special.unwrap_or(call_e)]);
    lf.clobber(st);
    if let Some(slot_out) = slot_out {
        // the returned vector is copied into the caller's slot
        let off = int_of(slot_out.item(1)?.item(1)?, "+")?;
        let pairs: Vec<Py> = (0..3).map(|k| tup![tup!["loc", off + 4 * k], tup!["field", &t, k]]).collect();
        let lvs: Vec<Py> = pairs.iter().map(|p| p.item(0).cloned()).collect::<Result<_>>()?;
        stmts.push(Py::Tuple(vec![Py::from("multiset"), Py::List(pairs)]));
        for lv in lvs {
            st.mem.shift_remove(&lv);
        }
        st.r.insert("eax".to_owned(), slot_out);
        return Ok(Flow::Next);
    }
    let cls = pav_class(ret.as_deref());
    let v = match cls {
        Some(c) if OBJECT_CLASSES.contains(&c) => tup!["objref", &t, c],
        _ => t.clone(),
    };
    st.r.insert("eax".to_owned(), v);
    st.fret = if ret.as_deref() == Some("M") { t } else { Py::None };
    Ok(Flow::Next)
}

/// The `obj[0] == "varptr"` branch: a rule variable's method.
fn var_call(lf: &mut Lifter<'_, '_>, st: &mut State, stmts: &mut Vec<Py>, a: i64, obj: &Py, slot: i64) -> Result<Flow> {
    let d = lf.d;
    let name = obj.item(1)?.clone();
    let kind = obj.item(2)?.clone();
    // `Dsl.var_slot`: the operation by slot, and the name lookup (which can raise)
    let names: &[String] = d.engine.get(&kind.to_str()).map_or(&[], |v| v.as_slice());
    let op = crate::dsl::var_op(slot);
    name_or_none(names, slot)?;
    let Some(op) = op else {
        return dec(format!("{} slot {slot} at {}", kind.to_str(), py_hex(a)));
    };
    let nargs = i64::from(matches!(op, "set" | "start" | "update"));
    let mut args = lf.take(st, nargs, a)?;
    let float_arg = |v: &Py| -> Result<Option<Py>> {
        if truthy_tag(v, "const")? { Ok(Some(tup!["float", float_of_bits(v.item(1)?)?])) } else { Ok(None) }
    };
    if matches!(op, "start" | "update") {
        let first = args.first().ok_or_else(|| DecodeError::python("IndexError", "list index out of range"))?;
        if let Some(f) = float_arg(first)? {
            args[0] = f;
        }
    }
    if kind.as_str() == Some("hot_float") && op == "set" {
        let first = args.first().ok_or_else(|| DecodeError::python("IndexError", "list index out of range"))?;
        if let Some(f) = float_arg(first)? {
            args[0] = f;
        }
    }
    if op == "get" {
        let v = tup!["var", &name, &kind];
        lf.clobber(st);
        st.r.insert("eax".to_owned(), v.clone());
        st.fret = if kind.as_str() == Some("hot_float") { v } else { Py::None };
        return Ok(Flow::Next);
    }
    if op == "time" || op == "is_dirty" {
        let t = lf.new_tmp();
        stmts.push(tup!["set", &t, tup!["varq", &name, op]]);
        lf.clobber(st);
        st.r.insert("eax".to_owned(), t.clone());
        st.fret = t;
        return Ok(Flow::Next);
    }
    {
        let nm = name.clone();
        lf.snap(st, stmts, &move |e| Ok(e.tag_is("var")? && *e.item(1)? == nm), Py::None)?;
    }
    let mut checked = Vec::with_capacity(args.len());
    for x in &args {
        checked.push(lf.arg(x, a)?);
    }
    stmts.push(tup!["var", &name, op, Py::List(checked)]);
    lf.clobber(st);
    Ok(Flow::Next)
}

/// The `obj[0] == "objref"` branch: a method of an object class (`SiegeFleet`, ...).
fn objref_call(lf: &mut Lifter<'_, '_>, st: &mut State, stmts: &mut Vec<Py>, a: i64, obj: &Py, slot: i64) -> Result<Flow> {
    let d = lf.d;
    let cls = obj.item(2)?.to_str();
    let names: &[String] = d.engine.get(&cls).map_or(&[], |v| v.as_slice());
    if slot >= names.len() as i64 {
        return dec(format!("{cls} slot {slot} unknown at {}", py_hex(a)));
    }
    let name = name_at(names, slot)?;
    let plain = plain_name(name);
    let Some(params) = msvc_params(name) else {
        return dec(format!("cannot read the signature of {cls}::{plain} at {}", py_hex(a)));
    };
    let n: i64 = params.iter().map(|p| push_count(p)).sum();
    let raw = lf.take(st, n, a)?;
    let args = typed(lf, &raw, &params, &format!("{cls}::{plain}"), a, Some(&mut *st))?;
    let key = format!("{cls}.{plain}");
    *lf.cur.engine_calls.entry(key).or_insert(0) += 1;
    let call_e = tup!["mcall", obj.item(1)?, &cls, &plain, Py::List(args)];
    let ret = msvc_ret(name)?;
    lf.clobber(st);
    if ret.as_deref() == Some("X") {
        stmts.push(tup!["eval", call_e]);
        return Ok(Flow::Next);
    }
    let t = lf.new_tmp();
    stmts.push(tup!["set", &t, call_e]);
    st.r.insert("eax".to_owned(), t.clone());
    st.fret = if ret.as_deref() == Some("M") { t } else { Py::None };
    Ok(Flow::Next)
}

/// `Lifter.set_lv`: a store the lifter makes itself (not decoded from a `mov`).
pub fn set_lv(lf: &mut Lifter<'_, '_>, st: &mut State, stmts: &mut Vec<Py>, lv: &Py, v: Py, a: i64) -> Result<()> {
    let k = tg(lv)?.to_owned();
    if !matches!(k.as_str(), "loc" | "gvar" | "locidx") {
        return dec(format!("store to {k} at {}", py_hex(a)));
    }
    if k == "locidx" {
        let doomed: Vec<Py> = st.mem.keys().filter(|x| matches!(x.tag(), Ok(Some("loc")))).cloned().collect();
        for x in doomed {
            st.mem.shift_remove(&x);
        }
    } else {
        let keep = v.tag_in(&["const", "float", "str", "tmp"])?;
        let val = if keep { v.clone() } else { lv_value(lv)? };
        st.mem.insert(lv.clone(), val);
    }
    let test = reads_location(lv)?;
    let v = lf.snap(st, stmts, &*test, v)?;
    stmts.push(tup!["set", lv, v]);
    Ok(())
}

/// `Lifter.vector_call`: a member of a `std::vector<T>` (see `Dsl::vector_kind`) on a local or a
/// static one. Iterators and element references are byte offsets from `begin()`; reading through
/// one needs the vector, which the lifter follows from the call that produced it.
pub fn vector_call(lf: &mut Lifter<'_, '_>, st: &mut State, stmts: &mut Vec<Py>, a: i64, op: &str) -> Result<Flow> {
    let this = st.rget("ecx");
    if !this.truthy() || !this.tag_is("addr")? || !this.item(1)?.tag_in(&["loc", "gvar"])? {
        return dec(format!("std::vector member on {} at {}", and_head(&this)?, py_hex(a)));
    }
    let key = this.item(1)?.clone();
    if key.tag_is("gvar")? {
        lf.d.note_static(int_of(key.item(1)?, "-")?, "word");
    }
    if op == "ctor" {
        lf.cur.vectors.insert(key.clone());
    } else if !lf.cur.vectors.contains(&key) {
        return dec(format!("std::vector member {op} on an object never constructed here at {}", py_hex(a)));
    }
    let mut args: Vec<Py> = Vec::new();
    if matches!(op, "ctor" | "index" | "push_back" | "erase") {
        let mut x = lf.take(st, 1, a)?.remove(0);
        if op == "push_back" {
            if !truthy_tag(&x, "addr")? {
                return dec(format!("push_back of {} at {}", and_head(&x)?, py_hex(a)));
            }
            let lv = x.item(1)?.clone();
            x = lf.load(st, &lv, a)?; // `const T &`: the element is read now
        }
        if op == "ctor" {
            x = Py::None; // the allocator argument
        }
        if !x.is_none() {
            args.push(lf.arg(&x, a)?);
        }
    }
    if matches!(op, "ctor" | "push_back" | "clear" | "dtor") {
        stmts.push(tup!["eval", tup!["vop", op, &key, Py::List(args)]]);
        lf.clobber(st);
        st.r.insert("eax".to_owned(), if op == "ctor" { this } else { Py::None });
        return Ok(Flow::Next);
    }
    let t = lf.new_tmp();
    stmts.push(tup!["set", &t, tup!["vop", op, &key, Py::List(args)]]);
    lf.clobber(st);
    st.r.insert("eax".to_owned(), t.clone());
    if matches!(op, "begin" | "end" | "index" | "erase") {
        lf.cur.itype.insert(t, key);
    }
    Ok(Flow::Next)
}

/// `re.search(r"%[-+ #0-9.]*[fgeEG]", s)`.
fn has_float_conversion(s: &str) -> bool {
    let b = s.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c != b'%' {
            continue;
        }
        let mut j = i + 1;
        while j < b.len() && matches!(b[j], b'-' | b'+' | b' ' | b'#' | b'0'..=b'9' | b'.') {
            j += 1;
        }
        if j < b.len() && matches!(b[j], b'f' | b'g' | b'e' | b'E' | b'G') {
            return true;
        }
    }
    false
}

/// `Lifter.known_call`: a call of a C runtime / compiler routine `Dsl::helper_kind` recognised.
pub fn known_call(lf: &mut Lifter<'_, '_>, st: &mut State, stmts: &mut Vec<Py>, a: i64, kind: &str, i: usize) -> Result<Flow> {
    let d = lf.d;
    if kind == "vec3" {
        // Vector3::Vector3(float x, float y, float z), `this` in ecx, the callee pops 3 words
        let this = st.rget("ecx");
        if !this.truthy() || !this.tag_is("addr")? {
            return dec(format!("Vector3 constructor on {} at {}", and_head(&this)?, py_hex(a)));
        }
        let lv = this.item(1)?.clone();
        let mut vals: Vec<Py> = Vec::new();
        for v in lf.take(st, 3, a)? {
            let mut v = lf.arg(&v, a)?;
            if v.tag_is("const")? {
                v = tup!["float", float_of_bits(v.item(1)?)?];
            }
            vals.push(v);
        }
        lf.clobber(st); // (the constructor writes only the three words below)
        let lk = tg(&lv)?.to_owned();
        for (k, v) in vals.into_iter().enumerate() {
            let k = k as i64;
            if lk == "loc" {
                set_lv(lf, st, stmts, &tup!["loc", int_of(lv.item(1)?, "+")? + 4 * k], v, a)?;
            } else if lk == "obj" && data_kind_of(d, lv.item(1)?)? == "Vector3" {
                stmts.push(tup!["set", tup!["vfield", lv.item(1)?, k], v]);
            } else {
                return dec(format!("Vector3 constructor on {lk} at {}", py_hex(a)));
            }
        }
        st.r.insert("eax".to_owned(), this);
        return Ok(Flow::Next);
    }
    let nxt = d.ins.get(i + 1).map_or("", |x| x.1.as_str());
    let Some(h) = pat::add_esp(nxt) else {
        return dec(format!("call of a {kind} routine without `add esp` at {}", py_hex(a)));
    };
    let n = py_int(h, 16)? / 4;
    let have = st.args.len() as i64;
    if n > have {
        return dec(format!("{kind} call with {n} arguments, {have} pushed at {}", py_hex(a)));
    }
    let raw: Vec<Py> = st.args[(have - n) as usize..].iter().rev().cloned().collect();
    if kind == "report" {
        // the debug reporter: no debugger is attached, it answers 0 (ignore)
        lf.clobber(st);
        st.r.insert("eax".to_owned(), tup!["const", 0]);
        return Ok(Flow::Next);
    }
    if kind == "atexit" {
        // `static T x;` registers its destructor to run when the DLL unloads: the mission script
        // has no unload, nothing to do (the answer is 0: registered)
        lf.clobber(st);
        st.r.insert("eax".to_owned(), tup!["const", 0]);
        return Ok(Flow::Next);
    }
    let mut args: Vec<Py> = Vec::with_capacity(raw.len());
    for (k, x) in raw.iter().enumerate() {
        if kind == "strcmp" || k >= 1 {
            // `string_arg`
            let v = lf.arg(x, a)?;
            let by_ptr = truthy_tag(&v, "addr")? && v.item(1)?.tag_in(&["loc", "gvar"])?;
            args.push(if by_ptr { by_pointer(lf, st, "PD", v.item(1)?, kind, a)? } else { v });
        } else {
            args.push(lf.arg(x, a)?);
        }
    }
    if kind == "strcmp" {
        if args.len() != 2 {
            return dec(format!("strcmp with {} arguments at {}", args.len(), py_hex(a)));
        }
        let t = lf.new_tmp();
        stmts.push(tup!["set", &t, tup!["strcmp", &args[0], &args[1]]]);
        lf.clobber(st);
        st.r.insert("eax".to_owned(), t);
        return Ok(Flow::Next);
    }
    // sprintf(buffer, format, ...)
    let buf = if raw.is_empty() { Py::None } else { lf.arg(&raw[0], a)? };
    if args.len() < 2 || !truthy_tag(&buf, "addr")? {
        return dec(format!("sprintf into {} at {}", and_head(&buf)?, py_hex(a)));
    }
    let lv = buf.item(1)?.clone();
    if lv.tag_is("gvar")? {
        d.set_static(int_of(lv.item(1)?, "-")?, "str");
    }
    let fmt = args[1].clone();
    if fmt.tag_is("str")? {
        let text = fmt.item(1)?.as_str().unwrap_or("");
        if has_float_conversion(text) {
            return dec(format!("sprintf with a floating point conversion at {}", py_hex(a)));
        }
    }
    let t = lf.new_tmp();
    stmts.push(tup!["set", &t, tup!["sprintf", &fmt, Py::List(args[2..].to_vec())]]);
    set_lv(lf, st, stmts, &lv, t.clone(), a)?;
    lf.clobber(st);
    let n2 = lf.new_tmp();
    stmts.push(tup!["set", &n2, tup!["slen", &t]]);
    st.r.insert("eax".to_owned(), n2);
    Ok(Flow::Next)
}

/// `Lifter.typed`: call arguments by the method's parameter types: a pushed constant in a `float`
/// is the float's bits; a `Vector3` by value is three slots.
pub fn typed(
    lf: &mut Lifter<'_, '_>,
    raw: &[Py],
    params: &[String],
    what: &str,
    a: i64,
    mut st: Option<&mut State>,
) -> Result<Vec<Py>> {
    let mut args: Vec<Py> = Vec::new();
    let mut k = 0usize;
    for p in params {
        let c = push_count(p);
        if c == 1 {
            let mut v = raw.get(k).cloned().ok_or_else(|| DecodeError::python("IndexError", "list index out of range"))?;
            if let Some(st) = st.as_deref_mut()
                && truthy_tag(&v, "addr")?
                && matches!(p.as_str(), "AVVector3@@" | "AH" | "PD")
            {
                v = by_pointer(lf, st, p, v.item(1)?, what, a)?;
            }
            if p == "M" && truthy_tag(&v, "const")? {
                v = tup!["float", float_of_bits(v.item(1)?)?];
            }
            let v = lf.arg(&v, a)?;
            // a `bool` parameter is a Lua boolean for the engine call (the stack word is an int;
            // the A2 bindings take strict booleans)
            args.push(if p == "_N" { tup!["boolarg", v] } else { v });
        } else if p.starts_with("VVector3") {
            let mut comps = Vec::new();
            for x in raw.iter().skip(k).take(3) {
                comps.push(lf.arg(x, a)?);
            }
            args.push(tup!["vec", Py::List(comps)]);
        } else {
            return dec(format!("{what}: parameter type {p} at {}", py_hex(a)));
        }
        k += c as usize;
    }
    Ok(args)
}

/// `Lifter.by_pointer`: an argument the compiler passed by address (`Vector3 &`, `int &`,
/// `char *`) as the value the callee reads: a stack temporary (or a global) the code filled just
/// before the call becomes a Vector, a handle or a string in the script. Never a Ref table.
pub fn by_pointer(lf: &mut Lifter<'_, '_>, st: &mut State, p: &str, lv: &Py, what: &str, a: i64) -> Result<Py> {
    let d = lf.d;
    let k = tg(lv)?.to_owned();
    if p == "AVVector3@@" {
        if k == "loc" {
            let base = int_of(lv.item(1)?, "+")?;
            let mut comps: Vec<Py> = Vec::new();
            for j in 0..3 {
                let slot = tup!["loc", base + 4 * j];
                let mut v = st.mem.get(&slot).cloned().unwrap_or(Py::None);
                if v.is_none() || v == slot {
                    v = tup!["fbits", &slot]; // filled in an earlier block: the slot holds float bits or a float
                } else if v.tag_is("const")? {
                    v = tup!["float", float_of_bits(v.item(1)?)?];
                }
                comps.push(v);
            }
            return Ok(tup!["vec", Py::List(comps)]);
        }
        if k == "obj" && data_kind_of(d, lv.item(1)?)? == "Vector3" {
            return Ok(tup!["vec", Py::List((0..3).map(|j| tup!["vfield", lv.item(1).cloned().unwrap_or(Py::None), j]).collect())]);
        }
        if k == "vfield" && *lv.item(2)? == Py::Int(0) {
            return Ok(tup!["vec", Py::List((0..3).map(|j| tup!["vfield", lv.item(1).cloned().unwrap_or(Py::None), j]).collect())]);
        }
        return dec(format!("{what}: Vector3 passed by address of {k} at {}", py_hex(a)));
    }
    if p == "AH" {
        let mut lv = lv.clone();
        if k == "obj" && data_kind_of(d, lv.item(1)?)? == "intarray" {
            lv = tup!["elem", lv.item(1)?, tup!["const", 0]]; // the address of an array is its first element
        }
        return lf.load(st, &lv, a);
    }
    // a `char *`: a buffer the code formatted, or a string of the image
    if k == "gvar" {
        if let Some(v) = st.mem.get(lv) {
            return Ok(v.clone());
        }
        let addr = int_of(lv.item(1)?, "-")?;
        if matches!(d.static_class(addr), "rodata" | "const") {
            match d.cstr(addr) {
                Ok(s) => return Ok(tup!["str", s]),
                Err(e) if e.kind == ErrorKind::Decode => {}
                Err(e) => return Err(e),
            }
        }
        d.set_static(addr, "str");
        return Ok(tup!["glob", addr]);
    }
    lf.load(st, lv, a)
}

/// `Lifter.special`: engine calls the converted script implements itself.
pub fn special(plain: &str, args: &[Py], ret: Option<&str>) -> Result<Option<Py>> {
    if let Some(rest) = plain.strip_prefix("ScriptCreate_") {
        let first = args.first().ok_or_else(|| DecodeError::python("IndexError", "list index out of range"))?;
        return Ok(Some(tup!["newvar", rest, first]));
    }
    if let Some(rest) = plain.strip_prefix("ScriptGet_") {
        let kind = pav_class(ret).unwrap_or("hot_int");
        return Ok(Some(tup!["enginevar", rest, kind]));
    }
    if matches!(plain, "AddToLoad" | "AddToSave" | "RemoveFromLoad" | "RemoveFromSave") {
        return Ok(Some(tup!["comment", format!("{plain}: native save/load callback, not used")]));
    }
    if plain.starts_with("ScriptDestroy_") {
        return Ok(Some(tup!["comment", format!("{plain}: native cleanup, not used")]));
    }
    Ok(None)
}
