//! LUA-39: format 33 carries guard rows in the scripted-order record; the previous formats
//! are refused and a forged or unknown-tag row is a load error.

use super::*;
use crate::command::{Command, CommandKind, Order};
use crate::movement::approach_tests;
use sta_core::{Fixed, PlayerId, Vec3};

fn guarded() -> Sim {
    let mut sim = approach_tests::sim(approach_tests::db(sta_content::Profile::Sta2Classic, Some(1100)));
    let db = sim.content.clone().unwrap();
    let at = |x: i32| Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::ZERO };
    let ahead = (Fixed::ZERO, Fixed::ONE);
    let ward = sim.spawn_def(db.key("st_target").unwrap(), PlayerId(1), at(0), ahead).unwrap();
    let guard = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), at(250), ahead).unwrap();
    let cmd = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Guard { units: vec![guard], ward }) };
    crate::WorldSource::step(&mut sim, &[cmd]);
    assert!(matches!(sim.scripted.job(guard), Some(crate::orders::Job::Guard { .. })));
    sim
}

#[test]
fn the_guard_rows_are_in_a_format_from_33_and_the_previous_formats_are_refused() {
    // Format 33 added the guard rows; LUA-41 followed with 34, SIM-46's transport attacks with 35,
    // SIM-40's repair queues with 36, SIM-41's repair-ship beams with 37, SIM-45's decommissions with 38,
    // SIM-75's nebula shields with 39, ReplaceWeapon with 40, shield zero holds with 41,
    // research pod links with 42, freighter docks with 43, and officer upgrades with 44.
    assert_eq!(FORMAT_VERSION, 50);
    let sim = guarded();
    let bytes = sim.snapshot();
    assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), FORMAT_VERSION);
    for old in 31u16..PRE_DIRECTORS_CUT_FORMAT {
        let mut stale = bytes.clone();
        stale[4..6].copy_from_slice(&old.to_le_bytes());
        assert!(matches!(Sim::from_snapshot(&stale, sim.content.clone()), Err(SnapshotError::FormatVersion { found }) if found == old));
    }
}

#[test]
fn a_guard_row_survives_and_an_unknown_tag_or_a_cut_file_is_a_load_error() {
    let sim = guarded();
    let bytes = sim.snapshot();
    let back = Sim::from_snapshot(&bytes, sim.content.clone()).unwrap();
    assert_eq!(back.snapshot(), bytes, "save, load, save");
    // Format 47 is the same record chain before the final Director's Cut field.
    let mut old = bytes[..bytes.len() - 1].to_vec();
    old[4..6].copy_from_slice(&PRE_DIRECTORS_CUT_FORMAT.to_le_bytes());
    let previous = Sim::from_snapshot(&old, sim.content.clone()).unwrap();
    assert!(!previous.directors_cut);
    assert_eq!(previous.snapshot(), bytes);
    // The guard's job tag is the second byte from the end of the scripted record; an unknown tag
    // is what a build without guards would meet.
    let rows = sim.scripted.to_rows();
    let record = chain::enc(&ScriptedRec { version: 1, rows });
    let mut forged = record.clone();
    let tag = forged.len() - 2;
    assert_eq!(forged[tag], 6, "Job::Guard is the seventh variant, tag 6");
    forged[tag] = 7;
    let bad = chain::with(&bytes, chain::SCRIPTED, forged);
    assert!(!chain::corrupt(&bad, sim.content.clone()).is_empty());
    // Truncation anywhere is refused.
    for cut in [bytes.len() - 1, bytes.len() / 2, HEADER + 1] {
        assert!(Sim::from_snapshot(&bytes[..cut], sim.content.clone()).is_err(), "cut at {cut}");
    }
}
