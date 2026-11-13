//! CON-37: typed `crewModifier`, `isColonizable` and `civilianPopulationModifier`.

use sta_content::{ContentDb, Overrides, PlanetRole, Profile, UnitDef, Warning, odf::load_with};
use sta_core::Fixed;

const PLANET: &str = "classLabel = \"planet\"\n";
const BACKGROUND: &str = "classLabel = \"planet_obj\"\n";

fn load(profile: Profile, child: &str, parent: Option<&str>) -> (ContentDb, Vec<Warning>) {
    load_with(profile, &Overrides::default(), &["target".into()], &mut |name, version| match (name, version) {
        ("target.odf", 0) => Some(child.to_owned()),
        ("target.odf", 1) => parent.map(str::to_owned),
        _ => None,
    })
    .unwrap()
}

fn def(db: &ContentDb) -> &UnitDef {
    db.unit(db.key("target").unwrap()).unwrap()
}

fn fx(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

#[test]
fn per_profile_fallback_table() {
    // (profile, background crewModifier, producer isColonizable, producer civilian modifier)
    for (profile, crew, colonizable, civ) in [
        (Profile::Sta1Classic, Some(fx(1.2)), false, Fixed::ZERO),
        (Profile::Sta2Classic, None, true, Fixed::ONE),
        (Profile::Fo4, None, true, Fixed::ONE),
        (Profile::Native, None, false, Fixed::ZERO),
    ] {
        let (db, warnings) = load(profile, BACKGROUND, None);
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        let d = def(&db);
        assert_eq!(d.planet_crew_modifier, if d.planet_role.is_some() { crew } else { None }, "{profile:?}");
        assert!(!d.planet_colonizable);
        let (db, warnings) = load(profile, PLANET, None);
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        let d = def(&db);
        assert_eq!(d.planet_crew_modifier, None, "{profile:?}: producers carry no crewModifier");
        if d.planet_role == Some(PlanetRole::Producer) {
            assert_eq!(d.planet_colonizable, colonizable, "{profile:?}");
            assert_eq!(d.planet_civilian_population_modifier, civ, "{profile:?}");
        } else {
            assert!(!d.planet_colonizable);
            assert_eq!(d.planet_civilian_population_modifier, Fixed::ZERO);
        }
    }
}

#[test]
fn authored_values_win_in_every_profile() {
    for profile in [Profile::Sta1Classic, Profile::Sta2Classic, Profile::Fo4] {
        let (db, w) = load(profile, &format!("{BACKGROUND}crewModifier = 1.5\n"), None);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(def(&db).planet_crew_modifier, Some(fx(1.5)));
        if profile == Profile::Sta1Classic {
            continue; // no Producer family on Armada I
        }
        let (db, w) = load(profile, &format!("{PLANET}isColonizable = 0\ncivilianPopulationModifier = 1.5f\n"), None);
        assert!(w.is_empty(), "{w:?}");
        assert!(!def(&db).planet_colonizable);
        assert_eq!(def(&db).planet_civilian_population_modifier, fx(1.5));
    }
}

#[test]
fn explicit_and_inherited_zero_stay_zero() {
    let (db, _) = load(Profile::Sta1Classic, &format!("{BACKGROUND}crewModifier = 0\n"), None);
    assert_eq!(def(&db).planet_crew_modifier, Some(Fixed::ZERO));
    let zeros = format!("{PLANET}isColonizable = 0\ncivilianPopulationModifier = 0.0f\n");
    for profile in [Profile::Sta2Classic, Profile::Fo4] {
        let (db, w) = load(profile, &zeros, None);
        assert!(w.is_empty(), "{w:?}");
        assert!(!def(&db).planet_colonizable);
        assert_eq!(def(&db).planet_civilian_population_modifier, Fixed::ZERO);
        // A child that includes the zero parent inherits it; a child that sets a value overrides it.
        let (db, _) = load(profile, "#include \"target.odf\"\n", Some(&zeros));
        assert!(!def(&db).planet_colonizable);
        assert_eq!(def(&db).planet_civilian_population_modifier, Fixed::ZERO);
        let (db, _) = load(profile, "#include \"target.odf\"\nisColonizable = 1\ncivilianPopulationModifier = 2\n", Some(&zeros));
        assert!(def(&db).planet_colonizable);
        assert_eq!(def(&db).planet_civilian_population_modifier, fx(2.0));
    }
}

#[test]
fn invalid_values_are_diagnosed_and_use_the_profile_fallback() {
    for raw in ["garbage", "", "nan", "inf"] {
        let (db, w) = load(Profile::Sta1Classic, &format!("{BACKGROUND}crewModifier = \"{raw}\"\n"), None);
        assert_eq!(def(&db).planet_crew_modifier, Some(fx(1.2)), "{raw:?}");
        assert!(w.iter().any(|w| w.file == "target.odf" && w.message.contains("crewModifier")), "{raw:?}: {w:?}");
        let (db, w) = load(Profile::Fo4, &format!("{PLANET}civilianPopulationModifier = \"{raw}\"\n"), None);
        assert_eq!(def(&db).planet_civilian_population_modifier, Fixed::ONE, "{raw:?}");
        assert!(w.iter().any(|w| w.message.contains("civilianPopulationModifier")), "{raw:?}: {w:?}");
    }
    for raw in ["maybe", "2", ""] {
        let (db, w) = load(Profile::Sta2Classic, &format!("{PLANET}isColonizable = \"{raw}\"\n"), None);
        assert!(def(&db).planet_colonizable, "{raw:?}");
        assert!(w.iter().any(|w| w.message.contains("isColonizable")), "{raw:?}: {w:?}");
    }
}

#[test]
fn keys_on_other_roles_are_raw_only_and_not_diagnosed() {
    let junk = "crewModifier = x\nisColonizable = x\ncivilianPopulationModifier = x\n";
    for (class, role) in [("planet", "none"), ("planet_obj", "none")] {
        let (db, w) = load(Profile::Sta2Classic, &format!("classLabel = \"{class}\"\nSTA64_planetRole = \"{role}\"\n{junk}"), None);
        assert!(w.is_empty(), "{class}: {w:?}");
        let d = def(&db);
        assert_eq!(d.planet_crew_modifier, None);
        assert!(!d.planet_colonizable);
        assert_eq!(d.planet_civilian_population_modifier, Fixed::ZERO);
        assert_eq!(d.raw.raw("isColonizable"), Some("x"));
    }
    // A background planet does not read the producer keys, a producer not crewModifier.
    let (db, w) = load(Profile::Sta2Classic, &format!("{BACKGROUND}isColonizable = x\ncivilianPopulationModifier = x\n"), None);
    assert!(w.is_empty(), "{w:?}");
    assert!(!def(&db).planet_colonizable);
    let (db, w) = load(Profile::Sta2Classic, &format!("{PLANET}crewModifier = x\n"), None);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(def(&db).planet_crew_modifier, None);
    // Raw keys stay reachable.
    assert_eq!(def(&db).raw.raw("crewModifier"), Some("x"));
}

#[test]
fn hash_tracks_effective_values_and_keeps_native_fake_identity() {
    let h = |p, t: &str| load(p, t, None).0.hash();
    // Absent and explicit defaults hash alike; effective changes do not.
    assert_eq!(h(Profile::Fo4, PLANET), h(Profile::Fo4, &format!("{PLANET}isColonizable = 1\ncivilianPopulationModifier = 1\n")));
    assert_ne!(h(Profile::Fo4, PLANET), h(Profile::Fo4, &format!("{PLANET}isColonizable = 0\n")));
    assert_ne!(h(Profile::Fo4, PLANET), h(Profile::Fo4, &format!("{PLANET}civilianPopulationModifier = 0.5\n")));
    assert_eq!(h(Profile::Sta1Classic, BACKGROUND), h(Profile::Sta1Classic, &format!("{BACKGROUND}crewModifier = 1.2\n")));
    assert_ne!(h(Profile::Sta1Classic, BACKGROUND), h(Profile::Sta1Classic, &format!("{BACKGROUND}crewModifier = 0\n")));
    // Keys that are not effective (Native, no role) do not move the hash.
    assert_eq!(h(Profile::Native, PLANET), h(Profile::Native, &format!("{PLANET}isColonizable = 0\ncrewModifier = 3\n")));
    assert_eq!(sta_content::fake::db().hash(), 0xb5e5_d26f_e961_348e);
}

#[test]
fn explicit_opt_in_roles_use_the_conservative_fallbacks_on_every_profile() {
    for profile in [Profile::Sta1Classic, Profile::Sta2Classic, Profile::Fo4, Profile::Native] {
        let (db, w) = load(profile, &format!("{PLANET}STA64_planetRole = \"producer\"\n"), None);
        assert!(w.is_empty(), "{profile:?}: {w:?}");
        let a2 = matches!(profile, Profile::Sta2Classic | Profile::Fo4);
        assert_eq!(def(&db).planet_colonizable, a2, "{profile:?}");
        assert_eq!(def(&db).planet_civilian_population_modifier, if a2 { Fixed::ONE } else { Fixed::ZERO });
        let (db, _) = load(profile, &format!("{PLANET}STA64_planetRole = \"producer\"\nisColonizable = 1\ncivilianPopulationModifier = 2\n"), None);
        assert!(def(&db).planet_colonizable, "{profile:?}");
        assert_eq!(def(&db).planet_civilian_population_modifier, fx(2.0));
        let (db, _) = load(profile, &format!("{BACKGROUND}STA64_planetRole = \"background\"\n"), None);
        assert_eq!(def(&db).planet_crew_modifier, (profile == Profile::Sta1Classic).then(|| fx(1.2)), "{profile:?}");
    }
}
