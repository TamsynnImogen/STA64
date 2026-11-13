//! SIM-55: the subsystem record's load validation, at the bytes level.

use super::*;
use crate::combat::damage::SubsystemRow;
use crate::combat::subsystem_tests::world;

fn record(rows: Vec<(EntityId, SubsystemRow)>) -> Vec<u8> {
    chain::enc(&SubsystemsRec { version: 1, rows })
}

#[test]
fn malformed_subsystem_records_are_rejected_for_the_right_reason() {
    let (sim, id, db) = world();
    let bytes = sim.snapshot();
    let load = |rec: Vec<u8>| chain::corrupt(&chain::with(&bytes, chain::SUBSYSTEMS, rec), Some(db.clone()));
    let ok = SubsystemRow { off: 0, damage: [0, Fixed::from_int(10).to_bits(), 0, 0, 0] };
    // The same bytes with a valid record load: each rejection below is the record's.
    assert!(Sim::from_snapshot(&chain::with(&bytes, chain::SUBSYSTEMS, record(vec![(id, ok)])), Some(db.clone())).is_ok());
    assert_eq!(load(record(vec![(EntityId(999), ok)])), "subsystem entity is not a craft or station");
    let over = SubsystemRow { off: 0, damage: [0, Fixed::from_int(81).to_bits(), 0, 0, 0] };
    assert_eq!(load(record(vec![(id, over)])), "subsystem damage above hit points");
    // sensorsHitPoints is 0 in the fixture: any damage there is above it.
    let none = SubsystemRow { off: 0, damage: [0, 0, 0, 0, 1] };
    assert_eq!(load(record(vec![(id, none)])), "subsystem damage above hit points");
    assert_eq!(load(record(vec![(id, SubsystemRow { off: 0b1, damage: [0; 5] })])), "invalid subsystem rows");
    assert_eq!(load(chain::enc(&SubsystemsRec { version: 2, rows: Vec::new() })), "unknown subsystems record version");
    // A non-craft entity: the prop of the same sim.
    let mut sim = sim;
    sim.units.get_mut(id).unwrap().kind = EntityKind::Prop;
    sim.units.get_mut(id).unwrap().crew = None;
    let prop = chain::with(&sim.snapshot(), chain::SUBSYSTEMS, record(vec![(id, ok)]));
    assert_eq!(chain::corrupt(&prop, Some(db)), "subsystem entity is not a craft or station");
}
