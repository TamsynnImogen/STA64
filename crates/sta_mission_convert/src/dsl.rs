//! The parts of a mission DLL the lifter reads (LUA-15d): the `Dsl` class of
//! `tools/dsl_to_lua.py` (thunks, exported data, the recognisers of compiler and C runtime
//! routines and of MSVC 6 `std::vector` members) and the small functions around it.
//!
//! The listing is the input the Python tool gets from `objdump -d -M intel --no-show-raw-insn`:
//! one `(address, text)` per instruction ([`Ins`]), the text trimmed. Every recogniser reads the
//! text by exact shape, as the Python regexes do; the patterns are hand-written matchers in
//! [`pat`] (no regex crate), each named for the Python pattern it stands for.
//!
//! Addresses are `i64` where the Python holds an `int` that can come from operand text (jump
//! targets, immediates), so the values the lifter puts into [`Py`] trees are the Python's.
//! The Python's dict and set containers keep their order where the output depends on it
//! (`IndexMap`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::disasm::Ins;
use crate::format::py_hex;
use crate::image::Image;
use crate::map::Vtables;
use crate::msvc::{data_kind, plain_name};
use crate::pyval::{first_max_key, py_int, py_isspace, py_split_ws_once};
use crate::{DecodeError, Result};

/// `REG32` of `dsl_to_lua.py`.
pub const REG32: [&str; 6] = ["eax", "ebx", "ecx", "edx", "esi", "edi"];

/// `x in REG32`.
pub fn is_reg32(x: &str) -> bool {
    REG32.contains(&x)
}

/// `REG8[x]`: the 32-bit register a byte or word register name stands for.
pub fn reg8(x: &str) -> Option<&'static str> {
    Some(match x {
        "al" | "ah" | "ax" => "eax",
        "bl" => "ebx",
        "cl" | "cx" => "ecx",
        "dl" | "dx" => "edx",
        _ => return None,
    })
}

/// `JCC` of `dsl_to_lua.py`: the comparison a conditional jump of the mission lifter tests
/// (the larger table; the rule decoder's small one lives in `algebra`).
pub fn jcc(mn: &str) -> Option<&'static str> {
    Some(match mn {
        "je" => "==",
        "jne" => "~=",
        "jl" => "<",
        "jge" => ">=",
        "jle" => "<=",
        "jg" => ">",
        "jb" => "<",
        "jae" => ">=",
        "jbe" => "<=",
        "ja" => ">",
        "js" => "<",
        "jns" => ">=",
        _ => return None,
    })
}

/// `SETCC` of `dsl_to_lua.py`: `x in SETCC`.
pub fn is_setcc(mn: &str) -> bool {
    matches!(mn, "sete" | "setne" | "setl" | "setge" | "setle" | "setg" | "setb" | "setae" | "setbe" | "seta")
}

/// `VAR_SLOTS[slot]`: the operation of a rule variable's vtable slot.
pub fn var_op(slot: i64) -> Option<&'static str> {
    Some(match slot {
        4 => "is_dirty",
        5 => "set_clean",
        6 => "mark_dirty",
        7 => "set",
        8 => "get",
        9 => "start",
        10 => "stop",
        11 => "update",
        12 => "time",
        _ => return None,
    })
}

// ------------------------------------------------------------------ pattern matchers

/// Hand-written matchers for the Python regexes the recognisers and the lifter use. A matcher
/// takes the whole text and answers like `re.fullmatch` (or `re.match` with a trailing `$`: the
/// texts are trimmed, so the two agree). `\s` is Python's whitespace, `\w` the Unicode word
/// class.
pub(crate) mod pat {
    use super::py_isspace;

    pub(crate) fn is_w(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    /// A cursor over the text.
    pub(crate) struct M<'a> {
        pub s: &'a str,
        pub p: usize,
    }

    impl<'a> M<'a> {
        pub fn new(s: &'a str) -> Self {
            M { s, p: 0 }
        }
        pub fn rest(&self) -> &'a str {
            &self.s[self.p..]
        }
        /// A literal.
        pub fn lit(&mut self, l: &str) -> bool {
            if self.rest().starts_with(l) {
                self.p += l.len();
                true
            } else {
                false
            }
        }
        /// `\s+`.
        pub fn ws1(&mut self) -> bool {
            let n: usize = self.rest().chars().take_while(|&c| py_isspace(c)).map(char::len_utf8).sum();
            self.p += n;
            n > 0
        }
        /// `\w+`.
        pub fn w1(&mut self) -> Option<&'a str> {
            let n: usize = self.rest().chars().take_while(|&c| is_w(c)).map(char::len_utf8).sum();
            let out = &self.rest()[..n];
            self.p += n;
            (n > 0).then_some(out)
        }
        /// `e\w\w`: a three letter register name.
        pub fn reg3(&mut self) -> Option<&'a str> {
            let r = self.rest();
            let mut it = r.chars();
            if it.next() != Some('e') {
                return None;
            }
            let (c1, c2) = (it.next()?, it.next()?);
            if !is_w(c1) || !is_w(c2) {
                return None;
            }
            let n = 1 + c1.len_utf8() + c2.len_utf8();
            self.p += n;
            Some(&r[..n])
        }
        /// `[0-9a-f]+`.
        pub fn hex1(&mut self) -> Option<&'a str> {
            let n = self.rest().bytes().take_while(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')).count();
            let out = &self.rest()[..n];
            self.p += n;
            (n > 0).then_some(out)
        }
        /// `0x[0-9a-f]+`: the digits with their prefix.
        pub fn hex0x(&mut self) -> Option<&'a str> {
            let start = self.p;
            if !self.lit("0x") || self.hex1().is_none() {
                self.p = start;
                return None;
            }
            Some(&self.s[start..self.p])
        }
        /// `[+-]0x[0-9a-f]+`: the sign, prefix and digits.
        pub fn signed_hex0x(&mut self) -> Option<&'a str> {
            let start = self.p;
            if !(self.lit("+") || self.lit("-")) || self.hex0x().is_none() {
                self.p = start;
                return None;
            }
            Some(&self.s[start..self.p])
        }
        /// Python's `$` (the end, or before a final newline).
        pub fn end(&self) -> bool {
            let r = self.rest();
            r.is_empty() || r == "\n"
        }
        /// The end exactly (`re.fullmatch`).
        pub fn end_exact(&self) -> bool {
            self.rest().is_empty()
        }
    }

    /// `jmp\s+0x([0-9a-f]+)$` (`re.match`): the hex digits.
    pub(crate) fn jmp_abs(t: &str) -> Option<&str> {
        let mut m = M::new(t);
        if !(m.lit("jmp") && m.ws1() && m.lit("0x")) {
            return None;
        }
        let h = m.hex1()?;
        m.end().then_some(h)
    }

    /// `call\s+0x([0-9a-f]+)` (`fullmatch`, or `match` with `$`): the hex digits.
    pub(crate) fn call_abs(t: &str) -> Option<&str> {
        let mut m = M::new(t);
        if !(m.lit("call") && m.ws1() && m.lit("0x")) {
            return None;
        }
        let h = m.hex1()?;
        m.end().then_some(h)
    }

    /// `(j\w+)\s+0x([0-9a-f]+)$` (`re.match`): the mnemonic and the hex digits.
    pub(crate) fn jcc_abs(t: &str) -> Option<(&str, &str)> {
        let mut m = M::new(t);
        if !m.lit("j") {
            return None;
        }
        let w = m.w1()?;
        let mn = &t[..1 + w.len()];
        if !(m.ws1() && m.lit("0x")) {
            return None;
        }
        let h = m.hex1()?;
        m.end().then_some((mn, h))
    }

    /// `cmp\s+e(si|di|bp),esp$` (`re.match`).
    pub(crate) fn cmp_frame_esp(t: &str) -> bool {
        let mut m = M::new(t);
        if !(m.lit("cmp") && m.ws1() && m.lit("e")) {
            return false;
        }
        if !(m.lit("si") || m.lit("di") || m.lit("bp")) {
            return false;
        }
        m.lit(",esp") && m.end()
    }

    /// `jmp\s+DWORD PTR \[(e\w\w)\*4\+(0x[0-9a-f]+)\]` (`fullmatch`): register and table.
    pub(crate) fn jmp_table(t: &str) -> Option<(&str, &str)> {
        let mut m = M::new(t);
        if !(m.lit("jmp") && m.ws1() && m.lit("DWORD PTR [")) {
            return None;
        }
        let r = m.reg3()?;
        if !m.lit("*4+") {
            return None;
        }
        let h = m.hex0x()?;
        (m.lit("]") && m.end_exact()).then_some((r, h))
    }

    /// `DWORD PTR \[(e\w\w)\*4\+0x[0-9a-f]+\]` (`fullmatch`): the register.
    pub(crate) fn dword_index4(t: &str) -> Option<&str> {
        let mut m = M::new(t);
        if !m.lit("DWORD PTR [") {
            return None;
        }
        let r = m.reg3()?;
        if !(m.lit("*4+") && m.hex0x().is_some()) {
            return None;
        }
        (m.lit("]") && m.end_exact()).then_some(r)
    }

    /// `cmp\s+.*,(0x[0-9a-f]+)` (`fullmatch`): the immediate after the last comma.
    pub(crate) fn cmp_bound(t: &str) -> Option<&str> {
        let mut m = M::new(t);
        if !m.lit("cmp") {
            return None;
        }
        // `\s+` takes at least one whitespace character, `.*` the rest up to the last comma.
        let first = m.rest().chars().next()?;
        if !py_isspace(first) {
            return None;
        }
        let after = &m.rest()[first.len_utf8()..];
        if after.contains('\n') {
            return None; // `.` does not match a newline
        }
        let comma = after.rfind(',')?;
        let imm = &after[comma + 1..];
        let mut n = M::new(imm);
        n.hex0x()?;
        n.end_exact().then_some(imm)
    }

    /// `mov\s+([abcd]l),BYTE PTR \[(e\w\w)\+(0x[0-9a-f]+)\]` (`fullmatch`).
    pub(crate) fn byte_load(t: &str) -> Option<(&str, &str, &str)> {
        let mut m = M::new(t);
        if !(m.lit("mov") && m.ws1()) {
            return None;
        }
        let r = m.rest();
        let rl = r.get(..2)?;
        if !(matches!(rl, "al" | "bl" | "cl" | "dl")) {
            return None;
        }
        m.p += 2;
        if !m.lit(",BYTE PTR [") {
            return None;
        }
        let reg = m.reg3()?;
        if !m.lit("+") {
            return None;
        }
        let h = m.hex0x()?;
        (m.lit("]") && m.end_exact()).then_some((rl, reg, h))
    }

    /// `f(?!i)\w+\s+DWORD PTR \[ebp([+-]0x[0-9a-f]+)\]` (`fullmatch`): the signed offset.
    pub(crate) fn float_slot(t: &str) -> Option<&str> {
        let mut m = M::new(t);
        if !m.lit("f") || m.rest().starts_with('i') {
            return None;
        }
        m.w1()?;
        if !(m.ws1() && m.lit("DWORD PTR [ebp")) {
            return None;
        }
        let o = m.signed_hex0x()?;
        (m.lit("]") && m.end_exact()).then_some(o)
    }

    /// `0x[0-9a-f]+` (`fullmatch`).
    pub(crate) fn is_hex0x(t: &str) -> bool {
        let mut m = M::new(t);
        m.hex0x().is_some() && m.end_exact()
    }

    /// `-?0x[0-9a-f]+` (`fullmatch`).
    pub(crate) fn is_neg_hex0x(t: &str) -> bool {
        is_hex0x(t.strip_prefix('-').unwrap_or(t))
    }

    /// `\d+` (`fullmatch`; ASCII digits are all a latin-1 text has in the class).
    pub(crate) fn is_digits(t: &str) -> bool {
        !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
    }

    /// `st\((\d)\)` (`fullmatch`): the digit.
    pub(crate) fn st_n(t: &str) -> Option<i64> {
        let b = t.strip_prefix("st(")?.strip_suffix(')')?;
        let mut c = b.chars();
        let d = c.next()?;
        (d.is_ascii_digit() && c.next().is_none()).then(|| i64::from(d.to_digit(10).unwrap_or(0)))
    }

    /// `st\((\d)\),st` (`fullmatch`): the digit.
    pub(crate) fn st_n_st(t: &str) -> Option<i64> {
        st_n(t.strip_suffix(",st")?)
    }

    /// `(DWORD|QWORD) PTR ds:(0x[0-9a-f]+)` (`fullmatch`): the size word and the address digits.
    pub(crate) fn sized_ds(t: &str) -> Option<(&str, &str)> {
        let (size, rest) = if let Some(r) = t.strip_prefix("DWORD PTR ds:") {
            ("DWORD", r)
        } else {
            ("QWORD", t.strip_prefix("QWORD PTR ds:")?)
        };
        is_hex0x(rest).then_some((size, rest))
    }

    /// `ds:(0x[0-9a-f]+)` (`fullmatch`).
    pub(crate) fn ds_abs(t: &str) -> Option<&str> {
        let r = t.strip_prefix("ds:")?;
        is_hex0x(r).then_some(r)
    }

    /// `\[ebp([+-]0x[0-9a-f]+)\]` (`fullmatch`).
    pub(crate) fn ebp_disp(t: &str) -> Option<&str> {
        let mut m = M::new(t);
        if !m.lit("[ebp") {
            return None;
        }
        let o = m.signed_hex0x()?;
        (m.lit("]") && m.end_exact()).then_some(o)
    }

    /// `\[(e\w\w)\]` (`fullmatch`).
    pub(crate) fn reg_ind(t: &str) -> Option<&str> {
        let mut m = M::new(t);
        if !m.lit("[") {
            return None;
        }
        let r = m.reg3()?;
        (m.lit("]") && m.end_exact()).then_some(r)
    }

    /// `\[(e\w\w)([+-]0x[0-9a-f]+)\]` (`fullmatch`).
    pub(crate) fn reg_disp(t: &str) -> Option<(&str, &str)> {
        let mut m = M::new(t);
        if !m.lit("[") {
            return None;
        }
        let r = m.reg3()?;
        let o = m.signed_hex0x()?;
        (m.lit("]") && m.end_exact()).then_some((r, o))
    }

    /// `\[(e\w\w)\*4\+(0x[0-9a-f]+)\]` (`fullmatch`).
    pub(crate) fn reg4_table(t: &str) -> Option<(&str, &str)> {
        let mut m = M::new(t);
        if !m.lit("[") {
            return None;
        }
        let r = m.reg3()?;
        if !m.lit("*4+") {
            return None;
        }
        let h = m.hex0x()?;
        (m.lit("]") && m.end_exact()).then_some((r, h))
    }

    /// `\[ebp\+(e\w\w)\*4([+-]0x[0-9a-f]+)\]` (`fullmatch`).
    pub(crate) fn ebp_index4(t: &str) -> Option<(&str, &str)> {
        let mut m = M::new(t);
        if !m.lit("[ebp+") {
            return None;
        }
        let r = m.reg3()?;
        if !m.lit("*4") {
            return None;
        }
        let o = m.signed_hex0x()?;
        (m.lit("]") && m.end_exact()).then_some((r, o))
    }

    /// `\[(e\w\w)\+(e\w\w)\*4\]` (`fullmatch`).
    pub(crate) fn reg_plus_index4(t: &str) -> bool {
        let mut m = M::new(t);
        m.lit("[") && m.reg3().is_some() && m.lit("+") && m.reg3().is_some() && m.lit("*4]") && m.end_exact()
    }

    /// `(?:DWORD|BYTE|WORD|QWORD) PTR ` at the start: the rest after the size word.
    pub(crate) fn strip_ptr(t: &str) -> &str {
        for p in ["DWORD PTR ", "BYTE PTR ", "WORD PTR ", "QWORD PTR "] {
            if let Some(r) = t.strip_prefix(p) {
                return r;
            }
        }
        t
    }

    /// `(?:DWORD|QWORD|BYTE|WORD) PTR \[esp(?:\+(0x[0-9a-f]+))?\]` (`fullmatch`): `Some(None)` for
    /// the bare form, `Some(Some(digits))` with an offset.
    pub(crate) fn esp_slot(t: &str) -> Option<Option<&str>> {
        let r = strip_ptr(t);
        if r.len() == t.len() {
            return None;
        }
        let mut m = M::new(r);
        if !m.lit("[esp") {
            return None;
        }
        let off = if m.lit("+") { Some(m.hex0x()?) } else { None };
        (m.lit("]") && m.end_exact()).then_some(off)
    }

    /// `add\s+esp,(0x[0-9a-f]+)` (`fullmatch`): the digits.
    pub(crate) fn add_esp(t: &str) -> Option<&str> {
        let mut m = M::new(t);
        if !(m.lit("add") && m.ws1() && m.lit("esp,")) {
            return None;
        }
        let h = m.hex0x()?;
        m.end_exact().then_some(h)
    }

    /// `re.sub(r"^(BYTE|WORD) PTR ", "", s)`.
    pub(crate) fn strip_byte_word_ptr(t: &str) -> &str {
        t.strip_prefix("BYTE PTR ").or_else(|| t.strip_prefix("WORD PTR ")).unwrap_or(t)
    }

    /// `re.sub(r"\s+", " ", t)`.
    pub(crate) fn squeeze_ws(t: &str) -> String {
        let mut out = String::with_capacity(t.len());
        let mut in_ws = false;
        for c in t.chars() {
            if py_isspace(c) {
                if !in_ws {
                    out.push(' ');
                }
                in_ws = true;
            } else {
                out.push(c);
                in_ws = false;
            }
        }
        out
    }

    /// `re.sub(r"0x[0-9a-f]{8}\b", "A", t)`.
    pub(crate) fn replace_hex8(t: &str) -> String {
        let cs: Vec<char> = t.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < cs.len() {
            if let Some(end) = hex_run(&cs, i, 8)
                && cs.get(end).is_none_or(|&c| !is_w(c))
            {
                out.push('A');
                i = end;
                continue;
            }
            out.push(cs[i]);
            i += 1;
        }
        out
    }

    /// `0x` and `n` hex digits at `i`: the index after them.
    fn hex_run(cs: &[char], i: usize, n: usize) -> Option<usize> {
        if cs.get(i) != Some(&'0') || cs.get(i + 1) != Some(&'x') {
            return None;
        }
        let digits = cs.get(i + 2..i + 2 + n)?;
        digits.iter().all(|c| matches!(c, '0'..='9' | 'a'..='f')).then_some(i + 2 + n)
    }

    /// `re.finditer(r"0x1[0-9a-f]{7}\b", ops)`: char index and value of each match.
    pub(crate) fn find_1xxxxxxx(ops: &[char]) -> Vec<(usize, i64)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < ops.len() {
            if ops.get(i + 2) == Some(&'1')
                && let Some(end) = hex_run(ops, i, 8)
                && ops.get(end).is_none_or(|&c| !is_w(c))
            {
                let digits: String = ops[i + 2..end].iter().collect();
                out.push((i, i64::from_str_radix(&digits, 16).unwrap_or(0)));
                i = end;
                continue;
            }
            i += 1;
        }
        out
    }

    /// `mov\s+DWORD PTR \[e\w\w\+(0xc|0x4)\],0x(42|7fffffff)` shapes of `helper_kind`'s sprintf
    /// test: `mov\s+DWORD PTR \[e\w\w\+OFF\],IMM` (`fullmatch`).
    pub(crate) fn mov_dword_reg_off_imm(t: &str, off: &str, imm: &str) -> bool {
        let mut m = M::new(t);
        if !(m.lit("mov") && m.ws1() && m.lit("DWORD PTR [") && m.reg3().is_some() && m.lit("+")) {
            return false;
        }
        m.lit(off) && m.lit("],") && m.lit(imm) && m.end_exact()
    }

    /// `DWORD PTR \[ebp\+(0x8|0xc|0x10)\]` (`re.search`): the first match's digits.
    pub(crate) fn search_ebp_arg(t: &str) -> Option<&'static str> {
        let key = "DWORD PTR [ebp+";
        let mut from = 0;
        while let Some(p) = t[from..].find(key) {
            let rest = &t[from + p + key.len()..];
            for alt in ["0x8", "0xc", "0x10"] {
                if let Some(r) = rest.strip_prefix(alt)
                    && r.starts_with(']')
                {
                    return Some(alt);
                }
            }
            from += p + 1;
        }
        None
    }

    /// `mov\s+DWORD PTR \[(?!ebp)e\w\w(?:\+(0x[0-9a-f]+))?\],e\w\w` (`fullmatch`): the offset
    /// (`Some(None)` for the form without one).
    pub(crate) fn vec3_store(t: &str) -> Option<Option<&str>> {
        let mut m = M::new(t);
        if !(m.lit("mov") && m.ws1() && m.lit("DWORD PTR [")) {
            return None;
        }
        if m.rest().starts_with("ebp") {
            return None;
        }
        m.reg3()?;
        let off = if m.rest().starts_with('+') {
            let save = m.p;
            m.p += 1;
            match m.hex0x() {
                Some(h) => Some(h),
                None => {
                    m.p = save;
                    None
                }
            }
        } else {
            None
        };
        if !m.lit("],") {
            return None;
        }
        m.reg3()?;
        m.end_exact().then_some(off)
    }
}

// ------------------------------------------------------------------ the Dsl

/// One exported datum: `(plain name, kind, size, mangled name)` (the Python tuple of `Dsl.data`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataItem {
    pub name: String,
    pub kind: &'static str,
    /// The gap to the next exported datum, at most 4096 (the last one has 4).
    pub size: i64,
    pub mangled: String,
}

/// How an unexported global is resolved (`Dsl.static_class`).
pub type StaticClass = &'static str;

/// `Dsl`: the parts of a mission DLL the lifter reads.
///
/// The caches the Python object keeps in attributes (`_ftol`, `_vec`, `_kinds`, `_statics`,
/// `_sites`, `static_refs`) are `RefCell`s, so the lifter can hold a shared reference.
pub struct Dsl<'a> {
    pub pe: &'a dyn Image,
    /// `{class: [mangled method per vtable slot]}` (empty when no engine names were found).
    pub engine: Vtables,
    pub ins: Vec<Ins>,
    /// Address to index into `ins` (a later duplicate address replaces an earlier one).
    pub at: HashMap<i64, usize>,
    pub exports: IndexMap<String, u32>,
    pub thunks: HashMap<i64, i64>,
    pub chkesp: Option<i64>,
    /// Exported data by address.
    pub data: IndexMap<i64, DataItem>,
    /// Entry address (thunks followed) to plain name, for exported code.
    pub code_names: IndexMap<i64, String>,
    ftol: RefCell<HashMap<i64, bool>>,
    vec: RefCell<HashMap<i64, Option<&'static str>>>,
    kinds: RefCell<HashMap<i64, Option<&'static str>>>,
    statics: RefCell<Option<(HashSet<i64>, HashSet<i64>)>>,
    sites: RefCell<Option<HashMap<i64, Vec<usize>>>>,
    /// Unexported data the lifted code touched: address to class (`word`, `str`, `rodata`,
    /// `const`), in first-touched order.
    pub static_refs: RefCell<IndexMap<i64, &'static str>>,
}

impl<'a> Dsl<'a> {
    /// `Dsl.__init__`: `exports` is `read_exports(pe)` ([`crate::Pe::exports`]), `ins` the
    /// listing. A DecodeError for an exported code address with no instruction (`entry`).
    pub fn new(pe: &'a dyn Image, engine: Vtables, ins: Vec<Ins>, exports: IndexMap<String, u32>) -> Result<Self> {
        let mut at = HashMap::new();
        for (i, (a, _)) in ins.iter().enumerate() {
            at.insert(i64::from(*a), i);
        }
        let mut thunks = HashMap::new();
        for (a, t) in &ins {
            if t == "int3" {
                continue;
            }
            let Some(h) = pat::jmp_abs(t) else { break };
            thunks.insert(i64::from(*a), py_int(h, 16)?);
        }
        let chkesp = chkesp(&ins)?;
        let mut d = Dsl {
            pe,
            engine,
            ins,
            at,
            exports,
            thunks,
            chkesp,
            data: IndexMap::new(),
            code_names: IndexMap::new(),
            ftol: RefCell::default(),
            vec: RefCell::default(),
            kinds: RefCell::default(),
            statics: RefCell::new(None),
            sites: RefCell::new(None),
            static_refs: RefCell::default(),
        };
        // Exported data: address -> (plain name, kind, size, mangled)
        let mut datas: Vec<(i64, String)> = Vec::new();
        for (n, a) in &d.exports {
            if d.section_of(i64::from(*a)).as_deref() != Some(".text") {
                datas.push((i64::from(*a), n.clone()));
            }
        }
        datas.sort(); // tuples (address, name): the same order as Python's sorted()
        for (i, (a, n)) in datas.iter().enumerate() {
            let size = if i + 1 < datas.len() { datas[i + 1].0 - a } else { 4 };
            d.data.insert(
                *a,
                DataItem { name: plain_name(n), kind: data_kind(n), size: size.min(4096), mangled: n.clone() },
            );
        }
        let exports: Vec<(String, u32)> = d.exports.iter().map(|(n, a)| (n.clone(), *a)).collect();
        for (n, a) in exports {
            if d.section_of(i64::from(a)).as_deref() == Some(".text") {
                let e = d.entry(i64::from(a))?;
                d.code_names.insert(e, plain_name(&n));
            }
        }
        Ok(d)
    }

    /// The address of instruction `i`.
    pub fn addr_at(&self, i: usize) -> i64 {
        i64::from(self.ins[i].0)
    }

    /// The text of instruction `i`.
    pub fn text_at(&self, i: usize) -> &str {
        &self.ins[i].1
    }

    /// `d.at.get(a)`.
    pub fn index_of(&self, a: i64) -> Option<usize> {
        self.at.get(&a).copied()
    }

    /// `d.pe.section_of(addr)` for a Python integer (an address outside 32 bits is in no section).
    pub fn section_of(&self, addr: i64) -> Option<String> {
        let a = u32::try_from(addr).ok()?;
        self.pe.section_of(a).map(str::to_owned)
    }

    /// `d.pe.cstr(addr)`.
    pub fn cstr(&self, addr: i64) -> Result<String> {
        match u32::try_from(addr) {
            Ok(a) => self.pe.cstr(a),
            Err(_) => Err(DecodeError::new(format!("string at {} outside the file", py_hex(addr)))),
        }
    }

    /// `d.pe.f32(addr)`, widened to a double.
    pub fn f32(&self, addr: i64) -> Result<f64> {
        match u32::try_from(addr) {
            Ok(a) => Ok(f64::from(self.pe.f32(a)?)),
            Err(_) => Err(DecodeError::new(format!("float at {} outside the file", py_hex(addr)))),
        }
    }

    /// `d.pe.f64(addr)`.
    pub fn f64(&self, addr: i64) -> Result<f64> {
        match u32::try_from(addr) {
            Ok(a) => self.pe.f64(a),
            Err(_) => Err(DecodeError::new(format!("double at {} outside the file", py_hex(addr)))),
        }
    }

    /// `Dsl.dword`: the 32-bit word the file image holds at `addr` (`None` outside it).
    pub fn dword(&self, addr: i64) -> Option<i64> {
        u32::try_from(addr).ok().and_then(|a| self.pe.dword(a)).map(i64::from)
    }

    /// `Dsl.static_class`: how an unexported global is resolved (the lifter never leaves one
    /// unnamed): `"rodata"` in a read-only section (the image's word is the value), `"const"` in
    /// `.data`/`.bss` and never written or address-taken by the DLL's code (the image's initial
    /// word is the value), `"mutable"` written or handed out (a slot of the script's `G` table,
    /// started from the image's initial word).
    pub fn static_class(&self, addr: i64) -> StaticClass {
        if self.statics.borrow().is_none() {
            let (mut written, mut taken) = (HashSet::new(), HashSet::new());
            for (_, t) in &self.ins {
                // `mn, _, ops = t.partition(" ")`
                let (mn, ops) = match t.split_once(' ') {
                    Some((m, o)) => (m, o),
                    None => (t.as_str(), ""),
                };
                let ops: Vec<char> = crate::pyval::py_strip(ops).chars().collect();
                if ops.is_empty() || mn == "call" || mn == "jmp" || mn.starts_with('j') {
                    continue;
                }
                // `first, _, rest = ops.partition(",")`
                let first_len = ops.iter().position(|&c| c == ',').unwrap_or(ops.len());
                for (start, n) in pat::find_1xxxxxxx(&ops) {
                    let in_first = start < first_len;
                    let lo = start.saturating_sub(3);
                    let near: String = ops[lo..start].iter().collect();
                    let before = &ops[..start];
                    let after_comma = match before.iter().rposition(|&c| c == ',') {
                        Some(p) => &before[p + 1..],
                        None => before,
                    };
                    let memory = near.contains("ds:") || after_comma.contains(&'[');
                    if memory
                        && in_first
                        && !matches!(
                            mn,
                            "cmp" | "test" | "push" | "fld" | "fild" | "fcom" | "fcomp" | "fadd" | "fsub" | "fmul" | "fdiv" | "fsubr" | "fdivr" | "fcmp"
                        )
                    {
                        written.insert(n);
                    } else if !memory {
                        taken.insert(n);
                    }
                }
            }
            *self.statics.borrow_mut() = Some((written, taken));
        }
        let st = self.statics.borrow();
        let (written, taken) = st.as_ref().expect("computed above");
        if self.section_of(addr).as_deref() == Some(".rdata") {
            return "rodata";
        }
        if written.contains(&addr) || taken.contains(&addr) {
            return "mutable";
        }
        "const"
    }

    /// `Dsl.helper_kind`: a routine of the C runtime or the compiler the mission code calls,
    /// recognised by what it does (never by address), so it is modelled instead of lifted:
    /// `sprintf`, `strcmp`, `vec3` (the Vector3 constructor from three floats), `report` (the
    /// debug assertion reporter) and `atexit`.
    pub fn helper_kind(&self, addr: i64) -> Result<Option<&'static str>> {
        if let Some(k) = self.kinds.borrow().get(&addr) {
            return Ok(*k);
        }
        self.kinds.borrow_mut().insert(addr, None);
        let Some(i) = self.index_of(addr) else { return Ok(None) };
        let mut body: Vec<&str> = Vec::new();
        for (_, t) in self.ins.iter().skip(i).take(400) {
            body.push(t);
            if t.starts_with("ret") {
                break;
            }
        }
        let head = &body[..body.len().min(70)];
        let mut kind: Option<&'static str> = None;
        if head.iter().any(|t| pat::mov_dword_reg_off_imm(t, "0xc", "0x42"))
            && head.iter().any(|t| pat::mov_dword_reg_off_imm(t, "0x4", "0x7fffffff"))
        {
            kind = Some("sprintf");
        } else if body.len() > 3
            && body[0].starts_with("mov    edx,DWORD PTR [esp+0x4]")
            && body[1].starts_with("mov    ecx,DWORD PTR [esp+0x8]")
            && body.contains(&"cmp    al,BYTE PTR [ecx]")
        {
            kind = Some("strcmp");
        } else if body.last() == Some(&"ret    0xc") {
            let loads: HashSet<&str> = body.iter().filter_map(|t| pat::search_ebp_arg(t)).collect();
            let mut offs: Vec<i64> = Vec::new();
            for t in &body {
                if let Some(o) = pat::vec3_store(t) {
                    offs.push(match o {
                        Some(h) => py_int(h, 16)?,
                        None => 0,
                    });
                }
            }
            offs.sort_unstable();
            let want: HashSet<&str> = ["0x8", "0xc", "0x10"].into_iter().collect();
            if loads == want && offs == [0, 4, 8] && !body.iter().any(|t| t.starts_with("call") || t.starts_with('f')) {
                kind = Some("vec3");
            }
        }
        if kind.is_none() && body.len() == 12 && body[..2] == ["push   ebp", "mov    ebp,esp"] {
            let calls: Vec<&&str> = body.iter().filter(|t| t.starts_with("call")).collect();
            let abs_calls: Vec<&&str> = body.iter().filter(|t| pat::call_abs(t).is_some()).collect();
            if calls == abs_calls
                && calls.len() == 1
                && body[5..10] == ["add    esp,0x4", "neg    eax", "sbb    eax,eax", "neg    eax", "dec    eax"]
            {
                kind = Some("atexit"); // the C runtime's `atexit`: registers a function to run at unload
            }
        }
        if kind.is_none() && self.ins_text(i, 400).iter().any(|t| t.starts_with("stos   WORD PTR es:[edi]")) {
            let sites = self.call_sites(addr)?;
            if sites.len() >= 10 && sites.iter().all(|&j| self.is_assert_site(j)) {
                kind = Some("report");
            }
        }
        self.kinds.borrow_mut().insert(addr, kind);
        Ok(kind)
    }

    /// `Dsl.vector_kind`: a member of MSVC 6's `std::vector<T>` (debug build, 4-byte elements:
    /// the layout `{allocator, first, last, end}`), recognised by the instructions of its body,
    /// never by address: `ctor`, `dtor`, `begin`, `end`, `size`, `index` (`operator[]`),
    /// `push_back`, `erase` (one iterator), `clear`. The wrappers are matched together with the
    /// members they call (`operator[]` calls `begin`, `push_back` calls `end`).
    pub fn vector_kind(&self, addr: i64) -> Result<Option<&'static str>> {
        let addr = self.thunks.get(&addr).copied().unwrap_or(addr);
        if let Some(k) = self.vec.borrow().get(&addr) {
            return Ok(*k);
        }
        self.vec.borrow_mut().insert(addr, None);
        let Some(i) = self.index_of(addr) else { return Ok(None) };
        let text: Vec<String> = self.ins_text(i, 120).iter().map(|t| pat::squeeze_ws(t)).collect();
        let Some(lo) = text.iter().position(|t| t == "mov DWORD PTR [ebp-0x4],ecx").map(|p| p + 1) else {
            return Ok(None);
        };
        let Some(hi) = text.iter().skip(lo).position(|t| t == "pop edi").map(|p| p + lo) else {
            return Ok(None);
        };
        let core = &text[lo..hi];
        let ret: &str = text[hi..].iter().find(|t| t.starts_with("ret")).map_or("", |s| s.as_str());
        let mut callees: Vec<Option<&'static str>> = Vec::new();
        for t in core {
            if let Some(h) = t.strip_prefix("call 0x")
                && !h.is_empty()
                && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                callees.push(self.vector_kind(py_int(h, 16)?)?);
            }
        }
        let shape: Vec<String> = core.iter().map(|t| pat::replace_hex8(t)).collect();
        let shape: Vec<&str> = shape.iter().map(String::as_str).collect();
        let this = "mov eax,DWORD PTR [ebp-0x4]";
        let callee0 = || -> Result<Option<&'static str>> {
            callees.first().copied().ok_or_else(|| DecodeError::raw("list index out of range"))
        };
        let mut kind: Option<&'static str> = None;
        if shape == [this, "mov eax,DWORD PTR [eax+0x4]"] && ret == "ret" {
            kind = Some("begin");
        } else if shape == [this, "mov eax,DWORD PTR [eax+0x8]"] && ret == "ret" {
            kind = Some("end");
        } else if shape
            == [
                this,
                "cmp DWORD PTR [eax+0x4],0x0",
                "jne A",
                "mov DWORD PTR [ebp-0x8],0x0",
                "jmp A",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "mov edx,DWORD PTR [ebp-0x4]",
                "mov eax,DWORD PTR [ecx+0x8]",
                "sub eax,DWORD PTR [edx+0x4]",
                "sar eax,0x2",
                "mov DWORD PTR [ebp-0x8],eax",
                "mov eax,DWORD PTR [ebp-0x8]",
            ]
            && ret == "ret"
        {
            kind = Some("size");
        } else if shape == ["mov ecx,DWORD PTR [ebp-0x4]", "call A", "mov ecx,DWORD PTR [ebp+0x8]", "lea eax,[eax+ecx*4]"]
            && callees == [Some("begin")]
            && ret == "ret 0x4"
        {
            kind = Some("index");
        } else if shape
            == [
                "mov eax,DWORD PTR [ebp+0x8]",
                "mov cl,BYTE PTR [eax]",
                "mov edx,DWORD PTR [ebp-0x4]",
                "mov BYTE PTR [edx],cl",
                "mov eax,DWORD PTR [ebp-0x4]",
                "mov DWORD PTR [eax+0x4],0x0",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "mov DWORD PTR [ecx+0x8],0x0",
                "mov edx,DWORD PTR [ebp-0x4]",
                "mov DWORD PTR [edx+0xc],0x0",
                "mov eax,DWORD PTR [ebp-0x4]",
            ]
            && ret == "ret 0x4"
        {
            kind = Some("ctor");
        } else if shape
            == [
                "mov eax,DWORD PTR [ebp+0x8]",
                "push eax",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "call A",
                "push eax",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "call A",
            ]
            && callee0()? == Some("end")
            && ret == "ret 0x4"
        {
            kind = Some("push_back"); // insert(end(), x)
        } else if shape
            == [
                "mov ecx,DWORD PTR [ebp-0x4]",
                "call A",
                "push eax",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "call A",
                "push eax",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "call A",
            ]
            && callees.get(..2) == Some(&[Some("end"), Some("begin")][..])
            && ret == "ret"
        {
            kind = Some("clear"); // erase(begin(), end())
        } else if shape
            == [
                "mov eax,DWORD PTR [ebp+0x8]",
                "push eax",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "call A",
                "push eax",
                "mov ecx,DWORD PTR [ebp+0x8]",
                "add ecx,0x4",
                "push ecx",
                "call A",
                "add esp,0xc",
                "mov edx,DWORD PTR [ebp-0x4]",
                "mov eax,DWORD PTR [edx+0x8]",
                "push eax",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "mov edx,DWORD PTR [ecx+0x8]",
                "sub edx,0x4",
                "push edx",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "call A",
                "mov eax,DWORD PTR [ebp-0x4]",
                "mov ecx,DWORD PTR [eax+0x8]",
                "sub ecx,0x4",
                "mov edx,DWORD PTR [ebp-0x4]",
                "mov DWORD PTR [edx+0x8],ecx",
                "mov eax,DWORD PTR [ebp+0x8]",
            ]
            && callee0()? == Some("end")
            && ret == "ret 0x4"
        {
            kind = Some("erase"); // copy the tail down, destroy the last element, shrink; returns the iterator
        } else if shape
            == [
                this,
                "mov ecx,DWORD PTR [eax+0x8]",
                "push ecx",
                "mov edx,DWORD PTR [ebp-0x4]",
                "mov eax,DWORD PTR [edx+0x4]",
                "push eax",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "call A",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "mov edx,DWORD PTR [ebp-0x4]",
                "mov eax,DWORD PTR [ecx+0xc]",
                "sub eax,DWORD PTR [edx+0x4]",
                "sar eax,0x2",
                "push eax",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "mov edx,DWORD PTR [ecx+0x4]",
                "push edx",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "call A",
                "mov eax,DWORD PTR [ebp-0x4]",
                "mov DWORD PTR [eax+0x4],0x0",
                "mov ecx,DWORD PTR [ebp-0x4]",
                "mov DWORD PTR [ecx+0x8],0x0",
                "mov edx,DWORD PTR [ebp-0x4]",
                "mov DWORD PTR [edx+0xc],0x0",
            ]
            && ret == "ret"
        {
            kind = Some("dtor");
        }
        self.vec.borrow_mut().insert(addr, kind);
        Ok(kind)
    }

    /// `Dsl.ins_text`: the instruction texts of the function at index `i` (up to `n`): to its
    /// `int3` padding.
    pub fn ins_text(&self, i: usize, n: usize) -> Vec<&str> {
        let (mut out, mut seen_ret) = (Vec::new(), false);
        for (_, t) in self.ins.iter().skip(i).take(n) {
            if t == "int3" && seen_ret {
                break;
            }
            seen_ret = seen_ret || t.starts_with("ret");
            out.push(t.as_str());
        }
        out
    }

    /// `Dsl.call_sites`: the indexes of the direct calls of `addr` (thunks followed).
    pub fn call_sites(&self, addr: i64) -> Result<Vec<usize>> {
        if self.sites.borrow().is_none() {
            let mut sites: HashMap<i64, Vec<usize>> = HashMap::new();
            for (j, (_, t)) in self.ins.iter().enumerate() {
                if let Some(h) = pat::call_abs(t) {
                    let tgt = py_int(h, 16)?;
                    sites.entry(self.thunks.get(&tgt).copied().unwrap_or(tgt)).or_default().push(j);
                }
            }
            *self.sites.borrow_mut() = Some(sites);
        }
        Ok(self.sites.borrow().as_ref().and_then(|s| s.get(&addr)).cloned().unwrap_or_default())
    }

    /// `Dsl.is_assert_site`: `push ...; push type; call f; add esp,N`: the C runtime's debug
    /// report (assert, error, warning: report type 0..2, a variable number of arguments).
    pub fn is_assert_site(&self, j: usize) -> bool {
        let prev = if j > 0 { self.ins[j - 1].1.as_str() } else { "" };
        let nxt = self.ins.get(j + 1).map_or("", |x| x.1.as_str());
        matches!(prev, "push   0x0" | "push   0x1" | "push   0x2") && nxt.starts_with("add    esp,")
    }

    /// `Dsl.entry`: the function address behind a thunk. A DecodeError when no instruction is
    /// there.
    pub fn entry(&self, addr: i64) -> Result<i64> {
        let addr = self.thunks.get(&addr).copied().unwrap_or(addr);
        if !self.at.contains_key(&addr) {
            return Err(DecodeError::new(format!("no instruction at {}", py_hex(addr))));
        }
        Ok(addr)
    }

    /// `Dsl.si`: the mangled name of `ScriptInterfaceImp` slot `slot`.
    pub fn si(&self, slot: i64) -> Option<&str> {
        let names = self.engine.get("ScriptInterfaceImp")?;
        usize::try_from(slot).ok().and_then(|s| names.get(s)).map(String::as_str)
    }

    /// `Dsl.var_slot`: the operation and the mangled name of slot `slot` of rule-variable
    /// class `kind`.
    pub fn var_slot(&self, kind: &str, slot: i64) -> (Option<&'static str>, Option<&str>) {
        let names = self.engine.get(kind);
        let name = names.and_then(|n| usize::try_from(slot).ok().and_then(|s| n.get(s))).map(String::as_str);
        (var_op(slot), name)
    }

    /// `Dsl.global_at`: the exported datum holding `addr`: `(name, kind, offset)`. The last
    /// array-like datum that contains the address wins.
    pub fn global_at(&self, addr: i64) -> Option<(String, &'static str, i64)> {
        if let Some(it) = self.data.get(&addr) {
            return Some((it.name.clone(), it.kind, 0));
        }
        let mut best = None;
        for (a, it) in &self.data {
            if *a < addr && addr < a + it.size && matches!(it.kind, "intarray" | "Vector3" | "floatarray") {
                best = Some((it.name.clone(), it.kind, addr - a));
            }
        }
        best
    }

    /// `Dsl.is_chkstk`: the C runtime's stack probe (`_chkstk`), which large frames call
    /// instead of `sub esp,N`.
    pub fn is_chkstk(&self, addr: i64) -> bool {
        let Some(i) = self.index_of(addr) else { return false };
        let text: Vec<&str> = self.ins.iter().skip(i).take(16).map(|x| x.1.as_str()).collect();
        text.iter().any(|t| t.starts_with("cmp    eax,0x1000")) && text.contains(&"mov    esp,ecx")
    }

    /// `Dsl.is_ftol`: the C runtime's float-to-int conversion (`_ftol`): x87 truncation to an
    /// integer through a changed control word.
    pub fn is_ftol(&self, addr: i64) -> Result<bool> {
        if let Some(v) = self.ftol.borrow().get(&addr) {
            return Ok(*v);
        }
        let mut ok = false;
        if let Some(i) = self.index_of(addr) {
            let mut seen: Vec<&str> = Vec::new();
            for (_, t) in self.ins.iter().skip(i).take(24) {
                let w = py_split_ws_once(t);
                seen.push(w.first().copied().ok_or_else(|| DecodeError::raw("list index out of range"))?);
                if t.starts_with("ret") {
                    break;
                }
            }
            ok = (seen.contains(&"fnstcw") || seen.contains(&"fstcw")) && seen.contains(&"fistp") && seen.contains(&"fldcw");
        }
        self.ftol.borrow_mut().insert(addr, ok);
        Ok(ok)
    }

    /// `d.static_refs.setdefault(addr, class)`.
    pub fn note_static(&self, addr: i64, class: &'static str) {
        self.static_refs.borrow_mut().entry(addr).or_insert(class);
    }

    /// `d.static_refs[addr] = class`.
    pub fn set_static(&self, addr: i64, class: &'static str) {
        self.static_refs.borrow_mut().insert(addr, class);
    }
}

/// `chkesp(ins)`: the address of the routine that the debug build's stack checks
/// (`cmp esi|edi|ebp,esp` followed by a call) call most often; the first one in listing order on
/// a tie.
pub fn chkesp(ins: &[Ins]) -> Result<Option<i64>> {
    let mut seen: IndexMap<i64, i64> = IndexMap::new();
    for i in 0..ins.len().saturating_sub(1) {
        if pat::cmp_frame_esp(&ins[i].1)
            && let Some(h) = pat::call_abs(&ins[i + 1].1)
        {
            *seen.entry(py_int(h, 16)?).or_insert(0) += 1;
        }
    }
    Ok(first_max_key(seen.iter()).copied())
}
