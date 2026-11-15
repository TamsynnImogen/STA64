//! SIM-46 P0: the transporter and boarding keys of unit ODFs, race ODFs and
//! `RTS_CFG.h` are typed and exposed; nothing reads them yet, so they do not
//! move the content hash. Authored fixtures first, then install-gated counts.

use std::path::Path;

use sta_content::{ContentDb, Fixed, OdfIndex, Overrides, Profile, RaceBoarding, Races, Rules, TransporterRules, odf::load_with};
use sta_vfs::Vfs;

fn load(files: &[(&str, &str)]) -> ContentDb {
    let names: Vec<String> = files.iter().filter(|f| f.0.starts_with("u_")).map(|f| f.0.trim_end_matches(".odf").to_owned()).collect();
    load_with(Profile::Sta2Classic, &Overrides::default(), &names, &mut |name, version| {
        files.iter().find(|f| f.0 == name && version == 0).map(|f| f.1.to_owned())
    })
    .unwrap()
    .0
}

fn def<'a>(db: &'a ContentDb, name: &str) -> &'a sta_content::UnitDef {
    db.unit(db.key(name).unwrap()).unwrap()
}

#[test]
fn unit_keys_are_read_with_their_defaults() {
    let db = load(&[
        ("u_base.odf", "unitName = \"x\"\nclassLabel = \"craft\"\ntransporter = 1\nhas_crew = 1\nshieldDelay = 3.0f\n"),
        ("u_child.odf", "#include \"u_base.odf\"\nassault = 1\nboardingPartyStrength = 5.0f\ndisableShieldsOnTransport = 0\nweldingAnimSpeed = 0.0\neventTransport = \"PicardTransport\"\n"),
        ("u_none.odf", "unitName = \"x\"\nclassLabel = \"craft\"\n"),
        ("u_zero.odf", "unitName = \"x\"\nclassLabel = \"craft\"\ntransporter = 0\nhas_crew = 0\nboardingPartyStrength = 0\nweldingAnimSpeed = 1.0\n"),
    ]);
    let none = &def(&db, "u_none").transport;
    assert_eq!(*none, sta_content::TransportDef::default());
    assert_eq!((none.transporter, none.has_crew, none.assault), (None, None, false));
    assert_eq!(none.boarding_party_strength, Fixed::ONE);
    assert!(none.disable_shields_on_transport);
    assert_eq!((none.welding_anim_speed, none.shield_delay, none.event_transport.as_deref()), (None, None, None));
    // The key set through an include reaches the child; the child's own keys follow.
    let child = &def(&db, "u_child").transport;
    assert_eq!((child.transporter, child.has_crew, child.assault), (Some(true), Some(true), true));
    assert_eq!(child.boarding_party_strength, Fixed::from_int(5));
    assert!(!child.disable_shields_on_transport, "an explicit zero is kept");
    assert_eq!(child.welding_anim_speed, Some(Fixed::ZERO), "an explicit zero is kept, not absent");
    assert_eq!(child.shield_delay, Some(Fixed::from_int(3)));
    assert_eq!(child.event_transport.as_deref(), Some("PicardTransport"));
    let zero = &def(&db, "u_zero").transport;
    assert_eq!((zero.transporter, zero.has_crew), (Some(false), Some(false)));
    assert_eq!(zero.boarding_party_strength, Fixed::ZERO, "an explicit zero is not the default 1.0");
    assert_eq!(zero.welding_anim_speed, Some(Fixed::ONE));
}

#[test]
fn the_keys_do_not_move_the_content_hash() {
    let plain = "unitName = \"x\"\nclassLabel = \"craft\"\n";
    let keyed = "unitName = \"x\"\nclassLabel = \"craft\"\ntransporter = 1\nhas_crew = 1\nassault = 1\nboardingPartyStrength = 5\ndisableShieldsOnTransport = 0\nweldingAnimSpeed = 0\nshieldDelay = 3\neventTransport = \"e\"\n";
    assert_eq!(load(&[("u_a.odf", plain)]).hash(), load(&[("u_a.odf", keyed)]).hash());
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
}

#[test]
fn bad_values_keep_the_defaults() {
    let db = load(&[("u_a.odf", "unitName = \"x\"\nclassLabel = \"craft\"\nboardingPartyStrength = lots\ndisableShieldsOnTransport = maybe\nshieldDelay = -2\n")]);
    let t = &def(&db, "u_a").transport;
    assert_eq!(t.boarding_party_strength, Fixed::ONE);
    assert!(t.disable_shields_on_transport);
    assert_eq!(t.shield_delay, Some(Fixed::ZERO), "negative seconds read as zero");
}

#[test]
fn rules_keys_default_to_the_engine_values() {
    let d = TransporterRules::default();
    assert_eq!((d.range, d.delay, d.action_delay, d.max), (Fixed::from_int(300), Fixed::ONE, Fixed::ONE, 5));
    let (none, w) = Rules::from_header("int MAX_CREW = 5;\n");
    assert!(w.is_empty());
    assert_eq!(none.transporter(), d, "no key: the engine defaults");
    let (set, w) = Rules::from_header("float TRANSPORTER_RANGE = 840.0;\nfloat TRANSPORTER_DELAY = 0;\nint   TRANSPORTER_MAX = 3;\n");
    assert!(w.is_empty(), "{w:?}");
    let t = set.transporter();
    assert_eq!((t.range, t.delay, t.max), (Fixed::from_int(840), Fixed::ZERO, 3), "explicit zero kept");
    assert_eq!(t.action_delay, Fixed::ONE, "a key the header leaves out keeps the engine default");
    let (bad, w) = Rules::from_header("float TRANSPORTER_RANGE = far;\nint TRANSPORTER_MAX = -1;\n");
    assert_eq!(w.len(), 2);
    assert_eq!(bad.transporter(), d);
    // Not digested (nothing reads them yet): the header alone does not change the rules digest.
    let (plain, _) = Rules::from_header("int MAX_CREW = 5;\n");
    let (keyed, _) = Rules::from_header("int MAX_CREW = 5;\nfloat TRANSPORTER_RANGE = 840.0;\nint TRANSPORTER_MAX = 3;\n");
    assert_eq!(plain.digest_entries(), keyed.digest_entries());
    assert!(!sta_content::TYPED_CFG_KEYS.contains(&"TRANSPORTER_RANGE"));
}

#[test]
fn race_keys_are_read_with_their_defaults() {
    let dir = std::env::temp_dir().join(format!("sta64-transport-keys-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("odf")).unwrap();
    let write = |name: &str, text: &str| std::fs::write(dir.join("odf").join(name), text).unwrap();
    write("races.odf", "numberOfRaces = 3\nrace0 = \"RaceA.odf\"\nrace1 = \"RaceB.odf\"\nrace2 = \"RaceC.odf\"\n");
    write("RaceA.odf", "name = \"Alpha\"\nboardingStrength = 1.75f\ncrewRetreatRatio = 0.2f // trailing comment\ntransportSprite = \"btransport\"\ntransportToFriendsOnNotEasy = 1\ntransportToEnemyOnHard = 1\n");
    write("RaceB.odf", "name = \"Beta\"\nboardingStrength = 0.0f\ncanCommandeer = 0\ncanBeCommandeered = 0\ntransportToFriendsOnNotEasy = 0\n");
    write("RaceC.odf", "name = \"Gamma\"\n");
    let mut vfs = Vfs::default();
    vfs.push_dir("layer0".into(), dir.clone()).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (races, _) = Races::load(&vfs, &index);
    std::fs::remove_dir_all(&dir).unwrap();
    let a = races.boarding("ALPHA");
    assert_eq!(a.boarding_strength, Fixed::from_f64(1.75));
    assert_eq!(a.crew_retreat_ratio, Fixed::from_f64(f64::from(0.2f32)));
    assert_eq!(a.transport_sprite.as_deref(), Some("btransport"));
    assert!(a.transport_to_enemy_on_hard && a.transport_to_friends_on_not_easy && a.can_commandeer && a.can_be_commandeered);
    let b = races.boarding("beta");
    assert_eq!(b.boarding_strength, Fixed::ZERO, "an explicit zero is kept");
    assert!(!b.can_commandeer && !b.can_be_commandeered && !b.transport_to_friends_on_not_easy && !b.transport_to_enemy_on_hard);
    assert_eq!(b.transport_sprite, None);
    // A race that sets none, and one that does not exist, read the defaults.
    assert_eq!(races.boarding("gamma"), RaceBoarding::default());
    assert_eq!(races.boarding("nobody"), RaceBoarding::default());
    let d = RaceBoarding::default();
    assert_eq!((d.boarding_strength, d.can_commandeer, d.can_be_commandeered), (Fixed::ONE, true, true));
    assert_eq!(d.crew_retreat_ratio, Fixed::from_f64(f64::from(0.15f32)));
    assert!(!d.transport_to_enemy_on_hard && d.transport_to_friends_on_not_easy);
}

#[test]
fn every_new_key_is_counted_as_read_by_the_coverage_list() {
    for k in [
        "transporter", "has_crew", "assault", "boardingPartyStrength", "disableShieldsOnTransport", "weldingAnimSpeed",
        "shieldDelay", "eventTransport", "boardingStrength", "canCommandeer", "canBeCommandeered", "transportSprite",
        "crewRetreatRatio", "transportToEnemyOnHard", "transportToFriendsOnNotEasy",
    ] {
        assert!(sta_content::coverage::READ_ODF_KEYS.contains(&k.to_ascii_lowercase().as_str()), "{k}");
    }
}

const HEROIC: &str = "/home/cedric/Games/Heroic";
const ROOTS: &str = "Star Trek Armada II Fleet Operations Roots";

#[derive(Default, Debug)]
struct Counts {
    units: usize,
    transporter: usize,
    transporter_off: usize,
    has_crew: usize,
    has_crew_off: usize,
    assault: usize,
    party: usize,
    shields_stay: usize,
    welding_zero: usize,
    welding: usize,
    delay: usize,
    event: usize,
    races: usize,
    race_strength: usize,
    race_commandeer_off: usize,
    race_sprite: usize,
    race_retreat: usize,
    race_policy: usize,
}

fn count(dir: &str, modname: Option<&str>) -> Option<(Counts, TransporterRules)> {
    let root = Path::new(HEROIC).join(dir);
    if !root.is_dir() {
        eprintln!("{dir}: skipped, not installed");
        return None;
    }
    let vfs = match Vfs::mount_install(&root, modname) {
        Ok(v) => v,
        Err(_) => {
            let mut v = Vfs::default();
            v.push_dir(dir.into(), root.clone()).ok()?;
            v
        }
    };
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).ok()?;
    let mut c = Counts::default();
    for u in db.units() {
        let t = &u.transport;
        c.units += 1;
        c.transporter += usize::from(t.transporter == Some(true));
        c.transporter_off += usize::from(t.transporter == Some(false));
        c.has_crew += usize::from(t.has_crew == Some(true));
        c.has_crew_off += usize::from(t.has_crew == Some(false));
        c.assault += usize::from(t.assault);
        c.party += usize::from(u.raw.raw("boardingPartyStrength").is_some());
        c.shields_stay += usize::from(!t.disable_shields_on_transport);
        c.welding += usize::from(t.welding_anim_speed.is_some());
        c.welding_zero += usize::from(t.welding_anim_speed == Some(Fixed::ZERO));
        c.delay += usize::from(t.shield_delay.is_some());
        c.event += usize::from(t.event_transport.is_some());
    }
    let (races, _) = Races::load(&vfs, &index);
    for r in races.iter() {
        c.races += 1;
        let b = races.boarding(&r.name);
        let d = RaceBoarding::default();
        c.race_strength += usize::from(b.boarding_strength != d.boarding_strength);
        c.race_commandeer_off += usize::from(!b.can_commandeer || !b.can_be_commandeered);
        c.race_sprite += usize::from(b.transport_sprite.is_some());
        c.race_retreat += usize::from(b.crew_retreat_ratio != d.crew_retreat_ratio);
        c.race_policy += usize::from(b.transport_to_enemy_on_hard != d.transport_to_enemy_on_hard || b.transport_to_friends_on_not_easy != d.transport_to_friends_on_not_easy);
    }
    Some((c, db.rules().transporter()))
}

/// Read from the installs at run time; absent installs are skipped. Counts are
/// printed with `--nocapture`.
#[test]
fn installed_content_carries_the_keys() {
    let cases = [
        ("A1", "Star Trek Armada", None),
        ("A2", "Star Trek Armada II (Stock)", None),
        ("FO 3.2.7", "Star Trek Armada II Fleet Operations", None),
        ("DW", "Star Trek Armada II Dominion War", None),
        ("Roots", ROOTS, None),
    ];
    for (label, dir, modname) in cases {
        let Some((c, t)) = count(dir, modname) else { continue };
        eprintln!("{label}: {c:?} rules {t:?}");
        assert!(c.units > 0, "{label}");
        // Lower bounds a little under what the installs carry (counts of definitions after includes).
        let hundred = |n: i32| Fixed::from_int(n);
        match label {
            "A1" => {
                assert_eq!((c.transporter, c.has_crew, c.assault, c.party, c.welding), (0, 0, 0, 0, 0), "A1 has none of these unit keys");
                assert!(c.races >= 8 && c.race_strength >= 6 && c.race_retreat >= 4, "A1 races carry boardingStrength and crewRetreatRatio");
                assert_eq!((t.range, t.max), (hundred(500), 5), "A1 header");
            }
            "A2" => {
                assert!(c.transporter >= 170 && c.transporter_off >= 10 && c.has_crew >= 190 && c.has_crew_off >= 100, "A2: {c:?}");
                assert!(c.assault >= 5 && c.party >= 10 && c.event >= 3 && c.delay >= 300, "A2: {c:?}");
                assert!(c.race_strength >= 6 && c.race_sprite >= 5 && c.race_retreat >= 6 && c.race_policy >= 3 && c.race_commandeer_off >= 1, "A2: {c:?}");
                assert_eq!((t.range, t.max, c.welding, c.shields_stay), (hundred(500), 5, 0, 0), "A2 header");
            }
            "FO 3.2.7" | "DW" => {
                assert!(c.transporter >= 2100 && c.transporter_off >= 70 && c.has_crew >= 1900, "{label}: {c:?}");
                assert!(c.assault >= 30 && c.party >= 150 && c.shields_stay >= 30 && c.race_sprite >= 9, "{label}: {c:?}");
                assert_eq!((t.range, t.max, c.welding), (hundred(1000), 3, 0), "{label} header");
            }
            _ => {
                assert!(c.transporter >= 450 && c.transporter_off >= 80 && c.has_crew >= 350, "Roots: {c:?}");
                assert!(c.welding >= 250 && c.welding_zero >= 200 && c.shields_stay >= 50 && c.race_sprite >= 12, "Roots: {c:?}");
                assert_eq!((t.range, t.max, c.assault, c.party), (hundred(840), 5, 0, 0), "Roots header");
            }
        }
    }
}
