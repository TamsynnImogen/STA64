//! Install-gated checks of the coverage report (CON-34, D7). With
//! `STA64_TEST_INSTALL` set to an install root (Data/Mods/...), every
//! installed mod is reported, and STA2 Classic Enhanced lists its 8 changed
//! cfg lines as ignored.

use std::path::Path;

use sta_content::coverage;
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

#[test]
fn classic_enhanced_lists_its_six_unread_cfg_lines_as_ignored() {
    let Some(root) = root() else { return };
    let vfs = match Vfs::mount_install(Path::new(&root), Some("STA2 Classic Mod Enhanced Gameplay")) {
        Ok(v) => v,
        Err(_) => return, // not installed here
    };
    let r = coverage::check(&vfs);
    let mut ignored: Vec<&str> = r.cfg.iter().filter(|c| !c.read).map(|c| c.key.as_str()).collect();
    ignored.sort();
    assert_eq!(
        ignored,
        [
            "BASE_CREW_PER_SEC",
            "HARD_AI_COST_HANDICAP",
            "HARD_AI_SPEED_HANDICAP",
            "REPAIR_SHIP_DISTANCE",
            "SHIELD_DAMAGE_PROTECTION",
            "cfgFREIGHTERS_PER_NEBULA",
        ]
    );
    // `why` names the layer and line for the file and for a key.
    let hits = coverage::why_file(&vfs, "RTS_CFG.h");
    assert_eq!(hits[0].layer, "STA2 Classic Mod Enhanced Gameplay");
    assert!(hits.iter().any(|h| h.layer == "STA2 Classic Mod"));
    let w = coverage::why_key(&vfs, "RTS_CFG.h", "SHIELD_DAMAGE_PROTECTION").unwrap();
    assert_eq!((w.layer.as_str(), w.value.as_str()), ("STA2 Classic Mod Enhanced Gameplay", "0.9"));
    assert!(w.line > 0);
}
