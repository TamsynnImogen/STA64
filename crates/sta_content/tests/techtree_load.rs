//! `ContentDb::load` mounts the skirmish tech tree (`tech1`) of a layered
//! vfs built in a temp dir: the top-most layer's file wins (a mod's `.tt`
//! over the base's `.xml`), includes resolve, and a missing or empty tree
//! leaves building ungated with a warning.

use std::fs;
use std::path::{Path, PathBuf};

use sta_content::{Availability, ContentDb, OdfIndex, Owned, Profile, Warning};
use sta_vfs::Vfs;

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sta64-techtree-load-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

const ODFS: &[(&str, &str)] = &[
    ("fbase", "[GameObjectClass]\nclassLabel = \"shipyard\"\nunitName = \"Starbase\"\nbuilditem1 = \"fyard\"\nbuilditem2 = \"fcruiser\"\n"),
    ("fyard", "[GameObjectClass]\nclassLabel = \"shipyard\"\nunitName = \"Yard\"\nbuilditem1 = \"fcruiser\"\n"),
    ("fcruiser", "[GameObjectClass]\nclassLabel = \"craft\"\nunitName = \"Cruiser\"\n"),
];

fn layer(tag: &str, files: &[(&str, &str)], odfs: bool) -> PathBuf {
    let d = scratch(tag);
    if odfs {
        for (n, t) in ODFS {
            put(&d, &format!("odf/{n}.odf"), t);
        }
    }
    for (p, t) in files {
        put(&d, p, t);
    }
    d
}

fn load(layers: &[(&str, &PathBuf)]) -> (ContentDb, Vec<Warning>) {
    let mut vfs = Vfs::default();
    for (n, d) in layers {
        vfs.push_dir((*n).to_owned(), (*d).clone()).unwrap();
    }
    let (index, _) = OdfIndex::build(&vfs);
    ContentDb::load_as(&vfs, Profile::Fo4, &index).unwrap()
}

fn start_options(db: &ContentDb) -> Vec<(String, bool)> {
    let tree = db.tech_tree().unwrap();
    let owned = Owned::new("unitedfederation").with("fbase", 1);
    tree.build_options(db, "fbase", &owned).into_iter().map(|o| (o.def.odf.clone(), matches!(o.availability, Availability::Available))).collect()
}

#[test]
fn a_tt_with_an_include_gates_the_start_station() {
    let base = layer(
        "tt",
        &[
            ("techtree/tech1.tt", "#include base.tt\nfcruiser 1 fyard\n"),
            ("techtree/base.tt", "fbase 0\nfyard 1 fbase\nfcruiser 1 fyard\n"),
        ],
        true,
    );
    let (db, warnings) = load(&[("base", &base)]);
    let tree = db.tech_tree().expect("tech1.tt loaded");
    assert_eq!(tree.files().len(), 2, "tech1.tt and its include");
    assert!(warnings.iter().all(|w| !w.message.contains("not gated")), "{warnings:?}");
    assert_eq!(start_options(&db), [("fyard".to_owned(), true), ("fcruiser".to_owned(), false)]);
    // The tree is part of the hash.
    let (bare, _) = load(&[("base", &layer("tt-bare", &[], true))]);
    assert_ne!(db.hash(), bare.hash());
}

#[test]
fn the_topmost_layer_decides_and_xml_beats_tt_within_a_layer() {
    let base = layer(
        "base",
        &[
            ("techtree/tech1.xml", "<techtree><object ODF=\"fbase.odf\"/><object ODF=\"fyard.odf\"/></techtree>"),
            ("techtree/tech1.tt", "fcruiser 0\n"),
        ],
        true,
    );
    let (db, _) = load(&[("base", &base)]);
    assert!(db.tech_tree().unwrap().entry("fyard").is_some(), "the .xml, not the .tt");
    assert!(db.tech_tree().unwrap().entry("fcruiser").is_none());

    let modl = layer("mod", &[("techtree/tech1.tt", "fcruiser 0\n")], false);
    let (db, _) = load(&[("mod", &modl), ("base", &base)]);
    let tree = db.tech_tree().unwrap();
    assert!(tree.entry("fcruiser").is_some() && tree.entry("fyard").is_none(), "the mod's .tt over the base's .xml");
}

#[test]
fn no_tree_means_no_gating_and_a_warning() {
    let base = layer("none", &[], true);
    let (db, warnings) = load(&[("base", &base)]);
    assert!(db.tech_tree().is_none());
    assert!(warnings.iter().any(|w| w.message.contains("no tech tree found")), "{warnings:?}");
}

#[test]
fn an_empty_or_malformed_tree_is_dropped_with_warnings() {
    let base = layer("bad", &[("techtree/tech1.xml", "<techtree><object ODF=")], true);
    let (db, warnings) = load(&[("base", &base)]);
    assert!(db.tech_tree().is_none());
    assert!(warnings.iter().any(|w| w.message.contains("not gated")), "{warnings:?}");
}
