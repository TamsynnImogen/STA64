//! `A1Lifter`'s control-flow graph and per-function lift (`is_tail`, `cfg`, `lift`, `linear`,
//! `check_helpers`, `reads_params`, `entry_state`, `pure`) and `imports_in` of
//! `tools/a1_to_lua.py`.

use std::collections::{BTreeSet, HashMap, HashSet};

use indexmap::IndexMap;

use crate::algebra::simplify;
use crate::dsl::pat::{self, M};
use crate::dsl::REG32;
use crate::format::py_hex;
use crate::lift::{dec, index_err, key_err, Flow, Lifted, Lifter, State};
use crate::msvc::signature;
use crate::pyval::{py_int, py_split_ws_once, Py};
use crate::tup;
use crate::{ErrorKind, Result};

use super::calls::call_name;
use super::dll::{import_sym, jmp_ds_import};
use super::{is_mem, is_uninit, mem, SAVED};

/// `PURE` of `a1_to_lua.py`: node kinds whose value is the same in every block of the function.
const PURE: [&str; 30] = [
    "const", "float", "str", "code", "tmp", "impaddr", "varptr", "world", "iiie", "script", "saved", "rule", "this", "field", "farith", "arith", "neg",
    "cmp", "not", "and", "or", "truth", "u", "ftol", "sign", "abs", "sqrt", "bnot", "vec", "boolarg",
];

/// `pure(v)` of `a1_to_lua.py`: a value that means the same in every block of the function (it
/// reads no register, stack slot or memory that could change).
pub fn pure(v: &Py) -> bool {
    let Py::Tuple(items) = v else { return false };
    let Some(head) = items.first() else { return false };
    let k = head.as_str();
    if k == Some("addr") {
        let Some(Py::Tuple(lv)) = items.get(1) else { return false };
        let Some(Some(lk)) = lv.first().map(Py::as_str) else { return false };
        return matches!(lk, "obj" | "elem" | "vfield" | "gvar" | "loc") && lv.iter().skip(2).all(|x| !matches!(x, Py::Tuple(_)) || pure(x));
    }
    if !k.is_some_and(|k| PURE.contains(&k)) {
        return false;
    }
    for x in &items[1..] {
        if let Py::Tuple(t) = x
            && matches!(t.first(), Some(Py::Str(_)))
            && !pure(x)
        {
            return false;
        }
        if let Py::List(l) = x
            && !l.iter().all(pure)
        {
            return false;
        }
    }
    true
}

/// What `A1Dll.resolve` answers: `("imp", symbol)` or `("fn", address)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    Imp(String),
    Fn(i64),
}

/// `re.fullmatch(r"(?:call|jmp)\s+0x([0-9a-f]+)", t)`: the digits.
fn call_or_jmp_abs(t: &str) -> Option<&str> {
    let mut m = M::new(t);
    if !((m.lit("call") || m.lit("jmp")) && m.ws1() && m.lit("0x")) {
        return None;
    }
    let h = m.hex1()?;
    m.end_exact().then_some(h)
}

/// `re.fullmatch(r"mov\s+(e\w\w),DWORD PTR ds:0x([0-9a-f]+)", t)`: register and digits.
fn mov_from_import(t: &str) -> Option<(&str, &str)> {
    let mut m = M::new(t);
    if !(m.lit("mov") && m.ws1()) {
        return None;
    }
    let r = m.reg3()?;
    if !m.lit(",DWORD PTR ds:0x") {
        return None;
    }
    let h = m.hex1()?;
    m.end_exact().then_some((r, h))
}

/// `re.fullmatch(r"(?:call|jmp)\s+(?:DWORD PTR ds:)?0x([0-9a-f]+)", t)`: the digits.
fn call_or_jmp_maybe_ds(t: &str) -> Option<&str> {
    let mut m = M::new(t);
    if !((m.lit("call") || m.lit("jmp")) && m.ws1()) {
        return None;
    }
    m.lit("DWORD PTR ds:");
    if !m.lit("0x") {
        return None;
    }
    let h = m.hex1()?;
    m.end_exact().then_some(h)
}

/// `re.fullmatch(r"call\s+(e\w\w)", t)`: the register.
fn call_reg(t: &str) -> Option<&str> {
    let mut m = M::new(t);
    if !(m.lit("call") && m.ws1()) {
        return None;
    }
    let r = m.reg3()?;
    m.end_exact().then_some(r)
}

/// `re.search(r"\[(esp|ebp)\+0x[0-9a-f]+\]", t)`: some operand reads a positive stack offset.
fn reads_stack_param(t: &str) -> bool {
    for (i, c) in t.char_indices() {
        if c != '[' {
            continue;
        }
        let mut m = M::new(&t[i + 1..]);
        if (m.lit("esp") || m.lit("ebp")) && m.lit("+0x") && m.hex1().is_some() && m.lit("]") {
            return true;
        }
    }
    false
}

impl<'d, 'a> Lifter<'d, 'a> {
    /// `A1Dll.resolve`: a call or jump target as `("imp", symbol)` or `("fn", address)`.
    pub fn a1_resolve(&self, addr: i64) -> Result<Resolved> {
        let d = self.d;
        let a = d.thunks.get(&addr).copied().unwrap_or(addr);
        if let Some(sym) = self.a1.stubs.get(&a) {
            return Ok(Resolved::Imp(sym.clone()));
        }
        if !d.at.contains_key(&a) {
            return dec(format!("no instruction at {}", py_hex(a)));
        }
        Ok(Resolved::Fn(a))
    }

    /// `A1Lifter.is_tail`: a jump to an import, a link-table thunk or another function.
    pub fn a1_is_tail(&self, start: i64, tgt: i64) -> bool {
        let d = self.d;
        let a = d.thunks.get(&tgt).copied().unwrap_or(tgt);
        self.a1.stubs.contains_key(&a) || d.thunks.contains_key(&tgt) || (self.a1.entries.contains(&a) && a != start)
    }

    /// `d.ins[i]` as `(address, text)`; an `IndexError` past the end.
    fn at_i(&self, i: usize) -> Result<(i64, &'d str)> {
        self.ins(i)
    }

    /// `A1Lifter.cfg`: blocks and edges: `{leader: [successors]}`; a jump to an import, a
    /// link-table thunk or another function is a tail call (no edge). Returns the leaders (those
    /// reached), the instruction addresses reached, and the successors by leader in address order.
    #[allow(clippy::type_complexity)]
    pub fn a1_cfg(&mut self, start: i64) -> Result<(HashSet<i64>, HashSet<i64>, IndexMap<i64, Vec<i64>>)> {
        let d = self.d;
        let mut todo: Vec<i64> = vec![start];
        let mut seen: HashSet<i64> = HashSet::new();
        let mut leaders: HashSet<i64> = HashSet::from([start]);
        while let Some(a) = todo.pop() {
            if seen.contains(&a) {
                continue;
            }
            let Some(mut i) = d.index_of(a) else {
                return dec(format!("jump outside the code at {}", py_hex(a)));
            };
            loop {
                let (x, t) = self.at_i(i)?;
                seen.insert(x);
                let mn = match py_split_ws_once(t).first() {
                    Some(w) => *w,
                    None => return index_err(),
                };
                if let Some((_, h)) = pat::jcc_abs(t) {
                    let tgt = py_int(h, 16)?;
                    if mn == "jmp" {
                        if !self.a1_is_tail(start, tgt) {
                            leaders.insert(tgt);
                            todo.push(tgt);
                        }
                        break;
                    }
                    leaders.insert(tgt);
                    todo.push(tgt);
                    let nxt = self.at_i(i + 1)?.0;
                    leaders.insert(nxt);
                    todo.push(self.at_i(i + 1)?.0);
                    break;
                }
                if mn == "jmp" {
                    if let Some(h) = jmp_ds_import(t) {
                        let slot = py_int(h, 16)?;
                        if import_sym(d.pe, slot).is_some() {
                            break;
                        }
                    }
                    let tbl = self.switch_table(i)?;
                    self.switches.insert(x, tbl.clone());
                    for tgt in tbl {
                        leaders.insert(tgt);
                        todo.push(tgt);
                    }
                    break;
                }
                if mn.starts_with('j') {
                    return dec(format!("indirect jump `{t}` at {}", py_hex(x)));
                }
                if mn == "ret" {
                    break;
                }
                i += 1;
                if i >= d.ins.len() {
                    return dec("ran off the code".to_owned());
                }
                if leaders.contains(&d.addr_at(i)) {
                    todo.push(d.addr_at(i));
                    break;
                }
            }
        }
        leaders.retain(|l| seen.contains(l));
        // successors, by walking each block to its end
        let mut succs: IndexMap<i64, Vec<i64>> = IndexMap::new();
        let mut sorted: Vec<i64> = leaders.iter().copied().collect();
        sorted.sort_unstable(); // address order: `succs`, then `preds`, keep it
        for b in sorted {
            let Some(mut i) = d.index_of(b) else {
                return key_err(&Py::Int(b));
            };
            let mut first = true;
            loop {
                let (x, t) = self.at_i(i)?;
                if !first && leaders.contains(&x) {
                    succs.insert(b, vec![x]);
                    break;
                }
                first = false;
                let mn = match py_split_ws_once(t).first() {
                    Some(w) => *w,
                    None => return index_err(),
                };
                if let Some((_, h)) = pat::jcc_abs(t) {
                    let tgt = py_int(h, 16)?;
                    if mn == "jmp" {
                        succs.insert(b, if self.a1_is_tail(start, tgt) { vec![] } else { vec![tgt] });
                    } else {
                        succs.insert(b, vec![tgt, self.at_i(i + 1)?.0]);
                    }
                    break;
                }
                if mn == "jmp" {
                    succs.insert(b, self.switches.get(&x).cloned().unwrap_or_default());
                    break;
                }
                if mn == "ret" {
                    succs.insert(b, vec![]);
                    break;
                }
                i += 1;
            }
        }
        Ok((leaders, seen, succs))
    }

    /// `A1Lifter.lift`: the function at `start` in topological block order, or the reason it is
    /// not decoded (`Lifted::error`) with the imports it calls.
    pub fn a1_lift(&mut self, start: i64) -> Result<Lifted> {
        self.cur = Lifted::new(start);
        self.switches = IndexMap::new();
        match self.a1_lift_body(start) {
            Ok(()) => {}
            Err(e) if e.kind == ErrorKind::Decode => {
                self.cur.error = Some(e.message);
                if self.cur.a1.imports_called.is_empty() {
                    let lin: Vec<i64> = self.a1_linear(start).into_iter().collect();
                    self.cur.a1.imports_called = self.a1_imports_in(&lin)?;
                }
            }
            Err(e) => return Err(e),
        }
        Ok(std::mem::take(&mut self.cur))
    }

    #[allow(clippy::mut_range_bound)]
    fn a1_lift_body(&mut self, start: i64) -> Result<()> {
        let d = self.d;
        let (leaders, seen, succs) = self.a1_cfg(start)?;
        self.cur.instructions = seen.len();
        let mut preds: HashMap<i64, Vec<i64>> = HashMap::new();
        for (b, ss) in &succs {
            for s in ss {
                preds.entry(*s).or_default().push(*b);
            }
        }
        // topological order (the CFG must be acyclic)
        let mut order: Vec<i64> = Vec::new();
        let mut mark: HashMap<i64, u8> = HashMap::new();
        visit(start, &succs, &mut mark, &mut order)?;
        order.reverse();
        let mut seen_sorted: Vec<i64> = seen.iter().copied().collect();
        seen_sorted.sort_unstable();
        self.cur.a1.imports_called = self.a1_imports_in(&seen_sorted)?;
        self.a1_check_helpers(start, &seen_sorted)?;
        self.ntmp = 0;
        let mut ends: HashMap<i64, State> = HashMap::new();
        let mut blocks: HashMap<i64, Vec<Py>> = HashMap::new();
        for &b in &order {
            let mut st = self.a1_entry_state(b, start, &preds, &ends)?;
            let mut stmts: Vec<Py> = Vec::new();
            let Some(mut i) = d.index_of(b) else { return key_err(&Py::Int(b)) };
            let mut first = true;
            let mut last_a;
            loop {
                let (a, t) = self.at_i(i)?;
                last_a = a;
                if !first && leaders.contains(&a) {
                    break;
                }
                first = false;
                let parts = py_split_ws_once(t);
                let Some(&mn) = parts.first() else { return index_err() };
                let ops = parts.get(1).copied().unwrap_or("");
                let r = self.step(&mut st, &mut stmts, a, mn, ops, i, start == b)?;
                let nxt = if i + 1 < d.ins.len() { Some(d.addr_at(i + 1)) } else { None };
                self.a1_note_compare(&mut st, mn, ops, &r, nxt)?;
                match r {
                    Flow::Ret => break,
                    Flow::Switch(idx, targets) => {
                        stmts.push(Py::Tuple(vec![Py::from("switch"), idx, Py::List(targets.iter().map(|t| Py::Int(*t)).collect())]));
                        break;
                    }
                    Flow::Jmp(tgt) => {
                        stmts.push(tup!["goto", tgt]);
                        break;
                    }
                    Flow::Jcc(tgt, pred) => {
                        stmts.push(tup!["if", simplify(&pred)?, tgt]);
                        break;
                    }
                    Flow::Next => {}
                }
                i += 1;
            }
            if !st.fpu.is_empty() {
                return dec(format!("x87 value live across a block end at {}", py_hex(last_a)));
            }
            ends.insert(b, st);
            blocks.insert(b, stmts);
        }
        // values a later block reads from registers or the slot table, also through blocks that
        // pass them on untouched (backward liveness)
        let mut live_r: HashMap<i64, BTreeSet<String>> = HashMap::new();
        let mut live_s: HashMap<i64, BTreeSet<i64>> = HashMap::new();
        for &b in order.iter().rev() {
            let mut out_r: BTreeSet<String> = BTreeSet::new();
            let mut out_s: BTreeSet<i64> = BTreeSet::new();
            for s in &succs[&b] {
                let (Some(lr), Some(ls)) = (live_r.get(s), live_s.get(s)) else {
                    return key_err(&Py::Int(*s));
                };
                out_r.extend(lr.iter().cloned());
                out_s.extend(ls.iter().copied());
            }
            let st = ends.get_mut(&b).expect("lifted block");
            let mut lr = st.reg_in.clone();
            lr.extend(out_r.iter().filter(|r| !st.r.contains_key(*r)).cloned());
            let mut ls = st.a1.loc_in.clone();
            ls.extend(out_s.iter().filter(|j| st.a1.slots.get(*j).is_none_or(is_mem) && st.a1.entry_mem.contains(*j)).copied());
            st.a1.out_r = out_r;
            st.a1.out_s = out_s;
            live_r.insert(b, lr);
            live_s.insert(b, ls);
        }
        let ls = &live_r[&start];
        if ls.iter().any(|r| !SAVED.contains(&r.as_str())) {
            let l = Py::List(ls.iter().map(|r| Py::from(r.as_str())).collect());
            return dec(format!("register {} read before it is set", l.repr()));
        }
        for &b in &order {
            let st = &ends[&b];
            let stmts = blocks.get_mut(&b).expect("block");
            let mut pairs: Vec<Py> = Vec::new();
            for r in &st.a1.out_r {
                let v = st.rget(r);
                if v.is_none() || v == tup!["reg", r] {
                    continue;
                }
                pairs.push(tup![tup!["reg", r], v]);
            }
            for &j in &st.a1.out_s {
                let v = st.a1.slots.get(&j).cloned().unwrap_or_else(mem);
                if is_mem(&v) || is_uninit(&v) || v.is_none() {
                    continue;
                }
                pairs.push(tup![tup!["loc", -4 * j], v]);
            }
            if !pairs.is_empty() {
                let mut k = stmts.len();
                while k > 0 && crate::lift::tg(&stmts[k - 1])?.eq_any(&["goto", "if", "return", "switch"]) {
                    k -= 1;
                }
                let n0 = stmts.len();
                for j in k..n0 {
                    let sj = stmts[j].clone();
                    if crate::lift::tg(&sj)?.eq_any(&["if", "switch"]) {
                        self.ntmp += 1;
                        let t = tup!["tmp", self.ntmp];
                        stmts.insert(k, tup!["set", &t, sj.item(1)?]);
                        k += 1;
                        let mut ns = vec![sj.item(0)?.clone(), t];
                        ns.extend_from_slice(sj.tail()?.get(1..).unwrap_or(&[]));
                        stmts[j + 1] = Py::Tuple(ns);
                    }
                }
                stmts.insert(k, Py::Tuple(vec![Py::from("multiset"), Py::List(pairs)]));
            }
            // a block that falls through to a block lifted out of order needs a jump
            let nxt = &succs[&b];
            if stmts.last().is_some_and(|s| matches!(crate::lift::tg(s), Ok("if"))) && nxt.len() == 2 {
                stmts.push(tup!["goto", nxt[1]]);
            } else if nxt.len() == 1 && stmts.last().is_none_or(|s| !matches!(crate::lift::tg(s), Ok("goto" | "return" | "switch"))) {
                stmts.push(tup!["goto", nxt[0]]);
            }
        }
        self.cur.blocks = order.iter().map(|b| (*b, blocks.remove(b).unwrap_or_default())).collect();
        Ok(())
    }

    /// `A1Lifter.linear`: addresses from `start` in address order up to the function's end, for
    /// listing the calls of a function the lifter could not follow.
    pub fn a1_linear(&self, start: i64) -> BTreeSet<i64> {
        let d = self.d;
        let mut out: BTreeSet<i64> = BTreeSet::new();
        let mut i = d.index_of(start);
        while let Some(k) = i {
            if k >= d.ins.len() || out.len() >= 4000 {
                break;
            }
            let (a, t) = (d.addr_at(k), d.text_at(k));
            if !out.is_empty() && self.a1.entries.contains(&a) {
                break;
            }
            out.insert(a);
            if t.starts_with("int3") && out.len() > 1 {
                break;
            }
            i = Some(k + 1);
        }
        out
    }

    /// `A1Lifter.check_helpers`: internal calls and tail jumps pass no arguments the lifter could
    /// miss: the callee must read none (the missions' helpers take none). `seen` is in address
    /// order: the first offending call site is named in the error text that reaches the output.
    pub fn a1_check_helpers(&mut self, start: i64, seen: &[i64]) -> Result<()> {
        let d = self.d;
        for &x in seen {
            let Some(i) = d.index_of(x) else { return key_err(&Py::Int(x)) };
            let t = d.text_at(i);
            let Some(h) = call_or_jmp_abs(t) else { continue };
            let tgt = py_int(h, 16)?;
            if t.starts_with("jmp") && !self.a1_is_tail(start, tgt) {
                continue;
            }
            let body = match self.a1_resolve(tgt) {
                Ok(Resolved::Fn(b)) => b,
                Ok(Resolved::Imp(_)) => continue,
                Err(e) if e.kind == ErrorKind::Decode => continue,
                Err(e) => return Err(e),
            };
            if body == start || d.is_ftol(body)? || self.a1.no_params.contains(&body) {
                continue;
            }
            if self.a1_reads_params(body)? {
                return dec(format!("call at {} to a helper that takes arguments", py_hex(x)));
            }
            self.a1.no_params.insert(body);
        }
        Ok(())
    }

    /// `A1Lifter.reads_params`: conservative: any positive stack offset.
    pub fn a1_reads_params(&self, body: i64) -> Result<bool> {
        let d = self.d;
        for a in self.a1_linear(body) {
            let Some(i) = d.index_of(a) else { return key_err(&Py::Int(a)) };
            if reads_stack_param(d.text_at(i)) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `A1Lifter.entry_state`: the state a block starts with, from its predecessors' ends.
    pub fn a1_entry_state(&self, b: i64, start: i64, preds: &HashMap<i64, Vec<i64>>, ends: &HashMap<i64, State>) -> Result<State> {
        let mut st = State::new();
        st.body = true;
        if b == start {
            for r in SAVED {
                st.r.insert(r.to_owned(), tup!["saved", r]);
            }
            return Ok(st);
        }
        let ps: &[i64] = preds.get(&b).map_or(&[], Vec::as_slice);
        if ps.is_empty() || ps.iter().any(|p| !ends.contains_key(p)) {
            return dec(format!("block {} reached before its predecessors", py_hex(b)));
        }
        let depths: HashSet<i64> = ps.iter().map(|p| ends[p].a1.depth).collect();
        let frames: HashSet<Option<i64>> = ps.iter().map(|p| ends[p].a1.frame).collect();
        if depths.len() != 1 || frames.len() != 1 {
            return dec(format!("stack depth differs between the paths into {}", py_hex(b)));
        }
        st.a1.depth = *depths.iter().next().expect("one depth");
        st.a1.frame = *frames.iter().next().expect("one frame");
        for r in REG32.iter().copied().chain(["ebp"]) {
            let vals: Vec<Option<Py>> = ps.iter().map(|p| Self::a1_edge_value(&ends[p], r, b)).collect();
            if vals.iter().all(|v| *v == vals[0])
                && let Some(v) = &vals[0]
                && pure(v)
            {
                st.r.insert(r.to_owned(), v.clone());
            }
        }
        for j in 1..=st.a1.depth {
            let vals: Vec<Py> = ps.iter().map(|p| ends[p].a1.slots.get(&j).cloned().unwrap_or_else(mem)).collect();
            if vals.iter().all(|v| *v == vals[0]) && (is_uninit(&vals[0]) || pure(&vals[0])) {
                st.a1.slots.insert(j, vals[0].clone());
            } else {
                st.a1.slots.insert(j, mem());
                st.a1.entry_mem.insert(j);
            }
        }
        Ok(st)
    }

    /// `imports_in(d, seen)`: imported functions a function calls (directly, through a register
    /// loaded from the import table, through a stub, or as a tail jump). `seen` in address order.
    pub fn a1_imports_in(&self, seen: &[i64]) -> Result<Vec<String>> {
        let d = self.d;
        let mut regs: HashMap<String, Option<String>> = HashMap::new();
        let mut out: Vec<String> = Vec::new();
        for &x in seen {
            let Some(i) = d.index_of(x) else { return key_err(&Py::Int(x)) };
            let t = d.text_at(i);
            if let Some((r, h)) = mov_from_import(t) {
                regs.insert(r.to_owned(), import_sym(d.pe, py_int(h, 16)?));
                continue;
            }
            let mut sym: Option<String> = None;
            if let Some(h) = call_or_jmp_maybe_ds(t) {
                let tgt = py_int(h, 16)?;
                sym = import_sym(d.pe, tgt);
                if sym.is_none() && t.starts_with("call") && !t.contains("ds:") {
                    sym = match self.a1_resolve(tgt) {
                        Ok(Resolved::Imp(y)) => Some(y),
                        Ok(Resolved::Fn(_)) => None,
                        Err(e) if e.kind == ErrorKind::Decode => None,
                        Err(e) => return Err(e),
                    };
                }
            }
            if let Some(r) = call_reg(t) {
                sym = regs.get(r).cloned().flatten();
            }
            if let Some(s) = sym.filter(|s| !s.is_empty())
                && signature(&s).is_some()
            {
                out.push(call_name(&s));
            }
        }
        Ok(out)
    }
}

/// The `visit` of `A1Lifter.lift`: a depth-first walk that reports a loop and lists the blocks
/// in reverse topological order.
fn visit(b: i64, succs: &IndexMap<i64, Vec<i64>>, mark: &mut HashMap<i64, u8>, order: &mut Vec<i64>) -> Result<()> {
    match mark.get(&b) {
        Some(1) => return dec(format!("loop at {}", py_hex(b))),
        Some(2) => return Ok(()),
        _ => {}
    }
    mark.insert(b, 1);
    let Some(ss) = succs.get(&b) else { return key_err(&Py::Int(b)) };
    let mut ss = ss.clone();
    ss.sort_unstable_by(|x, y| y.cmp(x));
    for s in ss {
        visit(s, succs, mark, order)?;
    }
    mark.insert(b, 2);
    order.push(b);
    Ok(())
}

/// Small string helper for the tag tests above.
trait EqAny {
    fn eq_any(&self, names: &[&str]) -> bool;
}

impl EqAny for str {
    fn eq_any(&self, names: &[&str]) -> bool {
        names.contains(&self)
    }
}
