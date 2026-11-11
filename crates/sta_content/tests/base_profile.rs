use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sta_content::{ContentDb, OdfIndex, Profile};
use sta_vfs::{Vfs, zfs};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        loop {
            let path = std::env::temp_dir().join(format!(
                "sta64-base-profile-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot create temporary test directory {path:?}: {e}"),
            }
        }
    }

    fn file(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn dir(&self, path: &str) {
        fs::create_dir_all(self.0.join(path)).unwrap();
    }

    fn zfs(&self) {
        let bytes = zfs::write(
            &[zfs::ZfsFile {
                name: "a.odf".into(),
                data: b"x".to_vec(),
                time: 0,
            }],
            0x9466_279b,
        )
        .unwrap();
        self.file("Data/assets.zfs", bytes);
    }

    fn mount(&self, active_mod: Option<&str>) -> Vfs {
        Vfs::mount_install(&self.0, active_mod).unwrap()
    }

    fn detect(&self, active_mod: Option<&str>) -> Profile {
        Profile::detect(&self.mount(active_mod))
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn base_data_authored_name_selects_fo4_with_case_insensitive_section_and_value() {
    for ini in [
        "[Mod]\nName = Fleet Operations: Roots\n",
        "[mOd]\nnAmE = fLeEt oPeRaTiOnS: rOoTs\n",
        "[Mod]\nName = Fo4 test\n",
    ] {
        let t = Tmp::new();
        t.file("Data/info.ini", ini);
        assert_eq!(t.detect(None), Profile::Fo4, "{ini}");
    }
}

#[test]
fn stock_unknown_and_empty_data_stay_a2_and_empty_vfs_stays_native() {
    for info in [
        None,
        Some("[Mod]\nName = Stock Armada II\n"),
        Some("[Mod]\nName = Unrelated Mod\n"),
    ] {
        let t = Tmp::new();
        t.dir("Data");
        if let Some(info) = info {
            t.file("Data/info.ini", info);
        }
        assert_eq!(t.detect(None), Profile::Sta2Classic, "{info:?}");
    }
    assert_eq!(Profile::detect(&Vfs::default()), Profile::Native);
}

#[test]
fn plain_child_mod_inherits_fo_name_from_base_data() {
    let t = Tmp::new();
    t.file("Data/info.ini", "[Mod]\nName = Fleet Operations: Roots\n");
    t.file("Data/Mods/Child/info.ini", "[Mod]\nName = Plain Child\n");
    assert_eq!(t.detect(Some("Child")), Profile::Fo4);
}

#[test]
fn valid_explicit_profiles_in_base_or_child_outrank_fo_names_anywhere() {
    for (value, expected) in [
        ("Native", Profile::Native),
        ("Sta2Classic", Profile::Sta2Classic),
        ("Sta1Classic", Profile::Sta1Classic),
    ] {
        let base = Tmp::new();
        base.file(
            "Data/info.ini",
            format!("[Mod]\nName=Fleet Operations: Roots\nProfile={value}\n"),
        );
        assert_eq!(base.detect(None), expected, "base {value}");

        let child = Tmp::new();
        child.file("Data/info.ini", "[Mod]\nName=Fleet Operations: Roots\n");
        child.file(
            "Data/Mods/Child/info.ini",
            format!("[Mod]\nName=Plain Child\nProfile={value}\n"),
        );
        assert_eq!(child.detect(Some("Child")), expected, "child {value}");

        let parent_profile = Tmp::new();
        parent_profile.file("Data/info.ini", format!("[Mod]\nProfile={value}\n"));
        parent_profile.file("Data/Mods/Child/info.ini", "[Mod]\nName=Fleet Operations: Child\n");
        assert_eq!(
            parent_profile.detect(Some("Child")),
            expected,
            "base Profile versus child FO Name {value}"
        );
    }
    let priority = Tmp::new();
    priority.file(
        "Data/info.ini",
        "[Mod]\nName=Fleet Operations: Roots\nProfile=Sta2Classic\n",
    );
    priority.file("Data/Mods/Child/info.ini", "[Mod]\nProfile=Native\n");
    assert_eq!(
        priority.detect(Some("Child")),
        Profile::Native,
        "first valid explicit profile is the child"
    );
    priority.file("Data/Mods/Child/info.ini", "[Mod]\nProfile=unknown\n");
    assert_eq!(
        priority.detect(Some("Child")),
        Profile::Sta2Classic,
        "invalid child value does not mask valid base profile"
    );
}

#[test]
fn valid_a1_zfs_signature_outweighs_fo_name_but_explicit_profile_wins() {
    let t = Tmp::new();
    t.zfs();
    t.file("Data/info.ini", "[Mod]\nName=Fleet Operations: Roots\n");
    assert_eq!(t.detect(None), Profile::Sta1Classic);
    t.file("Data/info.ini", "[Mod]\nName=Fleet Operations: Roots\nProfile=Native\n");
    assert_eq!(t.detect(None), Profile::Native);
}

#[test]
fn existing_non_data_name_folder_and_root_loader_hints_still_select_fo4() {
    let named = Tmp::new();
    named.dir("Data");
    named.file("Data/Mods/Plain/info.ini", "[Mod]\nName=Fleet Operations Addon\n");
    assert_eq!(named.detect(Some("Plain")), Profile::Fo4);

    let folder = Tmp::new();
    folder.dir("Data");
    folder.file("Data/Mods/FleetOps/info.ini", "[Mod]\nName=Plain Addon\n");
    assert_eq!(folder.detect(Some("FleetOps")), Profile::Fo4);

    let loader = Tmp::new();
    loader.dir("Data");
    loader.file("mmm_loader.dll", b"");
    assert_eq!(loader.detect(None), Profile::Fo4);
}

#[test]
fn contentdb_load_uses_fo_defaults_and_preserves_mod_then_odf_zero_overrides() {
    let plain = Tmp::new();
    plain.file("Data/info.ini", "[Mod]\nName=Fleet Operations: Roots\n");
    plain.file("Data/odf/ship.odf", "classLabel=\"craft\"\nunitName=\"Ship\"\n");
    let vfs = plain.mount(None);
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    assert_eq!(db.profile(), Profile::Fo4);
    assert!(db.features().extra_resources);
    assert!(db.features().weapon_damage_controls);
    let ship = db.unit(db.key("ship").unwrap()).unwrap();
    assert!(db.unit_features(ship).weapon_damage_controls);

    let overridden = Tmp::new();
    overridden.file(
        "Data/info.ini",
        "[Mod]\nName=Fleet Operations: Roots\n[Features]\nextra_resources=0\nweapon_damage_controls=1\n",
    );
    overridden.file(
        "Data/odf/ship.odf",
        "classLabel=\"craft\"\nunitName=\"Ship\"\nFeature_weapon_damage_controls=0\n",
    );
    let vfs = overridden.mount(None);
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    assert_eq!(db.profile(), Profile::Fo4);
    assert!(!db.features().extra_resources);
    assert!(db.features().weapon_damage_controls);
    let ship = db.unit(db.key("ship").unwrap()).unwrap();
    assert!(!db.unit_features(ship).weapon_damage_controls);
}
