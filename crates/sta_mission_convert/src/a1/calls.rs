//! The calls of the Armada I lifter: arguments read from the modelled stack by the import's
//! decorated signature (`call_name`, `ret_slots`, `call`, `internal`, `tail_import`, `arg_at`,
//! `engine`, `undecoded`, `this_var`, `special`, `typed`, `ref_lv`, `lv_plus`, `deref_value`,
//! `arg` of `tools/a1_to_lua.py`).
//!
//! These are the Armada I tool's own: it overrides `call` and never reaches the Armada II call
//! handling (`known_call`, `vector_call`, `by_pointer`, LUA-15h's), so this module depends on
//! nothing of that row.

use crate::dsl::pat::M;
use crate::format::py_hex;
use crate::lift::{dec, float_of_bits, index_err, int_of, py_err, tg, Flow, Lifter, State};
use crate::msvc::{data_kind, plain_name, push_count, signature, slots_of, VAR_KINDS};
use crate::pyval::{py_floordiv, py_int, Py};
use crate::tup;
use crate::{ErrorKind, Result};

use super::lift::Resolved;
use super::{and0, is_head, var_method, VAR_CLASSES, WORLD_IIIE};

/// `call_name(sym)`: the name a call is counted under: the function's own name, or
/// `class.method` for a method.
pub fn call_name(sym: &str) -> String {
    if let Some(s) = signature(sym)
        && let Some(cls) = s.cls.as_deref().filter(|c| !c.is_empty())
    {
        return format!("{cls}.{}", s.method);
    }
    plain_name(sym)
}

/// `str(e).split(" at 0x")[0]`: an error text without the address the lifter appended.
fn before_at(msg: &str) -> &str {
    msg.split(" at 0x").next().unwrap_or("")
}

/// `a, b = raw` for a list of known length: `ValueError` with CPython's text otherwise.
fn unpack(raw: &[Py], n: usize) -> Result<&[Py]> {
    if raw.len() < n {
        return py_err("ValueError", format!("not enough values to unpack (expected {n}, got {})", raw.len()));
    }
    if raw.len() > n {
        return py_err("ValueError", format!("too many values to unpack (expected {n}, got {})", raw.len()));
    }
    Ok(raw)
}

/// `re.fullmatch(r"ret\s+0x([0-9a-f]+)", t)`: the digits.
fn ret_n(t: &str) -> Option<&str> {
    let mut m = M::new(t);
    if !(m.lit("ret") && m.ws1() && m.lit("0x")) {
        return None;
    }
    let h = m.hex1()?;
    m.end_exact().then_some(h)
}

/// `re.fullmatch(r"add\s+esp,(0x[0-9a-f]+)", t)`: the number with its prefix.
fn add_esp(t: &str) -> Option<&str> {
    let mut m = M::new(t);
    if !(m.lit("add") && m.ws1() && m.lit("esp,")) {
        return None;
    }
    let h = m.hex0x()?;
    m.end_exact().then_some(h)
}

impl<'d, 'a> Lifter<'d, 'a> {
    /// `A1Lifter.ret_slots`: arguments a stdcall function pops (`ret N`), else 0.
    pub fn a1_ret_slots(&self, body: i64) -> i64 {
        let d = self.d;
        let mut i = d.index_of(body);
        while let Some(k) = i {
            if k >= d.ins.len() {
                break;
            }
            let t = d.text_at(k);
            if t.starts_with("ret") {
                return match ret_n(t) {
                    Some(h) => i64::from_str_radix(h, 16).map_or(0, |n| n / 4),
                    None => 0,
                };
            }
            i = Some(k + 1);
        }
        0
    }

    /// `A1Lifter.call`: a call instruction: a direct call of an internal function or of an
    /// import stub, or a call through a register that holds an import's address.
    pub fn a1_call(&mut self, st: &mut State, stmts: &mut Vec<Py>, a: i64, ops: &str, i: usize) -> Result<Flow> {
        if let Some(h) = super::ops::hex_target(ops) {
            return match self.a1_resolve(py_int(h, 16)?)? {
                Resolved::Fn(x) => self.a1_internal(st, stmts, a, x, i),
                Resolved::Imp(x) => self.a1_engine(st, stmts, a, &x, -4 * st.a1.depth),
            };
        }
        let v = self.read(st, ops, a)?;
        if v.truthy() && v.tag_is("impaddr")? {
            let sym = v.item(1)?.to_str();
            if signature(&sym).is_some() {
                return self.a1_engine(st, stmts, a, &sym, -4 * st.a1.depth);
            }
        }
        dec(format!("indirect call `{ops}` ({}) at {}", and0(&v)?, py_hex(a)))
    }

    /// `A1Lifter.internal`: a call of a function of the DLL.
    pub fn a1_internal(&mut self, st: &mut State, stmts: &mut Vec<Py>, a: i64, body: i64, i: usize) -> Result<Flow> {
        let d = self.d;
        if d.is_ftol(body)? {
            let v = self.fpop(st, a)?;
            st.r.insert("eax".to_owned(), tup!["ftol", v]);
            st.r.insert("edx".to_owned(), Py::None);
            return Ok(Flow::Next);
        }
        let mut n = self.a1_ret_slots(body);
        let pops = n;
        if n == 0 {
            let nxt = if i + 1 < d.ins.len() { d.text_at(i + 1) } else { "" };
            n = match add_esp(nxt) {
                Some(h) => py_int(h, 16)? / 4,
                None => 0,
            };
        }
        if n > st.a1.depth {
            return dec(format!("internal call with {n} arguments, {} pushed at {}", st.a1.depth, py_hex(a)));
        }
        self.snap(
            st,
            stmts,
            &|e| e.tag_in(&["obj", "elem", "vfield", "glob", "var", "loc"]),
            Py::None,
        )?;
        let mut args = Vec::new();
        for k in 0..n {
            let v = self.a1_slot_read(st, st.a1.depth - k);
            args.push(self.a1_arg(&v, a)?);
        }
        if pops != 0 {
            self.a1_drop(st, pops, a)?;
        }
        if !self.cur.calls.contains(&body) {
            self.cur.calls.push(body);
        }
        let t = self.new_tmp();
        stmts.push(tup!["set", &t, tup!["fcall", body, Py::List(args)]]);
        self.clobber(st);
        st.r.insert("eax".to_owned(), t.clone());
        st.fret = t;
        Ok(Flow::Next)
    }

    /// `A1Lifter.tail_import`: a jump to an import is a call that returns its result.
    pub fn a1_tail_import(&mut self, st: &mut State, stmts: &mut Vec<Py>, a: i64, sym: &str) -> Result<Flow> {
        if st.a1.depth != 0 {
            return dec(format!("tail call with {} slots on the stack at {}", st.a1.depth, py_hex(a)));
        }
        self.a1_engine(st, stmts, a, sym, 4)?;
        let v = st.rget("eax");
        stmts.push(tup!["return", v]);
        Ok(Flow::Ret)
    }

    /// `A1Lifter.arg_at`: the value of the argument at stack offset `off`.
    pub fn a1_arg_at(&self, st: &mut State, off: i64, a: i64) -> Result<Py> {
        if off >= 4 {
            let lv = tup!["param", py_floordiv(off - 4, 4)?];
            return Ok(st.mem.get(&lv).cloned().unwrap_or(lv));
        }
        let j = py_floordiv(-off, 4)?;
        if j < 1 || j > st.a1.depth {
            return dec(format!("argument outside the stack at {}", py_hex(a)));
        }
        Ok(self.a1_slot_read(st, j))
    }

    /// `A1Lifter.engine`: a call to an imported function: arguments start at stack offset `base`
    /// (the top of the stack, or the caller's own arguments for a tail call).
    pub fn a1_engine(&mut self, st: &mut State, stmts: &mut Vec<Py>, a: i64, sym: &str, base: i64) -> Result<Flow> {
        let sig = signature(sym);
        let plain = call_name(sym);
        let Some(sig) = sig else {
            return dec(format!("cannot read the signature of {sym}"));
        };
        let (conv, cls, name, ret, params) = (sig.convention, sig.cls, sig.method, sig.ret, sig.params);
        let n = slots_of(&params, &ret);
        let mut raw: Vec<Py> = Vec::new();
        for k in 0..n {
            raw.push(self.a1_arg_at(st, base + 4 * k, a)?);
        }
        // `this = st.r.get("ecx", "unset") if conv == "thiscall" else None`; "unset" reads `ecx`
        let this = match (conv == "thiscall").then(|| st.r.get("ecx").cloned()) {
            Some(Some(t)) => t,
            Some(None) => self.read(st, "ecx", a)?,
            None => Py::None,
        };
        if matches!(conv, "thiscall" | "stdcall") && base < 4 {
            self.a1_drop(st, n, a)?;
        }
        self.snap(st, stmts, &|e| e.tag_is("loc"), Py::None)?;
        if self.a1_special(st, stmts, a, cls.as_deref(), &name, &ret, &params, &raw, &this)? {
            return Ok(Flow::Next);
        }
        if let Some(cls) = &cls {
            let why = format!("method of {cls} on {}", and0(&this)?);
            return self.a1_undecoded(st, stmts, &plain, &ret, &why);
        }
        let mut hidden: Option<Py> = None;
        if ret.starts_with("?A") {
            let h = raw.first().cloned().unwrap_or(Py::None);
            raw.remove(0);
            if !(h.truthy() && h.tag_is("addr")? && h.item(1)?.tag_is("loc")?) {
                let why = format!("struct result into {}", and0(&h)?);
                return self.a1_undecoded(st, stmts, &plain, "X", &why);
            }
            hidden = Some(h);
        }
        let args = match self.a1_typed(st, &raw, &params, &plain, a) {
            Ok(v) => v,
            Err(e) if e.kind == ErrorKind::Decode => {
                return self.a1_undecoded(st, stmts, &plain, &ret, before_at(&e.message));
            }
            Err(e) => return Err(e),
        };
        *self.cur.engine_calls.entry(plain.clone()).or_insert(0) += 1;
        let call = tup!["call", plain.as_str(), Py::List(args)];
        if ret == "X" {
            stmts.push(tup!["eval", call]);
            self.clobber(st);
            return Ok(Flow::Next);
        }
        let t = self.new_tmp();
        stmts.push(tup!["set", &t, call]);
        self.clobber(st);
        if let Some(h) = hidden {
            let j0 = py_floordiv(-int_of(h.item(1)?.item(1)?, "-")?, 4)?;
            for k in 0..3 {
                self.a1_slot_write(st, stmts, j0 - k, tup!["field", &t, k])?;
            }
            st.r.insert("eax".to_owned(), h);
            return Ok(Flow::Next);
        }
        st.r.insert("eax".to_owned(), t.clone());
        st.fret = if ret == "M" { t } else { Py::None };
        Ok(Flow::Next)
    }

    /// `A1Lifter.undecoded`: one call whose arguments cannot be read, written as `UD(name, why)`.
    pub fn a1_undecoded(&mut self, st: &mut State, stmts: &mut Vec<Py>, name: &str, ret: &str, why: &str) -> Result<Flow> {
        self.cur.a1.undecoded.push((name.to_owned(), why.to_owned()));
        let e = tup!["undecodedcall", name, why];
        if ret == "X" {
            stmts.push(tup!["eval", e]);
            self.clobber(st);
            return Ok(Flow::Next);
        }
        let t = self.new_tmp();
        stmts.push(tup!["set", &t, e]);
        self.clobber(st);
        st.r.insert("eax".to_owned(), t.clone());
        st.fret = if ret == "M" { t } else { Py::None };
        Ok(Flow::Next)
    }

    /// `next((v for v in self.d.data.values() if v[0] == name), None)`: the exported datum named
    /// `name`.
    fn a1_data_named(&self, name: &Py) -> Option<&'d crate::dsl::DataItem> {
        self.d.data.values().find(|v| Py::Str(v.name.clone()) == *name)
    }

    /// `A1Lifter.this_var`: the variable a method's `this` names: an exported object of the
    /// mission DLL or one imported by name. `(name, kind)`.
    pub fn a1_this_var(&self, this: &Py) -> Result<Option<(Py, &'static str)>> {
        if !this.truthy() {
            return Ok(None);
        }
        if this.tag_is("addr")? && this.item(1)?.tag_is("obj")? {
            let name = this.item(1)?.item(1)?.clone();
            if let Some(g) = self.a1_data_named(&name)
                && VAR_KINDS.contains(&g.kind)
            {
                return Ok(Some((name, g.kind)));
            }
        }
        if this.tag_is("impaddr")? {
            let sym = this.item(1)?.to_str();
            let k = data_kind(&sym);
            if VAR_KINDS.contains(&k) {
                return Ok(Some((Py::from(plain_name(&sym)), k)));
            }
        }
        Ok(None)
    }

    /// `A1Lifter.special`: engine plumbing the converted script implements itself; `true` when
    /// handled.
    #[allow(clippy::too_many_arguments)]
    pub fn a1_special(
        &mut self,
        st: &mut State,
        stmts: &mut Vec<Py>,
        a: i64,
        cls: Option<&str>,
        name: &str,
        ret: &str,
        params: &[String],
        raw: &[Py],
        this: &Py,
    ) -> Result<bool> {
        if cls.is_some_and(|c| VAR_CLASSES.contains(&c)) && var_method(name).is_some() {
            let cls = cls.unwrap_or("");
            let var = self.a1_this_var(this)?;
            let op = var_method(name).unwrap_or("");
            let what = format!("{cls}.{name}");
            let Some((vname, kind)) = var else {
                let why = format!("variable method on {}", and0(this)?);
                self.a1_undecoded(st, stmts, &what, ret, &why)?;
                return Ok(true);
            };
            let args = match self.a1_typed(st, raw, params, &what, a) {
                Ok(v) => v,
                Err(e) if e.kind == ErrorKind::Decode => {
                    self.a1_undecoded(st, stmts, &what, ret, before_at(&e.message))?;
                    return Ok(true);
                }
                Err(e) => return Err(e),
            };
            if op == "get" {
                let v = tup!["var", &vname, kind];
                self.clobber(st);
                st.r.insert("eax".to_owned(), v.clone());
                st.fret = if kind == "hot_float" { v } else { Py::None };
                return Ok(true);
            }
            if op == "time" || op == "is_dirty" {
                let t = self.new_tmp();
                stmts.push(tup!["set", &t, tup!["varq", &vname, op]]);
                self.clobber(st);
                st.r.insert("eax".to_owned(), t.clone());
                st.fret = t;
                return Ok(true);
            }
            let want = vname.clone();
            self.snap(st, stmts, &move |e| Ok(e.tag_is("var")? && *e.item(1)? == want), Py::None)?;
            stmts.push(tup!["var", &vname, op, Py::List(args)]);
            self.clobber(st);
            return Ok(true);
        }
        if cls == Some("character") && name == "new_script" {
            let nm = &unpack(raw, 1)?[0];
            let t = self.new_tmp();
            let arg = self.a1_arg(nm, a)?;
            stmts.push(tup!["set", &t, tup!["newscript", arg]]);
            self.clobber(st);
            st.r.insert("eax".to_owned(), tup!["script", &t]);
            return Ok(true);
        }
        if cls == Some("script") && name == "add_step" {
            let f = &unpack(raw, 1)?[0];
            if !f.truthy() || !f.tag_is("code")? || !this.truthy() || !this.tag_is("script")? {
                return dec(format!("add_step with {} on {} at {}", and0(f)?, and0(this)?, py_hex(a)));
            }
            let body = self.d.entry(int_of(f.item(1)?, "+")?)?;
            self.cur.steps.push(body);
            stmts.push(tup!["addstep", this.item(1)?, body]);
            self.clobber(st);
            return Ok(true);
        }
        if cls == Some("IIIE") && name == "create_rule" {
            if *this != tup!["worldoff", WORLD_IIIE] {
                return dec(format!("create_rule on {} at {}", this.to_str(), py_hex(a)));
            }
            let r = unpack(raw, 3)?;
            let (cond, act, nm) = (&r[0], &r[1], &r[2]);
            if !(is_head(cond, "code")? && is_head(act, "code")? && is_head(nm, "str")?) {
                return dec(format!("create_rule arguments not constant at {}", py_hex(a)));
            }
            let idx = self.a1.rules.len() as i64;
            self.a1.rules.push(super::Rule {
                name: nm.item(1)?.to_str(),
                cond: int_of(cond.item(1)?, "+")?,
                act: int_of(act.item(1)?, "+")?,
                deps: Vec::new(),
            });
            self.cur.a1.rules.push(idx);
            self.clobber(st);
            st.r.insert("eax".to_owned(), tup!["rule", idx]);
            return Ok(true);
        }
        if cls == Some("IIIE") && name == "trash_rule" {
            let x = &unpack(raw, 1)?[0];
            if *this != tup!["worldoff", WORLD_IIIE] {
                return dec(format!("trash_rule on {} at {}", this.to_str(), py_hex(a)));
            }
            if is_head(x, "str")? {
                stmts.push(tup!["trashrule", x]);
            } else if x.truthy() && x.tag_in(&["rule", "glob"])? {
                let idx = if x.tag_is("rule")? { Some(int_of(x.item(1)?, "+")?) } else { self.a1.rule_globals.get(&int_of(x.item(1)?, "+")?).copied() };
                let Some(idx) = idx else {
                    return dec(format!("trash_rule of an unknown rule at {}", py_hex(a)));
                };
                let Some(rule) = usize::try_from(idx).ok().and_then(|i| self.a1.rules.get(i)) else {
                    return index_err();
                };
                stmts.push(tup!["trashrule", tup!["str", rule.name.as_str()]]);
            } else {
                return dec(format!("trash_rule of {} at {}", and0(x)?, py_hex(a)));
            }
            self.clobber(st);
            return Ok(true);
        }
        if cls == Some("hot_type") && matches!(name, "add_dependent_rule" | "remove_dependent_rule") {
            let x = &unpack(raw, 1)?[0];
            let var = self.a1_this_var(this)?;
            let mut idx: Option<i64> = None;
            if is_head(x, "rule")? {
                idx = Some(int_of(x.item(1)?, "+")?);
            } else if is_head(x, "glob")? {
                idx = self.a1.rule_globals.get(&int_of(x.item(1)?, "+")?).copied();
            }
            let (Some((vname, _)), Some(idx)) = (var, idx) else {
                return dec(format!("{name} with {} on {} at {}", and0(x)?, and0(this)?, py_hex(a)));
            };
            if name == "add_dependent_rule" {
                let Some(rule) = usize::try_from(idx).ok().and_then(|i| self.a1.rules.get_mut(i)) else {
                    return index_err();
                };
                rule.deps.push(vname.to_str());
            }
            self.clobber(st);
            return Ok(true);
        }
        if cls == Some("linked_list") && matches!(name, "Add_Data" | "Remove_Data") {
            stmts.push(tup!["comment", format!("{name}: native save/load callback, not used")]);
            self.clobber(st);
            st.r.insert("eax".to_owned(), tup!["const", 1]);
            return Ok(true);
        }
        Ok(false)
    }

    /// `A1Lifter.typed`: arguments by parameter type; an `int &` is the int it refers to and a
    /// `Vector3 const &` the vector.
    pub fn a1_typed(&mut self, st: &mut State, raw: &[Py], params: &[String], what: &str, a: i64) -> Result<Vec<Py>> {
        let mut out: Vec<Py> = Vec::new();
        let mut k: usize = 0;
        let at = |k: usize| -> Result<&Py> { raw.get(k).ok_or_else(|| crate::DecodeError::python("IndexError", "list index out of range")) };
        for p in params {
            let c = push_count(p);
            let p = p.as_str();
            if matches!(p, "AAH" | "ABH" | "AAM" | "ABM") {
                let lv = self.a1_ref_lv(at(k)?, what, a)?;
                out.push(self.a1_deref_value(st, &lv, a)?);
            } else if matches!(p, "AA_N" | "AB_N") {
                let lv = self.a1_ref_lv(at(k)?, what, a)?;
                let v = self.a1_deref_value(st, &lv, a)?;
                out.push(tup!["boolarg", v]);
            } else if matches!(p, "ABVVector3@@" | "AAVVector3@@" | "PAVVector3@@" | "PBVVector3@@") {
                let lv = self.a1_ref_lv(at(k)?, what, a)?;
                let mut parts = Vec::new();
                for n in 0..3 {
                    let l = self.a1_lv_plus(&lv, 4 * n, a)?;
                    parts.push(self.a1_deref_value(st, &l, a)?);
                }
                out.push(tup!["vec", Py::List(parts)]);
            } else if p.starts_with("VVector3") {
                let mut parts = Vec::new();
                for x in raw.iter().skip(k).take(3) {
                    parts.push(self.a1_arg(x, a)?);
                }
                out.push(tup!["vec", Py::List(parts)]);
            } else if matches!(p, "PAD" | "PBD") && *at(k)? == tup!["const", 0] {
                out.push(tup!["nil"]); // a null `char *`: no name
            } else if c == 1 {
                let mut v = at(k)?.clone();
                if v.truthy() && v.tag_is("addr")? && v.item(1)?.tag_is("loc")? {
                    return dec(format!("{what}: pointer to a stack slot at {}", py_hex(a)));
                }
                if p == "M" && v.truthy() && v.tag_is("const")? {
                    v = tup!["float", float_of_bits(v.item(1)?)?];
                }
                let v = self.a1_arg(&v, a)?;
                out.push(if p == "_N" { tup!["boolarg", v] } else { v });
            } else {
                return dec(format!("{what}: parameter type {p} at {}", py_hex(a)));
            }
            k += c as usize;
        }
        Ok(out)
    }

    /// `A1Lifter.ref_lv`: the location a pointer or reference argument names.
    pub fn a1_ref_lv(&self, v: &Py, what: &str, a: i64) -> Result<Py> {
        if is_head(v, "addr")? {
            return Ok(v.item(1)?.clone());
        }
        if is_head(v, "impaddr")? {
            return Ok(tup!["deref", v, 0]);
        }
        dec(format!("{what}: reference argument {} at {}", and0(v)?, py_hex(a)))
    }

    /// `A1Lifter.lv_plus`: the location `off` bytes into `lv`.
    pub fn a1_lv_plus(&mut self, lv: &Py, off: i64, a: i64) -> Result<Py> {
        if off == 0 {
            return Ok(lv.clone());
        }
        let k = tg(lv)?;
        if k == "loc" {
            return Ok(tup!["loc", int_of(lv.item(1)?, "+")? + off]);
        }
        if k == "obj" {
            return self.glob_off(lv, off, a);
        }
        if k == "vfield" {
            return Ok(tup!["vfield", lv.item(1)?, int_of(lv.item(2)?, "+")? + py_floordiv(off, 4)?]);
        }
        if k == "deref" {
            return Ok(tup!["deref", lv.item(1)?, int_of(lv.item(2)?, "+")? + off]);
        }
        dec(format!("offset into {k} at {}", py_hex(a)))
    }

    /// `A1Lifter.deref_value`: the value a location holds.
    pub fn a1_deref_value(&mut self, st: &mut State, lv: &Py, a: i64) -> Result<Py> {
        let k = tg(lv)?;
        if k == "loc" {
            let j = py_floordiv(-int_of(lv.item(1)?, "-")?, 4)?;
            let v = self.a1_slot_read(st, j);
            return self.a1_arg(&v, a);
        }
        if k == "obj" {
            let name = lv.item(1)?;
            let g = self.a1_data_named(name);
            if g.is_some_and(|g| matches!(g.kind, "int" | "float")) {
                return Ok(tup!["obj", name]);
            }
            if g.is_some_and(|g| g.kind == "Vector3") {
                return Ok(tup!["vfield", name, 0]);
            }
            let kind = g.map_or("None".to_owned(), |g| g.kind.to_owned());
            return dec(format!("reference to {kind} {} at {}", name.to_str(), py_hex(a)));
        }
        if k == "elem" || k == "vfield" {
            return Ok(lv.clone());
        }
        if k == "deref" && lv.item(1)?.truthy() && lv.item(1)?.tag_is("impaddr")? {
            let sym = lv.item(1)?.item(1)?.to_str();
            return self.a1_import_data(&sym, int_of(lv.item(2)?, "+")?, a);
        }
        if k == "gvar" {
            return Ok(tup!["glob", lv.item(1)?]);
        }
        dec(format!("reference to {k} at {}", py_hex(a)))
    }

    /// `A1Lifter.arg`: a call argument must be known and no unfilled slot or kept register.
    pub fn a1_arg(&self, v: &Py, a: i64) -> Result<Py> {
        if v.is_none() {
            return dec(format!("unknown call argument at {}", py_hex(a)));
        }
        let k = tg(v)?;
        if k == "stackslot" {
            return dec(format!("argument slot not filled at {}", py_hex(a)));
        }
        if matches!(k, "saved" | "worldoff" | "impaddr" | "rule" | "index") {
            return dec(format!("argument is a {k} at {}", py_hex(a)));
        }
        Ok(v.clone())
    }
}
