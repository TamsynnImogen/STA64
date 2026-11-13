//! CON-47: the path list of every installed Armada II / Fleet Operations map.
//! Install-gated: set `STA64_TEST_A2_STOCK_INSTALL`, `STA64_TEST_ROOTS_INSTALL`
//! and/or `STA64_TEST_A2_BZN_DIRS` (a `:`-separated list of install roots to
//! search for `*.bzn`); skipped when none is set.

use std::path::{Path, PathBuf};

fn bzn_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            bzn_files(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bzn")) {
            out.push(p);
        }
    }
}

fn roots() -> Vec<PathBuf> {
    let mut r: Vec<PathBuf> = ["STA64_TEST_A2_STOCK_INSTALL", "STA64_TEST_ROOTS_INSTALL"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    if let Some(v) = std::env::var_os("STA64_TEST_A2_BZN_DIRS") {
        r.extend(std::env::split_paths(&v));
    }
    r
}

#[test]
fn every_installed_map_has_a_sane_path_list() {
    let roots = roots();
    if roots.is_empty() {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL / STA64_TEST_ROOTS_INSTALL / STA64_TEST_A2_BZN_DIRS");
        return;
    }
    let (mut maps, mut paths, mut points) = (0, 0, 0);
    let mut empty: Vec<String> = Vec::new();
    for root in roots {
        let mut files = Vec::new();
        bzn_files(&root, &mut files);
        files.sort();
        for f in files {
            let bytes = std::fs::read(&f).unwrap();
            let map = sta_bzn::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            if map.a1.is_some() {
                continue;
            }
            maps += 1;
            if map.paths.is_empty() {
                // A map may hold a zero count (the stock `fulltech` placeholder does).
                empty.push(f.display().to_string());
            }
            assert!(map.paths.len() <= 4096, "{}: {} paths", f.display(), map.paths.len());
            for p in &map.paths {
                assert!(p.points.len() <= 100_000, "{}: {} has {} points", f.display(), p.label, p.points.len());
                assert!(
                    p.points.iter().flatten().all(|c| c.is_finite() && c.abs() < 1.0e6),
                    "{}: path {} has a point out of bounds",
                    f.display(),
                    p.label
                );
                assert!(p.belt_speed.is_finite() && p.belt_speed >= 0.0, "{}: {}", f.display(), p.label);
                points += p.points.len();
            }
            paths += map.paths.len();
            // Write then parse gives the same map.
            let again = sta_bzn::parse(&sta_bzn::write::write(&map).unwrap()).unwrap();
            assert_eq!(again.paths, map.paths, "{}: paths do not survive a write", f.display());
        }
    }
    eprintln!("{maps} maps, {paths} paths, {points} points; empty lists: {empty:?}");
    assert!(maps > 0, "no map found under the given roots");
    assert!(empty.len() * 50 <= maps, "too many maps without a path list: {empty:?}");
}
