//! CON-51: the typed income keys of orbital mining. Authored data on a fake
//! ODF set; arbitrary numbers that match no stock file.

use sta_content::{ContentDb, Overrides, Profile, Races, Rules, odf::load_with};
use sta_core::Fixed;

fn load_as(profile: Profile, files: &[(&str, &str)]) -> (ContentDb, Vec<sta_content::Warning>) {
    let names: Vec<String> = files.iter().filter(|f| f.0.starts_with("u_")).map(|f| f.0.trim_end_matches(".odf").to_owned()).collect();
    load_with(profile, &Overrides::default(), &names, &mut |name, version| {
        files.iter().find(|f| f.0 == name && version == 0).map(|f| f.1.to_owned())
    })
    .unwrap()
}

fn load(files: &[(&str, &str)]) -> ContentDb {
    let (db, w) = load_as(Profile::Sta2Classic, files);
    assert!(w.is_empty(), "{w:?}");
    db
}

fn def<'a>(db: &'a ContentDb, name: &str) -> &'a sta_content::UnitDef {
    db.unit(db.key(name).unwrap()).unwrap()
}

fn fx(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

fn station(extra: &str) -> String {
    format!("unitName = \"x\"\nclassLabel = \"p_mining\"\norbitTime = 90\n{extra}")
}

#[test]
fn station_keys_are_typed_and_absent_ones_stay_none() {
    let db = load(&[
        ("u_a.odf", &station("miningMultiplier = 0.7\nresourcesCanHandle = \"latinum\" \"biomatter\"\n")),
        ("u_b.odf", &station("")),
        ("u_c.odf", &station("resourcesCanHandle = \"\"\n")),
        ("u_d.odf", "unitName = \"x\"\nclassLabel = \"station\"\nminingMultiplier = 0.7\nresourcesCanHandle = \"metal\"\n"),
    ]);
    let a = def(&db, "u_a").station_mining.unwrap();
    assert_eq!(a.multiplier, Some(fx(0.7)));
    assert_eq!(a.can_handle, Some([false, true, false, true]));
    assert_eq!(def(&db, "u_b").station_mining, None);
    let c = def(&db, "u_c").station_mining.unwrap();
    assert_eq!((c.multiplier, c.can_handle), (None, Some([false; 4])));
    // Only class p_mining carries them.
    assert_eq!(def(&db, "u_d").station_mining, None);
}

#[test]
fn a_bad_station_value_warns_and_is_ignored() {
    let (db, w) = load_as(
        Profile::Sta2Classic,
        &[("u_a.odf", &station("miningMultiplier = lots\nresourcesCanHandle = \"metal\" \"cheese\"\n"))],
    );
    let m = def(&db, "u_a").station_mining.unwrap();
    assert_eq!((m.multiplier, m.can_handle), (None, Some([false, false, true, false])));
    assert_eq!(w.len(), 2, "{w:?}");
    let (db, w) = load_as(Profile::Sta2Classic, &[("u_a.odf", &station("miningMultiplier = -1\n"))]);
    assert_eq!(def(&db, "u_a").station_mining, None);
    assert_eq!(w.len(), 1);
}

#[test]
fn node_keys_are_typed_through_the_planets_resource() {
    let planet = |res: &str| format!("unitName = \"x\"\nclassLabel = \"planet\"\nresource = \"{res}\"\n");
    let db = load(&[
        ("u_world.odf", &planet("rich")),
        ("u_plain.odf", &planet("plain")),
        ("rich.odf", "canHaveOrbitalStation = 1\nminingRate = 7\ntrickleAmount = 40\ntrickleMiningRateFraction = .25\namtmetal = 5000\namtbiomatter = 6500\nisInfinite = 0\n"),
        ("plain.odf", "canHaveOrbitalStation = 1\n"),
    ]);
    let n = def(&db, "u_world").node_income.unwrap();
    assert_eq!(n.mining_rate, Some(Fixed::from_int(7)));
    assert_eq!(n.trickle_amount, Some(Fixed::from_int(40)));
    assert_eq!(n.trickle_fraction, Some(fx(0.25)));
    assert_eq!(n.infinite, Some(false));
    assert_eq!(n.amounts, [None, None, Some(Fixed::from_int(5000)), Some(Fixed::from_int(6500))]);
    assert_eq!(def(&db, "u_plain").node_income, None);
    assert!(def(&db, "u_world").can_have_orbital_station);
}

#[test]
fn only_definitions_with_keys_move_the_digest() {
    let planet = "unitName = \"x\"\nclassLabel = \"planet\"\nresource = \"n\"\n";
    let base = load(&[("u_s.odf", &station("")), ("u_p.odf", planet), ("n.odf", "canHaveOrbitalStation = 1\n")]);
    let hash = |files: &[(&str, &str)]| load(files).hash();
    // Same data: same hash; the key-less database is the one without the keys.
    assert_eq!(base.hash(), hash(&[("u_s.odf", &station("")), ("u_p.odf", planet), ("n.odf", "canHaveOrbitalStation = 1\n")]));
    let moved = hash(&[("u_s.odf", &station("miningMultiplier = 0.5\n")), ("u_p.odf", planet), ("n.odf", "canHaveOrbitalStation = 1\n")]);
    assert_ne!(moved, base.hash());
    let other = hash(&[("u_s.odf", &station("miningMultiplier = 0.6\n")), ("u_p.odf", planet), ("n.odf", "canHaveOrbitalStation = 1\n")]);
    assert_ne!(other, moved);
    let handle = hash(&[("u_s.odf", &station("resourcesCanHandle = \"metal\"\n")), ("u_p.odf", planet), ("n.odf", "canHaveOrbitalStation = 1\n")]);
    assert_ne!(handle, base.hash());
    for key in ["miningRate = 3", "trickleAmount = 10", "trickleMiningRateFraction = 0.5", "isInfinite = 1", "amtmetal = 900"] {
        let node = format!("canHaveOrbitalStation = 1\n{key}\n");
        let h = hash(&[("u_s.odf", &station("")), ("u_p.odf", planet), ("n.odf", &node)]);
        assert_ne!(h, base.hash(), "{key}");
    }
    // Native content (the fake database) does not read these keys and keeps its pinned hash.
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
}

#[test]
fn race_modifiers_are_typed_and_digested_only_when_authored() {
    let files: &[(&str, &str)] = &[
        ("races", "race0=a\nrace1=b\nrace2=c"),
        ("a", "name=alpha\nminingRateModifier=1.5"),
        ("b", "name=beta\nminingRateModifier=.5f"),
        ("c", "name=gamma"),
    ];
    let mut loader = |n: &str, v: usize| files.iter().find(|f| f.0 == n.trim_end_matches(".odf") && v == 0).map(|f| f.1.to_owned());
    let (races, w) = Races::load_with_profile(Profile::Sta2Classic, &mut loader);
    assert!(w.is_empty(), "{w:?}");
    let rows: Vec<_> = races.mining_rate_modifiers().collect();
    assert_eq!(rows, vec![("alpha", fx(1.5)), ("beta", fx(0.5))]);
    let rules = Rules::default().with_race_mining(rows.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect());
    assert_eq!(rules.race_mining_rate_modifier("ALPHA"), Some(fx(1.5)));
    assert_eq!(rules.race_mining_rate_modifier("gamma"), None);
    let has = |r: &Rules| r.digest_entries().iter().any(|e| e.key == "race:mining-rate");
    assert!(has(&rules));
    assert!(!has(&Rules::default()));
}
