use std::collections::HashMap;

use sta_content::{ContentDb, Overrides, Profile, Rules};

fn load(profile: Profile, compat: bool, race_metadata: bool, base_list: &str) -> ContentDb {
    let files = HashMap::from([
        ("fed.odf", "name = \"federation\"\nofficerUpgradeODF = \"fedoff.odf\"\n".to_owned()),
        ("fedoff.odf", "classLabel = \"upgrade\"\nofficerGain = 25\n".to_owned()),
        ("romoff.odf", "classLabel = \"upgrade\"\nofficerGain = 17\n".to_owned()),
        ("fbase.odf", format!("classLabel = \"starbase\"\nrace = \"federation\"\nunitName = \"Base\"\nofficerGain = 30\nmaximumUpgrades = 4\n{base_list}")),
        ("fship.odf", "classLabel = \"craft\"\nunitName = \"Ship\"\n".to_owned()),
    ]);
    let mut stems = vec!["fedoff", "romoff", "fbase", "fship"];
    if race_metadata { stems.push("fed"); }
    let stems: Vec<_> = stems.into_iter().map(str::to_owned).collect();
    let mut overrides = Overrides::default();
    if compat { overrides.insert("a1_compat", true); }
    let (db, _) = sta_content::odf::load_with(profile, &overrides, &stems, &mut |name, version| {
        if version != 0 { return None; }
        let name = format!("{}.odf", name.trim_end_matches(".odf"));
        files.get(name.as_str()).cloned()
    }).unwrap();
    db
}

#[test]
fn native_starbase_resolves_its_race_upgrade_and_keeps_normal_order() {
    let db = load(Profile::Sta1Classic, false, true,
        "buildItem0 = \"fship\"\nbuildItem1 = \"romoff\"\nbuildItem2 = \"fedoff\"\n");
    assert!(db.a1_officers());
    let base = db.unit(db.key("fbase").unwrap()).unwrap();
    let rule = base.a1_base_officers.unwrap();
    assert_eq!((rule.gain, rule.maximum_upgrades, rule.upgrade), (30, 4, db.key("fedoff")));
    assert_eq!(base.build.list.iter().map(|r| r.odf.as_str()).collect::<Vec<_>>(), ["fship", "fedoff"]);
    assert_eq!(db.unit(db.key("fedoff").unwrap()).unwrap().a1_officer_upgrade_gain, Some(25));
    assert!(db.digest().get("unit:fbase").unwrap().contains("a1_base_officers"));
}

#[test]
fn native_metadata_offers_upgrade_even_without_build_item() {
    let db = load(Profile::Sta1Classic, false, true, "buildItem0 = \"fship\"\n");
    let base = db.unit(db.key("fbase").unwrap()).unwrap();
    assert_eq!(base.build.list.iter().map(|r| r.odf.as_str()).collect::<Vec<_>>(), ["fship", "fedoff"]);
}

#[test]
fn compat_fallback_requires_one_eligible_upgrade_and_a2_stays_unchanged() {
    let items = "buildItem0 = \"fship\"\nbuildItem1 = \"fedoff\"\n";
    let compat = load(Profile::Sta2Classic, true, false, items);
    assert_eq!(compat.unit(compat.key("fbase").unwrap()).unwrap().a1_base_officers.unwrap().upgrade, compat.key("fedoff"));
    let ambiguous = load(Profile::Sta2Classic, true, false, "buildItem0 = \"fedoff\"\nbuildItem1 = \"romoff\"\n");
    assert_eq!(ambiguous.unit(ambiguous.key("fbase").unwrap()).unwrap().a1_base_officers.unwrap().upgrade, None);
    let ordinary = load(Profile::Sta2Classic, false, true, items);
    assert!(!ordinary.a1_officers());
    let upgrade = ordinary.unit(ordinary.key("fedoff").unwrap()).unwrap();
    assert!(upgrade.a1_officer_upgrade_gain.is_none());
    let base = ordinary.unit(ordinary.key("fbase").unwrap()).unwrap();
    assert!(base.a1_base_officers.is_none());
    assert_eq!(base.build.list.iter().map(|r| r.odf.as_str()).collect::<Vec<_>>(), ["fship", "fedoff"]);
}

#[test]
fn a1_cap_reads_only_explicit_valid_cfg_max_officers() {
    let db = load(Profile::Sta1Classic, false, true, "");
    assert_eq!(db.a1_starting_officers(), None);
    let (rules, _) = Rules::from_header("int SINGLE_PLAYER_OFFICER_LIMIT = 7;\nint cfgMaxOfficers = 33;\n");
    let db = db.with_rules(rules);
    assert_eq!(db.a1_starting_officers(), Some(33));
    assert_eq!(db.digest().get("content:a1-max-officers"), Some("33"));
}
