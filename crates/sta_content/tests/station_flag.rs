use sta_content::{ContentDb, Overrides, Profile, UnitDef, Warning, odf::load_with};

const CRAFT: &str = "classLabel = \"craft\"\nunitName = \"Craft\"\n";
const STATION: &str = "classLabel = \"starbase\"\nunitName = \"Station\"\n";
const WEAPON: &str = "classLabel = \"cannon\"\nwpnName = \"Weapon\"\n";

fn load(
    profile: Profile,
    craft: &str,
    parent: Option<&str>,
    station: &str,
    weapon: &str,
) -> (ContentDb, Vec<Warning>) {
    let stems = vec!["craft".into(), "station".into(), "weapon".into()];
    load_with(
        profile,
        &Overrides::default(),
        &stems,
        &mut |name, version| match (name.to_ascii_lowercase().as_str(), version) {
            ("craft.odf", 0) => Some(craft.to_owned()),
            ("craft.odf", 1) => parent.map(str::to_owned),
            ("station.odf", 0) => Some(station.to_owned()),
            ("weapon.odf", 0) => Some(weapon.to_owned()),
            _ => None,
        },
    )
    .unwrap()
}

fn craft_flag(db: &ContentDb) -> bool {
    db.unit(db.key("craft").unwrap()).unwrap().is_starbase
}

#[test]
fn absent_zero_and_integer_truth_are_consistent_across_profiles() {
    for profile in Profile::ALL {
        let (absent, warnings) = load(profile, CRAFT, None, STATION, WEAPON);
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert!(!craft_flag(&absent));
        assert!(
            !absent
                .unit(absent.key("station").unwrap())
                .unwrap()
                .is_starbase
        );
        let (zero, warnings) = load(
            profile,
            &format!("{CRAFT}is_starbase = 0\n"),
            None,
            STATION,
            WEAPON,
        );
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert!(!craft_flag(&zero));
        assert_eq!(zero.hash(), absent.hash(), "explicit false is a hash no-op");
        for value in ["1", "2", "-3"] {
            let (set, warnings) = load(
                profile,
                &format!("{CRAFT}is_starbase = {value}\n"),
                None,
                STATION,
                WEAPON,
            );
            assert!(warnings.is_empty(), "{profile:?} {value}: {warnings:?}");
            assert!(craft_flag(&set), "{profile:?} {value}");
            assert_ne!(set.hash(), absent.hash());
        }
    }
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
}

#[test]
fn integer_prefixes_follow_native_truth_across_profiles() {
    for profile in Profile::ALL {
        for (value, expected) in [
            ("1.5", true),
            ("2f", true),
            ("+2e3", true),
            ("0x1", false),
            ("0.5", false),
            ("-3tail", true),
        ] {
            let (db, warnings) = load(
                profile,
                &format!("{CRAFT}is_starbase = {value}\n"),
                None,
                STATION,
                WEAPON,
            );
            assert!(warnings.is_empty(), "{profile:?} {value}: {warnings:?}");
            assert_eq!(craft_flag(&db), expected, "{profile:?} {value}");
        }
    }
}

#[test]
fn boolean_words_are_valid_outside_a1_and_warn_in_a1() {
    for profile in [Profile::Sta2Classic, Profile::Fo4, Profile::Native] {
        for (value, expected) in [("TrUe", true), ("FaLsE", false)] {
            let (db, warnings) = load(
                profile,
                &format!("{CRAFT}is_starbase = {value}\n"),
                None,
                STATION,
                WEAPON,
            );
            assert!(warnings.is_empty(), "{profile:?} {value}: {warnings:?}");
            assert_eq!(craft_flag(&db), expected);
        }
    }
    let (a1, warnings) = load(
        Profile::Sta1Classic,
        &format!("{CRAFT}is_starbase = true\n"),
        None,
        STATION,
        WEAPON,
    );
    assert!(!craft_flag(&a1));
    assert!(
        warnings
            .iter()
            .any(|w| w.file == "craft.odf" && w.message.contains("is_starbase")),
        "{warnings:?}"
    );
}

#[test]
fn child_zero_overrides_inherited_one_without_changing_raw_key_access() {
    let parent = format!("{CRAFT}is_starbase = 1\n");
    let (inherited, warnings) = load(
        Profile::Fo4,
        "#include \"craft.odf\"\n",
        Some(&parent),
        STATION,
        WEAPON,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(craft_flag(&inherited));
    let (child, warnings) = load(
        Profile::Fo4,
        "#include \"craft.odf\"\nis_starbase = 0\n",
        Some(&parent),
        STATION,
        WEAPON,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!craft_flag(&child));
    assert_eq!(
        child
            .unit(child.key("craft").unwrap())
            .unwrap()
            .raw
            .raw("is_starbase"),
        Some("0")
    );
    assert_ne!(inherited.hash(), child.hash());
}

#[test]
fn malformed_and_overflow_values_warn_and_preserve_raw_text() {
    for value in ["oops", ".5", "+", "2147483648", "-2147483649"] {
        let (db, warnings) = load(
            Profile::Native,
            &format!("{CRAFT}is_starbase = {value}\n"),
            None,
            STATION,
            WEAPON,
        );
        assert!(!craft_flag(&db), "{value}");
        assert!(
            warnings
                .iter()
                .any(|w| w.file == "craft.odf" && w.message.contains("is_starbase")),
            "{value}: {warnings:?}"
        );
        assert_eq!(
            db.unit(db.key("craft").unwrap())
                .unwrap()
                .raw
                .raw("is_starbase"),
            Some(value)
        );
    }
}

#[test]
fn actor_class_does_not_infer_flag_and_weapon_key_is_ignored() {
    let craft = format!("{CRAFT}is_starbase = 1\n");
    let station = format!("{STATION}is_starbase = 0\n");
    let (plain, _) = load(Profile::Fo4, &craft, None, &station, WEAPON);
    let (weapon_key, warnings) = load(
        Profile::Fo4,
        &craft,
        None,
        &station,
        &format!("{WEAPON}is_starbase = 1\n"),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(craft_flag(&plain));
    assert_eq!(
        plain.unit(plain.key("craft").unwrap()).unwrap().class.0,
        "craft"
    );
    assert!(
        !plain
            .unit(plain.key("station").unwrap())
            .unwrap()
            .is_starbase
    );
    assert_eq!(
        plain.unit(plain.key("station").unwrap()).unwrap().class.0,
        "starbase"
    );
    assert_eq!(weapon_key.hash(), plain.hash());
    assert_eq!(
        weapon_key
            .weapon(weapon_key.key("weapon").unwrap())
            .unwrap()
            .raw
            .raw("is_starbase"),
        Some("1")
    );
}

#[test]
fn programmatic_true_changes_content_identity_but_default_false_does_not() {
    let plain = ContentDb::builder()
        .profile(Profile::Native)
        .features(Profile::Native.features())
        .unit(UnitDef::new("flag", "craft"))
        .build()
        .unwrap();
    let mut explicit_false = UnitDef::new("flag", "craft");
    explicit_false.is_starbase = false;
    let no_op = ContentDb::builder()
        .profile(Profile::Native)
        .features(Profile::Native.features())
        .unit(explicit_false)
        .build()
        .unwrap();
    assert_eq!(plain.hash(), no_op.hash());
    let mut true_flag = UnitDef::new("flag", "craft");
    true_flag.is_starbase = true;
    let changed = ContentDb::builder()
        .profile(Profile::Native)
        .features(Profile::Native.features())
        .unit(true_flag)
        .build()
        .unwrap();
    assert_ne!(plain.hash(), changed.hash());
}
