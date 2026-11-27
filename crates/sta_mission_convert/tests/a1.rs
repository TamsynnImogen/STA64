//! The Armada I front end (LUA-15e) against authored cases.
//!
//! The cases are `tests/a1_cases/cases.txt` (invented DLL shapes, no game data). Their expected
//! output, `tests/a1_cases/expected.txt`, is what the ORIGINAL `tools/a1_to_lua.py` prints for the
//! same cases (`tests/a1_cases/gen_cases.py`, which reads no game files): the Rust output must
//! equal it, case by case. `expected_cases_are_current` runs the Python again (when `python3` is
//! on the path) and checks the committed file is what it prints now.
//!
//! The install-gated comparison with the Python tool's output for the installed game is
//! `tests/a1_parity.rs`.

use std::collections::BTreeMap;
use std::path::Path;

use indexmap::IndexMap;
use sta_mission_convert::a1::{convert_dlls, find_missions, A1Dll};
use sta_mission_convert::emit::{render_function, Namer};
use sta_mission_convert::lift::Game;
use sta_mission_convert::msvc::plain_name;
use sta_mission_convert::prelude::{a1_prelude, A1_PRELUDE, ENGINE};
use sta_mission_convert::pyval::Py;
use sta_mission_convert::{ErrorKind, FlatImage};

/// The committed data with its line endings normalised (a checkout may have rewritten them).
fn data(s: &str) -> String {
    s.replace('\r', "")
}

type Block = (u32, Vec<String>);

#[derive(Default)]
struct Case {
    name: String,
    code: Vec<Block>,
    rcode: Vec<Block>,
    runs: Vec<String>,
}

#[derive(Default)]
struct Fixture {
    consts: BTreeMap<String, i64>,
    imports: Vec<(String, String)>,
    exports: Vec<(String, String)>,
    strings: Vec<(u32, String)>,
    base: Vec<Block>,
    rbase: Vec<Block>,
    cases: Vec<Case>,
    iat: BTreeMap<String, u32>,
}

fn num(s: &str) -> i64 {
    match s.strip_prefix("0x") {
        Some(h) => i64::from_str_radix(h, 16).expect("hex number"),
        None => s.parse().expect("decimal number"),
    }
}

impl Fixture {
    fn expand(&self, text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(i) = rest.find('{') {
            out.push_str(&rest[..i]);
            let j = rest[i..].find('}').expect("closing brace") + i;
            let body = &rest[i + 1..j];
            if let Some(k) = body.strip_prefix("iat:") {
                out.push_str(&format!("DWORD PTR ds:{:#x}", self.iat[k]));
            } else if let Some((n, k)) = body.split_once('+') {
                out.push_str(&format!("{:#x}", self.consts[n] + k.parse::<i64>().expect("offset")));
            } else {
                out.push_str(&format!("{:#x}", self.consts[body]));
            }
            rest = &rest[j + 1..];
        }
        out.push_str(rest);
        out
    }

    fn addr(&self, text: &str) -> u32 {
        let t = if text.contains('{') { self.expand(text) } else { text.to_owned() };
        num(&t) as u32
    }
}

fn parse(text: &str) -> Fixture {
    let mut fx = Fixture::default();
    // which block list the indented lines go to: (kind, index)
    let mut target: Option<(&str, usize)> = None;
    for line in text.split('\n') {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(t) = line.strip_prefix("  ") {
            let (k, i) = target.expect("a block");
            let list = match k {
                "base" => &mut fx.base,
                "rbase" => &mut fx.rbase,
                "code" => &mut fx.cases.last_mut().expect("a case").code,
                _ => &mut fx.cases.last_mut().expect("a case").rcode,
            };
            list[i].1.push(t.to_owned());
            continue;
        }
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        match word {
            "const" => {
                let (n, v) = rest.split_once(' ').expect("const NAME value");
                fx.consts.insert(n.to_owned(), num(v));
            }
            "import" => {
                let (dll, sym) = rest.split_once(' ').expect("import dll symbol");
                let slot = 0x1006_0000 + 4 * fx.imports.len() as u32;
                fx.imports.push((dll.to_owned(), sym.to_owned()));
                let key = if sym.starts_with("??") { sym.to_owned() } else { plain_name(sym) };
                fx.iat.insert(key, slot);
            }
            "export" => {
                let (sym, expr) = rest.rsplit_once(' ').expect("export symbol expr");
                fx.exports.push((sym.to_owned(), expr.to_owned()));
            }
            "string" => {
                let (a, s) = rest.split_once(' ').expect("string addr text");
                fx.strings.push((num(a) as u32, s.to_owned()));
            }
            "base" => {
                fx.base.push((num(rest) as u32, Vec::new()));
                target = Some(("base", fx.base.len() - 1));
            }
            "rbase" => {
                fx.rbase.push((num(rest) as u32, Vec::new()));
                target = Some(("rbase", fx.rbase.len() - 1));
            }
            "case" => fx.cases.push(Case { name: rest.to_owned(), ..Case::default() }),
            "code" | "rcode" => {
                let a = fx.addr(rest);
                let c = fx.cases.last_mut().expect("a case");
                let list = if word == "code" { &mut c.code } else { &mut c.rcode };
                list.push((a, Vec::new()));
                target = Some((if word == "code" { "code" } else { "rcode" }, list.len() - 1));
            }
            "run" => fx.cases.last_mut().expect("a case").runs.push(rest.to_owned()),
            "end" => {}
            other => panic!("cases.txt: unknown directive {other:?}"),
        }
    }
    fx
}

fn image(fx: &Fixture) -> FlatImage {
    let mut img = FlatImage::new(0x1000_0000, 0x7_0000)
        .with_section(".text", 0x1000, 0x2_F000)
        .with_section(".rdata", 0x4_0000, 0x1000)
        .with_section(".rdata", 0x6_0000, 0x1000)
        .with_section(".data", 0x5_0000, 0x1000);
    for (i, (dll, sym)) in fx.imports.iter().enumerate() {
        img = img.with_import(0x1006_0000 + 4 * i as u32, dll, sym);
    }
    for (a, s) in &fx.strings {
        let mut b: Vec<u8> = s.chars().map(|c| c as u8).collect();
        b.push(0);
        img.put(*a, &b);
    }
    img
}

/// `program(code)`: 4 bytes per instruction from each start, in address order.
fn program(fx: &Fixture, base: &[Block], blocks: &[Block]) -> Vec<(u32, String)> {
    let mut code: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for (start, lines) in base.iter().chain(blocks) {
        code.insert(*start, lines.iter().map(|l| fx.expand(l)).collect());
    }
    let mut ins = Vec::new();
    for (start, lines) in code {
        let mut a = start;
        for l in lines {
            ins.push((a, l));
            a += 4;
        }
    }
    ins
}

fn class_name(e: &sta_mission_convert::DecodeError) -> String {
    match (e.kind, e.class) {
        (ErrorKind::Decode, _) => "DecodeError".to_owned(),
        (_, Some(c)) => c.to_owned(),
        _ => "?".to_owned(),
    }
}

fn run_case(fx: &Fixture, case: &Case) -> String {
    let img = image(fx);
    let mut exports: IndexMap<String, u32> = IndexMap::new();
    for (sym, expr) in &fx.exports {
        exports.insert(sym.clone(), fx.addr(expr));
    }
    let mut out = format!("== {}\n", case.name);
    for r in &case.runs {
        let (what, arg) = r.split_once(' ').unwrap_or((r.as_str(), ""));
        out.push_str(&format!("-- {r}\n"));
        let s = A1Dll::new("Test1S.dsl", &img, program(fx, &fx.base, &case.code), exports.clone(), Some(0x1000_1100)).expect("a mission DLL");
        match what {
            "lift" => {
                let mut lf = s.lifter();
                let f = match lf.lift(i64::from(fx.addr(arg))) {
                    Ok(f) => f,
                    Err(e) => {
                        assert_eq!(e.kind, ErrorKind::Decode);
                        out.push_str(&format!("raise {}\n", Py::from(e.message).repr()));
                        continue;
                    }
                };
                out.push_str(f.canonical().trim_end_matches('\n'));
                out.push('\n');
                let und = Py::List(f.a1.undecoded.iter().map(|(a, b)| Py::Tuple(vec![Py::from(a.as_str()), Py::from(b.as_str())])).collect());
                out.push_str(&format!(" undecoded {}\n", und.repr()));
                out.push_str(&format!(" rules_made {}\n", Py::List(f.a1.rules.iter().map(|x| Py::Int(*x)).collect()).repr()));
                out.push_str(&format!(" imports_called {}\n", Py::List(f.a1.imports_called.iter().map(|x| Py::from(x.as_str())).collect()).repr()));
                out.push_str(&format!(" classify {}\n", Py::from(sta_mission_convert::a1::mission::classify(&f)).repr()));
                let funcs: IndexMap<i64, _> = IndexMap::from([(f.addr, f.clone())]);
                let roles = std::collections::HashMap::from([(f.addr, "step".to_owned())]);
                let mut nm = Namer::new(&s.dsl, &funcs, &roles);
                nm.game = Game::A1;
                match render_function(&f, &nm, "fn") {
                    Ok(l) => {
                        out.push_str("render ok\n");
                        out.push_str(&l.join("\n"));
                        out.push('\n');
                    }
                    Err(e) => out.push_str(&format!("render error {} {}\n", class_name(&e), Py::from(e.message).repr())),
                }
            }
            "mission" => {
                let rdll = (!case.rcode.is_empty())
                    .then(|| A1Dll::new("Test1R.drl", &img, program(fx, &fx.rbase, &case.rcode), IndexMap::new(), Some(0x1000_1100)).expect("a rule DLL"));
                let (mission, src, rep) = convert_dlls(&s, rdll.as_ref(), Some("test1")).expect("the mission converts");
                let (head, body) = src.split_once('\n').expect("a header line");
                assert!(head.starts_with("-- test1: converted by sta_mission_convert ("), "{head}");
                let prelude = a1_prelude().expect("prelude");
                let (pre, rest) = body.split_once(prelude.as_str()).expect("the prelude");
                let (_, after) = rest.split_once(A1_PRELUDE).expect("the A1 prelude");
                let (mid, _) = after.split_once(ENGINE).expect("the engine block");
                out.push_str(&format!("mission {}\nprefix\n{}\nbody\n{}\n", Py::from(mission).repr(), pre.trim_end_matches('\n'), mid.trim_end_matches('\n')));
                out.push_str(&rep.canonical());
            }
            other => panic!("cases.txt: unknown run {other:?}"),
        }
    }
    out
}

#[test]
fn every_case_equals_the_originals_output() {
    let fx = parse(&data(include_str!("a1_cases/cases.txt")));
    let expected = data(include_str!("a1_cases/expected.txt"));
    assert!(fx.cases.len() >= 10, "{} cases", fx.cases.len());
    let got: String = fx.cases.iter().map(|c| run_case(&fx, c)).collect();
    if got != expected {
        let line = got.lines().zip(expected.lines()).position(|(a, b)| a != b).unwrap_or(got.lines().count().min(expected.lines().count()));
        panic!(
            "output differs at line {}:\n   rust:   {}\n   python: {}",
            line + 1,
            got.lines().nth(line).unwrap_or("<end>"),
            expected.lines().nth(line).unwrap_or("<end>")
        );
    }
}

#[test]
fn the_cases_reach_the_front_ends_paths() {
    let e = data(include_str!("a1_cases/expected.txt"));
    for shape in ["undecodedcall", "'multiset'", "AddStep(", "NewScript(", "UD(", "rules 1\n", "registration_errors []", "classify 'partial'", "classify 'complete'", "loop at", "unbalanced stack", "takes arguments"] {
        assert!(e.contains(shape), "no case reaches {shape}");
    }
}

#[test]
fn expected_cases_are_current() {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("sta_a1_cases_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out_path = dir.join("expected.txt");
    let status = Command::new("python3")
        .arg("-I")
        .arg(here.join("tests/a1_cases/gen_cases.py"))
        .arg(here.join("../../tools"))
        .arg(here.join("tests/a1_cases/cases.txt"))
        .arg(&out_path)
        .status();
    let Ok(status) = status else {
        eprintln!("python3 not found: skipped");
        return;
    };
    assert!(status.success(), "the Python driver failed");
    let fresh = std::fs::read_to_string(&out_path).expect("driver output");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(data(&fresh), data(include_str!("a1_cases/expected.txt")), "tests/a1_cases/expected.txt is stale: run gen_cases.py again");
}

#[test]
fn norm_hides_addresses_and_slot_counts() {
    use sta_mission_convert::a1::mission::norm;
    assert_eq!(norm("call at 0x10002f00 to a helper"), "call at # to a helper");
    assert_eq!(norm("unbalanced stack at a return (3 slots) at 0x1"), "unbalanced stack at a return (n slots) at #");
    assert_eq!(norm("00x1f and 0xg"), "0# and 0xg");
}

#[test]
fn unmapped_pairs_follow_only_on_request() {
    let root = std::env::temp_dir().join(format!("sta_a1_find_{}", std::process::id()));
    std::fs::create_dir_all(root.join("missions")).expect("dirs");
    std::fs::create_dir_all(root.join("bzn")).expect("dirs");
    for stem in ["One", "Two"] {
        std::fs::write(root.join("missions").join(format!("{stem}R.drl")), b"x").expect("file");
        std::fs::write(root.join("missions").join(format!("{stem}S.dsl")), b"x").expect("file");
    }
    std::fs::write(root.join("bzn/map_a.bzn"), b"\0onER.drl\0OneR.drl\0OneS.dsl\0").expect("file");
    let mapped = find_missions(&root, false).expect("a listing");
    let every = find_missions(&root, true).expect("a listing");
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(mapped.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), ["map_a"]);
    assert_eq!(every.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), ["map_a", "two"]);
    assert_eq!(every[1].2.file_name().expect("a name").to_string_lossy(), "TwoR.drl");
}
