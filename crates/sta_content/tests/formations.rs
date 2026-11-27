//! CON-59 (ADR 0011 G3, G12): formation files and the group rules through the
//! mounted content. Authored files only, under cargo's target directory.

use std::path::PathBuf;

use sta_content::{ContentDb, GroupRules, OdfIndex, Profile};
use sta_vfs::Vfs;

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Tree {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("formations-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, relative: &str, text: &str) -> &Tree {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        self
    }

    fn load(&self, profile: Profile) -> (ContentDb, Vec<sta_content::Warning>) {
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), self.0.clone()).unwrap();
        let (index, _) = OdfIndex::build(&vfs);
        ContentDb::load_as(&vfs, profile, &index).unwrap()
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn authored(tree: &Tree) -> &Tree {
    for (path, text) in sta_content::fake::FORMATION_FILES {
        tree.write(&path.replace("ai/formations", "AI/formations"), text);
    }
    tree.write("AI/formations/formdef.h", "// not a formation\n")
}

#[test]
fn the_mounted_formation_files_load_with_no_warning_and_enter_the_digest() {
    let tree = Tree::new("load");
    authored(&tree);
    let (db, warnings) = tree.load(Profile::Sta2Classic);
    assert!(warnings.iter().all(|w| !w.file.ends_with(".frm")), "{warnings:?}");
    assert_eq!(db.formations().len(), 2);
    assert_eq!(db.formations().iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["FxLine", "FxBox"]);
    assert!(db.digest().get("formation:0").is_some_and(|v| v.contains("FxLine")));
    // Without the files the same content has none and a different digest.
    let bare = Tree::new("bare");
    let (none, _) = bare.load(Profile::Sta2Classic);
    assert!(none.formations().is_empty());
    assert!(none.digest().get("formation:0").is_none());
    assert_ne!(none.hash(), db.hash());
}

#[test]
fn armada_i_reads_no_formation_files() {
    let tree = Tree::new("a1");
    authored(&tree);
    let (db, _) = tree.load(Profile::Sta1Classic);
    assert!(db.formations().is_empty());
}

#[test]
fn the_digest_changes_with_a_slot_offset_and_with_each_key() {
    let hash = |files: &str, header: &str| {
        let tree = Tree::new("digest");
        authored(&tree);
        if !files.is_empty() {
            let text = sta_content::fake::FORMATION_FILES[0].1.replace("1.0, 0.0, -1.0", files);
            tree.write("AI/formations/fx_line.frm", &text);
        }
        tree.write("RTS_CFG.h", header);
        tree.load(Profile::Sta2Classic).0.hash()
    };
    let base = hash("", "");
    assert_eq!(base, hash("", "int UNRELATED = 3;\n"), "an unrelated key changes nothing");
    assert_ne!(base, hash("1.0, 0.0, -1.5", ""), "a slot offset");
    assert_ne!(base, hash("", "float FORMATION_SPACING = 120.0;\n"));
    assert_ne!(base, hash("", "float FORMATION_LEADER_SLOWDOWN = 0.5;\n"));
    assert_ne!(base, hash("", "int cfgDEFAULT_TO_FORMATION_MOVE = 0;\n"));
    assert_ne!(base, hash("", "int FORMATIONS_RELATIVE_TO_SHIP = 1;\n"));
}

#[test]
fn a_key_set_to_the_profiles_default_digests_like_an_absent_key() {
    let hash = |header: &str| {
        let tree = Tree::new("default");
        tree.write("RTS_CFG.h", header);
        tree.load(Profile::Sta2Classic).0.hash()
    };
    assert_eq!(hash(""), hash("int cfgDEFAULT_TO_FORMATION_MOVE = 1;\nfloat FORMATION_LEADER_SLOWDOWN = 1.0;\n"));
}

#[test]
fn a_mod_header_that_sets_the_default_move_key_to_the_other_value_gets_the_other_mode() {
    for (profile, other_text) in [(Profile::Sta2Classic, "0"), (Profile::Fo4, "0"), (Profile::Fo3, "1")] {
        let plain = Tree::new("mode-plain");
        let (db, _) = plain.load(profile);
        let default = db.group_rules().default_to_formation_move;
        assert_eq!(default, GroupRules::defaults(profile).default_to_formation_move);
        assert_eq!(default, other_text == "0", "{profile:?}: the profile's own default");
        let modded = Tree::new("mode-mod");
        modded.write("RTS_CFG.h", &format!("int cfgDEFAULT_TO_FORMATION_MOVE = {other_text};\n"));
        let (db, warnings) = modded.load(profile);
        assert!(warnings.iter().all(|w| w.file != "RTS_CFG.h"), "{warnings:?}");
        assert_eq!(db.group_rules().default_to_formation_move, !default, "{profile:?}");
    }
    // Armada I spells the key without the prefix.
    let a1 = Tree::new("mode-a1");
    a1.write("RTS_CFG.h", "int DEFAULT_TO_FORMATION_MOVE = 1;\nfloat FORMATIONS_RELATIVE_TO_SHIP = 0;\n");
    let (db, _) = a1.load(Profile::Sta1Classic);
    assert!(db.group_rules().default_to_formation_move);
    assert!(!GroupRules::defaults(Profile::Sta1Classic).default_to_formation_move);
}
