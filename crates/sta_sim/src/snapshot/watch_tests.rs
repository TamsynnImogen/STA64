//! The stuck-order watch rows' load validation, at the bytes level.

use super::*;

fn world() -> (Sim, EntityId, Arc<ContentDb>) {
    let mut sim = crate::movement::approach_tests::sim(crate::movement::approach_tests::db(sta_content::Profile::Sta2Classic, Some(1100)));
    let db = Arc::clone(sim.content.as_ref().unwrap());
    let id = sim.spawn_def(db.key("st_destroy").unwrap(), PlayerId(1), Vec3::ZERO, (Fixed::ZERO, Fixed::ONE)).unwrap();
    (sim, id, db)
}

fn row() -> crate::watchdog::Row {
    crate::watchdog::Row { target: Vec3::ZERO, best: Fixed::ZERO, since: 0, reported: false, anchor: Vec3::ZERO, anchored: 0, start: Fixed::ZERO, born: 0 }
}

#[test]
fn malformed_watch_records_are_rejected_for_the_right_reason() {
    let (sim, id, db) = world();
    let bytes = sim.snapshot();
    let load = |rec: Vec<u8>| chain::corrupt(&chain::with(&bytes, chain::WATCH, rec), Some(db.clone()));
    let ok = chain::with(&bytes, chain::WATCH, chain::enc(&WatchRec { version: 1, rows: vec![(id, row())] }));
    assert!(Sim::from_snapshot(&ok, Some(db.clone())).is_ok());
    assert_eq!(load(chain::enc(&WatchRec { version: 1, rows: vec![(id, row()), (id, row())] })), "order watch rows out of order");
    assert_eq!(load(chain::enc(&WatchRec { version: 1, rows: vec![(EntityId(999), row())] })), "order watch entity is not a craft");
    let future = crate::watchdog::Row { since: u64::MAX, ..row() };
    assert_eq!(load(chain::enc(&WatchRec { version: 1, rows: vec![(id, future)] })), "order watch row from the future");
    assert_eq!(load(chain::enc(&WatchRec { version: 2, rows: Vec::new() })), "unknown order watch record version");
    let trailing = [chain::split(&bytes).concat(), vec![0]].concat();
    assert_eq!(chain::corrupt(&trailing, Some(db.clone())), "trailing bytes");
}
