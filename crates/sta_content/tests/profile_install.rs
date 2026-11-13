//! CON-43, install-gated: profile detection on real installs. Each test skips
//! cleanly when its variable is not set (see docs/INSTALL_ROOTS.md).
//!
//! - `STA64_TEST_INSTALL`: the Fleet Operations Roots install root.
//! - `STA64_TEST_STOCK_A2`: a stock Armada II root (loose files or `Data`).
//! - `STA64_TEST_A1`: an Armada I root (`assets.zfs`).
//! - `STA64_TEST_FO3` (optional, several roots separated by the platform path
//!   separator), `STA64_TEST_FO327_INSTALL`, `STA64_TEST_DW_INSTALL`: Fleet
//!   Operations 3.2.7 / Dominion War roots; they detect as `Fo3` (CON-44).

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

/// The FO 3.2.7 and Dominion War roots: `STA64_TEST_FO3` (a path list) plus the
/// single-install variables the mission tests use.
fn fo3_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::env::var_os("STA64_TEST_FO3").map(|r| std::env::split_paths(&r).collect()).unwrap_or_default();
    for var in ["STA64_TEST_FO327_INSTALL", "STA64_TEST_DW_INSTALL"] {
        v.extend(std::env::var_os(var).map(PathBuf::from));
    }
    v.sort();
    v.dedup();
    v
}

#[test]
fn fleet_operations_3x_installs_detect_as_fo3_by_packages_ini() {
    for root in fo3_roots() {
        let d = detect(&root, None);
        // CON-44 (D5): no `info.ini`, `[FleetOps] Version=3.2.7` in `Data/packages.ini`.
        assert_eq!((d.profile, d.step), (Profile::Fo3, DetectStep::PackagesIni), "{root:?}");
        assert_eq!(d.fleetops_version.as_deref(), Some("3.2.7"), "{root:?}");
        assert!(d.file.as_deref().is_some_and(|p| p.ends_with("packages.ini")), "{root:?}");
        assert!(d.warnings.is_empty(), "{root:?}: {:?}", d.warnings);
    }
}

#[test]
fn fo3_content_loads_under_its_own_lineage_and_the_profile_is_in_the_identity() {
    for root in fo3_roots() {
        let vfs = Vfs::mount_install(&root, None).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let (db, _) = sta_content::ContentDb::load(&vfs, &index).unwrap();
        let (as_a2, _) = sta_content::ContentDb::load_as(&vfs, Profile::Sta2Classic, &index).unwrap();
        eprintln!("{root:?}: fo3 hash {:016x}, as sta2classic {:016x}", db.hash(), as_a2.hash());
        assert_eq!((db.profile(), db.lineage()), (Profile::Fo3, "fo3"));
        assert_ne!(db.hash(), as_a2.hash());
        // The data the feature row cites (see `Profile::features`): FO's own PD class,
        // slots beyond dilithium charged, build yards.
        assert!(db.weapons().any(|w| w.ordnance_defence), "{root:?}: ordnancedefenseweapon");
        assert!(db.units().any(|u| u.build.cost.iter().skip(1).any(|c| c.to_bits() > 0)), "{root:?}: latinum/metal/biomatter costs");
        assert_eq!(db.resources().name(1), "Tritanium", "{root:?}");
        assert_eq!(db.resources().name(3), "Supply", "{root:?}");
    }
}

#[test]
fn other_installs_keep_their_profile_and_the_fake_hash_is_unchanged() {
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
    if let Some(root) = std::env::var_os("STA64_TEST_STOCK_A2").map(PathBuf::from) {
        assert_eq!(detect(&root, None).profile, Profile::Sta2Classic);
    }
    if let Some(root) = std::env::var_os("STA64_TEST_INSTALL").map(PathBuf::from) {
        assert_eq!(detect(&root, None).profile, Profile::Fo4);
    }
}
