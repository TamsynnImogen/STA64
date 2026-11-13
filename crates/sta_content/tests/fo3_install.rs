//! CON-45, install-gated (D5, D7): coverage of the Fleet Operations 3.2.7 and
//! Dominion War installs. Each test skips when its variable is unset:
//!
//! - `STA64_TEST_FO3` (path list), `STA64_TEST_FO327_INSTALL`, `STA64_TEST_DW_INSTALL`.
//!
//! Prints the coverage report of each install (`cargo test -p sta_content
//! --test fo3_install -- --nocapture`).

use std::path::PathBuf;

use sta_content::{coverage, ContentDb, OdfIndex, Profile};
use sta_vfs::Vfs;

fn roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::env::var_os("STA64_TEST_FO3").map(|r| std::env::split_paths(&r).collect()).unwrap_or_default();
    for var in ["STA64_TEST_FO327_INSTALL", "STA64_TEST_DW_INSTALL"] {
        v.extend(std::env::var_os(var).map(PathBuf::from));
    }
    v.sort();
    v.dedup();
    v
}

#[test]
fn each_install_loads_under_fo3_and_its_coverage_has_no_dangling_reference() {
    for root in roots() {
        let vfs = Vfs::mount_install(&root, None).unwrap();
        let r = coverage::check(&vfs);
        eprintln!("== {}\n{r}", root.display());
        // Every effective ODF resolves, every map parses and every object, starfield and
        // tech tree a map names is provided (the installs are consistent as shipped).
        assert!(r.odf.files > 1000, "{root:?}: {} ODFs", r.odf.files);
        assert!(r.odf.errors.is_empty(), "{root:?}: {:?}", r.odf.errors);
        assert!(r.maps_checked > 0 && r.map_errors.is_empty(), "{root:?}: {:?}", r.map_errors);
        assert!(r.maps.is_empty(), "{root:?}: unresolved map references {:?}", r.maps);
        // A standalone `Data` has no hidden header to differ from; what STA64 does not read
        // of its own `RTS_CFG.h` is listed instead (`check_pack <root> --cfg-unread`).
        assert!(r.cfg.is_empty());
        assert!(!r.cfg_unread.is_empty());
        let (index, _) = OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        assert_eq!(db.profile(), Profile::Fo3, "{root:?}");
        assert!(db.units().count() > 1000 && db.weapons().count() > 1000, "{root:?}");
    }
}

#[test]
fn the_header_rules_of_both_installs_are_the_same_and_typed() {
    // Dominion War ships the 3.2.7 `RTS_CFG.h` unchanged: the typed `RTS_CFG.h` entries of the rules digest match (race rules differ by mod).
    let roots = roots();
    let mut digests = Vec::new();
    for root in &roots {
        let vfs = Vfs::mount_install(root, None).unwrap();
        let (index, _) = OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        digests.push(db.rules().digest_entries().iter().filter(|e| e.key.starts_with("RTS_CFG.h:")).map(|e| format!("{}={}", e.key, e.value)).collect::<Vec<_>>());
    }
    if let [a, b] = digests.as_slice() {
        assert!(!a.is_empty());
        assert_eq!(a, b);
    }
}
