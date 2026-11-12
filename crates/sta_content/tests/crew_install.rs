//! SIM-25, install-gated: the crew rules come from the installed data. Classic
//! (stock Armada II rules) gives the Federation 4 crew per second per
//! starbase; Fleet Ops Roots gives its playable races none. Absent or
//! unmountable installs are skipped.

use std::path::Path;

use sta_content::{ContentDb, OdfIndex};
use sta_vfs::Vfs;

const ROOTS: &str = "/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations Roots";

fn rates(active: Option<&str>) -> Option<(Option<i64>, Vec<(String, f64)>)> {
    let root = Path::new(ROOTS);
    if !root.is_dir() {
        return None;
    }
    let vfs = Vfs::mount_install(root, active).ok()?;
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).ok()?;
    let rules = db.rules().crew()?.clone();
    Some((rules.starting_capacity, rules.races.iter().map(|(k, r)| (k.clone(), f64::from(r.to_f32()))).collect()))
}

#[test]
fn classic_federation_accumulates_four_per_second_and_roots_none() {
    if let Some((cap, races)) = rates(Some("STA2 Classic Mod")) {
        assert!(cap.is_some_and(|c| c > 0), "STARTING_CREW_CAPACITY is read");
        let fed = races.iter().find(|(k, _)| k == "federation").expect("federation race");
        assert!((fed.1 - 4.0).abs() < 1e-6, "{fed:?}");
    }
    if let Some((cap, races)) = rates(None) {
        assert_eq!(cap, Some(1000));
        for (k, r) in races.iter().filter(|(k, _)| ["unitedfederation", "federation", "klingonempire", "klingon", "romulanstarempire", "cardassianunion"].contains(&k.as_str())) {
            assert_eq!(*r, 0.0, "{k}");
        }
    }
}
