//! SIM-36: the attack approach record's load validation, at the bytes level.

use super::*;

fn world() -> (Sim, EntityId, Arc<ContentDb>) {
    let mut sim = crate::movement::approach_tests::sim(crate::movement::approach_tests::db(sta_content::Profile::Sta2Classic, Some(1100)));
    let db = Arc::clone(sim.content.as_ref().unwrap());
    let id = sim.spawn_def(db.key("st_destroy").unwrap(), PlayerId(1), Vec3::ZERO, (Fixed::ZERO, Fixed::ONE)).unwrap();
    (sim, id, db)
}

fn record(warp: Vec<(EntityId, u8)>, engagement: crate::combat::targeting::EngagementRec) -> Vec<u8> {
    chain::enc(&AttackRec { version: 1, warp, engagement })
}

#[test]
fn malformed_attack_records_are_rejected_for_the_right_reason() {
    let (sim, id, db) = world();
    let bytes = sim.snapshot();
    let with = |rec: Vec<u8>| chain::with(&bytes, chain::ATTACK, rec);
    let load = |rec: Vec<u8>| chain::corrupt(&with(rec), Some(db.clone()));
    assert!(Sim::from_snapshot(&with(record(vec![(id, 1)], (vec![(id, (0, id))], vec![(id, 0)]))), Some(db.clone())).is_ok());
    assert_eq!(load(record(vec![(id, 3)], Default::default())), "attack warp rows out of order or of an unknown phase");
    assert_eq!(load(record(vec![(EntityId(999), 1)], Default::default())), "attack warp entity is not a craft");
    assert_eq!(load(record(Vec::new(), (vec![(id, (0, id)), (id, (0, id))], Vec::new()))), "engagement rows out of order, from the future or of an unknown entity");
    assert_eq!(load(record(Vec::new(), (vec![(id, (u64::MAX, id))], Vec::new()))), "engagement rows out of order, from the future or of an unknown entity");
    assert_eq!(load(record(Vec::new(), (Vec::new(), vec![(EntityId(999), 0)]))), "engagement rows out of order, from the future or of an unknown entity");
    assert_eq!(load(chain::enc(&AttackRec { version: 2, warp: Vec::new(), engagement: Default::default() })), "unknown attack approach record version");
}
