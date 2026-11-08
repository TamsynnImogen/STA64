//! The ODF index over a mod whose content is one `.sta` archive
//! (`fixtures/sta`): its ODFs are found anywhere under `odf/`, win over
//! Data's, reach Data's copy through a self-include, and its whiteout hides
//! Data's file.

use std::path::Path;

use sta_content::OdfIndex;
use sta_vfs::{Source, Vfs};

#[test]
fn index_finds_and_resolves_sta_odfs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sta");
    let vfs = Vfs::mount_install(&root, Some("StaMod")).unwrap();
    let (index, warnings) = OdfIndex::build(&vfs);
    assert!(warnings.is_empty(), "{warnings:?}");

    let akira = index.get("fakira").unwrap();
    assert_eq!(vfs.layers()[akira.layer].name(), "StaMod");
    assert_eq!(index.versions("fakira").len(), 2, "the .sta copy over Data's");
    let fnew = index.get("FNEW").unwrap();
    assert_eq!(fnew.path, "odf/my_pack/deep/fnew.odf");
    assert!(matches!(vfs.resolve(&fnew.path), Some((0, Source::Sta { .. }))));
    assert!(index.get("fdefiant").is_none(), "whited out by the archive");

    let odf = sta_odf::resolve_layered("fakira", &mut |n, v| index.load(&vfs, n, v)).unwrap();
    assert_eq!(odf.i64("maxHealth"), Some(1500));
    assert_eq!(odf.str("unitName"), Some("Akira (sta)"));
    assert_eq!(odf.class_label(), Some("craft"), "from Data's copy, through the self-include");
    assert!(odf.cut.is_empty());
}
