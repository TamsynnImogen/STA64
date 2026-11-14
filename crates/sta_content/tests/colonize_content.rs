//! SIM-30: typed `maxPopulation`, the colonizer pod keys and the population
//! rules of `RTS_CFG.h`. Planet levels and the pod keys are read from data
//! (stock Armada II: D/J unpopulated, H sparse, K light, L medium, M heavy; pod
//! `partySize` 100, `orbitRange` 150).

use std::path::Path;

use sta_content::{ContentDb, OdfIndex, Overrides, PlanetRole, Profile, Warning, odf::load_with};
use sta_core::Fixed;
use sta_vfs::Vfs;

fn load(profile: Profile, files: &'static [(&str, &str)], stems: &[&str]) -> (ContentDb, Vec<Warning>) {
    let stems: Vec<String> = stems.iter().map(|s| (*s).to_owned()).collect();
    load_with(profile, &Overrides::default(), &stems, &mut |name, version| {
        if version != 0 {
            return None;
        }
        files.iter().find(|(n, _)| *n == name).map(|(_, t)| (*t).to_owned())
    })
    .unwrap()
}

const PLANET: &str = "[GameObjectClass]\nclassLabel = \"planet\"\n";

#[test]
fn max_population_words_are_levels_and_case_insensitive() {
    for (word, level) in [("unpopulated", 0), ("Sparse", 1), ("LIGHT", 2), ("medium", 3), ("Heavy", 4)] {
        let text: &'static str = Box::leak(format!("{PLANET}maxPopulation = \"{word}\"\n").into_boxed_str());
        let files: &'static [(&str, &str)] = Box::leak(vec![("p.odf", text)].into_boxed_slice());
        let (db, w) = load(Profile::Sta2Classic, files, &["p"]);
        assert!(w.is_empty(), "{word}: {w:?}");
        let d = db.unit(db.key("p").unwrap()).unwrap();
        assert_eq!((d.planet_role, d.planet_max_population), (Some(PlanetRole::Producer), level), "{word}");
    }
}

#[test]
fn bad_word_warns_and_absent_and_other_roles_are_unpopulated() {
    let (db, w) = load(Profile::Sta2Classic, &[("p.odf", "[GameObjectClass]\nclassLabel = \"planet\"\nmaxPopulation = \"crowded\"\n")], &["p"]);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(db.unit(db.key("p").unwrap()).unwrap().planet_max_population, 0);
    let (db, w) = load(Profile::Sta2Classic, &[("p.odf", PLANET)], &["p"]);
    assert!(w.is_empty());
    assert_eq!(db.unit(db.key("p").unwrap()).unwrap().planet_max_population, 0);
    // A background planet neither reads nor diagnoses the key.
    let (db, w) = load(Profile::Sta2Classic, &[("b.odf", "[GameObjectClass]\nclassLabel = \"planet_obj\"\nmaxPopulation = \"crowded\"\n")], &["b"]);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(db.unit(db.key("b").unwrap()).unwrap().planet_max_population, 0);
}

const SHIP: &[(&str, &str)] = &[
    ("ship.odf", "[GameObjectClass]\nclassLabel = \"colony\"\nunitName = \"Colony\"\nweapon1 = \"pod\"\ncolonists = 1000\n"),
    ("pod.odf", "[WeaponClass]\nclassLabel = \"launcher\"\nwpnName = \"Pods\"\nordName = \"podo\"\nshotDelay = 5.0\nrange = 1000\n"),
    ("podo.odf", "[OrdnanceClass]\nclassLabel = \"ColonizerPod\"\npartySize = 60\norbitRange = 120.0f\n"),
    ("beam.odf", "[WeaponClass]\nclassLabel = \"cannon\"\nwpnName = \"Beam\"\nordName = \"beamo\"\nshotDelay = 5.0\n"),
    ("beamo.odf", "[OrdnanceClass]\nclassLabel = \"colonybm\"\ntransferAmount = 100\n"),
];

#[test]
fn pod_ordnance_keys_reach_the_weapon_def() {
    let (db, w) = load(Profile::Sta2Classic, SHIP, &["ship", "pod", "beam"]);
    assert!(w.is_empty(), "{w:?}");
    let pod = db.weapon(db.key("pod").unwrap()).unwrap();
    let c = pod.colonize.unwrap();
    assert_eq!((c.party_size, c.orbit_range, pod.refire), (Fixed::from_int(60), Fixed::from_int(120), 150));
    assert!(db.weapon(db.key("beam").unwrap()).unwrap().colonize.is_none());
    // The content hash sees the pod keys.
    let changed: &'static [(&str, &str)] = Box::leak(
        SHIP.iter().map(|(n, t)| if *n == "podo.odf" { (*n, "[OrdnanceClass]\nclassLabel = \"ColonizerPod\"\npartySize = 61\norbitRange = 120.0f\n") } else { (*n, *t) }).collect::<Vec<_>>().into_boxed_slice(),
    );
    let (db2, _) = load(Profile::Sta2Classic, changed, &["ship", "pod", "beam"]);
    assert_ne!(db.hash(), db2.hash());
}

#[test]
fn population_level_changes_the_hash_and_the_fake_hash_is_unchanged() {
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
    let light: &'static [(&str, &str)] = &[("p.odf", "[GameObjectClass]\nclassLabel = \"planet\"\nmaxPopulation = \"light\"\n")];
    let heavy: &'static [(&str, &str)] = &[("p.odf", "[GameObjectClass]\nclassLabel = \"planet\"\nmaxPopulation = \"heavy\"\n")];
    let none: &'static [(&str, &str)] = &[("p.odf", PLANET)];
    let h = |f| load(Profile::Sta2Classic, f, &["p"]).0.hash();
    assert_ne!(h(light), h(heavy));
    assert_ne!(h(light), h(none));
}

/// The stock Armada II install, or `STA64_TEST_INSTALL`; skipped when absent.
fn install() -> Option<Vfs> {
    let root = std::env::var("STA64_TEST_INSTALL").unwrap_or_else(|_| "/home/cedric/Games/Heroic/Star Trek Armada II (Stock)".into());
    let root = Path::new(&root);
    root.is_dir().then(|| Vfs::mount_install(root, None).ok()).flatten()
}

#[test]
fn installed_a2_planets_and_colony_ships_follow_the_survey() {
    let Some(vfs) = install() else { return };
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    if db.profile() != Profile::Sta2Classic {
        return;
    }
    // class letter -> (level, colonizable)
    for (odf, level, colonizable) in [("pb_clssd", 0, false), ("pb_clssj", 0, false), ("pb_clssh", 1, true), ("pb_clssk", 2, true), ("pb_clssl", 3, true), ("pb_clssm", 4, true)] {
        let Some(key) = db.key(odf) else { continue };
        let u = db.unit(key).unwrap();
        assert_eq!((u.planet_max_population, u.planet_colonizable), (level, colonizable), "{odf}");
    }
    let mut pods = 0;
    for u in db.units().filter(|u| u.class.0 == "colony") {
        let Some(h) = u.hardpoints.first() else { continue };
        let Some(w) = db.weapon(h.weapon.key) else { continue };
        if let Some(c) = w.colonize {
            pods += 1;
            assert_eq!((c.party_size, c.orbit_range, w.refire), (Fixed::from_int(100), Fixed::from_int(150), 150), "{}", u.odf);
        }
    }
    assert!(pods >= 4, "{pods} pod colony ships");
}
