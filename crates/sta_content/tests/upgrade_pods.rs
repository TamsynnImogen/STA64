use std::collections::BTreeMap;

use sta_content::{ContentDb, Fixed, OdfIndex, Overrides, Profile, Rules, UpgradeSystem, odf::load_with};

fn load(station: &str) -> (ContentDb, Vec<sta_content::Warning>) {
    let mut files = BTreeMap::new();
    files.insert(
        "station.odf".to_owned(),
        format!("classLabel = \"research\"\nunitName = \"Station\"\n{station}"),
    );
    for (name, system, tier) in [
        ("engine2", "engines", 2),
        ("engine3", "engines", 3),
        ("engine4", "engines", 4),
        ("engine5", "engines", 5),
        ("weapon3", "weapon_control", 3),
    ] {
        files.insert(format!("{name}.odf"), format!("classLabel = \"pod\"\nunitName = \"Pod\"\nisShipUpgrade = 1\nupgradeSystem = \"{system}\"\nupgradeLevel = {tier}\nupgradeMultiplier = 1.5\n"));
    }
    files.insert(
        "ordinary.odf".to_owned(),
        "classLabel = \"pod\"\nunitName = \"Ordinary\"\n".to_owned(),
    );
    let stems = files
        .keys()
        .map(|s| s.trim_end_matches(".odf").to_owned())
        .collect::<Vec<_>>();
    load_with(
        Profile::Native,
        &Overrides::default(),
        &stems,
        &mut |name, version| (version == 0).then(|| files.get(name).cloned()).flatten(),
    )
    .unwrap()
}

#[test]
fn mounted_invalid_child_keeps_parent_maximum() {
    let root = std::env::temp_dir().join(format!("sta64-upgrade-rule-{}", std::process::id()));
    let parent = root.join("parent");
    let child = root.join("child");
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::create_dir_all(&child).unwrap();
    std::fs::write(parent.join("RTS_CFG.h"), "int upgradePodMaximumTier = 9;\n").unwrap();
    std::fs::write(child.join("RTS_CFG.h"), "int upgradePodMaximumTier = 20;\n").unwrap();
    let mut vfs = sta_vfs::Vfs::default();
    vfs.push_dir("child".into(), child).unwrap();
    vfs.push_dir("parent".into(), parent).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, warnings) = ContentDb::load_as(&vfs, Profile::Native, &index).unwrap();
    assert_eq!(db.rules().upgrade_pod_maximum_tier(), 9);
    assert!(warnings.iter().any(|w| w.message.contains("upgradePodMaximumTier")));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pod_fields_are_typed_without_changing_legacy_flag() {
    let (db, warnings) = load("buildItem0 = \"engine2\"\nsecondaryBuildItem0 = \"engine3\"\n");
    assert!(warnings.is_empty(), "{warnings:?}");
    let pod = db.unit(db.key("engine2").unwrap()).unwrap();
    let upgrade = pod.system_upgrade.unwrap();
    assert_eq!(upgrade.system, UpgradeSystem::Engines);
    assert_eq!(upgrade.tier, 2);
    assert_eq!(upgrade.multiplier, Fixed::from_f64(1.5));
    assert!(!pod.research_station_upgrade);
}

#[test]
fn indexed_chains_preserve_holes_and_ordinary_items() {
    let (db, warnings) = load(
        "buildItem0 = \"engine2\"\nsecondaryBuildItem0 = \"engine3\"\ntier2BuildItem0 = \"engine4\"\ntier3BuildItem0 = \"engine5\"\nbuildItem4 = \"ordinary\"\nsecondaryBuildItem4 = \"ordinary\"\n",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let station = db.unit(db.key("station").unwrap()).unwrap();
    assert_eq!(
        station
            .research_build
            .iter()
            .map(|s| s.index)
            .collect::<Vec<_>>(),
        [0, 4]
    );
    assert_eq!(station.research_build[0].primary, db.key("engine2"));
    assert_eq!(
        station.research_build[0].successors,
        ["engine3", "engine4", "engine5"].map(|name| db.key(name).unwrap())
    );
    assert_eq!(station.research_build[1].primary, db.key("ordinary"));
    assert_eq!(
        station.research_build[1].successors,
        [db.key("ordinary").unwrap()]
    );
    assert_eq!(station.build.list.len(), 2);
    assert!(db.research_satisfies(db.key("engine4").unwrap(), db.key("engine2").unwrap()));
    assert!(!db.research_satisfies(db.key("engine2").unwrap(), db.key("engine4").unwrap()));
    assert!(!db.research_satisfies(db.key("engine4").unwrap(), db.key("ordinary").unwrap()));
}

#[test]
fn invalid_overrides_do_not_displace_native_slots_and_empty_clears_one_index() {
    let (db, warnings) = load(
        "buildItem0 = \"engine2\"\nsecondaryBuildItem0 = \"engine3\"\ntier1BuildItem0 = \"weapon3\"\ntier2BuildItem0 = \"engine4\"\nbuildItem4 = \"ordinary\"\ntier0BuildItem4 = \"\"\n",
    );
    let station = db.unit(db.key("station").unwrap()).unwrap();
    assert_eq!(
        station.research_build[0].successors,
        [db.key("engine3").unwrap(), db.key("engine4").unwrap()]
    );
    assert_eq!(station.research_build[1].primary, None);
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("tier1BuildItem") && w.message.contains("mismatched")),
        "{warnings:?}"
    );
    let (gap, warnings) = load("buildItem0 = \"engine2\"\ntier2BuildItem0 = \"engine4\"\n");
    assert!(
        gap.unit(gap.key("station").unwrap())
            .unwrap()
            .research_build[0]
            .successors
            .is_empty()
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("no preceding pod tier")),
        "{warnings:?}"
    );
}

#[test]
fn native_secondary_must_match_the_primary_ship_system() {
    let (db, warnings) = load("buildItem0 = \"engine2\"\nsecondaryBuildItem0 = \"weapon3\"\n");
    let station = db.unit(db.key("station").unwrap()).unwrap();
    assert!(station.research_build[0].successors.is_empty());
    assert!(warnings.iter().any(|w| w.message.contains("secondaryBuildItem0") && w.message.contains("matching tier-3")), "{warnings:?}");
}

#[test]
fn digest_tracks_effective_chains_and_rule_with_default_stable() {
    let (base, _) = load("buildItem0 = \"engine2\"\nsecondaryBuildItem0 = \"engine3\"\n");
    let (changed, _) = load(
        "buildItem0 = \"engine2\"\nsecondaryBuildItem0 = \"engine3\"\ntier2BuildItem0 = \"engine4\"\n",
    );
    assert_ne!(base.hash(), changed.hash());
    let (invalid_override, _) = load(
        "buildItem0 = \"engine2\"\nsecondaryBuildItem0 = \"engine3\"\ntier1BuildItem0 = \"weapon3\"\n",
    );
    assert_eq!(base.hash(), invalid_override.hash());
    let (plain, _) = Rules::from_header("");
    let (explicit, _) = Rules::from_header("int upgradePodMaximumTier = 6;\n");
    assert_eq!(plain.digest_entries(), explicit.digest_entries());
    assert_eq!(plain.upgrade_pod_maximum_tier(), 6);
    let (custom, _) = Rules::from_header("int upgradePodMaximumTier = 16;\n");
    assert_eq!(custom.upgrade_pod_maximum_tier(), 16);
    assert_ne!(plain.digest_entries(), custom.digest_entries());
    let (invalid, warnings) = Rules::from_header("int upgradePodMaximumTier = 17;\n");
    assert_eq!(invalid.upgrade_pod_maximum_tier(), 6);
    assert!(!warnings.is_empty());
    let (fractional, warnings) = Rules::from_header("int upgradePodMaximumTier = 3.5;\n");
    assert_eq!(fractional.upgrade_pod_maximum_tier(), 6);
    assert!(!warnings.is_empty());
}
