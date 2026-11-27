//! Tests of the mission converter (LUA-15h): the call handling, the worklist, the renderer and
//! the file emission of `tools/dsl_to_lua.py`.
//!
//! - `gated_missions_equal_the_python_output`: install-gated (`STA64_TEST_A2_STOCK_INSTALL`, the
//!   parity harness' reference with its objdump listings): every mission DLL of the install
//!   converts to the file the Python tool wrote, but for the tool token of the first line.

#[path = "parity/harness.rs"]
mod harness;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use sta_mission_convert::call::mission_lifter;
use sta_mission_convert::convert::{convert_install, emit_mission, lift_mission, load_engine};
use sta_mission_convert::disasm::Ins;
use sta_mission_convert::dsl::Dsl;
use sta_mission_convert::emit::{render_function, Namer};
use sta_mission_convert::map::Vtables;
use sta_mission_convert::prelude::{ENGINE, PRELUDE};
use sta_mission_convert::pyval::{py_isspace, py_strip, Py};
use sta_mission_convert::{ErrorKind, FlatImage};

#[path = "dsl_lift/common.rs"]
mod common;

// ------------------------------------------------------------------ authored cases

/// The committed data with its line endings normalised (a checkout may have rewritten them).
fn data(s: &str) -> String {
    s.replace('\r', "")
}

const STD_EXPORTS: [(&str, u32); 10] = [
    ("?g_pSI@@3PAVScriptInterface@@A", 0x1000_2800),
    ("?p_WORLD@@3PAVcharacter@@A", 0x1000_2804),
    ("?p_ALPHA@@3PAVhot_int@@A", 0x1000_2808),
    ("?p_BETA@@3PAVhot_float@@A", 0x1000_280C),
    ("?cnt@@3HA", 0x1000_2810),
    ("?flt@@3MA", 0x1000_2814),
    ("?arr@@3PAHA", 0x1000_2818),
    ("?farr@@3PAMA", 0x1000_2828),
    ("?vec@@3VVector3@@A", 0x1000_2838),
    ("?flag@@3_NA", 0x1000_2844),
];

const PROLOGUE: [&str; 10] = [
    "push   ebp",
    "mov    ebp,esp",
    "sub    esp,0x44",
    "push   ebx",
    "push   esi",
    "push   edi",
    "lea    edi,[ebp-0x44]",
    "mov    ecx,0x11",
    "mov    eax,0xcccccccc",
    "rep stos DWORD PTR es:[edi],eax",
];
const EPILOGUE: [&str; 6] = ["pop    edi", "pop    esi", "pop    ebx", "mov    esp,ebp", "pop    ebp", "ret"];

#[derive(Default)]
struct Case {
    name: String,
    base: u32,
    size: usize,
    sections: Vec<(String, u32, u32)>,
    puts: Vec<(u32, Vec<u8>)>,
    exports: Vec<(String, u32)>,
    engine: Vec<(String, Vec<String>)>,
    blocks: Vec<(u32, Vec<String>)>,
    runs: Vec<String>,
}

fn num(s: &str) -> i64 {
    match s.strip_prefix("0x") {
        Some(h) => i64::from_str_radix(h, 16).expect("hex number"),
        None => s.parse().expect("decimal number"),
    }
}

fn parse(text: &str) -> Vec<Case> {
    let mut cases: Vec<Case> = Vec::new();
    for line in text.split('\n') {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(t) = line.strip_prefix("  ") {
            cases.last_mut().expect("a case").blocks.last_mut().expect("a code block").1.push(t.to_owned());
            continue;
        }
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        if word == "case" {
            cases.push(Case { name: rest.to_owned(), ..Case::default() });
            continue;
        }
        if word == "end" {
            continue;
        }
        let cur = cases.last_mut().expect("a case");
        match word {
            "std" => {
                cur.base = 0x1000_0000;
                cur.size = 0x4000;
                cur.sections = vec![
                    (".text".into(), 0x1000, 0x1000),
                    (".rdata".into(), 0x2000, 0x400),
                    (".data".into(), 0x2400, 0x400),
                    (".bss".into(), 0x2800, 0x400),
                ];
            }
            "cstr" => {
                let (a, s) = rest.split_once(' ').unwrap_or((rest, ""));
                let mut bytes: Vec<u8> = s.replace("\\n", "\n").replace("\\t", "\t").chars().map(|c| c as u8).collect();
                bytes.push(0);
                cur.puts.push((num(a) as u32, bytes));
            }
            "stdexports" => cur.exports.extend(STD_EXPORTS.iter().map(|(n, a)| ((*n).to_owned(), *a))),
            "export" => {
                let (sym, a) = rest.rsplit_once(' ').expect("export <symbol> <addr>");
                cur.exports.push((sym.to_owned(), num(a) as u32));
            }
            "engine" => {
                let p: Vec<&str> = rest.split(' ').collect();
                cur.engine.push((p[0].to_owned(), p[1..].iter().map(|s| (*s).to_owned()).collect()));
            }
            "code" => cur.blocks.push((num(rest) as u32, Vec::new())),
            "run" => cur.runs.push(rest.to_owned()),
            other => panic!("cases.txt: unknown directive {other:?}"),
        }
    }
    cases
}

/// The listing: instructions 2 bytes apart from each `code` address, labels resolved.
fn listing(case: &Case) -> Vec<Ins> {
    let mut labels: HashMap<String, u32> = HashMap::new();
    let mut items: Vec<(u32, String)> = Vec::new();
    for (start, lines) in &case.blocks {
        let mut a = *start;
        let mut expanded: Vec<String> = Vec::new();
        for ln in lines {
            match ln.as_str() {
                "!p" => expanded.extend(PROLOGUE.iter().map(|s| (*s).to_owned())),
                "!e" => expanded.extend(EPILOGUE.iter().map(|s| (*s).to_owned())),
                _ => expanded.push(ln.clone()),
            }
        }
        for ln in expanded {
            if ln.starts_with('@') && ln.ends_with(':') {
                labels.insert(ln[1..ln.len() - 1].to_owned(), a);
            } else {
                items.push((a, ln));
                a += 2;
            }
        }
    }
    items
        .into_iter()
        .map(|(a, t)| {
            let mut out = String::new();
            let cs: Vec<char> = t.chars().collect();
            let mut i = 0;
            while i < cs.len() {
                if cs[i] == '@' {
                    let mut j = i + 1;
                    while j < cs.len() && (cs[j].is_alphanumeric() || cs[j] == '_') {
                        j += 1;
                    }
                    let name: String = cs[i + 1..j].iter().collect();
                    out.push_str(&format!("{:#x}", labels[&name]));
                    i = j;
                } else {
                    out.push(cs[i]);
                    i += 1;
                }
            }
            (a, out)
        })
        .collect()
}

fn run_case(case: &Case) -> String {
    let mut img = FlatImage::new(case.base, case.size);
    for (name, va, size) in &case.sections {
        img = img.with_section(name, *va, *size);
    }
    for (a, b) in &case.puts {
        img.put(*a, b);
    }
    let ins = listing(case);
    let mut exports: IndexMap<String, u32> = IndexMap::new();
    for (n, a) in &case.exports {
        exports.insert(n.clone(), *a);
    }
    let mut engine = Vtables::new();
    for (c, v) in &case.engine {
        engine.insert(c.clone(), v.clone());
    }
    let mut out = format!("== {}\n", case.name);
    let d = match Dsl::new(&img, engine.clone(), ins, exports) {
        Ok(d) => d,
        Err(e) => {
            assert_eq!(e.kind, ErrorKind::Decode);
            out.push_str(&format!("new error {}\n", Py::from(e.message).repr()));
            return out;
        }
    };
    let mut lf = mission_lifter(&d);
    for r in &case.runs {
        let (what, arg) = r.split_once(' ').unwrap_or((r.as_str(), ""));
        out.push_str(&format!("-- {r}\n"));
        let res: Result<String, sta_mission_convert::DecodeError> = (|| {
            Ok(match what {
                "lift" => lf.lift(num(arg))?.canonical().trim_end_matches('\n').to_owned(),
                "render" => {
                    let f = lf.lift(num(arg))?;
                    let funcs: IndexMap<i64, _> = IndexMap::from([(f.addr, f.clone())]);
                    let roles: HashMap<i64, String> = HashMap::from([(f.addr, "helper".to_owned())]);
                    let nm = Namer::new(&d, &funcs, &roles);
                    render_function(&f, &nm, &nm.fn_name(f.addr))?.join("\n")
                }
                "convert" => {
                    let (mut funcs, roles) = lift_mission(&d)?;
                    let (src, _) = emit_mission(&d, &mut funcs, &roles, None, "m", "-- h")?;
                    let body = src.split_once(PRELUDE).expect("the prelude").1;
                    let body = body.split_once(ENGINE).expect("the engine block").0;
                    body.trim_end_matches('\n').to_owned()
                }
                "statics" => String::new(),
                other => panic!("cases.txt: unknown run {other:?}"),
            })
        })();
        match res {
            Ok(s) if what == "statics" => {
                assert!(s.is_empty());
                out.push_str(&common::static_refs_line(&d));
                out.push('\n');
            }
            Ok(s) => {
                out.push_str(&s);
                out.push('\n');
            }
            Err(e) if e.kind == ErrorKind::Raw => {
                out.push_str(&format!("raw {} {}\n", e.class.unwrap_or("?"), Py::from(e.message).repr()));
                break;
            }
            Err(e) => out.push_str(&format!("error {}\n", Py::from(e.message).repr())),
        }
    }
    out.push_str(&common::static_refs_line(&d));
    out.push('\n');
    out
}

fn split_cases(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for l in text.split_inclusive('\n') {
        if let Some(n) = l.strip_prefix("== ") {
            out.push((n.trim_end().to_owned(), String::new()));
        }
        out.last_mut().expect("starts with a case header").1.push_str(l);
    }
    out
}

#[test]
fn every_case_equals_the_originals_output() {
    let cases = parse(&data(include_str!("dsl_cases/cases.txt")));
    let expected = split_cases(&data(include_str!("dsl_cases/expected.txt")));
    assert!(cases.len() >= 5, "{} cases", cases.len());
    assert_eq!(expected.len(), cases.len(), "cases and expected.txt differ in number");
    let mut bad = Vec::new();
    for (c, (name, want)) in cases.iter().zip(&expected) {
        assert_eq!(&c.name, name, "case order differs from expected.txt");
        let got = run_case(c);
        if &got != want {
            let line = got.lines().zip(want.lines()).position(|(a, b)| a != b).unwrap_or(got.lines().count().min(want.lines().count()));
            let (g, w) = (got.lines().nth(line).unwrap_or("<end>"), want.lines().nth(line).unwrap_or("<end>"));
            bad.push(format!("case {name}, line {}:\n   rust:   {g}\n   python: {w}", line + 1));
        }
    }
    assert!(bad.is_empty(), "{} case(s) differ:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn the_cases_reach_every_call_path() {
    // A guard on the authored data: the expected output holds the shapes of each call branch, so
    // a typo in a case cannot silently drop one.
    let e = data(include_str!("dsl_cases/expected.txt"));
    for shape in [
        "'newscript'", "'addstep'", "'trashrule'", "'callout'", "'mcall'", "'vop'", "'newvar'", "'enginevar'",
        "'varq'", "'boolarg'", "'vec'", "'fbits'", "'sprintf'", "'strcmp'", "'objref'", "'comment'", "'field'",
    ] {
        assert!(e.contains(shape), "no case reaches {shape}");
    }
    let stopped = e.lines().filter(|l| l.starts_with(" error ") && !l.starts_with(" error None")).count();
    assert!(stopped >= 10, "{stopped} functions stopped");
}

#[test]
fn expected_cases_are_current() {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("sta_dsl_cases_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out_path = dir.join("expected.txt");
    let status = Command::new("python3")
        .arg("-I")
        .arg(here.join("tests/dsl_cases/gen_cases.py"))
        .arg(here.join("../../tools"))
        .arg(here.join("tests/dsl_cases/cases.txt"))
        .arg(&out_path)
        .status();
    let Ok(status) = status else {
        eprintln!("python3 not found: skipped");
        return;
    };
    assert!(status.success(), "the Python driver failed");
    let fresh = std::fs::read_to_string(&out_path).expect("driver output");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(data(&fresh), data(include_str!("dsl_cases/expected.txt")), "tests/dsl_cases/expected.txt is stale: run gen_cases.py again");
}

#[test]
fn mission_names_and_dll_name_scans() {
    use sta_mission_convert::convert::mission_name;
    assert_eq!(mission_name("a2_xS.dsl"), "a2_x");
    assert_eq!(mission_name("a2_xs.dsl"), "a2_x");
    assert_eq!(mission_name("a2_x.dsl"), "a2_x.dsl");
    assert_eq!(mission_name("a2_xS.DSL"), "a2_xS.DSL");
}

#[test]
fn map_stems_reads_the_dll_names_of_the_bzn_folders() {
    use sta_mission_convert::convert::map_stems;
    let root = std::env::temp_dir().join(format!("sta_stems_{}", std::process::id()));
    let bzn = root.join("Bzn");
    std::fs::create_dir_all(bzn.join("deeper")).expect("dirs");
    std::fs::write(bzn.join("b_map.BZN"), b"xx one_missionS.dsl yy OTHER_s.DSL zz one_missionS.dsl").expect("file");
    std::fs::write(bzn.join("a_map.bzn"), b"\x00one_missionS.dsl\x00").expect("file");
    std::fs::write(bzn.join("note.txt"), b"third_missionS.dsl").expect("file");
    std::fs::write(bzn.join("deeper").join("c.bzn"), b"deepS.dsl").expect("file");
    let stems = map_stems(std::slice::from_ref(&root));
    let _ = std::fs::remove_dir_all(&root);
    let got: Vec<(String, Vec<String>)> = stems.into_iter().collect();
    // files in code-point order (`a_map` first), each stem once per mission, names lower-cased
    assert_eq!(
        got,
        [
            ("one_mission".to_owned(), vec!["a_map".to_owned(), "b_map".to_owned()]),
            ("other_".to_owned(), vec!["b_map".to_owned()]),
        ]
    );
}

#[test]
fn find_missions_pairs_a_rule_dll_by_lower_case_name_and_sorts_by_path() {
    use sta_mission_convert::convert::find_missions;
    let root = std::env::temp_dir().join(format!("sta_find_{}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).expect("dirs");
    for f in ["bS.dsl", "bR.drl", "AS.DSL", "sub/cS.dsl", "sub/cr.drl", "x.txt"] {
        std::fs::write(root.join(f), b"").expect("file");
    }
    let found = find_missions(&root);
    let _ = std::fs::remove_dir_all(&root);
    let names: Vec<(String, Option<String>)> = found
        .iter()
        .map(|(a, b)| {
            let rel = |p: &Path| p.strip_prefix(&root).map(|r| r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")).unwrap_or_default();
            (rel(a), b.as_deref().map(rel))
        })
        .collect();
    assert_eq!(
        names,
        [
            ("AS.DSL".to_owned(), None),
            ("bS.dsl".to_owned(), Some("bR.drl".to_owned())),
            ("sub/cS.dsl".to_owned(), Some("sub/cr.drl".to_owned())),
        ]
    );
}

// ------------------------------------------------------------------ gated parity

/// The folder `d/name` for a folder spelled in any case: the first of several in code-point order
/// (`child` of `convert_install_missions.py`).
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
        let Ok(addr) = u32::from_str_radix(a, 16) else { continue };
        out.push((addr, py_strip(t).to_owned()));
    }
    out
}

/// Gated. Every mission of the Armada II install converts to the Python tool's file, but for the
/// tool token of the first line; no file is missing or extra (so the missions the Rust code fails
/// on are the ones the reference failed on). Listings are the reference's own `objdump` listings.
#[test]
fn gated_missions_equal_the_python_output() {
    let Some(reference) = harness::obtain_or_skip() else {
        return;
    };
    let Some(install) = reference.install(harness::Kind::Armada2) else {
        eprintln!("the reference has no Armada II install: skipped");
        return;
    };
    let Some(root) = std::env::var_os(harness::A2_VAR).map(PathBuf::from).filter(|p| p.is_dir()) else {
        eprintln!("{} is not set: skipped (the mission DLLs are read from the install)", harness::A2_VAR);
        return;
    };
    if !install.has_listings() {
        eprintln!("the reference has no objdump listings: skipped");
        return;
    }
    let mdir = child(&root, "missions")
        .or_else(|| child(&child(&root, "Data").unwrap_or_else(|| root.clone()), "missions"))
        .expect("a missions folder");
    // the reference converted `--install <missions> --install <bzn next to it>`
    let mut roots = vec![mdir.clone()];
    if let Some(b) = mdir.parent().and_then(|p| child(p, "bzn")) {
        roots.push(b);
    }
    let engine = load_engine(&roots, Some(&root)).expect("the engine lookup");
    let mut listing = |p: &Path| -> sta_mission_convert::Result<Vec<(u32, String)>> {
        let name = format!("{}.txt", p.file_name().unwrap().to_string_lossy().to_lowercase());
        let bytes = install
            .listing(&name)
            .ok_or_else(|| sta_mission_convert::DecodeError::raw("a mission DLL has no listing in the reference"))?;
        Ok(parse_objdump(&String::from_utf8_lossy(&bytes)))
    };
    let run = convert_install(&roots, &engine, &[], &mut listing).expect("the run does not abort");
    let produced: BTreeMap<String, Vec<u8>> = run.files.iter().map(|(k, v)| (format!("missions/{k}"), v.clone())).collect();
    let report = install.compare_subset(&produced, |rel| rel.starts_with("missions/"));
    eprintln!(
        "mission conversion: {} DLLs converted, {} failed; {} files written, {} equal, {} differing, {} missing, {} extra",
        run.converted.len(),
        run.failed.len(),
        run.files.len(),
        report.equal,
        report.differing.len(),
        report.missing.len(),
        report.extra.len(),
    );
    // What the equal files covered (printed at run time only).
    let (mut total, mut decoded, mut rules, mut conds) = (0usize, 0usize, 0usize, 0usize);
    for r in &run.converted {
        for (t, d) in r.functions.values() {
            total += t;
            decoded += d;
        }
        rules += r.rules;
        conds += r.rule_conditions_decoded;
    }
    eprintln!("  coverage: {decoded} of {total} functions decoded, {conds} of {rules} rule conditions decoded");
    assert!(report.equal > 0, "nothing was compared");
    assert!(decoded * 10 >= total * 9, "implausibly many functions were not decoded");
    report.assert_ok("a2-missions");
}
