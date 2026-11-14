use sta_content::{ContentDb, Fixed, Overrides, Profile, RepairDef, Warning, odf::load_with};

const CRAFT: &str = "classLabel = \"craft\"\nunitName = \"Ship\"\n";

fn load(child: &str, parent: Option<&str>) -> (ContentDb, Vec<Warning>) {
    load_with(
        Profile::Native,
        &Overrides::default(),
        &["ship".to_owned()],
        &mut |name, version| match (name.to_ascii_lowercase().as_str(), version) {
            ("ship.odf", 0) => Some(child.to_owned()),
            ("ship.odf", 1) => parent.map(str::to_owned),
            _ => None,
        },
    )
    .unwrap()
}

fn repair(db: &ContentDb) -> &RepairDef {
    &db.unit(db.key("ship").unwrap()).unwrap().repair
}

#[test]
fn inherited_values_and_authored_absence_are_preserved() {
    let parent = format!(
        "{CRAFT}enginesRepairTime = 1.25\nweaponsRepairTime = 2\nshieldGeneratorRepairTime = 3\nlifeSupportRepairTime = 4\nsensorsRepairTime = 5\nrepairFacility = 1\nrepairHardpoint = \"HP01\"\nrepairSide = -2\ncanShipyardRepair = 1\n"
    );
    let (db, warnings) = load(
        "#include \"ship.odf\"\nenginesRepairTime = 0\nrepairFacility = 0\ncanShipyardRepair = 0\n",
        Some(&parent),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let r = repair(&db);
    assert_eq!(r.system_times, [0, 2, 3, 4, 5].map(|n| Some(Fixed::from_int(n))));
    assert_eq!(r.repair_facility, Some(false));
    assert_eq!(r.repair_hardpoint.as_deref(), Some("HP01"));
    assert_eq!(r.repair_side, Some(-2));
    assert_eq!(r.can_shipyard_repair, Some(false));

    let (plain, warnings) = load(CRAFT, None);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(*repair(&plain), RepairDef::default());
    assert_ne!(plain.hash(), db.hash());
}

#[test]
fn invalid_and_negative_times_warn_and_remain_absent() {
    let (db, warnings) = load(
        &format!("{CRAFT}enginesRepairTime = -2\nweaponsRepairTime = banana\nrepairSide = 1.5\n"),
        None,
    );
    assert_eq!(repair(&db).system_times, [None; 5]);
    assert_eq!(repair(&db).repair_side, None);
    assert_eq!(db.hash(), load(CRAFT, None).0.hash());
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("enginesRepairTime") && w.message.contains("negative")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("weaponsRepairTime") && w.message.contains("not a number")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("repairSide") && w.message.contains("whole number")),
        "{warnings:?}"
    );
}

#[test]
fn every_authored_field_changes_the_digest() {
    let (plain, warnings) = load(CRAFT, None);
    assert!(warnings.is_empty(), "{warnings:?}");
    for key in [
        "enginesRepairTime = 0",
        "weaponsRepairTime = 0",
        "shieldGeneratorRepairTime = 0",
        "lifeSupportRepairTime = 0",
        "sensorsRepairTime = 0",
        "repairFacility = 0",
        "repairHardpoint = \"hp01\"",
        "repairSide = 0",
        "canShipyardRepair = 0",
    ] {
        let (db, warnings) = load(&format!("{CRAFT}{key}\n"), None);
        assert!(warnings.is_empty(), "{key}: {warnings:?}");
        assert_ne!(plain.hash(), db.hash(), "{key}");
    }
    let (lower, _) = load(&format!("{CRAFT}repairHardpoint = \"hp01\"\n"), None);
    let (upper, _) = load(&format!("{CRAFT}repairHardpoint = \"HP01\"\n"), None);
    assert_eq!(lower.hash(), upper.hash());

    for (a, b) in [
        ("enginesRepairTime = 1", "enginesRepairTime = 2"),
        ("weaponsRepairTime = 1", "weaponsRepairTime = 2"),
        ("shieldGeneratorRepairTime = 1", "shieldGeneratorRepairTime = 2"),
        ("lifeSupportRepairTime = 1", "lifeSupportRepairTime = 2"),
        ("sensorsRepairTime = 1", "sensorsRepairTime = 2"),
        ("repairFacility = 1", "repairFacility = 0"),
        ("canShipyardRepair = 1", "canShipyardRepair = 0"),
        ("repairHardpoint = \"hp01\"", "repairHardpoint = \"hp02\""),
        ("repairSide = 1", "repairSide = 2"),
    ] {
        let (left, _) = load(&format!("{CRAFT}{a}\n"), None);
        let (right, _) = load(&format!("{CRAFT}{b}\n"), None);
        assert_ne!(left.hash(), right.hash(), "{a} vs {b}");
    }
}

#[test]
fn fractional_seconds_are_fixed_values_not_rounded_ticks() {
    let (db, warnings) = load(
        &format!("{CRAFT}enginesRepairTime = 1.25\nweaponsRepairTime = 1.5\nshieldGeneratorRepairTime = 2.25\nlifeSupportRepairTime = 3.5\nsensorsRepairTime = 4.75\n"),
        None,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(repair(&db).system_times, [1.25, 1.5, 2.25, 3.5, 4.75].map(|v| Some(Fixed::from_f64(v))));
}

#[test]
fn repair_side_accepts_exact_i32_boundaries_and_rejects_overflow() {
    for side in [i32::MIN, i32::MAX] {
        let (db, warnings) = load(&format!("{CRAFT}repairSide = {side}\n"), None);
        assert!(warnings.is_empty(), "{side}: {warnings:?}");
        assert_eq!(repair(&db).repair_side, Some(side));
    }
    for side in ["2147483648", "-2147483649", "1000000001", "1.5"] {
        let (db, warnings) = load(&format!("{CRAFT}repairSide = {side}\n"), None);
        // 1000000001 is in range and must survive without Reader's 1e9 clamp.
        if side == "1000000001" {
            assert_eq!(repair(&db).repair_side, Some(1_000_000_001));
            assert!(warnings.is_empty(), "{warnings:?}");
        } else {
            assert_eq!(repair(&db).repair_side, None);
            assert!(warnings.iter().any(|w| w.message.contains("repairSide")), "{side}: {warnings:?}");
        }
    }
}
