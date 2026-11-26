//! LUA-32: the alert record of snapshot format 32.

use super::*;
use super::chain;
use crate::WorldSource;
use crate::alert::AlertRec;
use crate::alert_tests::{game, put};
use crate::command::{AlertState, Command, CommandKind, Edit, Order};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn busy() -> (Sim, EntityId, EntityId, EntityId) {
    let mut sim = game(&[(1, 1), (2, 2), (3, 1)]);
    let a = put(&mut sim, "st_seer", 1, v(0, 0));
    let b = put(&mut sim, "st_seer", 1, v(0, 60));
    let friend = put(&mut sim, "st_target", 3, v(0, 300));
    let script = |sim: &mut Sim, e| sim.step(&[Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(e) }]);
    script(&mut sim, Edit::Alert(a, AlertState::Green));
    script(&mut sim, Edit::Alert(b, AlertState::Yellow));
    script(&mut sim, Edit::Decommissionable(b, false));
    sim.alerts.stamp(b, 1);
    WorldSource::step(&mut sim, &[Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![a], target: friend }) }]);
    (sim, a, b, friend)
}

fn rec(bytes: &[u8]) -> AlertRec {
    let (r, _): (AlertRec, _) = postcard::take_from_bytes(&chain::split(bytes)[chain::ALERTS]).unwrap();
    r
}

fn forged(bytes: &[u8], edit: impl FnOnce(&mut AlertRec)) -> Vec<u8> {
    let mut r = rec(bytes);
    edit(&mut r);
    chain::with(bytes, chain::ALERTS, chain::enc(&r))
}

#[test]
fn save_load_save_is_byte_identical_and_the_old_format_and_damage_are_refused() {
    let (sim, a, b, _) = busy();
    let bytes = sim.snapshot();
    let content = sim.content().cloned();
    let loaded = Sim::from_snapshot(&bytes, content.clone()).unwrap();
    assert_eq!(loaded.snapshot(), bytes);
    assert_eq!(loaded.view(), sim.view());
    assert_eq!(loaded.digest(), sim.digest());
    assert_eq!((loaded.alert_of(a), loaded.alert_of(b)), (AlertState::Green, AlertState::Yellow));
    assert!(loaded.switches(b).unwrap().no_decommission);
    let mut cut = chain::before(&bytes, chain::ALERTS);
    assert!(Sim::from_snapshot(&cut, content.clone()).is_err(), "the record is missing");
    let previous = PRE_DIRECTORS_CUT_FORMAT - 1;
    cut[4..6].copy_from_slice(&previous.to_le_bytes());
    assert_eq!(Sim::from_snapshot(&cut, content.clone()).err(), Some(SnapshotError::FormatVersion { found: previous }));
    let mut short = bytes.clone();
    short.pop();
    assert!(Sim::from_snapshot(&short, content.clone()).is_err(), "truncated");
    let mut trailing = bytes;
    trailing.push(0);
    assert!(chain::corrupt(&trailing, content).contains("trailing"));
}

#[test]
fn a_default_world_saves_an_empty_alert_record_and_a_stamp_is_always_saved() {
    let mut sim = game(&[(1, 1), (2, 2)]);
    let a = put(&mut sim, "st_seer", 1, v(0, 0));
    sim.alerts.stamp(a, 0);
    let bytes = sim.snapshot();
    let r = rec(&bytes);
    assert!(r.level.is_empty() && r.ally.is_empty() && r.enemy_hit == vec![(a, 0)]);
    assert_eq!(Sim::from_snapshot(&bytes, sim.content().cloned()).unwrap().snapshot(), bytes);
}

#[test]
fn a_noncanonical_alert_record_is_refused_with_its_reason() {
    let (sim, a, b, friend) = busy();
    let bytes = sim.snapshot();
    let content = sim.content().cloned();
    let why = |edit: Box<dyn FnOnce(&mut AlertRec)>| chain::corrupt(&forged(&bytes, edit), content.clone());
    assert_eq!(why(Box::new(|r| r.version = 2)), "unknown alert record version");
    assert_eq!(why(Box::new(|r| r.level = vec![(a, 0)])), "alert row of a unit that cannot hold one, or at red");
    assert_eq!(why(Box::new(|r| r.level = vec![(a, 3)])), "alert row of a unit that cannot hold one, or at red");
    assert_eq!(why(Box::new(|r| r.level = vec![(EntityId(99), 2)])), "alert row of a unit that cannot hold one, or at red");
    assert_eq!(why(Box::new(move |r| r.level = vec![(b, 2), (a, 2)])), "alert rows out of order");
    assert_eq!(why(Box::new(|r| r.enemy_hit = vec![(EntityId(99), 1)])), "enemy-hit row of an unknown unit or from the future");
    assert_eq!(why(Box::new(move |r| r.enemy_hit = vec![(b, 1 << 40)])), "enemy-hit row of an unknown unit or from the future");
    assert_eq!(why(Box::new(move |r| r.enemy_hit = vec![(b, 2), (a, 2)])), "enemy-hit rows out of order");
    assert_eq!(why(Box::new(|r| r.ally = vec![(3, 1, 0, 0)])), "ally attack rows");
    assert_eq!(why(Box::new(|r| r.ally = vec![(3, 1, 1, 0), (3, 1, 2, 0)])), "ally attack rows");
    assert_eq!(why(Box::new(|r| r.ally = vec![(3, 9, 1, 0)])), "ally attack rows");
    assert_eq!(why(Box::new(|r| r.ally_last = vec![])), "ally attack times");
    assert_eq!(why(Box::new(|r| r.ally_last = vec![(1, 1), (1, 2)])), "ally attack times");
    assert_eq!(why(Box::new(|r| r.ally_last = vec![(1, 1 << 40)])), "ally attack times");
    let _ = friend;
    // A craft with a level but a station-less non-craft is the only other kind; the valid record loads.
    assert!(Sim::from_snapshot(&forged(&bytes, |_| {}), content).is_ok());
}
