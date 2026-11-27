//! Install-gated (LUA-15e): every Armada I mission of the installed game converts to the file the
//! Python tool (`tools/a1_to_lua.py`) wrote, but for the tool token of the first line; no file is
//! missing or extra, so the set of missions that fail equals the reference's.
//!
//! Needs `STA64_TEST_A1_INSTALL` (read only) and the parity harness' reference with its objdump
//! listings (see `tests/parity/harness.rs`); skipped with a message otherwise. The unmapped
//! pairs are converted when the reference was made with `--include-unmapped`.

#[path = "parity/harness.rs"]
mod harness;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sta_mission_convert::a1::convert_install;
use sta_mission_convert::pyval::{py_isspace, py_strip};

/// `drl_to_lua.disassemble`'s parse of the objdump text.
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

#[test]
fn gated_a1_missions_equal_the_python_output() {
    let Some(reference) = harness::obtain_or_skip() else {
        return;
    };
    let Some(install) = reference.install(harness::Kind::Armada1) else {
        eprintln!("the reference has no Armada I install: skipped");
        return;
    };
    let Some(root) = std::env::var_os(harness::A1_VAR).map(PathBuf::from).filter(|p| p.is_dir()) else {
        eprintln!("{} is not set: skipped", harness::A1_VAR);
        return;
    };
    if !install.has_listings() {
        eprintln!("the reference has no objdump listings: skipped");
        return;
    }
    let mut listing = |p: &Path| -> sta_mission_convert::Result<Vec<(u32, String)>> {
        let name = format!("{}.txt", p.file_name().unwrap().to_string_lossy().to_lowercase());
        let bytes = install
            .listing(&name)
            .ok_or_else(|| sta_mission_convert::DecodeError::raw("a mission DLL has no listing in the reference"))?;
        Ok(parse_objdump(&String::from_utf8_lossy(&bytes)))
    };
    let run = convert_install(&root, install.include_unmapped(), &[], &mut listing).expect("the run does not abort");
    let produced: BTreeMap<String, Vec<u8>> = run.files.clone();
    let report = install.compare_subset(&produced, |rel| rel.starts_with("missions/"));
    eprintln!(
        "a1 conversion: {} converted, {} failed; {} files written, {} equal, {} differing, {} missing, {} extra",
        run.reports.len(),
        run.failed.len(),
        run.files.len(),
        report.equal,
        report.differing.len(),
        report.missing.len(),
        report.extra.len(),
    );
    // What the equal files covered (printed at run time only).
    let (mut total, mut complete, mut partial) = (0i64, 0i64, 0i64);
    for r in &run.reports {
        for v in r.functions.values() {
            total += v[0];
            complete += v[1];
            partial += v[2];
        }
    }
    eprintln!("  coverage: {complete} complete and {partial} partial of {total} functions");
    assert!(report.equal > 0, "nothing was compared");
    report.assert_ok("a1-missions");
}
