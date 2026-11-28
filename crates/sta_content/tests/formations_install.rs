//! CON-59, install-gated: the formation files and the group-movement keys of the
//! installs, read at run time (read-only; structure only, ADR 0003). Each test
//! skips when its variable is unset:
//!
//! - `STA64_TEST_STOCK_A2`: a stock Armada II root (loose files or `Data`).
//! - `STA64_TEST_INSTALL`: the Fleet Operations Roots install root.
//! - `STA64_TEST_FO327_INSTALL`: a Fleet Operations 3.2.7 root.
//! - `STA64_TEST_A1`: an Armada I root.
//!
//! Counts and key values are printed (`-- --nocapture`), not asserted.

use std::path::PathBuf;

use sta_content::{ContentDb, OdfIndex, Profile};
use sta_vfs::Vfs;

fn root(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

fn load(root: &PathBuf, m: Option<&str>) -> (ContentDb, Vec<sta_content::Warning>) {
    let vfs = Vfs::mount_install(root, m).unwrap_or_else(|e| panic!("{root:?} {m:?}: {e}"));
    let (index, _) = OdfIndex::build(&vfs);
    ContentDb::load(&vfs, &index).unwrap()
}

/// What every formation of the A2 family must be, and the keys that must read.
fn check_a2_family(name: &str, root: &PathBuf, m: Option<&str>, profile: Profile) {
    let (db, warnings) = load(root, m);
    assert_eq!(db.profile(), profile, "{name}");
    let frm: Vec<_> = warnings.iter().filter(|w| w.file.to_ascii_lowercase().ends_with(".frm")).collect();
    assert!(frm.is_empty(), "{name}: {frm:?}");
    let f = db.formations();
    eprintln!("{name}: {} formations", f.len());
    assert!(!f.is_empty(), "{name}");
    let mut ids = std::collections::BTreeSet::new();
    for formation in f.iter() {
        assert!(ids.insert(formation.id), "{name}: id repeated");
        assert!(!formation.name.is_empty() && formation.radius > sta_content::Fixed::ZERO, "{name}");
        assert!(!formation.slots.is_empty() && formation.leader_slot(formation.slots.len()).is_some(), "{name}: {}", formation.name);
    }
    let cfg = db.rules().group_cfg();
    eprintln!("{name}: spacing {:?} slowdown {:?} default-formation-move {:?}", cfg.formation_spacing.map(|v| v.to_f32()), cfg.leader_slowdown.map(|v| v.to_f32()), cfg.default_to_formation_move);
    assert!(cfg.formation_spacing.is_some() && cfg.leader_slowdown.is_some() && cfg.default_to_formation_move.is_some(), "{name}: a key was not read");
    assert_eq!(db.group_rules().default_to_formation_move, cfg.default_to_formation_move.unwrap(), "{name}");
}

#[test]
fn stock_armada_ii_loads_its_formations_and_reads_the_keys() {
    let Some(root) = root("STA64_TEST_STOCK_A2") else { return };
    check_a2_family("stock Armada II", &root, None, Profile::Sta2Classic);
}

#[test]
fn roots_loads_its_formations_and_reads_the_keys() {
    let Some(root) = root("STA64_TEST_INSTALL") else { return };
    check_a2_family("Roots", &root, None, Profile::Fo4);
}

#[test]
fn fleet_operations_3_2_7_loads_its_formations_and_reads_the_keys() {
    let Some(root) = root("STA64_TEST_FO327_INSTALL") else { return };
    check_a2_family("Fleet Operations 3.2.7", &root, None, Profile::Fo3);
}

#[test]
fn armada_i_has_no_formation_files_and_reads_its_keys() {
    let Some(root) = root("STA64_TEST_A1") else { return };
    let (db, _) = load(&root, None);
    assert_eq!(db.profile(), Profile::Sta1Classic);
    assert!(db.formations().is_empty());
    let cfg = db.rules().group_cfg();
    eprintln!("Armada I: {cfg:?}");
    assert!(cfg.formation_spacing.is_some() && cfg.leader_slowdown.is_some() && cfg.a1_default_to_formation_move.is_some() && cfg.formations_relative_to_ship.is_some(), "a key was not read");
}
