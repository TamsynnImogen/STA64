//! `freighterName` of a mining station: the unit it gets free when a rig finishes it.
//! Authored ODFs for the key and the digest; install-gated (STA64_TEST_INSTALL +
//! STA64_TEST_MOD, STA64_TEST_DIR; skipped when unset) for the real data, asserting
//! only the rule: a station that names a unit names one the database has, and has a dock.

use std::path::Path;

use sta_content::{odf::load_with, ContentDb, OdfIndex, Overrides, Profile};
use sta_vfs::Vfs;

fn load(station: &str) -> ContentDb {
    load_warned(station).0
}

fn load_warned(station: &str) -> (ContentDb, Vec<sta_content::Warning>) {
    load_with(
        Profile::Native,
        &Overrides::default(),
        &["station".to_owned(), "freight".to_owned()],
        &mut |name, version| match (name.to_ascii_lowercase().as_str(), version) {
            ("station.odf", 0) => Some(station.to_owned()),
            ("freight.odf", 0) => Some("classLabel = \"freighter\"\ncrewCost = 100\n".to_owned()),
            _ => None,
        },
    )
    .unwrap()
}

fn free_of(db: &ContentDb) -> Option<String> {
    db.unit(db.key("station").unwrap()).unwrap().free_unit.clone()
}

#[test]
fn key_is_read_and_unknown_or_absent_means_none() {
    let base = "classLabel = \"mining\"\n";
    assert_eq!(free_of(&load(&format!("{base}freighterName = \"Freight\"\n"))).as_deref(), Some("freight"));
    assert_eq!(free_of(&load(&format!("{base}freighterName = \"Unknown\"\n"))), None);
    assert_eq!(free_of(&load(base)), None);
}

#[test]
fn digest_moves_only_when_the_key_is_set() {
    let base = "classLabel = \"mining\"\n";
    let (a, b) = (load(base), load(&format!("{base}freighterName = \"Unknown\"\n")));
    assert_eq!(a.hash(), b.hash());
    let c = load(&format!("{base}freighterName = \"freight\"\n"));
    assert_ne!(a.hash(), c.hash());
    assert_eq!(c.digest().first_difference(a.digest()).map(|d| d.key), Some("unit:station".to_owned()));
}

fn check(vfs: &Vfs, what: &str) {
    let (index, _) = OdfIndex::build(vfs);
    let (db, _) = ContentDb::load(vfs, &index).unwrap();
    let mut named = 0;
    for u in db.units().filter(|u| u.free_unit.is_some()) {
        named += 1;
        let name = u.free_unit.as_deref().unwrap();
        assert!(db.key(name).is_some(), "{what}: {} names {name}, which is no unit", u.odf);
        assert!(u.dock.is_some(), "{what}: {} has no dock point", u.odf);
    }
    eprintln!("{what}: {named} units name a free unit");
    assert!(named > 0, "{what}: no station names a free unit");
}

#[test]
fn installed_stations_name_units_the_database_has() {
    if let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) {
        let vfs = Vfs::mount_install(Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
        check(&vfs, &format!("install {m}"));
    }
    if let Ok(dir) = std::env::var("STA64_TEST_DIR") {
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), dir.clone().into()).unwrap();
        check(&vfs, &format!("dir {dir}"));
    }
}

#[test]
fn a_name_that_is_no_unit_warns_and_gives_none() {
    let (db, warnings) = load_warned("classLabel = \"mining\"\nfreighterName = \"ghost\"\n");
    assert_eq!(free_of(&db), None);
    assert!(db.unit(db.key("station").unwrap()).unwrap().dock.is_none());
    assert!(warnings.iter().any(|w| w.to_string().contains("freighterName ghost is not a defined unit")), "{warnings:?}");
    let (_, ok) = load_warned("classLabel = \"mining\"\nfreighterName = \"freight\"\n");
    assert!(!ok.iter().any(|w| w.to_string().contains("freighterName")), "{ok:?}");
}
