//! LUA-41: the mission switches record of snapshot format 33.

use super::*;
use super::chain;
use crate::command::MissionSwitch;
use crate::mission_switches::{MissionSwitches, SwitchesRec};
use crate::mission_switches_tests::script;

fn switched() -> Sim {
    let mut sim = Sim::new(9);
    for w in [MissionSwitch::AllowWarp, MissionSwitch::CollisionPrevention, MissionSwitch::ObstacleAvoidance] {
        assert!(script(&mut sim, w, false));
    }
    assert!(script(&mut sim, MissionSwitch::PathPlanning, true));
    sim
}

#[test]
fn the_switches_survive_save_load_save_byte_for_byte() {
    let sim = switched();
    let bytes = sim.snapshot();
    let loaded = Sim::from_snapshot(&bytes, None).unwrap();
    assert_eq!(loaded.snapshot(), bytes);
    assert_eq!(loaded.digest(), sim.digest());
    let kept = loaded.mission_switches();
    assert!(!kept.allow_warp && !kept.collision_prevention && !kept.obstacle_avoidance);
    assert_eq!(kept.path_planning, Some(true));
    // The default is saved too (the record is mandatory) and reads back as the default.
    let plain = Sim::from_snapshot(&Sim::new(9).snapshot(), None).unwrap();
    assert!(plain.mission_switches().is_default());
}

#[test]
fn the_record_is_mandatory_and_schema_checked() {
    let bytes = switched().snapshot();
    let unknown = SwitchesRec::for_test(2, MissionSwitches::default());
    assert_eq!(chain::corrupt(&chain::with(&bytes, chain::MISSION_SWITCHES, chain::enc(&unknown)), None), "unknown mission switches record version");
    // A path override byte outside None/Some(false)/Some(true) does not decode.
    let mut forged = chain::split(&bytes);
    let last = forged[chain::MISSION_SWITCHES].len() - 1;
    forged[chain::MISSION_SWITCHES][last - 1] = 1;
    forged[chain::MISSION_SWITCHES][last] = 2;
    assert!(matches!(Sim::from_snapshot(&chain::join(&forged), None), Err(SnapshotError::Corrupt(_))));
    let missing = chain::before(&bytes, chain::MISSION_SWITCHES);
    assert!(matches!(Sim::from_snapshot(&missing, None), Err(SnapshotError::Corrupt(_))));
    let mut trailing = bytes;
    trailing.push(0);
    assert!(chain::corrupt(&trailing, None).contains("trailing"));
}

#[test]
fn the_previous_formats_are_refused() {
    let mut bytes = Sim::new(9).snapshot();
    for found in [31u16, 32] {
        bytes[4..6].copy_from_slice(&found.to_le_bytes());
        assert_eq!(Sim::from_snapshot(&bytes, None).err(), Some(SnapshotError::FormatVersion { found }));
    }
}
