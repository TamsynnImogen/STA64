//! The boolean-algebra and expression-tuple helpers of the converter tools (LUA-15c).
//!
//! Line-for-line ports, on the dynamic [`Py`] tree, of the helpers `tools/drl_to_lua.py` uses to
//! build and simplify the conditions of a rule DLL (`simplify`, `absorb`, `factor`, `negate`,
//! `conj`, `as_bool`, `branch_pred`, the atom keys and the truth table, `binop`, `expr_arg`,
//! `typed_args`) and the small tuple-tree helpers of `tools/dsl_to_lua.py` (`contains`, `fbinop`,
//! `truth`, `lv_value`). Condition expressions are tuples:
//!
//! ```text
//! ("const", n)  ("float", x)  ("str", s)  ("var", sym, kind)  ("obj", sym)  ("elem", sym, i)
//! ("call", name, [args])  ("not", x)  ("cmp", op, a, b)  ("arith", op, a, b)
//! ("and", [..])  ("or", [..])  ("bool", x)  ("truth", x)  ("boolv", x)  ("boolarg", x)
//! ```
//!
//! Errors: a fault the Python code raises as `DecodeError` is [`DecodeError::new`] (kind
//! `Decode`), a malformed tree (`IndexError`, `TypeError`, `KeyError`) is a raw error.

use std::collections::HashMap;

use crate::format::py_hex;
use crate::pyval::Py;
use crate::{tup, DecodeError, Result};
use crate::msvc::{import_kind, plain_name};

/// `TRUE` of `drl_to_lua.py` (also `TRUE_` of `dsl_to_lua.py`): `("const", 1)`.
pub fn true_() -> Py {
    tup!["const", 1]
}

/// `FALSE` of `drl_to_lua.py` (also `FALSE_`): `("const", 0)`.
pub fn false_() -> Py {
    tup!["const", 0]
}

/// `NEG[op]`: the comparison with the opposite truth value (`==` and `~=`, `<` and `>=`, `<=`
/// and `>`). A key that is not in the table is a `KeyError` as a raw error.
pub fn neg_op(op: &Py) -> Result<&'static str> {
    Ok(match op.as_str() {
        Some("==") => "~=",
        Some("~=") => "==",
        Some("<") => ">=",
        Some(">=") => "<",
        Some("<=") => ">",
        Some(">") => "<=",
        _ => return Err(DecodeError::python("KeyError", op.repr())),
    })
}

/// `JCC[mn]` of `drl_to_lua.py`: the comparison a conditional jump of the RULE-DLL decoder tests
/// (`je` `jne` `jl` `jge` `jle` `jg`); `None` for any other mnemonic (Python's `KeyError`).
/// This is the rule tool's 6-entry table only. `tools/dsl_to_lua.py` has its own, larger `JCC`
/// (it adds `jb` `jae` `jbe` `ja` `js` `jns`) and its own `SETCC`, and `drl_to_lua.py` has a
/// `SETCC` of its own too: the mission lifter (LUA-15d) and the rule decoder (LUA-15g) each add
/// the tables of their Python module; do not reuse this one for the mission lifter.
pub fn drl_jcc_op(mn: &str) -> Option<&'static str> {
    Some(match mn {
        "je" => "==",
        "jne" => "~=",
        "jl" => "<",
        "jge" => ">=",
        "jle" => "<=",
        "jg" => ">",
        _ => return None,
    })
}

fn jcc_or_key_error(mn: &str) -> Result<&'static str> {
    drl_jcc_op(mn).ok_or_else(|| DecodeError::python("KeyError", Py::from(mn).repr()))
}

/// `negate(p)`: the opposite comparison for a `cmp`, the operand of a `not`, else `("not", p)`.
pub fn negate(p: &Py) -> Result<Py> {
    if p.tag_is("cmp")? {
        return Ok(tup!["cmp", neg_op(p.item(1)?)?, p.item(2)?, p.item(3)?]);
    }
    if p.tag_is("not")? {
        return Ok(p.item(1)?.clone());
    }
    Ok(Py::Tuple(vec![Py::from("not"), p.clone()]))
}

/// `conj(preds)`: `("const", 1)` for none, the predicate itself for one, else `("and", [..])`.
pub fn conj(preds: &[Py]) -> Py {
    match preds {
        [] => tup!["const", 1],
        [one] => one.clone(),
        many => Py::Tuple(vec![Py::from("and"), Py::List(many.to_vec())]),
    }
}

/// `as_bool(v)`: the value as a condition (`truth` wraps what is not one already).
pub fn as_bool(v: &Py) -> Result<Py> {
    if v.tag_in(&["cmp", "not", "and", "or", "truth"])? {
        return Ok(v.clone());
    }
    if v.tag_is("bool")? {
        return as_bool(v.item(1)?);
    }
    if v.tag_is("boolv")? {
        return Ok(v.item(1)?.clone());
    }
    if v.tag_is("conflict")? {
        return Err(DecodeError::new("a value that differs between joined paths is used"));
    }
    if v.tag_is("arith")? && v.item(1)?.as_str() == Some("-") {
        return Ok(tup!["cmp", "~=", v.item(2)?, v.item(3)?]); // `x - y` tested for zero
    }
    Ok(Py::Tuple(vec![Py::from("truth"), v.clone()]))
}

/// `truth(v)` of `dsl_to_lua.py`: `v` when it already is a condition, else `("truth", v)`.
pub fn truth(v: &Py) -> Result<Py> {
    if v.tag_in(&["cmp", "not", "and", "or", "truth"])? {
        return Ok(v.clone());
    }
    Ok(Py::Tuple(vec![Py::from("truth"), v.clone()]))
}

/// `binop(op, x, y)`: `r-` and `r/` are the reversed subtraction and division.
pub fn binop(op: &str, x: &Py, y: &Py) -> Py {
    match op {
        "r-" => tup!["arith", "-", y, x],
        "r/" => tup!["arith", "/", y, x],
        _ => tup!["arith", op, x, y],
    }
}

/// `fbinop(op, x, y)` of `dsl_to_lua.py`: x87 arithmetic, `("farith", op, x, y)` (floating
/// point, no 32-bit wrap).
pub fn fbinop(op: &str, x: &Py, y: &Py) -> Result<Py> {
    let v = binop(op, x, y);
    let mut out = vec![Py::from("farith")];
    out.extend_from_slice(v.tail()?);
    Ok(Py::Tuple(out))
}

/// `lv_value(lv)` of `dsl_to_lua.py`: `("glob", addr)` for a `gvar` lvalue, else the lvalue.
pub fn lv_value(lv: &Py) -> Result<Py> {
    if lv.tag_is("gvar")? {
        Ok(tup!["glob", lv.item(1)?])
    } else {
        Ok(lv.clone())
    }
}

/// `contains(e, test)` of `dsl_to_lua.py`: whether expression `e` has a node for which `test`
/// holds. A tuple whose head is a string is a node (and `test` is applied to it); the generic
/// walk then goes through `e[1:]`, and through the head itself when it is not a string
/// (a tuple of expressions). Lists are walked element by element. `test` may fail like the
/// Python lambdas do (`e[1]` on a short tuple); the error propagates.
pub fn contains(e: &Py, test: &dyn Fn(&Py) -> Result<bool>) -> Result<bool> {
    match e {
        Py::Tuple(v) => {
            if let Some(Py::Str(_)) = v.first()
                && test(e)?
            {
                return Ok(true);
            }
            for x in v.iter().skip(1) {
                if contains(x, test)? {
                    return Ok(true);
                }
            }
            if let Some(head) = v.first()
                && !matches!(head, Py::Str(_))
            {
                return contains(head, test);
            }
            Ok(false)
        }
        Py::List(v) => {
            for x in v {
                if contains(x, test)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

// ------------------------------------------------------------------ atoms and the truth table

/// `atom_key(e)`: `(key, polarity)`; `x < y` and `x >= y` share a key (the `repr` of the
/// positive form). The key is the Python `repr` string, so `0.0` and `-0.0` stay distinct.
pub fn atom_key(e: &Py) -> Result<(String, bool)> {
    if e.tag_is("cmp")? && matches!(e.item(1)?.as_str(), Some("~=" | ">=" | ">")) {
        let t = tup!["cmp", neg_op(e.item(1)?)?, e.item(2)?, e.item(3)?];
        return Ok((t.repr(), false));
    }
    Ok((e.repr(), true))
}

/// `atoms(e, out)`: appends the atom keys of `e` to `out`, each once, in first-seen order.
pub fn atoms(e: &Py, out: &mut Vec<String>) -> Result<()> {
    let k = e.tag()?;
    match k {
        Some("and" | "or") => {
            for x in e.item(1)?.items()? {
                atoms(x, out)?;
            }
        }
        Some("not") => atoms(e.item(1)?, out)?,
        Some("const") => {}
        _ => {
            let key = atom_key(e)?.0;
            if !out.contains(&key) {
                out.push(key);
            }
        }
    }
    Ok(())
}

/// `evaluate(e, env)`: the truth value of `e` when each atom key has the value `env` gives
/// (`and`/`or` short-circuit like Python's `all`/`any`; a missing key is a `KeyError`).
pub fn evaluate(e: &Py, env: &HashMap<String, bool>) -> Result<bool> {
    let k = e.tag()?;
    match k {
        Some("and") => {
            for x in e.item(1)?.items()? {
                if !evaluate(x, env)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Some("or") => {
            for x in e.item(1)?.items()? {
                if evaluate(x, env)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Some("not") => Ok(!evaluate(e.item(1)?, env)?),
        Some("const") => Ok(e.item(1)?.truthy()),
        _ => {
            let (key, pol) = atom_key(e)?;
            let v = env.get(&key).ok_or_else(|| DecodeError::python("KeyError", Py::from(key.as_str()).repr()))?;
            Ok(*v == pol)
        }
    }
}

/// `truth_table(e, limit)`: `Some(true)` / `Some(false)` when `e` has that value for every
/// assignment of its atoms, `None` when it varies or has more than `limit` atoms (the
/// Python default is 14, see [`truth_table_default`]).
pub fn truth_table(e: &Py, limit: usize) -> Result<Option<bool>> {
    let mut names = Vec::new();
    atoms(e, &mut names)?;
    if names.len() > limit {
        return Ok(None);
    }
    let mut seen: Option<bool> = None;
    for bits in 0u64..(1u64 << names.len()) {
        let env: HashMap<String, bool> =
            names.iter().enumerate().map(|(i, n)| (n.clone(), (bits >> i) & 1 == 1)).collect();
        let v = evaluate(e, &env)?;
        match seen {
            None => seen = Some(v),
            Some(s) if s != v => return Ok(None),
            _ => {}
        }
    }
    Ok(seen)
}

/// `truth_table(e)` with Python's default limit of 14 atoms.
pub fn truth_table_default(e: &Py) -> Result<Option<bool>> {
    truth_table(e, 14)
}

// ------------------------------------------------------------------ simplify

/// `simplify(e)`: constant-fold, flatten, deduplicate, absorb and factor a condition, and
/// replace it by a constant when its truth table over its atoms is constant.
pub fn simplify(e: &Py) -> Result<Py> {
    let k = e.tag()?;
    match k {
        Some("truth") => {
            let v = e.item(1)?;
            if v.tag_is("const")? {
                return Ok(if v.item(1)?.truthy() { true_() } else { false_() });
            }
            if v.tag_is("boolv")? {
                return simplify(v.item(1)?);
            }
            if v.tag_is("conflict")? {
                return Err(DecodeError::new("a value that differs between joined paths is used"));
            }
            Ok(e.clone())
        }
        Some("not") => {
            let x = simplify(e.item(1)?)?;
            if x.tag_is("const")? {
                return Ok(if x.item(1)?.truthy() { false_() } else { true_() });
            }
            negate(&x)
        }
        Some("cmp") => {
            if e.item(2)?.tag_is("boolv")?
                && *e.item(3)? == tup!["const", 0]
                && matches!(e.item(1)?.as_str(), Some("==" | "~="))
            {
                let x = simplify(e.item(2)?.item(1)?)?;
                return if e.item(1)?.as_str() == Some("~=") { Ok(x) } else { simplify(&negate(&x)?) };
            }
            Ok(e.clone())
        }
        Some(kind @ ("and" | "or")) => {
            let (unit, zero) = if kind == "and" { (true_(), false_()) } else { (false_(), true_()) };
            let mut items: Vec<Py> = Vec::new();
            for y in e.item(1)?.items()? {
                let x = simplify(y)?;
                if x == unit {
                    continue;
                }
                if x == zero {
                    return Ok(zero);
                }
                let parts: Vec<Py> = if x.tag_is(kind)? { x.item(1)?.items()?.to_vec() } else { vec![x.clone()] };
                for y in parts {
                    if !items.contains(&y) {
                        items.push(y);
                    }
                }
            }
            if items.is_empty() {
                return Ok(unit);
            }
            if items.len() == 1 {
                return Ok(items.remove(0));
            }
            if kind == "or" {
                items = absorb(&items)?;
                if items.len() == 1 {
                    return Ok(items.remove(0));
                }
            }
            let mut out = Py::Tuple(vec![Py::from(kind), Py::List(items)]);
            if kind == "or" {
                out = factor(&out)?;
            }
            let tt = if out.tag_in(&["and", "or"])? { truth_table_default(&out)? } else { None };
            if let Some(t) = tt {
                return Ok(if t { true_() } else { false_() });
            }
            Ok(out)
        }
        _ => Ok(e.clone()),
    }
}

/// `simplify_neg(x)`: the operand of a `not`, else `negate(x)`.
pub fn simplify_neg(x: &Py) -> Result<Py> {
    if x.tag_is("not")? {
        Ok(x.item(1)?.clone())
    } else {
        negate(x)
    }
}

/// `absorb(items)`: `a or (not a and b)` becomes `a or b` (the shape `||` compiles to).
pub fn absorb(items: &[Py]) -> Result<Vec<Py>> {
    let mut out: Vec<Py> = Vec::new();
    for x in items {
        let mut x = x.clone();
        if x.tag_is("and")? {
            let mut lits = Vec::new();
            for y in x.item(1)?.items()? {
                if !out.contains(&simplify_neg(y)?) {
                    lits.push(y.clone());
                }
            }
            x = if lits.len() == 1 {
                lits.remove(0)
            } else if !lits.is_empty() {
                Py::Tuple(vec![Py::from("and"), Py::List(lits)])
            } else {
                true_()
            };
        } else if out.contains(&simplify_neg(&x)?) {
            x = true_();
        }
        if x == true_() {
            return Ok(vec![true_()]);
        }
        if !out.contains(&x) {
            out.push(x);
        }
    }
    Ok(out)
}

/// `factor(e)`: `(p and a) or (p and b)` becomes `p and (a or b)`: the leading terms the
/// compiler's paths share.
pub fn factor(e: &Py) -> Result<Py> {
    let mut terms: Vec<Vec<Py>> = Vec::new();
    for t in e.item(1)?.items()? {
        terms.push(if t.tag_is("and")? { t.item(1)?.items()?.to_vec() } else { vec![t.clone()] });
    }
    let mut n = 0usize;
    while terms.iter().all(|t| t.len() > n + 1) && terms.iter().all(|t| t[n] == terms[0][n]) {
        n += 1;
    }
    if n == 0 {
        return Ok(e.clone());
    }
    let rests: Vec<Py> = terms.iter().map(|t| conj(&t[n..])).collect();
    let rest = simplify(&Py::Tuple(vec![Py::from("or"), Py::List(rests)]))?;
    let mut lead: Vec<Py> = terms[0][..n].to_vec();
    lead.push(rest);
    simplify(&Py::Tuple(vec![Py::from("and"), Py::List(lead)]))
}

// ------------------------------------------------------------------ call arguments, branches

/// `expr_arg(v, a)`: a pushed value as a call argument (`a` is the instruction's address, for the
/// message): the address of an `int` or `int[]` global becomes `("obj", name)` / `("array",
/// name)`, expression nodes pass through, anything else is a `DecodeError`.
pub fn expr_arg(v: &Py, a: i64) -> Result<Py> {
    if v.is_none() {
        return Err(DecodeError::new(format!("unknown call argument at {}", py_hex(a))));
    }
    if v.tag_is("iat")? {
        // the address of a .dsl global, passed by reference (`int &`)
        let sym = str_of(v.item(1)?)?;
        let k = import_kind(sym);
        if k == "int" {
            return Ok(tup!["obj", plain_name(sym)]);
        }
        if k == "intarray" {
            return Ok(tup!["array", plain_name(sym)]);
        }
        return Err(DecodeError::new(format!(
            "reference to {k} {} passed at {}",
            plain_name(sym),
            py_hex(a)
        )));
    }
    if v.tag_in(&[
        "obj", "objf", "elem", "const", "float", "str", "var", "call", "arith", "cmp", "not", "bool", "truth",
    ])? {
        return Ok(v.clone());
    }
    Err(DecodeError::new(format!(
        "unsupported call argument {} at {}",
        v.item(0)?.to_str(),
        py_hex(a)
    )))
}

/// `TypeError` of `x & 0x...` for a value that is not an int.
fn and_type_error(v: &Py) -> DecodeError {
    DecodeError::python(
        "TypeError",
        format!("unsupported operand type(s) for &: '{}' and 'int'", v.type_name()),
    )
}

fn str_of(v: &Py) -> Result<&str> {
    v.as_str()
        .ok_or_else(|| {
            DecodeError::python(
                "TypeError",
                format!("expected string or bytes-like object, got '{}'", v.type_name()),
            )
        })
}

/// `typed_args(args, params)`: a pushed 32-bit constant in a `float` parameter (`M`) is the
/// float's bits, widened to a double; a `bool` parameter (`_N`) wraps the value in `boolarg`
/// (the pushed word is an int, the engine bindings take strict booleans). `params` of `None` or of
/// another length leaves the arguments as they are.
pub fn typed_args(args: &[Py], params: Option<&[String]>) -> Result<Vec<Py>> {
    let Some(params) = params else {
        return Ok(args.to_vec());
    };
    if params.len() != args.len() {
        return Ok(args.to_vec());
    }
    let mut out = Vec::with_capacity(args.len());
    for (v, t) in args.iter().zip(params) {
        let mut v = v.clone();
        if t == "M" && v.tag_is("const")? {
            let n = match v.item(1)? {
                Py::Int(n) => *n,
                Py::Bool(b) => i64::from(*b),
                other => return Err(and_type_error(other)),
            };
            v = tup!["float", f64::from(f32::from_bits((n & 0xFFFF_FFFF) as u32))];
        } else if t == "_N" && !v.tag_is("boolarg")? {
            v = Py::Tuple(vec![Py::from("boolarg"), v]);
        }
        out.push(v);
    }
    Ok(out)
}

/// `branch_pred(mn, f, a)`: the predicate under which the conditional jump `mn` is taken, given
/// the flags `f` the last compare left (`("test", v)`, `("cmp", x, y)` or `("ftest", (st0, m),
/// mask)`); `a` is the instruction's address, for messages.
pub fn branch_pred(mn: &str, f: &Py, a: i64) -> Result<Py> {
    let at = || py_hex(a);
    if f.tag_is("test")? {
        let v = f.item(1)?;
        if v.is_none() {
            return Err(DecodeError::new(format!("test of an unknown value at {}", at())));
        }
        if mn == "je" {
            return negate(&as_bool(v)?);
        }
        if mn == "jne" {
            return as_bool(v);
        }
        if matches!(mn, "jl" | "jge" | "jle" | "jg") {
            return Ok(tup!["cmp", jcc_or_key_error(mn)?, v, tup!["const", 0]]);
        }
    }
    if f.tag_is("cmp")? {
        let (x, y) = (f.item(1)?, f.item(2)?);
        if x.is_none() || y.is_none() {
            return Err(DecodeError::new(format!("compare of an unknown value at {}", at())));
        }
        return Ok(tup!["cmp", jcc_or_key_error(mn)?, x, y]);
    }
    if f.tag_is("ftest")? {
        // `(st0, m), mask = f[1], f[2]`: the right side is read first, then unpacked
        let (packed, mask_v) = (f.item(1)?, f.item(2)?);
        let pair = packed.as_seq().ok_or_else(|| {
            DecodeError::python("TypeError", format!("cannot unpack non-iterable {} object", packed.type_name()))
        })?;
        if pair.len() != 2 {
            return Err(DecodeError::python(
                "ValueError",
                format!(
                    "{} values to unpack (expected 2, got {})",
                    if pair.len() < 2 { "not enough" } else { "too many" },
                    pair.len()
                ),
            ));
        }
        let (st0, m) = (&pair[0], &pair[1]);
        let mask = match mask_v {
            Py::Int(n) => *n,
            Py::Bool(b) => i64::from(*b),
            other => return Err(and_type_error(other)),
        };
        if mask & !0x45 != 0 {
            return Err(DecodeError::new(format!("x87 status mask {} at {}", py_hex(mask), at())));
        }
        let mut bits: Vec<Py> = Vec::new();
        if mask & 0x01 != 0 {
            bits.push(tup!["cmp", "<", st0, m]);
        }
        if mask & 0x40 != 0 {
            bits.push(tup!["cmp", "==", st0, m]);
        }
        if bits.is_empty() {
            return Err(DecodeError::new(format!("x87 status mask {} at {}", py_hex(mask), at())));
        }
        let set = if bits.len() == 2 { tup!["cmp", "<=", st0, m] } else { bits.remove(0) };
        if mn == "je" {
            return negate(&set);
        }
        if mn == "jne" {
            return Ok(set);
        }
    }
    Err(DecodeError::new(format!(
        "unsupported branch `{mn}` after {} at {}",
        f.item(0)?.to_str(),
        at()
    )))
}
