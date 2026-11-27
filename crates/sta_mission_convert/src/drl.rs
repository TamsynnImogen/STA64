//! The Armada II rule-DLL decoder (LUA-15g): a line-for-line port of `tools/drl_to_lua.py`, the
//! part that is not shared core (the shared core is `pyval`, `format`, `msvc`, `algebra`).
//!
//! A rule DLL (`*R.drl`) holds no rule table, its rules are code: one function registers each rule
//! with the engine's rule engine (name, condition function, action function) and links it to the
//! variables that wake it; a condition reads variables and object handles and calls the engine
//! through `g_pSI`; an action calls one function of the mission DLL. The decoder follows that code
//! symbolically through every branch and reports what it does not recognise instead of guessing,
//! then writes a Lua scaffold ([`scaffold`]).
//!
//! # Input
//!
//! The decoder reads a listing, not bytes: one `(address, text)` per instruction ([`Ins`], the
//! contract of [`crate::disasm`]: the text as `objdump -d -M intel --no-show-raw-insn` prints it
//! after the tab, trimmed), plus the image ([`Image`]) for sections, imports, strings and floats.
//! The texts are expected to be ASCII and already stripped, as the Python's `disassemble` makes
//! them. The regular expressions of the Python are hand-coded here (the crate has no `regex`
//! dependency); where a pattern says `\w` the Unicode word class is used, `\s` is Python's
//! whitespace set.
//!
//! # Errors
//!
//! Python's `DecodeError` is [`DecodeError::new`] (kind `Decode`); any other exception the Python
//! can raise on a malformed listing (`ValueError` from `int()` or an unpacking, `IndexError`,
//! `KeyError`) is a raw error here, which the Python's `main` does not catch: it aborts the run.
//! `Unsupported` (a subclass of `DecodeError` in Python) is [`Failure::Unsupported`].
//!
//! # Names
//!
//! Each Python function's name is in the doc comment of its port. Python's `Drl._chkesp` is
//! [`Drl::new`] (it runs there), the module functions `merge`, `clone`, `fpeek`, `fpop` are
//! [`merge`], [`clone_state`], [`fpeek`], [`fpop`].

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use crate::algebra::{
    as_bool, binop, branch_pred, conj, expr_arg, false_, drl_jcc_op, negate, simplify, true_, typed_args,
};
use crate::disasm::Ins;
use crate::format::{lua_ident, lua_str, py_hex, render, Ctx};
use crate::image::Image;
use crate::map::{check_engine, engine_vtables, ExpectedSlot, Vtables};
use crate::msvc::{import_kind, msvc_params, plain_name, VAR_KINDS};
use crate::pyval::{first_max_key, py_int, py_isspace, py_lower, py_split_ws_once, py_strip, Py};
use crate::{tup, DecodeError, ErrorKind, Pe, Result};


/// `CHARACTER_GET_IIIE`: `character::GetIIIE()`.
pub const CHARACTER_GET_IIIE: i64 = 0;
/// `IIIE_CREATE_RULE`: `IIIE::create_rule(int (*)(), void (*)(), char *)`.
pub const IIIE_CREATE_RULE: i64 = 1;
/// `HOT_ADD_DEPENDENT_RULE`: `hot_type::add_dependent_rule(rule *)`.
pub const HOT_ADD_DEPENDENT_RULE: i64 = 1;
/// `HOT_VALUE`: `hot_int::operator int()` / `hot_float::operator float()`.
pub const HOT_VALUE: i64 = 8;

/// `ENGINE_CLASSES`: the classes whose vtables the decoder names.
pub const ENGINE_CLASSES: [&str; 7] = [
    "ScriptInterfaceImp",
    "character",
    "IIIE",
    "hot_int",
    "hot_float",
    "dirty_int",
    "timer_int",
];

/// The `want` table of `check_engine` of `drl_to_lua.py`.
pub const ENGINE_WANT: [ExpectedSlot<'static>; 7] = [
    ExpectedSlot { class: "character", slot: CHARACTER_GET_IIIE as usize, prefix: "?GetIIIE@character@@" },
    ExpectedSlot {
        class: "IIIE",
        slot: IIIE_CREATE_RULE as usize,
        prefix: "?create_rule@IIIE@@UAEPAVrule@@P6AHXZP6AXXZPAD@Z",
    },
    ExpectedSlot { class: "hot_int", slot: HOT_ADD_DEPENDENT_RULE as usize, prefix: "?add_dependent_rule@hot_type@@" },
    ExpectedSlot { class: "hot_int", slot: HOT_VALUE as usize, prefix: "??Bhot_int@@" },
    ExpectedSlot { class: "hot_float", slot: HOT_VALUE as usize, prefix: "??Bhot_float@@" },
    ExpectedSlot { class: "timer_int", slot: HOT_VALUE as usize, prefix: "??Bhot_int@@" },
    ExpectedSlot { class: "dirty_int", slot: HOT_VALUE as usize, prefix: "??Bhot_int@@" },
];

/// Why a rule DLL was not decoded: `Unsupported` is the Python's `Unsupported` (a DLL of another
/// shape, reported as skipped), `Error` its `DecodeError` (or a raw error, see [`DecodeError::kind`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Unsupported(String),
    Error(DecodeError),
}

impl From<DecodeError> for Failure {
    fn from(e: DecodeError) -> Self {
        Failure::Error(e)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Unsupported(s) => f.write_str(s),
            Failure::Error(e) => f.write_str(&e.message),
        }
    }
}

// ------------------------------------------------------------------ hand-coded patterns

fn is_w(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_reg32(s: &str) -> bool {
    matches!(s, "eax" | "ebx" | "ecx" | "edx" | "esi" | "edi" | "ebp" | "esp")
}

/// `REG8`.
fn reg8(s: &str) -> Option<&'static str> {
    match s {
        "al" | "ah" => Some("eax"),
        "cl" => Some("ecx"),
        "dl" => Some("edx"),
        "bl" => Some("ebx"),
        _ => None,
    }
}

/// `SETCC`.
fn setcc_op(mn: &str) -> Option<&'static str> {
    Some(match mn {
        "sete" => "==",
        "setne" => "~=",
        "setl" => "<",
        "setge" => ">=",
        "setle" => "<=",
        "setg" => ">",
        _ => return None,
    })
}

/// `[0-9a-f]+`, at least one.
fn hex_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `0x[0-9a-f]+` as the whole string.
fn is_hex0x(s: &str) -> bool {
    s.strip_prefix("0x").is_some_and(hex_digits)
}

/// The text after an optional one of the prefixes (the `(?:DWORD PTR |BYTE PTR )?` of the Python).
fn opt_prefix<'a>(s: &'a str, prefixes: &[&str]) -> &'a str {
    for p in prefixes {
        if let Some(r) = s.strip_prefix(p) {
            return r;
        }
    }
    s
}

/// `word\s+` at the start: the text after the whitespace.
fn word_ws<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let r = s.strip_prefix(word)?;
    let t = r.trim_start_matches(py_isspace);
    (t.len() < r.len()).then_some(t)
}

/// `re.match(word + r"\s+0x([0-9a-f]+)$", t)` (`jmp`, `call`): the `0x...` text.
fn m_word_hex<'a>(t: &'a str, word: &str) -> Option<&'a str> {
    word_ws(t, word).filter(|r| is_hex0x(r))
}

/// `re.match(r"j\w+\s+0x([0-9a-f]+)$", t)`: the `0x...` text.
fn m_jcc_target(t: &str) -> Option<&str> {
    let r = t.strip_prefix('j')?;
    let run = r.find(|c: char| !is_w(c)).unwrap_or(r.len());
    if run == 0 {
        return None;
    }
    let rest = r[run..].trim_start_matches(py_isspace);
    (rest.len() < r.len() - run && is_hex0x(rest)).then_some(rest)
}

/// `re.match(r"cmp\s+(e[sd]i),esp$", t)` (`allowed` lists the registers): the register.
fn m_cmp_esp<'a>(t: &'a str, allowed: &[&str]) -> Option<&'a str> {
    let r = word_ws(t, "cmp")?;
    let reg = r.strip_suffix(",esp")?;
    allowed.contains(&reg).then_some(reg)
}

/// `(?:prefix)?ds:(0x[0-9a-f]+)` as the whole operand: the `0x...` text.
fn m_ds<'a>(op: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    opt_prefix(op, prefixes).strip_prefix("ds:").filter(|r| is_hex0x(r))
}

/// `(?:prefix)?\[(\w+)\]` as the whole operand: the word.
fn m_bracket<'a>(op: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let inner = opt_prefix(op, prefixes).strip_prefix('[')?.strip_suffix(']')?;
    (!inner.is_empty() && inner.chars().all(is_w)).then_some(inner)
}

/// `(?:prefix)?\[ebp([+-]0x[0-9a-f]+)\]` as the whole operand: the signed `0x...` text.
fn m_ebp<'a>(op: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let inner = opt_prefix(op, prefixes).strip_prefix("[ebp")?.strip_suffix(']')?;
    let hex = inner.strip_prefix(['+', '-'])?;
    is_hex0x(hex).then_some(inner)
}

/// `(?:DWORD PTR )?\[(\w+)\+(0x[0-9a-f]+)\]` as the whole operand: the word and the `0x...` text.
fn m_base_plus(op: &str) -> Option<(&str, &str)> {
    let inner = opt_prefix(op, &["DWORD PTR "]).strip_prefix('[')?.strip_suffix(']')?;
    let (base, disp) = inner.split_once('+')?;
    (!base.is_empty() && base.chars().all(is_w) && is_hex0x(disp)).then_some((base, disp))
}

/// The Python `ops.split(",", 1)` stripped: two parts, else the `ValueError` of the unpacking.
fn split_pair(ops: &str) -> Result<(&str, &str)> {
    match ops.split_once(',') {
        Some((x, y)) => Ok((py_strip(x), py_strip(y))),
        None => Err(DecodeError::python("ValueError", "not enough values to unpack (expected 2, got 1)")),
    }
}

/// `t.split(None, 1)` as `(mn, ops)`; a blank text is the `IndexError` of `parts[0]`.
fn split_mn(t: &str) -> Result<(&str, &str)> {
    match py_split_ws_once(t).as_slice() {
        [] => Err(DecodeError::python("IndexError", "list index out of range")),
        [mn] => Ok((mn, "")),
        [mn, ops, ..] => Ok((mn, ops)),
    }
}

/// `int(s, 16)`.
fn hexval(s: &str) -> Result<i64> {
    py_int(s, 16)
}

/// The tag of a state value (`v[0]`), `""` for `None`.
fn head(v: &Py) -> &str {
    v.item(0).ok().and_then(Py::as_str).unwrap_or("")
}

/// The string at index 1 of a state value (a symbol name), `""` when there is none.
fn s1(v: &Py) -> &str {
    v.item(1).ok().and_then(Py::as_str).unwrap_or("")
}

// ------------------------------------------------------------------ the decoder state

/// The state of the condition walk: the Python's `st` dictionary. Values are `Py` (`None` is
/// `Py::None`; a register that was never set and one set to `None` read alike, as `st["r"].get`).
#[derive(Debug, Clone)]
pub struct St {
    pub r: IndexMap<String, Py>,
    pub local: IndexMap<i64, Py>,
    pub args: Vec<Py>,
    pub marks: IndexMap<String, usize>,
    pub fpu: Vec<Py>,
    pub flags: Py,
    pub fsw: Py,
    pub fcmp: Py,
    pub fret: Py,
    pub body: bool,
}

impl St {
    /// The start state of `condition()`: no registers, no locals, nothing pushed.
    pub fn new() -> Self {
        St {
            r: IndexMap::new(),
            local: IndexMap::new(),
            args: Vec::new(),
            marks: IndexMap::new(),
            fpu: Vec::new(),
            flags: Py::None,
            fsw: Py::None,
            fcmp: Py::None,
            fret: Py::None,
            body: false,
        }
    }

    /// `st["r"].get(name)`.
    pub fn reg(&self, name: &str) -> Py {
        self.r.get(name).cloned().unwrap_or(Py::None)
    }

    /// `st["local"].get(key)`.
    pub fn local_get(&self, key: i64) -> Py {
        self.local.get(&key).cloned().unwrap_or(Py::None)
    }
}

impl Default for St {
    fn default() -> Self {
        Self::new()
    }
}

/// `clone(st)`: a copy that shares nothing mutable with the original.
pub fn clone_state(st: &St) -> St {
    st.clone()
}

/// `fpop(st, a)`: pops st(0); with an empty x87 stack the pending call result is consumed once.
pub fn fpop(st: &mut St, a: i64) -> Result<Py> {
    if let Some(v) = st.fpu.pop() {
        return Ok(v);
    }
    if st.fret.is_none() {
        return Err(DecodeError::new(format!("x87 stack empty at {}", py_hex(a))));
    }
    // a call result is consumed once
    Ok(std::mem::replace(&mut st.fret, Py::None))
}

/// `fpeek(st, a)`: st(0) without popping; a pending call result is pushed first.
pub fn fpeek(st: &mut St, a: i64) -> Result<Py> {
    if st.fpu.is_empty() {
        let v = fpop(st, a)?;
        st.fpu.push(v);
    }
    Ok(st.fpu.last().cloned().expect("pushed above"))
}

/// A path condition: the predicates of the branches taken to a point.
pub type Path_ = Vec<Py>;

/// `merge(incoming)`: one state from the states that reach a join point. The path keeps what the
/// incoming paths share and joins the rest as an `or`; a register or local that differs between
/// the paths becomes `("boolv", ...)` when every value is the constant 0 or 1, else `("conflict",)`.
pub fn merge(mut incoming: Vec<(Path_, St)>) -> Result<(Path_, St)> {
    if incoming.len() == 1 {
        return Ok(incoming.remove(0));
    }
    let paths: Vec<&Path_> = incoming.iter().map(|(p, _)| p).collect();
    let mut n = 0usize;
    while paths.iter().all(|p| p.len() > n) && paths.iter().all(|p| p[n] == paths[0][n]) {
        n += 1;
    }
    let prefix: Vec<Py> = paths[0][..n].to_vec();
    let rests: Vec<Py> = paths.iter().map(|p| conj(&p[n..])).collect();
    let joined = simplify(&Py::Tuple(vec![Py::from("or"), Py::List(rests.clone())]))?;
    let path = if joined == true_() {
        prefix
    } else {
        let mut p = prefix;
        p.push(joined);
        p
    };
    let states: Vec<&St> = incoming.iter().map(|(_, s)| s).collect();
    let mut out = clone_state(states[0]);

    let pick = |vals: Vec<Py>| -> Result<Py> {
        if vals.iter().all(|v| *v == vals[0]) {
            return Ok(vals[0].clone());
        }
        let is_bit = |v: &Py| -> Result<bool> {
            if v.is_none() || !v.tag_is("const")? {
                return Ok(false);
            }
            let x = v.item(1)?;
            Ok(*x == Py::Int(0) || *x == Py::Int(1))
        };
        let mut all_bits = true;
        for v in &vals {
            if !is_bit(v)? {
                all_bits = false;
                break;
            }
        }
        if all_bits {
            let mut ones = Vec::new();
            for (r, v) in rests.iter().zip(&vals) {
                if *v.item(1)? == Py::Int(1) {
                    ones.push(r.clone());
                }
            }
            let joined = simplify(&Py::Tuple(vec![Py::from("or"), Py::List(ones)]))?;
            return Ok(tup!["boolv", joined]);
        }
        Ok(tup!["conflict"])
    };

    // first-seen order (a set of register names iterates by string hash)
    let mut keys: IndexMap<String, ()> = IndexMap::new();
    for s in &states {
        for k in s.r.keys() {
            keys.insert(k.clone(), ());
        }
    }
    let mut r = IndexMap::new();
    for k in keys.keys() {
        r.insert(k.clone(), pick(states.iter().map(|s| s.reg(k)).collect())?);
    }
    out.r = r;
    let mut lkeys: IndexMap<i64, ()> = IndexMap::new();
    for s in &states {
        for k in s.local.keys() {
            lkeys.insert(*k, ());
        }
    }
    let mut local = IndexMap::new();
    for k in lkeys.keys() {
        local.insert(*k, pick(states.iter().map(|s| s.local_get(*k)).collect())?);
    }
    out.local = local;
    let same = |f: &dyn Fn(&St) -> Py| -> Py {
        let vals: Vec<Py> = states.iter().map(|s| f(s)).collect();
        if vals.iter().all(|v| *v == vals[0]) { vals[0].clone() } else { Py::None }
    };
    out.flags = same(&|s| s.flags.clone());
    out.fsw = same(&|s| s.fsw.clone());
    out.fcmp = same(&|s| s.fcmp.clone());
    out.fret = same(&|s| s.fret.clone());
    if !states.iter().all(|s| s.args == states[0].args) {
        return Err(DecodeError::new("paths join with different args"));
    }
    if !states.iter().all(|s| s.fpu == states[0].fpu) {
        return Err(DecodeError::new("paths join with different fpu"));
    }
    Ok((path, out))
}

/// What one instruction does to the walk: `None` (go on), `"ret"`, or `(kind, target, pred)`.
#[derive(Debug, Clone, PartialEq)]
pub enum Next {
    Fall,
    Ret,
    /// `("jmp", target, None)`.
    Jmp(i64),
    /// `("jcc", target, pred)`.
    Jcc(i64, Py),
}

// ------------------------------------------------------------------ the decoder

/// A rule the registration code creates: name, condition address, action address.
pub type Created = (String, i64, i64);

/// The variables each rule (by creation index) depends on.
pub type Deps = HashMap<usize, Vec<String>>;

/// `Drl`: one decoded rule DLL over its image and its listing.
pub struct Drl<I: Image> {
    pub pe: I,
    /// The engine vtable names (`engine or {}`).
    pub engine: Vtables,
    pub ins: Vec<(i64, String)>,
    /// Address to the position in [`Drl::ins`] (the last one wins, as a dict comprehension).
    pub at: HashMap<i64, usize>,
    /// The function entry thunks at the start of `.text`: address to jump target.
    pub thunks: IndexMap<i64, i64>,
    pub chkesp: Option<i64>,
    /// The `ScriptInterface` vtable slots the conditions call.
    pub engine_slots: BTreeSet<i64>,
    pub dlls: Vec<String>,
}

impl<I: Image> Drl<I> {
    /// `Drl.__init__` and `Drl._chkesp`: the image (`Pe(path)` of the Python) and the listing
    /// (`disassemble(path)`) are made by the caller. `DecodeError("no code")` for an empty listing.
    pub fn new(pe: I, listing: Vec<Ins>, engine: Option<&Vtables>) -> Result<Self> {
        let ins: Vec<(i64, String)> = listing.into_iter().map(|(a, t)| (i64::from(a), t)).collect();
        let mut at = HashMap::new();
        for (i, (a, _)) in ins.iter().enumerate() {
            at.insert(*a, i);
        }
        if ins.is_empty() {
            return Err(DecodeError::new("no code"));
        }
        // Function entry thunks at the start of .text.
        let mut thunks = IndexMap::new();
        for (a, t) in &ins {
            if t == "int3" {
                continue;
            }
            let Some(m) = m_word_hex(t, "jmp") else {
                break;
            };
            thunks.insert(*a, hexval(m)?);
        }
        // `_chkesp`: the debug runtime's stack check (a no-op for the decoder).
        let mut seen: IndexMap<i64, i64> = IndexMap::new();
        for i in 0..ins.len() - 1 {
            if m_cmp_esp(&ins[i].1, &["esi", "edi", "ebp"]).is_some()
                && let Some(m) = m_word_hex(&ins[i + 1].1, "call")
            {
                *seen.entry(hexval(m)?).or_insert(0) += 1;
            }
        }
        let chkesp = first_max_key(seen.iter()).copied();
        let dlls: Vec<String> = {
            let set: BTreeSet<&String> = pe.imports().values().map(|(d, _)| d).collect();
            set.into_iter().cloned().collect()
        };
        Ok(Drl {
            pe,
            engine: engine.cloned().unwrap_or_default(),
            ins,
            at,
            thunks,
            chkesp,
            engine_slots: BTreeSet::new(),
            dlls,
        })
    }

    /// `entry(addr)`: the thunk's target, which must be an instruction.
    pub fn entry(&self, addr: i64) -> Result<i64> {
        let addr = self.thunks.get(&addr).copied().unwrap_or(addr);
        if !self.at.contains_key(&addr) {
            return Err(DecodeError::new(format!("no instruction at {}", py_hex(addr))));
        }
        Ok(addr)
    }

    /// `symbol(addr)`: the import at an IAT slot.
    pub fn symbol(&self, addr: i64) -> Option<&(String, String)> {
        u32::try_from(addr).ok().and_then(|a| self.pe.imports().get(&a))
    }

    /// `slot_params(slot)`: the parameter codes of a `ScriptInterfaceImp` method, when known.
    pub fn slot_params(&self, slot: i64) -> Option<Vec<String>> {
        let names = self.engine.get("ScriptInterfaceImp")?;
        let name = usize::try_from(slot).ok().and_then(|s| names.get(s))?;
        msvc_params(name)
    }

    /// `slot_name(cls, slot)`: the plain method name, or `slot_<n>`.
    pub fn slot_name(&self, cls: &str, slot: i64) -> String {
        if let Some(names) = self.engine.get(cls)
            && let Some(n) = usize::try_from(slot).ok().and_then(|s| names.get(s))
        {
            return plain_name(n);
        }
        format!("slot_{slot}")
    }

    fn ins_at(&self, i: usize) -> Result<&(i64, String)> {
        self.ins.get(i).ok_or_else(|| DecodeError::python("IndexError", "list index out of range"))
    }

    // ---------------------------------------------------------- operands

    /// `read(st, op)`: the value an operand holds.
    pub fn read(&self, st: &St, op: &str) -> Result<Py> {
        let op = py_strip(op);
        const DB: [&str; 2] = ["DWORD PTR ", "BYTE PTR "];
        if is_reg32(op) {
            return Ok(st.reg(op));
        }
        if let Some(r) = reg8(op) {
            return Ok(st.reg(r));
        }
        if let Some(h) = m_ds(op, &DB) {
            return Ok(self.load(hexval(h)?));
        }
        if let Some(w) = m_bracket(op, &DB) {
            return Ok(self.deref(&st.reg(w)));
        }
        if let Some(d) = m_ebp(op, &DB) {
            return Ok(st.local_get(hexval(d)?));
        }
        if let Some((b, h)) = m_base_plus(op) {
            let base = st.reg(b);
            if base.truthy() && head(&base) == "vt" {
                return Ok(tup!["method", base.item(1)?, hexval(h)? / 4]);
            }
            return Ok(Py::None);
        }
        if is_hex0x(op) {
            return self.immediate(hexval(op)?);
        }
        Ok(Py::None)
    }

    /// `immediate(n)`: a string, a code address or a constant.
    pub fn immediate(&self, n: i64) -> Result<Py> {
        let sec = u32::try_from(n).ok().and_then(|a| self.pe.section_of(a));
        if sec == Some(".rdata") || sec == Some(".data") {
            match self.pe.cstr(n as u32) {
                Ok(s) => {
                    if !s.is_empty() && s.chars().all(|c| (32..127).contains(&(c as u32))) {
                        return Ok(tup!["str", s]);
                    }
                }
                Err(e) if e.kind == ErrorKind::Decode => {}
                Err(e) => return Err(e),
            }
        }
        if sec == Some(".text") {
            return Ok(tup!["code", n]);
        }
        Ok(tup!["const", if n >= 1 << 31 { n - (1 << 32) } else { n }])
    }

    /// `load(addr)`: an import's address, else a global of the `.drl`.
    pub fn load(&self, addr: i64) -> Py {
        match self.symbol(addr) {
            Some(s) => tup!["iat", &s.1],
            None => tup!["glob", addr],
        }
    }

    /// `deref(v)`: the value behind a pointer value.
    pub fn deref(&self, v: &Py) -> Py {
        if v.is_none() {
            return Py::None;
        }
        match head(v) {
            "iat" => {
                let sym = s1(v);
                match import_kind(sym) {
                    "int" => tup!["obj", plain_name(sym)],
                    "float" => tup!["objf", plain_name(sym)],
                    // `int name[]`: a load through the import is its first element
                    "intarray" => tup!["elem", plain_name(sym), 0],
                    _ => tup!["val", sym],
                }
            }
            "val" | "iiie" | "rule" => tup!["vt", v],
            // `call [vtable]`: slot 0
            "vt" => tup!["method", v.item(1).cloned().unwrap_or(Py::None), 0],
            _ => Py::None,
        }
    }

    /// `take_args(st, i)`: the arguments of the call at instruction `i`, in declaration order: the
    /// pushes since the stack pointer was saved for the call's check.
    pub fn take_args(&self, st: &mut St, i: usize) -> Vec<Py> {
        let mut n = 0usize;
        if i + 1 < self.ins.len()
            && let Some(reg) = m_cmp_esp(&self.ins[i + 1].1, &["esi", "edi"])
            && let Some(&mark) = st.marks.get(reg)
        {
            n = mark.min(st.args.len());
        }
        let args = st.args.split_off(n);
        args.into_iter().rev().collect()
    }

    // ---------------------------------------------------------- registration

    /// `rules()`: `[(name, condition addr, action addr)]` in creation order, and the variables each
    /// rule depends on (by rule index).
    pub fn rules(&self) -> Result<(Vec<Created>, Deps)> {
        let mut rules: Vec<Created> = Vec::new();
        let mut deps: HashMap<usize, Vec<String>> = HashMap::new();
        let mut by_glob: HashMap<i64, i64> = HashMap::new();
        let starts: BTreeSet<i64> = self.thunks.values().copied().collect();
        for start in starts {
            let Some(&first) = self.at.get(&start) else {
                continue;
            };
            let mut st = St::new();
            let mut i = first;
            while i < self.ins.len() {
                let (a, t) = self.ins[i].clone();
                i += 1;
                if t.starts_with("ret") {
                    break;
                }
                let (mn, ops) = split_mn(&t)?;
                if mn == "push" {
                    let v = self.read(&st, ops)?;
                    st.args.push(v);
                } else if mn == "mov" {
                    let (dst, src) = split_pair(ops)?;
                    if m_ds(dst, &["DWORD PTR "]).is_some() {
                        let v = st.reg(src);
                        if v.truthy() && head(&v) == "rule" {
                            let g = hexval(m_ds(dst, &["DWORD PTR "]).expect("matched"))?;
                            by_glob.insert(g, v.item(1)?.as_int().unwrap_or(0));
                        }
                        continue;
                    }
                    if let Some(d) = m_ebp(dst, &["DWORD PTR "]) {
                        let v = self.read(&st, src)?;
                        st.local.insert(hexval(d)?, v);
                        continue;
                    }
                    if (dst == "esi" || dst == "edi") && src == "esp" {
                        st.marks.insert(dst.to_owned(), st.args.len());
                        continue;
                    }
                    if is_reg32(dst) {
                        let mut v = self.read(&st, src)?;
                        if v.truthy()
                            && head(&v) == "glob"
                            && let Some(&ri) = v.item(1)?.as_int().and_then(|g| by_glob.get(&g))
                        {
                            v = tup!["rule", ri];
                        }
                        st.r.insert(dst.to_owned(), v);
                    }
                } else if mn == "call" {
                    if is_hex0x(ops) && Some(hexval(ops)?) == self.chkesp {
                        continue;
                    }
                    let target = self.read(&st, ops)?;
                    let args = self.take_args(&mut st, i - 1);
                    st.r.insert("eax".to_owned(), Py::None);
                    if !target.truthy() || head(&target) != "method" {
                        continue;
                    }
                    let obj = target.item(1)?.clone();
                    let slot = target.item(2)?.as_int().unwrap_or(-1);
                    if obj.truthy() && head(&obj) == "val" && plain_name(s1(&obj)) == "p_WORLD" && slot == CHARACTER_GET_IIIE {
                        st.r.insert("eax".to_owned(), tup!["iiie"]);
                    } else if obj == tup!["iiie"] && slot == IIIE_CREATE_RULE && args.len() >= 3 {
                        let (cond, act, name) = (&args[0], &args[1], &args[2]);
                        let code = |v: &Py| v.truthy() && head(v) == "code";
                        if !(code(cond) && code(act) && name.truthy() && head(name) == "str") {
                            return Err(DecodeError::new(format!(
                                "create_rule at {} with unexpected arguments {}",
                                py_hex(a),
                                Py::List(args.clone()).to_str()
                            )));
                        }
                        rules.push((
                            name.item(1)?.to_str(),
                            cond.item(1)?.as_int().unwrap_or(0),
                            act.item(1)?.as_int().unwrap_or(0),
                        ));
                        st.r.insert("eax".to_owned(), tup!["rule", rules.len() - 1]);
                    } else if obj.truthy()
                        && head(&obj) == "val"
                        && slot == HOT_ADD_DEPENDENT_RULE
                        && !args.is_empty()
                        && args[0].truthy()
                        && head(&args[0]) == "rule"
                        && VAR_KINDS.contains(&import_kind(s1(&obj)))
                    {
                        let ri = args[0].item(1)?.as_int().unwrap_or(0) as usize;
                        deps.entry(ri).or_default().push(plain_name(s1(&obj)));
                    }
                } else if matches!(
                    mn,
                    "add" | "sub" | "lea" | "rep" | "cmp" | "test" | "pop" | "and" | "xor" | "or" | "inc" | "dec" | "neg" | "sbb"
                ) || mn.starts_with('j')
                    || mn.starts_with('f')
                    || mn.starts_with("set")
                {
                    if mn == "pop" && !st.args.is_empty() {
                        st.args.pop();
                    }
                    if (mn == "add" || mn == "sub") && ops.starts_with("esp,") {
                        st.args.clear();
                    }
                    continue;
                }
            }
        }
        Ok((rules, deps))
    }

    // ---------------------------------------------------------- conditions

    /// `condition(addr)`: the boolean expression a condition function returns.
    ///
    /// The debug compiler's conditions only jump forward, so the function is a DAG: states are run
    /// in address order and merged where branches join, keeping the path condition as a list of
    /// predicates. A value that differs between the joining paths (the 0/1 the compiler stores for a
    /// materialised `&&`/`||`) becomes the disjunction of the paths that stored 1.
    pub fn condition(&mut self, addr: i64) -> Result<Py> {
        let entry = self.entry(addr)?;
        let start = self.at[&entry];
        let mut targets: BTreeSet<i64> = BTreeSet::new();
        let mut i = start;
        loop {
            if i >= self.ins.len() {
                return Err(DecodeError::new("ran off the code"));
            }
            let (a, t) = &self.ins[i];
            if let Some(m) = m_jcc_target(t) {
                let tgt = hexval(m)?;
                if tgt <= *a {
                    return Err(DecodeError::new(format!("backward jump at {}", py_hex(*a))));
                }
                targets.insert(tgt);
            }
            if t.starts_with("ret") && !targets.iter().any(|x| x > a) {
                break;
            }
            i += 1;
        }
        let mut pending: BTreeMap<usize, Vec<(Path_, St)>> = BTreeMap::new();
        pending.insert(start, vec![(Vec::new(), St::new())]);
        let mut results: Vec<(Path_, Py)> = Vec::new();
        while let Some((&first_i, _)) = pending.iter().next() {
            let mut i = first_i;
            let incoming = pending.remove(&i).expect("present");
            let (mut path, mut st) = merge(incoming)?;
            let mut first = true;
            loop {
                let (a, t) = self.ins_at(i)?.clone();
                if !first && targets.contains(&a) {
                    pending.entry(i).or_default().push((path, st));
                    break;
                }
                first = false;
                let (mn, ops) = split_mn(&t)?;
                let nxt = self.step(&mut st, a, mn, ops, i)?;
                match nxt {
                    Next::Ret => {
                        results.push((path, st.reg("eax")));
                        break;
                    }
                    Next::Fall => {
                        i += 1;
                        continue;
                    }
                    Next::Jmp(target) => {
                        let Some(&j) = self.at.get(&target) else {
                            return Err(DecodeError::new(format!("jump outside the code at {}", py_hex(a))));
                        };
                        pending.entry(j).or_default().push((path, st));
                        break;
                    }
                    Next::Jcc(target, pred) => {
                        let Some(&j) = self.at.get(&target) else {
                            return Err(DecodeError::new(format!("jump outside the code at {}", py_hex(a))));
                        };
                        let pred = simplify(&pred)?;
                        if pred != false_() {
                            let mut p = path.clone();
                            p.push(pred.clone());
                            pending.entry(j).or_default().push((p, clone_state(&st)));
                        }
                        if pred == true_() {
                            break;
                        }
                        path.push(simplify(&negate(&pred)?)?);
                        i += 1;
                    }
                }
            }
        }
        let mut terms = Vec::new();
        for (path, v) in results {
            if v.is_none() {
                return Err(DecodeError::new("condition returns an unknown value"));
            }
            let mut p = path;
            p.push(as_bool(&v)?);
            terms.push(conj(&p));
        }
        if terms.is_empty() {
            Ok(false_())
        } else {
            simplify(&Py::Tuple(vec![Py::from("or"), Py::List(terms)]))
        }
    }

    /// `step(st, a, mn, ops, i)`: one instruction of a condition function.
    pub fn step(&mut self, st: &mut St, a: i64, mn: &str, ops: &str, i: usize) -> Result<Next> {
        let at = || py_hex(a);
        if mn == "ret" {
            return Ok(Next::Ret);
        }
        if mn == "push" {
            let v = self.read(st, ops)?;
            if matches!(ops, "ebp" | "ebx" | "esi" | "edi") && !st.body {
                return Ok(Next::Fall); // saved registers of the prologue
            }
            st.args.push(v);
            return Ok(Next::Fall);
        }
        if mn == "pop" {
            st.args.pop();
            return Ok(Next::Fall);
        }
        if matches!(mn, "int3" | "nop" | "fnclex" | "fwait") {
            return Ok(Next::Fall);
        }
        if mn == "rep" {
            st.body = true; // the debug prologue (fill with 0xcc) is done
            st.args.clear();
            return Ok(Next::Fall);
        }
        if mn == "lea" || ((mn == "sub" || mn == "add") && ops.starts_with("esp,")) {
            if mn == "add" {
                st.args.clear();
            }
            return Ok(Next::Fall);
        }
        if mn == "mov" {
            let (dst, src) = split_pair(ops)?;
            if (dst == "esi" || dst == "edi") && src == "esp" {
                st.marks.insert(dst.to_owned(), st.args.len());
                return Ok(Next::Fall);
            }
            if dst == "ebp" || dst == "esp" {
                return Ok(Next::Fall);
            }
            if let Some(d) = m_ebp(dst, &["DWORD PTR "]) {
                let v = self.read(st, src)?;
                st.local.insert(hexval(d)?, v);
                return Ok(Next::Fall);
            }
            if is_reg32(dst) {
                let v = self.read(st, src)?;
                st.r.insert(dst.to_owned(), v);
                return Ok(Next::Fall);
            }
            return Err(DecodeError::new(format!("unsupported store `{mn} {ops}` at {}", at())));
        }
        if mn == "call" {
            if is_hex0x(ops) && Some(hexval(ops)?) == self.chkesp {
                return Ok(Next::Fall);
            }
            let args = self.take_args(st, i);
            let target = self.read(st, ops)?;
            let v;
            let sym_call = m_ds(ops, &["DWORD PTR "]).map(hexval).transpose()?.and_then(|ad| self.symbol(ad).cloned());
            if let Some((_, sym)) = sym_call {
                let k = import_kind(&sym);
                if k != "function" && k != "action" {
                    return Err(DecodeError::new(format!("call to {sym} at {}", at())));
                }
                let mut ex = Vec::new();
                for x in &args {
                    ex.push(expr_arg(x, a)?);
                }
                let typed = typed_args(&ex, msvc_params(&sym).as_deref())?;
                v = Py::Tuple(vec![Py::from("call"), Py::from(plain_name(&sym)), Py::List(typed)]);
            } else if target.truthy() && head(&target) == "method" {
                let obj = target.item(1)?.clone();
                let slot = target.item(2)?.as_int().unwrap_or(-1);
                if head(&obj) != "val" {
                    return Err(DecodeError::new(format!("method call on {} at {}", obj.to_str(), at())));
                }
                let k = import_kind(s1(&obj));
                if VAR_KINDS.contains(&k) {
                    if slot != HOT_VALUE || !args.is_empty() {
                        return Err(DecodeError::new(format!("{k} slot {slot} in a condition at {}", at())));
                    }
                    v = tup!["var", plain_name(s1(&obj)), k];
                } else if k == "ScriptInterface" {
                    self.engine_slots.insert(slot);
                    let mut ex = Vec::new();
                    for x in &args {
                        ex.push(expr_arg(x, a)?);
                    }
                    let typed = typed_args(&ex, self.slot_params(slot).as_deref())?;
                    v = Py::Tuple(vec![
                        Py::from("call"),
                        Py::from(self.slot_name("ScriptInterfaceImp", slot)),
                        Py::List(typed),
                    ]);
                } else {
                    return Err(DecodeError::new(format!("call on {} slot {slot} at {}", s1(&obj), at())));
                }
            } else {
                return Err(DecodeError::new(format!("unsupported call `{ops}` at {}", at())));
            }
            st.r.insert("eax".to_owned(), v.clone());
            st.fret = v;
            for x in ["ecx", "edx"] {
                st.r.insert(x.to_owned(), Py::None);
            }
            return Ok(Next::Fall);
        }
        if mn == "test" {
            let (x, y) = split_pair(ops)?;
            if x == y {
                let v = self.read(st, x)?;
                st.flags = tup!["test", v];
                return Ok(Next::Fall);
            }
            if x == "ah" && st.fsw.truthy() {
                st.flags = tup!["ftest", &st.fsw, hexval(y)?];
                return Ok(Next::Fall);
            }
            return Err(DecodeError::new(format!("unsupported `test {ops}` at {}", at())));
        }
        if mn == "cmp" {
            let (x, y) = split_pair(ops)?;
            if y == "esp" {
                return Ok(Next::Fall);
            }
            let (vx, vy) = (self.read(st, x)?, self.read(st, y)?);
            st.flags = tup!["cmp", vx, vy];
            return Ok(Next::Fall);
        }
        if drl_jcc_op(mn).is_some() {
            let target = hexval(ops)?;
            let f = st.flags.clone();
            if f.is_none() {
                return Err(DecodeError::new(format!("branch without a compare at {}", at())));
            }
            return Ok(Next::Jcc(target, branch_pred(mn, &f, a)?));
        }
        if mn == "jmp" {
            return Ok(Next::Jmp(hexval(ops)?));
        }
        if let Some(op) = setcc_op(mn) {
            let dst = reg8(py_strip(ops));
            let f = st.flags.clone();
            let Some(dst) = dst else {
                return Err(DecodeError::new(format!("unsupported `{mn} {ops}` at {}", at())));
            };
            if !f.truthy() || head(&f) != "cmp" {
                return Err(DecodeError::new(format!("unsupported `{mn} {ops}` at {}", at())));
            }
            st.r.insert(dst.to_owned(), tup!["cmp", op, f.item(1)?, f.item(2)?]);
            return Ok(Next::Fall);
        }
        if mn == "xor" {
            let (x, y) = split_pair(ops)?;
            if x == y && is_reg32(x) {
                st.r.insert(x.to_owned(), tup!["const", 0]);
                return Ok(Next::Fall);
            }
            return Err(DecodeError::new(format!("unsupported `xor {ops}` at {}", at())));
        }
        if mn == "and" {
            let (x, y) = split_pair(ops)?;
            if is_reg32(x) && y == "0xff" {
                let v = st.reg(x);
                let nv = if v.truthy() && !matches!(head(&v), "cmp" | "not" | "and" | "or" | "bool" | "const") {
                    tup!["bool", v]
                } else {
                    v
                };
                st.r.insert(x.to_owned(), nv.clone());
                st.flags = tup!["test", nv];
                return Ok(Next::Fall);
            }
            return Err(DecodeError::new(format!("unsupported `and {ops}` at {}", at())));
        }
        if mn == "neg" && ops == "eax" {
            let v = st.reg("eax");
            // the two compiler idioms for `x != 0` and `!x`
            let nv = if v.truthy() && head(&v) == "m1" { as_bool(v.item(1)?)? } else { tup!["neg", v] };
            st.r.insert("eax".to_owned(), nv);
            return Ok(Next::Fall);
        }
        if mn == "sbb" && ops == "eax,eax" {
            let v = st.reg("eax");
            if !v.truthy() || head(&v) != "neg" {
                return Err(DecodeError::new(format!("unsupported sbb at {}", at())));
            }
            st.r.insert("eax".to_owned(), tup!["m1", v.item(1)?]);
            return Ok(Next::Fall);
        }
        if mn == "inc" && ops == "eax" {
            let v = st.reg("eax");
            if v.truthy() && head(&v) == "m1" {
                st.r.insert("eax".to_owned(), tup!["not", as_bool(v.item(1)?)?]);
                return Ok(Next::Fall);
            }
            if v.truthy() && head(&v) == "const" {
                let n = py_add_one(v.item(1)?)?;
                st.r.insert("eax".to_owned(), tup!["const", n]);
                return Ok(Next::Fall);
            }
            st.r.insert("eax".to_owned(), tup!["arith", "+", v, tup!["const", 1]]);
            return Ok(Next::Fall);
        }
        if mn == "add" || mn == "sub" {
            let (x, y) = split_pair(ops)?;
            if is_reg32(x) {
                let vx = st.reg(x);
                let vy = self.read(st, y)?;
                st.flags = if mn == "sub" {
                    tup!["cmp", &vx, &vy]
                } else {
                    tup!["test", tup!["arith", "+", &vx, &vy]]
                };
                st.r.insert(x.to_owned(), tup!["arith", if mn == "add" { "+" } else { "-" }, vx, vy]);
                return Ok(Next::Fall);
            }
            return Err(DecodeError::new(format!("unsupported `{mn} {ops}` at {}", at())));
        }
        // x87
        if mn == "fstp" {
            let v = fpop(st, a)?;
            if let Some(d) = ops
                .strip_prefix("DWORD PTR ")
                .or_else(|| ops.strip_prefix("QWORD PTR "))
                .and_then(|r| m_ebp(r, &[]))
            {
                st.local.insert(hexval(d)?, v);
                return Ok(Next::Fall);
            }
            if ops == "st(0)" {
                return Ok(Next::Fall);
            }
            return Err(DecodeError::new(format!("unsupported `fstp {ops}` at {}", at())));
        }
        if mn == "fld" || mn == "fild" {
            let v = self.fread(st, ops, a, mn == "fild")?;
            st.fpu.push(v);
            return Ok(Next::Fall);
        }
        if matches!(mn, "fadd" | "fsub" | "fmul" | "fdiv" | "fsubr" | "fdivr") {
            let op = match mn {
                "fadd" => "+",
                "fsub" => "-",
                "fmul" => "*",
                "fdiv" => "/",
                "fsubr" => "r-",
                _ => "r/",
            };
            let top = fpop(st, a)?;
            let v = if matches!(ops, "st(0),st" | "st,st(0)" | "st(0),st(0)") {
                if op == "+" {
                    tup!["arith", "*", tup!["float", 2.0], &top]
                } else {
                    tup!["arith", op, &top, &top]
                }
            } else {
                let y = self.fread(st, ops, a, false)?;
                binop(op, &top, &y)
            };
            st.fpu.push(v);
            return Ok(Next::Fall);
        }
        if matches!(mn, "fcomp" | "fcom" | "fcompp") {
            let mut other = if mn == "fcompp" { Py::None } else { self.fread(st, ops, a, false)? };
            let top = if mn != "fcom" { fpop(st, a)? } else { fpeek(st, a)? };
            if mn == "fcompp" {
                other = fpop(st, a)?;
            }
            st.fcmp = Py::Tuple(vec![top, other]);
            return Ok(Next::Fall);
        }
        if mn == "fnstsw" && ops == "ax" {
            if !st.fcmp.truthy() {
                return Err(DecodeError::new(format!("fnstsw without a compare at {}", at())));
            }
            st.fsw = st.fcmp.clone();
            st.r.insert("eax".to_owned(), Py::None);
            return Ok(Next::Fall);
        }
        Err(DecodeError::new(format!("unsupported instruction `{mn} {ops}` at {}", at())))
    }

    /// `fread(st, op, a, integer)`: the value of an x87 operand.
    pub fn fread(&self, st: &mut St, op: &str, a: i64, integer: bool) -> Result<Py> {
        let at = || py_hex(a);
        for (word, wide) in [("DWORD PTR ", false), ("QWORD PTR ", true)] {
            if let Some(h) = op.strip_prefix(word).and_then(|r| m_ds(r, &[])) {
                let addr = hexval(h)?;
                if let Some(s) = self.symbol(addr) {
                    return Ok(self.deref(&tup!["iat", &s.1]));
                }
                if integer {
                    return Err(DecodeError::new(format!("integer load from {} at {}", py_hex(addr), at())));
                }
                let outside = |what: &str| DecodeError::new(format!("{what} at {} outside the file", py_hex(addr)));
                let ad = u32::try_from(addr).map_err(|_| outside(if wide { "double" } else { "float" }))?;
                return Ok(tup!["float", if wide { self.pe.f64(ad)? } else { f64::from(self.pe.f32(ad)?) }]);
            }
        }
        for word in ["DWORD PTR ", "QWORD PTR "] {
            if let Some(d) = op.strip_prefix(word).and_then(|r| m_ebp(r, &[])) {
                return Ok(st.local_get(hexval(d)?));
            }
        }
        if let Some(w) = op.strip_prefix("DWORD PTR ").and_then(|r| m_bracket(r, &[])) {
            return Ok(self.deref(&st.reg(w)));
        }
        if let Some(d) = op.strip_prefix("st(").and_then(|r| r.strip_suffix(')'))
            && d.len() == 1
            && d.as_bytes()[0].is_ascii_digit()
        {
            fpeek(st, a)?;
            let k = usize::from(d.as_bytes()[0] - b'0');
            if k >= st.fpu.len() {
                return Err(DecodeError::new(format!("x87 st({k}) is empty at {}", at())));
            }
            return Ok(st.fpu[st.fpu.len() - 1 - k].clone());
        }
        Err(DecodeError::new(format!("unsupported x87 operand `{op}` at {}", at())))
    }
}

/// `x + 1` of the `inc` handler (`v[1] + 1`), printed as Python adds it.
fn py_add_one(v: &Py) -> Result<Py> {
    match v {
        Py::Int(n) => n.checked_add(1).map(Py::Int).ok_or_else(|| DecodeError::raw("integer overflow")),
        Py::Bool(b) => Ok(Py::Int(i64::from(*b) + 1)),
        Py::Float(x) => Ok(Py::Float(x + 1.0)),
        other => Err(DecodeError::python("TypeError", format!(
            "unsupported operand type(s) for +: '{}' and 'int'",
            other.type_name()
        ))),
    }
}

// ------------------------------------------------------------------ decode

/// One variable of the `.dsl` a condition reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub kind: String,
    /// The engine supplies it (`v["engine"] = True`).
    pub engine: bool,
}

/// One decoded rule (the Python's dict).
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub name: String,
    pub action: String,
    pub deps: Vec<String>,
    /// `rule["condition"]`: set before the expression is rendered.
    pub condition: Option<Py>,
    /// `rule["condition_lua"]`: `None` when the condition was not decoded.
    pub condition_lua: Option<String>,
    /// `rule["error"]` when the condition was not decoded.
    pub error: Option<String>,
}

/// The result of [`decode`] (the Python's dict).
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub mission: String,
    pub drl: String,
    pub dsl: String,
    pub variables: Vec<Variable>,
    pub objects: Vec<String>,
    pub actions: Vec<String>,
    pub rules: Vec<Rule>,
    pub calls: Vec<String>,
    pub call_sites: IndexMap<String, usize>,
    pub call_signatures: Vec<String>,
    pub unused_variables: Vec<String>,
    pub errors: Vec<String>,
}

/// `decode(path, engine)` over an image and its listing; `base` is the file's base name.
/// [`Failure::Unsupported`] is a DLL of another shape (Armada I's).
pub fn decode<I: Image>(pe: I, listing: Vec<Ins>, base: &str, engine: Option<&Vtables>) -> std::result::Result<Decoded, Failure> {
    let mut d = Drl::new(pe, listing, engine)?;
    let any_gpsi = d.pe.imports().values().any(|(_, sym)| plain_name(sym) == "g_pSI");
    if !any_gpsi && d.dlls.iter().any(|dll| py_lower(dll) == "armada.exe") {
        return Err(Failure::Unsupported(
            "not an Armada II rule DLL (Armada I shape): not decoded by this tool".to_owned(),
        ));
    }
    if d.pe.export_size() != 0 {
        return Err(DecodeError::new("has an export table; rule DLLs have none").into());
    }
    let (created, deps) = d.rules()?;
    if created.is_empty() {
        return Err(DecodeError::new("no create_rule calls found").into());
    }
    let mut imports: IndexMap<String, String> = IndexMap::new();
    for (dll, s) in d.pe.imports().values() {
        imports.insert(s.clone(), dll.clone());
    }
    let dsl = d.dlls.iter().find(|dll| py_lower(dll).ends_with(".dsl")).cloned();
    let (mut variables, mut objects, mut actions): (Vec<Variable>, Vec<String>, Vec<String>) = (vec![], vec![], vec![]);
    let mut getters: BTreeSet<String> = BTreeSet::new();
    if let Some(names) = engine.and_then(|e| e.get("ScriptInterfaceImp")) {
        for n in names {
            let p = plain_name(n);
            if let Some(g) = p.strip_prefix("ScriptGet_") {
                getters.insert(g.to_owned());
            }
        }
    }
    for (s, dll) in &imports {
        let k = import_kind(s);
        let p = plain_name(s);
        let is_dsl = py_lower(dll).ends_with(".dsl");
        if VAR_KINDS.contains(&k) {
            let bare = p.strip_prefix("p_").unwrap_or(&p);
            let engine_var = getters.contains(bare) || !is_dsl;
            variables.push(Variable { name: p.clone(), kind: k.to_owned(), engine: engine_var });
        } else if (k == "int" || k == "intarray") && is_dsl {
            objects.push(p);
        }
    }
    let (mut rules, mut seen, mut errors): (Vec<Rule>, HashMap<String, usize>, Vec<String>) = (vec![], HashMap::new(), vec![]);
    for (name, cond, act) in &created {
        let body = match d.entry(*act) {
            Ok(b) => b,
            Err(e) if e.kind == ErrorKind::Decode => {
                return Err(DecodeError::new(format!("{name}: action: {}", e.message)).into());
            }
            Err(e) => return Err(e.into()),
        };
        let target = action_target(&d, body)?;
        if !actions.contains(&target) {
            actions.push(target.clone());
        }
        let mut rule = Rule {
            name: name.clone(),
            action: target,
            deps: deps.get(&rules.len()).cloned().unwrap_or_default(),
            condition: None,
            condition_lua: None,
            error: None,
        };
        let n = seen.entry(name.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            rule.name = format!("{name}#{n}");
        }
        let cond_result = d.condition(*cond).and_then(|e| {
            rule.condition = Some(e.clone());
            render(&e, Ctx::Bool)
        });
        match cond_result {
            Ok(lua) => rule.condition_lua = Some(lua),
            Err(ex) if ex.kind == ErrorKind::Decode => {
                rule.condition_lua = None;
                rule.error = Some(ex.message.clone());
                errors.push(format!("{}: {}", rule.name, ex.message));
            }
            Err(ex) => return Err(ex.into()),
        }
        rules.push(rule);
    }
    // `re.sub(r"[rR]\.drl$", "", base)`
    let mission = if base.ends_with("r.drl") || base.ends_with("R.drl") { &base[..base.len() - 5] } else { base };
    let mut call_set: BTreeSet<String> = BTreeSet::new();
    let mut call_sites: IndexMap<String, usize> = IndexMap::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    for r in &rules {
        for x in &r.deps {
            used.insert(x.clone());
        }
        if let Some(c) = &r.condition {
            calls_in(c, &mut |s| {
                call_set.insert(s.to_owned());
            });
            call_shapes(c, &mut |k| {
                *call_sites.entry(k).or_insert(0) += 1;
            });
            vars_in(c, &mut |v| {
                used.insert(v.to_owned());
            });
        }
    }
    variables.sort_by(|a, b| a.name.cmp(&b.name));
    objects.sort();
    let sig_names = d.engine.get("ScriptInterfaceImp").cloned().unwrap_or_default();
    let mut call_signatures: Vec<String> = d
        .engine_slots
        .iter()
        .filter(|&&s| s >= 0 && (s as usize) < sig_names.len())
        .map(|&s| sig_names[s as usize].clone())
        .collect();
    call_signatures.sort();
    let mut unused_variables: Vec<String> =
        variables.iter().filter(|v| !used.contains(&v.name)).map(|v| v.name.clone()).collect();
    unused_variables.sort();
    Ok(Decoded {
        mission: mission.to_owned(),
        drl: base.to_owned(),
        dsl: dsl.unwrap_or_else(|| format!("{mission}S.dsl")),
        variables,
        objects,
        actions,
        rules,
        calls: call_set.into_iter().collect(),
        call_sites,
        call_signatures,
        unused_variables,
        errors,
    })
}

/// `action_target(d, start)`: the one `.dsl` function an action calls.
pub fn action_target<I: Image>(d: &Drl<I>, start: i64) -> Result<String> {
    let mut i = *d.at.get(&start).ok_or_else(|| DecodeError::python("KeyError", py_hex(start)))?;
    let mut called: Vec<String> = Vec::new();
    loop {
        let (a, t) = d.ins_at(i)?;
        if t.starts_with("ret") {
            break;
        }
        if let Some(h) = word_ws(t, "call").and_then(|r| m_ds(r, &["DWORD PTR "])) {
            let Some(s) = d.symbol(hexval(h)?) else {
                return Err(DecodeError::new(format!("action calls a non-import at {}", py_hex(*a))));
            };
            called.push(plain_name(&s.1));
        } else if let Some(h) = m_word_hex(t, "call") {
            let _ = h;
            let last = t.split(py_isspace).rfind(|x| !x.is_empty()).unwrap_or("");
            if Some(hexval(last)?) != d.chkesp {
                return Err(DecodeError::new(format!("action calls code of its own at {}", py_hex(*a))));
            }
        } else if t.starts_with("call") {
            return Err(DecodeError::new(format!("action makes an indirect call at {}", py_hex(*a))));
        }
        i += 1;
    }
    if called.len() != 1 {
        return Err(DecodeError::new(format!("action calls {} functions, expected 1", called.len())));
    }
    Ok(called.remove(0))
}

/// The nodes `calls_in`, `call_shapes` and `vars_in` of the Python walk: `e[1:]` of a tuple, a
/// nested tuple, or the tuples of a nested list.
fn walk_children(e: &Py, f: &mut dyn FnMut(&Py)) {
    let Py::Tuple(items) = e else {
        return;
    };
    for x in items.iter().skip(1) {
        match x {
            Py::Tuple(_) => f(x),
            Py::List(l) => {
                for y in l {
                    f(y);
                }
            }
            _ => {}
        }
    }
}

/// `calls_in(e)`: the engine call names an expression makes.
pub fn calls_in(e: &Py, out: &mut dyn FnMut(&str)) {
    if !matches!(e, Py::Tuple(_)) {
        return;
    }
    if e.tag().ok().flatten() == Some("call") {
        out(s1(e));
    }
    walk_children(e, &mut |x| calls_in(x, out));
}

/// `call_shapes(e)`: `Name/<argument count>` of each engine call of an expression.
pub fn call_shapes(e: &Py, out: &mut dyn FnMut(String)) {
    if !matches!(e, Py::Tuple(_)) {
        return;
    }
    if e.tag().ok().flatten() == Some("call") {
        let n = e.item(2).ok().and_then(|a| a.py_len().ok()).unwrap_or(0);
        out(format!("{}/{n}", s1(e)));
    }
    walk_children(e, &mut |x| call_shapes(x, out));
}

/// `vars_in(e)`: the variables an expression reads.
pub fn vars_in(e: &Py, out: &mut dyn FnMut(&str)) {
    if !matches!(e, Py::Tuple(_)) {
        return;
    }
    if e.tag().ok().flatten() == Some("var") {
        out(s1(e));
    }
    walk_children(e, &mut |x| vars_in(x, out));
}

// ------------------------------------------------------------------ scaffold

/// `scaffold(m)`: the Lua scaffold of one decoded mission. The header line names this tool where
/// the Python names `tools/drl_to_lua.py (LUA-07)` ([`crate::tool_token`]).
pub fn scaffold(m: &Decoded) -> String {
    let mut l: Vec<String> = Vec::new();
    macro_rules! w {
        ($s:expr) => {
            l.push(($s).to_string())
        };
    }
    w!(format!("-- {}: rule scaffold generated by {}.", m.mission, crate::tool_token()));
    w!(format!("-- Rules decoded from {}; actions, variable values and object handles", m.drl));
    w!(format!("-- live in {} and are left for the hand port. Regenerate, do not edit", m.dsl));
    w!("-- the generated tables. Not in the .drl: the engine's rule scheduling. Here each");
    w!("-- rule is a named condition monitor checked every tick that fires on a");
    w!("-- false-to-true edge; the stock engine re-evaluates a rule when one of its");
    w!("-- dependencies is marked dirty (that policy is not reproduced).");
    w!("");
    w!("local function T(v) return v ~= nil and v ~= false and v ~= 0 end");
    w!("local function N(v) if v == true then return 1 elseif type(v) ~= 'number' then return 0 end return v end");
    w!("");
    w!("local Mission = {");
    w!(format!("    name = {},", lua_str(&m.mission)));
    w!("    state = {},        -- the .dsl's rule variables, by name");
    w!("    objects = {},      -- the .dsl's object handles, set by the ported setup");
    w!("    timers = {},       -- running timer_int deadlines, by name");
    w!("    rules = {},        -- the decoded rule table, in creation order");
    w!("    ruleMonitors = {},");
    w!("    missing = {},      -- engine calls the runtime does not bind yet: name -> count");
    w!("    failed = {},       -- engine calls that raised an error: name -> count");
    w!("    callFailed = false, -- set while a condition's engine call is unbound or failed");
    w!("    time = 0,");
    w!("}");
    w!("");
    w!("-- Engine calls of the conditions go to the A2 table by the ScriptInterface");
    w!("-- method's own name. Unbound names and errors are counted, not raised, so an");
    w!("-- unported mission still runs; a rule whose engine call gave no answer does");
    w!("-- not hold that tick.");
    w!("local function A(name, ...)");
    w!("    local f = A2[name]");
    w!("    if type(f) ~= 'function' then");
    w!("        Mission.missing[name] = (Mission.missing[name] or 0) + 1");
    w!("        Mission.callFailed = true");
    w!("        return nil");
    w!("    end");
    w!("    local ok, v = pcall(f, ...)");
    w!("    if not ok then");
    w!("        Mission.failed[name] = (Mission.failed[name] or 0) + 1");
    w!("        Mission.callFailed = true");
    w!("        return nil");
    w!("    end");
    w!("    return v");
    w!("end");
    w!("");
    w!(format!("-- Variables ({}): kind, and whether the engine supplies it.", m.variables.len()));
    w!("Mission.variables = {");
    for v in &m.variables {
        let extra = if v.engine { ", engine = true" } else { "" };
        w!(format!("    {{ name = {}, kind = {}{extra} }},", lua_str(&v.name), lua_str(&v.kind)));
    }
    w!("}");
    let objs: Vec<String> = m.objects.iter().map(|o| lua_str(o)).collect();
    w!(format!("Mission.objectNames = {{ {} }}", objs.join(", ")));
    w!("");
    w!("-- Start values: the .dsl's setup assigns them; the hand port puts them here");
    w!("-- (unlisted variables start at 0). The engine's ALWAYS_EVALUATE reads -1");
    w!("-- (measured).");
    w!("Mission.initial = {");
    for v in &m.variables {
        if v.engine && v.kind == "dirty_int" {
            w!(format!("    {} = -1,", v.name));
        }
    }
    w!("}");
    w!("");
    w!(format!("-- Rules ({}), in the order the .drl creates them.", m.rules.len()));
    for (i, r) in m.rules.iter().enumerate() {
        w!(format!("Mission.rules[{}] = {{", i + 1));
        w!(format!("    name = {},", lua_str(&r.name)));
        w!(format!("    action = {},", lua_str(&r.action)));
        let deps: Vec<String> = r.deps.iter().map(|d| lua_str(d)).collect();
        w!(format!("    deps = {{ {} }},", deps.join(", ")));
        if let Some(c) = &r.condition_lua {
            w!(format!("    when = function(S, O) return {c} end,"));
        } else {
            w!(format!("    -- condition not decoded: {}", r.error.as_deref().unwrap_or("")));
            w!("    when = function(S, O) return false end,");
        }
        w!("}");
    }
    w!("");
    w!(format!("-- Actions ({}): the .dsl functions the rules call. Port each body", m.actions.len()));
    w!(format!("-- from {}; until then it only records that it ran.", m.dsl));
    w!("Mission.ran = {}");
    for a in &m.actions {
        match lua_ident(a) {
            Some(ident) => {
                w!(format!("function Mission:{ident}() self.ran[#self.ran + 1] = {} end", lua_str(a)))
            }
            None => w!(format!(
                "Mission[{}] = function(self) self.ran[#self.ran + 1] = {} end",
                lua_str(a),
                lua_str(a)
            )),
        }
    }
    w!("");
    w!("-- timer_int: reads 1 once its time has run out (measured); here starting or");
    w!("-- stopping a timer clears it.");
    w!("function Mission:startTimer(name, seconds)");
    w!("    self.timers[name] = self.time + seconds");
    w!("    self.state[name] = 0");
    w!("end");
    w!("");
    w!("function Mission:stopTimer(name)");
    w!("    self.timers[name] = nil");
    w!("    self.state[name] = 0");
    w!("end");
    w!("");
    w!("function Mission:setup()");
    w!("    for _, v in ipairs(self.variables) do");
    w!("        if self.state[v.name] == nil then self.state[v.name] = self.initial[v.name] or 0 end");
    w!("    end");
    w!("    for i, rule in ipairs(self.rules) do");
    w!("        local monitor = Monitor(rule.name, false, nil, function()");
    w!("            self.callFailed = false");
    w!("            local holds = rule.when(self.state, self.objects) == true");
    w!("            return holds and not self.callFailed  -- no answer from the engine: does not hold");
    w!("        end, nil)");
    w!("        monitor:hook(rule.name, MonitorHook.Type.OnTrue, nil, function()");
    w!("            self[rule.action](self)");
    w!("        end, nil)");
    w!("        self.ruleMonitors[i] = monitor");
    w!("    end");
    w!("end");
    w!("");
    w!("function Mission:update(time)");
    w!("    self.time = time");
    w!("    for _, v in ipairs(self.variables) do");
    w!("        local at = self.timers[v.name]");
    w!("        if at and time > at then");
    w!("            self.timers[v.name] = nil");
    w!("            self.state[v.name] = 1");
    w!("        end");
    w!("    end");
    w!("end");
    w!("");
    w!("MMM.register(Mission)");
    let mut out = l.join("\n");
    out.push('\n');
    out
}

// ------------------------------------------------------------------ the command line's parts

/// `find_drls(root)`: every file under `root` whose lower-cased name ends in `r.drl`, sorted by
/// path string (symbolic links to folders are not followed, as `os.walk`).
pub fn find_drls(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            let is_dir = if e.file_type().is_ok_and(|t| t.is_symlink()) {
                // `os.walk` lists a link to a folder with the folders and does not enter it
                if std::fs::metadata(&p).is_ok_and(|m| m.is_dir()) {
                    continue;
                }
                false
            } else {
                e.file_type().is_ok_and(|t| t.is_dir())
            };
            if is_dir {
                walk(&p, out);
            } else if py_lower(&e.file_name().to_string_lossy()).ends_with("r.drl") {
                out.push(p.to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out.into_iter().map(PathBuf::from).collect()
}

/// `os.path.basename(os.path.normpath(root))`: empty and `.` parts dropped, `..` collapsed against
/// a preceding name (a leading `..` is kept in a relative path and dropped in an absolute one),
/// `.` for a relative path that collapses to nothing, the empty string for the root.
fn normpath_basename(root: &Path) -> String {
    let s = root.to_string_lossy();
    let sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    let absolute = s.starts_with(sep);
    let mut stack: Vec<&str> = Vec::new();
    for part in s.split(sep) {
        match part {
            "" | "." => {}
            ".." => {
                if stack.last().is_some_and(|l| *l != "..") {
                    stack.pop();
                } else if !absolute {
                    stack.push("..");
                }
            }
            p => stack.push(p),
        }
    }
    match stack.last() {
        Some(l) => (*l).to_owned(),
        None if absolute => String::new(),
        None => ".".to_owned(),
    }
}

/// `install_subdirs(installs)`: one output folder name per install, distinct even for equal
/// basenames (`[^A-Za-z0-9_.-]+` runs become `_`).
pub fn install_subdirs(installs: &[PathBuf]) -> Vec<String> {
    let mut out = Vec::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    for root in installs {
        let raw = normpath_basename(root);
        let mut base = String::new();
        let mut in_run = false;
        for c in raw.chars() {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                base.push(c);
                in_run = false;
            } else if !in_run {
                base.push('_');
                in_run = true;
            }
        }
        if base.is_empty() {
            base = "install".to_owned();
        }
        let (mut name, mut n) = (base.clone(), 1);
        while used.contains(&py_lower(&name)) {
            n += 1;
            name = format!("{base}_{n}");
        }
        used.insert(py_lower(&name));
        out.push(name);
    }
    out
}

/// The engine lookup of `main`: the first root (`engine_root`, else each install in turn) whose
/// `Armada2.exe` and `armada2.map` give vtables, checked with `check_engine`. An unreadable or
/// undecodable root is skipped like the Python's `except (OSError, DecodeError)`; a raw fault
/// aborts. `Ok(None)` means no engine names (calls are written as `slot_<n>`); a mismatch of the
/// slots the decoder relies on is the Python's `sys.exit` and an `Err` of kind `Decode`.
pub fn load_engine(installs: &[PathBuf], engine_root: Option<&Path>) -> Result<Option<Vtables>> {
    let candidates: Vec<&Path> = match engine_root {
        Some(r) => vec![r],
        None => installs.iter().map(PathBuf::as_path).collect(),
    };
    let mut engine: Option<Vtables> = None;
    for root in candidates {
        engine = match engine_vtables(root, "armada2.exe", "armada2.map", &ENGINE_CLASSES) {
            Ok(v) => v,
            Err(e) if e.kind != ErrorKind::Raw => None,
            Err(e) => return Err(e),
        };
        // `if engine:`: an empty dictionary is false
        if engine.as_ref().is_some_and(|v| !v.is_empty()) {
            break;
        }
    }
    let engine = engine.filter(|v| !v.is_empty());
    if let Some(vt) = &engine {
        let bad = check_engine(vt, &ENGINE_WANT, true);
        if !bad.is_empty() {
            return Err(DecodeError::new(format!(
                "rule-engine slots differ from the ones this decoder was measured on:\n  {}",
                bad.join("\n  ")
            )));
        }
    }
    Ok(engine)
}

/// What [`decode_install`] found: the scaffolds by file name (`<mission>.lua`, a later file of the
/// same name replaces an earlier one, as the Python overwrites), and the counts of `main`.
#[derive(Debug, Default)]
pub struct InstallRun {
    pub files: BTreeMap<String, Vec<u8>>,
    /// DLLs read.
    pub read: usize,
    /// DLLs `main` counts as failed (a `DecodeError`, or a condition not decoded).
    pub failed: usize,
    /// DLLs of another shape.
    pub skipped: usize,
}

/// The loop of `main` for one install root, in memory: [`find_drls`], then per file `decode` and
/// `scaffold`. `listing` gives the instruction listing of a DLL (the Python runs `objdump`); it is
/// called after the file's image was read, as the Python's `Drl.__init__` does. A raw fault (or an
/// unreadable file) aborts the run, as in the Python.
pub fn decode_install(
    root: &Path,
    engine: Option<&Vtables>,
    listing: &mut dyn FnMut(&Path) -> Result<Vec<Ins>>,
) -> Result<InstallRun> {
    let mut run = InstallRun::default();
    for f in find_drls(root) {
        run.read += 1;
        let base = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let decoded = (|| -> std::result::Result<Decoded, Failure> {
            let pe = Pe::open(&f)?;
            let ins = listing(&f)?;
            decode(pe, ins, &base, engine)
        })();
        match decoded {
            Ok(m) => {
                if !m.errors.is_empty() {
                    run.failed += 1;
                }
                run.files.insert(format!("{}.lua", m.mission), scaffold(&m).into_bytes());
            }
            Err(Failure::Unsupported(_)) => run.skipped += 1,
            Err(Failure::Error(e)) if e.kind == ErrorKind::Decode => run.failed += 1,
            Err(Failure::Error(e)) => return Err(e),
        }
    }
    Ok(run)
}
