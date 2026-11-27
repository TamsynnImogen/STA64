//! Tests of the shared core (LUA-15c): the value tree, the Python-exact formatting, the MSVC
//! name helpers, the boolean algebra, the Lua text constants.
//!
//! Almost everything compares with the ORIGINAL Python functions' results: the tables in
//! `core_data/tables.rs` were produced by running `tools/{drl,dsl,a1}_to_lua.py` on inputs
//! invented for the purpose (see that file's header); the tests here read each input (the Python
//! `repr` of the argument tuple), run the Rust function and compare the `repr` of its result with
//! Python's. Where `python3` is on PATH, two more tests run the Python tools live (float
//! formatting over tens of thousands of generated values, the prelude constants) and skip with a
//! message otherwise. No game file is read.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use indexmap::IndexMap;
use sta_mission_convert::algebra::*;
use sta_mission_convert::format::*;
use sta_mission_convert::msvc::*;
use sta_mission_convert::prelude::*;
use sta_mission_convert::pyval::*;
use sta_mission_convert::{list, tup, DecodeError, ErrorKind, Result};

#[path = "core_data/tables.rs"]
mod tables;

// ------------------------------------------------------------------ literal parser

/// Parses the Python `repr` of a value made of tuples, lists, strings, numbers, `None`, `True`,
/// `False`, `nan`, `inf`.
fn parse(text: &str) -> Py {
    let chars: Vec<char> = text.chars().collect();
    let mut p = 0usize;
    let v = parse_value(&chars, &mut p);
    skip_ws(&chars, &mut p);
    assert_eq!(p, chars.len(), "trailing text in literal {text:?}");
    v
}

fn skip_ws(c: &[char], p: &mut usize) {
    while *p < c.len() && c[*p].is_whitespace() {
        *p += 1;
    }
}

fn parse_value(c: &[char], p: &mut usize) -> Py {
    skip_ws(c, p);
    match c[*p] {
        '(' | '[' => {
            let close = if c[*p] == '(' { ')' } else { ']' };
            let is_tuple = close == ')';
            *p += 1;
            let mut items = Vec::new();
            loop {
                skip_ws(c, p);
                if c[*p] == close {
                    *p += 1;
                    break;
                }
                items.push(parse_value(c, p));
                skip_ws(c, p);
                if c[*p] == ',' {
                    *p += 1;
                }
            }
            if is_tuple {
                Py::Tuple(items)
            } else {
                Py::List(items)
            }
        }
        q @ ('\'' | '"') => {
            *p += 1;
            let mut s = String::new();
            while c[*p] != q {
                if c[*p] == '\\' {
                    *p += 1;
                    match c[*p] {
                        '\\' => s.push('\\'),
                        '\'' => s.push('\''),
                        '"' => s.push('"'),
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        'x' => {
                            let h: String = c[*p + 1..*p + 3].iter().collect();
                            s.push(char::from_u32(u32::from_str_radix(&h, 16).unwrap()).unwrap());
                            *p += 2;
                        }
                        'u' => {
                            let h: String = c[*p + 1..*p + 5].iter().collect();
                            s.push(char::from_u32(u32::from_str_radix(&h, 16).unwrap()).unwrap());
                            *p += 4;
                        }
                        'U' => {
                            let h: String = c[*p + 1..*p + 9].iter().collect();
                            s.push(char::from_u32(u32::from_str_radix(&h, 16).unwrap()).unwrap());
                            *p += 8;
                        }
                        other => panic!("unknown escape \\{other}"),
                    }
                } else {
                    s.push(c[*p]);
                }
                *p += 1;
            }
            *p += 1;
            Py::Str(s)
        }
        ch if ch.is_ascii_alphabetic() || ch == '-' && c.get(*p + 1) == Some(&'i') => {
            let neg = ch == '-';
            if neg {
                *p += 1;
            }
            let start = *p;
            while *p < c.len() && c[*p].is_ascii_alphabetic() {
                *p += 1;
            }
            let w: String = c[start..*p].iter().collect();
            match (w.as_str(), neg) {
                ("None", false) => Py::None,
                ("True", false) => Py::Bool(true),
                ("False", false) => Py::Bool(false),
                ("nan", false) => Py::Float(f64::NAN),
                ("inf", false) => Py::Float(f64::INFINITY),
                ("inf", true) => Py::Float(f64::NEG_INFINITY),
                _ => panic!("unknown word {w:?}"),
            }
        }
        _ => {
            let start = *p;
            while *p < c.len() && (c[*p].is_ascii_digit() || matches!(c[*p], '-' | '+' | '.' | 'e' | 'E')) {
                *p += 1;
            }
            let t: String = c[start..*p].iter().collect();
            if t.contains(['.', 'e', 'E']) {
                Py::Float(t.parse().unwrap_or_else(|_| panic!("bad float {t:?}")))
            } else {
                Py::Int(t.parse().unwrap_or_else(|_| panic!("bad int {t:?}")))
            }
        }
    }
}

fn show(r: Result<Py>) -> String {
    match r {
        Ok(p) => p.repr(),
        Err(e) if e.kind == ErrorKind::Decode => format!("!DecodeError:{}", e.message),
        Err(_) => "!raw".to_owned(),
    }
}

/// Like [`show`], but a Python exception carries its class and text (`!raw:<class>:<text>`).
fn show_text(r: Result<Py>) -> String {
    match r {
        Ok(p) => p.repr(),
        Err(e) if e.kind == ErrorKind::Decode => format!("!DecodeError:{}", e.message),
        Err(e) => format!("!raw:{}:{}", e.class.unwrap_or("?"), e.message),
    }
}

fn s(x: &str) -> Py {
    Py::Str(x.to_owned())
}

fn strs(v: &[String]) -> Py {
    Py::List(v.iter().map(|x| s(x)).collect())
}

/// Runs a table: `f` gets the parsed argument tuple's items and returns the text to compare with
/// the expected string. Returns the number of pairs checked.
fn check(name: &str, table: &[(&str, &str)], f: impl Fn(&[Py]) -> String) -> usize {
    let mut bad = Vec::new();
    for (inp, exp) in table {
        let args = parse(inp);
        let got = f(args.items().unwrap());
        if got != *exp {
            bad.push(format!("  input {inp}\n    expected {exp}\n    got      {got}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{name}: {} of {} pairs differ from Python:\n{}",
        bad.len(),
        table.len(),
        bad.iter().take(8).cloned().collect::<Vec<_>>().join("\n")
    );
    table.len()
}

fn h(p: &Py) -> u64 {
    let mut s = DefaultHasher::new();
    p.hash(&mut s);
    s.finish()
}

// ------------------------------------------------------------------ pyval

#[test]
fn python_equality_and_ordering() {
    let n = check("PY_EQ_LT", tables::PY_EQ_LT, |a| {
        let eq = a[0] == a[1];
        if eq {
            assert_eq!(h(&a[0]), h(&a[1]), "equal values hash alike: {:?} {:?}", a[0], a[1]);
        }
        let lt = match a[0].py_lt(&a[1]) {
            Ok(b) => Py::Bool(b),
            Err(e) => {
                assert_eq!(e.kind, ErrorKind::Raw);
                s("!raw")
            }
        };
        Py::Tuple(vec![Py::Bool(eq), lt]).repr()
    });
    assert_eq!(n, tables::PY_EQ_LT.len());
}

#[test]
fn python_equality_edge_cases_without_a_python_counterpart() {
    // a NaN equals itself here (Python's identity shortcut; the tools only copy NaNs around)
    assert_eq!(Py::Float(f64::NAN), Py::Float(f64::NAN));
    assert_ne!(Py::Float(f64::NAN), Py::Float(1.0));
    assert!(!Py::Float(f64::NAN).py_lt(&Py::Float(1.0)).unwrap());
    assert!(!Py::Float(1.0).py_lt(&Py::Float(f64::NAN)).unwrap());
    // exact comparison of an int and a float beyond 2**53
    assert_ne!(Py::Int(9_007_199_254_740_993), Py::Float(9_007_199_254_740_992.0));
    assert!(Py::Float(9_007_199_254_740_992.0).py_lt(&Py::Int(9_007_199_254_740_993)).unwrap());
    assert!(Py::Int(i64::MAX).py_lt(&Py::Float(9.3e18)).unwrap());
    assert_eq!(Py::Int(1), Py::Bool(true));
    assert_eq!(h(&Py::Float(-0.0)), h(&Py::Int(0)));
}

#[test]
fn python_repr_str_and_truthiness() {
    check("PY_REPR", tables::PY_REPR, |a| {
        Py::Tuple(vec![s(&a[0].repr()), s(&a[0].to_str()), Py::Bool(a[0].truthy())]).repr()
    });
}

#[test]
fn python_sorted_is_stable_and_raises_like_python() {
    check("PY_SORT", tables::PY_SORT, |a| {
        show(py_sorted(a[0].items().unwrap().to_vec()).map(Py::List))
    });
    // key sort keeps the order of equal keys
    let v = py_sort_by_key(vec![(1, "b"), (0, "z"), (1, "a"), (0, "y")], |x| Py::Int(x.0)).unwrap();
    assert_eq!(v, vec![(0, "z"), (0, "y"), (1, "b"), (1, "a")]);
}

#[test]
fn python_integer_division_and_modulo() {
    check("INT_DIV", tables::INT_DIV, |a| {
        let (x, y) = (a[0].as_int().unwrap(), a[1].as_int().unwrap());
        show(py_floordiv(x, y).and_then(|q| py_mod(x, y).map(|r| tup![q, r])))
    });
    assert!(py_floordiv(i64::MIN, -1).is_err());
    assert_eq!(py_mod(i64::MIN, -1).unwrap(), 0);
}

#[test]
fn python_int_from_text() {
    check("INT_PARSE", tables::INT_PARSE, |a| {
        show(py_int(a[0].as_str().unwrap(), a[1].as_int().unwrap() as u32).map(Py::Int))
    });
}

#[test]
fn python_string_methods() {
    check("STRMETH", tables::STRMETH, |a| {
        let t = a[0].as_str().unwrap();
        let split: Vec<Py> = py_split_ws_once(t).into_iter().map(s).collect();
        let digits: Vec<Py> = t.chars().map(|c| Py::Bool(py_isdigit(c))).collect();
        let spaces: Vec<Py> = t.chars().map(|c| Py::Bool(py_isspace(c))).collect();
        Py::Tuple(vec![s(py_strip(t)), Py::List(split), s(&py_lower(t)), s(&py_upper(t)), Py::List(digits), Py::List(spaces)])
            .repr()
    });
    check("SLICE", tables::SLICE, |a| {
        let chars: Vec<char> = a[0].as_str().unwrap().chars().collect();
        s(&py_slice(&chars, a[1].as_int().unwrap() as usize, a[2].as_int().unwrap() as usize)).repr()
    });
}

#[test]
fn python_dict_order() {
    check("DICT_OPS", tables::DICT_OPS, |a| {
        let mut d: IndexMap<Py, Py> = IndexMap::new();
        for op in a[0].items().unwrap() {
            let op = op.items().unwrap();
            let name = op[0].as_str().unwrap();
            let k = op[1].clone();
            match name {
                "set" => {
                    d.insert(k, op[2].clone());
                }
                "del" => {
                    if d.py_del(&k).is_err() {
                        return "!raw".to_owned();
                    }
                }
                "setdefault" => {
                    d.py_setdefault(k, op[2].clone());
                }
                "pop" => {
                    d.py_pop(&k);
                }
                other => panic!("{other}"),
            }
        }
        Py::List(d.into_iter().map(|(k, v)| Py::Tuple(vec![k, v])).collect()).repr()
    });
    check("FIRST_MAX", tables::FIRST_MAX, |a| {
        let mut d: IndexMap<String, i64> = IndexMap::new();
        for p in a[0].items().unwrap() {
            let p = p.items().unwrap();
            d.insert(p[0].as_str().unwrap().to_owned(), p[1].as_int().unwrap());
        }
        match first_max_key(d.iter()) {
            Some(k) => s(k).repr(),
            None => "!raw".to_owned(),
        }
    });
}

// ------------------------------------------------------------------ format

#[test]
fn float_repr_and_lua_num_match_python() {
    for (bits, repr, lua) in tables::FLOAT_BITS {
        let x = f64::from_bits(*bits);
        assert_eq!(py_float_repr(x), *repr, "repr of bits {bits:#018x}");
        assert_eq!(lua_num_f64(x), *lua, "lua_num of bits {bits:#018x}");
    }
}

#[test]
fn float_text_for_the_awkward_cases() {
    for (x, repr, lua) in [
        (0.0, "0.0", "0.0"),
        (-0.0, "-0.0", "0.0"),
        (1e16, "1e+16", "1e+16"),
        (1e15, "1000000000000000.0", "1000000000000000.0"),
        (999_999_999_999_999.0, "999999999999999.0", "999999999999999.0"),
        (1.5e-5, "1.5e-05", "1.5e-05"),
        (0.0001, "0.0001", "0.0001"),
        (f64::INFINITY, "inf", "math.huge"),
        (f64::NEG_INFINITY, "-inf", "(-math.huge)"),
        (f64::NAN, "nan", "(0/0)"),
        (5e-324, "5e-324", "5e-324"),
        (-123.0, "-123.0", "-123.0"),
    ] {
        assert_eq!(py_float_repr(x), repr);
        assert_eq!(lua_num_f64(x), lua);
    }
}

#[test]
fn lua_num_of_other_values() {
    check("LUA_NUM_OTHER", tables::LUA_NUM_OTHER, |a| s(&lua_num(&a[0])).repr());
}

#[test]
fn hex_with_a_sign() {
    check("HEX", tables::HEX, |a| {
        let n = a[0].as_int().unwrap();
        Py::Tuple(vec![s(&py_hex(n)), s(&py_hex_plain(n))]).repr()
    });
}

#[test]
fn string_repr_and_lua_string() {
    check("STR", tables::STR, |a| {
        let t = a[0].as_str().unwrap();
        Py::Tuple(vec![s(&py_str_repr(t)), s(&lua_str(t)), Py::Int(t.chars().count() as i64)]).repr()
    });
}

#[test]
fn lua_identifiers() {
    check("LUA_IDENT", tables::LUA_IDENT, |a| {
        let t = a[0].as_str().unwrap();
        Py::Tuple(vec![lua_ident(t).map_or(Py::None, s), s(&a1_lua_ident(t))]).repr()
    });
}

fn ctx_of(p: &Py) -> Ctx {
    match p.as_str().unwrap() {
        "bool" => Ctx::Bool,
        "num" => Ctx::Num,
        "arg" => Ctx::Arg,
        other => panic!("{other}"),
    }
}

#[test]
fn rendering_conditions() {
    check("RENDER", tables::RENDER, |a| show(render(&a[0], ctx_of(&a[1])).map(Py::Str)));
    check("PAREN", tables::PAREN, |a| show(paren(&a[0]).map(Py::Str)));
}

// ------------------------------------------------------------------ msvc

fn opt_params(p: Option<Vec<String>>) -> Py {
    p.map_or(Py::None, |v| strs(&v))
}

#[test]
fn msvc_names_signatures_and_kinds() {
    check("MSVC", tables::MSVC, |a| {
        let sym = a[0].as_str().unwrap();
        let ret = match msvc_ret(sym) {
            Ok(r) => r.map_or(Py::None, |x| s(&x)),
            Err(e) => {
                assert_eq!(e.kind, ErrorKind::Raw);
                s("!raw")
            }
        };
        let sig = signature(sym).map_or(Py::None, |g| {
            Py::Tuple(vec![s(g.convention), g.cls.as_deref().map_or(Py::None, s), s(&g.method), s(&g.ret), strs(&g.params)])
        });
        Py::Tuple(vec![
            s(&plain_name(sym)),
            s(import_kind(sym)),
            s(data_kind(sym)),
            opt_params(msvc_params(sym)),
            ret,
            sig,
        ])
        .repr()
    });
}

#[test]
fn msvc_type_code_reader() {
    check("PARSE_TYPES", tables::PARSE_TYPES, |a| match parse_types(a[0].as_str().unwrap()) {
        Ok(v) => strs(&v).repr(),
        Err(ParseFault::Value) => "!Value".to_owned(),
        Err(ParseFault::Index) => "!Index".to_owned(),
    });
    check("PUSH_SLOTS", tables::PUSH_SLOTS, |a| match a[0].as_str().unwrap() {
        "push" => Py::Int(push_count(a[1].as_str().unwrap())).repr(),
        "slots" => {
            let params: Vec<String> = a[1].items().unwrap().iter().map(|x| x.as_str().unwrap().to_owned()).collect();
            Py::Int(slots_of(&params, a[2].as_str().unwrap())).repr()
        }
        other => panic!("{other}"),
    });
}

/// The helper tests of `tools/test_drl_to_lua.py`, `test_dsl_to_lua.py` and `test_a1_to_lua.py`,
/// ported.
#[test]
fn ported_python_helper_tests() {
    // HelperTests.test_msvc_params
    let v = |x: &[&str]| Some(x.iter().map(|y| y.to_string()).collect::<Vec<_>>());
    assert_eq!(msvc_params("?UnderAttack@ScriptInterfaceImp@@UAE_NHM@Z"), v(&["H", "M"]));
    assert_eq!(msvc_params("?GetDistance@ScriptInterfaceImp@@UAEMAAH0@Z"), v(&["AH", "AH"]));
    assert_eq!(msvc_params("?GetTime@ScriptInterfaceImp@@UAEMXZ"), v(&[]));
    // HelperTests.test_float_arguments_are_reinterpreted
    let bits = 9.0f32.to_bits() as i64;
    let args = [tup!["obj", "x"], tup!["const", bits]];
    let params = ["H".to_string(), "M".to_string()];
    assert_eq!(typed_args(&args, Some(&params)).unwrap(), vec![tup!["obj", "x"], tup!["float", 9.0]]);
    // HelperTests.test_import_kinds
    assert_eq!(import_kind("?p_A@@3PAVtimer_int@@A"), "timer_int");
    assert_eq!(import_kind("?Enterprise@@3HA"), "int");
    assert_eq!(import_kind("?Mission_Win@@YAXXZ"), "action");
    // test_dsl_to_lua: return_types, lua_num of infinity
    assert_eq!(msvc_ret("?IsAlive@ScriptInterfaceImp@@UAE_NH@Z").unwrap().as_deref(), Some("_N"));
    assert_eq!(msvc_ret("?MoveTo@ScriptInterfaceImp@@UAEXHPAD@Z").unwrap().as_deref(), Some("X"));
    assert_eq!(
        msvc_ret("?ScriptCreate_hot_int@ScriptInterfaceImp@@UAEPAVhot_int@@PAD@Z").unwrap().as_deref(),
        Some("PAVhot_int@@")
    );
    assert_eq!(push_count("VVector3@@"), 3);
    assert_eq!(lua_num_f64(f64::INFINITY), "math.huge");
    // test_a1_to_lua.Signatures
    let sg = |x: &str| signature(x).unwrap();
    let g = sg("?GetDistance@@YAMAAH0@Z");
    assert_eq!((g.convention, g.cls, g.method.as_str(), g.ret.as_str()), ("cdecl", None, "GetDistance", "M"));
    assert_eq!(g.params, vec!["AAH", "AAH"]);
    let g = sg("??4hot_int@@QAEXH@Z");
    assert_eq!((g.convention, g.cls.as_deref(), g.method.as_str(), g.ret.as_str()), ("thiscall", Some("hot_int"), "op4", "X"));
    assert_eq!(g.params, vec!["H"]);
    let g = sg("??Bhot_int@@QAEHXZ");
    assert_eq!((g.method.as_str(), g.ret.as_str(), g.params.len()), ("opB", "H", 0));
    let g = sg("?create_rule@IIIE@@QAEPAVrule@@P6AHXZP6AXXZPAD@Z");
    assert_eq!((g.convention, g.cls.as_deref(), g.ret.as_str(), g.params.len()), ("thiscall", Some("IIIE"), "PAVrule@@", 3));
    assert_eq!(sg("?GetAttackLocation@@YA?AVVector3@@HH@Z").ret, "?AVVector3@@");
    assert!(signature("?ship@@3HA").is_none());
    assert_eq!(sg("?f@@YAPAVa@@PAVb@@0@Z").params, vec!["PAVb@@", "PAVb@@"]);
    let p = |x: &[&str]| x.iter().map(|y| y.to_string()).collect::<Vec<_>>();
    assert_eq!(slots_of(&p(&["H", "H"]), "?AVVector3@@"), 3);
    assert_eq!(slots_of(&p(&["H", "VVector3@@"]), "X"), 4);
}

// ------------------------------------------------------------------ algebra

fn one(a: &[Py], f: impl Fn(&Py) -> Result<Py>) -> String {
    show(f(&a[0]))
}

#[test]
fn simplifier_and_its_parts() {
    check("SIMPLIFY", tables::SIMPLIFY, |a| one(a, simplify));
    check("NEGATE", tables::NEGATE, |a| one(a, negate));
    check("AS_BOOL", tables::AS_BOOL, |a| one(a, as_bool));
    check("TRUTH", tables::TRUTH, |a| one(a, truth));
    check("SIMPLIFY_NEG", tables::SIMPLIFY_NEG, |a| one(a, simplify_neg));
    check("FACTOR", tables::FACTOR, |a| one(a, factor));
    check("ABSORB", tables::ABSORB, |a| {
        show(a[0].items().and_then(absorb).map(Py::List))
    });
    check("CONJ", tables::CONJ, |a| match a[0].items() {
        Ok(items) => conj(items).repr(),
        Err(_) => "!raw".to_owned(),
    });
}

#[test]
fn atoms_keys_and_truth_tables() {
    check("ATOM_KEY", tables::ATOM_KEY, |a| {
        show(atom_key(&a[0]).map(|(k, pol)| Py::Tuple(vec![s(&k), Py::Bool(pol)])))
    });
    check("ATOMS", tables::ATOMS, |a| {
        let mut out = Vec::new();
        show(atoms(&a[0], &mut out).map(|()| strs(&out)))
    });
    let tt = |r: Result<Option<bool>>| show(r.map(|o| o.map_or(Py::None, Py::Bool)));
    check("TRUTH_TABLE", tables::TRUTH_TABLE, |a| tt(truth_table_default(&a[0])));
    check("TRUTH_TABLE_LIMIT", tables::TRUTH_TABLE_LIMIT, |a| {
        tt(truth_table(&a[0], a[1].as_int().unwrap() as usize))
    });
    check("EVALUATE", tables::EVALUATE, |a| {
        let env: HashMap<String, bool> = a[1]
            .items()
            .unwrap()
            .iter()
            .map(|p| (p.item(0).unwrap().as_str().unwrap().to_owned(), matches!(p.item(1).unwrap(), Py::Bool(true))))
            .collect();
        show(evaluate(&a[0], &env).map(Py::Bool))
    });
}

#[test]
fn expression_helpers() {
    check("BINOP", tables::BINOP, |a| binop(a[0].as_str().unwrap(), &a[1], &a[2]).repr());
    check("FBINOP", tables::FBINOP, |a| show(fbinop(a[0].as_str().unwrap(), &a[1], &a[2])));
    check("LV_VALUE", tables::LV_VALUE, |a| one(a, lv_value));
    check("EXPR_ARG", tables::EXPR_ARG, |a| show(expr_arg(&a[0], a[1].as_int().unwrap())));
    check("TYPED_ARGS", tables::TYPED_ARGS, |a| {
        let params: Option<Vec<String>> = match &a[1] {
            Py::None => None,
            p => Some(p.items().unwrap().iter().map(|x| x.as_str().unwrap().to_owned()).collect()),
        };
        show(typed_args(a[0].items().unwrap(), params.as_deref()).map(Py::List))
    });
    check("BRANCH_PRED", tables::BRANCH_PRED, |a| {
        show(branch_pred(a[0].as_str().unwrap(), &a[1], a[2].as_int().unwrap()))
    });
}

#[test]
fn contains_walks_like_python() {
    check("CONTAINS", tables::CONTAINS, |a| {
        type Test = Box<dyn Fn(&Py) -> Result<bool>>;
        let test: Test = match a[1].as_str().unwrap() {
            "head_var" => Box::new(|e| e.tag_is("var")),
            "obj_x" => Box::new(|e| Ok(e.tag_is("obj")? && *e.item(1)? == s("x"))),
            "len3" => Box::new(|e| Ok(e.py_len()? == 3)),
            "second_is_1" => Box::new(|e| Ok(*e.item(1)? == Py::Int(1))),
            "never" => Box::new(|_| Ok(false)),
            "always" => Box::new(|_| Ok(true)),
            other => panic!("{other}"),
        };
        show(contains(&a[0], &*test).map(Py::Bool))
    });
}

#[test]
fn ported_condition_shapes() {
    // `a or (not a and b)` is what `||` compiles to; absorb leaves `a or b`
    let a = tup!["cmp", "<", tup!["var", "a", "hot_int"], tup!["const", 1]];
    let b = tup!["truth", tup!["obj", "b"]];
    let e = tup!["or", list![a.clone(), tup!["and", list![negate(&a).unwrap(), b.clone()]]]];
    assert_eq!(simplify(&e).unwrap(), tup!["or", list![a.clone(), b.clone()]]);
    // a constant over its atoms folds to a constant
    let t = tup!["or", list![a.clone(), negate(&a).unwrap()]];
    assert_eq!(simplify(&t).unwrap(), true_());
    assert_eq!(truth_table_default(&tup!["and", list![a.clone(), negate(&a).unwrap()]]).unwrap(), Some(false));
}

// ------------------------------------------------------------------ prelude

fn fnv(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[test]
fn prelude_constants_match_the_python_text() {
    let a1p = a1_prelude().unwrap();
    for (name, len, lines, hash) in tables::PRELUDE_DIGESTS {
        let text: &str = match *name {
            "PRELUDE" => PRELUDE,
            "ENGINE" => ENGINE,
            "A1_PRELUDE" => A1_PRELUDE,
            "a1_prelude" => &a1p,
            other => panic!("{other}"),
        };
        assert_eq!(text.len(), *len, "{name} length");
        assert_eq!(text.matches('\n').count(), *lines, "{name} lines");
        assert_eq!(fnv(text), *hash, "{name} hash");
    }
}

#[test]
fn the_a1_replacement_target_is_still_in_the_shared_prelude() {
    for t in a1_replacement_targets() {
        assert!(PRELUDE.contains(t), "`{t}` is not in PRELUDE any more");
    }
    let a1p = a1_prelude().unwrap();
    assert!(a1p.contains("local f = A1 and A1[name]"));
    assert!(!a1p.contains("local f = A2 and A2[name]"));
    assert!(PRELUDE.starts_with('\n') && ENGINE.starts_with('\n') && A1_PRELUDE.starts_with('\n'));
}

#[test]
fn a1_prelude_fails_like_python_when_the_target_is_gone() {
    // the same text the Python raises; exercised through the public error type only
    let e = DecodeError::new("tools/dsl_to_lua.py prelude changed: `local f = A2 and A2[name]` not found");
    assert_eq!(e.kind, ErrorKind::Decode);
}

// ------------------------------------------------------------------ live comparison with Python

fn python3() -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("python3").arg("--version").output().ok()?;
    out.status.success().then(|| std::path::PathBuf::from("python3"))
}

fn tools_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools")
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Python's own `repr` and `lua_num` over a deterministic stream of floats: random bit patterns,
/// decimal numbers of random digits and exponents, integers, and f32 values widened.
#[test]
fn float_text_matches_python_live() {
    let Some(py) = python3() else {
        eprintln!("skipped: python3 not found (the committed FLOAT_BITS table covers the same functions)");
        return;
    };
    let mut st = 0x5eed_u64;
    let mut vals: Vec<u64> = Vec::new();
    for _ in 0..12_000 {
        vals.push(splitmix(&mut st));
    }
    for _ in 0..12_000 {
        let m = splitmix(&mut st) % 100_000_000_000_000_000;
        let e = (splitmix(&mut st) % 640) as i32 - 330;
        let x: f64 = format!("{m}e{e}").parse().unwrap();
        vals.push(x.to_bits());
        vals.push((-x).to_bits());
    }
    for _ in 0..4_000 {
        let n = (splitmix(&mut st) % 4_000_000_000_000_000) as f64;
        vals.push(n.to_bits());
        vals.push(f64::from(f32::from_bits(splitmix(&mut st) as u32)).to_bits());
    }
    // every exact power of two of the double range (2098) and its negative: the rounding interval
    // is asymmetric there, so the shortest digits can differ from the nearest decimal
    for k in -1074i32..=1023 {
        let bits = if k >= -1022 { ((k + 1023) as u64) << 52 } else { 1u64 << (k + 1074) };
        vals.push(bits);
        vals.push(bits | (1 << 63));
    }
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("float_bits.txt");
    std::fs::write(&input, vals.iter().map(|b| format!("{b:x}\n")).collect::<String>()).unwrap();
    let script = "import sys, struct\nsys.path.insert(0, sys.argv[1])\nimport drl_to_lua as drl\n\
for line in open(sys.argv[2]):\n    x = struct.unpack('<d', struct.pack('<Q', int(line, 16)))[0]\n    print(repr(x) + '|' + drl.lua_num(x))\n";
    let out = std::process::Command::new(py)
        .args(["-I", "-c", script])
        .arg(tools_dir())
        .arg(&input)
        .output()
        .expect("run python3");
    assert!(out.status.success(), "python failed: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), vals.len());
    let mut bad = 0;
    for (b, line) in vals.iter().zip(lines) {
        let x = f64::from_bits(*b);
        let (r, l) = line.split_once('|').unwrap();
        if py_float_repr(x) != r || lua_num_f64(x) != l {
            bad += 1;
            if bad <= 5 {
                eprintln!("bits {b:#018x}: python {line}, rust {} | {}", py_float_repr(x), lua_num_f64(x));
            }
        }
    }
    assert_eq!(bad, 0, "{bad} of {} floats differ from Python", vals.len());
}

#[test]
fn prelude_constants_match_python_live() {
    let Some(py) = python3() else {
        eprintln!("skipped: python3 not found (the committed digests cover the same constants)");
        return;
    };
    let script = "import sys\nsys.path.insert(0, sys.argv[1])\nimport dsl_to_lua as dsl\nimport a1_to_lua as a1\n\
sys.stdout.buffer.write('\\0'.join([dsl.PRELUDE, dsl.ENGINE, a1.A1_PRELUDE, a1.a1_prelude()]).encode('utf-8'))\n";
    let out = std::process::Command::new(py).args(["-I", "-c", script]).arg(tools_dir()).output().expect("run python3");
    assert!(out.status.success(), "python failed: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    let parts: Vec<&str> = text.split('\0').collect();
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0], PRELUDE);
    assert_eq!(parts[1], ENGINE);
    assert_eq!(parts[2], A1_PRELUDE);
    assert_eq!(parts[3], a1_prelude().unwrap());
}

// ------------------------------------------------------------------ review follow-up

#[test]
fn whole_number_floats_near_two_to_the_63_hash_like_their_int() {
    for (x, n) in [(9.2e18, 9_200_000_000_000_000_000i64), (4_611_686_018_427_387_904.0, 1 << 62), (-9_223_372_036_854_775_808.0, i64::MIN)] {
        assert_eq!(Py::Float(x), Py::Int(n));
        assert_eq!(h(&Py::Float(x)), h(&Py::Int(n)), "{x}");
    }
    // 2**63 is no i64: unequal to i64::MAX, and still equal to itself
    assert_ne!(Py::Float(9_223_372_036_854_775_808.0), Py::Int(i64::MAX));
}

/// The Armada I converter writes `{class}: {text}` of an exception into its output, so the
/// class and text of what the shared helpers raise are Python's own.
#[test]
fn exception_classes_and_texts_match_python() {
    use std::collections::HashMap;
    check("RAW_TEXT", tables::RAW_TEXT, |a| {
        let name = a[0].as_str().unwrap();
        let args = a[1].items().unwrap();
        let v = &args[0];
        match name {
            "item" => show_text(v.item(args[1].as_int().unwrap() as usize).cloned()),
            "items" => show_text(v.items().map(|x| Py::List(x.to_vec()))),
            "len" => show_text(v.py_len().map(|n| Py::Int(n as i64))),
            "lt" => show_text(v.py_lt(&args[1]).map(Py::Bool)),
            "floordiv" => show_text(py_floordiv(v.as_int().unwrap(), args[1].as_int().unwrap()).map(Py::Int)),
            "mod" => show_text(py_mod(v.as_int().unwrap(), args[1].as_int().unwrap()).map(Py::Int)),
            "int" => show_text(py_int(v.as_str().unwrap(), args[1].as_int().unwrap() as u32).map(Py::Int)),
            "del" => {
                let mut d: IndexMap<Py, Py> = IndexMap::new();
                for p in v.items().unwrap() {
                    d.insert(p.item(0).unwrap().clone(), p.item(1).unwrap().clone());
                }
                show_text(d.py_del(&args[1]).map(|_| Py::None))
            }
            "lua_str" => show_text(lua_str_py(v).map(Py::Str)),
            "render_elem" => show_text(render(v, ctx_of(&args[1])).map(Py::Str)),
            "typed_and" => show_text(typed_args(std::slice::from_ref(v), Some(&["M".to_string()])).map(Py::List)),
            "unpack" => show_text(branch_pred("je", &tup!["ftest", v, &args[1]], 0x1000A)),
            "neg" => show_text(negate(v)),
            "jcc" => show_text(branch_pred(v.as_str().unwrap(), &tup!["cmp", tup!["const", 1], tup!["const", 2]], 0x1000A)),
            "evaluate" => show_text(evaluate(v, &HashMap::new()).map(Py::Bool)),
            "msvc_ret" => show_text(msvc_ret(v.as_str().unwrap()).map(|o| o.map_or(Py::None, Py::Str))),
            "expr_arg" => show_text(expr_arg(v, 0x1000)),
            "tag" => show_text(v.item(0).cloned()),
            other => panic!("{other}"),
        }
    });
}

/// Runs the committed generator and compares its output with the committed table (skipped where
/// `python3` is missing or is not CPython 3.14, which the table was made with: the texts of some
/// Python exceptions differ between versions).
#[test]
fn committed_tables_are_what_the_generator_writes() {
    let Some(py) = python3() else {
        eprintln!("skipped: python3 not found");
        return;
    };
    let version = std::process::Command::new(&py).arg("--version").output().unwrap();
    if !String::from_utf8_lossy(&version.stdout).starts_with("Python 3.14.") {
        eprintln!("skipped: the table was made with CPython 3.14, found {}", String::from_utf8_lossy(&version.stdout).trim());
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let gen_py = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/core_data/gen_tables.py");
    let out = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("tables_regenerated.rs");
    let status = std::process::Command::new(py)
        .arg("-I")
        .arg(&gen_py)
        .arg(&root)
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .status()
        .expect("run python3");
    assert!(status.success(), "the generator failed");
    let committed = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/core_data/tables.rs")).unwrap();
    assert!(committed == std::fs::read(&out).unwrap(), "tests/core_data/tables.rs differs from the generator's output ({})", out.display());
}
