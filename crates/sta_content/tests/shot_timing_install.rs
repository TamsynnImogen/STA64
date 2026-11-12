//! Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD; skipped when unset):
//! load the weapon definitions of a Roots install with a mod and count the
//! SIM-34/35 keys: `initialShotDelayModifier`, `ignoreShotDelayEffects` and
//! weapons of the FO class `ordnancedefenseweapon`. Prints the counts.
//!
//! - `STA64_TEST_EXPECT_ZERO_MODIFIER=1`: some weapon must carry
//!   `initialShotDelayModifier = 0` (STA2 Classic and Fleet Ops 4.0 both set
//!   0.0 on some of their weapons; read from the data, not measured).
//! - `STA64_TEST_EXPECT_PD=1`: at least one point-defence weapon must load.

use std::path::Path;

use sta_content::{ContentDb, Fixed, OdfIndex, WeaponKind};
use sta_vfs::Vfs;

#[test]
fn shot_timing_keys_load_from_the_install() {
    let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else {
        return;
    };
    let vfs = Vfs::mount_install(Path::new(&root), Some(&m)).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let (mut all, mut keyed, mut zero, mut ignore, mut pd, mut fo_pd) = (0, 0, 0, 0, 0, 0);
    for w in db.weapons() {
        all += 1;
        keyed += w.initial_shot_delay_modifier.is_some() as u32;
        zero += (w.initial_shot_delay_modifier == Some(Fixed::ZERO)) as u32;
        ignore += w.ignore_shot_delay_effects as u32;
        pd += (w.kind == WeaponKind::PointDefence) as u32;
        fo_pd += w.ordnance_defence as u32;
    }
    eprintln!("{m}: {all} weapons, {keyed} with initialShotDelayModifier ({zero} of them 0), {ignore} with ignoreShotDelayEffects, {pd} point defence ({fo_pd} ordnancedefenseweapon)");
    if std::env::var_os("STA64_TEST_EXPECT_ZERO_MODIFIER").is_some() {
        assert!(zero > 0, "no weapon carries initialShotDelayModifier = 0");
    }
    if std::env::var_os("STA64_TEST_EXPECT_PD").is_some() {
        assert!(pd > 0, "no point-defence weapon loaded");
    }
}
