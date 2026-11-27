//! MSVC name decoration helpers of the converter tools (LUA-15c).
//!
//! The mission and rule DLLs import engine functions and variables by decorated C++ name; the
//! tools read just enough of the decoration grammar to name a symbol, tell what kind of thing it
//! is, and find its parameter and return types (to tell `float` and `bool` parameters, and to
//! count stack slots). These are line-for-line ports of `plain_name`, `import_kind`,
//! `msvc_params` (`tools/drl_to_lua.py`), `msvc_ret`, `push_count`, `data_kind`
//! (`tools/dsl_to_lua.py`) and `signature`, `parse_types`, `slots_of` (`tools/a1_to_lua.py`).
//!
//! Symbols are latin-1 decoded strings (every char at most U+00FF); the code works on chars, as
//! Python indexes a `str`. Python's `re` quirks are kept where a symbol could trigger them: `$`
//! also matches before one trailing newline, `.` does not match a newline, and `\w` is the
//! Unicode word class ([`char::is_alphanumeric`] or `_`).

use crate::pyval::{py_isdigit, py_slice};
use crate::{DecodeError, Result};

/// `VAR_KINDS` of `drl_to_lua.py`: the rule-variable classes.
pub const VAR_KINDS: [&str; 4] = ["hot_int", "hot_float", "dirty_int", "timer_int"];

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `\w` of Python's `re` on a `str`.
fn is_w(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `re.search(lit + "$", s)` for a literal: `s` ends with `lit`, or with `lit` and one newline.
fn ends_dollar(s: &str, lit: &str) -> bool {
    s.ends_with(lit) || s.strip_suffix('\n').is_some_and(|t| t.ends_with(lit))
}

/// What `(.*)$` captures after a `re.match` prefix: the rest of the text when it has no newline,
/// without a single trailing newline, and `None` when a newline sits anywhere else (the `.`
/// cannot cross it and `$` only accepts the very end or just before the last newline).
fn dot_star_dollar(rest: &[char]) -> Option<&[char]> {
    match rest.iter().position(|c| *c == '\n') {
        None => Some(rest),
        Some(p) if p + 1 == rest.len() => Some(&rest[..p]),
        Some(_) => None,
    }
}

/// End of `[A-Za-z_][A-Za-z0-9_]*` starting at `i` (ASCII classes), or `None`.
fn ascii_ident(s: &[char], i: usize) -> Option<usize> {
    if i < s.len() && is_ident_start(s[i]) {
        let mut j = i + 1;
        while j < s.len() && is_ident_char(s[j]) {
            j += 1;
        }
        Some(j)
    } else {
        None
    }
}

/// End of `[A-Za-z_]\w*` (Unicode `\w`) starting at `i`, or `None`.
fn w_ident(s: &[char], i: usize) -> Option<usize> {
    if i < s.len() && is_ident_start(s[i]) {
        let mut j = i + 1;
        while j < s.len() && is_w(s[j]) {
            j += 1;
        }
        Some(j)
    } else {
        None
    }
}

fn starts_with_chars(s: &[char], at: usize, lit: &str) -> bool {
    for (i, c) in (at..).zip(lit.chars()) {
        if i >= s.len() || s[i] != c {
            return false;
        }
    }
    true
}

/// `plain_name`: `?Name@Class@@...` or `?Name@@...` gives `Name`, anything else is unchanged.
/// (`re.match(r"\?([A-Za-z_][A-Za-z0-9_]*)@", symbol)`.)
pub fn plain_name(symbol: &str) -> String {
    let s: Vec<char> = symbol.chars().collect();
    if s.first() == Some(&'?')
        && let Some(end) = ascii_ident(&s, 1)
        && end < s.len()
        && s[end] == '@'
    {
        return s[1..end].iter().collect();
    }
    symbol.to_owned()
}

/// `import_kind`: what a `.dsl`/engine import is, from its MSVC decoration. One of the
/// [`VAR_KINDS`], `character`, `ScriptInterface`, `int`, `intarray`, `float`, `action`,
/// `function`, `other`.
pub fn import_kind(symbol: &str) -> &'static str {
    for k in VAR_KINDS.iter().chain(["character", "ScriptInterface"].iter()) {
        if ends_dollar(symbol, &format!("@@3PAV{k}@@A")) || ends_dollar(symbol, &format!("@@3V{k}@@A")) {
            return k;
        }
    }
    if symbol.ends_with("@@3HA") {
        return "int";
    }
    if symbol.ends_with("@@3PAHA") {
        return "intarray";
    }
    if symbol.ends_with("@@3MA") {
        return "float";
    }
    if ends_dollar(symbol, "@@YAXXZ") {
        return "action";
    }
    if symbol.contains("@@YA") || symbol.contains("@@YG") || symbol.contains("@@YI") {
        return "function";
    }
    "other"
}

/// `data_kind` of `dsl_to_lua.py`: [`import_kind`], and for `other` also `Vector3`,
/// `floatarray` and `int` (a `bool` variable, `_N`).
pub fn data_kind(symbol: &str) -> &'static str {
    let k = import_kind(symbol);
    if k != "other" {
        return k;
    }
    if symbol.ends_with("@@3VVector3@@A") {
        return "Vector3";
    }
    if ends_dollar(symbol, "@@3PAMA") {
        return "floatarray";
    }
    if ends_dollar(symbol, "@@3_NA") {
        return "int";
    }
    "other"
}

/// `push_count`: stack slots one parameter of MSVC type code `t` takes.
pub fn push_count(t: &str) -> i64 {
    if t == "N" {
        return 2;
    }
    if t.starts_with("VVector3") {
        return 3;
    }
    if t == "_J" || t == "_K" {
        return 2;
    }
    1
}

/// `slots_of` of `a1_to_lua.py`: stack slots a call's arguments take (a struct returned by value,
/// a return code starting `?A`, adds the hidden result pointer).
pub fn slots_of(params: &[String], ret: &str) -> i64 {
    params.iter().map(|p| push_count(p)).sum::<i64>() + i64::from(ret.starts_with("?A"))
}

/// The tail after `\?name@Class@@[QU]A[EG]` of a member-function name (`re.match(...(.*)$)`).
fn member_tail(s: &[char]) -> Option<&[char]> {
    if s.first() != Some(&'?') {
        return None;
    }
    let a = ascii_ident(s, 1)?;
    if a >= s.len() || s[a] != '@' {
        return None;
    }
    let b = ascii_ident(s, a + 1)?;
    if !starts_with_chars(s, b, "@@") {
        return None;
    }
    let i = b + 2;
    if i + 3 <= s.len() && matches!(s[i], 'Q' | 'U') && s[i + 1] == 'A' && matches!(s[i + 2], 'E' | 'G') {
        dot_star_dollar(&s[i + 3..])
    } else {
        None
    }
}

/// The tail after `\?name@@Y[AGI]` of a free-function name.
fn free_tail(s: &[char]) -> Option<&[char]> {
    if s.first() != Some(&'?') {
        return None;
    }
    let a = ascii_ident(s, 1)?;
    if !starts_with_chars(s, a, "@@Y") {
        return None;
    }
    let i = a + 3;
    if i < s.len() && matches!(s[i], 'A' | 'G' | 'I') {
        dot_star_dollar(&s[i + 1..])
    } else {
        None
    }
}

/// A fault of the type-code readers: Python's `ValueError` or `IndexError`, which the callers
/// catch together (`except (ValueError, IndexError)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseFault {
    Value,
    Index,
}

struct Reader<'a> {
    s: &'a [char],
    pos: usize,
    seen: Vec<String>,
}

impl Reader<'_> {
    fn find_at_at(&self, from: usize) -> std::result::Result<usize, ParseFault> {
        (from..self.s.len().saturating_sub(1))
            .find(|&j| self.s[j] == '@' && self.s[j + 1] == '@')
            .ok_or(ParseFault::Value)
    }

    /// `one()` of `msvc_params`.
    fn one_params(&mut self) -> std::result::Result<String, ParseFault> {
        let s = self.s;
        let i = self.pos;
        if i >= s.len() {
            return Err(ParseFault::Value);
        }
        let c = s[i];
        if c == '_' {
            self.pos += 2;
            return Ok(py_slice(s, i, i + 2));
        }
        if matches!(c, 'P' | 'Q' | 'A' | 'B') && i + 1 < s.len() && matches!(s[i + 1], 'A' | 'B' | 'C' | 'D') {
            self.pos += 2;
            return Ok(format!("{c}{}", self.one_params()?));
        }
        if matches!(c, 'V' | 'U' | 'T' | 'W') {
            let j = self.find_at_at(i)?;
            self.pos = j + 2;
            return Ok(py_slice(s, i, j + 2));
        }
        if c == '?' {
            self.pos += 2; // ?A / ?B storage class of a return value
            return self.one_params();
        }
        if py_isdigit(c) {
            self.pos += 1;
            let n = c.to_digit(10).ok_or(ParseFault::Value)? as usize; // int('²') raises
            return Ok(self.seen.get(n).cloned().unwrap_or_else(|| "?".to_owned()));
        }
        self.pos += 1;
        Ok(c.to_string())
    }

    /// `one()` of `parse_types`.
    fn one_types(&mut self) -> std::result::Result<String, ParseFault> {
        let s = self.s;
        let i = self.pos;
        let c = *s.get(i).ok_or(ParseFault::Index)?;
        if c == '_' {
            self.pos += 2;
            return Ok(py_slice(s, i, i + 2));
        }
        if c == '?' {
            self.pos += 2; // ?A / ?B: storage class of a returned value
            return Ok(format!("?A{}", self.one_types()?));
        }
        let next = if matches!(c, 'P' | 'Q' | 'A' | 'B') {
            Some(*s.get(i + 1).ok_or(ParseFault::Index)?)
        } else {
            None
        };
        if next == Some('6') {
            self.pos += 3; // function pointer: P6 <convention> <return> <params> Z
            self.one_types()?;
            while !matches!(*s.get(self.pos).ok_or(ParseFault::Index)?, '@' | 'Z') {
                self.one_types()?;
            }
            self.pos += 1;
            return Ok(py_slice(s, i, self.pos));
        }
        if let Some(n) = next
            && matches!(n, 'A' | 'B' | 'C' | 'D')
        {
            self.pos += 2;
            return Ok(format!("{c}{n}{}", self.one_types()?));
        }
        if matches!(c, 'V' | 'U' | 'T' | 'W') {
            let j = self.find_at_at(i)?;
            self.pos = j + 2;
            return Ok(py_slice(s, i, j + 2));
        }
        if py_isdigit(c) {
            self.pos += 1;
            let n = c.to_digit(10).ok_or(ParseFault::Value)? as usize;
            return self.seen.get(n).cloned().ok_or(ParseFault::Index);
        }
        self.pos += 1;
        Ok(c.to_string())
    }
}

/// `msvc_params`: the parameter type codes of an MSVC member-function (or free-function) name,
/// or `None`. Enough of the grammar to tell which arguments are `float` (`M`): primitives,
/// pointers and references, named types and back-references.
pub fn msvc_params(symbol: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = symbol.chars().collect();
    let tail = member_tail(&chars).or_else(|| free_tail(&chars))?;
    let mut r = Reader { s: tail, pos: 0, seen: Vec::new() };
    let result = (|| -> std::result::Result<Vec<String>, ParseFault> {
        r.one_params()?; // return type
        let mut params: Vec<String> = Vec::new();
        while r.pos < tail.len() && !matches!(tail[r.pos], '@' | 'Z') {
            let t = r.one_params()?;
            if t == "X" && params.is_empty() {
                return Ok(Vec::new());
            }
            if t.chars().count() > 1 {
                r.seen.push(t.clone());
            }
            params.push(t);
        }
        Ok(params)
    })();
    result.ok()
}

/// `msvc_ret`: the return-type code of an MSVC member-function name (`X` void, `H` int, `M`
/// float, `_N` bool, `P...` pointer, `?AV...` struct by value), or `None` when the name is not a
/// member function. Python's `IndexError` / `ValueError` on a truncated name are raw errors.
pub fn msvc_ret(symbol: &str) -> Result<Option<String>> {
    let chars: Vec<char> = symbol.chars().collect();
    let Some(s) = member_tail(&chars) else {
        return Ok(None);
    };
    let head2: String = s.iter().take(2).collect();
    if head2 == "_N" {
        return Ok(Some("_N".to_owned()));
    }
    if head2 == "?A" {
        let j = (0..s.len().saturating_sub(1)).find(|&j| s[j] == '@' && s[j + 1] == '@');
        return Ok(Some(match j {
            Some(j) if j > 0 => py_slice(s, 0, j + 2),
            _ => py_slice(s, 0, 3),
        }));
    }
    if matches!(head2.as_str(), "PA" | "PB" | "AB" | "AA") {
        let third = *s.get(2).ok_or_else(|| DecodeError::python("IndexError", "string index out of range"))?;
        if matches!(third, 'V' | 'U') {
            let j = (0..s.len().saturating_sub(1))
                .find(|&j| s[j] == '@' && s[j + 1] == '@')
                .ok_or_else(|| DecodeError::python("ValueError", "substring not found"))?;
            return Ok(Some(py_slice(s, 0, j + 2)));
        }
        return Ok(Some(py_slice(s, 0, 3)));
    }
    Ok(Some(py_slice(s, 0, 1)))
}

/// The result of [`signature`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// `thiscall`, `cdecl` or `stdcall` (`fastcall` is never returned).
    pub convention: &'static str,
    /// The class, `None` for a free function.
    pub cls: Option<String>,
    /// The method name; an operator is `op` followed by its code character (`??4` is `op4`).
    pub method: String,
    /// The return type code.
    pub ret: String,
    /// The parameter type codes (`[]` for a lone `X`).
    pub params: Vec<String>,
}

/// `signature` of `a1_to_lua.py`: (convention, class, method, return code, parameter codes) of a
/// decorated function name, or `None` for data and names it does not read.
pub fn signature(sym: &str) -> Option<Signature> {
    let s: Vec<char> = sym.chars().collect();
    // `fullmatch` with a trailing `(.*)`: no newline anywhere in the rest.
    let rest_ok = |from: usize| !s[from..].contains(&'\n');
    let conv_of = |c: char| match c {
        'E' => "thiscall",
        'A' => "cdecl",
        'G' => "stdcall",
        _ => "fastcall", // 'I'
    };
    let class_a = |c: char| c.is_ascii_uppercase();

    // \?\?([0-9A-Z_])([A-Za-z_]\w*)@@([A-Z])([AB])([AEG])(.*)
    let try_a = || -> Option<(String, Option<String>, &'static str, usize)> {
        if !starts_with_chars(&s, 0, "??") || s.len() < 3 {
            return None;
        }
        let c1 = s[2];
        if !(c1.is_ascii_digit() || c1.is_ascii_uppercase() || c1 == '_') {
            return None;
        }
        let e = w_ident(&s, 3)?;
        if !starts_with_chars(&s, e, "@@") {
            return None;
        }
        let i = e + 2;
        if i + 3 <= s.len()
            && class_a(s[i])
            && matches!(s[i + 1], 'A' | 'B')
            && matches!(s[i + 2], 'A' | 'E' | 'G')
            && rest_ok(i + 3)
        {
            let cls: String = s[3..e].iter().collect();
            return Some((format!("op{c1}"), Some(cls), conv_of(s[i + 2]), i + 3));
        }
        None
    };
    // \?([A-Za-z_]\w*)@([A-Za-z_]\w*)@@([A-Z])([AB])([AEG])(.*)
    let try_b = || -> Option<(String, Option<String>, &'static str, usize)> {
        if s.first() != Some(&'?') {
            return None;
        }
        let e1 = w_ident(&s, 1)?;
        if e1 >= s.len() || s[e1] != '@' {
            return None;
        }
        let e2 = w_ident(&s, e1 + 1)?;
        if !starts_with_chars(&s, e2, "@@") {
            return None;
        }
        let i = e2 + 2;
        if i + 3 <= s.len() && class_a(s[i]) && matches!(s[i + 1], 'A' | 'B') && matches!(s[i + 2], 'A' | 'E' | 'G') && rest_ok(i + 3)
        {
            let name: String = s[1..e1].iter().collect();
            let cls: String = s[e1 + 1..e2].iter().collect();
            return Some((name, Some(cls), conv_of(s[i + 2]), i + 3));
        }
        None
    };
    // \?([A-Za-z_]\w*)@@Y([AGI])(.*)
    let try_c = || -> Option<(String, Option<String>, &'static str, usize)> {
        if s.first() != Some(&'?') {
            return None;
        }
        let e1 = w_ident(&s, 1)?;
        if !starts_with_chars(&s, e1, "@@Y") {
            return None;
        }
        let i = e1 + 3;
        if i < s.len() && matches!(s[i], 'A' | 'G' | 'I') && rest_ok(i + 1) {
            let name: String = s[1..e1].iter().collect();
            return Some((name, None, conv_of(s[i]), i + 1));
        }
        None
    };
    let (name, cls, convention, from) = try_a().or_else(try_b).or_else(try_c)?;
    let rest: String = s[from..].iter().collect();

    if (cls.is_none() && convention == "thiscall") || convention == "fastcall" {
        return None; // fastcall passes arguments in registers: not modelled
    }
    let types = parse_types(&rest).ok()?;
    if types.is_empty() {
        return None;
    }
    let ret = types[0].clone();
    let mut params: Vec<String> = types[1..].to_vec();
    if params == ["X"] {
        params = Vec::new();
    }
    Some(Signature { convention, cls, method: name, ret, params })
}

/// `parse_types` of `a1_to_lua.py`: the MSVC type codes of a decorated signature's return type
/// and parameters (up to the closing `@` or `Z`): primitives, pointers and references, named
/// classes, function pointers (`P6...`) and back-references. A fault is Python's `ValueError` or
/// `IndexError` ([`signature`] catches both).
pub fn parse_types(text: &str) -> std::result::Result<Vec<String>, ParseFault> {
    let s: Vec<char> = text.chars().collect();
    let mut r = Reader { s: &s, pos: 0, seen: Vec::new() };
    let mut out: Vec<String> = Vec::new();
    while r.pos < s.len() && !matches!(s[r.pos], '@' | 'Z') {
        let t = r.one_types()?;
        if !out.is_empty() && t.chars().count() > 1 && !t.starts_with("?A") {
            r.seen.push(t.clone()); // only parameter types are remembered for back-references
        }
        let is_x = t == "X";
        out.push(t);
        if out.len() == 2 && is_x {
            break;
        }
    }
    Ok(out)
}
