//! Install-gated (`STA64_TEST_INSTALL`, optional `STA64_TEST_MOD`; skipped when unset): the
//! explosion definitions of a mounted install load, and every reference resolves. Asserts
//! structure only (found, counts above zero, references resolve), never values.

use std::path::Path;

use sta_content::{ContentDb, OdfIndex};
use sta_vfs::Vfs;

#[test]
fn explosion_definitions_load_and_every_reference_resolves() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        return;
    };
    let vfs = Vfs::mount_install(Path::new(&root), std::env::var("STA64_TEST_MOD").ok().as_deref()).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, warnings) = ContentDb::load(&vfs, &index).unwrap();
    let table = db.explosions();
    let (mut named, mut resolved) = (0, 0);
    for unit in db.units() {
        for (name, found) in [
            (unit.fx.explosion.as_deref(), db.explosion_of(unit)),
            (unit.stats.self_destruct.as_deref(), db.self_destruct_explosion_of(unit)),
        ] {
            let Some(name) = name.map(str::trim).filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("null")) else { continue };
            named += 1;
            resolved += found.is_some() as u32;
            assert!(found.is_some(), "{}: {name} does not resolve", unit.odf);
        }
    }
    eprintln!("{named} references, {resolved} resolved, {} explosions, {} shockwaves", table.explosions().count(), table.shockwaves().count());
    assert!(named > 0 && table.explosions().count() > 0, "no explosion definitions found");
    for e in table.explosions() {
        if let Some(wave) = e.shockwave.as_deref().map(str::trim).filter(|w| !w.is_empty() && !w.eq_ignore_ascii_case("null")) {
            assert!(table.shockwave(wave).is_some(), "{}: shockwave {wave} does not resolve", e.name);
        }
    }
    assert!(!warnings.iter().any(|w| w.message.contains("Shockwave ODF unavailable")), "an explosion names a missing shockwave");
}
