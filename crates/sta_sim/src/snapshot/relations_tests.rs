//! SIM-63: the mission-set relation rows' load validation, at the bytes level.

use super::*;

fn rec(version: u8, rows: Vec<(u8, u8, u8)>) -> Vec<u8> {
    chain::enc(&RelationsRec { version, rows })
}

#[test]
fn malformed_relation_records_are_rejected_for_the_right_reason() {
    let (sim, _, db) = crate::combat::subsystem_tests::world();
    let bytes = sim.snapshot();
    let load = |r: Vec<u8>| chain::corrupt(&chain::with(&bytes, chain::RELATIONS, r), Some(db.clone()));
    // The fixture's seats 0 and 1 are on different teams, so Neutral is an override.
    let ok = chain::with(&bytes, chain::RELATIONS, rec(1, vec![(0, 1, 1)]));
    assert!(Sim::from_snapshot(&ok, Some(db.clone())).is_ok());
    assert_eq!(load(rec(2, vec![])), "unknown relations record version");
    assert_eq!(load(rec(1, vec![(0, 1, 3)])), "relation code unknown");
    assert_eq!(load(rec(1, vec![(0, 0, 1)])), "relation between seats that do not exist or a seat and itself");
    assert_eq!(load(rec(1, vec![(0, 9, 1)])), "relation between seats that do not exist or a seat and itself");
    assert_eq!(load(rec(1, vec![(0, 1, 0)])), "relation row equals the default");
}
