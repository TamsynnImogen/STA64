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

#[test]
fn malformed_team_adjustment_records_are_rejected_for_the_right_reason() {
    let (sim, _, db) = crate::combat::subsystem_tests::world();
    let bytes = sim.snapshot();
    let load = |rows: Vec<(PlayerId, i64, i64)>, version: u8| {
        chain::corrupt(&chain::with(&bytes, chain::ADJUST, chain::enc(&AdjustRec { version, rows })), Some(db.clone()))
    };
    let one = Fixed::ONE.to_bits();
    let ok = chain::with(&bytes, chain::ADJUST, chain::enc(&AdjustRec { version: 1, rows: vec![(PlayerId(0), one, 0), (PlayerId(1), 0, -one)] }));
    assert!(Sim::from_snapshot(&ok, Some(db.clone())).is_ok());
    assert_eq!(load(vec![], 2), "unknown team adjustment record version");
    assert_eq!(load(vec![(PlayerId(1), one, 0), (PlayerId(0), one, 0)], 1), "invalid team adjustment rows");
    assert_eq!(load(vec![(PlayerId(0), 0, 0)], 1), "invalid team adjustment rows");
    assert_eq!(load(vec![(PlayerId(9), one, 0)], 1), "invalid team adjustment rows");
    assert_eq!(load(vec![(PlayerId(0), i64::MIN, 0)], 1), "invalid team adjustment rows");
}
