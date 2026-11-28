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
    for good in ["Fleet Ops 4.0", "Mod", "a.b", "..x", ".hidden", "a b", "a..b", " lead", "Name (v2)", "ÄÖ", "ab:c", "xy:"] {
        assert!(single_folder_name(good), "{good:?}");
    }
    for bad in ["", ".", "..", "a/b", "a\\b", "../x", "..\\x", "/etc", "\\x", "C:", "c:x", "C:\\x", "\\\\server\\share", "a\0b", "x/"] {
        assert!(!single_folder_name(bad), "{bad:?}");
    }
    // A dot or a space at the end: only dots (the mods folder itself where the system drops them), a name
    // with a dot or a space after it (another folder's name there), only spaces, and mixtures.
    for bad in ["...", "....", "x.", "x..", "Mod.", "Mod ", "Mod  ", "Mod. ", "Mod .", " ", "  ", ". ", ".. ", " ."] {
        assert!(!single_folder_name(bad), "{bad:?}");
    }
}

#[test]
fn a_name_or_a_parent_entry_ending_in_a_dot_or_a_space_is_refused_and_names_who_asked() {
    let t = Tmp::new("trailing");
    t.write("Data/odf/base.odf", "base");
    t.write("Data/Mods/Real/info.ini", "[mod]\n");
    t.write("Data/Mods/Real/odf/r.odf", "real");
    t.write("Data/Mods/Other/info.ini", "[mod]\n");
    // the name given
    for name in ["...", "....", "Real.", "Real..", "Real ", "Real. ", " "] {
        let (got, asked_by) = bad_name(Vfs::mount_install(&t.0, Some(name)));
        assert_eq!(got, name);
        assert!(asked_by.contains("command line"), "{asked_by}");
        assert!(matches!(mod_chain(&t.data(), name), Err(VfsError::BadModName { .. })), "{name:?}");
        assert!(matches!(mod_chain_info(&t.data(), name), Err(VfsError::BadModName { .. })), "{name:?}");
    }
    // a parent entry: the ini reader trims spaces around a value, so the forms that reach the check end in a dot
    for (i, parent) in ["...", "....", "Real.", "Real..", "Real .", ". ."].iter().enumerate() {
        let child = format!("Child{i}");
        t.write(&format!("Data/Mods/{child}/info.ini"), &format!("[mod]\nParentMod={parent}\n"));
        let (got, asked_by) = bad_name(Vfs::mount_install(&t.0, Some(&child)));
        assert_eq!(&got, parent);
        assert!(asked_by.contains(&child) && asked_by.contains("ParentMod"), "{asked_by}");
        assert!(matches!(mod_chain(&t.data(), &child), Err(VfsError::BadModName { .. })), "{child}");
    }
    // a parent entry with spaces around it is the plain name, as before
    t.write("Data/Mods/Spaced/info.ini", "[mod]\nParentMod =  Real  \n");
    let v = Vfs::mount_install(&t.0, Some("Spaced")).unwrap();
    assert_eq!(layers(&v), ["Spaced", "Real", "Data"]);
    // the message says what is wrong
    let e = Vfs::mount_install(&t.0, Some("...")).err().unwrap().to_string();
    assert!(e.contains("\"...\"") && e.contains("dot or space at the end"), "{e}");
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

    /// The list offers what the mount takes: a mod folder that is a link to a folder is listed (the player can
    /// pick the mod they linked in), a link that leads nowhere or to a file is not.
    #[test]
    fn the_mod_list_has_a_linked_mod_folder_and_skips_a_link_that_is_no_folder() {
        let t = Tmp::new("list_links");
        t.write("Data/Mods/Real/info.ini", "[mod]\nName=Real\n");
        t.write("elsewhere/Kept/info.ini", "[mod]\nName=Kept elsewhere\nParentMod=Real\n");
        t.write("elsewhere/a_file", "not a folder");
        symlink(t.0.join("elsewhere/Kept"), t.0.join("Data/Mods/Linked")).unwrap();
        symlink(t.0.join("elsewhere/gone"), t.0.join("Data/Mods/Dangling")).unwrap();
        symlink(t.0.join("elsewhere/a_file"), t.0.join("Data/Mods/ToFile")).unwrap();
        let (mods, notes) = crate::mods::list_mods(&t.data()).unwrap();
        let folders: Vec<&str> = mods.iter().map(|m| m.folder.as_str()).collect();
        assert_eq!(folders, ["Linked", "Real"], "notes: {notes:?}");
        let linked = crate::mods::find_mod(&mods, "linked").expect("listed under the link's name");
        assert_eq!(linked.name, "Kept elsewhere", "its own info.ini was read through the link");
        assert!(notes.iter().all(|n| !n.contains("Dangling") && !n.contains("ToFile")), "skipped as no folder, without a note: {notes:?}");
        // and the list's own chain and the mount agree on it
        assert!(crate::mods::chain(&mods, "Linked").is_ok());
        assert_eq!(layers(&Vfs::mount_install(&t.0, Some("Linked")).unwrap()), ["Linked", "Real", "Data"]);
    }

    #[test]
    fn a_link_to_a_sibling_mod_is_followed_as_a_parent_and_one_that_leaves_the_mods_folder_is_refused_as_a_parent() {
        let t = Tmp::new("links");
        t.write("Data/Mods/Real/info.ini", "[mod]\n");
        t.write("Data/Mods/Real/odf/r.odf", "real");
        symlink(t.0.join("Data/Mods/Real"), t.0.join("Data/Mods/Alias")).unwrap();
        symlink(t.0.join("outside"), t.0.join("Data/Mods/Escape")).unwrap();
        t.write("Data/Mods/ViaAlias/info.ini", "[mod]\nParentMod=Alias\n");
        t.write("Data/Mods/Child/info.ini", "[mod]\nParentMod=Escape\n");
        // A parent that is a link to a sibling mod is followed.
        let v = Vfs::mount_install(&t.0, Some("ViaAlias")).unwrap();
        assert_eq!(v.read("odf/r.odf").map(|r| r.unwrap()), Some(b"real".to_vec()));
        // A parent entry that is a link leaving the mods folder is refused, naming the child that asked.
        match Vfs::mount_install(&t.0, Some("Child")) {
            Err(VfsError::ModOutsideMods { name, asked_by, target }) => {
                assert_eq!(name, "Escape");
                assert!(asked_by.contains("Child") && asked_by.contains("ParentMod"), "{asked_by}");
                assert!(target.ends_with("outside"), "{}", target.display());
            }
            other => panic!("{:?}", other.map(|v| layers(&v).iter().map(|s| s.to_string()).collect::<Vec<_>>())),
        }
    }

    #[test]
    fn the_mod_the_player_names_may_be_a_link_to_a_folder_elsewhere() {
        // A mod kept on another drive and linked in: the name is the player's own choice (D1).
        let t = Tmp::new("named_link");
        t.write("Data/odf/base.odf", "base");
        symlink(t.0.join("outside"), t.0.join("Data/Mods/Linked")).unwrap();
        let v = Vfs::mount_install(&t.0, Some("Linked")).unwrap();
        assert_eq!(layers(&v), ["Linked", "Data"]);
        assert_eq!(v.read("odf/secret.odf").map(|r| r.unwrap()), Some(b"outside".to_vec()));
        assert!(mod_chain(&t.data(), "linked").is_ok());
    }

    #[test]
    fn a_linked_mods_parent_is_looked_up_under_the_installs_mods_folder_not_next_to_the_target() {
        let t = Tmp::new("linked_parent");
        t.write("elsewhere/Linked/info.ini", "[mod]\nParentMod=Sib\n");
        t.write("elsewhere/Sib/info.ini", "[mod]\n");
        t.write("elsewhere/Sib/odf/s.odf", "next to the target");
        symlink(t.0.join("elsewhere/Linked"), t.0.join("Data/Mods/Linked")).unwrap();
        // `Sib` exists next to the link's target but not in the install's mods folder: not found.
        assert!(matches!(Vfs::mount_install(&t.0, Some("Linked")), Err(VfsError::ModNotFound(n)) if n == "Sib"));
        // Once it is in the mods folder it is the one mounted.
        t.write("Data/Mods/Sib/info.ini", "[mod]\n");
        t.write("Data/Mods/Sib/odf/s.odf", "in the mods folder");
        let v = Vfs::mount_install(&t.0, Some("Linked")).unwrap();
        assert_eq!(v.read("odf/s.odf").map(|r| r.unwrap()), Some(b"in the mods folder".to_vec()));
        // A linked parent that leaves the mods folder is refused even when a linked mod asks for it.
        symlink(t.0.join("elsewhere/Sib"), t.0.join("Data/Mods/Far")).unwrap();
        t.write("elsewhere/Linked/info.ini", "[mod]\nParentMod=Far\n");
        assert!(matches!(Vfs::mount_install(&t.0, Some("Linked")), Err(VfsError::ModOutsideMods { name, .. }) if name == "Far"));
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
