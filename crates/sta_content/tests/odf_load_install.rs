//! Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD; skipped when unset):
//! load the content database from a real install and require that every ODF
//! of a craft class (`odf::CRAFT_CLASSES`) either has a definition or is named
//! in a warning. Prints the counts. Run it against FO4 and against A2.

use std::collections::BTreeSet;
use std::path::Path;

use sta_content::odf::CRAFT_CLASSES;
use sta_content::{ContentDb, OdfIndex, Profile};
use sta_vfs::Vfs;

#[test]
fn every_craft_odf_yields_a_def_or_a_warning() {
    let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else {
        return;
    };
    let vfs = Vfs::mount_install(Path::new(&root), Some(&m)).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, warnings) = ContentDb::load(&vfs, Profile::Fo4, &index).unwrap();
    let warned: BTreeSet<String> = warnings.iter().map(|w| w.file.to_ascii_lowercase()).collect();
    let (mut crafts, mut defined, mut listed, mut missing) = (0, 0, 0, Vec::new());
    for (stem, _) in index.iter() {
        let Ok(odf) = sta_odf::resolve_layered(stem, &mut |n, v| index.load(&vfs, n, v)) else {
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
