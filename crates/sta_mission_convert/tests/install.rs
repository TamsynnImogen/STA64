//! Install-gated: the mission DLLs of a player's install are opened read-only and only structural
//! facts are asserted (they parse, the counts are not zero). Nothing about their content is
//! checked or printed. The install is named by `STA64_TEST_A2_STOCK_INSTALL` (Armada II) and
//! `STA64_TEST_A1_INSTALL` (Armada I); a test without its variable returns at once.

use std::path::{Path, PathBuf};

use sta_mission_convert::map::MapFile;
use sta_mission_convert::{Image, Pe};

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                walk(&p, depth - 1, out);
            }
        } else {
            out.push(p);
        }
    }
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// Every `.dsl` and `.drl` under the install, and how many parse.
fn check_install(var: &str) -> Option<PathBuf> {
    let Ok(root) = std::env::var(var) else {
        eprintln!("{var} not set: skipped");
        return None;
    };
    let root = PathBuf::from(root);
    let mut files = Vec::new();
    walk(&root, 3, &mut files);
    let dlls: Vec<&PathBuf> = files
        .iter()
        .filter(|p| has_ext(p, "dsl") || has_ext(p, "drl"))
        .collect();
    assert!(!dlls.is_empty(), "no mission DLL under the install");
    let (mut with_imports, mut with_exports) = (0, 0);
    for p in &dlls {
        let pe = Pe::open(p).unwrap_or_else(|e| panic!("a mission DLL does not parse: {e}"));
        assert!(!pe.sections.is_empty());
        assert!(pe.sections.iter().any(|s| s.is_code()), "no code section");
        assert!(pe.entry_point() > pe.base);
        assert!(pe.off(pe.sections[0].va).is_some());
        with_imports += usize::from(!pe.imports.is_empty());
        with_exports += usize::from(
            !pe.exports()
                .unwrap_or_else(|e| panic!("export table: {e}"))
                .is_empty(),
        );
    }
    assert!(with_imports > 0, "no DLL with imports");
    assert!(with_exports > 0, "no DLL with exports");
    eprintln!("{var}: {} DLLs parse", dlls.len());
    Some(root)
}

#[test]
fn a2_mission_dlls_parse() {
    let Some(root) = check_install("STA64_TEST_A2_STOCK_INSTALL") else {
        return;
    };
    // The engine executable and its map, when the install has a pair: both parse, the map is not empty.
    let mut files = Vec::new();
    walk(&root, 0, &mut files);
    for map in files.iter().filter(|p| has_ext(p, "map")) {
        let Some(exe) = files
            .iter()
            .find(|p| has_ext(p, "exe") && p.file_stem() == map.file_stem())
        else {
            continue;
        };
        let pe = Pe::open(exe).expect("the executable parses");
        assert!(pe.sections.iter().any(|s| s.name.starts_with(".text")));
        assert!(!MapFile::open(map).expect("the map reads").names.is_empty());
    }
}

#[test]
fn a1_mission_dlls_parse() {
    check_install("STA64_TEST_A1_INSTALL");
}
