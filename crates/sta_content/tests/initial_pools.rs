use std::sync::Arc;

use sta_content::{ContentDb, Fixed, Overrides, Profile, UnitDef, Warning, odf::load_with};

const CRAFT: &str = "classLabel = \"craft\"\nunitName = \"Ship\"\n";

fn load(profile: Profile, child: &str, parent: Option<&str>) -> (ContentDb, Vec<Warning>) {
    let stems = vec!["ship".to_owned()];
    load_with(
        profile,
        &Overrides::default(),
        &stems,
        &mut |name, version| match (name.to_ascii_lowercase().as_str(), version) {
            ("ship.odf", 0) => Some(child.to_owned()),
            ("ship.odf", 1) => parent.map(str::to_owned),
            _ => None,
        },
    )
    .unwrap()
}

fn ship(db: &ContentDb) -> &UnitDef {
    db.unit(db.key("ship").unwrap()).unwrap()
}

fn f(value: i32) -> Fixed {
    Fixed::from_int(value)
}

fn pools(db: &ContentDb) -> (Fixed, Fixed, Fixed, Fixed) {
    let unit = ship(db);
    (
        unit.hull,
        unit.initial_hull.unwrap_or(unit.hull),
        unit.shields.max,
        unit.initial_shields.unwrap_or(unit.shields.max),
    )
}

#[test]
fn explicit_and_partial_currents_resolve_in_every_profile() {
    for profile in [
        Profile::Native,
        Profile::Sta1Classic,
        Profile::Sta2Classic,
        Profile::Fo4,
    ] {
        let (db, warnings) = load(
            profile,
            &format!("{CRAFT}maxHealth = 120\ncurHealth = 35\nmaxShields = 90\ncurShields = 20\n"),
            None,
        );
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(pools(&db), (f(120), f(35), f(90), f(20)), "{profile:?}");

        let (db, warnings) = load(
            profile,
            &format!("{CRAFT}maxHealth = 120\nmaxShield = 90\ncurShield = 0\n"),
            None,
        );
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(pools(&db), (f(120), f(120), f(90), f(0)), "{profile:?}");

        let (db, warnings) = load(
            profile,
            &format!("{CRAFT}maxHealth = 120\ncurShields = 17\n"),
            None,
        );
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(pools(&db), (f(120), f(120), f(17), f(17)), "{profile:?}");
    }
}

#[test]
fn singular_shield_keys_take_priority_over_plural_keys() {
    let odf = format!(
        "{CRAFT}maxHealth = 100\nmaxShield = 80\nmaxShields = 90\ncurShield = 0\ncurShields = 30\n"
    );
    let (db, warnings) = load(Profile::Fo4, &odf, None);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(pools(&db), (f(100), f(100), f(80), f(0)));

    let (db, warnings) = load(
        Profile::Fo4,
        &format!("{CRAFT}maxHealth = 100\nmaxShield = 0\nmaxShields = 90\ncurShields = 30\n"),
        None,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(pools(&db), (f(100), f(100), f(0), f(30)));
}

#[test]
fn absent_shields_follow_profile_even_when_recharge_is_authored() {
    let odf = format!("{CRAFT}maxHealth = 120\ncurHealth = 35\nshieldRate = 8\n");
    for profile in [Profile::Sta2Classic, Profile::Fo4] {
        let (db, warnings) = load(profile, &odf, None);
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(pools(&db), (f(120), f(35), f(120), f(35)), "{profile:?}");
        assert!(ship(&db).shields.recharge > Fixed::ZERO);
    }
    for profile in [Profile::Native, Profile::Sta1Classic] {
        let (db, warnings) = load(profile, &odf, None);
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(pools(&db), (f(120), f(35), f(0), f(0)), "{profile:?}");
    }
}

#[test]
fn inherited_currents_and_explicit_zero_override_resolve_before_loading() {
    let parent =
        format!("{CRAFT}maxHealth = 120\ncurHealth = 35\nmaxShield = 90\ncurShield = 25\n");
    let (inherited, warnings) = load(Profile::Fo4, "#include \"ship.odf\"\n", Some(&parent));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(pools(&inherited), (f(120), f(35), f(90), f(25)));

    let (overridden, warnings) = load(
        Profile::Fo4,
        "#include \"ship.odf\"\ncurHealth = 0\ncurShield = 0\n",
        Some(&parent),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(pools(&overridden), (f(120), f(0), f(90), f(0)));
    assert_ne!(inherited.hash(), overridden.hash());
}

#[test]
fn malformed_selected_numbers_keep_reader_warnings() {
    let (db, warnings) = load(
        Profile::Fo4,
        &format!(
            "{CRAFT}maxHealth = 100\ncurHealth = broken\nmaxShield = invalid\nmaxShields = 90\ncurShield = nope\ncurShields = 20\n"
        ),
        None,
    );
    assert_eq!(pools(&db), (f(100), f(0), f(0), f(0)));
    for key in ["curHealth", "maxShield", "curShield"] {
        assert!(
            warnings.iter().any(|w| w.file == "ship.odf"
                && w.message.contains(key)
                && w.message.contains("is not a number")),
            "missing {key} warning: {warnings:?}"
        );
    }
}

#[test]
fn digest_uses_effective_initial_values_and_aliases() {
    let base = format!("{CRAFT}maxHealth = 120\nmaxShield = 90\n");
    let (omitted, warnings) = load(Profile::Fo4, &base, None);
    assert!(warnings.is_empty(), "{warnings:?}");
    let (full, warnings) = load(
        Profile::Fo4,
        &format!("{base}curHealth = 120\ncurShield = 90\n"),
        None,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(omitted.hash(), full.hash());

    let (partial, warnings) = load(
        Profile::Fo4,
        &format!("{base}curHealth = 35\ncurShield = 20\n"),
        None,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_ne!(omitted.hash(), partial.hash());
    let (other_hull, _) = load(
        Profile::Fo4,
        &format!("{base}curHealth = 34\ncurShield = 20\n"),
        None,
    );
    let (other_shields, _) = load(
        Profile::Fo4,
        &format!("{base}curHealth = 35\ncurShield = 19\n"),
        None,
    );
    assert_ne!(partial.hash(), other_hull.hash());
    assert_ne!(partial.hash(), other_shields.hash());

    let (plural, warnings) = load(
        Profile::Fo4,
        &format!("{CRAFT}maxHealth = 120\nmaxShields = 90\ncurHealth = 35\ncurShields = 20\n"),
        None,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(pools(&partial), pools(&plural));
    assert_eq!(partial.hash(), plural.hash());
}

#[test]
fn cloned_definition_raw_metadata_is_not_a_digest_input() {
    let (loaded, warnings) = load(
        Profile::Native,
        &format!("{CRAFT}maxHealth = 120\ncurHealth = 35\nmaxShield = 90\ncurShield = 20\n"),
        None,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let original = ship(&loaded).clone();
    let mut changed_raw = original.clone();
    changed_raw.raw = Arc::new(
        sta_odf::resolve("ship", &mut |_| {
            Some("classLabel = \"craft\"\nunitName = \"Different raw only\"\n".to_owned())
        })
        .unwrap(),
    );
    let a = ContentDb::builder().unit(original).build().unwrap();
    let b = ContentDb::builder().unit(changed_raw).build().unwrap();
    assert_eq!(a.hash(), b.hash());
}
