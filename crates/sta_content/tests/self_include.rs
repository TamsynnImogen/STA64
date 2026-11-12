//! Self-include inheritance for `.h` cfg files and `.tt` tech trees (CON-36,
//! D11), over synthetic mod trees and, with `STA64_TEST_INSTALL`, over an
//! install's Classic Enhanced.

use std::fs;
use std::path::{Path, PathBuf};

use sta_content::{TechTree, coverage};
use sta_vfs::Vfs;

struct Tmp(PathBuf);
impl Tmp {
    fn new(name: &str) -> Tmp {
        let d = std::env::temp_dir().join(format!("sta_selfinc_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        Tmp(d)
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.0.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tree(name: &str, top_tt: &str) -> (Tmp, Vfs) {
    let t = Tmp::new(name);
    t.write("Data/techtree/tech1.tt", "ship.odf 0\nyard.odf 1 base.odf\n");
    t.write("Data/Mods/Child/info.ini", "[mod]\n");
    t.write("Data/Mods/Child/techtree/tech1.tt", top_tt);
    let vfs = Vfs::mount_install(&t.0, Some("Child")).unwrap();
    (t, vfs)
}

#[test]
fn tech_tree_inherits_its_parent_and_why_names_layer_and_line() {
    let (_t, vfs) = tree("tt", "#include tech1.tt\nship.odf -1\nnew.odf 0\n");
    let (t, w) = TechTree::from_vfs(&vfs, "tech1").unwrap().unwrap();
    assert!(w.is_empty(), "{w:?}");
    assert!(t.entry("yard").is_some(), "inherited from Data");
    assert!(t.entry("ship").unwrap().withheld, "the child's line wins");
    let w = coverage::why_key(&vfs, "tech1.tt", "yard").unwrap();
    assert_eq!((w.layer.as_str(), w.line), ("Data", 2));
    let w = coverage::why_key(&vfs, "tech1.tt", "ship").unwrap();
    assert_eq!((w.layer.as_str(), w.line, w.value.as_str()), ("Child", 2, "withheld"));
    let text = coverage::why_text(&vfs, "tech1.tt", Some("yard"));
    assert!(!text.contains("no layer provides") && text.contains("wins") && text.contains("needs base"), "{text}");
}

#[test]
fn a_true_tech_tree_cycle_fails_the_load() {
    let (_t, vfs) = {
        let t = Tmp::new("tt_cycle");
        t.write("Data/Mods/Child/info.ini", "[mod]\nstandalone=1\n");
        t.write("Data/Mods/Child/techtree/tech1.tt", "#include other.tt\n");
        t.write("Data/Mods/Child/techtree/other.tt", "#include tech1.tt\n");
        let vfs = Vfs::mount_install(&t.0, Some("Child")).unwrap();
        (t, vfs)
    };
    let err = TechTree::from_vfs(&vfs, "tech1").unwrap_err();
    assert!(matches!(err, sta_content::ContentError::IncludeCycle(_)), "{err:?}");
}

#[test]
fn cfg_self_include_shows_in_why_and_not_as_a_change() {
    let t = Tmp::new("cfg");
    t.write("Data/RTS_CFG.h", "float SHIELD_DAMAGE_PROTECTION = 0.8;\nint A = 1;\n");
    t.write("Data/Mods/Child/info.ini", "[mod]\n");
    t.write("Data/Mods/Child/RTS_CFG.h", "#include \"RTS_CFG.h\"\nfloat SHIELD_DAMAGE_PROTECTION = 0.9;\n");
    let vfs = Vfs::mount_install(&t.0, Some("Child")).unwrap();
    let w = coverage::why_key(&vfs, "RTS_CFG.h", "A").unwrap();
    assert_eq!((w.layer.as_str(), w.line, w.value.as_str()), ("Data", 2, "1"), "inherited keys name the layer below");
    let w = coverage::why_key(&vfs, "RTS_CFG.h", "SHIELD_DAMAGE_PROTECTION").unwrap();
    assert_eq!((w.layer.as_str(), w.line, w.value.as_str()), ("Child", 2, "0.9"));
    let changed: Vec<_> = coverage::cfg_changes(&vfs, "RTS_CFG.h").into_iter().map(|c| c.key).collect();
    assert_eq!(changed, ["SHIELD_DAMAGE_PROTECTION"], "only what the child states is a change");
}

/// No tree or header of any installed mod may cycle, and every mod's content loads. (The
/// installed Classic Enhanced ships full copies, not self-includes; the inheritance itself is
/// covered by the synthetic tests above.) Also records the profile each mod mounts as.
#[test]
fn installed_mods_resolve_without_cycles() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let data = fs::read_dir(&root).unwrap().flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("data")).unwrap().path();
    let (mods, _) = sta_vfs::mods::list_mods(&data).unwrap();
    for m in &mods {
        let vfs = Vfs::mount_install(Path::new(&root), Some(&m.folder)).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let (db, _) = sta_content::ContentDb::load(&vfs, &index).unwrap_or_else(|e| panic!("{}: {e}", m.folder));
        eprintln!("{}: profile {:?}", m.folder, db.profile());
        let expect = if m.folder.starts_with("Fleet Ops") { sta_content::Profile::Fo4 } else { sta_content::Profile::Sta2Classic };
        assert_eq!(db.profile(), expect, "{}", m.folder);
        for stem in ["tech1", "tech1_multi", "earlytech_multi", "freetech_multi", "fulltech_multi"] {
            if let Some((t, w)) = TechTree::from_vfs(&vfs, stem).unwrap_or_else(|e| panic!("{} {stem}: {e}", m.folder)) {
                eprintln!("{} {stem}: {} entries, {} warnings, files {:?}", m.folder, t.len(), w.len(), t.files());
            }
        }
        let w = coverage::why_file(&vfs, "RTS_CFG.h");
        eprintln!("{}: RTS_CFG.h from {:?}", m.folder, w.iter().map(|h| h.layer.as_str()).collect::<Vec<_>>());
    }
}
