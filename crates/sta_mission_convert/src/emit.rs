//! Naming and rendering of the lifted functions (LUA-15h): `Namer`, `render_expr`, `vkey`,
//! `render_lv`, `first_effect`, `stmt_exprs`, `inline_temps`, `render_function` and `render_stmt`
//! of `tools/dsl_to_lua.py`, line for line.
//!
//! Expressions and statements are the lifter's tuples ([`Py`]); the renderer walks them with the
//! Python's own tests (a node whose head is not a string, a missing element, an `int` where a
//! tuple is expected), so a malformed tree gives the Python's exception class and text. A
//! [`DecodeError`] of kind `Decode` is what the Python raises as `DecodeError` (the function is
//! then written as not decoded); every other kind aborts the run.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::dsl::Dsl;
use crate::format::{lua_ident, lua_num, lua_str, lua_str_py, py_hex, py_hex_plain, Ctx};
use crate::lift::Lifted;
use crate::pyval::{py_upper, Py};
use crate::{DecodeError, Result};

fn dec<T>(msg: String) -> Result<T> {
    Err(DecodeError::new(msg))
}

/// `e[i]` as an int for a site where Python adds or formats it.
fn int(v: &Py, op: &str) -> Result<i64> {
    match v.as_int() {
        Some(n) => Ok(n),
        None => Err(DecodeError::python(
            "TypeError",
            format!("unsupported operand type(s) for {op}: '{}' and 'int'", v.type_name()),
        )),
    }
}

fn py_err<T>(class: &'static str, msg: impl Into<String>) -> Result<T> {
    Err(DecodeError::python(class, msg))
}

/// `f"{v:x}"` (`alt` false) or `f"{v:#x}"` (`alt` true) of any value, with CPython's exceptions
/// for the ones that have no hex form.
fn hex_fmt(v: &Py, alt: bool) -> Result<String> {
    match v {
        Py::Int(_) | Py::Bool(_) => {
            let n = v.as_int().unwrap_or(0);
            Ok(if alt { py_hex(n) } else { py_hex_plain(n) })
        }
        Py::Float(_) | Py::Str(_) => py_err("ValueError", format!("Unknown format code 'x' for object of type '{}'", v.type_name())),
        other => py_err("TypeError", format!("unsupported format string passed to {}.__format__", other.type_name())),
    }
}

/// `v - n` for an int `n` (a float stays a float).
fn sub_num(v: &Py, n: i64) -> Result<Py> {
    match v {
        Py::Int(_) | Py::Bool(_) => Ok(Py::Int(v.as_int().unwrap_or(0) - n)),
        Py::Float(f) => Ok(Py::Float(f - n as f64)),
        other => py_err("TypeError", format!("unsupported operand type(s) for -: '{}' and 'int'", other.type_name())),
    }
}

/// `str(v + 1)`.
fn add_one(v: &Py) -> Result<String> {
    match v {
        Py::Int(_) | Py::Bool(_) => Ok((v.as_int().unwrap_or(0) + 1).to_string()),
        Py::Float(f) => Ok(Py::Float(f + 1.0).to_str()),
        Py::Str(_) | Py::Tuple(_) | Py::List(_) => {
            let t = v.type_name();
            py_err("TypeError", format!("can only concatenate {t} (not \"int\") to {t}"))
        }
        other => py_err("TypeError", format!("unsupported operand type(s) for +: '{}' and 'int'", other.type_name())),
    }
}

/// `lv, v = p`: the two elements of a pair, with CPython's unpacking errors.
fn pair(p: &Py) -> Result<(Py, Py)> {
    let Some(items) = p.as_seq() else {
        return py_err("TypeError", format!("cannot unpack non-iterable {} object", p.type_name()));
    };
    match items.len() {
        2 => Ok((items[0].clone(), items[1].clone())),
        n if n > 2 => py_err("ValueError", format!("too many values to unpack (expected 2, got {n})")),
        n => py_err("ValueError", format!("not enough values to unpack (expected 2, got {n})")),
    }
}

/// The text `set.add(v)` raises for an unhashable `v`, `Ok` for a hashable one.
fn hashable(v: &Py) -> Result<()> {
    fn inner(v: &Py) -> Option<&'static str> {
        match v {
            Py::List(_) => Some("list"),
            Py::Tuple(items) => items.iter().find_map(inner),
            _ => None,
        }
    }
    match inner(v) {
        None => Ok(()),
        Some(t) => py_err("TypeError", format!("cannot use '{}' as a set element (unhashable type: '{t}')", v.type_name())),
    }
}

/// `v.upper()` of a value that should be a string.
fn upper_of(v: &Py) -> Result<String> {
    match v {
        Py::Str(s) => Ok(py_upper(s)),
        other => py_err("AttributeError", format!("'{}' object has no attribute 'upper'", other.type_name())),
    }
}

/// `Namer`: the Lua name of each function.
pub struct Namer<'d, 'a> {
    pub d: &'d Dsl<'a>,
    pub names: HashMap<i64, String>,
    /// LUA-15e edit: the game being rendered. For Armada I, `render_expr` first asks
    /// `a1_to_lua.render_expr`'s own cases (the Python patches `dsl.render_expr`).
    pub game: crate::lift::Game,
    /// LUA-15e edit: set for `a1_to_lua.CondNamer` (names by the table, else `R_fn_<offset>`).
    pub cond: bool,
}

impl<'d, 'a> Namer<'d, 'a> {
    /// `Namer.__init__`: the exported names, then `<owner>_step[k]` for the steps of a named
    /// function (in address order), then `<role>_<offset>` for the rest.
    pub fn new(d: &'d Dsl<'a>, funcs: &IndexMap<i64, Lifted>, roles: &HashMap<i64, String>) -> Self {
        let mut names: HashMap<i64, String> = HashMap::new();
        for (a, n) in &d.code_names {
            names.insert(*a, n.clone());
        }
        let mut addrs: Vec<i64> = funcs.keys().copied().collect();
        addrs.sort_unstable();
        let base = i64::from(d.pe.base());
        for a in &addrs {
            let f = &funcs[a];
            let owner = names.get(a).cloned().filter(|o| !o.is_empty());
            if let Some(owner) = owner
                && !f.steps.is_empty()
            {
                for (k, s) in f.steps.iter().enumerate() {
                    if !names.contains_key(s) {
                        let n = if f.steps.len() > 1 { format!("{owner}_step{}", k + 1) } else { format!("{owner}_step") };
                        names.insert(*s, n);
                    }
                }
            }
        }
        for a in &addrs {
            if !names.contains_key(a) {
                let role = roles.get(a).map_or("fn", String::as_str);
                names.insert(*a, format!("{role}_{}", py_hex_plain(a - base)));
            }
        }
        Namer { d, names, game: crate::lift::Game::A2, cond: false }
    }

    /// LUA-15e edit: `CondNamer(d, names)` of `a1_to_lua.py`.
    pub fn cond_namer(d: &'d Dsl<'a>, names: HashMap<i64, String>) -> Self {
        Namer { d, names, game: crate::lift::Game::A1, cond: true }
    }

    /// `Namer.fn`: the Lua identifier of the function at `a`.
    pub fn fn_name(&self, a: i64) -> String {
        if self.cond {
            // `CondNamer.fn`: `self.names.get(a) or f"R_fn_{a - base:x}"`
            return match self.names.get(&a).filter(|n| !n.is_empty()) {
                Some(n) => n.clone(),
                None => format!("R_fn_{}", py_hex_plain(a - i64::from(self.d.pe.base()))),
            };
        }
        let fallback = || format!("fn_{}", py_hex_plain(a - i64::from(self.d.pe.base())));
        match self.names.get(&a).filter(|n| !n.is_empty()) {
            None => fallback(),
            Some(n) => {
                if lua_ident(n).is_some() {
                    n.clone()
                } else {
                    fallback()
                }
            }
        }
    }

    /// The label text `b{address - base:x}`.
    fn label(&self, a: &Py) -> Result<String> {
        Ok(format!("b{}", hex_fmt(&sub_num(a, i64::from(self.d.pe.base()))?, false)?))
    }
}

/// `", " + render_expr(x, nm, "arg")` for each element of the list `v`, concatenated.
fn comma_args(v: &Py, nm: &Namer<'_, '_>) -> Result<String> {
    let mut out = String::new();
    for x in v.items()? {
        out.push_str(", ");
        out.push_str(&render_expr(x, nm, Ctx::Arg)?);
    }
    Ok(out)
}

/// `render_expr(e, nm, ctx)`: the Lua source of an expression; `ctx` is `Num` (a number), `Bool`
/// or `Arg`.
pub fn render_expr(e: &Py, nm: &Namer<'_, '_>, ctx: Ctx) -> Result<String> {
    if nm.game == crate::lift::Game::A1
        && let Some(r) = crate::a1::render::intercept(e, ctx)
    {
        return r; // LUA-15e edit: `a1_to_lua.render_expr` wraps this function
    }
    let kv = e.item(0)?;
    let k = kv.as_str().unwrap_or("");
    if matches!(k, "cmp" | "not" | "and" | "or" | "truth") {
        let s = match k {
            "cmp" => format!(
                "{} {} {}",
                render_expr(e.item(2)?, nm, Ctx::Num)?,
                e.item(1)?.to_str(),
                render_expr(e.item(3)?, nm, Ctx::Num)?
            ),
            "not" => format!("not ({})", render_expr(e.item(1)?, nm, Ctx::Bool)?),
            "and" | "or" => {
                let mut parts: Vec<String> = Vec::new();
                for x in e.item(1)?.items()? {
                    parts.push(format!("({})", render_expr(x, nm, Ctx::Bool)?));
                }
                parts.join(&format!(" {k} "))
            }
            _ => format!("T({})", render_expr(e.item(1)?, nm, Ctx::Arg)?),
        };
        return Ok(if ctx == Ctx::Bool { s } else { format!("N({s})") });
    }
    if k == "const" {
        return Ok(if ctx == Ctx::Bool {
            if e.item(1)?.truthy() { "true" } else { "false" }.to_owned()
        } else {
            lua_num(e.item(1)?)
        });
    }
    if k == "float" {
        let s = lua_num(e.item(1)?);
        return Ok(if ctx != Ctx::Bool { s } else { format!("T({s})") });
    }
    if k == "str" {
        return lua_str_py(e.item(1)?);
    }
    let s = match k {
        "var" => format!("S.{}", e.item(1)?.to_str()),
        "obj" => format!("O.{}", e.item(1)?.to_str()),
        "elem" => format!("O.{}[{}]", e.item(1)?.to_str(), render_expr(e.item(2)?, nm, Ctx::Num)?),
        "vfield" => format!("O.{}[{}]", e.item(1)?.to_str(), add_one(e.item(2)?)?),
        "loc" => format!("L[{}]", e.item(1)?.to_str()),
        "reg" => format!("R.{}", e.item(1)?.to_str()),
        "tmp" => format!("t[{}]", e.item(1)?.to_str()),
        "glob" => format!("G[{}]", hex_fmt(&sub_num(e.item(1)?, 0)?, true)?),
        "param" => format!("P[{}]", add_one(e.item(1)?)?),
        "call" => {
            let args = comma_args(e.item(2)?, nm)?;
            format!("A({}{args})", lua_str_py(e.item(1)?)?)
        }
        "callout" => {
            let args = comma_args(e.item(2)?, nm)?;
            format!("AO({}{args})", lua_str_py(e.item(1)?)?)
        }
        "outval" => format!("OUTV({})", render_lv(e.item(1)?, nm)?),
        "sprintf" => {
            let args = comma_args(e.item(2)?, nm)?;
            format!("SPRINTF({}{args})", render_expr(e.item(1)?, nm, Ctx::Arg)?)
        }
        "strcmp" => format!(
            "STRCMP({}, {})",
            render_expr(e.item(1)?, nm, Ctx::Arg)?,
            render_expr(e.item(2)?, nm, Ctx::Arg)?
        ),
        "slen" => format!("#({})", render_expr(e.item(1)?, nm, Ctx::Arg)?),
        "fbits" => format!("FB({})", render_lv(e.item(1)?, nm)?),
        "fcall" => {
            let mut parts: Vec<String> = Vec::new();
            for x in e.item(2)?.items()? {
                parts.push(render_expr(x, nm, Ctx::Arg)?);
            }
            format!("M.{}({})", nm.fn_name(int(e.item(1)?, "-")?), parts.join(", "))
        }
        "vop" => {
            let args = comma_args(e.item(3)?, nm)?;
            format!("V{}({}{args})", upper_of(e.item(1)?)?, vkey(e.item(2)?)?)
        }
        "vget" => format!("VGET({}, {})", vkey(e.item(1)?)?, render_expr(e.item(2)?, nm, Ctx::Num)?),
        "arith" => {
            let op = e.item(1)?;
            let x = render_expr(e.item(2)?, nm, Ctx::Num)?;
            let y = render_expr(e.item(3)?, nm, Ctx::Num)?;
            match op.as_str().unwrap_or("") {
                o @ ("+" | "-" | "*") => format!("I32({x} {o} {y})"),
                "/" => format!("({x} / {y})"),
                "//" => format!("IDIV({x}, {y})"),
                "%" => format!("IMOD({x}, {y})"),
                "&" => format!("(I({x}) & I({y}))"),
                "|" => format!("(I({x}) | I({y}))"),
                "~" => format!("(I({x}) ~ I({y}))"),
                "<<" => format!("I32(I({x}) << I({y}))"),
                ">>" => format!("SAR({x}, {y})"),
                ">>>" => format!("((I({x}) & 0xffffffff) >> I({y}))"),
                _ => return dec(format!("cannot render arith {}", op.to_str())),
            }
        }
        "u" => format!("U({})", render_expr(e.item(1)?, nm, Ctx::Num)?),
        "farith" => format!(
            "({} {} {})",
            render_expr(e.item(2)?, nm, Ctx::Num)?,
            e.item(1)?.to_str(),
            render_expr(e.item(3)?, nm, Ctx::Num)?
        ),
        "neg" => format!("(-({}))", render_expr(e.item(1)?, nm, Ctx::Num)?),
        "bnot" => format!("(~I({}))", render_expr(e.item(1)?, nm, Ctx::Num)?),
        "abs" => format!("math.abs({})", render_expr(e.item(1)?, nm, Ctx::Num)?),
        "sqrt" => format!("math.sqrt({})", render_expr(e.item(1)?, nm, Ctx::Num)?),
        "ftol" => format!("FTOL({})", render_expr(e.item(1)?, nm, Ctx::Num)?),
        "sign" => format!("(N({}) < 0 and -1 or 0)", render_expr(e.item(1)?, nm, Ctx::Num)?),
        "sbb" => {
            let fl = e.item(1)?;
            if fl.truthy() && fl.tag_is("test")? && fl.item(1)?.truthy() && fl.item(1)?.tag_is("neg")? {
                format!("(T({}) and -1 or 0)", render_expr(fl.item(1)?.item(1)?, nm, Ctx::Arg)?)
            } else if fl.truthy() && fl.tag_is("cmp")? {
                format!(
                    "(ULT({}, {}) and -1 or 0)",
                    render_expr(fl.item(1)?, nm, Ctx::Num)?,
                    render_expr(fl.item(2)?, nm, Ctx::Num)?
                )
            } else {
                return dec("cannot render sbb".to_owned());
            }
        }
        "mcall" => {
            let args = comma_args(e.item(4)?, nm)?;
            format!(
                "FM({}, {}, {}{args})",
                render_expr(e.item(1)?, nm, Ctx::Arg)?,
                lua_str_py(e.item(2)?)?,
                lua_str_py(e.item(3)?)?
            )
        }
        "objref" => render_expr(e.item(1)?, nm, Ctx::Arg)?,
        "boolarg" => {
            let x = e.item(1)?;
            if x.tag_is("const")? {
                if x.item(1)?.truthy() { "true" } else { "false" }.to_owned()
            } else {
                format!("T({})", render_expr(x, nm, Ctx::Arg)?)
            }
        }
        "locidx" => format!("L[{} + 4 * I({})]", e.item(1)?.to_str(), render_expr(e.item(2)?, nm, Ctx::Num)?),
        "code" => format!("M.{}", nm.fn_name(nm.d.entry(int(e.item(1)?, "-")?)?)),
        // an address reaches Lua only as the value the callee reads (`Lifter.by_pointer`)
        "addr" => return dec(format!("the address of {} is passed on as a value", e.item(1)?.item(0)?.to_str())),
        "varptr" => format!("V.{}", e.item(1)?.to_str()),
        "script" => render_expr(e.item(1)?, nm, Ctx::Num)?,
        "field" => format!("VEC({}, {})", render_expr(e.item(1)?, nm, Ctx::Arg)?, add_one(e.item(2)?)?),
        "vec" => {
            let mut parts: Vec<String> = Vec::new();
            for x in e.item(1)?.items()? {
                parts.push(render_expr(x, nm, Ctx::Num)?);
            }
            format!("Vector({})", parts.join(", "))
        }
        "newvar" => format!("NewVar({}, {})", lua_str_py(e.item(1)?)?, render_expr(e.item(2)?, nm, Ctx::Arg)?),
        "enginevar" => format!("EngineVar({}, {})", lua_str_py(e.item(1)?)?, lua_str_py(e.item(2)?)?),
        "varq" => format!("VarQuery({}, {})", lua_str_py(e.item(1)?)?, lua_str_py(e.item(2)?)?),
        "newscript" => format!("NewScript({})", render_expr(e.item(1)?, nm, Ctx::Arg)?),
        "this" => "P.this".to_owned(),
        "si" | "world" | "iiie" => py_upper(k),
        _ => return dec(format!("cannot render {}", kv.to_str())),
    };
    if ctx == Ctx::Bool {
        return Ok(format!("T({s})"));
    }
    if ctx == Ctx::Num
        && matches!(
            k,
            "call" | "callout" | "strcmp" | "slen" | "fbits" | "tmp" | "var" | "obj" | "reg" | "loc" | "glob" | "param"
                | "elem" | "fcall" | "field" | "vfield" | "locidx" | "mcall" | "objref"
        )
    {
        return Ok(format!("N({s})"));
    }
    Ok(s)
}

/// `vkey(key)`: the Lua table of a `std::vector` held in a stack slot or a static.
pub fn vkey(key: &Py) -> Result<String> {
    Ok(if key.tag_is("loc")? {
        format!("VL(L, {})", key.item(1)?.to_str())
    } else {
        format!("VG({})", hex_fmt(key.item(1)?, true)?)
    })
}

/// `render_lv(lv, nm)`: the Lua location of an lvalue.
pub fn render_lv(lv: &Py, nm: &Namer<'_, '_>) -> Result<String> {
    let kv = lv.item(0)?;
    let k = kv.as_str().unwrap_or("");
    Ok(match k {
        "obj" => format!("O.{}", lv.item(1)?.to_str()),
        "elem" => format!("O.{}[{}]", lv.item(1)?.to_str(), render_expr(lv.item(2)?, nm, Ctx::Num)?),
        "vfield" => format!("O.{}[{}]", lv.item(1)?.to_str(), add_one(lv.item(2)?)?),
        "loc" => format!("L[{}]", lv.item(1)?.to_str()),
        "locidx" => format!("L[{} + 4 * I({})]", lv.item(1)?.to_str(), render_expr(lv.item(2)?, nm, Ctx::Num)?),
        "gvar" => format!("G[{}]", hex_fmt(lv.item(1)?, true)?),
        "param" => format!("P[{}]", add_one(lv.item(1)?)?),
        "reg" => format!("R.{}", lv.item(1)?.to_str()),
        "tmp" => format!("t[{}]", lv.item(1)?.to_str()),
        _ => return dec(format!("cannot assign to {}", kv.to_str())),
    })
}

/// `CALLS`: the node kinds that run engine code.
const CALLS: [&str; 9] = ["call", "callout", "fcall", "mcall", "newvar", "varq", "newscript", "enginevar", "vop"];

/// `first_effect(e, n)`: in Lua's evaluation order (arguments left to right, then the call),
/// whether temp `n` is reached (`"tmp"`) before any call completes (`"call"`).
pub fn first_effect(e: &Py, n: &Py) -> Option<&'static str> {
    match e {
        Py::List(items) => {
            for x in items {
                if let Some(r) = first_effect(x, n) {
                    return Some(r);
                }
            }
            None
        }
        Py::Tuple(items) if !items.is_empty() => {
            if matches!(e, Py::Tuple(v) if v.len() == 2 && v[0] == Py::from("tmp") && v[1] == *n) {
                return Some("tmp");
            }
            let Some(head) = items[0].as_str() else {
                return first_effect(&Py::List(items.clone()), n);
            };
            for x in &items[1..] {
                if let Some(r) = first_effect(x, n) {
                    return Some(r);
                }
            }
            if CALLS.contains(&head) { Some("call") } else { None }
        }
        _ => None,
    }
}

/// `stmt_exprs(s)`: the expressions a statement evaluates.
pub fn stmt_exprs(s: &Py) -> Result<Vec<Py>> {
    let k = s.item(0)?.as_str().unwrap_or("");
    Ok(match k {
        "set" => vec![s.item(2)?.clone()],
        "multiset" => {
            let mut out = Vec::new();
            for p in s.item(1)?.items()? {
                // `for _, v in s[1]`: a pair
                let pair = p.items()?;
                if pair.len() != 2 {
                    return Err(DecodeError::python(
                        "ValueError",
                        if pair.len() > 2 {
                            format!("too many values to unpack (expected 2, got {})", pair.len())
                        } else {
                            format!("not enough values to unpack (expected 2, got {})", pair.len())
                        },
                    ));
                }
                out.push(pair[1].clone());
            }
            out
        }
        "var" => s.item(3)?.items()?.to_vec(),
        "bindvar" => vec![s.item(3)?.clone()],
        "eval" | "if" | "return" | "switch" | "trashrule" | "addstep" => vec![s.item(1)?.clone()],
        _ => Vec::new(),
    })
}

fn count_uses(e: &Py, uses: &mut HashMap<Py, i64>) {
    match e {
        Py::Tuple(items) => {
            if items.first().is_some_and(|h| h.as_str() == Some("tmp")) {
                if let Some(n) = items.get(1) {
                    *uses.entry(n.clone()).or_insert(0) += 1;
                }
                return;
            }
            for x in items.iter().skip(1) {
                count_uses(x, uses);
            }
        }
        Py::List(items) => {
            for x in items {
                count_uses(x, uses);
            }
        }
        _ => {}
    }
}

fn subst(e: &Py, n: &Py, v: &Py) -> Py {
    match e {
        Py::Tuple(items) => {
            if items.len() == 2 && items[0].as_str() == Some("tmp") && items[1] == *n {
                return v.clone();
            }
            Py::Tuple(items.iter().map(|x| subst(x, n, v)).collect())
        }
        Py::List(items) => Py::List(items.iter().map(|x| subst(x, n, v)).collect()),
        other => other.clone(),
    }
}

/// `inline_temps(blocks)`: `t[n] = <call>` used once, by the next statement of the same block,
/// is written in place of that use.
pub fn inline_temps(blocks: &mut [(i64, Vec<Py>)]) -> Result<()> {
    let mut uses: HashMap<Py, i64> = HashMap::new();
    for (_, stmts) in blocks.iter() {
        for s in stmts {
            if s.tag_is("set")? {
                count_uses(s.item(2)?, &mut uses);
                if !s.item(1)?.tag_is("tmp")? {
                    count_uses(s.item(1)?, &mut uses);
                }
            } else {
                for x in s.items()?.iter().skip(1) {
                    count_uses(x, &mut uses);
                }
            }
        }
    }
    for blk in blocks.iter_mut() {
        let stmts = std::mem::take(&mut blk.1);
        let mut out: Vec<Py> = Vec::new();
        for s in stmts {
            out.push(s);
            while out.len() >= 2 {
                let (prev, cur) = (&out[out.len() - 2], &out[out.len() - 1]);
                let ok = prev.tag_is("set")? && prev.item(1)?.tag_is("tmp")? && uses.get(prev.item(1)?.item(1)?) == Some(&1);
                if !ok {
                    break;
                }
                let n = prev.item(1)?.item(1)?.clone();
                if prev.item(2)?.tag_in(&["newscript", "callout"])? {
                    break;
                }
                if first_effect(&Py::List(stmt_exprs(cur)?), &n) != Some("tmp") {
                    break; // another call would run first: keep the compiler's order
                }
                let new = subst(cur, &n, prev.item(2)?);
                if new == *cur {
                    break;
                }
                let at = out.len() - 2;
                out.truncate(at);
                out.push(new);
            }
        }
        blk.1 = out;
    }
    Ok(())
}

/// `render_function(f, nm, name)`: the Lua text of one function as lines.
pub fn render_function(f: &Lifted, nm: &Namer<'_, '_>, name: &str) -> Result<Vec<String>> {
    let mut lines: Vec<String> = Vec::new();
    if let Some(err) = &f.error {
        lines.push(format!("function M.{name}(...)"));
        lines.push(format!("    M.notDecoded({}, {})", lua_str(name), lua_str(err)));
        lines.push("    return 0".to_owned());
        lines.push("end".to_owned());
        return Ok(lines);
    }
    let mut blocks: Vec<(i64, Vec<Py>)> = f.blocks.clone();
    inline_temps(&mut blocks)?;
    let mut used: HashSet<Py> = HashSet::new();
    for (_, stmts) in &blocks {
        for s in stmts {
            if s.tag_in(&["goto", "if"])? {
                hashable(s.item_back(1)?)?;
                used.insert(s.item_back(1)?.clone());
            }
            if s.tag_is("switch")? {
                for t in s.item(2)?.items()? {
                    hashable(t)?;
                    used.insert(t.clone());
                }
            }
        }
    }
    lines.push(format!("function M.{name}(...)"));
    lines.push("    local P, L, R, t = {...}, {}, {}, {}".to_owned());
    for (b, stmts) in &blocks {
        if used.contains(&Py::Int(*b)) {
            lines.push(format!("    ::{}::", nm.label(&Py::Int(*b))?));
        }
        for s in stmts {
            lines.push(format!("    {}", render_stmt(s, nm)?));
        }
    }
    lines.push("end".to_owned());
    Ok(lines)
}

/// `render_stmt(s, nm)`: the Lua text of one statement.
pub fn render_stmt(s: &Py, nm: &Namer<'_, '_>) -> Result<String> {
    let kv = s.item(0)?;
    let k = kv.as_str().unwrap_or("");
    Ok(match k {
        "set" => format!("{} = {}", render_lv(s.item(1)?, nm)?, render_expr(s.item(2)?, nm, Ctx::Arg)?),
        "multiset" => {
            let mut lvs: Vec<String> = Vec::new();
            let mut vals: Vec<String> = Vec::new();
            for p in s.item(1)?.items()? {
                lvs.push(render_lv(&pair(p)?.0, nm)?);
            }
            for p in s.item(1)?.items()? {
                vals.push(render_expr(&pair(p)?.1, nm, Ctx::Arg)?);
            }
            format!("{} = {}", lvs.join(", "), vals.join(", "))
        }
        "eval" => render_expr(s.item(1)?, nm, Ctx::Arg)?,
        "bindvar" => format!(
            "BindVar({}, {}, {})",
            lua_str_py(s.item(1)?)?,
            lua_str_py(s.item(2)?)?,
            render_expr(s.item(3)?, nm, Ctx::Arg)?
        ),
        "var" => {
            let (name, op, args) = (s.item(1)?, s.item(2)?, s.item(3)?);
            let mut a = String::new();
            for x in args.items()? {
                a.push_str(", ");
                a.push_str(&render_expr(x, nm, Ctx::Num)?);
            }
            format!("VarOp({}, {}{a})", lua_str_py(name)?, lua_str_py(op)?)
        }
        "goto" => format!("goto {}", nm.label(s.item(1)?)?),
        "if" => format!(
            "if {} then goto {} end",
            render_expr(s.item(1)?, nm, Ctx::Bool)?,
            nm.label(s.item(2)?)?
        ),
        "return" => {
            let v = s.item(1)?;
            if v.is_none() { "do return end".to_owned() } else { format!("do return {} end", render_expr(v, nm, Ctx::Arg)?) }
        }
        "comment" => format!("-- {}", s.item(1)?.to_str()),
        "switch" => {
            let idx = render_expr(s.item(1)?, nm, Ctx::Num)?;
            let mut cases: Vec<String> = Vec::new();
            for (i, t) in s.item(2)?.items()?.iter().enumerate() {
                cases.push(format!("if X == {i} then goto {} end", nm.label(t)?));
            }
            format!("do local X = I({idx}) {} end", cases.join(" "))
        }
        "addstep" => {
            let target = int(s.item(2)?, "-")?;
            format!(
                "AddStep({}, M.{}, {})",
                render_expr(s.item(1)?, nm, Ctx::Arg)?,
                nm.fn_name(target),
                lua_str(&nm.fn_name(target))
            )
        }
        "trashrule" => format!("TrashRule({})", render_expr(s.item(1)?, nm, Ctx::Arg)?),
        _ => return dec(format!("cannot render statement {}", kv.to_str())),
    })
}
