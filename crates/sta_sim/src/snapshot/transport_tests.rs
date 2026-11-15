//! SIM-46: the transport record of the snapshot (`combat::boarding::TransportsRec`).

use super::*;
use crate::combat::boarding::Transports;

fn sim() -> Sim {
    let mut sim = Sim::new(3);
    let st = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    sim.spawn(0, Vec3::default(), (Fixed::ZERO, Fixed::ONE), st);
    sim.spawn(1, Vec3::default(), (Fixed::ZERO, Fixed::ONE), st);
    sim
}

#[test]
fn live_transport_state_round_trips_and_a_dangling_record_is_corrupt() {
    let mut live = sim();
    live.transports = Transports::sample_for_test();
    let bytes = live.snapshot();
    let copy = Sim::from_snapshot(&bytes, None).unwrap();
    assert_eq!(copy.digest(), live.digest());
    assert_eq!(copy.snapshot(), bytes);

    // The same record in a world that lacks its units is refused.
    let mut rec = Transports::sample_for_test().to_rec();
    rec.rows[0].1 = sta_core::EntityId(99);
    let bad = chain::with(&bytes, chain::TRANSPORTS, chain::enc(&rec));
    assert_eq!(chain::corrupt(&bad, None), "transport rows out of order");
    // A snapshot cut before the record (format 28 layout) is not read.
    assert!(Sim::from_snapshot(&chain::before(&bytes, chain::TRANSPORTS), None).is_err());
}
