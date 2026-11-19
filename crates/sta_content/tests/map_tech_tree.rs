//! A map's `techTreeFilename` picks the game's tech tree (`ContentDb::with_map_tech_tree`),
//! over a synthetic mount with authored trees.

use std::fs;
use std::path::PathBuf;

use sta_content::{ContentDb, TechTree, UnitDef};
use sta_vfs::Vfs;

struct Tmp(PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn mount(name: &str) -> (Tmp, Vfs) {
    let d = std::env::temp_dir().join(format!("sta_maptt_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    let w = |rel: &str, text: &str| {
        let p = d.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    };
    w("Data/techtree/tech1.tt", "yard.odf 0\nship.odf 1 yard.odf\nbig.odf 1 yard.odf\n");
    w("Data/techtree/small.tt", "#include tech1.tt\nbig.odf -1\n");
    w("Data/techtree/empty.tt", "// nothing\n");
    w("Data/Mods/M/info.ini", "[mod]\n");
    let vfs = Vfs::mount_install(&d, Some("M")).unwrap();
    (Tmp(d), vfs)
}

fn base(vfs: &Vfs) -> ContentDb {
    let (t, _) = TechTree::from_vfs(vfs, "tech1").unwrap().unwrap();
    let mut b = ContentDb::builder().tech_tree(t);
    for n in ["yard", "ship", "big"] {
        b = b.unit(UnitDef::new(n, "craft"));
    }
    b.build().unwrap()
}

#[test]
fn the_named_tree_replaces_the_default_and_moves_the_digest() {
    let (_t, vfs) = mount("swap");
    let db = base(&vfs);
    assert!(!db.tech_tree().unwrap().entry("big").unwrap().withheld);
    let before = db.digest().clone();
    let (new, w) = db.with_map_tech_tree(&vfs, "small.tt");
    assert!(w.is_empty(), "{w:?}");
    let db = new.expect("swapped");
    assert!(db.tech_tree().unwrap().entry("big").unwrap().withheld);
    assert!(db.tech_tree().unwrap().is_text());
    assert_ne!(&before, db.digest());
}

#[test]
fn the_same_default_empty_or_none_names_change_nothing() {
    let (_t, vfs) = mount("same");
    for name in ["tech1.tt", "TECH1.TT", "", "  ", "none", "None"] {
        let db = base(&vfs);
        let digest = db.digest().clone();
        let (new, w) = db.with_map_tech_tree(&vfs, name);
        assert!(w.is_empty(), "{name}: {w:?}");
        assert!(new.is_none(), "{name}");
        assert_eq!(&digest, base(&vfs).digest(), "{name}");
    }
}

#[test]
fn a_tree_the_install_lacks_or_without_entries_keeps_the_default_and_warns() {
    let (_t, vfs) = mount("missing");
    for name in ["gone.tt", "empty.tt"] {
        let db = base(&vfs);
        let digest = db.digest().clone();
        let (new, w) = db.with_map_tech_tree(&vfs, name);
        assert_eq!(w.len(), 1, "{name}: {w:?}");
        assert!(w[0].message.contains("current tree stays"), "{}", w[0].message);
        assert!(new.is_none(), "{name}");
        assert_eq!(&digest, base(&vfs).digest(), "{name}");
    }
}

/// A mission game's content is rebuilt from the same map name, so its digest is the live game's;
/// the default tree's digest differs, which a snapshot's content check refuses.
#[test]
fn rebuilding_the_content_from_the_map_gives_the_same_digest() {
    let (_t, vfs) = mount("reload");
    let live = base(&vfs).with_map_tech_tree(&vfs, "small.tt").0.unwrap();
    let reloaded = base(&vfs).with_map_tech_tree(&vfs, "small.tt").0.unwrap();
    assert_eq!(live.digest(), reloaded.digest());
    assert_ne!(live.digest(), base(&vfs).digest());
}
