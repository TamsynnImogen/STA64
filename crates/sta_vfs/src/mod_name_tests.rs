//! A mod name or a `ParentMod` entry is one folder name under `Data/Mods` (#1387; ADR 0004 D1, D4, D6).

use std::fs;
use std::path::PathBuf;

use super::{VfsError, mod_chain, mod_chain_info, single_folder_name, Vfs};

struct Tmp(PathBuf);

impl Tmp {
    /// An install with `Data/Mods`, and a folder `outside` next to `Data` that looks like a mod (an `info.ini`
    /// and a file): what the lookup must never mount.
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("sta_vfs_modname_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("Data/Mods")).unwrap();
        let t = Tmp(d);
        t.write("outside/info.ini", "[mod]\nName=Outside\n");
        t.write("outside/odf/secret.odf", "outside");
        t
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.0.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    fn data(&self) -> PathBuf {
        self.0.join("Data")
    }
    fn outside(&self) -> String {
        self.0.join("outside").to_string_lossy().into_owned()
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn layers(v: &Vfs) -> Vec<&str> {
    v.layers().iter().map(|l| l.name()).collect()
}

/// The message names the mod that asked.
fn bad_name(r: Result<Vfs, VfsError>) -> (String, String) {
    match r {
        Err(VfsError::BadModName { name, asked_by }) => (name, asked_by),
        Err(e) => panic!("another error: {e}"),
        Ok(v) => panic!("mounted {:?}", layers(&v)),
    }
}

#[test]
fn a_folder_name_is_one_plain_name_on_every_platform() {
    for good in ["Fleet Ops 4.0", "Mod", "a.b", "..x", "x..", " lead", "Name (v2)", "ÄÖ", "ab:c", "xy:"] {
        assert!(single_folder_name(good), "{good:?}");
    }
    for bad in ["", ".", "..", "a/b", "a\\b", "../x", "..\\x", "/etc", "\\x", "C:", "c:x", "C:\\x", "\\\\server\\share", "a\0b", "x/"] {
        assert!(!single_folder_name(bad), "{bad:?}");
    }
}

#[test]
fn a_requested_name_with_path_parts_is_refused_and_names_who_asked() {
    let t = Tmp::new("requested");
    t.write("Data/Mods/Real/info.ini", "[mod]\n");
    let outside = t.outside();
    for name in ["..", ".", "", "../../outside", "..\\..\\outside", "Real/../Real", "Real/", &outside, "C:", "C:\\outside", "\\\\host\\share\\x"] {
        let (got, asked_by) = bad_name(Vfs::mount_install(&t.0, Some(name)));
        assert_eq!(got, name);
        assert!(asked_by.contains("command line"), "{asked_by}");
        // The two chain functions the mount and the mod screen use refuse the same.
        assert!(matches!(mod_chain(&t.data(), name), Err(VfsError::BadModName { .. })), "{name:?}");
        assert!(matches!(mod_chain_info(&t.data(), name), Err(VfsError::BadModName { .. })), "{name:?}");
    }
    // The message reads as a message.
    let e = Vfs::mount_install(&t.0, Some("../../outside")).err().unwrap().to_string();
    assert!(e.contains("../../outside") && e.contains("single folder name"), "{e}");
}

#[test]
fn a_parent_entry_with_path_parts_is_refused_and_names_the_child() {
    let t = Tmp::new("parent");
    let outside = t.outside();
    t.write("Data/Mods/Child/info.ini", "[mod]\nParentMod=../../outside\n");
    t.write("Data/Mods/Slash/info.ini", "[mod]\nParentMod=Other/Sub\n");
    t.write("Data/Mods/Dots/info.ini", "[mod]\nParentMod=..\n");
    t.write("Data/Mods/Abs/info.ini", &format!("[mod]\nParentMod={outside}\n"));
    t.write("Data/Mods/Drive/info.ini", "[mod]\nParentMod=C:\\outside\n");
    for (child, parent) in [("Child", "../../outside"), ("Slash", "Other/Sub"), ("Dots", ".."), ("Abs", outside.as_str()), ("Drive", "C:\\outside")] {
        let (got, asked_by) = bad_name(Vfs::mount_install(&t.0, Some(child)));
        assert_eq!(got, parent);
        assert!(asked_by.contains(child) && asked_by.contains("ParentMod"), "{asked_by}");
        assert!(matches!(mod_chain(&t.data(), child), Err(VfsError::BadModName { .. })), "{child}");
        assert!(matches!(mod_chain_info(&t.data(), child), Err(VfsError::BadModName { .. })), "{child}");
    }
    // A standalone child never follows its parent entry, as before: the entry is not looked at.
    t.write("Data/Mods/Alone/info.ini", "[mod]\nstandalone=1\nParentMod=../../outside\n");
    assert!(Vfs::mount_install(&t.0, Some("Alone")).is_ok());
}

#[test]
fn well_formed_names_mount_as_before_with_spaces_and_any_letter_case() {
    let t = Tmp::new("good");
    t.write("Data/odf/base.odf", "base");
    t.write("Data/Mods/My Parent Mod/info.ini", "[mod]\n");
    t.write("Data/Mods/My Parent Mod/odf/p.odf", "parent");
    t.write("Data/Mods/Child Mod/info.ini", "[mod]\nParentMod=my parent MOD\n");
    t.write("Data/Mods/Child Mod/odf/c.odf", "child");
    for name in ["Child Mod", "child mod", "CHILD MOD"] {
        let v = Vfs::mount_install(&t.0, Some(name)).unwrap();
        // The layers carry the spelling that asked for them, as before.
        assert_eq!(layers(&v), [name, "my parent MOD", "Data"], "{name}");
        assert_eq!(v.read("odf/c.odf").map(|r| r.unwrap()), Some(b"child".to_vec()));
        assert_eq!(v.read("odf/p.odf").map(|r| r.unwrap()), Some(b"parent".to_vec()));
    }
    let chain = mod_chain(&t.data(), "child mod").unwrap();
    assert_eq!(chain.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["child mod", "my parent MOD"]);
    // A name that is not there is still "not found", not a path error.
    assert!(matches!(Vfs::mount_install(&t.0, Some("nope")), Err(VfsError::ModNotFound(n)) if n == "nope"));
}

#[test]
fn the_mod_listing_never_joins_a_name_onto_a_path() {
    // `mods.rs` finds mods and parents by comparing the folder names it listed, so a parent entry with path parts
    // is simply a parent that is not installed.
    let t = Tmp::new("listing");
    t.write("Data/Mods/Child/info.ini", "[mod]\nParentMod=../../outside\n");
    let (mods, _) = crate::mods::list_mods(&t.data()).unwrap();
    assert!(crate::mods::find_mod(&mods, "../../outside").is_none());
    assert!(crate::mods::find_mod(&mods, "..").is_none());
    assert!(matches!(crate::mods::chain(&mods, "Child"), Err(crate::mods::ChainError::MissingParent { .. })));
}

#[cfg(unix)]
mod links {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn a_link_to_a_sibling_mod_is_followed_and_a_link_that_leaves_the_mods_folder_is_refused() {
        let t = Tmp::new("links");
        t.write("Data/Mods/Real/info.ini", "[mod]\n");
        t.write("Data/Mods/Real/odf/r.odf", "real");
        symlink(t.0.join("Data/Mods/Real"), t.0.join("Data/Mods/Alias")).unwrap();
        symlink(t.0.join("outside"), t.0.join("Data/Mods/Escape")).unwrap();
        t.write("Data/Mods/Child/info.ini", "[mod]\nParentMod=Escape\n");
        let v = Vfs::mount_install(&t.0, Some("Alias")).unwrap();
        assert_eq!(v.read("odf/r.odf").map(|r| r.unwrap()), Some(b"real".to_vec()));
        for name in ["Escape", "Child"] {
            match Vfs::mount_install(&t.0, Some(name)) {
                Err(VfsError::ModOutsideMods { name: n, asked_by, target }) => {
                    assert_eq!(n, "Escape");
                    assert!(target.ends_with("outside"), "{}", target.display());
                    if name == "Child" {
                        assert!(asked_by.contains("Child"), "{asked_by}");
                    }
                }
                other => panic!("{name}: {:?}", other.map(|v| layers(&v).iter().map(|s| s.to_string()).collect::<Vec<_>>())),
            }
        }
    }

    #[test]
    fn a_mods_folder_that_is_itself_a_link_still_works() {
        // The player keeps the whole mods folder elsewhere and links it: the mods lie directly under what it
        // resolves to.
        let t = Tmp::new("modslink");
        fs::remove_dir_all(t.0.join("Data/Mods")).unwrap();
        t.write("elsewhere/M/info.ini", "[mod]\n");
        symlink(t.0.join("elsewhere"), t.0.join("Data/Mods")).unwrap();
        assert!(Vfs::mount_install(&t.0, Some("m")).is_ok());
    }
}
