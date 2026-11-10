use sta_content::{ContentDb, Overrides, Profile, Warning, odf::load_with};

const SHIP: &str = "classLabel = \"craft\"\nunitName = \"Ship\"\n";
const GUN: &str = "classLabel = \"cannon\"\nwpnName = \"Gun\"\n";

fn load(
    profile: Profile,
    mod_features: &str,
    yard: &str,
    parent: Option<&str>,
    ship: &str,
    gun: &str,
) -> (ContentDb, Vec<Warning>) {
    let overrides = Overrides::from_info_ini(mod_features);
    let stems = vec!["yard".into(), "ship".into(), "gun".into()];
    load_with(profile, &overrides, &stems, &mut |name, version| match (
        name.to_ascii_lowercase().as_str(),
        version,
    ) {
        ("yard.odf", 0) => Some(yard.to_owned()),
        ("yard.odf", 1) => parent.map(str::to_owned),
        ("ship.odf", 0) => Some(ship.to_owned()),
        ("gun.odf", 0) => Some(gun.to_owned()),
        _ => None,
    })
    .unwrap()
}

fn yard(db: &ContentDb) -> bool {
    let def = db.unit(db.key("yard").unwrap()).unwrap();
    db.unit_features(def).pay_on_queue
}

fn ship(db: &ContentDb) -> bool {
    let def = db.unit(db.key("ship").unwrap()).unwrap();
    db.unit_features(def).pay_on_queue
}

#[test]
fn absent_and_no_op_native_keys_keep_profile_and_content_hash() {
    let plain = "classLabel = \"shipyard\"\nunitName = \"Yard\"\nbuildItem0 = \"ship\"\n";
    for (profile, no_op) in [(Profile::Fo4, "0"), (Profile::Native, "1")] {
        let (base, warnings) = load(profile, "", plain, None, SHIP, GUN);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(yard(&base), profile.features().pay_on_queue);
        let (same, warnings) = load(
            profile,
            "",
            &format!("{plain}queueInstantDeduct = {no_op}\n"),
            None,
            SHIP,
            GUN,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(yard(&same), yard(&base));
        assert_eq!(same.hash(), base.hash());
        let changed = if no_op == "0" { "1" } else { "0" };
        let (different, _) = load(
            profile,
            "",
            &format!("{plain}queueInstantDeduct = {changed}\n"),
            None,
            SHIP,
            GUN,
        );
        assert_ne!(yard(&different), yard(&base));
        assert_ne!(different.hash(), base.hash());
    }
}

#[test]
fn layered_child_zero_overrides_parent_one_and_explicit_feature_wins() {
    let parent = "classLabel = \"shipyard\"\nunitName = \"Yard\"\nbuildItem0 = \"ship\"\nqueueInstantDeduct = 1\n";
    let (inherited, warnings) = load(
        Profile::Fo4,
        "",
        "#include \"yard.odf\"\n",
        Some(parent),
        SHIP,
        GUN,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(yard(&inherited));
    let (zero, warnings) = load(
        Profile::Fo4,
        "",
        "#include \"yard.odf\"\nqueueInstantDeduct = 0\n",
        Some(parent),
        SHIP,
        GUN,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!yard(&zero));
    assert_eq!(
        zero.unit(zero.key("yard").unwrap())
            .unwrap()
            .raw
            .raw("queueInstantDeduct"),
        Some("0")
    );
    let (feature, warnings) = load(
        Profile::Fo4,
        "",
        "#include \"yard.odf\"\nqueueInstantDeduct = 0\nFeature_pay_on_queue = 1\n",
        Some(parent),
        SHIP,
        GUN,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(yard(&feature));
    let (parent_feature, warnings) = load(
        Profile::Fo4,
        "",
        "#include \"yard.odf\"\nqueueInstantDeduct = 1\n",
        Some(&format!("{parent}Feature_pay_on_queue = 0\n")),
        SHIP,
        GUN,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(
        !yard(&parent_feature),
        "inherited Feature key has higher precedence"
    );
}

#[test]
fn mod_default_sits_below_native_key_and_invalid_value_keeps_fallback() {
    let base = "classLabel = \"shipyard\"\nunitName = \"Yard\"\n";
    let (db, _) = load(
        Profile::Fo4,
        "[Features]\npay_on_queue = 1\n",
        &format!("{base}queueInstantDeduct = 0\n"),
        None,
        SHIP,
        GUN,
    );
    assert!(db.features().pay_on_queue);
    assert!(!yard(&db));
    let (db, warnings) = load(
        Profile::Fo4,
        "[Features]\npay_on_queue = 1\n",
        &format!("{base}queueInstantDeduct = 1e3\n"),
        None,
        SHIP,
        GUN,
    );
    assert!(yard(&db));
    assert!(
        warnings
            .iter()
            .any(|w| w.file == "yard.odf" && w.message.contains("queueInstantDeduct")),
        "{warnings:?}"
    );
    let (noncanonical, warnings) = load(
        Profile::Fo4,
        "",
        &format!("{base}queueInstantDeduct = 2\n"),
        None,
        SHIP,
        GUN,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(yard(&noncanonical));
    for (value, expected) in [("-2", true), ("true", true), ("false", false)] {
        let (parsed, warnings) = load(
            Profile::Fo4,
            "",
            &format!("{base}queueInstantDeduct = {value}\n"),
            None,
            SHIP,
            GUN,
        );
        assert!(warnings.is_empty(), "{value}: {warnings:?}");
        assert_eq!(yard(&parsed), expected, "{value}");
    }
    let (overflow, warnings) = load(
        Profile::Fo4,
        "[Features]\npay_on_queue = 1\n",
        &format!("{base}queueInstantDeduct = 2147483648\n"),
        None,
        SHIP,
        GUN,
    );
    assert!(yard(&overflow));
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("queueInstantDeduct")),
        "{warnings:?}"
    );
}

#[test]
fn stock_profiles_and_weapons_ignore_the_fo_native_key() {
    let base = "classLabel = \"shipyard\"\nunitName = \"Yard\"\n";
    for profile in [Profile::Sta1Classic, Profile::Sta2Classic] {
        let (plain, _) = load(profile, "", base, None, SHIP, GUN);
        let (keyed, warnings) = load(
            profile,
            "",
            &format!("{base}queueInstantDeduct = 1\n"),
            None,
            SHIP,
            &format!("{GUN}queueInstantDeduct = 1\n"),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(yard(&keyed), yard(&plain));
        assert_eq!(keyed.hash(), plain.hash());
        assert_eq!(
            keyed
                .unit(keyed.key("yard").unwrap())
                .unwrap()
                .raw
                .raw("queueInstantDeduct"),
            Some("1")
        );
    }
    let (plain, _) = load(Profile::Fo4, "", base, None, SHIP, GUN);
    let (weapon_key, warnings) = load(
        Profile::Fo4,
        "",
        base,
        None,
        SHIP,
        &format!("{GUN}queueInstantDeduct = 1\n"),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(weapon_key.hash(), plain.hash());
    assert_eq!(
        weapon_key
            .weapon_features(weapon_key.weapon(weapon_key.key("gun").unwrap()).unwrap())
            .pay_on_queue,
        plain.features().pay_on_queue
    );
    assert!(!ship(&plain));
}
