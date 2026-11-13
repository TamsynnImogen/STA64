//! SIM-25, install-gated: the crew rules come from the installed data. Classic
//! (stock Armada II rules) gives the Federation 4 crew per second per
//! starbase; Fleet Ops Roots gives its playable races none. Absent or
//! unmountable installs are skipped.

use std::path::Path;

use sta_content::{ContentDb, OdfIndex};
use sta_vfs::Vfs;

const ROOTS: &str = "/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations Roots";

type Rates = (Option<i64>, Vec<(String, f64)>);

fn rates(active: Option<&str>) -> Option<Rates> {
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

/// SIM-39: the per-hit crew keys are typed from the ordnance and the unit
/// ODFs (stock Armada II rules via the Classic mod; absent installs skip).
#[test]
fn classic_ordnance_and_units_carry_the_crew_loss_keys() {
    let root = Path::new(ROOTS);
    if !root.is_dir() {
        return;
    }
    let Ok(vfs) = Vfs::mount_install(root, Some("STA2 Classic Mod")) else { return };
    let (index, _) = OdfIndex::build(&vfs);
    let Ok((db, _)) = ContentDb::load(&vfs, &index) else { return };
    let close = |f: sta_core::Fixed, v: f32| (f.to_f32() - v).abs() < 1e-4;
    if let Some(w) = db.weapons().find(|w| w.odf.eq_ignore_ascii_case("fgphaso")) {
        let c = w.crew_loss.expect("fgphaso carries crew-loss keys");
        assert!(close(c.threshold, 25.0) && close(c.shield, 0.2) && close(c.hull, 0.1), "{c:?}");
    }
    let sane = |n: &str| db.units().find(|u| u.odf.eq_ignore_ascii_case(n)).map(|u| u.sane_crew_loss);
    assert!(sane("fassault").unwrap_or(true));
    assert!(!sane("fcruise2").unwrap_or(false));
    // SIM-55: the system hit points (engines, weapons, shield generator, life support, sensors).
    if let Some(u) = db.units().find(|u| u.odf.eq_ignore_ascii_case("fassault")) {
        assert_eq!(u.system_hit_points.map(|f| f.to_f32()), [120.0, 80.0, 120.0, 100.0, 80.0]);
    }
}
