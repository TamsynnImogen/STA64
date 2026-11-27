//! The Python value tree and the small pieces of Python semantics the converter tools lean on
//! (LUA-15c).
//!
//! The Python lifters keep expressions and statements as plain tuples (`("cmp", "<", a, b)`,
//! `("and", [x, y])`, `("const", 1)`) and walk them with generic code whose quirks the output
//! depends on (`contains` looks at a non-string head, `inline_temps` skips element 0 of every
//! tuple, `simplify` tests `y not in items` with Python equality, `repr` keys the boolean atoms).
//! The port therefore keeps the same dynamic shape: [`Py`] is a tuple/list/scalar tree with
//! Python's equality (`1 == 1.0 == True`, `0.0 == -0.0`), hash, `repr`, `str`, truthiness and
//! ordering. Typed state structs are fine around it; the expression trees stay [`Py`].
//!
//! Also here: integer floor division and modulo, `int(s, base)`, the string methods whose Python
//! behaviour differs from Rust's (`strip`, `split(None, 1)`, `isdigit`), sorting with Python's
//! comparison rules (mixed numbers, tuples, `TypeError` for the rest), `first-maximum` and the
//! dict helpers (`IndexMap` keeps insertion order; `del` is `shift_remove`, never `swap_remove`).
//!
//! Assumption shared by the string helpers: the strings come from `latin1` decoding, so every
//! char is at most U+00FF. Behaviour beyond that is documented per function and not relied on.
//!
//! NaN: Python tells two NaN objects apart (a NaN equals itself only as the same object), so a
//! tree holding two NaN constants from separate reads keeps both where this port, which cannot
//! see object identity, merges a NaN with any NaN of the same bits (`Eq` is what `simplify`'s
//! `y not in items` and the state merge need). The gated parity test of the converters is the
//! guard for NaN float constants. Python 3.14 also keeps a signalling NaN's payload through
//! `struct.unpack('<f')` where Rust's `f64::from(f32)` quiets it; only equality sees the bits,
//! the text of a NaN is `nan` / `(0/0)` either way.
//!
//! Errors: where Python raises a raw exception (`TypeError`, `IndexError`, `KeyError`,
//! `ValueError`, `ZeroDivisionError`) these functions return a [`DecodeError::python`] with the
//! exception's class name and the text CPython 3.14 gives (`ZeroDivisionError: division by zero`;
//! older Pythons say `integer division or modulo by zero`). The Armada I converter writes
//! `{class}: {text}` into the Lua of a function whose rendering raised, so those texts are
//! compared with the real Python in `tests/core.rs` (table `RAW_TEXT`). Left approximate, because
//! no path of the tools can put them into output: the `i64` overflow cases (Python has big ints,
//! here STA64 errors with class `None`), indexing a `str`, and the unpacking of a string. A
//! caller that ports "this aborts the run" propagates the error with `?`.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use indexmap::IndexMap;

use crate::format::{py_float_repr, py_str_repr};
use crate::{DecodeError, Result};

/// A Python value: what the tools' tuples, lists and scalars hold.
///
/// `Eq`/`Hash` follow Python: `Int(1) == Float(1.0) == Bool(true)` and they hash alike,
/// `Float(0.0) == Float(-0.0)`, a NaN equals a NaN with the same bits (Python's identity
/// shortcut; the tools only ever copy a NaN around), a tuple never equals a list. Integers are
/// `i64` (every value the tools handle is a 32-bit quantity or a small count).
#[derive(Debug, Clone)]
pub enum Py {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Tuple(Vec<Py>),
    List(Vec<Py>),
}

/// A tuple: `tup!["const", 1]` is `("const", 1)`. Each element goes through [`Py::from`].
#[macro_export]
macro_rules! tup {
    ($($x:expr),* $(,)?) => { $crate::pyval::Py::Tuple(vec![$($crate::pyval::Py::from($x)),*]) };
}

/// A list: `list![a, b]` is `[a, b]`. Each element goes through [`Py::from`].
#[macro_export]
macro_rules! list {
    ($($x:expr),* $(,)?) => { $crate::pyval::Py::List(vec![$($crate::pyval::Py::from($x)),*]) };
}

impl From<&str> for Py {
    fn from(s: &str) -> Self {
        Py::Str(s.to_owned())
    }
}
impl From<String> for Py {
    fn from(s: String) -> Self {
        Py::Str(s)
    }
}
impl From<&String> for Py {
    fn from(s: &String) -> Self {
        Py::Str(s.clone())
    }
}
impl From<i64> for Py {
    fn from(n: i64) -> Self {
        Py::Int(n)
    }
}
impl From<i32> for Py {
    fn from(n: i32) -> Self {
        Py::Int(n.into())
    }
}
impl From<u32> for Py {
    fn from(n: u32) -> Self {
        Py::Int(n.into())
    }
}
impl From<usize> for Py {
    fn from(n: usize) -> Self {
        Py::Int(n as i64)
    }
}
impl From<f64> for Py {
    fn from(x: f64) -> Self {
        Py::Float(x)
    }
}
impl From<bool> for Py {
    fn from(b: bool) -> Self {
        Py::Bool(b)
    }
}
impl From<&Py> for Py {
    fn from(p: &Py) -> Self {
        p.clone()
    }
}

/// A numeric view of `None`-free scalars for comparisons.
#[derive(Clone, Copy)]
enum Num {
    I(i64),
    F(f64),
}

/// A fault of STA64's own with no Python counterpart text (see the module docs).
fn raw(msg: impl Into<String>) -> DecodeError {
    DecodeError::raw(msg)
}

/// A Python exception: class name and the text `str(e)` gives (CPython 3.14).
fn py(class: &'static str, msg: impl Into<String>) -> DecodeError {
    DecodeError::python(class, msg)
}

/// `TypeError: 'X' object is not subscriptable`.
fn not_subscriptable(v: &Py) -> DecodeError {
    py("TypeError", format!("'{}' object is not subscriptable", v.type_name()))
}

impl Py {
    /// Python's `type(x).__name__`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Py::None => "NoneType",
            Py::Bool(_) => "bool",
            Py::Int(_) => "int",
            Py::Float(_) => "float",
            Py::Str(_) => "str",
            Py::Tuple(_) => "tuple",
            Py::List(_) => "list",
        }
    }

    fn num(&self) -> Option<Num> {
        match self {
            Py::Bool(b) => Some(Num::I(*b as i64)),
            Py::Int(n) => Some(Num::I(*n)),
            Py::Float(x) => Some(Num::F(*x)),
            _ => None,
        }
    }

    /// `x is None`.
    pub fn is_none(&self) -> bool {
        matches!(self, Py::None)
    }

    /// The string of a `Str`, else `None`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Py::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The integer of an `Int` (or a `Bool`, which is an int in Python), else `None`.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Py::Int(n) => Some(*n),
            Py::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    /// The elements of a tuple or list, else `None`.
    pub fn as_seq(&self) -> Option<&[Py]> {
        match self {
            Py::Tuple(v) | Py::List(v) => Some(v),
            _ => None,
        }
    }

    /// Python truthiness (`if x:`).
    pub fn truthy(&self) -> bool {
        match self {
            Py::None => false,
            Py::Bool(b) => *b,
            Py::Int(n) => *n != 0,
            Py::Float(x) => *x != 0.0,
            Py::Str(s) => !s.is_empty(),
            Py::Tuple(v) | Py::List(v) => !v.is_empty(),
        }
    }

    /// `x[i]` for a tuple or list (`i >= 0`): `IndexError` / `TypeError` as raw errors.
    /// Indexing a string is not supported (the tools never do it on a tree).
    pub fn item(&self, i: usize) -> Result<&Py> {
        match self {
            Py::Tuple(v) | Py::List(v) => v
                .get(i)
                .ok_or_else(|| py("IndexError", format!("{} index out of range", self.type_name()))),
            other => Err(not_subscriptable(other)),
        }
    }

    /// `x[i]` with a negative index counted from the end (`x[-1]` is `item_back(1)`).
    pub fn item_back(&self, i: usize) -> Result<&Py> {
        match self {
            Py::Tuple(v) | Py::List(v) => {
                if i >= 1 && i <= v.len() {
                    Ok(&v[v.len() - i])
                } else {
                    Err(py("IndexError", format!("{} index out of range", self.type_name())))
                }
            }
            other => Err(not_subscriptable(other)),
        }
    }

    /// The elements for iteration (`for y in x`): a tuple or list, else a `TypeError`.
    pub fn items(&self) -> Result<&[Py]> {
        self.as_seq()
            .ok_or_else(|| py("TypeError", format!("'{}' object is not iterable", self.type_name())))
    }

    /// `x[1:]` of a tuple (as a list of references): the generic walkers' "every element but the
    /// head". A `TypeError` for a non-sequence.
    pub fn tail(&self) -> Result<&[Py]> {
        Ok(self.items()?.get(1..).unwrap_or(&[]))
    }

    /// `len(x)` of a tuple or list; a `TypeError` otherwise.
    pub fn py_len(&self) -> Result<usize> {
        Ok(self.items().map_err(|_| py("TypeError", format!("object of type '{}' has no len()", self.type_name())))?.len())
    }

    /// `x[0]` as the tag string: `Ok(Some(tag))` when it is a `str`, `Ok(None)` when it is
    /// something else (every `== "kind"` test in Python is then false), an error when `x` has no
    /// element 0 (`IndexError`, `TypeError`).
    pub fn tag(&self) -> Result<Option<&str>> {
        Ok(self.item(0)?.as_str())
    }

    /// `x[0] == name`.
    pub fn tag_is(&self, name: &str) -> Result<bool> {
        Ok(self.tag()? == Some(name))
    }

    /// `x[0] in names`.
    pub fn tag_in(&self, names: &[&str]) -> Result<bool> {
        Ok(match self.tag()? {
            Some(t) => names.contains(&t),
            None => false,
        })
    }

    /// Python's `repr(x)`.
    pub fn repr(&self) -> String {
        match self {
            Py::None => "None".to_owned(),
            Py::Bool(true) => "True".to_owned(),
            Py::Bool(false) => "False".to_owned(),
            Py::Int(n) => n.to_string(),
            Py::Float(x) => py_float_repr(*x),
            Py::Str(s) => py_str_repr(s),
            Py::Tuple(v) => match v.len() {
                0 => "()".to_owned(),
                1 => format!("({},)", v[0].repr()),
                _ => format!("({})", join_repr(v)),
            },
            Py::List(v) => format!("[{}]", join_repr(v)),
        }
    }

    /// Python's `str(x)` (what an f-string `{x}` prints): the text of a `str`, `repr` otherwise.
    pub fn to_str(&self) -> String {
        match self {
            Py::Str(s) => s.clone(),
            other => other.repr(),
        }
    }

    /// `x == y` as Python computes it (same as `==`).
    pub fn py_eq(&self, other: &Py) -> bool {
        self == other
    }

    /// `x < y` as Python computes it: numbers compare exactly (int against float without
    /// rounding), strings by code point, tuples and lists element by element (the first pair that
    /// is not `==` decides, then the lengths). Anything else is a `TypeError` as a raw error.
    pub fn py_lt(&self, other: &Py) -> Result<bool> {
        if let (Some(a), Some(b)) = (self.num(), other.num()) {
            return Ok(num_cmp(a, b) == Some(Ordering::Less));
        }
        match (self, other) {
            (Py::Str(a), Py::Str(b)) => Ok(a < b),
            (Py::Tuple(a), Py::Tuple(b)) | (Py::List(a), Py::List(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if x != y {
                        return x.py_lt(y);
                    }
                }
                Ok(a.len() < b.len())
            }
            _ => Err(py("TypeError", format!(
                "'<' not supported between instances of '{}' and '{}'",
                self.type_name(),
                other.type_name()
            ))),
        }
    }

    /// The ordering Python's `<` gives (`Less` when `x < y`, `Greater` when `y < x`, else
    /// `Equal`, which includes NaN comparisons).
    pub fn py_cmp(&self, other: &Py) -> Result<Ordering> {
        if self.py_lt(other)? {
            Ok(Ordering::Less)
        } else if other.py_lt(self)? {
            Ok(Ordering::Greater)
        } else {
            Ok(Ordering::Equal)
        }
    }
}

fn join_repr(v: &[Py]) -> String {
    let mut out = String::new();
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&x.repr());
    }
    out
}

/// Exact comparison of two numbers (an int against a float is not rounded); `None` for NaN.
fn num_cmp(a: Num, b: Num) -> Option<Ordering> {
    match (a, b) {
        (Num::I(x), Num::I(y)) => Some(x.cmp(&y)),
        (Num::F(x), Num::F(y)) => x.partial_cmp(&y),
        (Num::I(x), Num::F(y)) => int_float_cmp(x, y),
        (Num::F(x), Num::I(y)) => int_float_cmp(y, x).map(Ordering::reverse),
    }
}

/// 2^63 as an f64 (exact): the floats in `[-TWO63, TWO63)` that are whole numbers are the ones
/// equal to some `i64`; equality, ordering and hashing all use this bound.
const TWO63: f64 = 9_223_372_036_854_775_808.0;

fn int_float_cmp(i: i64, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    if f >= TWO63 {
        return Some(Ordering::Less);
    }
    if f < -TWO63 {
        return Some(Ordering::Greater);
    }
    let t = f.trunc();
    let ti = t as i64; // exact: t is integral and in range
    match i.cmp(&ti) {
        Ordering::Equal => {
            let frac = f - t;
            if frac > 0.0 {
                Some(Ordering::Less)
            } else if frac < 0.0 {
                Some(Ordering::Greater)
            } else {
                Some(Ordering::Equal)
            }
        }
        o => Some(o),
    }
}

impl PartialEq for Py {
    fn eq(&self, other: &Py) -> bool {
        if let (Some(a), Some(b)) = (self.num(), other.num()) {
            return match (a, b) {
                (Num::F(x), Num::F(y)) => x == y || (x.is_nan() && y.is_nan() && x.to_bits() == y.to_bits()),
                _ => num_cmp(a, b) == Some(Ordering::Equal),
            };
        }
        match (self, other) {
            (Py::None, Py::None) => true,
            (Py::Str(a), Py::Str(b)) => a == b,
            (Py::Tuple(a), Py::Tuple(b)) | (Py::List(a), Py::List(b)) => a == b,
            _ => false,
        }
    }
}
impl Eq for Py {}

impl Hash for Py {
    fn hash<H: Hasher>(&self, h: &mut H) {
        match self {
            Py::None => 0u8.hash(h),
            Py::Bool(_) | Py::Int(_) | Py::Float(_) => {
                1u8.hash(h);
                match self.num() {
                    Some(Num::I(n)) => n.hash(h),
                    Some(Num::F(x)) => {
                        if x == x.trunc() && (-TWO63..TWO63).contains(&x) {
                            (x as i64).hash(h); // 1.0 hashes like 1, -0.0 like 0
                        } else if x.is_nan() {
                            f64::NAN.to_bits().hash(h);
                        } else {
                            x.to_bits().hash(h);
                        }
                    }
                    None => {}
                }
            }
            Py::Str(s) => {
                2u8.hash(h);
                s.hash(h);
            }
            Py::Tuple(v) => {
                3u8.hash(h);
                v.hash(h);
            }
            Py::List(v) => {
                4u8.hash(h);
                v.hash(h);
            }
        }
    }
}

// ------------------------------------------------------------------ sorting

/// `sorted(items, key=key)`: stable, the keys are computed once, compared with Python's `<`
/// ([`Py::py_lt`]); the first `TypeError` is returned as a raw error. Mixed incomparable keys
/// raise here when two of them meet in a comparison; which pairs meet depends on the sort
/// algorithm (Python's is timsort, this is a merge sort), so a list with incomparable keys that
/// Python happens to sort without comparing them may fail here: the tools do not have such lists.
/// A NaN among the keys gives the same unspecified order Python gives.
pub fn py_sort_by_key<T>(items: Vec<T>, key: impl Fn(&T) -> Py) -> Result<Vec<T>> {
    let keys: Vec<Py> = items.iter().map(&key).collect();
    let mut idx: Vec<usize> = (0..items.len()).collect();
    merge_sort(&mut idx, &keys)?;
    let mut slots: Vec<Option<T>> = items.into_iter().map(Some).collect();
    Ok(idx.into_iter().map(|i| slots[i].take().expect("each index once")).collect())
}

/// `sorted(values)` for a list of [`Py`] values.
pub fn py_sorted(values: Vec<Py>) -> Result<Vec<Py>> {
    py_sort_by_key(values, |v| v.clone())
}

fn merge_sort(idx: &mut [usize], keys: &[Py]) -> Result<()> {
    let n = idx.len();
    if n < 2 {
        return Ok(());
    }
    let mid = n / 2;
    merge_sort(&mut idx[..mid], keys)?;
    merge_sort(&mut idx[mid..], keys)?;
    let left = idx[..mid].to_vec();
    let right = idx[mid..].to_vec();
    let (mut i, mut j, mut k) = (0, 0, 0);
    while i < left.len() && j < right.len() {
        // stable: the right element goes first only when it is strictly smaller
        if keys[right[j]].py_lt(&keys[left[i]])? {
            idx[k] = right[j];
            j += 1;
        } else {
            idx[k] = left[i];
            i += 1;
        }
        k += 1;
    }
    while i < left.len() {
        idx[k] = left[i];
        i += 1;
        k += 1;
    }
    while j < right.len() {
        idx[k] = right[j];
        j += 1;
        k += 1;
    }
    Ok(())
}

/// `max(d, key=d.get)`: the first key, in iteration order, whose value is the greatest (a later
/// equal value does not replace it). `None` for an empty map (Python raises `ValueError`; the
/// tools test for emptiness first).
pub fn first_max_key<'a, K: 'a, V: PartialOrd + 'a>(items: impl IntoIterator<Item = (&'a K, &'a V)>) -> Option<&'a K> {
    let mut best: Option<(&K, &V)> = None;
    for (k, v) in items {
        if best.is_none_or(|(_, bv)| v > bv) {
            best = Some((k, v));
        }
    }
    best.map(|(k, _)| k)
}

// ------------------------------------------------------------------ integers

/// Python's `a // b` for ints (floor division). `ZeroDivisionError` and the one overflow case
/// (`i64::MIN // -1`, which Python would answer with a bigger int) are raw errors.
pub fn py_floordiv(a: i64, b: i64) -> Result<i64> {
    if b == 0 {
        return Err(py("ZeroDivisionError", "division by zero"));
    }
    let q = a.checked_div(b).ok_or_else(|| raw("integer overflow in floor division"))?;
    let r = a % b; // cannot overflow: b != -1 here or a != MIN
    Ok(if r != 0 && ((r < 0) != (b < 0)) { q - 1 } else { q })
}

/// Python's `a % b` for ints: the result has the sign of `b` (`-5 % 4 == 3`).
pub fn py_mod(a: i64, b: i64) -> Result<i64> {
    if b == 0 {
        return Err(py("ZeroDivisionError", "division by zero"));
    }
    let r = a.checked_rem(b).unwrap_or(0); // MIN % -1 is 0
    Ok(if r != 0 && ((r < 0) != (b < 0)) { r + b } else { r })
}

/// Python's `int(s, base)` for a `str` (base 10 or 16; base 16 takes an optional `0x`).
/// Leading and trailing whitespace, a sign and single underscores between digits are accepted as
/// Python does; only ASCII digits are (Python also takes other scripts' digits, which the tools'
/// inputs, matched by `[0-9a-f]` regexes, never contain). Failure is a `ValueError` as a raw
/// error with Python's text; a value beyond `i64` is a raw error too.
pub fn py_int(s: &str, base: u32) -> Result<i64> {
    // Python's message cuts the `repr` of the text to 200 chars (`%.200R`)
    let bad = || {
        let r: String = py_str_repr(s).chars().take(200).collect();
        py("ValueError", format!("invalid literal for int() with base {base}: {r}"))
    };
    // int() strips whitespace, but not U+001C to U+001F (`str.strip()` does)
    let t = s.trim_matches(|c| py_isspace(c) && !(28..=31).contains(&(c as u32)));
    let (neg, rest) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (rest, prefixed) = if base == 16 {
        match rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
            Some(r) => (r, true),
            None => (rest, false),
        }
    } else {
        (rest, false)
    };
    let too_big = || raw(format!("integer too large for i64: {}", py_str_repr(s)));
    let mut value: i128 = 0;
    let mut underscore_ok = prefixed; // an underscore may follow the base prefix
    let mut last_underscore = false;
    let mut any = false;
    for c in rest.chars() {
        if c == '_' {
            if !underscore_ok {
                return Err(bad());
            }
            underscore_ok = false;
            last_underscore = true;
            continue;
        }
        let d = c.to_digit(base).ok_or_else(bad)?;
        value = value * i128::from(base) + i128::from(d);
        if value > i128::from(i64::MAX) + 1 {
            return Err(too_big());
        }
        underscore_ok = true;
        last_underscore = false;
        any = true;
    }
    if !any || last_underscore {
        return Err(bad());
    }
    let v = if neg { -value } else { value };
    i64::try_from(v).map_err(|_| too_big())
}

// ------------------------------------------------------------------ strings

/// `str.isspace()` for one char (Python's whitespace set, which is wider than Rust's for
/// U+001C to U+001F).
pub fn py_isspace(c: char) -> bool {
    matches!(c as u32, 9..=13 | 28..=32 | 0x85 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000)
}

/// `str.strip()` with no argument.
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(py_isspace)
}

/// `s.split(None, 1)`: `[]` for blank input, else the first word and the rest with its leading
/// whitespace dropped (trailing whitespace of the rest is kept; a rest that is only whitespace is
/// dropped).
pub fn py_split_ws_once(s: &str) -> Vec<&str> {
    let s = s.trim_start_matches(py_isspace);
    if s.is_empty() {
        return Vec::new();
    }
    match s.find(py_isspace) {
        None => vec![s],
        Some(i) => {
            let rest = s[i..].trim_start_matches(py_isspace);
            if rest.is_empty() {
                vec![&s[..i]]
            } else {
                vec![&s[..i], rest]
            }
        }
    }
}

/// `str.isdigit()` for one char of a latin-1 decoded string: `0` to `9` and the superscript
/// digits U+00B2, U+00B3, U+00B9 (for which `int(c)` then raises). Other chars are never
/// digits here.
pub fn py_isdigit(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '\u{b2}' | '\u{b3}' | '\u{b9}')
}

/// `str.lower()`.
pub fn py_lower(s: &str) -> String {
    s.to_lowercase()
}

/// `str.upper()`.
pub fn py_upper(s: &str) -> String {
    s.to_uppercase()
}

/// `s[start:end]` with char indices and Python's clamping (`s[:3]`, `s[i:i + 2]`); `end` past
/// the length is the length, `start` past the end gives the empty string.
pub fn py_slice(s: &[char], start: usize, end: usize) -> String {
    let end = end.min(s.len());
    if start >= end {
        String::new()
    } else {
        s[start..end].iter().collect()
    }
}

// ------------------------------------------------------------------ dicts

/// A dict key that can print its Python `repr` (the text of a `KeyError`).
pub trait PyKey {
    fn py_repr(&self) -> String;
}
impl PyKey for Py {
    fn py_repr(&self) -> String {
        self.repr()
    }
}
impl PyKey for String {
    fn py_repr(&self) -> String {
        py_str_repr(self)
    }
}
impl PyKey for &str {
    fn py_repr(&self) -> String {
        py_str_repr(self)
    }
}
impl PyKey for i64 {
    fn py_repr(&self) -> String {
        self.to_string()
    }
}

/// The dict operations whose Python meaning differs from `IndexMap`'s defaults. A Python dict
/// keeps insertion order; assigning to an existing key keeps its place (`IndexMap::insert` does
/// the same); `del` followed by a new insert moves the key to the end, so deletion must be
/// `shift_remove` (which keeps the order of the rest), never `swap_remove`.
pub trait PyDict<K, V> {
    /// `del d[k]`: `KeyError` as a raw error when absent.
    fn py_del(&mut self, k: &K) -> Result<V>
    where
        K: PyKey;
    /// `d.setdefault(k, v)`: the stored value, inserting `v` at the end when absent.
    fn py_setdefault(&mut self, k: K, v: V) -> &mut V;
    /// `d.pop(k, None)`.
    fn py_pop(&mut self, k: &K) -> Option<V>;
}

impl<K: Hash + Eq, V> PyDict<K, V> for IndexMap<K, V> {
    fn py_del(&mut self, k: &K) -> Result<V>
    where
        K: PyKey,
    {
        self.shift_remove(k).ok_or_else(|| py("KeyError", k.py_repr()))
    }
    fn py_setdefault(&mut self, k: K, v: V) -> &mut V {
        self.entry(k).or_insert(v)
    }
    fn py_pop(&mut self, k: &K) -> Option<V> {
        self.shift_remove(k)
    }
}
