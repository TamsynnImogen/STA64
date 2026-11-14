//! LUA-31: the warp speed record of snapshot format 28.

use super::*;
use super::chain;
use crate::command::Edit;
use crate::warp_override_tests::{game, script};

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

#[test]
fn a_corrupt_warp_record_is_refused() {
    let (mut sim, s) = game();
    assert!(script(&mut sim, Edit::WarpSpeed(s, Some(n(30)))));
    let bytes = sim.snapshot();
    let content = sim.content().cloned();
    let rec = |rows: Vec<(EntityId, Fixed)>, version: u8| chain::enc(&(version, rows));
    for (rows, version) in [
        (vec![(EntityId(999), n(30))], 1),
        (vec![(s, n(-1))], 1),
        (vec![(s, crate::warp_override::MAX_WARP + n(1))], 1),
        (vec![(s, n(30)), (s, n(31))], 1),
        (vec![(s, n(30))], 2),
    ] {
        let broken = chain::with(&bytes, chain::WARP, rec(rows, version));
        assert!(matches!(Sim::from_snapshot(&broken, content.clone()), Err(crate::snapshot::SnapshotError::Corrupt(_))));
    }
    assert!(Sim::from_snapshot(&chain::with(&bytes, chain::WARP, rec(vec![(s, n(30))], 1)), content).is_ok());
}

