//! SIM-75 slice 2: format 39 appends the nebula shield record; format 38 is refused.

use super::*;
use crate::nebula::tests::{content_in, neb, put, run, sim_with, v};
use crate::nebula::NebulaShieldsRec;

fn mid_delay() -> Sim {
    let mut s = sim_with(vec![neb("fx_still", v(1000, 1000), 300)], content_in(false, None));
    let id = put(&mut s, "fx_cruiser", 1, v(1000, 1000));
    run(&mut s, 3);
    s.units.get_mut(id).unwrap().pos = v(5000, 1000);
    run(&mut s, 10);
    s
}

#[test]
fn format_38_is_refused_and_a_forged_record_is_a_load_error() {
    assert_eq!(FORMAT_VERSION, 39);
    let s = mid_delay();
    let bytes = s.snapshot();
    assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 39);
    let mut old = bytes.clone();
    old[4..6].copy_from_slice(&38u16.to_le_bytes());
    assert!(matches!(Sim::from_snapshot(&old, s.content.clone()), Err(SnapshotError::FormatVersion { found: 38 })));
    let back = Sim::from_snapshot(&bytes, s.content.clone()).unwrap();
    assert_eq!(back.snapshot(), bytes, "save, load, save");
    // An unknown record version, a craft that does not exist, rows out of order, a missing record.
    let rec = |version, id: u32| NebulaShieldsRec { version, rows: vec![(EntityId(id), [0; 4], 0)] };
    let id = s.units.iter().next().unwrap().id.0;
    let at = |pools: [i64; 4], left: u64| NebulaShieldsRec { version: 1, rows: vec![(EntityId(id), pools, left)] };
    let future = s.tick + 2; // stored as left + 1: a leave tick after the snapshot's
    let forged = [rec(2, 1), rec(1, 9999), at([0; 4], future), at([0; 4], u64::MAX), at([-1, 0, 0, 0], 0), at([1_000_000_001i64 << 32, 0, 0, 0], 0)];
    for bad in forged {
        let forged = chain::with(&bytes, chain::NEBULA_SHIELDS, chain::enc(&bad));
        assert!(!chain::corrupt(&forged, s.content.clone()).is_empty());
    }
    let unordered = NebulaShieldsRec { version: 1, rows: vec![(EntityId(2), [0; 4], 0), (EntityId(1), [0; 4], 0)] };
    assert!(!chain::corrupt(&chain::with(&bytes, chain::NEBULA_SHIELDS, chain::enc(&unordered)), s.content.clone()).is_empty());
    assert!(Sim::from_snapshot(&chain::before(&bytes, chain::NEBULA_SHIELDS), s.content.clone()).is_err());
}
