//! The lifter core (LUA-15d): `Dsl` and `Lifter` against authored listings and flat images.
//!
//! The cases are `tests/dsl_lift/cases.txt` (invented listings, no game data). Their expected
//! output, `tests/dsl_lift/expected.txt`, is what the ORIGINAL `tools/dsl_to_lua.py` prints for
//! the same cases (`tests/dsl_lift/gen_pairs.py`, which reads no game files): the Rust output
//! must equal it, case by case. `expected_pairs_are_current` runs the Python again (when
//! `python3` is on the path) and checks the committed pairs are what it prints now.
//!
//! The install-gated parity test over every installed Armada II mission DLL is
//! `tests/dsl_lift_parity.rs`.

use std::collections::HashMap;

use indexmap::IndexMap;
use sta_mission_convert::disasm::Ins;
use sta_mission_convert::dsl::Dsl;
use sta_mission_convert::lift::{d_immediate, Lifter, State};
use sta_mission_convert::map::Vtables;
use sta_mission_convert::pyval::Py;
use sta_mission_convert::{tup, ErrorKind, FlatImage};

#[path = "dsl_lift/common.rs"]
mod common;
mod staleness;

const RAW_CASES: &str = include_str!("dsl_lift/cases.txt");
const RAW_EXPECTED: &str = include_str!("dsl_lift/expected.txt");

/// The committed data without `\r`: a checkout may convert line endings (Windows), the Python
/// driver writes `\n` only.
fn cases_text() -> String {
    RAW_CASES.replace('\r', "")
}

fn expected_text() -> String {
    RAW_EXPECTED.replace('\r', "")
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

enum Put {
    Bytes(Vec<u8>),
    /// 32-bit words: numbers or `@label` addresses.
    Words(Vec<String>),
}

#[derive(Default)]
struct Case {
    name: String,
    base: u32,
    size: usize,
    sections: Vec<(String, u32, u32)>,
    puts: Vec<(u32, Put)>,
    exports: Vec<(String, u32)>,
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
    let mut in_case = false;
    for raw in text.split('\n') {
        let line = raw.trim_end_matches('\r');
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
            in_case = true;
            continue;
        }
        if word == "end" {
            in_case = false;
            continue;
        }
        assert!(in_case, "directive outside a case: {word}");
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
            "base" => cur.base = num(rest) as u32,
            "size" => cur.size = num(rest) as usize,
            "section" => {
                let p: Vec<&str> = rest.split(' ').collect();
                cur.sections.push((p[0].to_owned(), num(p[1]) as u32, num(p[2]) as u32));
            }
            "put" => {
                let (a, hexes) = rest.split_once(' ').expect("put <addr> <bytes>");
                let bytes = hexes.split_whitespace().map(|x| u8::from_str_radix(x, 16).expect("hex byte")).collect();
                cur.puts.push((num(a) as u32, Put::Bytes(bytes)));
            }
            "dwords" => {
                let p: Vec<&str> = rest.split(' ').collect();
                cur.puts.push((num(p[0]) as u32, Put::Words(p[1..].iter().map(|s| (*s).to_owned()).collect())));
            }
            "cstr" => {
                let (a, s) = rest.split_once(' ').unwrap_or((rest, ""));
                let s = s.replace("\\n", "\n").replace("\\t", "\t");
                let mut bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
                bytes.push(0);
                cur.puts.push((num(a) as u32, Put::Bytes(bytes)));
            }
            "stdexports" => cur.exports.extend(STD_EXPORTS.iter().map(|(n, a)| ((*n).to_owned(), *a))),
            "export" => {
                let (sym, a) = rest.rsplit_once(' ').expect("export <symbol> <addr>");
                cur.exports.push((sym.to_owned(), num(a) as u32));
            }
            "code" => cur.blocks.push((num(rest) as u32, Vec::new())),
            "run" => cur.runs.push(rest.to_owned()),
            other => panic!("cases.txt: unknown directive {other:?}"),
        }
    }
    cases
}

/// The listing: instructions 2 bytes apart from each `code` address, labels resolved; and the
/// labels.
fn listing(case: &Case) -> (Vec<Ins>, HashMap<String, u32>) {
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
    let ins = items
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
        .collect();
    (ins, labels)
}

fn run_case(case: &Case) -> String {
    let mut img = FlatImage::new(case.base, case.size);
    for (name, va, size) in &case.sections {
        img = img.with_section(name, *va, *size);
    }
    let (ins, labels) = listing(case);
    for (a, p) in &case.puts {
        match p {
            Put::Bytes(b) => img.put(*a, b),
            Put::Words(w) => {
                let bytes: Vec<u8> = w
                    .iter()
                    .flat_map(|x| match x.strip_prefix('@') {
                        Some(l) => labels[l].to_le_bytes(),
                        None => (num(x) as u32).to_le_bytes(),
                    })
                    .collect();
                img.put(*a, &bytes);
            }
        }
    }
    let mut exports: IndexMap<String, u32> = IndexMap::new();
    for (n, a) in &case.exports {
        exports.insert(n.clone(), *a);
    }
    let mut out = format!("== {}\n", case.name);
    let d = match Dsl::new(&img, Vtables::new(), ins, exports) {
        Ok(d) => d,
        Err(e) => {
            assert_eq!(e.kind, ErrorKind::Decode);
            out.push_str(&format!("new error {}\n", Py::from(e.message).repr()));
            return out;
        }
    };
    let mut lf = Lifter::new(&d);
    for r in &case.runs {
        let (what, arg) = r.split_once(' ').unwrap_or((r.as_str(), ""));
        out.push_str(&format!("-- {r}\n"));
        let res: Result<String, sta_mission_convert::DecodeError> = (|| {
            Ok(match what {
                "lift" => lf.lift(num(arg))?.canonical().trim_end_matches('\n').to_owned(),
                "blocks" => {
                    let (leaders, seen) = lf.blocks(num(arg))?;
                    let mut seen: Vec<i64> = seen.into_iter().collect();
                    seen.sort_unstable();
                    let ints = |v: &[i64]| Py::List(v.iter().map(|x| Py::Int(*x)).collect()).repr();
                    let sw: Vec<Py> = lf
                        .switches
                        .iter()
                        .map(|(k, v)| Py::Tuple(vec![Py::Int(*k), Py::List(v.iter().map(|x| Py::Int(*x)).collect())]))
                        .collect();
                    let bi: Vec<Py> = lf
                        .byte_index
                        .iter()
                        .map(|(k, (a, r))| Py::Tuple(vec![Py::Int(*k), tup![*a, r.as_str()]]))
                        .collect();
                    format!(
                        "leaders {}\nseen {}\nswitches {}\nbyte_index {}",
                        ints(&leaders),
                        ints(&seen),
                        Py::List(sw).repr(),
                        Py::List(bi).repr()
                    )
                }
                "facts" => common::kind_facts(&d, num(arg))?,
                "dsl" => common::dsl_facts(&d).join("\n"),
                "static" => Py::from(d.static_class(num(arg))).repr(),
                "statics" => String::new(),
                "dimm" => d_immediate(&d, num(arg))?.repr(),
                "global_at" => match d.global_at(num(arg)) {
                    Some((n, k, o)) => tup![n, k, o].repr(),
                    None => "None".to_owned(),
                },
                "entry" => d.entry(num(arg))?.to_string(),
                "dword" => d.dword(num(arg)).map_or("None".to_owned(), |v| v.to_string()),
                "take" => {
                    let (n, k) = arg.split_once(' ').expect("take <n> <count>");
                    let mut st = State::new();
                    st.args = (0..num(k)).map(|i| tup!["const", i]).collect();
                    st.sub_esp = true;
                    let res = lf.take(&mut st, num(n), 0x1000)?;
                    tup![Py::List(res), Py::List(st.args.clone()), st.sub_esp].repr()
                }
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

/// Parsed `expected.txt` cases by name: `(name, text)` in file order.
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
    let cases = parse(&cases_text());
    assert!(cases.len() >= 20, "{} cases", cases.len());
    let expected = split_cases(&expected_text());
    assert_eq!(expected.len(), cases.len(), "cases and expected.txt differ in number");
    let mut bad = Vec::new();
    for (c, (name, want)) in cases.iter().zip(&expected) {
        assert_eq!(&c.name, name, "case order differs from expected.txt");
        let got = run_case(c);
        if &got != want {
            // the first differing line, for the message
            let line = got.lines().zip(want.lines()).position(|(a, b)| a != b).unwrap_or(got.lines().count().min(want.lines().count()));
            let (g, w) = (got.lines().nth(line).unwrap_or("<end>"), want.lines().nth(line).unwrap_or("<end>"));
            bad.push(format!("case {name}, line {}:\n   rust:   {g}\n   python: {w}", line + 1));
        }
    }
    assert!(bad.is_empty(), "{} case(s) differ:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn the_cases_exercise_a_lifted_function_per_form() {
    // A guard on the authored data: the expected output has functions that lifted and functions
    // that stopped with a decode error, so neither path went untested by a typo in a case.
    let expected = expected_text();
    let ok = expected.matches(" error None").count();
    let stopped = expected.lines().filter(|l| l.starts_with(" error '")).count();
    assert!(ok >= 40, "{ok} functions lifted");
    assert!(stopped >= 25, "{stopped} functions stopped");
}

#[test]
fn expected_pairs_are_current() {
    // some expected lines carry CPython exception texts: compared under the pinned CPython only
    if !staleness::pinned_python_available("expected_pairs_are_current") {
        return;
    }
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("sta_lift_pairs_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out_path = dir.join("expected.txt");
    let status = Command::new("python3")
        .arg("-I")
        .arg(here.join("tests/dsl_lift/gen_pairs.py"))
        .arg(here.join("../../tools"))
        .arg(here.join("tests/dsl_lift/cases.txt"))
        .arg(&out_path)
        .status();
    let Ok(status) = status else {
        eprintln!("python3 not found: skipped");
        return;
    };
    assert!(status.success(), "the Python driver failed");
    let fresh = std::fs::read_to_string(&out_path).expect("driver output");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(fresh.replace('\r', ""), expected_text(), "tests/dsl_lift/expected.txt is stale: run gen_pairs.py again");
}
