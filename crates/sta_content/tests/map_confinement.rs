use sta_content::{
    ConfinementInitial as Initial, ConfinementOverrides as Overrides,
    ConfinementSpatial as Spatial, ContentDb, Fixed, Profile, UnitDef, Warning, odf::load_with,
};

fn built(profile: Profile, class: &str, overrides: Overrides) -> ContentDb {
    let mut unit = UnitDef::new("subject", class);
    unit.confinement = overrides;
    ContentDb::builder()
        .profile(profile)
        .unit(unit)
        .build()
        .unwrap()
}

fn rules(db: &ContentDb) -> (sta_content::ConfinementRules, sta_content::ConfinementRules) {
    let key = db.key("subject").unwrap();
    (
        *db.map_confinement(key).unwrap(),
        *db.construction_confinement(key).unwrap(),
    )
}

fn odf(profile: Profile, child: &str, parent: Option<&str>) -> (ContentDb, Vec<Warning>) {
    load_with(
        profile,
        &sta_content::Overrides::default(),
        &["subject".into()],
        &mut |name, version| match (name, version) {
            ("subject.odf", 0) => Some(child.into()),
            ("subject.odf", 1) => parent.map(str::to_owned),
            _ => None,
        },
    )
    .unwrap()
}

fn odf_rule(profile: Profile, body: &str) -> (ContentDb, Vec<Warning>) {
    odf(
        profile,
        &format!("classLabel = \"craft\"\nunitName = \"Subject\"\n{body}"),
        None,
    )
}

#[test]
fn profile_defaults_and_cached_construction_are_distinct() {
    let native = built(Profile::Native, "craft", Overrides::default());
    let (finished, construction) = rules(&native);
    assert!(!native.has_map_confinement());
    assert!(!finished.capability);
    assert_eq!(finished.spatial, Spatial::Native);
    assert_eq!(finished.initial, Initial::Disabled);
    assert!(!finished.rearm);
    assert_eq!(construction.spatial, Spatial::Plain);
    assert_eq!(sta_content::fake::db().hash(), 0x6977_6a57_dec2_2e6c);

    for (profile, expected_initial, expected_rearm) in [
        (Profile::Sta1Classic, Initial::PaddedXz, true),
        (Profile::Sta2Classic, Initial::RawXz, false),
        (Profile::Fo4, Initial::RawXz, false),
    ] {
        let db = built(profile, "craft", Overrides::default());
        let (finished, construction) = rules(&db);
        assert!(db.has_map_confinement());
        assert_eq!(finished.spatial, Spatial::Craft);
        assert_eq!(finished.initial, expected_initial);
        assert_eq!(finished.rearm, expected_rearm);
        assert_eq!(construction.spatial, Spatial::Plain);
        assert_eq!(construction.initial, expected_initial);
        assert_eq!(construction.rearm, expected_rearm);
        assert_eq!(finished.inset, Fixed::from_int(100));
        assert_eq!(finished.upper_epsilon, Fixed::from_bits(429496736));
    }
}

#[test]
fn terrain_placeholder_and_unsupported_policy() {
    for profile in [Profile::Sta1Classic, Profile::Sta2Classic, Profile::Fo4] {
        let db = built(profile, "nebula_obj", Overrides::default());
        let (finished, construction) = rules(&db);
        assert_eq!(finished.spatial, Spatial::Terrain);
        assert_eq!(construction.spatial, Spatial::Plain);
        assert_eq!(
            finished.initial,
            if profile == Profile::Sta1Classic {
                Initial::Enabled
            } else {
                Initial::RawXz
            }
        );
        assert_eq!(
            construction.initial,
            if profile == Profile::Sta1Classic {
                Initial::PaddedXz
            } else {
                Initial::RawXz
            }
        );
        assert!(!finished.rearm);
        assert_eq!(construction.rearm, profile == Profile::Sta1Classic);
    }

    let db = built(
        Profile::Sta1Classic,
        "unknown_factory",
        Overrides {
            capability: Some(true),
            ..Overrides::default()
        },
    );
    let (finished, construction) = rules(&db);
    assert_eq!(finished.spatial, Spatial::Unsupported);
    assert!(!finished.is_enabled());
    assert_eq!(construction, finished);
    assert!(!db.has_map_confinement());
    assert!(
        db.map_confinement_warnings()
            .iter()
            .any(|w| w.file == "subject.odf" && w.message.contains("no verified spatial policy"))
    );
}

#[test]
fn layered_odf_preserves_explicit_and_inherited_zero() {
    let parent = "classLabel = \"craft\"\nunitName = \"Subject\"\nSTA64_mapConfinement = 1\nSTA64_mapConfinementInitial = enabled\nSTA64_mapConfinementRearm = 1\nSTA64_mapConfinementInset = 27\nSTA64_mapConfinementUpperEpsilon = 2\n";
    let (inherited, warnings) = odf(Profile::Native, "#include \"subject.odf\"\n", Some(parent));
    assert!(warnings.is_empty(), "{warnings:?}");
    let (finished, _) = rules(&inherited);
    assert!(finished.is_enabled());
    assert_eq!(finished.initial, Initial::Enabled);
    assert!(finished.rearm);
    assert_eq!(finished.inset, Fixed::from_int(27));
    assert_eq!(finished.upper_epsilon, Fixed::from_int(2));

    let child = "#include \"subject.odf\"\nSTA64_mapConfinement = 0\nSTA64_mapConfinementInitial = 0\nSTA64_mapConfinementRearm = 0\nSTA64_mapConfinementInset = 0\nSTA64_mapConfinementUpperEpsilon = 0\n";
    let (overridden, warnings) = odf(Profile::Native, child, Some(parent));
    assert!(warnings.is_empty(), "{warnings:?}");
    let (finished, _) = rules(&overridden);
    assert!(!finished.is_enabled());
    assert_eq!(finished.initial, Initial::Disabled);
    assert!(!finished.rearm);
    assert_eq!(finished.inset, Fixed::ZERO);
    assert_eq!(finished.upper_epsilon, Fixed::ZERO);
    assert_eq!(overridden.hash(), odf_rule(Profile::Native, "").0.hash());
}

#[test]
fn malformed_overrides_warn_and_keep_defaults() {
    let baseline = odf_rule(Profile::Fo4, "").0;
    for (key, value) in [
        ("STA64_mapConfinement", "maybe"),
        ("STA64_mapConfinementInitial", "diagonal"),
        ("STA64_mapConfinementRearm", "maybe"),
        ("STA64_mapConfinementInset", "-1"),
        ("STA64_mapConfinementInset", "nan"),
        ("STA64_mapConfinementInset", "2147483648"),
        ("STA64_mapConfinementUpperEpsilon", "inf"),
        ("STA64_mapConfinementUpperEpsilon", "-0.5"),
    ] {
        let (db, warnings) = odf_rule(Profile::Fo4, &format!("{key} = {value}\n"));
        assert_eq!(rules(&db), rules(&baseline), "{key}={value}");
        assert_eq!(db.hash(), baseline.hash(), "{key}={value}");
        assert!(
            warnings
                .iter()
                .any(|w| w.file == "subject.odf" && w.message.contains(key)),
            "{key}={value}: {warnings:?}"
        );
    }
}

#[test]
fn odf_binary32_epsilon_and_inactive_settings() {
    let (default, _) = odf_rule(Profile::Fo4, "");
    let (explicit, warnings) = odf_rule(Profile::Fo4, "STA64_mapConfinementUpperEpsilon = 0.1\n");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(rules(&explicit).0.upper_epsilon.to_bits(), 429496736);
    assert_eq!(explicit.hash(), default.hash());

    let (disabled, warnings) = odf_rule(
        Profile::Native,
        "STA64_mapConfinementInitial = enabled\nSTA64_mapConfinementRearm = true\n",
    );
    assert!(!disabled.has_map_confinement());
    assert!(warnings.is_empty(), "ODF parse warnings: {warnings:?}");
    assert!(
        disabled
            .map_confinement_warnings()
            .iter()
            .any(|w| w.file == "subject.odf" && w.message.contains("inactive"))
    );

    let (unknown, _) = odf(
        Profile::Fo4,
        "classLabel = \"unknown_factory\"\nunitName = \"Subject\"\nSTA64_mapConfinement = true\n",
        None,
    );
    assert!(!unknown.has_map_confinement());
    assert_eq!(rules(&unknown).0.spatial, Spatial::Unsupported);
    assert_eq!(rules(&unknown).1.spatial, Spatial::Unsupported);
    assert!(
        unknown
            .map_confinement_warnings()
            .iter()
            .any(|w| w.file == "subject.odf" && w.message.contains("no verified spatial policy"))
    );
}

#[test]
fn effective_and_construction_rules_control_hash() {
    let base = built(Profile::Fo4, "craft", Overrides::default());
    let no_op = built(
        Profile::Fo4,
        "craft",
        Overrides {
            capability: Some(true),
            initial: Some(Initial::RawXz),
            rearm: Some(false),
            inset: Some(Fixed::from_int(100)),
            upper_epsilon: Some(Fixed::from_bits(429496736)),
        },
    );
    assert_eq!(rules(&base), rules(&no_op));
    assert_eq!(base.hash(), no_op.hash());
    let enabled_change = built(
        Profile::Fo4,
        "craft",
        Overrides {
            inset: Some(Fixed::ZERO),
            ..Overrides::default()
        },
    );
    assert_ne!(base.hash(), enabled_change.hash());

    let native = built(Profile::Native, "craft", Overrides::default());
    let disabled_change = built(
        Profile::Native,
        "craft",
        Overrides {
            inset: Some(Fixed::ZERO),
            ..Overrides::default()
        },
    );
    assert_eq!(native.hash(), disabled_change.hash());
    let opted_in = built(
        Profile::Native,
        "craft",
        Overrides {
            capability: Some(true),
            ..Overrides::default()
        },
    );
    assert!(opted_in.has_map_confinement());
    assert_ne!(native.hash(), opted_in.hash());

    let terrain = built(Profile::Fo4, "nebula_obj", Overrides::default());
    let authored = built(
        Profile::Fo4,
        "nebula_obj",
        Overrides {
            rearm: Some(true),
            ..Overrides::default()
        },
    );
    assert_eq!(rules(&terrain).0, rules(&authored).0);
    assert!(!rules(&terrain).1.rearm);
    assert!(rules(&authored).1.rearm);
    assert_ne!(terrain.hash(), authored.hash());
    assert!(
        authored
            .map_confinement_warnings()
            .iter()
            .any(|w| w.message.contains("STA64_mapConfinementRearm"))
    );
}
