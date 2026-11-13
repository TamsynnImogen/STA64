use sta_content::{ContentDb, Overrides, PlanetRole, Profile, UnitDef, Warning, odf::load_with};

const PLANET: &str = "classLabel = \"planet\"\n";
const BACKGROUND: &str = "classLabel = \"planet_obj\"\n";

fn load(profile: Profile, child: &str, parent: Option<&str>) -> (ContentDb, Vec<Warning>) {
    load_with(
        profile,
        &Overrides::default(),
        &["target".into()],
        &mut |name, version| match (name, version) {
            ("target.odf", 0) => Some(child.to_owned()),
            ("target.odf", 1) => parent.map(str::to_owned),
            _ => None,
        },
    )
    .unwrap()
}

fn gain(db: &ContentDb) -> i32 {
    db.unit(db.key("target").unwrap()).unwrap().planet_crew_gain
}

fn role(db: &ContentDb) -> Option<PlanetRole> {
    db.unit(db.key("target").unwrap()).unwrap().planet_role
}

#[test]
fn producer_profile_fallbacks_and_native_opt_in() {
    for (profile, expected_role, expected_gain) in [
        (Profile::Sta1Classic, None, 0),
        (Profile::Sta2Classic, Some(PlanetRole::Producer), 10_000),
        (Profile::Fo4, Some(PlanetRole::Producer), 10_000),
        (Profile::Native, None, 0),
    ] {
        let (db, warnings) = load(profile, PLANET, None);
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(role(&db), expected_role);
        assert_eq!(gain(&db), expected_gain);
    }
    for profile in [Profile::Sta1Classic, Profile::Native] {
        let (db, warnings) = load(profile, &format!("{PLANET}STA64_planetRole = \"producer\"\n"), None);
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(role(&db), Some(PlanetRole::Producer));
        assert_eq!(gain(&db), 0);
    }
}

#[test]
fn authored_signed_decimal_i32_values_include_zero_and_boundaries() {
    for (raw, expected) in [
        ("0", 0),
        ("+7", 7),
        ("-5", -5),
        ("2147483647", i32::MAX),
        ("-2147483648", i32::MIN),
        ("  -42  ", -42),
    ] {
        let (db, warnings) = load(Profile::Fo4, &format!("{PLANET}maxCrewGain = \"{raw}\"\n"), None);
        assert!(warnings.is_empty(), "{raw}: {warnings:?}");
        assert_eq!(gain(&db), expected, "{raw}");
    }
}

#[test]
fn layered_self_include_keeps_parent_zero_and_child_override() {
    let parent = format!("{PLANET}maxCrewGain = 0\n");
    let (inherited, warnings) = load(Profile::Fo4, "#include \"target.odf\"\n", Some(&parent));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(gain(&inherited), 0);
    let (child, warnings) = load(
        Profile::Fo4,
        "#include \"target.odf\"\nmaxCrewGain = -12\n",
        Some(&parent),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(gain(&child), -12);
    assert_eq!(
        child.unit(child.key("target").unwrap()).unwrap().raw.raw("maxCrewGain"),
        Some("-12")
    );
    assert_ne!(child.hash(), inherited.hash());

    let parent = format!("{PLANET}maxCrewGain = 35\n");
    let (zero, warnings) = load(
        Profile::Fo4,
        "#include \"target.odf\"\nmaxCrewGain = 0\n",
        Some(&parent),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(gain(&zero), 0, "child zero is not absence");
}

#[test]
fn malformed_and_out_of_range_producer_values_warn_and_use_profile_fallback() {
    for (profile, prefix, fallback) in [
        (Profile::Sta2Classic, PLANET.to_owned(), 10_000),
        (Profile::Native, format!("{PLANET}STA64_planetRole = \"producer\"\n"), 0),
    ] {
        for raw in ["", "1.5", "0x10", "2tail", "2147483648", "-2147483649"] {
            let (db, warnings) = load(profile, &format!("{prefix}maxCrewGain = \"{raw}\"\n"), None);
            assert_eq!(gain(&db), fallback, "{profile:?} {raw}");
            assert!(
                warnings
                    .iter()
                    .any(|w| w.file == "target.odf" && w.message.contains("maxCrewGain")),
                "{profile:?} {raw}: {warnings:?}"
            );
        }
    }
}

#[test]
fn unused_raw_key_on_nonproducer_roles_neither_warns_nor_contributes() {
    for (profile, body, expected_role) in [
        (
            Profile::Fo4,
            format!("{BACKGROUND}maxCrewGain = garbage\n"),
            Some(PlanetRole::Background),
        ),
        (Profile::Native, format!("{PLANET}maxCrewGain = garbage\n"), None),
        (
            Profile::Fo4,
            format!("{PLANET}STA64_planetRole = \"none\"\nmaxCrewGain = garbage\n"),
            None,
        ),
    ] {
        let (db, warnings) = load(profile, &body, None);
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(role(&db), expected_role);
        assert_eq!(gain(&db), 0);
        assert_eq!(
            db.unit(db.key("target").unwrap()).unwrap().raw.raw("maxCrewGain"),
            Some("garbage")
        );
    }
    assert_eq!(UnitDef::new("synthetic", "planet").planet_crew_gain, 0);
}

#[test]
fn hash_tracks_effective_producer_gain_only_and_keeps_native_fake_identity() {
    let (absent, _) = load(Profile::Fo4, PLANET, None);
    let (same, _) = load(Profile::Fo4, &format!("{PLANET}maxCrewGain = 10000\n"), None);
    let (different, _) = load(Profile::Fo4, &format!("{PLANET}maxCrewGain = 0\n"), None);
    assert_eq!(gain(&absent), 10_000);
    assert_eq!(absent.hash(), same.hash());
    assert_ne!(absent.hash(), different.hash());

    let (native, _) = load(
        Profile::Native,
        &format!("{PLANET}STA64_planetRole = \"producer\"\n"),
        None,
    );
    let (native_zero, _) = load(
        Profile::Native,
        &format!("{PLANET}STA64_planetRole = \"producer\"\nmaxCrewGain = 0\n"),
        None,
    );
    assert_eq!(native.hash(), native_zero.hash());

    for role in [None, Some(PlanetRole::Background)] {
        let mut def = UnitDef::new("synthetic", "planet_obj");
        def.planet_role = role;
        let plain = ContentDb::builder().unit(def.clone()).build().unwrap();
        def.planet_crew_gain = 27;
        let ignored = ContentDb::builder().unit(def).build().unwrap();
        assert_eq!(plain.hash(), ignored.hash(), "{role:?} does not hash unused gain");
    }
    let mut def = UnitDef::new("synthetic", "planet");
    def.planet_role = Some(PlanetRole::Producer);
    let plain = ContentDb::builder().unit(def.clone()).build().unwrap();
    def.planet_crew_gain = -27;
    let changed = ContentDb::builder().unit(def).build().unwrap();
    assert_ne!(plain.hash(), changed.hash());
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
}
