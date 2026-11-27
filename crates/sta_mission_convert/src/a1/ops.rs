//! The Armada I stack model and the overrides of the core lifter's operand and instruction
//! methods (`A1Lifter` of `tools/a1_to_lua.py`: `stack_off`, `slot_lv`, `push`, `drop`,
//! `slot_read`, `slot_write`, `read`, `lvalue`, `write`, `glob`, `load`, `bad`, `import_data`,
//! `store`, `snap`, `step`, `arith`).
//!
//! The core's methods in `lift.rs` hand over to the `a1_*` methods here when the lifter's
//! [`Game`] is `A1`; the Python's `super().x(...)` calls are the core's `x_base` methods.

use regex_lite_free::{hex_imm, imm_text};

use crate::algebra::{lv_value, simplify};
use crate::dsl::pat::{self, M};
use crate::dsl::{is_reg32, reg8};
use crate::format::{lua_num, py_hex};
use crate::lift::{dec, int_of, py_err, tg, Flow, Game, Lifter, State};
use crate::msvc::{data_kind, plain_name};
use crate::pyval::{py_floordiv, py_int, py_mod, py_strip, Py};
use crate::tup;
use crate::Result;

use super::{and0, is_head, is_mem, is_uninit, mem, uninit, CFact, Edge};

/// Small matchers for the regexes of this module (no regex crate).
mod regex_lite_free {
    use crate::dsl::pat::M;

    /// `0x[0-9a-f]+` (`fullmatch`).
    pub fn hex_imm(t: &str) -> bool {
        let mut m = M::new(t);
        m.hex0x().is_some() && m.end_exact()
    }

    /// `-?0x[0-9a-f]+|-?[0-9]+` (`fullmatch`).
    pub fn imm_text(t: &str) -> bool {
        let u = t.strip_prefix('-').unwrap_or(t);
        hex_imm(u) || (!u.is_empty() && u.bytes().all(|b| b.is_ascii_digit()))
    }
}

impl<'d, 'a> Lifter<'d, 'a> {
    // ------------------------------------------------------------ stack

    /// `A1Lifter.stack_off`: the offset (from the return address, downwards negative) a stack
    /// operand names.
    pub fn a1_stack_off(&self, st: &mut State, reg: &str, k: i64, a: i64, byte: bool) -> Result<i64> {
        let mut off;
        if reg == "esp" {
            off = -4 * st.a1.depth + k;
        } else if reg == "ebp" {
            let Some(frame) = st.a1.frame else {
                return dec(format!("`ebp` used without a frame at {}", py_hex(a)));
            };
            off = -4 * frame + k;
        } else {
            return dec(format!("stack access through {reg} at {}", py_hex(a)));
        }
        let m = py_mod(off, 4)?;
        if m != 0 {
            // a one-byte temporary (a `bool const &` argument): its dword slot, only while
            // nothing else lives in that slot
            let j = py_floordiv(-(off - m), 4)?;
            let other = st.a1.slots.get(&j).is_none_or(|v| !is_uninit(v));
            if !byte || (other && !st.a1.byte_slots.contains(&j)) {
                return dec(format!("unaligned stack access at {}", py_hex(a)));
            }
            st.a1.byte_slots.insert(j);
            off -= m;
        }
        if off == 0 {
            return dec(format!("access to the return address at {}", py_hex(a)));
        }
        Ok(off)
    }

    /// `A1Lifter.slot_lv`: a stack offset as a location (a parameter above, a slot below).
    pub fn a1_slot_lv(&self, st: &State, off: i64, a: i64) -> Result<Py> {
        if off >= 4 {
            return Ok(tup!["param", py_floordiv(off - 4, 4)?]);
        }
        let j = py_floordiv(-off, 4)?;
        if j > st.a1.depth {
            return dec(format!("access below the stack pointer at {}", py_hex(a)));
        }
        Ok(tup!["loc", off])
    }

    /// `A1Lifter.push`.
    pub fn a1_push(&self, st: &mut State, v: Py) {
        st.a1.depth += 1;
        let d = st.a1.depth;
        st.a1.slots.insert(d, v);
        st.a1.entry_mem.remove(&d);
    }

    /// `A1Lifter.drop`.
    pub fn a1_drop(&self, st: &mut State, n: i64, a: i64) -> Result<()> {
        if n > st.a1.depth {
            return dec(format!("stack underflow at {}", py_hex(a)));
        }
        for _ in 0..n.max(0) {
            let d = st.a1.depth;
            st.a1.slots.shift_remove(&d);
            st.a1.entry_mem.remove(&d);
            st.a1.byte_slots.remove(&d);
            st.a1.depth -= 1;
        }
        Ok(())
    }

    /// `A1Lifter.slot_read`: the value in slot `j`, or the slot itself when its value is in the
    /// slot table.
    pub fn a1_slot_read(&self, st: &mut State, j: i64) -> Py {
        match st.a1.slots.get(&j) {
            Some(v) if !is_mem(v) => v.clone(),
            _ => {
                if st.a1.entry_mem.contains(&j) {
                    st.a1.loc_in.insert(j);
                }
                tup!["loc", -4 * j]
            }
        }
    }

    /// `A1Lifter.slot_write`.
    pub fn a1_slot_write(&mut self, st: &mut State, stmts: &mut Vec<Py>, j: i64, v: Py) -> Result<()> {
        let target = tup!["loc", -4 * j];
        self.snap(st, stmts, &move |e| Ok(*e == target), Py::None)?;
        st.a1.slots.insert(j, v);
        st.a1.entry_mem.remove(&j);
        Ok(())
    }

    // ------------------------------------------------------------ operands

    /// `A1Lifter.read`.
    pub fn a1_read(&mut self, st: &mut State, op: &str, a: i64) -> Result<Py> {
        let op = py_strip(op);
        if op == "ebp" {
            return Ok(match st.r.get("ebp") {
                None => {
                    st.reg_in.insert("ebp".to_owned());
                    tup!["reg", "ebp"]
                }
                Some(v) => v.clone(),
            });
        }
        if op == "esp" {
            return dec(format!("`esp` as a value at {}", py_hex(a)));
        }
        self.read_base(st, op, a)
    }

    /// `A1Lifter.lvalue` (`lea` for the `lea` instruction: the one-byte temporaries count).
    pub fn a1_lvalue(&mut self, st: &mut State, op: &str, a: i64, lea: bool) -> Result<Py> {
        let stripped = py_strip(op);
        let byte = lea || stripped.starts_with("BYTE PTR");
        let op2 = pat::strip_ptr(stripped);
        // `\[(esp|ebp)(?:([+-])(0x[0-9a-f]+))?\]`
        let mut m = M::new(op2);
        if m.lit("[") {
            let reg = if m.lit("esp") {
                Some("esp")
            } else if m.lit("ebp") {
                Some("ebp")
            } else {
                None
            };
            if let Some(reg) = reg {
                let mut k = 0;
                let sign = if m.lit("+") {
                    Some('+')
                } else if m.lit("-") {
                    Some('-')
                } else {
                    None
                };
                let mut ok = true;
                if sign.is_some() {
                    match m.hex0x() {
                        Some(h) => {
                            k = py_int(h, 16)?;
                            if sign == Some('-') {
                                k = -k;
                            }
                        }
                        None => ok = false,
                    }
                }
                if ok && m.lit("]") && m.end_exact() {
                    if reg == "ebp" && st.a1.frame.is_none() {
                        let base = self.read(st, "ebp", a)?;
                        return Ok(tup!["deref", base, k]); // `ebp` as a plain register
                    }
                    let off = self.a1_stack_off(st, reg, k, a, byte)?;
                    return self.a1_slot_lv(st, off, a);
                }
            }
        }
        // `\[(e\w\w)\+(e\w\w)\*4\]`
        let mut m = M::new(op2);
        if m.lit("[")
            && let Some(r1) = m.reg3()
            && m.lit("+")
            && let Some(r2) = m.reg3()
            && m.lit("*4]")
            && m.end_exact()
        {
            let base = self.read(st, r1, a)?;
            let idx = self.read(st, r2, a)?;
            return Ok(tup!["deref", tup!["index", base, idx], 0]);
        }
        self.lvalue_base(st, op, a)
    }

    /// `A1Lifter.write`.
    pub fn a1_write(&mut self, st: &mut State, stmts: &mut Vec<Py>, x: &str, v: Py, a: i64) -> Result<()> {
        if x == "ebp" && st.a1.frame.is_none() {
            st.r.insert("ebp".to_owned(), v);
            return Ok(());
        }
        self.write_base(st, stmts, x, v, a)
    }

    /// `A1Lifter.glob`: an imported slot is the import, else the core's.
    pub fn a1_glob(&mut self, addr: i64, a: i64) -> Result<Py> {
        if let Some(sym) = super::dll::import_sym(self.d.pe, addr) {
            return Ok(tup!["impslot", sym]);
        }
        self.glob_base(addr, a)
    }

    /// `A1Lifter.load`.
    pub fn a1_load(&mut self, st: &mut State, lv: &Py, a: i64) -> Result<Py> {
        let k = tg(lv)?;
        if k == "impslot" {
            return Ok(tup!["impaddr", lv.item(1)?]);
        }
        if k == "loc" {
            let j = py_floordiv(-int_of(lv.item(1)?, "-")?, 4)?;
            return Ok(self.a1_slot_read(st, j));
        }
        if k == "param" {
            return Ok(st.mem.get(lv).cloned().unwrap_or_else(|| lv.clone()));
        }
        if k == "deref" {
            let (base, off) = (lv.item(1)?.clone(), lv.item(2)?.clone());
            if base.truthy() && base.tag_is("index")? {
                let (arr, idx) = (base.item(1)?.clone(), base.item(2)?.clone());
                if arr.truthy() && arr.tag_is("impaddr")? && data_kind(&arr.item(1)?.to_str()) == "intarray" {
                    return Ok(tup!["elem", plain_name(&arr.item(1)?.to_str()), idx]);
                }
                if arr.truthy() && arr.tag_is("addr")? && arr.item(1)?.tag_is("obj")? {
                    return Ok(tup!["elem", arr.item(1)?.item(1)?, idx]);
                }
                return dec(format!("indexed load from {} at {}", and0(&arr)?, py_hex(a)));
            }
            let off_i = int_of(&off, "%")?;
            if base.truthy() && base.tag_is("impaddr")? {
                return self.a1_import_data(&base.item(1)?.to_str(), off_i, a);
            }
            if base.truthy() && base.tag_is("addr")? && base.item(1)?.tag_is("loc")? {
                if py_mod(off_i, 4)? == 0 {
                    let j = py_floordiv(-(int_of(base.item(1)?.item(1)?, "+")? + off_i), 4)?;
                    return Ok(self.a1_slot_read(st, j));
                }
                return self.a1_bad(a);
            }
            if base.truthy() && base.tag_is("addr")? && base.item(1)?.tag_is("param")? && off_i == 0 {
                let inner = base.item(1)?.clone();
                return self.load(st, &inner, a);
            }
        }
        self.load_base(st, lv, a)
    }

    /// `A1Lifter.bad`.
    fn a1_bad<T>(&self, a: i64) -> Result<T> {
        dec(format!("unaligned stack access at {}", py_hex(a)))
    }

    /// `A1Lifter.import_data`: a read of imported data at offset `off`.
    pub fn a1_import_data(&self, sym: &str, off: i64, a: i64) -> Result<Py> {
        let kind = data_kind(sym);
        let name = plain_name(sym);
        if matches!(kind, "int" | "float") && off == 0 {
            return Ok(tup!["obj", name]);
        }
        if kind == "intarray" && py_mod(off, 4)? == 0 {
            return Ok(tup!["elem", name, tup!["const", py_floordiv(off, 4)?]]);
        }
        if kind == "Vector3" && matches!(off, 0 | 4 | 8) {
            return Ok(tup!["vfield", name, py_floordiv(off, 4)?]);
        }
        dec(format!("read of imported {kind} {name}+{} at {}", py_hex(off), py_hex(a)))
    }

    /// `A1Lifter.store`.
    pub fn a1_store(&mut self, st: &mut State, stmts: &mut Vec<Py>, dst: &str, v: Py, a: i64) -> Result<()> {
        let mut lv = self.lvalue(st, dst, a)?;
        if tg(&lv)? == "deref" {
            let base = lv.item(1)?.clone();
            if base.truthy() && base.tag_is("addr")? && base.item(1)?.tag_is("loc")? {
                let off = int_of(base.item(1)?.item(1)?, "+")? + int_of(lv.item(2)?, "+")?;
                lv = tup!["loc", off];
            }
        }
        if tg(&lv)? == "deref" {
            let base = lv.item(1)?.clone();
            if base.truthy() && base.tag_is("impaddr")? {
                let sym = base.item(1)?.to_str();
                let kind = data_kind(&sym);
                let name = plain_name(&sym);
                let off = int_of(lv.item(2)?, "//")?;
                lv = if matches!(kind, "int" | "float") && off == 0 {
                    tup!["obj", name]
                } else if kind == "intarray" {
                    tup!["elem", name, tup!["const", py_floordiv(off, 4)?]]
                } else if kind == "Vector3" && matches!(off, 0 | 4 | 8) {
                    tup!["vfield", name, py_floordiv(off, 4)?]
                } else {
                    return dec(format!("store to imported {kind} {name} at {}", py_hex(a)));
                };
                let test = crate::lift::reads_location(&lv)?;
                let value = self.snap(st, stmts, &*test, v)?;
                stmts.push(tup!["set", &lv, value]);
                return Ok(());
            }
        }
        if tg(&lv)? == "loc" {
            let off = int_of(lv.item(1)?, "%")?;
            if py_mod(off, 4)? != 0 {
                return dec(format!("unaligned stack store at {}", py_hex(a)));
            }
            return self.a1_slot_write(st, stmts, py_floordiv(-off, 4)?, v);
        }
        if !v.is_none() && v.tag_is("rule")? && tg(&lv)? == "gvar" {
            let (key, idx) = (int_of(lv.item(1)?, "+")?, int_of(v.item(1)?, "+")?);
            self.a1.rule_globals.insert(key, idx);
            return Ok(());
        }
        if tg(&lv)? == "param" {
            let target = lv.clone();
            self.snap(st, stmts, &move |e| Ok(*e == target), Py::None)?;
            st.mem.insert(lv.clone(), v.clone());
            stmts.push(tup!["set", &lv, v]);
            return Ok(());
        }
        self.store_base(st, stmts, dst, v, a)
    }

    /// `A1Lifter.snap`: before a write, values the block still holds that read the written
    /// memory are copied to temps; the stack slots are part of the state.
    pub fn a1_snap(&mut self, st: &mut State, stmts: &mut Vec<Py>, test: &dyn Fn(&Py) -> Result<bool>, value: Py) -> Result<Py> {
        let mut cache: indexmap::IndexMap<Py, Py> = indexmap::IndexMap::new();
        let keys: Vec<String> = st.r.keys().cloned().collect();
        for k in keys {
            let v = st.r[&k].clone();
            let nv = self.snap_fix(&v, test, &mut cache, stmts)?;
            st.r.insert(k, nv);
        }
        let js: Vec<i64> = st.a1.slots.keys().copied().collect();
        for j in js {
            let v = st.a1.slots[&j].clone();
            if !is_mem(&v) && !is_uninit(&v) {
                let nv = self.snap_fix(&v, test, &mut cache, stmts)?;
                st.a1.slots.insert(j, nv);
            }
        }
        let mut fpu = Vec::with_capacity(st.fpu.len());
        for v in std::mem::take(&mut st.fpu) {
            fpu.push(self.snap_fix(&v, test, &mut cache, stmts)?);
        }
        st.fpu = fpu;
        let fret = st.fret.clone();
        st.fret = self.snap_fix(&fret, test, &mut cache, stmts)?;
        let keys: Vec<Py> = st.mem.keys().cloned().collect();
        for k in keys {
            if st.mem[&k] != lv_value(&k)? {
                let v = st.mem[&k].clone();
                let nv = self.snap_fix(&v, test, &mut cache, stmts)?;
                st.mem.insert(k, nv);
            }
        }
        self.snap_fix(&value, test, &mut cache, stmts)
    }

    // ------------------------------------------------------------ instructions

    /// `A1Lifter.step`.
    #[allow(clippy::too_many_arguments)]
    pub fn a1_step(&mut self, st: &mut State, stmts: &mut Vec<Py>, a: i64, mn: &str, ops: &str, i: usize, entry: bool) -> Result<Flow> {
        let d = self.d;
        if mn == "ret" {
            if st.a1.depth != 0 {
                return dec(format!("unbalanced stack at a return ({} slots) at {}", st.a1.depth, py_hex(a)));
            }
            let v = match st.r.get("eax") {
                None => self.read(st, "eax", a)?,
                Some(v) => v.clone(),
            };
            stmts.push(tup!["return", v]);
            return Ok(Flow::Ret);
        }
        if mn == "push" {
            let o = py_strip(ops);
            if entry && matches!(o, "eax" | "ecx" | "edx") && !st.r.contains_key(o) {
                self.a1_push(st, uninit()); // `push ecx` reserving a slot: the value is never used
                return Ok(Flow::Next);
            }
            let v = self.read(st, ops, a)?;
            self.a1_push(st, v);
            return Ok(Flow::Next);
        }
        if mn == "pop" {
            if st.a1.depth == 0 {
                return dec(format!("pop of the return address at {}", py_hex(a)));
            }
            let depth = st.a1.depth;
            let v = self.a1_slot_read(st, depth);
            self.a1_drop(st, 1, a)?;
            let o = py_strip(ops);
            if is_reg32(o) || o == "ebp" {
                st.r.insert(o.to_owned(), v);
            } else {
                self.store(st, stmts, ops, v, a)?;
            }
            return Ok(Flow::Next);
        }
        if (mn == "add" || mn == "sub") && ops.starts_with("esp,") {
            let n = match ops.split(',').nth(1) {
                Some(s) => py_int(s, 16)?,
                None => return crate::lift::index_err(),
            };
            if py_mod(n, 4)? != 0 {
                return dec(format!("`{mn} esp,{}` at {}", py_hex(n), py_hex(a)));
            }
            if mn == "add" {
                self.a1_drop(st, py_floordiv(n, 4)?, a)?;
            } else {
                for _ in 0..py_floordiv(n, 4)?.max(0) {
                    self.a1_push(st, uninit());
                }
            }
            return Ok(Flow::Next);
        }
        if mn == "leave" {
            let Some(frame) = st.a1.frame else {
                return dec(format!("`leave` without a frame at {}", py_hex(a)));
            };
            self.a1_drop(st, st.a1.depth - frame, a)?;
            let depth = st.a1.depth;
            let v = self.a1_slot_read(st, depth);
            st.r.insert("ebp".to_owned(), v);
            self.a1_drop(st, 1, a)?;
            st.a1.frame = None;
            return Ok(Flow::Next);
        }
        if mn == "mov" {
            let (dst, src) = crate::lift::split2(ops)?;
            if dst == "ebp" && src == "esp" {
                st.a1.frame = Some(st.a1.depth);
                return Ok(Flow::Next);
            }
            if dst == "esp" && src == "ebp" {
                let Some(frame) = st.a1.frame else {
                    return dec(format!("`mov esp,ebp` without a frame at {}", py_hex(a)));
                };
                self.a1_drop(st, st.a1.depth - frame, a)?;
                return Ok(Flow::Next);
            }
            if is_reg32(&dst) && src == "esp" {
                if st.a1.depth == 0 {
                    return dec(format!("address of the return slot at {}", py_hex(a)));
                }
                st.r.insert(dst, tup!["addr", tup!["loc", -4 * st.a1.depth]]);
                return Ok(Flow::Next);
            }
            if dst == "esp" || (dst == "ebp" && st.a1.frame.is_some()) {
                return dec(format!("`mov {ops}` at {}", py_hex(a)));
            }
            if is_reg32(&dst) || dst == "ebp" {
                let v = self.read(st, &src, a)?;
                st.r.insert(dst, v);
                return Ok(Flow::Next);
            }
            if matches!(dst.as_str(), "al" | "cl" | "dl" | "bl") {
                let v = self.read(st, &src, a)?;
                st.r.insert(reg8(&dst).unwrap_or("eax").to_owned(), v);
                return Ok(Flow::Next);
            }
            let v = self.read(st, &src, a)?;
            self.store(st, stmts, &dst, v, a)?;
            return Ok(Flow::Next);
        }
        if mn == "lea" {
            let (dst, src) = crate::lift::split2(ops)?;
            if !is_reg32(&dst) {
                return dec(format!("`lea {ops}` at {}", py_hex(a)));
            }
            let lv = self.a1_lvalue(st, &src, a, true)?;
            if tg(&lv)? == "deref" {
                let base = lv.item(1)?;
                if base.truthy() && base.tag_is("impaddr")? && data_kind(&base.item(1)?.to_str()) == "character" {
                    let off = lv.item(2)?.clone();
                    st.r.insert(dst, tup!["worldoff", off]);
                    return Ok(Flow::Next);
                }
            }
            st.r.insert(dst, tup!["addr", lv]);
            return Ok(Flow::Next);
        }
        if (mn == "add" || mn == "sub") && is_reg32(py_strip(ops.split(',').next().unwrap_or(""))) {
            let (x, y) = crate::lift::split2(ops)?;
            let vx = self.read(st, &x, a)?;
            if vx.truthy() && vx.tag_is("impaddr")? && data_kind(&vx.item(1)?.to_str()) == "character" && hex_imm(&y) {
                let n = py_int(&y, 16)?;
                st.r.insert(x, tup!["worldoff", if mn == "add" { n } else { -n }]);
                return Ok(Flow::Next);
            }
        }
        if mn == "call" {
            return self.a1_call(st, stmts, a, ops, i);
        }
        if mn == "jmp" {
            let mut tail = None;
            if let Some(h) = hex_target(ops) {
                let t = py_int(h, 16)?;
                if self.a1_is_tail(self.cur.addr, t) {
                    tail = Some(t);
                }
            }
            if let Some(h) = ds_target(ops) {
                let slot = py_int(h, 16)?;
                if let Some(sym) = super::dll::import_sym(d.pe, slot).filter(|s| !s.is_empty()) {
                    return self.a1_tail_import(st, stmts, a, &sym);
                }
            }
            if let Some(t) = tail {
                match self.a1_resolve(t)? {
                    super::lift::Resolved::Imp(sym) => return self.a1_tail_import(st, stmts, a, &sym),
                    super::lift::Resolved::Fn(x) => {
                        if st.a1.depth != 0 {
                            return dec(format!("tail call with {} slots on the stack at {}", st.a1.depth, py_hex(a)));
                        }
                        let tmp = self.new_tmp();
                        let n = self.a1_ret_slots(x);
                        let args = Py::List((0..n).map(|k| tup!["param", k]).collect());
                        stmts.push(tup!["set", &tmp, tup!["fcall", x, args]]);
                        if !self.cur.calls.contains(&x) {
                            self.cur.calls.push(x);
                        }
                        stmts.push(tup!["return", tmp]);
                        return Ok(Flow::Ret);
                    }
                }
            }
        }
        self.step_base(st, stmts, a, mn, ops, i, entry)
    }

    /// `A1Lifter.arith`.
    pub fn a1_arith(&mut self, st: &mut State, stmts: &mut Vec<Py>, a: i64, mn: &str, ops: &str) -> Result<()> {
        let (x, y) = crate::lift::split2(ops)?;
        if mn == "or" && y == "0xffffffff" {
            self.write(st, stmts, &x, tup!["const", -1], a)?; // `or r,-1`: r = -1
            st.set_flags(tup!["test", tup!["const", -1]]);
            return Ok(());
        }
        if mn == "and" && y == "0x0" {
            self.write(st, stmts, &x, tup!["const", 0], a)?;
            st.set_flags(tup!["test", tup!["const", 0]]);
            return Ok(());
        }
        if !y.contains(',') && !(x == y && matches!(mn, "xor" | "sub" | "sbb")) {
            let vx = self.read(st, &x, a)?;
            let vy = self.read(st, &y, a)?;
            if vx.is_none() || vy.is_none() {
                return dec(format!("`{mn} {ops}` on an unknown value at {}", py_hex(a)));
            }
        }
        self.arith_base(st, stmts, a, mn, ops)
    }

    // ------------------------------------------------------------ compare facts

    /// `A1Lifter.note_compare`: remember what a `test r,r` / `cmp r,imm` plus `je`/`jne` says
    /// about the register on each edge: the compiler reuses a register it has just proved
    /// equal to a constant (`test eax,eax; jne L; push eax` passes a null).
    pub fn a1_note_compare(&self, st: &mut State, mn: &str, ops: &str, r: &Flow, nxt: Option<i64>) -> Result<()> {
        if mn == "test" || mn == "cmp" {
            st.a1.cfact = None;
            // `x, _, y = [t.strip() for t in ops.partition(",")]`
            let (x, y) = match ops.split_once(',') {
                Some((x, y)) => (py_strip(x), py_strip(y)),
                None => (py_strip(ops), ""),
            };
            if !crate::dsl::is_reg32(x) || !st.r.contains_key(x) {
                return Ok(());
            }
            let imm;
            if mn == "test" && y == x {
                imm = 0;
            } else if mn == "cmp" && imm_text(y) {
                let n = if y.contains("0x") { py_int(y, 16)? } else { py_int(y, 10)? };
                imm = n & 0xFFFF_FFFF; // unsigned, as `mov r,imm` reads
            } else {
                return Ok(());
            }
            st.a1.cfact = Some(CFact { reg: x.to_owned(), imm, value: st.rget(x), flags_gen: st.flags_gen });
        } else if let Flow::Jcc(tgt, _) = r
            && let Some(f) = st.a1.cfact.clone()
            && matches!(mn, "je" | "jz" | "jne" | "jnz")
            && st.rget(&f.reg) == f.value
            && st.flags_gen == f.flags_gen
            && Some(*tgt) != nxt
        {
            st.a1.edge = Some(Edge { reg: f.reg.clone(), imm: f.imm, eq_on_taken: matches!(mn, "je" | "jz"), tgt: *tgt });
        }
        Ok(())
    }

    /// `A1Lifter.edge_value`: the value of `reg` at the end of a predecessor, as seen on its edge
    /// into `b` (`None` is the Python's `"unset"`).
    pub fn a1_edge_value(pend: &State, reg: &str, b: i64) -> Option<Py> {
        if let Some(e) = &pend.a1.edge
            && e.reg == reg
        {
            let taken = b == e.tgt;
            if taken == e.eq_on_taken {
                return Some(tup!["const", e.imm]);
            }
        }
        pend.r.get(reg).cloned()
    }
}

/// `re.fullmatch(r"0x([0-9a-f]+)", ops)`: the digits.
pub(crate) fn hex_target(ops: &str) -> Option<&str> {
    let mut m = M::new(ops);
    if !m.lit("0x") {
        return None;
    }
    let h = m.hex1()?;
    m.end_exact().then_some(h)
}

/// `re.fullmatch(r"DWORD PTR ds:0x([0-9a-f]+)", ops)`: the digits.
pub(crate) fn ds_target(ops: &str) -> Option<&str> {
    let mut m = M::new(ops);
    if !m.lit("DWORD PTR ds:0x") {
        return None;
    }
    let h = m.hex1()?;
    m.end_exact().then_some(h)
}

#[allow(dead_code)]
fn _unused(_: &str) -> Result<()> {
    let _ = (lua_num, simplify, py_err::<()>, is_head, mem, Game::A1);
    Ok(())
}
