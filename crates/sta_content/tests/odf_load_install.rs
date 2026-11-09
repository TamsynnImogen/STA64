//! Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD; skipped when unset):
//! load the content database from a real install and require that every ODF
//! of a craft class (`odf::CRAFT_CLASSES`) either has a definition or is named
//! in a warning. Prints the counts. Run it against FO4 and against A2.
//!
//! The profile `ContentDb::load` detects is checked too. `STA64_TEST_EXPECT`
//! (fo4, sta2, sta1, native) names the one the install + mod must give.
//! Stock games that have no `Data` folder (GOG's A2, A1) are plain
//! directories: `STA64_TEST_DIR` is one such folder and `STA64_TEST_DIR_EXPECT`
//! its profile.

use std::collections::BTreeSet;
use std::path::Path;

use sta_content::odf::CRAFT_CLASSES;
use sta_content::{resolve_features, ContentDb, OdfIndex, Overrides, Profile};
use sta_vfs::Vfs;

#[test]
fn every_craft_odf_yields_a_def_or_a_warning() {
    if let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) {
        let vfs = Vfs::mount_install(Path::new(&root), Some(&m)).unwrap();
        check(&vfs, "STA64_TEST_EXPECT");
    }
    if let Ok(dir) = std::env::var("STA64_TEST_DIR") {
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), dir.into()).unwrap();
        check(&vfs, "STA64_TEST_DIR_EXPECT");
    }
}

fn check(vfs: &Vfs, expect_var: &str) {
    let (index, _) = OdfIndex::build(vfs);
    let (db, warnings) = ContentDb::load(vfs, &index).unwrap();
    // `load` detects the profile from the mod chain; STA64_TEST_EXPECT
    // (fo4, sta2, sta1, native) names the one this install must give.
    eprintln!("profile {}", db.profile().name());
    assert_eq!(db.profile(), Profile::detect(vfs));
    if let Some(e) = std::env::var(expect_var).ok().and_then(|e| Profile::from_name(&e)) {
        assert_eq!(db.profile(), e);
    }
    let mod_ov = Overrides::from_vfs(vfs);
    assert_eq!(db.features(), &resolve_features(db.profile(), &[&mod_ov]));
    let own = db.units().filter(|u| db.unit_features(u) != db.features()).count();
    eprintln!("{own} units with ODF feature overrides");
    let warned: BTreeSet<String> = warnings.iter().map(|w| w.file.to_ascii_lowercase()).collect();
    let (mut crafts, mut defined, mut listed, mut missing) = (0, 0, 0, Vec::new());
    for (stem, _) in index.iter() {
        let Ok(odf) = sta_odf::resolve_layered(stem, &mut |n, v| index.load(vfs, n, v)) else {
            // Cannot resolve: `load` lists it, which the check below needs.
            if !warned.contains(&format!("{stem}.odf")) {
                missing.push(stem.to_owned());
            }
            continue;
        };
        let class = odf.class_label().unwrap_or("").trim().to_ascii_lowercase();
        if !CRAFT_CLASSES.contains(&class.as_str()) {
            continue;
        }
        crafts += 1;
        if db.key(stem).and_then(|k| db.unit(k)).is_some() {
            defined += 1;
        } else if warned.contains(&format!("{stem}.odf")) {
            listed += 1;
        } else {
            missing.push(stem.to_owned());
        }
    }
    eprintln!("{crafts} craft ODFs: {defined} defined, {listed} listed in warnings; {} defs in all, {} warnings", db.len(), warnings.len());
    assert!(crafts > 0);
    assert!(missing.is_empty(), "neither a def nor a warning: {missing:?}");
}
