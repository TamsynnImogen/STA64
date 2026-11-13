//! SIM-56 / SIM-58 snapshot records (format 21): load validation at the bytes level.

use super::*;
use crate::orders::Job;
use crate::switches::Switches;

fn world() -> (Sim, EntityId, Arc<ContentDb>) {
    let mut sim = crate::movement::approach_tests::sim(crate::movement::approach_tests::db(sta_content::Profile::Sta2Classic, Some(1100)));
    let db = Arc::clone(sim.content.as_ref().unwrap());
    let id = sim.spawn_def(db.key("st_destroy").unwrap(), PlayerId(1), Vec3::ZERO, (Fixed::ZERO, Fixed::ONE)).unwrap();
    (sim, id, db)
}

#[test]
fn malformed_scripted_order_records_are_rejected_for_the_right_reason() {
    let (sim, id, db) = world();
    let bytes = sim.snapshot();
    let rec = |rows| chain::enc(&ScriptedRec { version: 1, rows });
    let load = |r: Vec<u8>| chain::corrupt(&chain::with(&bytes, chain::SCRIPTED, r), Some(db.clone()));
    let ok = chain::with(&bytes, chain::SCRIPTED, rec(vec![(id, Job::SearchAndDestroy)]));
    assert!(Sim::from_snapshot(&ok, Some(db.clone())).is_ok());
    assert_eq!(load(rec(vec![(id, Job::AttackAny), (id, Job::Warp)])), "scripted order rows out of order");
    assert_eq!(load(rec(vec![(EntityId(999), Job::Warp)])), "scripted order entity is not a craft or a patrol has no loop");
    assert_eq!(load(rec(vec![(id, Job::Patrol { points: vec![Vec3::ZERO], leg: 0 })])), "scripted order entity is not a craft or a patrol has no loop");
    assert_eq!(load(chain::enc(&ScriptedRec { version: 2, rows: Vec::new() })), "unknown scripted orders record version");
}

#[test]
fn malformed_switch_records_are_rejected_for_the_right_reason() {
    let (sim, id, db) = world();
    let bytes = sim.snapshot();
    let rec = |rows| chain::enc(&SwitchesRec { version: 1, rows });
    let load = |r: Vec<u8>| chain::corrupt(&chain::with(&bytes, chain::SWITCHES, r), Some(db.clone()));
    let on = Switches { hidden: true, ..Switches::default() };
    let ok = chain::with(&bytes, chain::SWITCHES, rec(vec![(id, on.clone())]));
    assert!(Sim::from_snapshot(&ok, Some(db.clone())).is_ok());
    assert_eq!(load(rec(vec![(id, Switches::default())])), "switch rows out of order");
    assert_eq!(load(rec(vec![(id, Switches { name: Some("x".repeat(65)), ..on.clone() })])), "switch rows out of order");
    assert_eq!(load(rec(vec![(EntityId(999), on)])), "switches of an unknown entity");
    assert_eq!(load(chain::enc(&SwitchesRec { version: 2, rows: Vec::new() })), "unknown switches record version");
}
