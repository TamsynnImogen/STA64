//! SIM-54: the mission-set race rows' load validation, at the bytes level.

use super::*;
use crate::attrs::AttrsRec;

fn rec(races: Vec<(EntityId, String)>, player_races: Vec<(PlayerId, String)>) -> Vec<u8> {
    chain::enc(&AttrsRec { version: 1, races, player_races })
}

#[test]
fn malformed_attribute_records_are_rejected_for_the_right_reason() {
    let (sim, id, db) = crate::combat::subsystem_tests::world();
    let bytes = sim.snapshot();
    let load = |r: Vec<u8>| chain::corrupt(&chain::with(&bytes, chain::ATTRS, r), Some(db.clone()));
    let n = |i: u32| (EntityId(i), "x".to_string());
    let ok = chain::with(&bytes, chain::ATTRS, rec(vec![(id, "fx".into())], vec![(PlayerId(1), "fx".into())]));
    assert!(Sim::from_snapshot(&ok, Some(db.clone())).is_ok());
    assert_eq!(load(rec(vec![n(2), n(1)], vec![])), "attribute rows out of order");
    assert_eq!(load(rec(vec![], vec![(PlayerId(1), "a".into()), (PlayerId(0), "a".into())])), "attribute rows out of order");
    assert_eq!(load(rec(vec![n(999)], vec![])), "attribute entity is not a craft or station");
    assert_eq!(load(chain::enc(&AttrsRec { version: 2, races: vec![], player_races: vec![] })), "unknown attribute record version");
    assert_eq!(load(rec(vec![(id, String::new())], vec![])), "attribute text empty or too long");
    assert_eq!(load(rec(vec![], vec![(PlayerId(9), "fx".into())])), "attribute seat does not exist");
    let trailing = [chain::split(&bytes).concat(), vec![0]].concat();
    assert_eq!(chain::corrupt(&trailing, Some(db.clone())), "trailing bytes");
}
