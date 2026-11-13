//! Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD; skipped when unset):
//! `warpSpeed` loads into `MoveStats::warp_speed` (SIM-38). The Federation
//! destroyer `fdestroy` has a positive warp speed above its impulse speed in
//! the stock lineage's data; the counts are printed.

use std::path::Path;

use sta_content::{ContentDb, Fixed, OdfIndex};
use sta_vfs::Vfs;

#[test]
fn warp_speed_loads_from_the_install() {
    let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else {
        return;
    };
    let vfs = Vfs::mount_install(Path::new(&root), Some(&m)).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let with_warp = db.units().filter(|u| u.movement.warp_speed > Fixed::ZERO).count();
    eprintln!("{m}: {} units, {with_warp} with warpSpeed", db.units().count());
    assert!(with_warp > 0, "no unit carries warpSpeed");
    if let Some(d) = db.key("fdestroy").and_then(|k| db.unit(k)) {
        assert!(d.movement.warp_speed > d.movement.max_speed, "warp {:?} impulse {:?}", d.movement.warp_speed, d.movement.max_speed);
    }
}
