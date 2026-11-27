//! Tests of the rule-DLL decoder (LUA-15g, `src/drl.rs`).
//!
//! - `cases_match_the_python`: invented listings (`drl_cases/cases.txt`) whose expected results
//!   were computed by the original Python functions (`drl_cases/gen_cases.py` reads no game file).
//! - Unit tests of the decoder state helpers, the scaffold text, the install walk and the engine
//!   lookup, over data these tests author.
//! - `gated_scaffolds_equal_the_python_output`: install-gated (`STA64_TEST_A2_STOCK_INSTALL`, the
//!   parity harness' reference): the scaffold of every rule DLL of the install equals the Python
//!   tool's file, but for the first line. It asserts equality and prints counts only.

#[path = "parity/harness.rs"]
mod harness;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sta_mission_convert::drl::{
    clone_state, decode, decode_install, find_drls, fpeek, fpop, install_subdirs, load_engine, merge, scaffold,
    Decoded, Drl, Failure, Rule, St, Variable, ENGINE_CLASSES,
};
use sta_mission_convert::fixture::{Import, PeBuilder, CODE, DATA};
use sta_mission_convert::format::{render, Ctx};
use sta_mission_convert::map::Vtables;
use sta_mission_convert::pyval::{py_isspace, py_strip, Py};
use sta_mission_convert::{tup, ErrorKind, FlatImage};

// ------------------------------------------------------------------ the case file

const CASES: &str = include_str!("drl_cases/cases.txt");

#[derive(Default)]
struct Case {
    name: String,
    iat: Vec<(u32, String, String)>,
    strs: Vec<(u32, String)>,
    bytes: Vec<(u32, Vec<u8>)>,
    export: u32,
    engine: Option<Vtables>,
    ins: Vec<(u32, String)>,
    op: Vec<String>,
    expect: Vec<String>,
}

fn addr(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).expect("address")
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect()
}

fn parse_cases() -> Vec<Case> {
    let mut out: Vec<Case> = Vec::new();
    for line in CASES.lines() {
        let (tag, rest) = line.split_once('|').expect("tag|payload");
        if tag == "case" {
            out.push(Case { name: rest.to_owned(), ..Case::default() });
            continue;
        }
        let c = out.last_mut().expect("a case first");
        match tag {
            "iat" => {
                let p: Vec<&str> = rest.splitn(3, '|').collect();
                c.iat.push((addr(p[0]), p[1].to_owned(), p[2].to_owned()));
            }
            "str" => {
                let (a, s) = rest.split_once('|').expect("str");
                c.strs.push((addr(a), s.to_owned()));
            }
            "f32" | "f64" => {
                let (a, h) = rest.split_once('|').expect("float");
                c.bytes.push((addr(a), unhex(h)));
            }
            "export" => c.export = rest.parse().expect("size"),
            "engine" => {
                let (cls, names) = rest.split_once('|').expect("engine");
                let names: Vec<String> =
                    if names.is_empty() { Vec::new() } else { names.split('|').map(str::to_owned).collect() };
                c.engine.get_or_insert_with(Vtables::new).insert(cls.to_owned(), names);
            }
            "ins" => {
                let (a, t) = rest.split_once('|').expect("ins");
                c.ins.push((addr(a), t.to_owned()));
            }
            "op" => c.op = rest.split('|').map(str::to_owned).collect(),
            _ => c.expect.push(line.to_owned()),
        }
    }
    out
}

/// The image of the Python test double: `.text` and `.rdata` ranges, the data at its addresses.
fn image(c: &Case) -> FlatImage {
    let mut img = FlatImage::new(0x1000_0000, 0x40000)
        .with_section(".text", 0x1000, 0x35000)
        .with_section(".rdata", 0x36000, 0xa000);
    for (a, dll, sym) in &c.iat {
        img = img.with_import(*a, dll, sym);
    }
    if c.export != 0 {
        img = img.with_export_size(c.export);
    }
    for (a, s) in &c.strs {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        img.put(*a, &b);
    }
    for (a, b) in &c.bytes {
        img.put(*a, b);
    }
    img
}

fn failure_line(e: &sta_mission_convert::DecodeError) -> String {
    if e.kind == ErrorKind::Raw {
        "raw|".to_owned()
    } else {
        format!("err|{}", e.message)
    }
}

fn run_case(c: &Case) -> Vec<String> {
    let mut out = Vec::new();
    let img = image(c);
    let ins: Vec<(u32, String)> = c.ins.clone();
    match c.op[0].as_str() {
        "condition" => {
            let a = i64::from(addr(&c.op[1]));
            let r = Drl::new(img, ins, c.engine.as_ref()).and_then(|mut d| {
                let e = d.condition(a)?;
                let lua = render(&e, Ctx::Bool)?;
                Ok((d.engine_slots.clone(), e, lua))
            });
            match r {
                Ok((slots, e, lua)) => {
                    out.push(format!("ok|{lua}"));
                    let s: Vec<String> = slots.iter().map(i64::to_string).collect();
                    out.push(format!("slots|{}", s.join(",")));
                    let mut shapes = Vec::new();
                    sta_mission_convert::drl::call_shapes(&e, &mut |s| shapes.push(s));
                    out.push(format!("shapes|{}", shapes.join(",")));
                }
                Err(e) => out.push(failure_line(&e)),
            }
        }
        "rules" => match Drl::new(img, ins, c.engine.as_ref()).and_then(|d| d.rules()) {
            Ok((created, deps)) => {
                for (n, cond, act) in &created {
                    out.push(format!("rule|{n}|{cond:#x}|{act:#x}"));
                }
                let mut keys: Vec<&usize> = deps.keys().collect();
                keys.sort();
                for k in keys {
                    out.push(format!("dep|{k}|{}", deps[k].join(",")));
                }
            }
            Err(e) => out.push(failure_line(&e)),
        },
        "decode" => match decode(img, ins, &c.op[1], c.engine.as_ref()) {
            Ok(m) => {
                out.push(format!("calls|{}", m.calls.join(",")));
                let sites: Vec<String> = m.call_sites.iter().map(|(k, v)| format!("{k}={v}")).collect();
                out.push(format!("sites|{}", sites.join(",")));
                out.push(format!("sigs|{}", m.call_signatures.join(",")));
                out.push(format!("unused|{}", m.unused_variables.join(",")));
                out.push(format!("errors|{}", m.errors.join("\\n")));
                let text = scaffold(&m);
                for l in text.split('\n').take(text.matches('\n').count()) {
                    out.push(format!("out|{l}"));
                }
            }
            Err(Failure::Unsupported(s)) => out.push(format!("unsupported|{s}")),
            Err(Failure::Error(e)) => out.push(failure_line(&e)),
        },
        other => panic!("unknown op {other}"),
    }
    out
}

fn normalise(line: &str) -> String {
    // the Python's exception class name is not compared, only that the fault was a raw one
    if line.starts_with("raw|") { "raw|".to_owned() } else { line.to_owned() }
}

#[test]
fn cases_match_the_python() {
    let cases = parse_cases();
    assert!(cases.len() > 100, "{} cases", cases.len());
    let mut bad = Vec::new();
    for c in &cases {
        let got = run_case(c);
        let want: Vec<String> = c.expect.iter().map(|l| normalise(l)).collect();
        if got != want {
            let at = got.iter().zip(&want).position(|(a, b)| a != b).unwrap_or(got.len().min(want.len()));
            bad.push(format!(
                "case {}: line {at}\n  python: {:?}\n  rust:   {:?}",
                c.name,
                want.get(at),
                got.get(at)
            ));
        }
    }
    assert!(bad.is_empty(), "{} of {} cases differ:\n{}", bad.len(), cases.len(), bad.join("\n"));
}

#[test]
fn the_case_file_covers_every_kind_of_result() {
    let cases = parse_cases();
    for kind in ["ok|", "err|", "raw|", "unsupported|", "rule|", "dep|", "out|"] {
        assert!(cases.iter().any(|c| c.expect.iter().any(|l| l.starts_with(kind))), "no case with {kind}");
    }
}

// ------------------------------------------------------------------ state helpers

fn st_with(regs: &[(&str, Py)], locals: &[(i64, Py)]) -> St {
    let mut st = St::new();
    for (k, v) in regs {
        st.r.insert((*k).to_owned(), v.clone());
    }
    for (k, v) in locals {
        st.local.insert(*k, v.clone());
    }
    st
}

/// The Python `JoinOrderTests`: a merged state keeps the first-seen order of registers and locals.
#[test]
fn merge_keeps_first_seen_register_order() {
    let first: Vec<String> = (1..=40).rev().map(|n| format!("r{n:02}x")).collect();
    let mut second: Vec<String> = vec!["extra_a".into(), "extra_b".into()];
    second.extend(first.iter().rev().take(20).cloned());
    let one = st_with(
        &first.iter().map(|k| (k.as_str(), tup!["const", 1])).collect::<Vec<_>>(),
        &(1..=20).rev().map(|n| (8 * n, tup!["const", 1])).collect::<Vec<_>>(),
    );
    let two = st_with(
        &second.iter().map(|k| (k.as_str(), tup!["const", 1])).collect::<Vec<_>>(),
        &[(4, tup!["const", 1]), (8, tup!["const", 1])],
    );
    let (path, out) = merge(vec![(vec![], one), (vec![], two)]).unwrap();
    assert!(path.is_empty());
    let mut want = first.clone();
    want.extend(["extra_a".to_owned(), "extra_b".to_owned()]);
    assert_eq!(out.r.keys().cloned().collect::<Vec<_>>(), want);
    let mut want_local: Vec<i64> = (1..=20).rev().map(|n| 8 * n).collect();
    want_local.push(4);
    assert_eq!(out.local.keys().copied().collect::<Vec<_>>(), want_local);
}

#[test]
fn merge_joins_zero_one_values_and_flags_conflicts() {
    let a = tup!["cmp", "==", tup!["obj", "x"], tup!["const", 1]];
    let one = st_with(&[("eax", tup!["const", 1])], &[(4, tup!["const", 7])]);
    let two = st_with(&[("eax", tup!["const", 0])], &[(4, tup!["const", 8])]);
    let (path, out) = merge(vec![(vec![a.clone()], one), (vec![], two)]).unwrap();
    assert!(path.is_empty());
    // the value 1 came along the path `a`: the join is `a`
    assert_eq!(out.reg("eax"), tup!["boolv", a]);
    assert_eq!(out.local_get(4), tup!["conflict"]);
    // different pushed arguments cannot join
    let mut p = St::new();
    p.args.push(tup!["const", 1]);
    let e = merge(vec![(vec![], p), (vec![], St::new())]).unwrap_err();
    assert_eq!(e.message, "paths join with different args");
    // one incoming state comes back unchanged
    let (path, same) = merge(vec![(vec![tup!["const", 1]], St::new())]).unwrap();
    assert_eq!((path.len(), same.r.len()), (1, 0));
}

#[test]
fn fpop_and_fpeek_use_a_pending_call_result_once() {
    let mut st = St::new();
    assert_eq!(fpop(&mut st, 0x10).unwrap_err().message, "x87 stack empty at 0x10");
    st.fret = tup!["call", "f", Py::List(vec![])];
    assert_eq!(fpeek(&mut st, 0x10).unwrap(), tup!["call", "f", Py::List(vec![])]);
    assert_eq!(st.fpu.len(), 1);
    assert!(st.fret.is_none());
    assert_eq!(fpop(&mut st, 0x10).unwrap(), tup!["call", "f", Py::List(vec![])]);
    assert!(fpop(&mut st, 0x10).is_err());
    // a result is consumed once even when it comes through fpop directly
    st.fret = tup!["const", 1];
    assert_eq!(fpop(&mut st, 0).unwrap(), tup!["const", 1]);
    assert!(fpop(&mut st, 0).is_err());
}

#[test]
fn clone_state_shares_nothing() {
    let mut a = st_with(&[("eax", tup!["const", 1])], &[(4, tup!["const", 2])]);
    a.args.push(tup!["const", 3]);
    a.marks.insert("esi".into(), 1);
    let b = clone_state(&a);
    a.r.insert("eax".into(), Py::None);
    a.local.clear();
    a.args.clear();
    a.marks.clear();
    assert_eq!(b.reg("eax"), tup!["const", 1]);
    assert_eq!(b.local_get(4), tup!["const", 2]);
    assert_eq!((b.args.len(), b.marks.len()), (1, 1));
}

// ------------------------------------------------------------------ scaffold

/// The Python `ScaffoldTests`: the synthetic mission the runtime's test loads is what the
/// scaffold writer makes of it (the first line names this tool where the fixture names the Python).
#[test]
fn the_scaffold_is_the_runtime_fixture() {
    let src = include_str!("../../sta_mission/tests/drl.rs");
    let start = src.find("const SYNTHETIC_SCAFFOLD: &str = r#\"").expect("fixture") + "const SYNTHETIC_SCAFFOLD: &str = r#\"".len();
    let end = start + src[start..].find("\"#;").expect("end of the fixture");
    let fixture = &src[start..end];

    let alpha = tup![
        "and",
        Py::List(vec![
            tup!["truth", tup!["var", "p_ALWAYS_EVALUATE", "dirty_int"]],
            tup!["truth", tup!["var", "p_ALPHA", "hot_int"]],
            tup!["not", tup!["truth", tup!["call", "IsAlive", Py::List(vec![tup!["obj", "ship"]])]]],
        ])
    ];
    let timer = tup!["truth", tup!["var", "p_tWait", "timer_int"]];
    let missing = tup!["cmp", ">", tup!["call", "NotBoundYet", Py::List(vec![tup!["const", 1]])], tup!["const", 0]];
    let rule = |name: &str, action: &str, deps: &[&str], e: Py| Rule {
        name: name.into(),
        action: action.into(),
        deps: deps.iter().map(|s| (*s).to_owned()).collect(),
        condition_lua: Some(render(&e, Ctx::Bool).unwrap()),
        condition: Some(e),
        error: None,
    };
    let var = |n: &str, k: &str, engine: bool| Variable { name: n.into(), kind: k.into(), engine };
    let m = Decoded {
        mission: "synthetic01".into(),
        drl: "synthetic01R.drl".into(),
        dsl: "synthetic01S.dsl".into(),
        variables: vec![var("p_ALPHA", "hot_int", false), var("p_ALWAYS_EVALUATE", "dirty_int", true), var("p_tWait", "timer_int", false)],
        objects: vec!["ship".into()],
        actions: vec!["Do_Alpha".into(), "Do_Timer".into(), "Do_Missing".into()],
        rules: vec![
            rule("AlphaRule", "Do_Alpha", &["p_ALPHA", "p_ALWAYS_EVALUATE"], alpha),
            rule("TimerRule", "Do_Timer", &["p_tWait"], timer),
            rule("MissingRule", "Do_Missing", &["p_ALWAYS_EVALUATE"], missing),
        ],
        calls: vec![],
        call_sites: Default::default(),
        call_signatures: vec![],
        unused_variables: vec![],
        errors: vec![],
    };
    let got = scaffold(&m);
    let (gfirst, grest) = got.split_once('\n').unwrap();
    let (ffirst, frest) = fixture.split_once('\n').unwrap();
    assert_eq!(grest, frest);
    assert_eq!(
        gfirst.replace("sta_mission_convert (LUA-15g)", "tools/drl_to_lua.py (LUA-07)"),
        ffirst
    );
}

#[test]
fn a_condition_that_was_not_decoded_is_a_comment_and_a_false_function() {
    let m = Decoded {
        mission: "m".into(),
        drl: "mR.drl".into(),
        dsl: "mS.dsl".into(),
        variables: vec![],
        objects: vec![],
        actions: vec!["Do-It".into(), "Do_It".into()],
        rules: vec![Rule {
            name: "it's".into(),
            action: "Do-It".into(),
            deps: vec![],
            condition: None,
            condition_lua: None,
            error: Some("why".into()),
        }],
        calls: vec![],
        call_sites: Default::default(),
        call_signatures: vec![],
        unused_variables: vec![],
        errors: vec![],
    };
    let text = scaffold(&m);
    for want in [
        "    -- condition not decoded: why\n    when = function(S, O) return false end,\n",
        "Mission['Do-It'] = function(self) self.ran[#self.ran + 1] = 'Do-It' end\n",
        "function Mission:Do_It() self.ran[#self.ran + 1] = 'Do_It' end\n",
        "    name = 'it\\'s',\n",
        "Mission.objectNames = {  }\n",
        "    deps = {  },\n",
    ] {
        assert!(text.contains(want), "missing {want:?}");
    }
    assert!(text.ends_with("MMM.register(Mission)\n"));
}

// ------------------------------------------------------------------ the install walk

fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("drl_{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn install_subdirs_are_distinct_and_cleaned() {
    let roots: Vec<PathBuf> = ["/a/Star Trek: A2", "/b/Star Trek: A2/", "/c/star trek: a2", "/d/ok-1.2_x", "/", ".", "/e/a  b"]
        .iter()
        .map(PathBuf::from)
        .collect();
    assert_eq!(
        install_subdirs(&roots),
        ["Star_Trek_A2", "Star_Trek_A2_2", "star_trek_a2_3", "ok-1.2_x", "install", ".", "a_b"]
    );
}

#[test]
fn find_drls_matches_the_end_of_the_lower_cased_name_and_sorts_the_paths() {
    let root = tmp("find");
    for f in ["b/xR.drl", "a/yr.DRL", "a/zR.drl", "c/notes.txt", "c/foo.drl", "AR.DRL", "d/e/deepR.drl"] {
        let p = root.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }
    let found: Vec<String> = find_drls(&root)
        .iter()
        .map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(found, ["AR.DRL", "a/yr.DRL", "a/zR.drl", "b/xR.drl", "d/e/deepR.drl"]);
}

/// A tiny rule DLL as bytes: one registered rule, its condition and its action, over invented
/// imports. Returns the file and its listing.
fn synthetic_dll(armada1: bool) -> (Vec<u8>, Vec<(u32, String)>) {
    const BASE: u32 = 0x1000_0000;
    // The Armada I shape: no `g_pSI`, and an import from `Armada.exe`.
    let names: Vec<&str> = if armada1 {
        vec!["?p_WORLD@@3PAVcharacter@@A", "?p_ALPHA@@3PAVhot_int@@A", "?Do_Alpha@@YAXXZ"]
    } else {
        vec![
            "?p_WORLD@@3PAVcharacter@@A",
            "?g_pSI@@3PAVScriptInterface@@A",
            "?p_ALPHA@@3PAVhot_int@@A",
            "?Do_Alpha@@YAXXZ",
        ]
    };
    let mut rdata = b"RuleOne\0".to_vec();
    rdata.resize(0x40, 0);
    let mut b = PeBuilder::new(BASE)
        .section(".text", 0x1000, &[0u8; 0x1000], CODE)
        .section(".rdata", 0x3000, &rdata, DATA)
        .import("synthS.dsl", names.iter().map(|n| Import::name(n)).collect());
    if armada1 {
        b = b.import("Armada.exe", vec![Import::name("?Foo@@YAXXZ")]);
    }
    let built = b.build();
    let slot = |n: &str| built.import_slots[names.iter().position(|x| x.contains(n)).expect("an import")];
    let (world, alpha, action) = (slot("p_WORLD"), slot("p_ALPHA"), slot("Do_Alpha"));
    let chk = 0x1000_1900u32;
    let mut l: Vec<(u32, String)> = Vec::new();
    fn emit(l: &mut Vec<(u32, String)>, at: u32, lines: &[String]) {
        for (i, t) in lines.iter().enumerate() {
            l.push((at + 4 * i as u32, t.clone()));
        }
    }
    let prologue: Vec<String> = [
        "push   ebp", "mov    ebp,esp", "sub    esp,0x44", "push   ebx", "push   esi", "push   edi", "lea    edi,[ebp-0x44]",
        "mov    ecx,0x11", "mov    eax,0xcccccccc", "rep stos DWORD PTR es:[edi],eax",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let epilogue: Vec<String> = [
        "pop    edi".to_owned(), "pop    esi".to_owned(), "pop    ebx".to_owned(), "add    esp,0x44".to_owned(),
        "cmp    ebp,esp".to_owned(), format!("call   {chk:#x}"), "mov    esp,ebp".to_owned(), "pop    ebp".to_owned(),
        "ret".to_owned(),
    ]
    .to_vec();
    // thunks: registration, condition, action
    let thunks = [(0x1000_1000u32, 0x1000_1100u32), (0x1000_1005, 0x1000_1400), (0x1000_100a, 0x1000_1700)];
    for (a, t) in thunks {
        l.push((a, format!("jmp    {t:#x}")));
    }
    let reg: Vec<String> = [
        format!("mov    eax,ds:{world:#x}"),
        "mov    ecx,DWORD PTR [eax]".to_owned(),
        "mov    edx,DWORD PTR [ecx]".to_owned(),
        "mov    esi,esp".to_owned(),
        "call   DWORD PTR [edx]".to_owned(),
        "cmp    esi,esp".to_owned(),
        format!("call   {chk:#x}"),
        "mov    DWORD PTR [ebp-0x4],eax".to_owned(),
        "mov    esi,esp".to_owned(),
        "push   0x10003000".to_owned(),
        "push   0x1000100a".to_owned(),
        "push   0x10001005".to_owned(),
        "mov    ecx,DWORD PTR [ebp-0x4]".to_owned(),
        "mov    edx,DWORD PTR [ecx]".to_owned(),
        "mov    ecx,DWORD PTR [ebp-0x4]".to_owned(),
        "call   DWORD PTR [edx+0x4]".to_owned(),
        "cmp    esi,esp".to_owned(),
        format!("call   {chk:#x}"),
        "mov    DWORD PTR ds:0x10002000,eax".to_owned(),
        format!("mov    eax,ds:{alpha:#x}"),
        "mov    ecx,DWORD PTR [eax]".to_owned(),
        "mov    edx,DWORD PTR [ecx]".to_owned(),
        "mov    esi,esp".to_owned(),
        "mov    eax,DWORD PTR ds:0x10002000".to_owned(),
        "push   eax".to_owned(),
        "call   DWORD PTR [edx+0x4]".to_owned(),
        "cmp    esi,esp".to_owned(),
        format!("call   {chk:#x}"),
    ]
    .to_vec();
    let cond: Vec<String> = [
        format!("mov    eax,ds:{alpha:#x}"),
        "mov    ecx,DWORD PTR [eax]".to_owned(),
        format!("mov    edx,DWORD PTR ds:{alpha:#x}"),
        "mov    eax,DWORD PTR [edx]".to_owned(),
        "mov    edx,DWORD PTR [eax]".to_owned(),
        "mov    esi,esp".to_owned(),
        "call   DWORD PTR [edx+0x20]".to_owned(),
        "cmp    esi,esp".to_owned(),
        format!("call   {chk:#x}"),
    ]
    .to_vec();
    let act = vec![format!("call   DWORD PTR ds:{action:#x}")];
    emit(&mut l, 0x1000_1100, &[prologue.clone(), reg, epilogue.clone()].concat());
    emit(&mut l, 0x1000_1400, &[prologue.clone(), cond, epilogue.clone()].concat());
    emit(&mut l, 0x1000_1700, &[prologue, act, epilogue].concat());
    l.push((chk, "ret".to_owned()));
    l.sort();
    (built.bytes, l)
}

#[test]
fn decode_install_runs_main_over_a_folder() {
    let root = tmp("install");
    let (good, ins) = synthetic_dll(false);
    let (armada1, _) = synthetic_dll(true);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("synthR.drl"), &good).unwrap();
    std::fs::write(root.join("sub/otherR.drl"), &good).unwrap();
    // the same mission name in a folder that sorts later: it replaces the earlier file
    std::fs::create_dir_all(root.join("zz")).unwrap();
    std::fs::write(root.join("zz/synthR.drl"), &good).unwrap();
    std::fs::write(root.join("sub/brokenR.drl"), b"not a pe file").unwrap();
    // the Armada I shape (no g_pSI, an import from Armada.exe) is skipped
    std::fs::write(root.join("sub/oldR.drl"), &armada1).unwrap();
    std::fs::write(root.join("sub/readme.txt"), b"x").unwrap();

    let mut listed = 0usize;
    let run = decode_install(&root, None, &mut |p: &Path| {
        listed += 1;
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if name == "oldR.drl" {
            // the Armada I shape has no g_pSI: only the imports matter for it
            Ok(vec![(0x1000_1000, "ret".to_owned())])
        } else {
            Ok(ins.clone())
        }
    })
    .unwrap();
    assert_eq!(run.read, 5);
    assert_eq!(listed, 4, "the file that is no PE is not listed (the Python opens the image first)");
    assert_eq!((run.failed, run.skipped), (1, 1), "{run:?}");
    assert_eq!(run.files.keys().map(String::as_str).collect::<Vec<_>>(), ["other.lua", "synth.lua"]);
    let text = String::from_utf8(run.files["synth.lua"].clone()).unwrap();
    assert!(text.starts_with("-- synth: rule scaffold generated by sta_mission_convert (LUA-15g).\n"));
    assert!(text.contains("name = 'RuleOne',") && text.contains("action = 'Do_Alpha',"));
    assert!(text.contains("deps = { 'p_ALPHA' },") && text.contains("return T(S.p_ALPHA) end,"));
    assert!(text.contains("-- Rules decoded from synthR.drl;") && text.contains("-- live in synthS.dsl and"));
    // the file name decides the mission name
    assert!(String::from_utf8(run.files["other.lua"].clone()).unwrap().starts_with("-- other: rule scaffold"));

    // a raw fault in any file aborts the run
    let e = decode_install(&root, None, &mut |_: &Path| {
        Err(sta_mission_convert::DecodeError::raw("boom"))
    })
    .unwrap_err();
    assert!(e.is_raw());
    // a fault of the decoder kind from the listing source is a failed file
    let run = decode_install(&root, None, &mut |_: &Path| Err(sta_mission_convert::DecodeError::new("no listing"))).unwrap();
    assert_eq!((run.files.len(), run.failed), (0, 5));
}

// ------------------------------------------------------------------ the engine lookup

/// A fake engine executable and its map: each class's vtable lists the given names, each name is
/// a function at its own code address. Written as `Armada2.exe` / `armada2.map` in `dir`.
fn write_engine(dir: &Path, tables: &[(&str, Vec<&str>)]) {
    std::fs::create_dir_all(dir).unwrap();
    let mut funcs: Vec<&str> = Vec::new();
    for (_, names) in tables {
        for n in names {
            if !funcs.contains(n) {
                funcs.push(n);
            }
        }
    }
    let code = |n: &str| 0x40_1000u32 + 0x10 * funcs.iter().position(|f| *f == n).unwrap() as u32;
    let mut rd: Vec<u8> = Vec::new();
    let mut map = String::new();
    for (i, f) in funcs.iter().enumerate() {
        map.push_str(&format!(" 0001:{:08x} {f} {:08x} f\n", 0x10 * i, code(f)));
    }
    for (cls, names) in tables {
        map.push_str(&format!(" 0002:{:08x} ??_7{cls}@@6B@ {:08x} \n", rd.len(), 0x40_2000 + rd.len()));
        for n in names {
            rd.extend_from_slice(&code(n).to_le_bytes());
        }
        // a word outside the code ends the table even where no symbol follows
        rd.extend_from_slice(&0u32.to_le_bytes());
    }
    let exe = PeBuilder::new(0x40_0000)
        .section(".text", 0x1000, &vec![0x90u8; 0x400], CODE)
        .section(".rdata", 0x2000, &rd, DATA)
        .build()
        .bytes;
    std::fs::write(dir.join("Armada2.exe"), exe).unwrap();
    std::fs::write(dir.join("armada2.map"), map).unwrap();
}

fn good_tables() -> Vec<(&'static str, Vec<&'static str>)> {
    let slots = |special: &[(usize, &'static str)], n: usize| -> Vec<&'static str> {
        let pad: [&'static str; 9] = ["?p0@x@@", "?p1@x@@", "?p2@x@@", "?p3@x@@", "?p4@x@@", "?p5@x@@", "?p6@x@@", "?p7@x@@", "?p8@x@@"];
        let mut v: Vec<&'static str> = pad[..n].to_vec();
        for (i, s) in special {
            v[*i] = *s;
        }
        v
    };
    vec![
        ("ScriptInterfaceImp", vec!["?Foo@ScriptInterfaceImp@@UAEXXZ", "?ScriptGet_ship@ScriptInterfaceImp@@UAEHXZ"]),
        ("character", slots(&[(0, "?GetIIIE@character@@UAEPAVIIIE@@XZ")], 1)),
        ("IIIE", slots(&[(1, "?create_rule@IIIE@@UAEPAVrule@@P6AHXZP6AXXZPAD@Z")], 2)),
        ("hot_int", slots(&[(1, "?add_dependent_rule@hot_type@@UAEXPAVrule@@@Z"), (8, "??Bhot_int@@UAEHXZ")], 9)),
        ("hot_float", slots(&[(8, "??Bhot_float@@UAEMXZ")], 9)),
        ("dirty_int", slots(&[(8, "??Bhot_int@@UAEHXZ")], 9)),
        ("timer_int", slots(&[(8, "??Bhot_int@@UAEHXZ")], 9)),
    ]
}

#[test]
fn the_engine_lookup_names_the_vtables_and_checks_the_slots() {
    let base = tmp("engine");
    let good = base.join("good");
    write_engine(&good, &good_tables());
    let vt = load_engine(std::slice::from_ref(&good), None).unwrap().expect("engine names");
    assert_eq!(vt.keys().map(String::as_str).collect::<Vec<_>>(), ENGINE_CLASSES.to_vec());
    assert_eq!(vt["character"], ["?GetIIIE@character@@UAEPAVIIIE@@XZ"]);
    assert_eq!(vt["hot_float"][8], "??Bhot_float@@UAEMXZ");
    assert_eq!(vt["ScriptInterfaceImp"].len(), 2);

    // a slot that is not the one the decoder was measured on: the Python's `sys.exit`
    let mut tables = good_tables();
    tables[3].1[8] = "?other@x@@";
    let bad = base.join("bad");
    write_engine(&bad, &tables);
    let e = load_engine(std::slice::from_ref(&bad), None).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Decode);
    assert_eq!(
        e.message,
        "rule-engine slots differ from the ones this decoder was measured on:\n  \
         hot_int slot 8: expected ??Bhot_int@@, found ?other@x@@"
    );

    // no executable and map: no names. An empty folder, and a folder that does not exist.
    let empty = base.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(load_engine(&[empty.clone(), base.join("missing")], None).unwrap().is_none());

    // a root whose map names none of the classes (an empty table is false in Python): the next root
    let none = base.join("none");
    write_engine(&none, &[("Other", vec!["?a@x@@"])]);
    let vt = load_engine(&[none.clone(), good.clone()], None).unwrap().unwrap();
    assert!(vt.contains_key("character"));
    // an engine root named explicitly is the only candidate
    assert!(load_engine(std::slice::from_ref(&good), Some(&none)).unwrap().is_none());
    assert!(load_engine(std::slice::from_ref(&none), Some(&good)).unwrap().is_some());
    // an unreadable or undecodable root is skipped, the next one is tried
    let junk = base.join("junk");
    std::fs::create_dir_all(&junk).unwrap();
    std::fs::write(junk.join("Armada2.exe"), b"not a pe").unwrap();
    std::fs::write(junk.join("armada2.map"), b"").unwrap();
    assert!(load_engine(&[junk, base.join("missing"), good], None).unwrap().is_some());
}

// ------------------------------------------------------------------ gated parity

/// The first of the Python `missions_dir` candidates: a folder spelled in any case, the first of
/// several in code-point order.
fn child(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut names: Vec<String> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names.into_iter().find(|n| n.to_lowercase() == name.to_lowercase() && dir.join(n).is_dir()).map(|n| dir.join(n))
}

/// `disassemble` of the Python tool, on the text of an `objdump` listing: `^\s*([0-9a-f]+):\t(.*)$`
/// per line, the text stripped.
fn parse_objdump(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let l = line.trim_start_matches(py_isspace);
        let Some((a, rest)) = l.split_once(':') else { continue };
        let Some(t) = rest.strip_prefix('\t') else { continue };
        if a.is_empty() || !a.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            continue;
        }
        out.push((u32::from_str_radix(a, 16).expect("address"), py_strip(t).to_owned()));
    }
    out
}

#[test]
fn parse_objdump_reads_the_lines_the_python_reads() {
    let text = "\nx.drl:     file format pei-i386\n\nDisassembly of section .text:\n\n10001000 <.text>:\n\
                10001000:\tjmp    0x10002000  \n  10001005:\tint3\n\t...\n10001010:\r\n10001011:\tmov    eax,ebx\r\n\
                10001013:mov\n";
    assert_eq!(
        parse_objdump(text),
        [
            (0x1000_1000, "jmp    0x10002000".to_owned()),
            (0x1000_1005, "int3".to_owned()),
            (0x1000_1011, "mov    eax,ebx".to_owned()),
        ]
    );
}

/// Gated. The scaffold of every rule DLL of the Armada II install equals the Python tool's file,
/// but for the tool token of the first line. Listings are the reference's own `objdump` listings.
#[test]
fn gated_scaffolds_equal_the_python_output() {
    let Some(reference) = harness::obtain_or_skip() else {
        return;
    };
    let Some(install) = reference.install(harness::Kind::Armada2) else {
        eprintln!("the reference has no Armada II install: skipped");
        return;
    };
    let Some(root) = std::env::var_os(harness::A2_VAR).map(PathBuf::from).filter(|p| p.is_dir()) else {
        eprintln!("{} is not set: skipped (the rule DLLs are read from the install)", harness::A2_VAR);
        return;
    };
    if !install.has_listings() {
        eprintln!("the reference has no objdump listings: skipped");
        return;
    }
    let mdir = child(&root, "missions")
        .or_else(|| child(&child(&root, "Data").unwrap_or_else(|| root.clone()), "missions"))
        .expect("a missions folder");
    let engine = load_engine(std::slice::from_ref(&mdir), Some(&root)).expect("the engine lookup");
    assert!(engine.is_some(), "the install has no engine names");
    let mut listing = |p: &Path| -> sta_mission_convert::Result<Vec<(u32, String)>> {
        let name = format!("{}.txt", p.file_name().unwrap().to_string_lossy().to_lowercase());
        let bytes = install
            .listing(&name)
            .ok_or_else(|| sta_mission_convert::DecodeError::raw("a rule DLL has no listing in the reference"))?;
        Ok(parse_objdump(&String::from_utf8_lossy(&bytes)))
    };
    let run = decode_install(&mdir, engine.as_ref(), &mut listing).expect("the run does not abort");
    let sub = &install_subdirs(std::slice::from_ref(&mdir))[0];
    let produced: BTreeMap<String, Vec<u8>> =
        run.files.iter().map(|(k, v)| (format!("drl/{sub}/{k}"), v.clone())).collect();
    let report = install.compare_subset(&produced, |rel| rel.starts_with("drl/"));
    eprintln!(
        "rule scaffolds: {} DLLs read, {} written, {} equal, {} differing, {} missing, {} extra; \
         {} DLLs failed to decode, {} skipped",
        run.read,
        run.files.len(),
        report.equal,
        report.differing.len(),
        report.missing.len(),
        report.extra.len(),
        run.failed,
        run.skipped
    );
    assert!(report.equal > 0, "nothing was compared");
    report.assert_ok("a2-rule-scaffolds");
}
