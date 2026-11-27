//! Python-exact number and string formatting, and the Lua text the tools write (LUA-15c).
//!
//! The converter's output must be byte-equal to the Python tools', so the conversions Python's
//! `repr()`, `str()` and f-strings do are reproduced here instead of using Rust's `{}`/`{:?}`/
//! `{:#x}`, which differ (`1e-05`, `1e+16`, `-0x4`, `inf`, `-0.0`, quote choice and escapes).
//! The tools use no `%`-style conversion in Python (the `%` formats in `PRELUDE` are Lua text);
//! the f-string conversions they use are `{x}` (`str`), `{x:#x}`, `{x:x}` and `repr(x)`.

use crate::pyval::Py;
use crate::{DecodeError, Result};

// ------------------------------------------------------------------ numbers

/// Python's `repr(x)` of a float (also `str(x)`): the shortest digits that round-trip, plain
/// notation for decimal exponents from -4 to 15 and scientific notation outside that
/// (`1e-05`, `1.5e+16`, `1e+16`), always with `.0` for an integral value in plain notation,
/// `nan`, `inf`, `-inf`, and `-0.0` for negative zero.
///
/// Rust's `{:e}` (shortest round-trip digits) supplies the digits; the layout is Python's
/// (`format_float_short` with the `r` code).
pub fn py_float_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    let sign = if x.is_sign_negative() { "-" } else { "" };
    let mut e = format!("{:e}", x.abs());
    // Rust's shortest digits break an exact tie upward, Python's round half to even: take the
    // correctly rounded decimal of the same length when it still reads back as `x`.
    let ndigits = e.split_once('e').map_or(1, |(m, _)| m.chars().filter(char::is_ascii_digit).count());
    let exact = format!("{:.*e}", ndigits - 1, x.abs());
    if exact.parse::<f64>().ok() == Some(x.abs()) {
        e = exact;
    }
    let (mant, exp) = e.split_once('e').expect("{:e} has an exponent");
    let exp: i32 = exp.parse().expect("{:e} exponent is an integer");
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let decpt = exp + 1; // value = 0.DIGITS * 10^decpt
    let n = digits.len() as i32;
    let body = if -4 < decpt && decpt <= 16 {
        if decpt <= 0 {
            format!("0.{}{}", "0".repeat((-decpt) as usize), digits)
        } else if decpt >= n {
            format!("{}{}.0", digits, "0".repeat((decpt - n) as usize))
        } else {
            format!("{}.{}", &digits[..decpt as usize], &digits[decpt as usize..])
        }
    } else {
        let frac = if n > 1 { format!(".{}", &digits[1..]) } else { String::new() };
        format!("{}{}e{:+03}", &digits[..1], frac, exp)
    };
    format!("{sign}{body}")
}

/// Python's `f"{n:#x}"`: `0x1f`, `0x0`, and for a negative number a minus sign before the prefix
/// (`-0x4`), not two's complement as Rust's `{:#x}` prints.
pub fn py_hex(n: i64) -> String {
    if n < 0 {
        format!("-{:#x}", n.unsigned_abs())
    } else {
        format!("{n:#x}")
    }
}

/// Python's `f"{n:x}"`: lowercase hex, a minus sign for a negative number (`-4`).
pub fn py_hex_plain(n: i64) -> String {
    if n < 0 {
        format!("-{:x}", n.unsigned_abs())
    } else {
        format!("{n:x}")
    }
}

// ------------------------------------------------------------------ strings

/// Whether Python's `str.isprintable()` holds for a char. Exact for chars up to U+00FF (the
/// strings are latin-1 decoded: 0x00 to 0x1f, 0x7f to 0xa0 and 0xad are not printable); beyond
/// that an approximation of the Unicode categories (controls, format chars, separators,
/// surrogate and private-use blocks) that the tools' inputs never reach.
fn is_printable(c: char) -> bool {
    let u = c as u32;
    match u {
        0..=0x1f | 0x7f..=0xa0 | 0xad => false,
        0x20..=0x7e | 0xa1..=0xac | 0xae..=0xff => true,
        0x600..=0x605 | 0x61c | 0x6dd | 0x70f | 0x1680 | 0x180e => false,
        0x2000..=0x200f | 0x2028..=0x202f | 0x205f..=0x206f | 0x3000 | 0xfeff | 0xfff9..=0xfffb => false,
        0xd800..=0xf8ff => false,
        _ => !c.is_control(),
    }
}

/// Python's `repr(s)` of a string: single quotes unless the text has a `'` and no `"`, the quote
/// and `\` escaped, `\n`, `\r`, `\t`, other non-printable chars as `\xNN`, `\uNNNN`, `\UNNNNNNNN`.
pub fn py_str_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if is_printable(c) => out.push(c),
            c => {
                let u = c as u32;
                if u <= 0xff {
                    out.push_str(&format!("\\x{u:02x}"));
                } else if u <= 0xffff {
                    out.push_str(&format!("\\u{u:04x}"));
                } else {
                    out.push_str(&format!("\\U{u:08x}"));
                }
            }
        }
    }
    out.push(quote);
    out
}

// ------------------------------------------------------------------ Lua text

/// `lua_str` of `drl_to_lua.py` (also `dsl_to_lua.lua_str`): a single-quoted Lua string literal;
/// only `\` and `'` are escaped, newlines and tabs pass through raw.
pub fn lua_str(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// `lua_str(x)` for a [`Py`] value: a raw `AttributeError` when it is not a string (Python's
/// `str.replace` on a non-string).
pub fn lua_str_py(v: &Py) -> Result<String> {
    match v {
        Py::Str(s) => Ok(lua_str(s)),
        other => Err(DecodeError::python(
            "AttributeError",
            format!("'{}' object has no attribute 'replace'", other.type_name()),
        )),
    }
}

/// `lua_num` of `drl_to_lua.py` for a float: `(0/0)` for NaN, `math.huge` / `(-math.huge)`,
/// `N.0` for an integral value below 1e15 in magnitude (`-0.0` prints as `0.0`), else Python's
/// `repr`.
pub fn lua_num_f64(x: f64) -> String {
    if x.is_nan() {
        return "(0/0)".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "math.huge" } else { "(-math.huge)" }.to_owned();
    }
    if x == x.trunc() && x.abs() < 1e15 {
        return format!("{}.0", x as i64);
    }
    py_float_repr(x)
}

/// `lua_num(x)` for any value as Python's function takes it: a float as [`lua_num_f64`], anything
/// else as `str(x)` (an int prints as decimal, `True` as `True`).
pub fn lua_num(x: &Py) -> String {
    match x {
        Py::Float(f) => lua_num_f64(*f),
        other => other.to_str(),
    }
}

/// `lua_num(n)` for an int.
pub fn lua_num_int(n: i64) -> String {
    n.to_string()
}

fn ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

/// `lua_ident` of `drl_to_lua.py`: the string itself when it is `[A-Za-z_][A-Za-z0-9_]*`
/// (`re.fullmatch`), else `None`.
pub fn lua_ident(s: &str) -> Option<&str> {
    let mut it = s.chars();
    match it.next() {
        Some(c) if ident_start(c) && it.all(|c| c.is_ascii_alphanumeric() || c == '_') => Some(s),
        _ => None,
    }
}

/// `lua_ident` of `a1_to_lua.py`: every char outside `[A-Za-z0-9_]` becomes `_`, and a result
/// that does not start with a letter or `_` gets a `_` in front (so the empty string gives `_`).
pub fn a1_lua_ident(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if t.chars().next().is_some_and(ident_start) {
        t
    } else {
        format!("_{t}")
    }
}

// ------------------------------------------------------------------ render

/// The `ctx` argument of `render`: how the value is used in the Lua text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ctx {
    /// `"bool"`: a Lua condition.
    Bool,
    /// `"num"`: a number.
    Num,
    /// `"arg"`: an argument of an engine call.
    Arg,
}

/// `render` of `drl_to_lua.py`: the Lua source of a condition expression. Errors are Python's:
/// a [`DecodeError`] for a node kind it cannot render, raw errors for a malformed tree.
pub fn render(e: &Py, ctx: Ctx) -> Result<String> {
    let k = e.tag()?;
    if matches!(k, Some("cmp" | "not" | "and" | "or" | "truth")) {
        let s = match k {
            Some("cmp") => format!(
                "{} {} {}",
                render(e.item(2)?, Ctx::Num)?,
                e.item(1)?.to_str(),
                render(e.item(3)?, Ctx::Num)?
            ),
            Some("not") => {
                if e.item(1)?.tag_in(&["truth", "call"])? {
                    format!("not {}", render(e.item(1)?, Ctx::Bool)?)
                } else {
                    format!("not ({})", render(e.item(1)?, Ctx::Bool)?)
                }
            }
            Some("and") => join_paren(e.item(1)?, " and ")?,
            Some("or") => join_paren(e.item(1)?, " or ")?,
            _ => {
                let inner = e.item(1)?;
                let c = if inner.tag_is("call")? { Ctx::Arg } else { Ctx::Num };
                format!("T({})", render(inner, c)?)
            }
        };
        return Ok(if ctx == Ctx::Bool { s } else { format!("N({s})") });
    }
    match k {
        Some("boolv") => render(e.item(1)?, ctx),
        Some("boolarg") => {
            let x = e.item(1)?;
            let s = if x.tag_is("const")? {
                if x.item(1)?.truthy() { "true".to_owned() } else { "false".to_owned() }
            } else {
                format!("T({})", render(x, Ctx::Arg)?)
            };
            Ok(if ctx != Ctx::Num { s } else { format!("N({s})") })
        }
        Some("const") => {
            if ctx == Ctx::Bool {
                Ok(if e.item(1)?.truthy() { "true" } else { "false" }.to_owned())
            } else {
                Ok(lua_num(e.item(1)?))
            }
        }
        Some("float") => Ok(lua_num(e.item(1)?)),
        Some("str") => lua_str_py(e.item(1)?),
        Some("var") => {
            let n = e.item(1)?.to_str();
            Ok(if ctx != Ctx::Bool { format!("S.{n}") } else { format!("T(S.{n})") })
        }
        Some("obj" | "objf") => {
            let n = e.item(1)?.to_str();
            Ok(match ctx {
                Ctx::Arg => format!("O.{n}"),
                Ctx::Num => format!("N(O.{n})"),
                Ctx::Bool => format!("T(O.{n})"),
            })
        }
        Some("elem") => {
            let n = e.item(1)?.to_str();
            let idx = py_add_one(e.item(2)?)?;
            let s = format!("(O.{n} or {{}})[{idx}]");
            Ok(match ctx {
                Ctx::Arg => s,
                Ctx::Num => format!("N({s})"),
                Ctx::Bool => format!("T({s})"),
            })
        }
        Some("array") => Ok(format!("O.{}", e.item(1)?.to_str())),
        Some("call") => {
            let mut parts = Vec::new();
            for x in e.item(2)?.items()? {
                parts.push(render(x, Ctx::Arg)?);
            }
            let args = parts.join(", ");
            let s = format!(
                "A({}{})",
                lua_str_py(e.item(1)?)?,
                if args.is_empty() { String::new() } else { format!(", {args}") }
            );
            Ok(match ctx {
                Ctx::Num => format!("N({s})"),
                Ctx::Bool => format!("T({s})"),
                Ctx::Arg => s,
            })
        }
        Some("arith") => {
            let s = format!(
                "({} {} {})",
                render(e.item(2)?, Ctx::Num)?,
                e.item(1)?.to_str(),
                render(e.item(3)?, Ctx::Num)?
            );
            Ok(if ctx != Ctx::Bool { s } else { format!("T({s})") })
        }
        Some("bool") => render(e.item(1)?, ctx),
        _ => Err(DecodeError::new(format!("cannot render {}", e.item(0)?.to_str()))),
    }
}

/// `x + 1` for the index of an `elem` node, printed as Python prints the sum.
fn py_add_one(v: &Py) -> Result<String> {
    match v {
        Py::Int(n) => n
            .checked_add(1)
            .map(|m| m.to_string())
            .ok_or_else(|| DecodeError::raw("integer overflow")),
        Py::Bool(b) => Ok((i64::from(*b) + 1).to_string()),
        Py::Float(x) => Ok(py_float_repr(x + 1.0)),
        // a str, tuple or list says it can only concatenate
        Py::Str(_) | Py::Tuple(_) | Py::List(_) => Err(DecodeError::python(
            "TypeError",
            format!("can only concatenate {0} (not \"int\") to {0}", v.type_name()),
        )),
        other => Err(DecodeError::python(
            "TypeError",
            format!("unsupported operand type(s) for +: '{}' and 'int'", other.type_name()),
        )),
    }
}

fn join_paren(items: &Py, sep: &str) -> Result<String> {
    let mut parts = Vec::new();
    for x in items.items()? {
        parts.push(paren(x)?);
    }
    Ok(parts.join(sep))
}

/// `paren` of `drl_to_lua.py`: `render(e, "bool")`, parenthesised when `e` is an `and` or `or`.
pub fn paren(e: &Py) -> Result<String> {
    let s = render(e, Ctx::Bool)?;
    Ok(if e.tag_in(&["and", "or"])? { format!("({s})") } else { s })
}
