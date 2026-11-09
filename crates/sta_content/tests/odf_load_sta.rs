//! `ContentDb::load` through the real index and VFS, over the layered
//! `.sta` fixture: the mod's `fakira` (inheriting Data's through a
//! self-include) becomes a unit; the whited-out `fdefiant` does not.

use std::path::Path;

use sta_content::{ContentDb, Fixed, OdfIndex, Profile};
use sta_vfs::Vfs;

#[test]
fn load_makes_the_layered_unit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sta");
    let vfs = Vfs::mount_install(&root, Some("StaMod")).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, warnings) = ContentDb::load_as(&vfs, Profile::Fo4, &index).unwrap();
    let akira = db.unit(db.key("fakira").unwrap()).unwrap();
    assert_eq!(akira.class.0, "craft");
    assert_eq!(akira.hull, Fixed::from_int(1500));
    assert!(db.key("fdefiant").is_none());
    assert_eq!(db.profile(), Profile::Fo4);
    eprintln!("{} definitions, {} warnings: {warnings:?}", db.len(), warnings.len());
    let (again, _) = ContentDb::load_as(&vfs, Profile::Fo4, &index).unwrap();
    assert_eq!(again.hash(), db.hash());
}

#[test]
fn load_detects_the_profile_and_resolves_features() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sta");
    let vfs = Vfs::mount_install(&root, Some("StaMod")).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    assert_eq!(db.profile(), Profile::detect(&vfs));
    let akira = db.unit(db.key("fakira").unwrap()).unwrap();
    assert_eq!(db.unit_features(akira), &db.profile().features());
}
