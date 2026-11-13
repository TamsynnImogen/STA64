//! Install-gated (STA64_TEST_INSTALL, optional STA64_TEST_MOD; skipped when
//! unset): every planet-class ODF of the install resolves its CON-37 numerics
//! from its own keys, or the profile fallback when the key is absent.

use std::path::Path;

use sta_content::{ContentDb, OdfIndex, PlanetRole, Profile};
use sta_core::Fixed;
use sta_vfs::Vfs;

#[test]
fn installed_planets_follow_their_keys() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
    let vfs = Vfs::mount_install(Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let a2 = matches!(db.profile(), Profile::Sta2Classic | Profile::Fo4);
    let (mut background, mut producer) = (0, 0);
    for u in db.units() {
        match u.planet_role {
            Some(PlanetRole::Background) => {
                background += 1;
                let expected = match u.raw.f32("crewModifier") {
                    Some(v) => Some(Fixed::from_f64(v.to_string().parse().unwrap())),
                    None if db.profile() == Profile::Sta1Classic => Some(Fixed::from_f64(1.2)),
                    None => None,
                };
                assert_eq!(u.planet_crew_modifier, expected, "{}", u.odf);
            }
            Some(PlanetRole::Producer) => {
                producer += 1;
                assert_eq!(u.planet_colonizable, u.raw.bool("isColonizable").unwrap_or(a2), "{}", u.odf);
                let civ = u.raw.f32("civilianPopulationModifier").map(|v| Fixed::from_f64(v.to_string().parse().unwrap()));
                assert_eq!(u.planet_civilian_population_modifier, civ.unwrap_or(if a2 { Fixed::ONE } else { Fixed::ZERO }), "{}", u.odf);
            }
            _ => {}
        }
    }
    eprintln!("{root} {m}: {background} background and {producer} producer planets checked");
}
