//! Stock install smoke test. The proprietary ODFs are supplied by the caller
//! via STA64_TEST_INSTALL and STA64_TEST_MOD, as in the other install tests.

use std::path::Path;

use sta_content::{A1SpecialKind, A2Effect, A2Target, ContentDb, Fixed, OdfIndex, Profile};
use sta_vfs::Vfs;

#[test]
fn the_installed_a2_specials_resolve_their_launcher_and_ordnance_keys() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        return;
    };
    let chosen_mod = std::env::var("STA64_TEST_MOD")
        .ok()
        .filter(|m| !m.is_empty());
    let vfs = Vfs::mount_install(Path::new(&root), chosen_mod.as_deref()).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    if db.profile() != Profile::Sta2Classic && chosen_mod.as_deref() != Some("STA2 Classic Mod") {
        return;
    }
    let weapon = |name| db.weapon(db.key(name).unwrap()).unwrap();
    let support = db
        .weapons()
        .filter(|w| {
            w.a2_special.is_some() || w.a1_special.is_some() || w.class.0 == "phasecloakdevice"
        })
        .count();
    eprintln!("{support} loaded Armada II definitions have supported special payloads");
    let defender = weapon("gmyotron");
    assert_eq!(defender.a2_special.unwrap().effect, A2Effect::Myotron);
    assert_eq!(defender.energy_cost, Fixed::from_int(1_000));
    assert_eq!(defender.a2_special.unwrap().max_targets, 4);
    assert_eq!(
        weapon("gpoleron").a2_special.unwrap().target,
        A2Target::Entity
    );
    assert!(matches!(
        weapon("gholdbm").a2_special.unwrap().effect,
        A2Effect::HoldingBeam { .. }
    ));
    assert_eq!(
        weapon("gborgbor").a2_special.unwrap().effect,
        A2Effect::AutoAssimilator(Fixed::from_int(5))
    );
    assert_eq!(
        weapon("gshieldi").a2_special.unwrap().effect,
        A2Effect::ShieldInversion(Fixed::from_int(50))
    );
    let reflector = weapon("gcorbom").a1_special.unwrap();
    assert_eq!(reflector.kind, A1SpecialKind::Reflector);
    assert_eq!(
        reflector.reflect_chance,
        Some((
            Fixed::from_f64(f64::from(0.9_f32)),
            Fixed::from_f64(f64::from(0.3_f32))
        ))
    );
    let ship = db.unit(db.key("fbattle").unwrap()).unwrap();
    assert_eq!(
        ship.self_destruct_blast.unwrap().damage,
        Fixed::from_int(380)
    );
}
