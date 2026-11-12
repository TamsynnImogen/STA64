//! CON-43, install-gated: profile detection on real installs. Each test skips
//! cleanly when its variable is not set (see docs/INSTALL_ROOTS.md).
//!
//! - `STA64_TEST_INSTALL`: the Fleet Operations Roots install root.
//! - `STA64_TEST_STOCK_A2`: a stock Armada II root (loose files or `Data`).
//! - `STA64_TEST_A1`: an Armada I root (`assets.zfs`).
//! - `STA64_TEST_FO3` (optional, several roots separated by the platform path
//!   separator): Fleet Operations 3.x / Dominion War roots; their detection
//!   is recorded, and must stay what it was before CON-43.

use std::path::{Path, PathBuf};

use sta_content::{DetectStep, Detection, Profile};
use sta_vfs::Vfs;

fn detect(root: &Path, m: Option<&str>) -> Detection {
    let vfs = Vfs::mount_install(root, m).unwrap_or_else(|e| panic!("{root:?} {m:?}: {e}"));
    let d = Profile::detect_with_source(&vfs);
    eprintln!("{} {:?}: {d}", root.display(), m);
    d
}

#[test]
fn roots_install_and_all_its_mods_detect_as_the_roots_lineage_by_asset_version() {
    let Some(root) = std::env::var_os("STA64_TEST_INSTALL").map(PathBuf::from) else { return };
    let d = detect(&root, None);
    assert_eq!((d.profile, d.step), (Profile::Fo4, DetectStep::AssetVersion), "base Roots");
    assert!(d.file.as_deref().is_some_and(|p| p.ends_with("info.ini")));
    for m in ["Fleet Ops 4.0", "STA2 Classic Mod", "STA2 Classic Mod Enhanced Gameplay"] {
        let d = detect(&root, Some(m));
        assert_eq!((d.profile, d.step), (Profile::Fo4, DetectStep::AssetVersion), "{m}");
        assert!(d.detail.starts_with("AssetVersion = "), "{m}: {}", d.detail);
        assert!(d.warnings.is_empty(), "{m}: {:?}", d.warnings);
    }
}

#[test]
fn stock_armada_ii_detects_as_sta2_classic() {
    let Some(root) = std::env::var_os("STA64_TEST_STOCK_A2").map(PathBuf::from) else { return };
    let d = detect(&root, None);
    assert_eq!(d.profile, Profile::Sta2Classic);
    assert_eq!(d.step, DetectStep::Heuristic);
    assert!(d.fleetops_version.is_none());
}

#[test]
fn armada_i_detects_as_sta1_classic_by_its_zfs_archive() {
    let Some(root) = std::env::var_os("STA64_TEST_A1").map(PathBuf::from) else { return };
    let d = detect(&root, None);
    assert_eq!((d.profile, d.step), (Profile::Sta1Classic, DetectStep::Zfs));
}

#[test]
fn classic_content_identity_moves_with_its_profile_back_to_roots() {
    let Some(root) = std::env::var_os("STA64_TEST_INSTALL").map(PathBuf::from) else { return };
    for m in ["STA2 Classic Mod", "STA2 Classic Mod Enhanced Gameplay"] {
        let vfs = Vfs::mount_install(&root, Some(m)).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let (detected, _) = sta_content::ContentDb::load(&vfs, &index).unwrap();
        let (as_a2, _) = sta_content::ContentDb::load_as(&vfs, Profile::Sta2Classic, &index).unwrap();
        eprintln!("{m}: Roots-lineage content hash {:016x}, previous (Sta2Classic) {:016x}", detected.hash(), as_a2.hash());
        assert_eq!(detected.profile(), Profile::Fo4);
        assert_ne!(detected.hash(), as_a2.hash(), "{m}: the profile is part of the content identity");
    }
}

#[test]
fn fleet_operations_3x_installs_detect_as_before_and_record_their_version() {
    let Some(roots) = std::env::var_os("STA64_TEST_FO3") else { return };
    for root in std::env::split_paths(&roots) {
        let d = detect(&root, None);
        // Before CON-43 these were `Sta2Classic` by the Armada II layout (no `info.ini`, no FO name);
        // the 3.x engine has no profile variant in v1 (D5).
        assert_eq!((d.profile, d.step), (Profile::Sta2Classic, DetectStep::Heuristic), "{root:?}");
        assert_eq!(d.fleetops_version.as_deref(), Some("3.2.7"), "{root:?}");
    }
}
