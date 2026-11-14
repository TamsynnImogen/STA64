//! SIM-54, install-gated: every installed profile loads the race keys the sim
//! reads for a team's race (`maxOfficerMultiplier`, the derived or authored
//! latinum factor) and `MAX_CREW`. Absent installs are skipped. Read from data.

use std::path::Path;

use sta_content::{ContentDb, OdfIndex};
use sta_core::Fixed;
use sta_vfs::Vfs;

const HEROIC: &str = "/home/cedric/Games/Heroic";
const ROOTS: &str = "Star Trek Armada II Fleet Operations Roots";

fn load(dir: &str, modname: Option<&str>) -> Option<ContentDb> {
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
    ContentDb::load(&vfs, &index).ok().map(|(db, _)| db)
}

#[test]
fn every_installed_profile_loads_the_seat_race_rules() {
    let cases = [
        ("A1", "Star Trek Armada", None),
        ("A2", "Star Trek Armada II (Stock)", None),
        ("Classic", ROOTS, Some("STA2 Classic Mod")),
        ("Classic Enhanced", ROOTS, Some("STA2 Classic Mod Enhanced Gameplay")),
        ("FO 3.2.7", "Star Trek Armada II Fleet Operations", None),
        ("DW", "Star Trek Armada II Dominion War", None),
        ("Roots", ROOTS, None),
    ];
    for (label, dir, modname) in cases {
        let Some(db) = load(dir, modname) else { continue };
        let rules = db.rules();
        let spec = rules.seat_race("species8472");
        let borg = rules.seat_race("borg");
        let max_crew = rules.crew().and_then(|c| c.max_crew);
        eprintln!("{label}: species8472 {spec:?}, borg {borg:?}, MAX_CREW {max_crew:?}");
        match label {
            // Only species8472 sets maxOfficerMultiplier (0.24) in A2 stock and Classic (read from data).
            "A2" | "Classic" | "Classic Enhanced" => {
                assert_eq!(spec.officer_multiplier, Fixed::from_f64(0.24f32 as f64), "{label}");
                assert_eq!(max_crew, Some(99999), "{label}");
            }
            "A1" => assert_eq!(spec.officer_multiplier, Fixed::ONE, "{label}: A1 has no such key"),
            _ => assert_eq!(spec.officer_multiplier, Fixed::ONE, "{label}"),
        }
        // The Borg latinum rule is derived from the race name, except for Armada I (not traced).
        let want = if label == "A1" { Fixed::ONE } else { Fixed::ZERO };
        assert_eq!(borg.latinum_scale, want, "{label}");
        assert_eq!(borg.officer_multiplier, Fixed::ONE, "{label}");
        // No other race of any install changes the cap or the cost.
        assert_eq!(rules.seat_race("federation").latinum_scale, Fixed::ONE, "{label}");
    }
}
