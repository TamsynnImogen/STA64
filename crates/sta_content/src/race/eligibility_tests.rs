use super::*;

fn load(profile: Profile, files: &[(&str, &str)]) -> (Races, Vec<Warning>) {
    Races::load_with_profile(profile, &mut |name, version| {
        (version == 0)
            .then(|| {
                files
                    .iter()
                    .find(|(key, _)| *key == stem(name))
                    .map(|(_, text)| (*text).into())
            })
            .flatten()
    })
}

#[test]
fn catalogue_slots_include_zero_and_do_not_depend_on_units_or_loadouts() {
    let (races, warnings) = load(
        Profile::Fo4,
        &[
            (
                "races",
                "numberofraces=5\nrace0=late\nrace1=first\nrace2=civilian\nrace3=negative\nrace4=malformed",
            ),
            ("late", "name=late\ninstantActionSlot=3"),
            ("first", "name=first\ninstantActionSlot=0"),
            ("civilian", "name=civilian\ninterfaceConfiguration=gui.cfg"),
            ("negative", "name=negative\ninstantActionSlot=-1"),
            ("malformed", "name=malformed\ninstantActionSlot=NaN"),
            ("unlisted", "name=unlisted\ninstantActionSlot=1"),
        ],
    );
    assert_eq!(warnings.len(), 2);
    let db = crate::fake::db();
    assert!(races.loadout(&db, "first").units.is_empty());
    assert_eq!(skirmish_races(&races, &db), ["first", "late"]);
}

#[test]
fn only_an_absent_catalogue_allows_unit_inference() {
    let db = crate::fake::db();
    let (absent, _) = load(Profile::Native, &[]);
    assert_eq!(skirmish_races(&absent, &db), ["fx"]);
    for profile in [Profile::Sta1Classic, Profile::Sta2Classic, Profile::Fo4] {
        let (missing, _) = load(profile, &[]);
        assert!(
            skirmish_races(&missing, &db).is_empty(),
            "{profile:?}: missing legacy catalogue must not promote NPCs"
        );
    }
    for files in [
        vec![("races", "numberofraces=0")],
        vec![("races", "#include \"missing\"")],
        vec![("races", "numberofraces=1\nrace0=missing")],
        vec![
            ("races", "numberofraces=1\nrace0=civilian"),
            ("civilian", "name=civilian"),
        ],
    ] {
        let (races, _) = load(Profile::Native, &files);
        assert!(skirmish_races(&races, &db).is_empty(), "{files:?}");
    }
    assert!(skirmish_races(&Races::from_defs(Vec::new()), &db).is_empty());
}

const A1: &[(&str, &str)] = &[
    ("races", "numberofraces=3\nrace0=federation\nrace1=borg\nrace2=civilian"),
    ("federation", "name=federation\ninterfaceConfiguration=gui_fed.cfg"),
    ("borg", "name=borg\ninterfaceConfiguration=gui_borg.cfg"),
    ("civilian", "name=civilian"),
];

#[test]
fn a1_legacy_menu_uses_interface_fields_in_declaration_order() {
    let (races, warnings) = load(Profile::Sta1Classic, A1);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(races.get("federation").unwrap().slot, Some(0));
    assert_eq!(races.get("borg").unwrap().slot, Some(1));
    assert_eq!(races.get("civilian").unwrap().slot, None);
    assert_eq!(skirmish_races(&races, &crate::fake::db()), ["federation", "borg"]);
    for profile in [Profile::Native, Profile::Sta2Classic, Profile::Fo4] {
        let (modern, _) = load(profile, A1);
        assert!(skirmish_races(&modern, &crate::fake::db()).is_empty(), "{profile:?}");
    }
}

#[test]
fn a1_fully_valid_explicit_slots_override_legacy_interface_defaults() {
    let files = [
        A1[0],
        (
            "federation",
            "name=federation\ninstantActionSlot=2\ninterfaceConfiguration=gui_fed.cfg",
        ),
        (
            "borg",
            "name=borg\ninstantActionSlot=0\ninterfaceConfiguration=gui_borg.cfg",
        ),
        ("civilian", "name=civilian\ninstantActionSlot=1"),
    ];
    let (races, warnings) = load(Profile::Sta1Classic, &files);
    assert!(warnings.is_empty());
    assert_eq!(
        skirmish_races(&races, &crate::fake::db()),
        ["borg", "civilian", "federation"]
    );
}

#[test]
fn a1_duplicate_or_out_of_range_slots_trigger_the_native_legacy_rule() {
    for second in ["0", "3"] {
        let borg = format!("name=borg\ninstantActionSlot={second}\ninterfaceConfiguration=gui_borg.cfg");
        let files = [
            A1[0],
            (
                "federation",
                "name=federation\ninstantActionSlot=0\ninterfaceConfiguration=gui_fed.cfg",
            ),
            ("borg", borg.as_str()),
            ("civilian", "name=civilian\ninstantActionSlot=2"),
        ];
        let (races, _) = load(Profile::Sta1Classic, &files);
        assert_eq!(skirmish_races(&races, &crate::fake::db()), ["federation", "borg"]);
        assert_eq!(races.get("civilian").unwrap().slot, None);
    }
}

#[test]
fn inherited_slot_and_child_override_keep_layered_odf_precedence() {
    let (races, warnings) = Races::load_with(&mut |name, version| match (stem(name).as_str(), version) {
        ("races", 0) => Some("numberofraces=1\nrace0=chosen".into()),
        ("chosen", 0) => Some("#include \"chosen.odf\"\ninstantActionSlot=0".into()),
        ("chosen", 1) => Some("#include \"base\"\nname=chosen".into()),
        ("base", 0) => Some("instantActionSlot=4\ninterfaceConfiguration=gui.cfg".into()),
        _ => None,
    });
    assert!(warnings.is_empty(), "{warnings:?}");
    let chosen = races.get("chosen").unwrap();
    assert_eq!(chosen.slot, Some(0));
    assert_eq!(chosen.interface.as_deref(), Some("gui.cfg"));
}

#[test]
fn playable_keys_keep_their_own_loadout_when_an_npc_stem_collides() {
    let (races, warnings) = load(
        Profile::Fo4,
        &[
            ("races", "numberofraces=3\nrace0=late\nrace1=fx\nrace2=chosen"),
            ("late", "name=fx\ninstantActionSlot=4\nnormalDilithium=44"),
            ("fx", "name=npc\nnormalDilithium=99\nstandardUnits1=fx_station"),
            (
                "chosen",
                "name=fx\ninstantActionSlot=0\nnormalDilithium=7\nstandardUnits1=fx_yard",
            ),
        ],
    );
    assert!(warnings.is_empty());
    assert_eq!(
        races.get("fx").unwrap().name,
        "npc",
        "general ODF-stem lookup is preserved"
    );
    assert_eq!(races.get_playable("FX").unwrap().stem, "chosen");
    assert_eq!(skirmish_races(&races, &crate::fake::db()), ["fx"]);
    let start = races.loadout(&crate::fake::db(), "fx");
    assert_eq!(start.source, "race chosen");
    assert_eq!(start.stock[0], Fixed::from_int(7));
}
