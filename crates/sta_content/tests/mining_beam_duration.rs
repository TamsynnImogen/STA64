//! A mining beam's `lifeSpan` reaches its weapon's `duration` although its ordnance class
//! (`minebm`) is not one of the drawn beam classes. Authored numbers; the install-gated part
//! asserts the rule on real data and no values.

use std::path::Path;

use sta_content::{ContentDb, OdfIndex, Overrides, Profile, WeaponKind, odf::load_with};
use sta_vfs::Vfs;

fn db(class: &str) -> ContentDb {
    let ord = format!("classLabel = \"{class}\"\nlifeSpan = 2.0\nshotSpeed = 1000\n");
    let files = [
        ("w_a", "wpnName = \"a\"\nclassLabel = \"cannon\"\nordName = \"o_a\"\nrange = 150\nshotDelay = 3\n".to_owned()),
        ("o_a", ord),
    ];
    let stems: Vec<String> = files.iter().map(|f| f.0.to_owned()).collect();
    load_with(Profile::Sta2Classic, &Overrides::default(), &stems, &mut |n, v| {
        files.iter().find(|f| f.0 == n.trim_end_matches(".odf") && v == 0).map(|f| f.1.clone())
    })
    .unwrap()
    .0
}

#[test]
fn a_mining_beam_keeps_its_kind_and_takes_the_ordnance_life_span() {
    let d = db("minebm");
    let w = d.weapons().find(|w| w.odf == "w_a").unwrap();
    assert_eq!(w.kind, WeaponKind::Projectile);
    assert_eq!(w.duration, 60);
    // Another projectile class keeps no duration, as before.
    let d = db("torpedo");
    assert_eq!(d.weapons().find(|w| w.odf == "w_a").unwrap().duration, 0);
}

#[test]
fn install_orbital_stations_with_a_multiplier_have_a_beam_with_a_positive_duration() {
    let heroic = Path::new("/home/cedric/Games/Heroic");
    let roots = "Star Trek Armada II Fleet Operations Roots";
    for (dir, modname) in [("Star Trek Armada II (Stock)", None), (roots, Some("STA2 Classic Mod")), ("Star Trek Armada II Fleet Operations", None), (roots, None)] {
        let root = heroic.join(dir);
        if !root.is_dir() {
            eprintln!("{dir}: skipped, not installed");
            continue;
        }
        let vfs = match Vfs::mount_install(&root, modname) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let (index, _) = OdfIndex::build(&vfs);
        let Ok((db, _)) = ContentDb::load(&vfs, &index) else { continue };
        for u in db.units().filter(|u| u.station_mining.is_some_and(|m| m.multiplier.is_some())) {
            let beam = u.hardpoints.iter().filter_map(|h| db.weapon(h.weapon.key)).find(|w| w.class.0 == "minebm");
            let beam = beam.unwrap_or_else(|| panic!("{dir}: {} has no mining beam", u.odf));
            assert!(beam.duration > 0, "{dir}: {} beam duration", u.odf);
            assert!(beam.refire > 0 && beam.duration <= beam.refire, "{dir}: {} pulse fits its period", u.odf);
        }
    }
}
