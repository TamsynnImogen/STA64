//! Synthetic profile hand-over tests, independent of installed content.
use sta_content::{Fixed, Overrides, Profile, Races, odf::load_with};

#[test]
fn only_a1_types_first_prefix_values_and_hashes_the_effective_result() {
    for profile in [Profile::Sta1Classic, Profile::Sta2Classic, Profile::Fo3, Profile::Fo4, Profile::Native] {
        let load = |extra: &str| load_with(profile, &Overrides::default(), &["ship".into()], &mut |name, version| {
            (name == "ship.odf" && version == 0).then(|| format!("classLabel=craft\nunitName=Test\nmaxHealthExtra={extra}\nmaxHealth=200\nmaxHealth=300\n"))
        }).unwrap().0;
        let db = load("100");
        let def = db.unit(db.key("ship").unwrap()).unwrap();
        assert_eq!(def.hull, Fixed::from_int(if profile == Profile::Sta1Classic { 100 } else { 300 }), "{profile:?}");
        assert_eq!(def.raw.source("maxHealth").unwrap().line, if profile == Profile::Sta1Classic { 3 } else { 5 });
        assert_eq!(db.hash() == load("101").hash(), profile != Profile::Sta1Classic, "{profile:?}");
    }
}

#[test]
fn race_catalogue_and_race_odfs_use_the_same_profile_rule() {
    for profile in [Profile::Sta1Classic, Profile::Sta2Classic] {
        let (races, warnings) = Races::load_with_profile(profile, &mut |name, version| {
            if version != 0 { return None; }
            match name {
                "races.odf" => Some("numberOfRaces=1\nrace0Extra=first\nrace0=second\n".into()),
                "first.odf" => Some("nameExtra=First\nname=Unused\n".into()),
                "second.odf" => Some("name=Second\n".into()),
                _ => None,
            }
        });
        assert!(warnings.is_empty(), "{profile:?}: {warnings:?}");
        assert_eq!(races.iter().next().unwrap().name, if profile == Profile::Sta1Classic { "first" } else { "second" });
    }
}

#[test]
fn a1_alias_only_numbered_keys_are_visited_without_exact_assignments() {
    let (races, warnings) = Races::load_with_profile(Profile::Sta1Classic, &mut |name, _| match name {
        "races.odf" => Some("numberOfRaces=1\nrace0Extra=first\n".into()),
        "first.odf" => Some("name=First\n".into()),
        _ => None,
    });
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(races.iter().next().unwrap().name, "first");
    let (db, warnings) = load_with(Profile::Sta1Classic, &Overrides::default(), &["ship".into(), "built".into()], &mut |name, _| match name {
        "ship.odf" => Some("classLabel=craft\nunitName=Ship\nbuildItem10Extra=built\n".into()),
        "built.odf" => Some("classLabel=craft\nunitName=Built\n".into()),
        _ => None,
    }).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let ship = db.unit(db.key("ship").unwrap()).unwrap();
    // buildItem1 and buildItem10 both select the first matching assignment.
    assert_eq!(ship.build.list.len(), 2);
    assert!(ship.build.list.iter().all(|item| item.odf == "built"));
}

#[test]
fn a1_build_slot_flags_are_discovered_through_prefix_aliases() {
    for policy in [sta_odf::LookupPolicy::PrefixFirst, sta_odf::LookupPolicy::Exact] {
        let raw = sta_odf::resolve_layered_with_policy("yard", policy, &mut |_, _| Some(
            "buildItem1=ship\nbuildItem1availabilityExtra=1\n".into()
        )).unwrap();
        assert_eq!(sta_content::BuildDef::flagged_slots_of(&raw),
            (policy == sta_odf::LookupPolicy::PrefixFirst).then(|| vec!["ship".to_owned()]));
    }
}
