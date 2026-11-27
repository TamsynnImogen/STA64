//! Tests of the mission converter (LUA-15h): the call handling, the worklist, the renderer and
//! the file emission of `tools/dsl_to_lua.py`.
//!
//! - `gated_missions_equal_the_python_output`: install-gated (`STA64_TEST_A2_STOCK_INSTALL`, the
//!   parity harness' reference with its objdump listings): every mission DLL of the install
//!   converts to the file the Python tool wrote, but for the tool token of the first line.

#[path = "parity/harness.rs"]
mod harness;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sta_mission_convert::convert::{convert_install, load_engine};
use sta_mission_convert::pyval::{py_isspace, py_strip};

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
