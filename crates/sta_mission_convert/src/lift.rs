//! The lifter core (LUA-15d): the `Lifter` class of `tools/dsl_to_lua.py` up to the call
//! dispatch. Control-flow recovery (`switch_table`, `blocks`), the per-block lift with the
//! registers that live across blocks (`lift`), the instruction semantics (`step`), operands and
//! locations (`read`, `lvalue`, `glob`, `load`, `static_value`, `glob_off`), stores and the copy
//! of values that read a location about to be written (`store`, `snap`), arithmetic and branches
//! (`arith`, `branch`), the call-argument bookkeeping (`take`, `new_tmp`, `clobber`, `arg`) and the
//! x87 stack (`fpop`, `fread`, `x87`), plus the state (`State`, `Lifted`) and the small functions
//! around them (`d_immediate`, `reads_location`).
//!
//! # The intermediate form
//!
//! [`Lifter::lift`] answers a [`Lifted`] per function: the blocks `(leader address, statements)`
//! with the cross-block register stores in place, the internal functions called, the script
//! steps added, the engine calls counted, the error text that stopped the function (`None` when
//! it lifted), the instruction, loop and parameter counts and the type notes
//! (`loctypes`, `floatlocs`, `itype`, `vectors`, `litype`). Expressions and statements are the
//! Python's tuples ([`Py`]); the shapes are listed in the comment above `Lifted` in
//! `dsl_to_lua.py` and unchanged here. LUA-15h consumes it: its worklist (`lift_mission`) reads
//! `calls` and `steps`, its renderer reads `blocks`. [`Lifted::canonical`] is the text form the
//! parity test compares with the Python's.
//!
//! # Where the cut is
//!
//! `step` dispatches `call` instructions through [`Lifter::call_fn`]. The Python's `call` (and
//! `known_call`, `vector_call`, `typed`, `by_pointer`, `special`) is LUA-15h's. Until it lands,
//! the field holds [`call_model`], which implements the paths of `Lifter.call` that need
//! nothing but the core (the debug stack check, `_chkstk`, `_ftol`, a direct call of an internal
//! function) and models every other call (a recognised runtime helper, a `std::vector` member, an
//! engine method through a vtable) as one opaque call that takes everything pushed so far. It
//! keeps the machine going so the rest of a function is lifted, and it is the model the parity
//! test runs on both sides. LUA-15h replaces the field with the full `call`, which can call
//! [`call_model`]'s pieces ([`Lifter::take`], [`Lifter::new_tmp`], [`Lifter::clobber`],
//! [`Lifter::arg`], [`Lifter::snap`]).
//!
//! The Armada I front end (LUA-15e) overrides about 25 of the Python methods in a subclass.
//! Here the methods are public and non-virtual, [`Lifter::game`] says which game the lifter
//! serves, and the override points branch on it (nothing in this module does yet).
//!
//! # Errors
//!
//! A fault the Python raises as `DecodeError` stops the function: it is caught in
//! [`Lifter::lift`] and stored in [`Lifted::error`]. A malformed text (`IndexError`, `ValueError`
//! ...) is a raw [`DecodeError`] (built with [`DecodeError::python`]: the Python exception's
//! class and CPython's text) that propagates out of `lift`, as the Python's run aborts.
//!
//! # Known differences from CPython (documented, not emulated)
//!
//! - The hand-written matchers in `dsl::pat` use ASCII classes for `\d` and `char::is_alphanumeric`
//!   for `\w`; Python's differ on some non-ASCII digits and marks. Listing text is ASCII.
//! - Python raises `RecursionError` on a very deep expression tree (`contains`, `repr`, hashing);
//!   the Rust recursion has no such limit (and a thread's stack is its only bound). Only a
//!   pathological chain of register-only arithmetic could reach it.

use std::collections::{BTreeSet, HashMap, HashSet};

use indexmap::{IndexMap, IndexSet};

use crate::algebra::{branch_pred, contains, false_, fbinop, lv_value, negate, simplify, true_, truth};
use crate::dsl::{is_reg32, is_setcc, jcc, pat, reg8, Dsl};
use crate::format::py_hex;
use crate::msvc::VAR_KINDS;
use crate::pyval::{py_floordiv, py_int, py_split_ws_once, py_strip, Py};
use crate::{tup, DecodeError, ErrorKind, Result};

pub(crate) fn dec<T>(msg: String) -> Result<T> {
    Err(DecodeError::new(msg))
}

/// A fault of one of Python's own exceptions (class and CPython's text).
pub(crate) fn py_err<T>(class: &'static str, msg: impl Into<String>) -> Result<T> {
    Err(DecodeError::python(class, msg))
}

/// `IndexError: list index out of range`.
pub(crate) fn index_err<T>() -> Result<T> {
    py_err("IndexError", "list index out of range")
}

/// `KeyError` for a dict miss: the text is the key's `repr`.
pub(crate) fn key_err<T>(key: &Py) -> Result<T> {
    py_err("KeyError", key.repr())
}

/// Python's `seq[i]` with a possibly negative index on a list of length `len`.
pub(crate) fn pyidx(len: usize, i: i64) -> Result<usize> {
    let j = if i < 0 { i + len as i64 } else { i };
    if j < 0 || j as usize >= len {
        return index_err();
    }
    Ok(j as usize)
}

/// `x[0]` of a tuple as a string, `""` for a head that is not one (every `== "kind"` is false).
pub(crate) fn tg(v: &Py) -> Result<&str> {
    Ok(v.tag()?.unwrap_or(""))
}

/// `a, b = [x.strip() for x in ops.split(",", 1)]`.
pub(crate) fn split2(ops: &str) -> Result<(String, String)> {
    let mut it = ops.splitn(2, ',');
    let a = it.next().unwrap_or("");
    match it.next() {
        Some(b) => Ok((py_strip(a).to_owned(), py_strip(b).to_owned())),
        None => py_err("ValueError", "not enough values to unpack (expected 2, got 1)"),
    }
}

/// The float whose 32 bits a constant holds, widened to a double
/// (`struct.unpack("<f", struct.pack("<I", n & 0xFFFFFFFF))[0]`).
pub fn float_of_bits(n: &Py) -> Result<f64> {
    match n {
        Py::Int(n) => Ok(f64::from(f32::from_bits((*n & 0xFFFF_FFFF) as u32))),
        Py::Bool(b) => Ok(f64::from(f32::from_bits(u32::from(*b)))),
        other => py_err(
            "TypeError",
            format!("unsupported operand type(s) for &: '{}' and 'int'", other.type_name()),
        ),
    }
}

/// The integer `v` is, for a site where Python does arithmetic on it with an int (`op`):
/// `TypeError` with CPython's text for anything else.
pub(crate) fn int_of(v: &Py, op: &str) -> Result<i64> {
    match v.as_int() {
        Some(n) => Ok(n),
        None => py_err("TypeError", format!("unsupported operand type(s) for {op}: '{}' and 'int'", v.type_name())),
    }
}

/// Python hashes a tuple that holds a list by raising `TypeError`.
pub(crate) fn hashable(v: &Py) -> Result<()> {
    match v {
        Py::List(_) => py_err("TypeError", "unhashable type: 'list'"),
        Py::Tuple(x) => x.iter().try_for_each(hashable),
        _ => Ok(()),
    }
}

/// Which game the lifter serves (the A1 front end branches on it at its override points).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Game {
    #[default]
    A2,
    A1,
}

/// `Lifted`: the result for one function. See the module docs.
#[derive(Debug, Clone, Default)]
pub struct Lifted {
    pub addr: i64,
    /// `[(label address, [statements])]`.
    pub blocks: Vec<(i64, Vec<Py>)>,
    /// Internal functions called.
    pub calls: Vec<i64>,
    /// The step functions added (their entry addresses).
    pub steps: Vec<i64>,
    /// Engine call name to count.
    pub engine_calls: IndexMap<String, i64>,
    pub error: Option<String>,
    pub instructions: usize,
    pub params: i64,
    pub loops: i64,
    /// Stack slot offset to the class of the object it holds.
    pub loctypes: IndexMap<Py, Py>,
    /// Stack slot offsets the function uses as floats (x87 DWORD operands).
    pub floatlocs: BTreeSet<i64>,
    /// Temp to the `std::vector` an iterator or element reference points into.
    pub itype: IndexMap<Py, Py>,
    /// `std::vector`s whose constructor the function ran.
    pub vectors: IndexSet<Py>,
    /// Stack slot offset to the same, for a slot that holds one.
    pub litype: IndexMap<Py, Py>,
    /// The Armada I front end's additions to `Lifted` (LUA-15e); empty for Armada II.
    pub a1: crate::a1::LiftedExtra,
}

impl Lifted {
    pub fn new(addr: i64) -> Self {
        Lifted { addr, ..Lifted::default() }
    }

    /// The comparison form of the parity test: one text per function, every value as Python's
    /// `repr` (so a Python driver prints the same text from its own object). Sets are sorted.
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        let line = |out: &mut String, k: &str, v: String| {
            out.push(' ');
            out.push_str(k);
            out.push(' ');
            out.push_str(&v);
            out.push('\n');
        };
        out.push_str(&format!("func {}\n", py_hex(self.addr)));
        let error = match &self.error {
            Some(e) => Py::Str(e.clone()),
            None => Py::None,
        };
        line(&mut out, "error", error.repr());
        line(&mut out, "instructions", self.instructions.to_string());
        line(&mut out, "params", self.params.to_string());
        line(&mut out, "loops", self.loops.to_string());
        let ints = |v: &[i64]| Py::List(v.iter().map(|x| Py::Int(*x)).collect()).repr();
        line(&mut out, "calls", ints(&self.calls));
        line(&mut out, "steps", ints(&self.steps));
        let pairs = |it: Vec<(Py, Py)>| Py::List(it.into_iter().map(|(k, v)| Py::Tuple(vec![k, v])).collect()).repr();
        line(
            &mut out,
            "engine_calls",
            pairs(self.engine_calls.iter().map(|(k, v)| (Py::from(k), Py::Int(*v))).collect()),
        );
        line(&mut out, "loctypes", pairs(self.loctypes.iter().map(|(k, v)| (k.clone(), v.clone())).collect()));
        let fl: Vec<i64> = self.floatlocs.iter().copied().collect();
        line(&mut out, "floatlocs", ints(&fl));
        line(&mut out, "itype", pairs(self.itype.iter().map(|(k, v)| (k.clone(), v.clone())).collect()));
        let mut vs: Vec<Py> = self.vectors.iter().cloned().collect();
        vs.sort_by_key(Py::repr);
        line(&mut out, "vectors", Py::List(vs).repr());
        line(&mut out, "litype", pairs(self.litype.iter().map(|(k, v)| (k.clone(), v.clone())).collect()));
        for (b, stmts) in &self.blocks {
            out.push_str(&format!(" block {}\n", py_hex(*b)));
            for s in stmts {
                out.push_str("  ");
                out.push_str(&s.repr());
                out.push('\n');
            }
        }
        out
    }
}

/// The symbolic machine state of one block (`State`). Registers hold an expression; a register
/// that is present with `Py::None` (unknown) differs from an absent one (never written here).
#[derive(Debug, Clone)]
pub struct State {
    /// Register to expression, in the order they were first written.
    pub r: IndexMap<String, Py>,
    /// The words pushed so far (arguments of the next call), oldest first.
    pub args: Vec<Py>,
    pub marks: IndexMap<String, usize>,
    /// The x87 stack, top last.
    pub fpu: Vec<Py>,
    /// The value an x87-returning call left in `st(0)` (`None` when none).
    pub fret: Py,
    /// The last compare: `("test", v)`, `("cmp", x, y)`, `("ftest", fcmp, mask)`.
    pub flags: Py,
    pub fsw: Py,
    pub fcmp: Py,
    pub body: bool,
    pub sub_esp: bool,
    /// Location (lvalue) to the value it holds in this block.
    pub mem: IndexMap<Py, Py>,
    pub escaped: IndexSet<Py>,
    /// Registers read before this block wrote them.
    pub reg_in: BTreeSet<String>,
    pub restored: bool,
    /// How many times `flags` was assigned (`st.flags is f[3]` of the Armada I front end is
    /// "not assigned since"); every assignment goes through [`State::set_flags`].
    pub flags_gen: u64,
    /// The Armada I front end's additions to `State` (LUA-15e); unused for Armada II.
    pub a1: crate::a1::StateExtra,
}

impl State {
    /// `st.flags = v`.
    pub fn set_flags(&mut self, v: Py) {
        self.flags = v;
        self.flags_gen += 1;
    }

    pub fn new() -> Self {
        State {
            r: IndexMap::new(),
            args: Vec::new(),
            marks: IndexMap::new(),
            fpu: Vec::new(),
            fret: Py::None,
            flags: Py::None,
            fsw: Py::None,
            fcmp: Py::None,
            body: false,
            sub_esp: false,
            mem: IndexMap::new(),
            escaped: IndexSet::new(),
            reg_in: BTreeSet::new(),
            restored: false,
            flags_gen: 0,
            a1: crate::a1::StateExtra::default(),
        }
    }

    /// `st.r.get(reg)`: `None` for an absent register and for one holding `None`.
    pub fn rget(&self, reg: &str) -> Py {
        self.r.get(reg).cloned().unwrap_or(Py::None)
    }
}

impl Default for State {
    fn default() -> Self {
        State::new()
    }
}

/// What `step` returns: `None` (go on), `"ret"`, `("switch", index, targets)`,
/// `("jcc", target, predicate)` or `("jmp", target, None)`.
#[derive(Debug, Clone, PartialEq)]
pub enum Flow {
    Next,
    Ret,
    Switch(Py, Vec<i64>),
    Jcc(i64, Py),
    Jmp(i64),
}

/// The call dispatch of `step` (`Lifter.call`): `(lifter, state, block statements, address of the
/// call, operand text, index of the call in the listing)`.
pub type CallFn = fn(&mut Lifter<'_, '_>, &mut State, &mut Vec<Py>, i64, &str, usize) -> Result<Flow>;

/// `lv_value`-style helper: `d_immediate(d, n)`: the value of an immediate that may be an
/// address (a string, a global, a code address) or a plain 32-bit constant.
pub fn d_immediate(d: &Dsl<'_>, n: i64) -> Result<Py> {
    let sec = d.section_of(n);
    let sec = sec.as_deref();
    if matches!(sec, Some(".rdata" | ".data")) {
        match d.cstr(n) {
            Ok(s) => {
                if !s.is_empty() && s.chars().all(|c| (' '..='~').contains(&c) && c != '\u{7f}' || c == '\n' || c == '\t') {
                    return Ok(tup!["str", s]);
                }
            }
            Err(e) if e.kind != ErrorKind::Decode => return Err(e),
            Err(_) => {}
        }
        if let Some((name, _, off)) = d.global_at(n) {
            return Ok(tup![
                "addr",
                if off == 0 { tup!["obj", name] } else { tup!["elem", name, tup!["const", off / 4]] }
            ]);
        }
        return Ok(tup!["addr", tup!["gvar", n]]);
    }
    if sec == Some(".bss") {
        if let Some((name, _, _)) = d.global_at(n) {
            return Ok(tup!["addr", tup!["obj", name]]);
        }
        return Ok(tup!["addr", tup!["gvar", n]]);
    }
    if sec == Some(".text") {
        return Ok(tup!["code", n]);
    }
    Ok(tup!["const", if n >= 1 << 31 { n - (1 << 32) } else { n }])
}

/// A test over expression nodes (the lambdas of the Python).
pub type LocationTest = Box<dyn Fn(&Py) -> Result<bool>>;

/// `reads_location(lv)`: a test for expressions that read memory location `lv`.
pub fn reads_location(lv: &Py) -> Result<LocationTest> {
    let k = tg(lv)?.to_owned();
    let lv = lv.clone();
    Ok(match k.as_str() {
        "loc" => Box::new(move |e| Ok(*e == lv || e.tag_is("locidx")?)),
        "locidx" => Box::new(|e| e.tag_in(&["loc", "locidx"])),
        "obj" | "elem" | "vfield" => {
            let name = lv.item(1)?.clone();
            Box::new(move |e| Ok(e.tag_in(&["obj", "elem", "vfield"])? && *e.item(1)? == name))
        }
        "gvar" => {
            let addr = lv.item(1)?.clone();
            Box::new(move |e| Ok(e.tag_is("glob")? && *e.item(1)? == addr))
        }
        "param" => Box::new(move |e| Ok(*e == lv)),
        _ => Box::new(|_| Ok(false)),
    })
}

/// The lifter. See the module docs for the cut with LUA-15h and the A1 front end.
pub struct Lifter<'d, 'a> {
    pub d: &'d Dsl<'a>,
    /// A `jmp` address of a two-level jump table to (the byte load's address, the index register).
    pub byte_index: IndexMap<i64, (i64, String)>,
    /// Switch jump address to its targets (reset by every [`Lifter::blocks`]).
    pub switches: IndexMap<i64, Vec<i64>>,
    /// The function being lifted (`self.cur`).
    pub cur: Lifted,
    /// The number of the last temp (`ntmp[0]`).
    pub ntmp: i64,
    pub game: Game,
    /// The `call` instruction's handler; [`call_model`] until LUA-15h's lands.
    pub call_fn: CallFn,
    /// The Armada I front end's fields of `A1Lifter` (LUA-15e); unused for Armada II.
    pub a1: crate::a1::LifterExtra,
}

impl<'d, 'a> Lifter<'d, 'a> {
    /// `Lifter.__init__`.
    pub fn new(d: &'d Dsl<'a>) -> Self {
        Lifter {
            d,
            byte_index: IndexMap::new(),
            switches: IndexMap::new(),
            cur: Lifted::default(),
            ntmp: 0,
            game: Game::A2,
            call_fn: call_model,
            a1: crate::a1::LifterExtra::default(),
        }
    }

    pub(crate) fn ins(&self, i: usize) -> Result<(i64, &'d str)> {
        let d = self.d;
        match d.ins.get(i) {
            Some((a, t)) => Ok((i64::from(*a), t.as_str())),
            None => index_err(),
        }
    }

    // ------------------------------------------------------------ CFG

    /// `Lifter.switch_table`: targets of a `switch` jump table: `cmp idx,N; ja default; ...;
    /// jmp [idx*4+table]` (the compiler bounds the index first).
    pub fn switch_table(&mut self, i: usize) -> Result<Vec<i64>> {
        let d = self.d;
        let (a, t) = self.ins(i)?;
        let Some((_, table)) = pat::jmp_table(t) else {
            return dec(format!("indirect jump `{t}` at {}", py_hex(a)));
        };
        let mut n: Option<i64> = None;
        for j in (i.saturating_sub(8)..i).rev() {
            if let Some(h) = pat::cmp_bound(d.text_at(j)) {
                n = Some(py_int(h, 16)?);
                break;
            }
        }
        let n = match n {
            Some(n) if n <= 256 => n,
            _ => return dec(format!("jump table without a bound at {}", py_hex(a))),
        };
        let base = py_int(table, 16)?;
        let word = |k: i64| -> Result<i64> {
            let rva = base + 4 * k - i64::from(d.pe.base());
            let o = u32::try_from(rva).ok().and_then(|r| d.pe.off(r));
            let Some(o) = o else {
                return dec(format!("jump table outside the file at {}", py_hex(a)));
            };
            match d.pe.data().get(o..o + 4) {
                Some(b) => Ok(i64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))),
                None => Err(crate::image::short_buffer(o, 4, d.pe.data().len())),
            }
        };
        // Two-level table (`xor r,r; mov rl,BYTE PTR [idx+bytes]; jmp [r*4+targets]`): the bytes
        // map the bounded index to an entry of the target table.
        if i >= 2
            && d.text_at(i - 2).starts_with("xor")
            && let Some((_, reg, bytes)) = pat::byte_load(d.text_at(i - 1))
        {
            let bytes_at = py_int(bytes, 16)?;
            let rva = bytes_at - i64::from(d.pe.base());
            let o = u32::try_from(rva).ok().and_then(|r| d.pe.off(r));
            let Some(o) = o else {
                return dec(format!("jump table outside the file at {}", py_hex(a)));
            };
            let data = d.pe.data();
            let end = (o + n as usize + 1).min(data.len());
            let idx: Vec<i64> = data[o.min(end)..end].iter().map(|&b| i64::from(b)).collect();
            self.byte_index.insert(a, (d.addr_at(i - 1), reg.to_owned()));
            return idx.into_iter().map(word).collect();
        }
        (0..=n).map(word).collect()
    }

    /// `Lifter.blocks`: the block leaders (sorted) and every instruction address reached from
    /// `start`.
    pub fn blocks(&mut self, start: i64) -> Result<(Vec<i64>, HashSet<i64>)> {
        let d = self.d;
        self.switches = IndexMap::new();
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
                let (x, t) = self.ins(i)?;
                seen.insert(x);
                let mn = match py_split_ws_once(t).first() {
                    Some(w) => {
                        // `t.split()[0]`: the first whitespace-separated word
                        *w
                    }
                    None => return index_err(),
                };
                if let Some((_, h)) = pat::jcc_abs(t) {
                    let tgt = py_int(h, 16)?;
                    leaders.insert(tgt);
                    todo.push(tgt);
                    if mn == "jmp" {
                        if i + 1 < d.ins.len() {
                            leaders.insert(d.addr_at(i + 1));
                        }
                        break;
                    }
                    let nxt = self.ins(i + 1)?.0;
                    leaders.insert(nxt);
                    todo.push(nxt);
                    break;
                }
                if mn == "jmp" {
                    let tbl = self.switch_table(i)?;
                    self.switches.insert(x, tbl.clone());
                    for tgt in tbl {
                        leaders.insert(tgt);
                        todo.push(tgt);
                    }
                    if i + 1 < d.ins.len() {
                        leaders.insert(d.addr_at(i + 1));
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
        let mut out: Vec<i64> = leaders.intersection(&seen).copied().collect();
        out.sort_unstable();
        Ok((out, seen))
    }

    // ------------------------------------------------------------ lift

    /// `Lifter.lift`: the function at `start`, lifted block by block. A `DecodeError` stops the
    /// function (stored in [`Lifted::error`]); a raw error propagates.
    pub fn lift(&mut self, start: i64) -> Result<Lifted> {
        if self.game == Game::A1 {
            return self.a1_lift(start); // `A1Lifter.lift` replaces this method
        }
        self.cur = Lifted::new(start);
        match self.lift_body(start) {
            Ok(()) => {}
            Err(e) if e.kind == ErrorKind::Decode => self.cur.error = Some(e.message),
            Err(e) => return Err(e),
        }
        Ok(std::mem::take(&mut self.cur))
    }

    // The `k` of the register-store pass moves while the fixed range of statements is walked,
    // as the Python's `range(k, len(stmts))` does (its end is not re-evaluated).
    #[allow(clippy::mut_range_bound)]
    fn lift_body(&mut self, start: i64) -> Result<()> {
        let d = self.d;
        let (leaders, seen) = self.blocks(start)?;
        self.cur.instructions = seen.len();
        // stack slots the function uses as floats (x87 DWORD operands)
        for x in &seen {
            let Some(i) = d.index_of(*x) else { return key_err(&Py::Int(*x)) };
            if let Some(o) = pat::float_slot(d.text_at(i)) {
                self.cur.floatlocs.insert(py_int(o, 16)?);
            }
        }
        let lset: HashSet<i64> = leaders.iter().copied().collect();
        self.ntmp = 0;
        let mut reg_in: HashMap<i64, BTreeSet<String>> = HashMap::new();
        let mut ends: HashMap<i64, State> = HashMap::new();
        let mut succs: HashMap<i64, Vec<i64>> = HashMap::new();
        for &b in &leaders {
            let mut st = State::new();
            st.body = b != start;
            if b == start {
                st.r.insert("ecx".to_owned(), tup!["this"]);
            }
            let mut stmts: Vec<Py> = Vec::new();
            let Some(mut i) = d.index_of(b) else { return key_err(&Py::Int(b)) };
            let mut first = true;
            let mut last_a;
            loop {
                let (a, t) = self.ins(i)?;
                last_a = a;
                if !first && lset.contains(&a) {
                    succs.insert(b, vec![a]);
                    break;
                }
                first = false;
                let parts = py_split_ws_once(t);
                let Some(&mn) = parts.first() else {
                    return index_err();
                };
                let ops = parts.get(1).copied().unwrap_or("");
                match self.step(&mut st, &mut stmts, a, mn, ops, i, start == b)? {
                    Flow::Ret => {
                        succs.insert(b, vec![]);
                        break;
                    }
                    Flow::Switch(idx, targets) => {
                        stmts.push(Py::Tuple(vec![
                            Py::from("switch"),
                            idx,
                            Py::List(targets.iter().map(|t| Py::Int(*t)).collect()),
                        ]));
                        succs.insert(b, targets);
                        break;
                    }
                    Flow::Jcc(tgt, pred) => {
                        if tgt <= a {
                            self.cur.loops += 1;
                        }
                        stmts.push(tup!["if", simplify(&pred)?, tgt]);
                        let nxt = self.ins(i + 1)?.0;
                        succs.insert(b, vec![tgt, nxt]);
                        i += 1;
                        if lset.contains(&self.ins(i)?.0) {
                            break;
                        }
                        continue;
                    }
                    Flow::Jmp(tgt) => {
                        if tgt <= a {
                            self.cur.loops += 1;
                        }
                        stmts.push(tup!["goto", tgt]);
                        succs.insert(b, vec![tgt]);
                        break;
                    }
                    Flow::Next => {}
                }
                i += 1;
            }
            if !st.args.is_empty() {
                return dec(format!("arguments pushed across a block end at {}", py_hex(last_a)));
            }
            if !st.fpu.is_empty() {
                return dec(format!("x87 value live across a block end at {}", py_hex(last_a)));
            }
            reg_in.insert(b, st.reg_in.clone());
            ends.insert(b, st);
            self.cur.blocks.push((b, stmts));
        }
        // registers live across blocks: store them where a successor reads them
        for bi in 0..self.cur.blocks.len() {
            let b = self.cur.blocks[bi].0;
            let st = &ends[&b];
            let mut need: BTreeSet<String> = BTreeSet::new();
            if let Some(ss) = succs.get(&b) {
                for s in ss {
                    if let Some(r) = reg_in.get(s) {
                        need.extend(r.iter().cloned());
                    }
                }
            }
            let mut pairs: Vec<Py> = Vec::new();
            for r in &need {
                let v = st.rget(r);
                if v.is_none() || v == tup!["reg", r] {
                    continue;
                }
                pairs.push(tup![tup!["reg", r], v]);
            }
            if pairs.is_empty() {
                continue;
            }
            let mut stmts = std::mem::take(&mut self.cur.blocks[bi].1);
            let mut k = stmts.len();
            while k > 0 && tg(&stmts[k - 1])?.eq_any(&["goto", "if", "return", "switch"]) {
                k -= 1;
            }
            // a branch after the stores must still see the old register values
            let n0 = stmts.len();
            for j in k..n0 {
                let sj = stmts[j].clone();
                if tg(&sj)?.eq_any(&["if", "switch"]) && contains(sj.item(1)?, &|e| e.tag_is("reg"))? {
                    self.ntmp += 1;
                    let t = tup!["tmp", self.ntmp];
                    stmts.insert(k, tup!["set", &t, sj.item(1)?]);
                    k += 1;
                    let mut ns = vec![sj.item(0)?.clone(), t];
                    ns.extend_from_slice(sj.tail()?.get(1..).unwrap_or(&[]));
                    stmts[j + 1] = Py::Tuple(ns);
                }
            }
            // all registers are stored at once (a multiple assignment)
            stmts.insert(k, Py::Tuple(vec![Py::from("multiset"), Py::List(pairs)]));
            self.cur.blocks[bi].1 = stmts;
        }
        Ok(())
    }

    // ------------------------------------------------------------ step

    /// `Lifter.step` (`A1Lifter.step` for Armada I, which falls back to [`Lifter::step_base`]).
    #[allow(clippy::too_many_arguments)]
    pub fn step(&mut self, st: &mut State, stmts: &mut Vec<Py>, a: i64, mn: &str, ops: &str, i: usize, entry: bool) -> Result<Flow> {
        if self.game == Game::A1 {
            return self.a1_step(st, stmts, a, mn, ops, i, entry);
        }
        self.step_base(st, stmts, a, mn, ops, i, entry)
    }

    /// `Lifter.step`: the effect of one instruction on the state and the block's statements.
    #[allow(clippy::too_many_arguments)]
    pub fn step_base(
        &mut self,
        st: &mut State,
        stmts: &mut Vec<Py>,
        a: i64,
        mn: &str,
        ops: &str,
        i: usize,
        _entry: bool,
    ) -> Result<Flow> {
        let d = self.d;
        if matches!(mn, "int3" | "nop" | "fnclex" | "fwait" | "wait" | "cld") {
            return Ok(Flow::Next);
        }
        if mn == "ret" {
            let v = match st.r.get("eax") {
                None => self.read(st, "eax", a)?,
                Some(v) => v.clone(),
            };
            stmts.push(tup!["return", v]);
            return Ok(Flow::Ret);
        }
        if mn == "push" {
            if ops == "ebp" || (matches!(ops, "ebx" | "esi" | "edi") && !st.body) {
                return Ok(Flow::Next); // the frame and the saved registers of a prologue
            }
            let v = self.read(st, ops, a)?;
            st.args.push(v);
            return Ok(Flow::Next);
        }
        if mn == "pop" {
            if !st.args.is_empty() {
                st.args.pop();
            } else if is_reg32(ops) && st.body && !st.restored {
                st.restored = true; // `push ecx ... rep stos ... pop ecx`: the prologue keeps `this`
            } else if is_reg32(ops) {
                st.r.insert(ops.to_owned(), Py::None);
            }
            return Ok(Flow::Next);
        }
        if mn == "rep" {
            if !st.body {
                st.body = true; // the debug prologue's 0xcc fill
                st.args = Vec::new();
                return Ok(Flow::Next);
            }
            return dec(format!("`rep {ops}` at {}", py_hex(a)));
        }
        if mn == "leave" {
            return Ok(Flow::Next);
        }
        if mn == "lea" {
            let (dst, src) = split2(ops)?;
            if dst == "edi" && !st.body {
                return Ok(Flow::Next);
            }
            if is_reg32(&dst) {
                let lv = self.lvalue(st, &src, a)?;
                st.r.insert(dst, tup!["addr", &lv]);
                st.escaped.insert(lv);
                return Ok(Flow::Next);
            }
            return dec(format!("`lea {ops}` at {}", py_hex(a)));
        }
        if (mn == "sub" || mn == "add") && ops.starts_with("esp,") {
            let n = match ops.split(',').nth(1) {
                Some(s) => py_int(s, 16)?,
                None => return index_err(),
            };
            if mn == "add" {
                let k = py_floordiv(n, 4)?;
                // `st.args[:-k] if k and k <= len(st.args) else ([] if k else st.args)`
                if k != 0 && k <= st.args.len() as i64 {
                    // a negative k keeps `args[:-k]`, the first |k| words (all of them when fewer)
                    let keep = if k > 0 { st.args.len() - k as usize } else { st.args.len().min((-k) as usize) };
                    st.args.truncate(keep);
                } else if k != 0 {
                    st.args = Vec::new();
                }
            } else if i + 1 < d.ins.len() && d.text_at(i + 1) == "push   ebx" {
                st.body = false; // a frame prologue (also behind a patched entry)
            } else if st.body {
                if n % 4 != 0 {
                    return dec(format!("`sub esp,{}` at {}", py_hex(n), py_hex(a)));
                }
                for _ in 0..py_floordiv(n, 4)? {
                    st.args.push(tup!["stackslot"]);
                }
                st.sub_esp = true;
            }
            return Ok(Flow::Next);
        }
        if mn == "mov" && self.byte_index.values().any(|b| b.0 == a) {
            return Ok(Flow::Next); // the byte of a two-level jump table: the switch reads the index itself
        }
        if mn == "mov" && ops.contains("fs:0x0") {
            // the SEH frame chain: the link is pushed and restored, nothing reads it
            let (dst, _src) = split2(ops)?;
            if is_reg32(&dst) {
                st.r.insert(dst, tup!["seh"]);
            }
            return Ok(Flow::Next);
        }
        if mn == "mov" {
            let (dst, src) = split2(ops)?;
            if matches!(dst.as_str(), "esi" | "edi" | "ebx") && src == "esp" {
                st.marks.insert(dst, st.args.len()); // the debug stack check (nested calls use several)
                return Ok(Flow::Next);
            }
            if dst == "ebp" || dst == "esp" {
                return Ok(Flow::Next);
            }
            if is_reg32(&dst) {
                if src == "esp" && st.sub_esp {
                    st.r.insert(dst, tup!["stacktop"]);
                    return Ok(Flow::Next);
                }
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
        if mn == "movsx" || mn == "movzx" {
            let (dst, src) = split2(ops)?;
            let src = pat::strip_byte_word_ptr(&src).to_owned();
            if is_reg32(&dst) {
                let v = self.read(st, &src, a)?;
                st.r.insert(dst, v);
                return Ok(Flow::Next);
            }
            return dec(format!("`{mn} {ops}` at {}", py_hex(a)));
        }
        if mn == "call" {
            let f = self.call_fn;
            return f(self, st, stmts, a, ops, i);
        }
        if mn == "test" {
            let (x, y) = split2(ops)?;
            if x == y || (matches!(x.as_str(), "al" | "eax") && matches!(y.as_str(), "al" | "eax")) {
                let v = self.read(st, &x, a)?;
                st.set_flags(tup!["test", v]);
                return Ok(Flow::Next);
            }
            if x == "ah" && st.fsw.truthy() {
                let f = tup!["ftest", &st.fsw, py_int(&y, 16)?];
                st.set_flags(f);
                return Ok(Flow::Next);
            }
            if pat::is_hex0x(&y) {
                let v = self.read(st, &x, a)?;
                st.set_flags(tup!["test", tup!["arith", "&", v, tup!["const", py_int(&y, 16)?]]]);
                return Ok(Flow::Next);
            }
            return dec(format!("`test {ops}` at {}", py_hex(a)));
        }
        if mn == "cmp" {
            let (x, y) = split2(ops)?;
            if y == "esp" {
                return Ok(Flow::Next);
            }
            let vx = self.read(st, &x, a)?;
            let vy = self.read(st, &y, a)?;
            st.set_flags(tup!["cmp", vx, vy]);
            return Ok(Flow::Next);
        }
        if jcc(mn).is_some() {
            if st.flags.is_none() {
                return dec(format!("branch without a compare at {}", py_hex(a)));
            }
            let tgt = py_int(ops, 16)?;
            return Ok(Flow::Jcc(tgt, self.branch(mn, &st.flags, a)?));
        }
        if mn == "jmp" {
            if self.switches.contains_key(&a) {
                let reg = pat::dword_index4(ops);
                if let Some((_, reg)) = self.byte_index.get(&a).cloned() {
                    let idx = self.read(st, &reg, a)?;
                    return Ok(Flow::Switch(idx, self.switches[&a].clone()));
                }
                let Some(reg) = reg else {
                    return py_err("AttributeError", "'NoneType' object has no attribute 'group'");
                };
                let idx = self.read(st, reg, a)?;
                return Ok(Flow::Switch(idx, self.switches[&a].clone()));
            }
            return Ok(Flow::Jmp(py_int(ops, 16)?));
        }
        if is_setcc(mn) {
            let dst = reg8(py_strip(ops));
            let (Some(dst), true) = (dst, st.flags.truthy()) else {
                return dec(format!("`{mn} {ops}` at {}", py_hex(a)));
            };
            let v = self.branch(&format!("j{}", &mn[3..]), &st.flags, a)?;
            st.r.insert(dst.to_owned(), v);
            return Ok(Flow::Next);
        }
        if matches!(mn, "xor" | "and" | "or" | "add" | "sub" | "imul" | "shl" | "sar" | "shr" | "adc" | "sbb") {
            self.arith(st, stmts, a, mn, ops)?;
            return Ok(Flow::Next);
        }
        if matches!(mn, "inc" | "dec" | "neg" | "not") {
            let x = py_strip(ops);
            let v = self.read(st, x, a)?;
            let nv = match mn {
                "inc" => tup!["arith", "+", v, tup!["const", 1]],
                "dec" => tup!["arith", "-", v, tup!["const", 1]],
                "neg" => tup!["neg", v],
                _ => tup!["bnot", v],
            };
            st.set_flags(tup!["test", &nv]);
            self.write(st, stmts, x, nv, a)?;
            return Ok(Flow::Next);
        }
        if mn == "cdq" {
            let v = st.rget("eax");
            st.r.insert("edx".to_owned(), tup!["sign", v]);
            return Ok(Flow::Next);
        }
        if mn == "idiv" || mn == "div" {
            let x = self.read(st, ops, a)?;
            let n = st.rget("eax");
            st.r.insert("eax".to_owned(), tup!["arith", "//", &n, &x]);
            st.r.insert("edx".to_owned(), tup!["arith", "%", &n, &x]);
            return Ok(Flow::Next);
        }
        // x87
        self.x87(st, stmts, a, mn, ops)?;
        Ok(Flow::Next)
    }

    // ------------------------------------------------------------ operands

    /// `Lifter.read` (`A1Lifter.read` for Armada I): the value of an operand.
    pub fn read(&mut self, st: &mut State, op: &str, a: i64) -> Result<Py> {
        if self.game == Game::A1 {
            return self.a1_read(st, op, a);
        }
        self.read_base(st, op, a)
    }

    /// `Lifter.read`: the value of an operand.
    pub fn read_base(&mut self, st: &mut State, op: &str, a: i64) -> Result<Py> {
        let d = self.d;
        let op = py_strip(op);
        if is_reg32(op) {
            return Ok(self.read_reg(st, op));
        }
        if let Some(reg) = reg8(op) {
            return Ok(self.read_reg(st, reg));
        }
        if pat::is_hex0x(op) {
            return d_immediate(d, py_int(op, 16)?);
        }
        if pat::is_neg_hex0x(op) {
            return Ok(tup!["const", py_int(op, 16)?]);
        }
        if pat::is_digits(op) {
            return Ok(tup!["const", py_int(op, 10)?]);
        }
        let lv = self.lvalue(st, op, a)?;
        self.load(st, &lv, a)
    }

    fn read_reg(&self, st: &mut State, reg: &str) -> Py {
        match st.r.get(reg) {
            None => {
                st.reg_in.insert(reg.to_owned());
                tup!["reg", reg]
            }
            Some(v) => v.clone(),
        }
    }

    /// `Lifter.lvalue` (`A1Lifter.lvalue` for Armada I): a memory operand as a location.
    pub fn lvalue(&mut self, st: &mut State, op: &str, a: i64) -> Result<Py> {
        if self.game == Game::A1 {
            return self.a1_lvalue(st, op, a, false);
        }
        self.lvalue_base(st, op, a)
    }

    /// `Lifter.lvalue`: a memory operand as a location.
    pub fn lvalue_base(&mut self, st: &mut State, op: &str, a: i64) -> Result<Py> {
        let op = pat::strip_ptr(py_strip(op));
        if let Some(h) = pat::ds_abs(op) {
            return self.glob(py_int(h, 16)?, a);
        }
        if let Some(o) = pat::ebp_disp(op) {
            let off = py_int(o, 16)?;
            if off >= 8 {
                return Ok(tup!["param", py_floordiv(off - 8, 4)?]);
            }
            return Ok(tup!["loc", off]);
        }
        if let Some(r) = pat::reg_ind(op) {
            let base = self.read(st, r, a)?;
            return Ok(tup!["deref", base, 0]);
        }
        if let Some((r, o)) = pat::reg_disp(op) {
            let base = self.read(st, r, a)?;
            return Ok(tup!["deref", base, py_int(o, 16)?]);
        }
        if let Some((r, h)) = pat::reg4_table(op) {
            let base = self.glob(py_int(h, 16)?, a)?;
            if base.tag_in(&["obj", "elem"])? {
                let name = base.item(1)?.clone();
                let k = if base.tag_is("elem")? { base.item(2)?.clone() } else { tup!["const", 0] };
                let mut idx = self.read(st, r, a)?;
                if k != tup!["const", 0] {
                    idx = tup!["arith", "+", idx, k];
                }
                return Ok(tup!["elem", name, idx]);
            }
            return dec(format!("indexed access to {} at {}", base.to_str(), py_hex(a)));
        }
        if let Some((r, o)) = pat::ebp_index4(op) {
            let idx = self.read(st, r, a)?;
            return Ok(tup!["locidx", py_int(o, 16)?, idx]);
        }
        if pat::reg_plus_index4(op) {
            return dec(format!("indexed access `{op}` at {}", py_hex(a)));
        }
        dec(format!("operand `{op}` at {}", py_hex(a)))
    }

    /// `Lifter.glob` (`A1Lifter.glob` for Armada I): a global address as a location.
    pub fn glob(&mut self, addr: i64, a: i64) -> Result<Py> {
        if self.game == Game::A1 {
            return self.a1_glob(addr, a);
        }
        self.glob_base(addr, a)
    }

    /// `Lifter.glob`: a global address as a location.
    pub fn glob_base(&mut self, addr: i64, a: i64) -> Result<Py> {
        let d = self.d;
        if let Some((name, kind, off)) = d.global_at(addr) {
            if matches!(kind, "intarray" | "floatarray") {
                return Ok(tup!["elem", name, tup!["const", off / 4]]);
            }
            if kind == "Vector3" {
                return Ok(tup!["vfield", name, off / 4]);
            }
            return Ok(if off == 0 { tup!["obj", name] } else { tup!["elem", name, tup!["const", off / 4]] });
        }
        let sec = d.section_of(addr);
        if sec.is_none() || matches!(sec.as_deref(), Some(".data" | ".rdata" | ".bss" | ".idata")) {
            d.note_static(addr, "word");
            return Ok(tup!["gvar", addr]);
        }
        dec(format!("global in {} at {} ({})", sec.unwrap_or_default(), py_hex(addr), py_hex(a)))
    }

    pub(crate) fn data_named(&self, name: &Py) -> Result<&'d crate::dsl::DataItem> {
        let d = self.d;
        d.data
            .values()
            .find(|v| Py::Str(v.name.clone()) == *name)
            .ok_or_else(|| DecodeError::python("StopIteration", ""))
    }

    /// `Lifter.load` (`A1Lifter.load` for Armada I): the value read from a location.
    pub fn load(&mut self, st: &mut State, lv: &Py, a: i64) -> Result<Py> {
        if self.game == Game::A1 {
            return self.a1_load(st, lv, a);
        }
        self.load_base(st, lv, a)
    }

    /// `Lifter.load`: the value read from a location.
    pub fn load_base(&mut self, st: &mut State, lv: &Py, a: i64) -> Result<Py> {
        let k = tg(lv)?;
        if k == "obj" {
            let name = lv.item(1)?;
            let kind = self.data_named(name)?.kind;
            let name_s = name.to_str();
            if VAR_KINDS.contains(&kind) {
                return Ok(tup!["varptr", name, kind]);
            }
            if kind == "ScriptInterface" {
                return Ok(tup!["si"]);
            }
            if kind == "character" {
                return Ok(tup!["world"]);
            }
            if kind == "int" || kind == "float" {
                return Ok(tup!["obj", name]);
            }
            return dec(format!("read of {kind} {name_s} at {}", py_hex(a)));
        }
        if k == "locidx" {
            return Ok(lv.clone());
        }
        if matches!(k, "elem" | "vfield" | "gvar" | "param") {
            if (k == "param" || k == "gvar") && st.mem.contains_key(lv) {
                return Ok(st.mem[lv].clone());
            }
            if k == "gvar" {
                return self.static_value(int_of(lv.item(1)?, "-")?);
            }
            return Ok(lv.clone());
        }
        if k == "loc" {
            if let Some(v) = st.mem.get(lv) {
                return Ok(v.clone());
            }
            if let Some(t) = self.cur.loctypes.get(lv.item(1)?) {
                return Ok(tup!["objref", lv, t]);
            }
            return Ok(lv.clone());
        }
        if k == "deref" {
            let (base, off) = (lv.item(1)?.clone(), int_of(lv.item(2)?, "//")?);
            if base.is_none() {
                return dec(format!("load through an unknown pointer at {}", py_hex(a)));
            }
            let bk = tg(&base)?;
            if off == 0 && matches!(bk, "varptr" | "si" | "world" | "iiie" | "script" | "objref") {
                return Ok(tup!["vt", &base]);
            }
            if bk == "vt" {
                return Ok(tup!["method", base.item(1)?, py_floordiv(off, 4)?]);
            }
            if bk == "addr" {
                let inner = base.item(1)?.clone();
                if off == 0 {
                    return self.load(st, &inner, a);
                }
                if tg(&inner)? == "loc" {
                    let l = tup!["loc", int_of(inner.item(1)?, "+")? + off];
                    return self.load(st, &l, a);
                }
                if tg(&inner)? == "obj" {
                    let g = self.glob_off(&inner, off, a)?;
                    return self.load(st, &g, a);
                }
            }
            let key: Option<Py> = if bk == "tmp" {
                hashable(&base)?;
                self.cur.itype.get(&base).cloned()
            } else if bk == "loc" {
                self.cur.litype.get(base.item(1)?).cloned()
            } else {
                None
            };
            if let Some(key) = key.filter(Py::truthy) {
                if off != 0 {
                    return dec(format!("std::vector element read at offset {} at {}", py_hex(off), py_hex(a)));
                }
                return Ok(tup!["vget", key, &base]); // an element of a std::vector, through an iterator
            }
            if matches!(bk, "tmp" | "loc" | "reg" | "glob" | "obj" | "field") && matches!(off, 0 | 4 | 8) {
                // a vector the engine returned by reference
                return Ok(tup!["field", &base, off / 4]);
            }
            return dec(format!("load [{}+{}] at {}", base.item(0)?.to_str(), py_hex(off), py_hex(a)));
        }
        dec(format!("load of {k} at {}", py_hex(a)))
    }

    /// `Lifter.static_value`: the value of a global the DLL does not export (never a silent
    /// nil): the word the image holds when the code only reads it (constants, never-written
    /// statics), else a slot of the script's `G` table that the image's word starts.
    pub fn static_value(&mut self, addr: i64) -> Result<Py> {
        let d = self.d;
        let w = d.dword(addr);
        if let Some(w) = w {
            let class = d.static_class(addr);
            if class == "rodata" || class == "const" {
                d.set_static(addr, class);
                let sec = d.section_of(w);
                return if matches!(sec.as_deref(), Some(".rdata" | ".data" | ".bss" | ".text")) {
                    d_immediate(d, w)
                } else {
                    Ok(tup!["const", if w >= 1 << 31 { w - (1 << 32) } else { w }])
                };
            }
        }
        d.set_static(addr, "word");
        Ok(tup!["glob", addr])
    }

    /// `Lifter.glob_off`: the location `off` bytes into the exported datum `lv`.
    pub fn glob_off(&mut self, lv: &Py, off: i64, a: i64) -> Result<Py> {
        let name = lv.item(1)?.clone();
        let addr = self
            .d
            .data
            .iter()
            .find(|(_, v)| Py::Str(v.name.clone()) == name)
            .map(|(x, _)| *x)
            .ok_or_else(|| DecodeError::python("StopIteration", ""))?;
        self.glob(addr + off, a)
    }

    /// `Lifter.store` (`A1Lifter.store` for Armada I): a store to the operand text `dst`.
    pub fn store(&mut self, st: &mut State, stmts: &mut Vec<Py>, dst: &str, v: Py, a: i64) -> Result<()> {
        if self.game == Game::A1 {
            return self.a1_store(st, stmts, dst, v, a);
        }
        self.store_base(st, stmts, dst, v, a)
    }

    /// `Lifter.store`: a store to the operand text `dst`.
    pub fn store_base(&mut self, st: &mut State, stmts: &mut Vec<Py>, dst: &str, v: Py, a: i64) -> Result<()> {
        if let Some(off) = pat::esp_slot(py_strip(dst)) {
            // an argument slot reserved by `push reg` / `sub esp` and filled in place
            let k = match off {
                Some(h) => py_floordiv(py_int(h, 16)?, 4)?,
                None => 0,
            };
            if k as usize >= st.args.len() {
                return dec(format!("store to an unreserved stack slot at {}", py_hex(a)));
            }
            let n = st.args.len();
            st.args[n - 1 - k as usize] = v;
            return Ok(());
        }
        let mut lv = self.lvalue(st, dst, a)?;
        if tg(&lv)? == "deref" {
            let (base, off) = (lv.item(1)?.clone(), int_of(lv.item(2)?, "//")?);
            if base.truthy() && tg(&base)? == "stacktop" {
                // a by-value struct argument built in place
                let idx = st.args.len() as i64 - 1 - py_floordiv(off, 4)?;
                if 0 <= idx && (idx as usize) < st.args.len() {
                    st.args[idx as usize] = v;
                    return Ok(());
                }
            }
            if base.truthy() && tg(&base)? == "addr" {
                let inner = base.item(1)?.clone();
                lv = if tg(&inner)? == "loc" {
                    tup!["loc", int_of(inner.item(1)?, "+")? + off]
                } else if tg(&inner)? == "obj" && off != 0 {
                    self.glob_off(&inner, off, a)?
                } else {
                    inner
                };
            } else {
                let what = if base.truthy() { base.item(0)?.to_str() } else { base.to_str() };
                return dec(format!("store through {what} at {}", py_hex(a)));
            }
        }
        if tg(&lv)? == "obj" {
            let g = self.data_named(lv.item(1)?)?;
            if VAR_KINDS.contains(&g.kind) {
                stmts.push(tup!["bindvar", lv.item(1)?, g.kind, &v]);
                return Ok(());
            }
            if g.kind == "ScriptInterface" || g.kind == "character" {
                stmts.push(tup!["comment", format!("{} is set by the engine", lv.item(1)?.to_str())]);
                return Ok(());
            }
            if g.kind != "int" && g.kind != "float" {
                return dec(format!("store to {} {} at {}", g.kind, lv.item(1)?.to_str(), py_hex(a)));
            }
        }
        let lk = tg(&lv)?.to_owned();
        if lk == "locidx" {
            let doomed: Vec<Py> = st.mem.keys().filter(|x| matches!(x.tag(), Ok(Some("loc")))).cloned().collect();
            for x in doomed {
                st.mem.shift_remove(&x);
            }
        }
        if lk == "loc" && !v.is_none() && v.tag_is("objref")? {
            self.cur.loctypes.insert(lv.item(1)?.clone(), v.item(2)?.clone());
        }
        if lk == "loc" && !v.is_none() && {
            hashable(&v)?;
            self.cur.itype.contains_key(&v)
        } {
            let it = self.cur.itype[&v].clone();
            self.cur.litype.insert(lv.item(1)?.clone(), it);
        } else if lk == "loc" && !(v.truthy() && v.tag_is("arith")? && v.tail()?.get(1..).unwrap_or(&[]).contains(&tup!["loc", lv.item(1)?])) {
            self.cur.litype.shift_remove(lv.item(1)?); // the slot holds something else now (`++it` keeps it)
        }
        if matches!(lk.as_str(), "loc" | "param" | "gvar") {
            let keep = !v.is_none()
                && v.tag_in(&["const", "float", "str", "code", "obj", "addr", "varptr", "si", "world", "tmp", "script", "objref"])?;
            let val = if keep { v.clone() } else { lv_value(&lv)? };
            st.mem.insert(lv.clone(), val);
        }
        if v.is_none() {
            return dec(format!("store of an unknown value at {}", py_hex(a)));
        }
        let mut v = v;
        if v.tag_is("const")? && self.is_float_slot(&lv)? {
            v = tup!["float", float_of_bits(v.item(1)?)?];
        }
        let test = reads_location(&lv)?;
        let v = self.snap(st, stmts, &*test, v)?;
        stmts.push(tup!["set", &lv, v]);
        Ok(())
    }

    /// `Lifter.snap`: before a write, values the block still holds that read the written memory
    /// are copied to temps, so they keep the value they were read with. Returns `value` (the
    /// value about to be written, `Py::None` when there is none) with the same treatment.
    /// (`A1Lifter.snap` for Armada I.)
    pub fn snap(&mut self, st: &mut State, stmts: &mut Vec<Py>, test: &dyn Fn(&Py) -> Result<bool>, value: Py) -> Result<Py> {
        if self.game == Game::A1 {
            return self.a1_snap(st, stmts, test, value);
        }
        self.snap_base(st, stmts, test, value)
    }

    /// `Lifter.snap` itself.
    pub fn snap_base(&mut self, st: &mut State, stmts: &mut Vec<Py>, test: &dyn Fn(&Py) -> Result<bool>, value: Py) -> Result<Py> {
        let mut cache: IndexMap<Py, Py> = IndexMap::new();
        let keys: Vec<String> = st.r.keys().cloned().collect();
        for k in keys {
            let v = st.r[&k].clone();
            let nv = self.snap_fix(&v, test, &mut cache, stmts)?;
            st.r.insert(k, nv);
        }
        let mut args = Vec::with_capacity(st.args.len());
        for v in std::mem::take(&mut st.args) {
            args.push(self.snap_fix(&v, test, &mut cache, stmts)?);
        }
        st.args = args;
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
                // a slot that holds its own value needs no copy
                let v = st.mem[&k].clone();
                let nv = self.snap_fix(&v, test, &mut cache, stmts)?;
                st.mem.insert(k, nv);
            }
        }
        // (flags are not kept: the compiler compares right before it branches)
        self.snap_fix(&value, test, &mut cache, stmts)
    }

    fn snap_fix(
        &mut self,
        v: &Py,
        test: &dyn Fn(&Py) -> Result<bool>,
        cache: &mut IndexMap<Py, Py>,
        stmts: &mut Vec<Py>,
    ) -> Result<Py> {
        if v.is_none() || !matches!(v, Py::Tuple(_)) || !contains(v, test)? {
            return Ok(v.clone());
        }
        if v.tag_in(&["tmp", "addr", "code", "const", "float", "str"])? {
            return Ok(v.clone());
        }
        hashable(v)?;
        if !cache.contains_key(v) {
            self.ntmp += 1;
            let t = tup!["tmp", self.ntmp];
            stmts.push(tup!["set", &t, v]);
            cache.insert(v.clone(), t);
        }
        Ok(cache[v].clone())
    }

    /// `Lifter.is_float_slot`: a location the compiler stores a float into as raw bits.
    pub fn is_float_slot(&self, lv: &Py) -> Result<bool> {
        let k = tg(lv)?;
        if k == "vfield" {
            return Ok(true);
        }
        if k == "loc" {
            // `lv[1] in f.floatlocs`: a non-integer is simply not in the set
            return Ok(lv.item(1)?.as_int().is_some_and(|o| self.cur.floatlocs.contains(&o)));
        }
        if k == "obj" || k == "elem" {
            let name = lv.item(1)?;
            let g = self.d.data.values().find(|x| Py::Str(x.name.clone()) == *name);
            return Ok(g.is_some_and(|g| matches!(g.kind, "float" | "floatarray")));
        }
        Ok(false)
    }

    /// `Lifter.write` (`A1Lifter.write` for Armada I): a write to a register or a memory operand.
    pub fn write(&mut self, st: &mut State, stmts: &mut Vec<Py>, x: &str, v: Py, a: i64) -> Result<()> {
        if self.game == Game::A1 {
            return self.a1_write(st, stmts, x, v, a);
        }
        self.write_base(st, stmts, x, v, a)
    }

    /// `Lifter.write`: a write to a register or a memory operand.
    pub fn write_base(&mut self, st: &mut State, stmts: &mut Vec<Py>, x: &str, v: Py, a: i64) -> Result<()> {
        if is_reg32(x) {
            st.r.insert(x.to_owned(), v);
        } else if let Some(r) = reg8(x) {
            st.r.insert(r.to_owned(), v);
        } else {
            self.store(st, stmts, x, v, a)?;
        }
        Ok(())
    }

    /// `Lifter.arith` (`A1Lifter.arith` for Armada I).
    pub fn arith(&mut self, st: &mut State, stmts: &mut Vec<Py>, a: i64, mn: &str, ops: &str) -> Result<()> {
        if self.game == Game::A1 {
            return self.a1_arith(st, stmts, a, mn, ops);
        }
        self.arith_base(st, stmts, a, mn, ops)
    }

    /// `Lifter.arith`.
    pub fn arith_base(&mut self, st: &mut State, stmts: &mut Vec<Py>, a: i64, mn: &str, ops: &str) -> Result<()> {
        let (x, y) = split2(ops)?;
        if mn == "xor" && x == y {
            return self.write(st, stmts, &x, tup!["const", 0], a);
        }
        if mn == "imul" && y.contains(',') {
            let (y1, y2) = split2(&y)?;
            let v1 = self.read(st, &y1, a)?;
            let v2 = self.read(st, &y2, a)?;
            return self.write(st, stmts, &x, tup!["arith", "*", v1, v2], a);
        }
        let vx = self.read(st, &x, a)?;
        let vy = self.read(st, &y, a)?;
        let v = if mn == "and" && vy == tup!["const", 0xff] {
            vx.clone()
        } else if mn == "and" && x == "esp" {
            return Ok(());
        } else {
            let op = match mn {
                "xor" => "~",
                "and" => "&",
                "or" => "|",
                "add" => "+",
                "sub" => "-",
                "imul" => "*",
                "shl" => "<<",
                "sar" => ">>",
                "shr" => ">>>",
                "adc" => "+c",
                _ => "-c",
            };
            if mn == "sbb" && x == y {
                tup!["sbb", &st.flags]
            } else if mn == "adc" {
                return dec(format!("`adc {ops}` at {}", py_hex(a)));
            } else {
                tup!["arith", op, &vx, &vy]
            }
        };
        st.set_flags(if mn == "sub" { tup!["cmp", &vx, &vy] } else { tup!["test", &v] });
        self.write(st, stmts, &x, v, a)
    }

    /// `Lifter.branch`: the predicate under which the conditional jump `mn` is taken, given the
    /// flags the last compare left.
    pub fn branch(&self, mn: &str, flags: &Py, a: i64) -> Result<Py> {
        let fk = tg(flags)?;
        if fk == "test" {
            let v = flags.item(1)?;
            if v.is_none() {
                return dec(format!("test of an unknown value at {}", py_hex(a)));
            }
            match mn {
                "je" => return negate(&truth(v)?),
                "jne" => return truth(v),
                "js" | "jl" => return Ok(tup!["cmp", "<", v, tup!["const", 0]]),
                "jns" | "jge" => return Ok(tup!["cmp", ">=", v, tup!["const", 0]]),
                "jle" => return Ok(tup!["cmp", "<=", v, tup!["const", 0]]),
                "jg" => return Ok(tup!["cmp", ">", v, tup!["const", 0]]),
                _ => {}
            }
        }
        if fk == "test" && matches!(mn, "ja" | "jbe" | "jb" | "jae") {
            let v = flags.item(1)?;
            return Ok(match mn {
                "ja" => truth(v)?,
                "jbe" => negate(&truth(v)?)?,
                "jb" => false_(),
                _ => true_(),
            });
        }
        if fk == "cmp" {
            let (x, y) = (flags.item(1)?, flags.item(2)?);
            if x.is_none() || y.is_none() {
                return dec(format!("compare of an unknown value at {}", py_hex(a)));
            }
            let op = match jcc(mn) {
                Some(op) => op,
                None => return key_err(&Py::from(mn)),
            };
            if matches!(mn, "jb" | "jae" | "jbe" | "ja") {
                return Ok(tup!["cmp", op, tup!["u", x], tup!["u", y]]);
            }
            if matches!(mn, "js" | "jns") {
                return Ok(tup!["cmp", op, tup!["arith", "-", x, y], tup!["const", 0]]);
            }
            return Ok(tup!["cmp", op, x, y]);
        }
        if fk == "ftest" {
            return branch_pred(mn, flags, a);
        }
        dec(format!("`{mn}` after {} at {}", flags.item(0)?.to_str(), py_hex(a)))
    }

    // ------------------------------------------------------------ call bookkeeping

    /// `Lifter.take`: the `n` words a call takes off the pushed arguments, the first parameter
    /// first.
    pub fn take(&mut self, st: &mut State, n: i64, a: i64) -> Result<Vec<Py>> {
        let len = st.args.len();
        if n < 0 {
            // `st.args[len - n:]` is empty and `st.args[:len - n]` is everything
            if st.args.is_empty() {
                st.sub_esp = false;
            }
            return Ok(Vec::new());
        }
        if n as usize > len {
            return dec(format!("call takes {n} arguments, {len} pushed at {}", py_hex(a)));
        }
        let n = n as usize;
        let mut args: Vec<Py> = st.args[len - n..].to_vec();
        st.args.truncate(len - n);
        if st.args.is_empty() {
            st.sub_esp = false;
        }
        args.reverse();
        Ok(args)
    }

    /// `Lifter.new_tmp`: `("tmp", n)` for the next number.
    pub fn new_tmp(&mut self) -> Py {
        self.ntmp += 1;
        tup!["tmp", self.ntmp]
    }

    /// `Lifter.clobber`: what a call destroys: the caller-saved registers, and the memory it may
    /// write through (slots whose address was handed out, globals).
    pub fn clobber(&mut self, st: &mut State) {
        for x in ["eax", "ecx", "edx"] {
            st.r.insert(x.to_owned(), Py::None);
        }
        let doomed: Vec<Py> = st
            .mem
            .keys()
            .filter(|lv| st.escaped.contains(*lv) || matches!(lv.tag(), Ok(Some("gvar"))))
            .cloned()
            .collect();
        for lv in doomed {
            st.mem.shift_remove(&lv);
        }
    }

    /// `Lifter.arg`: a call argument must be known and no unfilled struct slot.
    pub fn arg(&self, v: &Py, a: i64) -> Result<Py> {
        if v.is_none() {
            return dec(format!("unknown call argument at {}", py_hex(a)));
        }
        if v.tag_in(&["stackslot", "stacktop"])? {
            return dec(format!("struct argument not filled at {}", py_hex(a)));
        }
        Ok(v.clone())
    }

    // ------------------------------------------------------------ x87

    /// `Lifter.fpop`: pop the top of the x87 stack (or the value an x87-returning call left).
    pub fn fpop(&mut self, st: &mut State, a: i64) -> Result<Py> {
        if let Some(v) = st.fpu.pop() {
            return Ok(v);
        }
        if !st.fret.is_none() {
            return Ok(std::mem::replace(&mut st.fret, Py::None));
        }
        dec(format!("x87 stack empty at {}", py_hex(a)))
    }

    /// `Lifter.fread`: an x87 load operand as a value.
    pub fn fread(&mut self, st: &mut State, op: &str, a: i64, integer: bool) -> Result<Py> {
        let d = self.d;
        if let Some((size, h)) = pat::sized_ds(op) {
            let addr = py_int(h, 16)?;
            let g = d.global_at(addr);
            if size == "QWORD" && g.is_none() && matches!(d.static_class(addr), "rodata" | "const") && !integer {
                return Ok(tup!["float", d.f64(addr)?]); // a double nothing writes: its image value
            }
            if g.is_some() || matches!(d.section_of(addr).as_deref(), Some(".data" | ".bss")) {
                return self.read(st, op, a);
            }
            if integer {
                return dec(format!("integer load from {} at {}", py_hex(addr), py_hex(a)));
            }
            return Ok(tup!["float", if size == "DWORD" { d.f32(addr)? } else { d.f64(addr)? }]);
        }
        if let Some(k) = pat::st_n(op) {
            if st.fpu.is_empty() && !st.fret.is_none() {
                let v = self.fpop(st, a)?;
                st.fpu.push(v);
            }
            if k as usize >= st.fpu.len() {
                return dec(format!("x87 st({k}) is empty at {}", py_hex(a)));
            }
            return Ok(st.fpu[st.fpu.len() - 1 - k as usize].clone());
        }
        if op == "st" {
            return self.fread(st, "st(0)", a, false);
        }
        let mut v = self.read(st, op, a)?;
        if !integer && op.starts_with("DWORD PTR") && v.truthy() && v.tag_is("const")? {
            v = tup!["float", float_of_bits(v.item(1)?)?];
        }
        Ok(v)
    }

    /// `Lifter.x87`: the x87 instructions.
    pub fn x87(&mut self, st: &mut State, stmts: &mut Vec<Py>, a: i64, mn: &str, ops: &str) -> Result<()> {
        if mn == "fld" || mn == "fild" {
            let v = self.fread(st, ops, a, mn == "fild")?;
            st.fpu.push(v);
            return Ok(());
        }
        if mn == "fld1" || mn == "fldz" {
            st.fpu.push(tup!["float", if mn == "fld1" { 1.0 } else { 0.0 }]);
            return Ok(());
        }
        if matches!(mn, "fstp" | "fst" | "fistp" | "fist") {
            let mut v = if mn.ends_with('p') {
                self.fpop(st, a)?
            } else {
                match st.fpu.last() {
                    Some(v) => v.clone(),
                    None => st.fret.clone(),
                }
            };
            if mn.starts_with("fi") {
                v = tup!["ftol", v];
            }
            if let Some(mut k) = pat::st_n(ops) {
                if k != 0 {
                    // st(k) := st(0) (then the pop above): st(k) counts from before the pop
                    if mn.ends_with('p') {
                        k -= 1;
                    }
                    if k as usize >= st.fpu.len() {
                        return dec(format!("`{mn} {ops}` on a short x87 stack at {}", py_hex(a)));
                    }
                    let n = st.fpu.len();
                    st.fpu[n - 1 - k as usize] = v;
                }
                return Ok(());
            }
            return self.store(st, stmts, ops, v, a);
        }
        if matches!(
            mn,
            "fadd" | "fsub" | "fmul" | "fdiv" | "fsubr" | "fdivr" | "faddp" | "fsubp" | "fmulp" | "fdivp" | "fsubrp" | "fdivrp" | "fiadd" | "fisub" | "fimul" | "fidiv"
        ) {
            let base = if mn.ends_with("rp") || matches!(mn, "faddp" | "fsubp" | "fmulp" | "fdivp") {
                mn.trim_end_matches('p')
            } else {
                mn
            };
            let base = base.replace("fi", "f");
            let op = match base.as_str() {
                "fadd" => "+",
                "fsub" => "-",
                "fmul" => "*",
                "fdiv" => "/",
                "fsubr" => "r-",
                "fdivr" => "r/",
                _ => return key_err(&Py::from(base.as_str())),
            };
            let m = pat::st_n_st(ops);
            if mn.ends_with('p') || ops.is_empty() {
                let k = m.unwrap_or(1);
                let y = self.fpop(st, a)?;
                if k > st.fpu.len() as i64 {
                    if !st.fret.is_none() && k == 1 {
                        let v = self.fpop(st, a)?;
                        st.fpu.push(v);
                    } else {
                        return dec(format!("`{mn} {ops}` on a short x87 stack at {}", py_hex(a)));
                    }
                }
                // st(k) = st(k) op st(0), pop
                let idx = pyidx(st.fpu.len(), -k)?;
                let nv = fbinop(op, &st.fpu[idx], &y)?;
                let idx = pyidx(st.fpu.len(), -k)?;
                st.fpu[idx] = nv;
                return Ok(());
            }
            if let Some(k) = m.filter(|k| *k != 0) {
                let top = self.fread(st, "st(0)", a, false)?;
                if k as usize >= st.fpu.len() {
                    return dec(format!("`{mn} {ops}` on a short x87 stack at {}", py_hex(a)));
                }
                let idx = st.fpu.len() - 1 - k as usize;
                let nv = fbinop(op, &st.fpu[idx], &top)?;
                st.fpu[idx] = nv;
                return Ok(());
            }
            let top = self.fpop(st, a)?;
            let v = if matches!(ops, "st(0),st" | "st,st(0)" | "st(0),st(0)" | "st,st") {
                fbinop(op, &top, &top)?
            } else {
                let o = self.fread(st, ops, a, false)?;
                fbinop(op, &top, &o)?
            };
            st.fpu.push(v);
            return Ok(());
        }
        if mn == "fchs" {
            let v = self.fpop(st, a)?;
            st.fpu.push(tup!["neg", v]);
            return Ok(());
        }
        if mn == "fabs" {
            let v = self.fpop(st, a)?;
            st.fpu.push(tup!["abs", v]);
            return Ok(());
        }
        if mn == "fsqrt" {
            let v = self.fpop(st, a)?;
            st.fpu.push(tup!["sqrt", v]);
            return Ok(());
        }
        if mn == "fxch" {
            if st.fpu.len() < 2 && !st.fret.is_none() {
                let v = self.fpop(st, a)?;
                st.fpu.insert(0, v);
            }
            if st.fpu.len() < 2 {
                return dec(format!("fxch on a short x87 stack at {}", py_hex(a)));
            }
            let n = st.fpu.len();
            st.fpu.swap(n - 1, n - 2);
            return Ok(());
        }
        if matches!(mn, "fcomp" | "fcom" | "fcompp" | "fucomp" | "fucompp" | "ficomp") {
            let (top, other);
            if mn == "fcompp" || mn == "fucompp" {
                top = self.fpop(st, a)?;
                other = self.fpop(st, a)?;
            } else {
                top = self.fread(st, "st(0)", a, false)?;
                other = self.fread(st, if ops.is_empty() { "st(1)" } else { ops }, a, false)?;
                if mn != "fcom" {
                    self.fpop(st, a)?;
                }
            }
            st.fcmp = tup![top, other];
            return Ok(());
        }
        if mn == "fnstsw" {
            if !st.fcmp.truthy() {
                return dec(format!("fnstsw without a compare at {}", py_hex(a)));
            }
            st.fsw = st.fcmp.clone();
            st.r.insert("eax".to_owned(), Py::None);
            return Ok(());
        }
        dec(format!("unsupported instruction `{mn} {ops}` at {}", py_hex(a)))
    }
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

/// The call model the lifter runs until LUA-15h's `call` replaces [`Lifter::call_fn`]: the paths
/// of `Lifter.call` that need only the core (a call of the stack-check routine, `_chkstk`,
/// `_ftol`, a direct call of an internal function), and one opaque call that takes everything
/// pushed so far for every other call (a recognised helper, a `std::vector` member, an indirect
/// call). A model for the parity test, not a claim about what the Python does with those calls.
pub fn call_model(lf: &mut Lifter<'_, '_>, st: &mut State, stmts: &mut Vec<Py>, a: i64, ops: &str, i: usize) -> Result<Flow> {
    let d = lf.d;
    if let Some(h) = ops.strip_prefix("0x").filter(|h| !h.is_empty() && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))) {
        let tgt = py_int(h, 16)?;
        if Some(tgt) == d.chkesp {
            return Ok(Flow::Next);
        }
        let body = d.entry(tgt)?;
        if d.is_chkstk(body) {
            st.body = false; // `mov eax,N; call _chkstk`: a large frame's prologue
            return Ok(Flow::Next);
        }
        if !d.is_ftol(body)? {
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
        if d.is_ftol(body)? {
            let v = lf.fpop(st, a)?;
            st.r.insert("eax".to_owned(), tup!["ftol", v]);
            st.r.insert("edx".to_owned(), Py::None);
            return Ok(Flow::Next);
        }
        let nxt = d.ins.get(i + 1).map_or("", |x| x.1.as_str());
        let args: Vec<Py> = if let Some(h) = pat::add_esp(nxt) {
            let n = (py_int(h, 16)? / 4) as usize;
            if n > st.args.len() {
                return dec(format!("internal call with {n} arguments, {} pushed at {}", st.args.len(), py_hex(a)));
            }
            st.args[st.args.len() - n..].iter().rev().cloned().collect()
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
    let n = st.args.len() as i64;
    let args = lf.take(st, n, a)?;
    let t = lf.new_tmp();
    stmts.push(tup!["set", &t, tup!["icall", tgt, Py::List(args)]]);
    lf.clobber(st);
    st.r.insert("eax".to_owned(), t.clone());
    st.fret = t;
    Ok(Flow::Next)
}
