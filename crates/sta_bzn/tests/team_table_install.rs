//! The team table of the installed campaign maps: every mission map an install's `mshell.set` lists (lines
//! not commented out) yields a table of eight seats for the human seat. Structure and general rules only,
//! no stock values. Install-gated: set `STA64_TEST_A2_STOCK_INSTALL` and/or `STA64_TEST_ROOTS_INSTALL`;
//! skipped when none is set. Evidence grade of the rule: read from data.

use std::path::{Path, PathBuf};

fn data_dir(root: &Path) -> PathBuf {
    if root.join("Data").is_dir() { root.join("Data") } else { root.to_path_buf() }
}

#[test]
fn every_listed_mission_map_has_a_team_table_for_the_human_seat() {
    let roots: Vec<PathBuf> = ["STA64_TEST_A2_STOCK_INSTALL", "STA64_TEST_ROOTS_INSTALL"].iter().filter_map(std::env::var_os).map(PathBuf::from).collect();
    if roots.is_empty() {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL / STA64_TEST_ROOTS_INSTALL");
        return;
    }
    let mut problems = Vec::new();
    let mut checked = 0;
    for root in roots {
        let data = data_dir(&root);
        let Ok(set) = std::fs::read_to_string(data.join("mshell.set")) else { continue };
        for line in set.lines().map(str::trim).filter(|l| l.to_ascii_lowercase().ends_with(".bzn") && !l.starts_with("//")) {
            let Ok(bytes) = std::fs::read(data.join("bzn").join(line)) else { continue };
            checked += 1;
            match sta_bzn::team_table(&bytes) {
                sta_bzn::TeamTable::Found(t) if t.len() == 8 => {
                    assert!(t.iter().all(|s| s.dilithium.is_finite() && s.biomatter.is_finite() && s.crew.is_finite()), "{line}: non-finite value");
                }
                other => problems.push(format!("{}: {line}: {other:?}", root.display())),
            }
        }
    }
    assert!(problems.is_empty(), "{checked} maps checked; no table for the human seat: {problems:#?}");
}
