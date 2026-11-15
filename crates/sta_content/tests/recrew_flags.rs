use sta_content::{ContentDb, Overrides, Profile, UnitDef, Warning, odf::load_with};

const STATION: &str = "classLabel = \"starbase\"\nunitName = \"Station\"\n";

fn load(child: &str, parent: Option<&str>) -> (ContentDb, Vec<Warning>) {
    load_with(
        Profile::Native,
        &Overrides::default(),
        &["station".to_owned()],
        &mut |name, version| match (name.to_ascii_lowercase().as_str(), version) {
            ("station.odf", 0) => Some(child.to_owned()),
            ("station.odf", 1) => parent.map(str::to_owned),
            _ => None,
        },
    )
    .unwrap()
}

fn station(db: &ContentDb) -> &UnitDef {
    db.unit(db.key("station").unwrap()).unwrap()
}

#[test]
fn absent_and_inherited_flags_preserve_explicit_zero() {
    let (plain, warnings) = load(STATION, None);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        (station(&plain).facility, station(&plain).has_crew),
        (None, None)
    );
    let authored = UnitDef::new("synthetic", "starbase");
    assert_eq!((authored.facility, authored.has_crew), (None, None));

    let parent = format!("{STATION}facility = 1\nhas_crew = 0\n");
    let (inherited, warnings) = load("#include \"station.odf\"\n", Some(&parent));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        (station(&inherited).facility, station(&inherited).has_crew),
        (Some(true), Some(false))
    );

    let (overridden, warnings) = load(
        "#include \"station.odf\"\nfacility = 0\nhas_crew = 1\n",
        Some(&parent),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        (station(&overridden).facility, station(&overridden).has_crew),
        (Some(false), Some(true))
    );
}

#[test]
fn each_authored_flag_and_value_changes_the_keyed_content_digest() {
    let (plain, warnings) = load(STATION, None);
    assert!(warnings.is_empty(), "{warnings:?}");
    let plain_text = plain.digest().get("unit:station").unwrap();
    assert!(!plain_text.contains("facility="));
    assert!(!plain_text.contains("has_crew="));

    for key in ["facility", "has_crew"] {
        let (enabled, warnings) = load(&format!("{STATION}{key} = 1\n"), None);
        assert!(warnings.is_empty(), "{key}: {warnings:?}");
        let (disabled, warnings) = load(&format!("{STATION}{key} = 0\n"), None);
        assert!(warnings.is_empty(), "{key}: {warnings:?}");
        assert_ne!(plain.hash(), enabled.hash(), "{key} = 1");
        assert_ne!(plain.hash(), disabled.hash(), "{key} = 0");
        assert_ne!(enabled.hash(), disabled.hash(), "{key}: 1 vs 0");
        assert_eq!(
            plain
                .digest()
                .first_difference(enabled.digest())
                .unwrap()
                .key,
            "unit:station"
        );
        assert_eq!(
            plain
                .digest()
                .first_difference(disabled.digest())
                .unwrap()
                .key,
            "unit:station"
        );
    }
}
