//! Install-gated checks of the coverage report (CON-34, D7). With
//! `STA64_TEST_INSTALL` set to an install root (Data/Mods/...), every
//! installed mod is reported, and the typed rule keys (CON-35) that
//! STA2 Classic Enhanced changes are listed as changed, compared between
//! peers but still reported as ignored until a system applies them.

use std::collections::BTreeSet;
use std::path::Path;

use sta_content::{coverage, layered, CfgRules, TYPED_CFG_KEYS};
use sta_vfs::Vfs;

fn root() -> Option<String> {
    std::env::var("STA64_TEST_INSTALL").ok()
}

#[test]
fn every_installed_mod_reports() {
    let Some(root) = root() else { return };
    let data = std::fs::read_dir(&root).unwrap().flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("data")).unwrap().path();
    let (mods, _) = sta_vfs::mods::list_mods(&data).unwrap();
    assert!(!mods.is_empty());
    for m in &mods {
        let vfs = Vfs::mount_install(Path::new(&root), Some(&m.folder)).unwrap();
        let r = coverage::check(&vfs);
        eprintln!("== {}\n{r}", m.folder);
        assert!(r.odf.uses > 0 || r.odf.files == 0, "{}: no ODF keys counted", m.folder);
        assert!(r.maps_checked > 0, "{}: no maps checked", m.folder);
    }
}

/// The keys of `files` that the top layer's own copy supplies with a value other than the
/// copy it hides (or that the hidden copy lacks): what the mod changes, read straight from
/// the layered headers and not from the report.
fn keys_the_mod_changes(vfs: &Vfs) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for file in coverage::CFG_FILES {
        let (Some(top), Some(below)) = (layered::resolve_header(vfs, file, 0), layered::resolve_header(vfs, file, 1)) else { continue };
        for (key, e) in &top.entries {
            if e.version == 0 && below.entries.get(key).map(|b| b.value.as_str()) != Some(e.value.as_str()) {
                out.insert(key.clone());
            }
        }
    }
    out
}

/// D7, D9: the typed rule keys the report lists as changed are exactly the keys STA64 types
/// as rules (`TYPED_CFG_KEYS`) that the checked mod changes, and those no system applies yet
/// (`CfgRules::KEYS`) are still reported as ignored. The report may list other changed keys
/// too (untyped ones); the comparison is over the typed keys only.
#[test]
fn reported_typed_cfg_keys_are_the_typed_rules_the_mod_changes_and_unapplied_ones_are_ignored() {
    let Some(root) = root() else { return };
    let vfs = match Vfs::mount_install(Path::new(&root), Some("STA2 Classic Mod Enhanced Gameplay")) {
        Ok(v) => v,
        Err(_) => return, // not installed here
    };
    let r = coverage::check(&vfs);
    let typed: BTreeSet<&str> = TYPED_CFG_KEYS.iter().copied().collect();
    let unapplied: BTreeSet<&str> = CfgRules::KEYS.iter().copied().collect();
    assert!(unapplied.is_subset(&typed), "CfgRules::KEYS are typed rule keys");

    let changes = keys_the_mod_changes(&vfs);
    let expected: BTreeSet<&str> = changes.iter().map(String::as_str).filter(|k| typed.contains(k)).collect();
    // Without a typed key to compare, the test would pass on two empty sets.
    assert!(!expected.is_empty(), "the checked mod changes none of the typed rule keys: nothing to compare");

    let reported_typed: BTreeSet<&str> = r.cfg.iter().map(|c| c.key.as_str()).filter(|k| typed.contains(k)).collect();
    assert_eq!(reported_typed, expected, "typed keys the report lists as changed vs typed keys the mod changes");

    // Every key the report lists is one the mod changes; the typed ones that no system applies
    // yet are ignored, the others are read.
    assert!(r.cfg.iter().all(|c| changes.contains(&c.key)), "the report lists a key the mod does not change");
    let ignored_typed: BTreeSet<&str> = r.cfg.iter().filter(|c| !c.read && typed.contains(c.key.as_str())).map(|c| c.key.as_str()).collect();
    let expected_ignored: BTreeSet<&str> = expected.intersection(&unapplied).copied().collect();
    assert_eq!(ignored_typed, expected_ignored, "typed keys reported as ignored vs changed typed keys no system applies");
    assert!(r.cfg.iter().filter(|c| typed.contains(c.key.as_str()) && !unapplied.contains(c.key.as_str())).all(|c| c.read), "an applied typed key is reported as ignored");
}

/// `why` names the layer and line for the file and for a key.
#[test]
fn why_names_the_layer_and_line_of_the_top_cfg() {
    let Some(root) = root() else { return };
    let mod_layer = "STA2 Classic Mod Enhanced Gameplay";
    let vfs = match Vfs::mount_install(Path::new(&root), Some(mod_layer)) {
        Ok(v) => v,
        Err(_) => return, // not installed here
    };
    let hits = coverage::why_file(&vfs, "RTS_CFG.h");
    assert_eq!(hits[0].layer, mod_layer);
    assert!(hits.len() > 1, "a hidden copy of the file is listed below the mod's");
    // A key the mod's own copy supplies, taken from the layered header: `why_key` names that
    // layer and the same line.
    let top = layered::resolve_header(&vfs, "RTS_CFG.h", 0).expect("the header resolves");
    let (key, e) = top.entries.iter().find(|(_, e)| e.version == 0).expect("the mod's own copy supplies a key");
    let w = coverage::why_key(&vfs, "RTS_CFG.h", key).unwrap();
    assert_eq!((w.layer.as_str(), w.line, w.value.as_str()), (mod_layer, e.line, e.value.as_str()));
    assert!(w.line > 0);
}
