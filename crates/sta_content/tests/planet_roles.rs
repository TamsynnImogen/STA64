use sta_content::{ContentDb, Overrides, PlanetRole, Profile, UnitDef, Warning, odf::load_with};

const BACKGROUND: &str = "classLabel = \"planet_obj\"\n";
const PLANET: &str = "classLabel = \"planet\"\n";
const CRAFT: &str = "classLabel = \"craft\"\nunitName = \"Ship\"\n";
const WEAPON: &str = "classLabel = \"cannon\"\nwpnName = \"Gun\"\n";

fn load(
    profile: Profile,
    background: &str,
    parent: Option<&str>,
    planet: &str,
    craft: &str,
    weapon: &str,
) -> (ContentDb, Vec<Warning>) {
    let stems = vec!["background".into(), "planet".into(), "craft".into(), "weapon".into()];
    load_with(profile, &Overrides::default(), &stems, &mut |name, version| match (
        name.to_ascii_lowercase().as_str(),
        version,
    ) {
        ("background.odf", 0) => Some(background.to_owned()),
        ("background.odf", 1) => parent.map(str::to_owned),
        ("planet.odf", 0) => Some(planet.to_owned()),
        ("craft.odf", 0) => Some(craft.to_owned()),
        ("weapon.odf", 0) => Some(weapon.to_owned()),
        _ => None,
    })
    .unwrap()
}

fn role(db: &ContentDb, name: &str) -> Option<PlanetRole> {
    db.unit(db.key(name).unwrap()).unwrap().planet_role
}

#[test]
fn all_profile_class_defaults_and_unit_name_free_admission() {
    for (profile, background, planet) in [
        (Profile::Sta1Classic, Some(PlanetRole::Background), None),
        (
            Profile::Sta2Classic,
            Some(PlanetRole::Background),
            Some(PlanetRole::Producer),
        ),
        (Profile::Fo4, Some(PlanetRole::Background), Some(PlanetRole::Producer)),
        (Profile::Native, None, None),
    ] {
        let (db, warnings) = load(profile, BACKGROUND, None, PLANET, CRAFT, WEAPON);
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(role(&db, "background"), background, "{profile:?}");
        assert_eq!(role(&db, "planet"), planet, "{profile:?}");
        assert_eq!(role(&db, "craft"), None, "{profile:?}");
        assert!(
            db.unit(db.key("background").unwrap()).is_some(),
            "planet_obj admitted without unitName"
        );
    }
}

#[test]
fn producer_family_is_independent_of_colonizability_and_numeric_planet_fields() {
    let planet =
        format!("{PLANET}isColonizable = 0\ncrewModifier = 0\ncivilianPopulationModifier = 0\nmaxCrewGain = 0\n");
    for profile in [Profile::Sta2Classic, Profile::Fo4] {
        let (db, warnings) = load(profile, BACKGROUND, None, &planet, CRAFT, WEAPON);
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(role(&db, "planet"), Some(PlanetRole::Producer));
        let raw = &db.unit(db.key("planet").unwrap()).unwrap().raw;
        assert_eq!(raw.raw("isColonizable"), Some("0"));
        assert_eq!(raw.raw("maxCrewGain"), Some("0"));
    }
}

#[test]
fn native_explicit_opt_in_for_both_planet_classes() {
    for (value, expected) in [
        ("background", PlanetRole::Background),
        ("producer", PlanetRole::Producer),
    ] {
        let key = format!("STA64_planetRole = \"{value}\"\n");
        let (db, warnings) = load(
            Profile::Native,
            &format!("{BACKGROUND}{key}"),
            None,
            &format!("{PLANET}{key}"),
            CRAFT,
            WEAPON,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(role(&db, "background"), Some(expected));
        assert_eq!(role(&db, "planet"), Some(expected));
    }
}

#[test]
fn valid_values_trim_and_compare_ascii_case_and_none_opts_out() {
    for (value, expected) in [
        ("  BaCkGrOuNd  ", Some(PlanetRole::Background)),
        ("  PrOdUcEr  ", Some(PlanetRole::Producer)),
        ("  NoNe  ", None),
    ] {
        let (db, warnings) = load(
            Profile::Fo4,
            &format!("{BACKGROUND}sTa64_PlanetRole = \"{value}\"\n"),
            None,
            PLANET,
            CRAFT,
            WEAPON,
        );
        assert!(warnings.is_empty(), "{value}: {warnings:?}");
        assert_eq!(role(&db, "background"), expected, "{value}");
    }
}

#[test]
fn self_include_inherits_and_child_none_overrides_parent() {
    let parent = format!("{BACKGROUND}STA64_planetRole = \"producer\"\n");
    let (inherited, warnings) = load(
        Profile::Sta2Classic,
        "#include \"background.odf\"\n",
        Some(&parent),
        PLANET,
        CRAFT,
        WEAPON,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(role(&inherited, "background"), Some(PlanetRole::Producer));
    let (direct, warnings) = load(
        Profile::Sta2Classic,
        &format!("{BACKGROUND}sta64_PLANETrole = \" PrOdUcEr \"\n"),
        None,
        PLANET,
        CRAFT,
        WEAPON,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(direct.hash(), inherited.hash(), "equivalent layered spelling");
    let (child, warnings) = load(
        Profile::Sta2Classic,
        "#include \"background.odf\"\nSTA64_planetRole = \"none\"\n",
        Some(&parent),
        PLANET,
        CRAFT,
        WEAPON,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(role(&child, "background"), None);
    assert_eq!(
        child
            .unit(child.key("background").unwrap())
            .unwrap()
            .raw
            .raw("STA64_planetRole"),
        Some("\"none\"")
    );
    assert_ne!(inherited.hash(), child.hash());
}

#[test]
fn malformed_values_warn_with_file_and_key_but_keep_default_and_raw() {
    let (baseline, _) = load(Profile::Fo4, BACKGROUND, None, PLANET, CRAFT, WEAPON);
    for value in ["", "surprise", "true"] {
        let (db, warnings) = load(
            Profile::Fo4,
            &format!("{BACKGROUND}STA64_planetRole = \"{value}\"\n"),
            None,
            PLANET,
            CRAFT,
            WEAPON,
        );
        assert_eq!(role(&db, "background"), Some(PlanetRole::Background));
        assert_eq!(
            db.hash(),
            baseline.hash(),
            "invalid override is an effective no-op: {value}"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.file == "background.odf" && w.message.contains("STA64_planetRole")),
            "{value}: {warnings:?}"
        );
        let expected_raw = format!("\"{value}\"");
        assert_eq!(
            db.unit(db.key("background").unwrap())
                .unwrap()
                .raw
                .raw("STA64_planetRole"),
            Some(expected_raw.as_str())
        );
    }
}

#[test]
fn unsupported_unit_class_warns_and_weapon_key_is_not_read() {
    let (plain, warnings) = load(Profile::Fo4, BACKGROUND, None, PLANET, CRAFT, WEAPON);
    assert!(warnings.is_empty(), "{warnings:?}");
    let (db, warnings) = load(
        Profile::Fo4,
        BACKGROUND,
        None,
        PLANET,
        &format!("{CRAFT}STA64_planetRole = \"producer\"\n"),
        &format!("{WEAPON}STA64_planetRole = \"producer\"\n"),
    );
    assert_eq!(role(&db, "craft"), None);
    assert!(
        warnings
            .iter()
            .any(|w| w.file == "craft.odf" && w.message.contains("STA64_planetRole") && w.message.contains("craft")),
        "{warnings:?}"
    );
    assert!(!warnings.iter().any(|w| w.file == "weapon.odf"), "{warnings:?}");
    assert_eq!(
        db.weapon(db.key("weapon").unwrap())
            .unwrap()
            .raw
            .raw("STA64_planetRole"),
        Some("\"producer\"")
    );
    assert_eq!(db.hash(), plain.hash());
}

#[test]
fn standalone_unit_def_and_programmatic_constructor_leave_role_unset() {
    assert_eq!(UnitDef::new("synthetic", "planet").planet_role, None);
    let odf = sta_odf::resolve_layered("standalone", &mut |name, version| {
        (name == "standalone.odf" && version == 0).then(|| format!("{PLANET}STA64_planetRole = \"producer\"\n"))
    })
    .unwrap();
    let mut warnings = Vec::new();
    let def = sta_content::unit::unit_def("standalone", &odf, None, &mut warnings);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(def.planet_role, None, "standalone unit_def has no profile");
    assert_eq!(def.raw.raw("STA64_planetRole"), Some("\"producer\""));
}

#[test]
fn effective_hash_records_ignore_noops_and_preserve_native_fake_identity() {
    for (profile, default) in [
        (Profile::Sta1Classic, "background"),
        (Profile::Sta2Classic, "background"),
        (Profile::Fo4, "background"),
        (Profile::Native, "none"),
    ] {
        let (absent, _) = load(profile, BACKGROUND, None, PLANET, CRAFT, WEAPON);
        let (same, warnings) = load(
            profile,
            &format!("{BACKGROUND}STA64_planetRole = \"{default}\"\n"),
            None,
            PLANET,
            CRAFT,
            WEAPON,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(same.hash(), absent.hash(), "{profile:?} effective no-op");
        let changed = if default == "background" {
            "producer"
        } else {
            "background"
        };
        let (different, _) = load(
            profile,
            &format!("{BACKGROUND}STA64_planetRole = \"{changed}\"\n"),
            None,
            PLANET,
            CRAFT,
            WEAPON,
        );
        assert_ne!(different.hash(), absent.hash(), "{profile:?} effective role change");
    }
    let mut programmatic = UnitDef::new("synthetic", "planet");
    let plain = ContentDb::builder()
        .profile(Profile::Native)
        .features(Profile::Native.features())
        .unit(programmatic.clone())
        .build()
        .unwrap();
    programmatic.planet_role = Some(PlanetRole::Background);
    let background = ContentDb::builder()
        .profile(Profile::Native)
        .features(Profile::Native.features())
        .unit(programmatic.clone())
        .build()
        .unwrap();
    programmatic.planet_role = Some(PlanetRole::Producer);
    let producer = ContentDb::builder()
        .profile(Profile::Native)
        .features(Profile::Native.features())
        .unit(programmatic)
        .build()
        .unwrap();
    assert_ne!(plain.hash(), background.hash());
    assert_ne!(plain.hash(), producer.hash());
    assert_ne!(background.hash(), producer.hash());
    assert_eq!(sta_content::fake::db().hash(), 0xb5e5_d26f_e961_348e);
}
