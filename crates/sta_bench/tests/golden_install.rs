//! TOOL-12, install-gated (D5): the Fleet Operations 3.2.7 and Dominion War
//! installs start a skirmish on every readable map, step it twice to the same
//! digest, and report the `fo3` profile. Skips when its variables are unset:
//! `STA64_TEST_FO3` (path list), `STA64_TEST_FO327_INSTALL`, `STA64_TEST_DW_INSTALL`.
//! No digest is pinned here: install-derived goldens stay outside the repository
//! (`sta-bench golden --record`, docs/TESTBENCH.md).

use std::path::PathBuf;

use sta_bench::cli::golden::{collect, differences};
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
fn fo3_installs_start_every_map_and_repeat_their_digests() {
    for root in roots() {
        let vfs = Vfs::mount_install(&root, None).unwrap();
        let a = collect(&vfs, None, &[], 60, 7, 2).unwrap();
        assert_eq!(a.profile, "fo3", "{root:?}");
        assert!(a.maps.len() > 20, "{root:?}: {} maps", a.maps.len());
        for m in &a.maps {
            assert!(m.error.is_none() && m.repeatable, "{root:?} {}: {m:?}", m.map);
            assert_eq!(m.digest_start.len(), 16);
        }
        // A second collection with another job count agrees with the first.
        let b = collect(&vfs, None, &[], 60, 7, 1).unwrap();
        assert!(differences(&a, &b).is_empty(), "{root:?}: {:?}", differences(&a, &b));
        // Another seed or tick count is a different recording.
        let c = collect(&vfs, None, &[], 61, 7, 2).unwrap();
        assert!(!differences(&a, &c).is_empty());
    }
}
