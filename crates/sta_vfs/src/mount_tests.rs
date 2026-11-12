//! Mounting by the mods' own rules (CON-33, MODDING_DECISIONS D4 and D6).

use std::fs;
use std::path::{Path, PathBuf};

use super::{Vfs, VfsError};

struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("sta_vfs_mount_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("Data/Mods")).unwrap();
        Tmp(d)
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.0.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    fn mount(&self, m: &str) -> Result<Vfs, VfsError> {
        Vfs::mount_install(&self.0, Some(m))
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

fn text(v: &Vfs, p: &str) -> Option<String> {
    v.read(p).map(|r| String::from_utf8(r.unwrap()).unwrap())
}

/// Base Roots `Data` with ODFs, a tech tree and maps, plus a standalone FO 4.0 over it.
fn fo_install(name: &str) -> Tmp {
    let t = Tmp::new(name);
    t.write("Data/odf/base_only.odf", "base");
    t.write("Data/odf/shared.odf", "base shared");
    t.write("Data/techtree/base.tt", "base tt");
    t.write("Data/bzn/base_map.bzn", "base map");
    t.write("Data/sounds/beep.wav", "beep");
    t.write("Data/Mods/Fleet Ops 4.0/info.ini", "[mod]\nName=Fleet Operations\nstandalone=1\nignoreParentMaps=1\n");
    t.write("Data/Mods/Fleet Ops 4.0/odf/shared.odf", "fo shared");
    t.write("Data/Mods/Fleet Ops 4.0/bzn/fo_map.bzn", "fo map");
    t
}

#[test]
fn standalone_mod_mounts_without_base_data() {
    let t = fo_install("standalone");
    let v = t.mount("Fleet Ops 4.0").unwrap();
    assert_eq!(layers(&v), ["Fleet Ops 4.0"]);
    assert_eq!(text(&v, "odf/shared.odf").as_deref(), Some("fo shared"));
    assert_eq!(text(&v, "odf/base_only.odf"), None, "base ODF must not leak in");
    assert_eq!(text(&v, "techtree/base.tt"), None, "base tech tree must not leak in");
    assert_eq!(text(&v, "bzn/base_map.bzn"), None, "base map must not leak in");
    assert_eq!(v.list("bzn"), ["bzn/fo_map.bzn"]);
}

#[test]
fn non_standalone_mod_still_mounts_over_data() {
    let t = fo_install("addon");
    t.write("Data/Mods/Skin/info.ini", "[mod]\nName=Skin\n");
    t.write("Data/Mods/Skin/odf/shared.odf", "skin shared");
    let v = t.mount("Skin").unwrap();
    assert_eq!(layers(&v), ["Skin", "Data"]);
    assert_eq!(text(&v, "odf/shared.odf").as_deref(), Some("skin shared"));
    assert_eq!(text(&v, "odf/base_only.odf").as_deref(), Some("base"));
    assert_eq!(text(&v, "bzn/base_map.bzn").as_deref(), Some("base map"), "maps stay without ignoreParentMaps");
}

#[test]
fn standalone_parent_ends_the_chain_and_child_keeps_its_parent() {
    let t = fo_install("chain");
    t.write("Data/Mods/Enh/info.ini", "[mod]\nParentMod=\"Fleet Ops 4.0\"\nstandalone=0\nignoreParentMaps=0\n");
    t.write("Data/Mods/Enh/odf/new.odf", "new");
    let v = t.mount("Enh").unwrap();
    assert_eq!(layers(&v), ["Enh", "Fleet Ops 4.0"], "no Data below a standalone ancestor");
    assert_eq!(text(&v, "odf/new.odf").as_deref(), Some("new"));
    assert_eq!(text(&v, "bzn/fo_map.bzn").as_deref(), Some("fo map"), "parent maps stay");
    assert_eq!(text(&v, "odf/base_only.odf"), None);
}

#[test]
fn standalone_mod_does_not_follow_its_own_parent() {
    let t = fo_install("ownparent");
    t.write("Data/Mods/Lone/info.ini", "[mod]\nParentMod=Missing\nstandalone=1\n");
    let v = t.mount("Lone").unwrap();
    assert_eq!(layers(&v), ["Lone"]);
}

#[test]
fn ignore_parent_maps_hides_parent_and_data_maps_only() {
    let t = fo_install("ignoremaps");
    t.write("Data/Mods/Parent/info.ini", "[mod]\n");
    t.write("Data/Mods/Parent/bzn/parent_map.bzn", "parent map");
    t.write("Data/Mods/Parent/odf/parent.odf", "parent odf");
    t.write("Data/Mods/Child/info.ini", "[mod]\nParentMod=Parent\nignoreParentMaps=1\n");
    t.write("Data/Mods/Child/bzn/child_map.bzn", "child map");
    let v = t.mount("Child").unwrap();
    assert_eq!(layers(&v), ["Child", "Parent", "Data"]);
    assert_eq!(v.list("bzn"), ["bzn/child_map.bzn"]);
    assert_eq!(text(&v, "odf/parent.odf").as_deref(), Some("parent odf"), "only maps are hidden");
    assert_eq!(text(&v, "odf/base_only.odf").as_deref(), Some("base"));
}

#[test]
fn parent_maps_show_when_a_middle_mod_ignores_only_below_it() {
    let t = fo_install("middle");
    t.write("Data/Mods/Top/info.ini", "[mod]\nParentMod=Mid\n");
    t.write("Data/Mods/Top/bzn/top.bzn", "t");
    t.write("Data/Mods/Mid/info.ini", "[mod]\nParentMod=Low\nignoreParentMaps=1\n");
    t.write("Data/Mods/Mid/bzn/mid.bzn", "m");
    t.write("Data/Mods/Low/info.ini", "[mod]\n");
    t.write("Data/Mods/Low/bzn/low.bzn", "l");
    let v = t.mount("Top").unwrap();
    assert_eq!(v.list("bzn"), ["bzn/mid.bzn", "bzn/top.bzn"]);
}

#[test]
fn no_active_mod_mounts_data_only() {
    let t = fo_install("nomod");
    let v = Vfs::mount_install(&t.0, None).unwrap();
    assert_eq!(layers(&v), ["Data"]);
    assert_eq!(text(&v, "bzn/base_map.bzn").as_deref(), Some("base map"));
}

#[test]
fn manifest_engine_requirement_is_checked_at_mount() {
    let t = fo_install("engine");
    t.write("Data/Mods/Future/info.ini", "[mod]\n");
    t.write("Data/Mods/Future/manifest.toml", "[mod]\nid='future'\nengine='>=99'\n");
    match t.mount("Future") {
        Err(VfsError::Engine { name, requirement, .. }) => assert_eq!((name.as_str(), requirement.as_str()), ("Future", ">=99")),
        other => panic!("{:?}", other.map(|_| ())),
    }
    t.write("Data/Mods/Now/info.ini", "[mod]\n");
    t.write("Data/Mods/Now/manifest.toml", "[mod]\nid='now'\nengine='>=0.1'\n");
    t.mount("Now").unwrap();
}

#[test]
fn load_order_mount_obeys_the_same_rules() {
    let t = fo_install("order");
    let (mods, _) = crate::mods::list_mods(&t.0.join("Data")).unwrap();
    let order = crate::order::resolve(&mods, "Fleet Ops 4.0").unwrap();
    let v = order.mount(&t.0.join("Data")).unwrap();
    assert_eq!(layers(&v), ["Fleet Ops 4.0"]);
    assert_eq!(text(&v, "odf/base_only.odf"), None);
}

#[test]
fn a_manifest_that_says_standalone_mounts_without_data() {
    let t = fo_install("manifestkind");
    t.write("Data/Mods/Packed/info.ini", "[mod]\nName=Packed\n");
    t.write("Data/Mods/Packed/manifest.toml", "[mod]\nid='packed'\nkind='standalone'\n");
    assert_eq!(layers(&t.mount("Packed").unwrap()), ["Packed"]);
    // A manifest without `kind` and without dependencies is not taken for standalone.
    t.write("Data/Mods/Plain/info.ini", "[mod]\n");
    t.write("Data/Mods/Plain/manifest.toml", "[mod]\nid='plain'\n");
    assert_eq!(layers(&t.mount("Plain").unwrap()), ["Plain", "Data"]);
}

/// With STA64_TEST_INSTALL (a Roots install): every installed mod mounts by its own
/// rules; a standalone one has no `Data` layer and no map or ODF of base Data leaks in.
#[test]
fn real_install_if_available() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let data = super::find_child_ci(Path::new(&root), "Data").expect("Data folder");
    let (mods, _) = crate::mods::list_mods(&data).unwrap();
    for m in &mods {
        let v = Vfs::mount_install(Path::new(&root), Some(&m.folder)).unwrap();
        let names = layers(&v);
        let maps = v.list("bzn");
        eprintln!("{:<36} standalone={} ignoreParentMaps={} layers={names:?} maps={}", m.folder, m.standalone, m.ignore_parent_maps, maps.len());
        if m.standalone {
            assert!(!names.contains(&"Data"), "{}: base Data mounted under a standalone mod", m.folder);
            assert_eq!(names.len(), 1);
        }
        if m.ignore_parent_maps {
            let leaked = maps.iter().filter(|p| !matches!(v.resolve(p), Some((0, _)))).count();
            assert_eq!(leaked, 0, "{}: a parent or base map leaks through ignoreParentMaps", m.folder);
        }
    }
    assert!(!mods.is_empty());
}

#[test]
fn generated_manifest_kind_follows_the_standalone_flag() {
    use crate::manifest::{Manifest, ModKind};
    let t = Tmp::new("kind");
    t.write("Data/Mods/A/info.ini", "[mod]\nstandalone=1\nParentMod=B\nProfile=fo4\n");
    t.write("Data/Mods/B/info.ini", "[mod]\n");
    t.write("Data/Mods/C/info.ini", "[mod]\nParentMod=B\n");
    let (mods, _) = crate::mods::list_mods(&t.0.join("Data")).unwrap();
    let get = |n: &str| Manifest::from_info(crate::mods::find_mod(&mods, n).unwrap());
    assert_eq!(get("A").kind, ModKind::Standalone);
    assert!(get("A").dependencies.is_empty(), "a standalone mod's ParentMod is not a dependency");
    assert_eq!(get("A").profile.as_deref(), Some("fo4"));
    assert_eq!(get("B").kind, ModKind::Addon, "no parent but not standalone: over the game's data");
    assert_eq!(get("C").kind, ModKind::Addon);
}
