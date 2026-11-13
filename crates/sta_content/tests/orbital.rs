//! SIM-62: `canHaveOrbitalStation` of a planet's resource node and `orbitTime` of a `p_mining` station.

use sta_content::{ContentDb, Overrides, Profile, odf::load_with};
use sta_core::Fixed;

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
fn the_keys_are_read_and_only_set_ones_move_the_hash() {
    let planet = |res: &str| format!("unitName = \"x\"\nclassLabel = \"planet\"\nresource = \"{res}\"\n");
    let station = |extra: &str| format!("unitName = \"x\"\nclassLabel = \"p_mining\"\n{extra}");
    let db = load(&[
        ("u_world.odf", &planet("slot")),
        ("u_rock.odf", &planet("noslot")),
        ("u_orbital.odf", &station("orbitTime = 120\n")),
        ("u_plain.odf", "unitName = \"x\"\nclassLabel = \"station\"\n"),
        ("slot.odf", "canHaveOrbitalStation = 1\n"),
        ("noslot.odf", "canHaveOrbitalStation = 0\n"),
    ]);
    assert!(def(&db, "u_world").can_have_orbital_station);
    assert!(!def(&db, "u_rock").can_have_orbital_station);
    assert_eq!(def(&db, "u_orbital").orbit_time, Some(Fixed::from_int(120)));
    assert_eq!(def(&db, "u_plain").orbit_time, None);
    // A p_mining station without the key is still orbital; its hash is the class's alone.
    let no_time = load(&[("u_orbital.odf", &station(""))]);
    assert_eq!(def(&no_time, "u_orbital").orbit_time, Some(Fixed::ZERO));
    let h = |t: &str| load(&[("u_orbital.odf", &station(t))]).hash();
    assert_eq!(h(""), h("orbitTime = 0\n"));
    assert_ne!(h(""), h("orbitTime = 60\n"));
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
}
