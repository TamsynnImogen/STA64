//! CON-43: profile detection by `AssetVersion`, the host base `Data/info.ini`
//! and `Data/packages.ini` (docs/FO_PROFILE_DETECTION.md, amended 2026-11-12).
//! Synthetic fixtures only; the installed-content checks are in
//! `profile_install.rs`.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sta_content::{ContentDb, DetectStep, Detection, OdfIndex, Profile};
use sta_vfs::{Vfs, zfs};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("sta64-asset-version-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn file(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn zfs(&self) {
        let bytes = zfs::write(&[zfs::ZfsFile { name: "a.odf".into(), data: b"x".to_vec(), time: 0 }], 0x9466_279b).unwrap();
        self.file("Data/assets.zfs", bytes);
    }

    fn mount(&self, active_mod: Option<&str>) -> Vfs {
        Vfs::mount_install(&self.0, active_mod).unwrap()
    }

    fn detect(&self, active_mod: Option<&str>) -> Detection {
        Profile::detect_with_source(&self.mount(active_mod))
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const ROOTS_BASE: &str = "[mod]\nName=Fleet Operations: Roots\nAssetVersion=50000\nstandalone=1\n";

fn mod_with(t: &Tmp, name: &str, info: &str) {
    t.file(&format!("Data/Mods/{name}/info.ini"), info);
}

fn layer_names(vfs: &Vfs) -> Vec<&str> {
    vfs.layers().iter().map(|l| l.name.as_str()).collect()
}

// Step 3: AssetVersion in the mod's own info.ini.

#[test]
fn asset_version_threshold_selects_the_roots_lineage() {
    for (value, roots) in [("50000", true), ("50001", true), ("60000", true), ("49999", false), ("30205", false), ("0", false)] {
        let t = Tmp::new();
        t.file("Data/odf/x.odf", "classLabel=\"craft\"\n");
        mod_with(&t, "M", &format!("[mod]\nName=Some Mod\nstandalone=1\nAssetVersion={value}\n"));
        let d = t.detect(Some("M"));
        if roots {
            assert_eq!(d.profile, Profile::Fo4, "{value}");
            assert_eq!(d.step, DetectStep::AssetVersion, "{value}");
            assert_eq!(d.file.as_deref(), Some(t.0.join("Data/Mods/M/info.ini").as_path()));
            assert_eq!(d.detail, format!("AssetVersion = {value}"));
            assert!(d.warnings.is_empty(), "{value}: {:?}", d.warnings);
        } else {
            assert_ne!(d.profile, Profile::Fo4, "{value} is not the Roots lineage");
            assert_ne!(d.step, DetectStep::AssetVersion, "{value}");
        }
    }
}

#[test]
fn asset_version_is_read_with_bom_crlf_quotes_case_and_inline_comments() {
    for body in [
        "\u{feff}[Mod]\r\nName=x\r\nAssetVersion = 50000\r\n",
        "[MOD]\nassetversion=\"50000\"\n",
        "[mod]\nASSETVERSION = ' 50000 '\n",
        "[mod]\nAssetVersion=50000 ; Fleet Ops 5.0\n",
        "[mod]\nAssetVersion=50000\nAssetVersion=30205\n",
    ] {
        let t = Tmp::new();
        t.file("Data/Mods/M/INFO.INI", body);
        let d = t.detect(Some("M"));
        assert_eq!((d.profile, d.step), (Profile::Fo4, DetectStep::AssetVersion), "{body:?}");
    }
}

#[test]
fn a_child_inherits_the_asset_version_of_its_mod_parent_and_the_top_most_value_decides() {
    let t = Tmp::new();
    t.file("Data/odf/x.odf", "classLabel=\"craft\"\n");
    mod_with(&t, "Parent", "[mod]\nName=Parent\nstandalone=1\nAssetVersion=50000\n");
    mod_with(&t, "Child", "[mod]\nName=Child\nParentMod=Parent\n");
    let d = t.detect(Some("Child"));
    assert_eq!((d.profile, d.step), (Profile::Fo4, DetectStep::AssetVersion));
    assert!(d.file.unwrap().ends_with("Parent/info.ini"), "the layer that states it");

    // A child that states an older value does not select the lineage by itself.
    mod_with(&t, "Old", "[mod]\nName=Old\nParentMod=Parent\nAssetVersion=30205\n");
    assert_ne!(t.detect(Some("Old")).step, DetectStep::AssetVersion);
}

// Step 1 and the ZFS rule keep their place.

#[test]
fn explicit_profile_beats_asset_version() {
    for (profile, expected) in [("Native", Profile::Native), ("Sta2Classic", Profile::Sta2Classic), ("Sta1Classic", Profile::Sta1Classic)] {
        let t = Tmp::new();
        mod_with(&t, "M", &format!("[mod]\nstandalone=1\nAssetVersion=50000\nProfile={profile}\n"));
        let d = t.detect(Some("M"));
        assert_eq!((d.profile, d.step), (expected, DetectStep::Explicit), "{profile} in info.ini");

        let t = Tmp::new();
        mod_with(&t, "M", "[mod]\nstandalone=1\nAssetVersion=50000\n");
        t.file("Data/Mods/M/manifest.toml", format!("[mod]\nid = \"m\"\nprofile = \"{profile}\"\n"));
        let d = t.detect(Some("M"));
        assert_eq!((d.profile, d.step), (expected, DetectStep::Explicit), "{profile} in manifest.toml");
        assert!(d.file.unwrap().ends_with("manifest.toml"));
    }
    // A Data-layer explicit profile beats a mod's AssetVersion as well.
    let t = Tmp::new();
    t.file("Data/info.ini", "[mod]\nProfile=Sta2Classic\n");
    mod_with(&t, "M", "[mod]\nAssetVersion=50000\n");
    assert_eq!(t.detect(Some("M")).profile, Profile::Sta2Classic);
}

#[test]
fn a_zfs_archive_still_wins_over_every_new_step() {
    let t = Tmp::new();
    t.zfs();
    t.file("Data/info.ini", ROOTS_BASE);
    t.file("Data/packages.ini", "[FleetOps]\nVersion=3.2.7\n");
    mod_with(&t, "M", "[mod]\nAssetVersion=50000\n");
    for m in [None, Some("M")] {
        let d = t.detect(m);
        assert_eq!((d.profile, d.step), (Profile::Sta1Classic, DetectStep::Zfs), "{m:?}");
    }
    // ... and an explicit Profile still wins over the ZFS rule.
    mod_with(&t, "M", "[mod]\nAssetVersion=50000\nProfile=Native\n");
    assert_eq!(t.detect(Some("M")).profile, Profile::Native);
}

// Step 4: the host's base Data/info.ini, identity only.

#[test]
fn host_base_info_names_roots_for_a_mod_without_asset_version_and_is_not_mounted() {
    let t = Tmp::new();
    t.file("Data/info.ini", "[mod]\nName=Fleet Operations: Roots\n[Features]\nextra_resources=0\n");
    t.file("Data/odf/only_in_base.odf", "classLabel=\"craft\"\n");
    mod_with(&t, "M", "[mod]\nName=Plain Standalone\nstandalone=1\n");

    let vfs = t.mount(Some("M"));
    assert_eq!(layer_names(&vfs), ["M"], "D6: base Data is not mounted under a standalone mod");
    let d = Profile::detect_with_source(&vfs);
    assert_eq!((d.profile, d.step), (Profile::Fo4, DetectStep::HostBaseInfo));
    assert_eq!(d.file.as_deref(), Some(t.0.join("Data/info.ini").as_path()));
    assert!(d.warnings.is_empty());
    assert_eq!(layer_names(&vfs), ["M"], "detection did not change the layers");
    let (index, _) = OdfIndex::build(&vfs);
    assert!(index.get("only_in_base").is_none(), "base ODFs did not leak");
    assert!(sta_content::Overrides::from_vfs(&vfs).is_empty(), "base [Features] did not leak");

    // An add-on mod mounts over Data as before and detects the same way.
    mod_with(&t, "Addon", "[mod]\nName=Plain Addon\n");
    let vfs = t.mount(Some("Addon"));
    assert_eq!(layer_names(&vfs), ["Addon", "Data"]);
    assert_eq!(Profile::detect(&vfs), Profile::Fo4);
}

#[test]
fn host_base_info_not_naming_fo_changes_nothing() {
    let t = Tmp::new();
    t.file("Data/info.ini", "[mod]\nName=Stock Armada II\n");
    t.file("Data/odf/x.odf", "classLabel=\"craft\"\n");
    mod_with(&t, "Addon", "[mod]\nName=Plain Addon\n");
    let d = t.detect(Some("Addon"));
    assert_eq!((d.profile, d.step), (Profile::Sta2Classic, DetectStep::Heuristic));
}

#[test]
fn a_stated_old_asset_version_under_a_roots_host_is_not_roots() {
    let t = Tmp::new();
    t.file("Data/info.ini", "[mod]\nName=Fleet Operations: Roots\n");
    t.file("Data/odf/ship.odf", "classLabel=\"craft\"\nunitName=\"Ship\"\n");
    mod_with(&t, "Old", "[mod]\nName=Designed For FO 3.2.5\nstandalone=1\nAssetVersion=30205\n");
    let d = t.detect(Some("Old"));
    assert_ne!(d.profile, Profile::Fo4, "a mod designed for FO 3.x is not a Roots mod");
    assert_ne!(d.step, DetectStep::HostBaseInfo);
    assert!(d.warnings.is_empty(), "no Roots lineage, so nothing is stale: {:?}", d.warnings);
}

#[test]
fn an_old_asset_version_on_a_mod_loading_as_roots_warns_and_still_loads() {
    // An add-on over a Roots base whose Data names Fleet Operations: the mounted Data layer
    // makes the load Roots (existing name heuristic); the mod's own old value warns.
    let t = Tmp::new();
    t.file("Data/info.ini", "[mod]\nName=Fleet Operations: Roots\n");
    t.file("Data/odf/ship.odf", "classLabel=\"craft\"\nunitName=\"Ship\"\n");
    mod_with(&t, "Old", "[mod]\nName=Designed For FO 3.2.5\nAssetVersion=30205\n");
    let vfs = t.mount(Some("Old"));
    let d = Profile::detect_with_source(&vfs);
    assert_eq!((d.profile, d.step), (Profile::Fo4, DetectStep::Heuristic));
    assert_eq!(d.warnings.len(), 1, "{:?}", d.warnings);
    assert!(d.warnings[0].message.contains("30205") && d.warnings[0].message.contains("older"), "{:?}", d.warnings[0]);
    assert!(std::path::Path::new(&d.warnings[0].file).ends_with("Old/info.ini"));

    // The warning reaches ContentDb::load; the load itself succeeds.
    let (index, _) = OdfIndex::build(&vfs);
    let (db, warnings) = ContentDb::load(&vfs, &index).unwrap();
    assert_eq!(db.profile(), Profile::Fo4);
    assert!(warnings.iter().any(|w| w.message.contains("AssetVersion 30205")), "{warnings:?}");

    // No warning at or above the lineage, and none outside it.
    mod_with(&t, "Old", "[mod]\nAssetVersion=50000\n");
    assert!(t.detect(Some("Old")).warnings.is_empty());
    let plain = Tmp::new();
    plain.file("Data/odf/x.odf", "classLabel=\"craft\"\n");
    mod_with(&plain, "Old", "[mod]\nAssetVersion=30205\n");
    let d = plain.detect(Some("Old"));
    assert_eq!(d.profile, Profile::Sta2Classic);
    assert!(d.warnings.is_empty(), "an old AssetVersion on a non-Roots load is not stale");
}

#[test]
fn host_data_is_only_taken_from_a_folder_named_data() {
    let t = Tmp::new();
    t.file("elsewhere/info.ini", "[mod]\nName=Fleet Operations: Roots\n");
    t.file("elsewhere/Mods/M/info.ini", "[mod]\nName=Plain\n");
    let mut vfs = Vfs::default();
    vfs.push_dir("M".into(), t.0.join("elsewhere/Mods/M")).unwrap();
    assert_ne!(Profile::detect(&vfs), Profile::Fo4);
}

#[test]
fn an_older_asset_version_with_an_explicit_roots_profile_warns_but_loads() {
    let t = Tmp::new();
    mod_with(&t, "M", "[mod]\nstandalone=1\nProfile=fo4\nAssetVersion=30205\n");
    let d = t.detect(Some("M"));
    assert_eq!((d.profile, d.step), (Profile::Fo4, DetectStep::Explicit));
    assert_eq!(d.warnings.len(), 1);
}

#[test]
fn a_malformed_asset_version_is_ignored_with_a_warning() {
    for bad in ["abc", "-50000", "5.0", "99999999999999999999", "0x50000", "50 000"] {
        let t = Tmp::new();
        t.file("Data/odf/x.odf", "classLabel=\"craft\"\n");
        mod_with(&t, "M", &format!("[mod]\nName=Some Mod\nAssetVersion={bad}\n"));
        let d = t.detect(Some("M"));
        assert_eq!((d.profile, d.step), (Profile::Sta2Classic, DetectStep::Heuristic), "{bad}");
        assert_eq!(d.warnings.len(), 1, "{bad}: {:?}", d.warnings);
        assert!(d.warnings[0].message.contains("not a whole number") && d.warnings[0].message.contains(bad), "{bad}");

        // The value is dropped, not fatal: an FO name still decides, as before.
        mod_with(&t, "M", &format!("[mod]\nName=Fleet Operations Addon\nAssetVersion={bad}\n"));
        let d = t.detect(Some("M"));
        assert_eq!((d.profile, d.step), (Profile::Fo4, DetectStep::Heuristic), "{bad}");
        assert_eq!(d.warnings.len(), 1, "{bad}");
    }
}

// Step 5: packages.ini (CON-44: a 3.x version selects the FO 3.2.7 lineage).

#[test]
fn packages_ini_version_3_selects_the_fo3_lineage() {
    let t = Tmp::new();
    t.file("Data/odf/x.odf", "classLabel=\"craft\"\n");
    t.file("Data/Packages.INI", "[FleetOps]\r\nVersion=3.2.7\r\nMMpack=false\r\n");
    let d = t.detect(None);
    assert_eq!(d.fleetops_version.as_deref(), Some("3.2.7"));
    assert_eq!((d.profile, d.step), (Profile::Fo3, DetectStep::PackagesIni));
    assert!(d.file.as_deref().is_some_and(|p| p.file_name().unwrap().eq_ignore_ascii_case("packages.ini")));
    assert!(d.to_string().contains("Version=3.2.7"), "{d}");

    // A mod of the 3.x install (no info.ini of its own, or one without AssetVersion)
    // runs on the 3.x engine; the host Data carries the version.
    mod_with(&t, "M", "[mod]\nName=Plain\nstandalone=1\n");
    let d = t.detect(Some("M"));
    assert_eq!((d.profile, d.fleetops_version.as_deref()), (Profile::Fo3, Some("3.2.7")));
    // A mod designed for 3.2.5 (AssetVersion 30205) is a 3.x mod too.
    mod_with(&t, "M", "[mod]\nAssetVersion=30205\nstandalone=1\n");
    assert_eq!(t.detect(Some("M")).profile, Profile::Fo3);
}

#[test]
fn only_a_major_of_3_selects_fo3_and_the_steps_before_it_still_win() {
    for (v, want) in [("4.0.0", Profile::Sta2Classic), ("5.1.0", Profile::Sta2Classic), ("30.2", Profile::Sta2Classic), ("3", Profile::Fo3), ("3.0", Profile::Fo3)] {
        let t = Tmp::new();
        t.file("Data/odf/x.odf", "classLabel=\"craft\"\n");
        t.file("Data/packages.ini", format!("[FleetOps]\nVersion={v}\n"));
        assert_eq!(t.detect(None).profile, want, "{v}");
    }
    let t = Tmp::new();
    t.file("Data/packages.ini", "[FleetOps]\nVersion=3.2.7\n");
    t.file("Data/odf/x.odf", "classLabel=\"craft\"\n");
    // Explicit Profile= wins; a Roots AssetVersion wins; the A1 ZFS rule wins (see above).
    mod_with(&t, "E", "[mod]\nProfile=Sta2Classic\n");
    assert_eq!(t.detect(Some("E")).profile, Profile::Sta2Classic);
    mod_with(&t, "R", "[mod]\nAssetVersion=50000\n");
    assert_eq!(t.detect(Some("R")).profile, Profile::Fo4);
    // The name matcher of the base Data/info.ini (step 4) is ahead of it.
    t.file("Data/info.ini", "[mod]\nName=Fleet Operations: Roots\n");
    mod_with(&t, "P", "[mod]\nName=Plain\nstandalone=1\n");
    assert_eq!(t.detect(Some("P")).profile, Profile::Fo4);
}

#[test]
fn missing_or_unparsable_packages_ini_is_ignored() {
    let t = Tmp::new();
    t.file("Data/odf/x.odf", "classLabel=\"craft\"\n");
    assert_eq!(t.detect(None).fleetops_version, None, "missing");
    for body in [&b"\xff\xfe\x00garbage"[..], b"[FleetOps]\nVersion=\n", b"[FleetOps]\nVersion=abc\n", b"[Other]\nVersion=3.2.7\n", b"not an ini"] {
        t.file("Data/packages.ini", body);
        let d = t.detect(None);
        assert_eq!(d.fleetops_version, None, "{body:?}");
        assert_eq!(d.profile, Profile::Sta2Classic);
        assert!(d.warnings.is_empty());
    }
}

// `[modules]` is not read.

#[test]
fn a_modules_section_is_ignored_without_warnings() {
    let t = Tmp::new();
    t.file("Data/odf/ship.odf", "classLabel=\"craft\"\nunitName=\"Ship\"\n");
    mod_with(&t, "M", "[mod]\nName=Fleet Operations\nstandalone=1\nAssetVersion=50000\n[modules]\nactive0=\"A2FOFeaturePack\"\nrequired0=\"Missing\"\n");
    let vfs = t.mount(Some("M"));
    let d = Profile::detect_with_source(&vfs);
    assert_eq!(d.profile, Profile::Fo4);
    assert!(d.warnings.is_empty());
}

// The existing outcomes did not move.

#[test]
fn without_any_new_data_the_old_heuristics_decide_as_before() {
    let t = Tmp::new();
    t.file("Data/odf/x.odf", "classLabel=\"craft\"\n");
    let d = t.detect(None);
    assert_eq!((d.profile, d.step, d.file), (Profile::Sta2Classic, DetectStep::Heuristic, None));
    let d = Profile::detect_with_source(&Vfs::default());
    assert_eq!((d.profile, d.step), (Profile::Native, DetectStep::Heuristic));

    let t = Tmp::new();
    t.file("Data/info.ini", "[mod]\nName=Fleet Operations: Roots\n");
    let d = t.detect(None);
    assert_eq!((d.profile, d.step), (Profile::Fo4, DetectStep::HostBaseInfo), "a base naming FO is still Roots");
}
