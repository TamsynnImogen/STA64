use sta_content::{ContentDb, Fixed, Overrides, Profile, UnitDef, Warning, odf::load_with};

const CRAFT: &str = "classLabel = \"craft\"\nunitName = \"Ship\"\n";

fn f(value: i32) -> Fixed {
    Fixed::from_int(value)
}

fn load(profile: Profile, child: &str, parent: Option<&str>) -> (ContentDb, Vec<Warning>) {
    load_with(
        profile,
        &Overrides::default(),
        &["ship".into()],
        &mut |name, version| match (name, version) {
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

#[test]
fn missing_inherited_and_zero_capacity_keep_starting_cost_independent() {
    for profile in [
        Profile::Native,
        Profile::Sta1Classic,
        Profile::Sta2Classic,
        Profile::Fo4,
    ] {
        let (missing, warnings) = load(profile, &format!("{CRAFT}crewCost = 150\n"), None);
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(ship(&missing).crew, f(150));
        assert_eq!(ship(&missing).maximum_crew, None);

        let parent = format!("{CRAFT}crewCost = 150\nmaximumCrew = 1500\n");
        let (inherited, warnings) = load(profile, "#include \"ship.odf\"\n", Some(&parent));
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(ship(&inherited).crew, f(150));
        assert_eq!(ship(&inherited).maximum_crew, Some(f(1500)));

        let (zero, warnings) = load(
            profile,
            "#include \"ship.odf\"\nmaximumCrew = 0\n",
            Some(&parent),
        );
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(ship(&zero).crew, f(150));
        assert_eq!(ship(&zero).maximum_crew, Some(Fixed::ZERO));
    }
}

#[test]
fn malformed_capacity_uses_existing_numeric_diagnostic() {
    let (db, warnings) = load(
        Profile::Sta2Classic,
        &format!("{CRAFT}crewCost = 150\nmaximumCrew = broken\n"),
        None,
    );
    assert_eq!(ship(&db).crew, f(150));
    assert_eq!(ship(&db).maximum_crew, Some(Fixed::ZERO));
    assert!(
        warnings.iter().any(|w| w.file == "ship.odf"
            && w.message.contains("maximumCrew")
            && w.message.contains("is not a number")),
        "{warnings:?}"
    );
}

#[test]
fn hash_tracks_effective_capacity_only() {
    let base = format!("{CRAFT}crewCost = 150\n");
    let (missing, _) = load(Profile::Native, &base, None);
    let (equivalent, _) = load(Profile::Native, &format!("{base}maximumCrew = 150\n"), None);
    let (larger, _) = load(
        Profile::Native,
        &format!("{base}maximumCrew = 1500\n"),
        None,
    );
    let (zero, _) = load(Profile::Native, &format!("{base}maximumCrew = 0\n"), None);
    assert_eq!(missing.hash(), equivalent.hash());
    assert_ne!(missing.hash(), larger.hash());
    assert_ne!(missing.hash(), zero.hash());
    assert_ne!(larger.hash(), zero.hash());
}
