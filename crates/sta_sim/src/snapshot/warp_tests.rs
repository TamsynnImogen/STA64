//! LUA-31: the warp speed record of snapshot format 28.

use super::*;
use super::chain;
use crate::WorldSource;
use crate::command::Edit;
use crate::warp_override_tests::{game, script};

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

fn forged(bytes: &[u8], rows: Vec<(EntityId, Fixed)>, version: u8) -> Vec<u8> {
    chain::with(bytes, chain::WARP, chain::enc(&(version, rows)))
}

#[test]
fn a_corrupt_warp_record_is_refused_with_its_reason() {
    let (mut sim, s) = game();
    assert!(script(&mut sim, Edit::WarpSpeed(s, Some(n(30)))));
    let station = sim.spawn_def(sim.content().unwrap().key("fx_station").unwrap(), PlayerId(0), Vec3::ZERO, (Fixed::ZERO, Fixed::ONE)).unwrap();
    let bare = sim.spawn(1, Vec3::ZERO, (Fixed::ZERO, Fixed::ONE), MoveStats::STATIC);
    let bytes = sim.snapshot();
    let content = sim.content().cloned();
    let why = |rows: Vec<(EntityId, Fixed)>, version: u8| chain::corrupt(&forged(&bytes, rows, version), content.clone());
    assert_eq!(why(vec![(EntityId(999), n(30))], 1), "warp speed of an unknown entity");
    assert_eq!(why(vec![(s, n(-1))], 1), "warp speed of a non-craft or out of range");
    assert_eq!(why(vec![(s, crate::warp_override::MAX_WARP + n(1))], 1), "warp speed of a non-craft or out of range");
    assert_eq!(why(vec![(station, n(30))], 1), "warp speed of a non-craft or out of range");
    assert_eq!(why(vec![(s, n(30)), (s, n(31))], 1), "warp speed rows out of order");
    assert_eq!(why(vec![(s, n(30))], 2), "unknown warp speed record version");
    // Not what the writer produces: a row equal to the definition's value, or for a craft with none.
    assert_eq!(why(vec![(s, n(100))], 1), "warp speed row without a definition or equal to it");
    assert_eq!(why(vec![(bare, n(30))], 1), "warp speed row without a definition or equal to it");
    assert!(Sim::from_snapshot(&forged(&bytes, vec![(s, n(30))], 1), content).is_ok());
}

#[test]
fn save_load_save_is_byte_identical_and_the_old_format_and_damage_are_refused() {
    let (mut sim, s) = game();
    assert!(script(&mut sim, Edit::WarpSpeed(s, Some(n(30)))));
    let bytes = sim.snapshot();
    let content = sim.content().cloned();
    let loaded = Sim::from_snapshot(&bytes, content.clone()).unwrap();
    assert_eq!(loaded.snapshot(), bytes);
    assert_eq!(loaded.view(), sim.view());
    // The record is the last one; without it the file is cut short, and format 27 is refused by version.
    let mut cut = chain::before(&bytes, chain::WARP);
    assert!(Sim::from_snapshot(&cut, content.clone()).is_err());
    cut[4..6].copy_from_slice(&27u16.to_le_bytes());
    assert_eq!(Sim::from_snapshot(&cut, content.clone()).err(), Some(SnapshotError::FormatVersion { found: 27 }));
    let mut trailing = bytes;
    trailing.push(0);
    assert!(chain::corrupt(&trailing, content).contains("trailing"));
}
