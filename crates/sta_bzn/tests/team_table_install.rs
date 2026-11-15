//! The team table of the installed campaign maps: every mission map an install's `mshell.set` lists (lines
//! not commented out) yields a table of eight seats for the human seat. Structure and general rules only,
//! no stock values. Install-gated: set `STA64_TEST_A2_STOCK_INSTALL` and/or `STA64_TEST_ROOTS_INSTALL`;
//! skipped when none is set. When a variable is set, its install must be readable and at least one listed
//! map must be checked: an unreadable `mshell.set` or map fails the test. Evidence grade: read from data.
//!
//! No range is asserted on the stock values: real maps carry negative crew and very large values on some
//! seats (a consumer clamps them), so only finiteness and a non-negative officer limit are checked.

use std::path::{Path, PathBuf};

fn data_dir(root: &Path) -> PathBuf {
    if root.join("Data").is_dir() { root.join("Data") } else { root.to_path_buf() }
}

/// The file `name` of `dir`, matched case-insensitively (a script's list does not always match the file name's case).
fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name)))
}

#[test]
fn every_listed_mission_map_has_a_team_table_for_the_human_seat() {
    let roots: Vec<PathBuf> = ["STA64_TEST_A2_STOCK_INSTALL", "STA64_TEST_ROOTS_INSTALL"].iter().filter_map(std::env::var_os).map(PathBuf::from).collect();
    if roots.is_empty() {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL / STA64_TEST_ROOTS_INSTALL");
        return;
    }
    let mut problems = Vec::new();
    for root in roots {
        let data = data_dir(&root);
        let set = std::fs::read_to_string(data.join("mshell.set")).unwrap_or_else(|e| panic!("{}: mshell.set unreadable: {e}", root.display()));
        let mut checked = 0;
        for line in set.lines().map(str::trim).filter(|l| l.to_ascii_lowercase().ends_with(".bzn") && !l.starts_with("//")) {
            let path = find_ci(&data.join("bzn"), line).unwrap_or_else(|| panic!("{}: {line} not found", root.display()));
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {line} unreadable: {e}", root.display()));
            checked += 1;
            match sta_bzn::team_table(&bytes) {
                sta_bzn::TeamTable::Found(t) if t.len() == 8 => {
                    for (n, s) in t.iter().enumerate() {
                        let floats = [s.dilithium, s.latinum, s.metal, s.biomatter, s.crew];
                        if !floats.iter().all(|f| f.is_finite()) || s.officers < 0 {
                            problems.push(format!("{}: {line}: block {n}: non-finite float or negative officer limit", root.display()));
                        }
                    }
                }
                other => problems.push(format!("{}: {line}: {other:?}", root.display())),
            }
        }
        assert!(checked > 0, "{}: mshell.set lists no readable mission map", root.display());
    }
    assert!(problems.is_empty(), "no table for the human seat, or a bad value: {problems:#?}");
}
