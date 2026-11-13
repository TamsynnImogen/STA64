use sta_content::{ContentDb, Overrides, Profile, Warning, odf::load_with};

const PRODUCT: &str = "classLabel = \"craft\"\nunitName = \"Product\"\n";
const WEAPON: &str = "classLabel = \"cannon\"\nwpnName = \"Weapon\"\n";

fn load(
    profile: Profile,
    yard: &str,
    parent: Option<&str>,
    product: &str,
    weapon: &str,
) -> (ContentDb, Vec<Warning>) {
    let stems = vec!["yard".into(), "product".into(), "weapon".into()];
    load_with(
        profile,
        &Overrides::default(),
        &stems,
        &mut |name, version| match (name.to_ascii_lowercase().as_str(), version) {
            ("yard.odf", 0) => Some(yard.to_owned()),
            ("yard.odf", 1) => parent.map(str::to_owned),
            ("product.odf", 0) => Some(product.to_owned()),
            ("weapon.odf", 0) => Some(weapon.to_owned()),
            _ => None,
        },
    )
    .unwrap()
}

fn limit(db: &ContentDb) -> u32 {
    db.build_queue_limit(db.unit(db.key("yard").unwrap()).unwrap())
}

#[test]
fn profiles_use_five_or_ten_and_no_op_keys_keep_the_hash() {
    let yard = "classLabel = \"shipyard\"\nunitName = \"Yard\"\nbuildItem0 = \"product\"\n";
    for (profile, expected) in [
        (Profile::Sta1Classic, 5),
        (Profile::Native, 5),
        (Profile::Sta2Classic, 10),
        (Profile::Fo4, 10),
    ] {
        let (plain, warnings) = load(profile, yard, None, PRODUCT, WEAPON);
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(profile.build_queue_limit(), expected);
        assert_eq!(limit(&plain), expected);
        let (no_op, warnings) = load(
            profile,
            &format!("{yard}STA64_queueLimit = {expected}\n"),
            None,
            PRODUCT,
            WEAPON,
        );
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(limit(&no_op), expected);
        assert_eq!(
            plain.hash(),
            no_op.hash(),
            "effective no-op for {profile:?}"
        );
        let (authored, warnings) = load(
            profile,
            &format!("{yard}STA64_queueLimit = 1\n"),
            None,
            PRODUCT,
            WEAPON,
        );
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(limit(&authored), 1);
        assert_ne!(authored.hash(), plain.hash());
    }
    assert_eq!(sta_content::fake::db().hash(), 0x6977_6a57_dec2_2e6c);
}

#[test]
fn inherited_zero_overrides_parent_and_large_authored_limit_is_not_clamped() {
    let parent = "classLabel = \"shipyard\"\nunitName = \"Yard\"\nbuildItem0 = \"product\"\nSTA64_queueLimit = 12\n";
    let (inherited, warnings) = load(
        Profile::Fo4,
        "#include \"yard.odf\"\n",
        Some(parent),
        PRODUCT,
        WEAPON,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(limit(&inherited), 12);
    let (disabled, warnings) = load(
        Profile::Fo4,
        "#include \"yard.odf\"\nSTA64_queueLimit = 0\n",
        Some(parent),
        PRODUCT,
        WEAPON,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(limit(&disabled), 0);
    assert_eq!(
        disabled
            .unit(disabled.key("yard").unwrap())
            .unwrap()
            .raw
            .raw("STA64_queueLimit"),
        Some("0")
    );
    assert_ne!(disabled.hash(), inherited.hash());
    let (large, warnings) = load(
        Profile::Fo4,
        "#include \"yard.odf\"\nSTA64_queueLimit = 4294967295\n",
        Some(parent),
        PRODUCT,
        WEAPON,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(limit(&large), u32::MAX);
}

#[test]
fn malformed_values_warn_and_keep_profile_fallback() {
    let yard = "classLabel = \"shipyard\"\nunitName = \"Yard\"\n";
    for invalid in ["-1", "1.5", "4294967296", "oops"] {
        let (db, warnings) = load(
            Profile::Sta2Classic,
            &format!("{yard}STA64_queueLimit = {invalid}\n"),
            None,
            PRODUCT,
            WEAPON,
        );
        assert_eq!(limit(&db), 10, "{invalid}");
        assert!(
            warnings
                .iter()
                .any(|w| w.file == "yard.odf" && w.message.contains("STA64_queueLimit")),
            "{invalid}: {warnings:?}"
        );
    }
}

#[test]
fn weapon_key_is_ignored_and_effective_override_changes_hash() {
    let yard = "classLabel = \"shipyard\"\nunitName = \"Yard\"\n";
    let (plain, _) = load(Profile::Native, yard, None, PRODUCT, WEAPON);
    let (weapon_only, warnings) = load(
        Profile::Native,
        yard,
        None,
        PRODUCT,
        &format!("{WEAPON}STA64_queueLimit = 0\n"),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(weapon_only.hash(), plain.hash());
    assert_eq!(limit(&weapon_only), 5);
    let (different, warnings) = load(
        Profile::Native,
        &format!("{yard}STA64_queueLimit = 1\n"),
        None,
        PRODUCT,
        WEAPON,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(limit(&different), 1);
    assert_ne!(different.hash(), plain.hash());
}
