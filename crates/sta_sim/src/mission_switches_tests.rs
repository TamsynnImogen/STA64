//! LUA-41: the mission switches. The warp rows run on the stock-shaped
//! destroyer of the SIM-36 approach tests (impulse 140, warp 400, warp
//! distance 1100); numbers are authored, not any mission's.

use sta_content::Profile;

use crate::command::{Command, CommandKind, Edit, MissionSwitch, Order};
use crate::mission_switches::MissionSwitches;
use crate::movement::approach_tests;
use crate::movement::approach;
use crate::{EntityId, Fixed, PlayerId, Sim, SimEvent, Vec3, WorldSource};

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}
fn at(x: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: Fixed::ZERO, z: n(z) }
}
const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);
const HZ: u32 = sta_core::TICK_HZ;

pub(crate) fn script(sim: &mut Sim, which: MissionSwitch, on: bool) -> bool {
    let c = Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(Edit::MissionSwitch(which, on)) };
    !sim.step(&[c]).events.iter().any(|e| matches!(e, SimEvent::Rejected { .. }))
}

fn order(sim: &mut Sim, o: Order) {
    sim.step(&[Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(o) }]);
}

/// A seer at the origin and a target `dist` ahead (not yet ordered).
fn stage(dist: i32) -> (Sim, EntityId, EntityId) {
    let mut sim = approach_tests::sim(approach_tests::db(Profile::Sta2Classic, Some(1100)));
    let db = std::sync::Arc::clone(sim.content.as_ref().unwrap());
    let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), at(0, 0), AHEAD).unwrap();
    let t = sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), at(0, dist), AHEAD).unwrap();
    (sim, s, t)
}

fn top_speed(sim: &mut Sim, id: EntityId, secs: u32) -> Fixed {
    let mut top = Fixed::ZERO;
    for _ in 0..secs * HZ {
        sim.step(&[]);
        top = top.max(sim.unit(id).unwrap().speed);
    }
    top
}

fn phase(sim: &Sim, id: EntityId) -> Option<u8> {
    sim.approach_warp.phase(id)
}

#[test]
fn the_default_allows_warp_and_a_script_edit_is_the_only_writer() {
    let (mut sim, _, _) = stage(3000);
    assert_eq!(sim.mission_switches(), MissionSwitches::default());
    assert!(sim.mission_switches().allow_warp && sim.mission_switches().collision_prevention);
    // A player's own command is refused like every edit.
    let c = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Edit(Edit::MissionSwitch(MissionSwitch::AllowWarp, false)) };
    assert!(sim.step(&[c]).events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert!(sim.mission_switches().allow_warp);
    assert!(script(&mut sim, MissionSwitch::AllowWarp, false));
    assert!(!sim.mission_switches().allow_warp);
    assert!(script(&mut sim, MissionSwitch::PathPlanning, false));
    assert_eq!(sim.mission_switches().path_planning, Some(false));
    assert_eq!(sim.view().attrs.mission_switches, sim.mission_switches());
}

#[test]
fn with_allow_warp_off_the_attack_approach_never_warps() {
    let (mut on, s, t) = stage(3000);
    order(&mut on, Order::Attack { units: vec![s], target: t });
    assert!(top_speed(&mut on, s, 30) > n(390), "control: the approach warps");
    let (mut off, s, t) = stage(3000);
    assert!(script(&mut off, MissionSwitch::AllowWarp, false));
    order(&mut off, Order::Attack { units: vec![s], target: t });
    assert!(top_speed(&mut off, s, 30) <= n(141), "warped with AllowWarp off");
    assert_eq!(phase(&off, s), None);
}

#[test]
fn with_allow_warp_off_the_warp_move_never_warps() {
    let go = |off: bool| {
        let (mut sim, s, _) = stage(100);
        if off {
            assert!(script(&mut sim, MissionSwitch::AllowWarp, false));
        }
        order(&mut sim, Order::MoveWarp { units: vec![s], to: at(0, 6000), queue: false });
        top_speed(&mut sim, s, 20)
    };
    assert!(go(false) > n(300), "control: the warp move warps");
    assert!(go(true) <= n(141), "warped with AllowWarp off");
}

#[test]
fn a_craft_in_warp_warps_out_the_next_tick_when_the_switch_goes_off() {
    // The attack approach.
    let (mut sim, s, t) = stage(6000);
    order(&mut sim, Order::Attack { units: vec![s], target: t });
    for _ in 0..40 * HZ {
        sim.step(&[]);
        if phase(&sim, s) == Some(approach::IN_WARP) {
            break;
        }
    }
    assert_eq!(phase(&sim, s), Some(approach::IN_WARP), "control: in warp");
    assert!(script(&mut sim, MissionSwitch::AllowWarp, false));
    assert_eq!(phase(&sim, s), Some(approach::WARP_OUT), "dropping out on the tick after the edit");
    let after = top_speed(&mut sim, s, 20);
    assert!(sim.unit(s).unwrap().speed <= n(141), "back to impulse: {after:?}");
    // The warp move.
    let (mut sim, s, _) = stage(100);
    order(&mut sim, Order::MoveWarp { units: vec![s], to: at(0, 9000), queue: false });
    for _ in 0..30 * HZ {
        sim.step(&[]);
        if phase(&sim, s) == Some(approach::IN_WARP) {
            break;
        }
    }
    assert_eq!(phase(&sim, s), Some(approach::IN_WARP), "control: the warp move is in warp");
    assert!(script(&mut sim, MissionSwitch::AllowWarp, false));
    assert_eq!(phase(&sim, s), Some(approach::WARP_OUT));
    top_speed(&mut sim, s, 20);
    assert!(sim.unit(s).unwrap().speed <= n(141));
}

#[test]
fn turning_it_back_on_lets_the_next_leg_warp() {
    let (mut sim, s, _) = stage(100);
    assert!(script(&mut sim, MissionSwitch::AllowWarp, false));
    assert!(script(&mut sim, MissionSwitch::AllowWarp, true));
    order(&mut sim, Order::MoveWarp { units: vec![s], to: at(0, 6000), queue: false });
    assert!(top_speed(&mut sim, s, 20) > n(300));
}

#[test]
fn default_values_hash_as_before_and_non_default_values_are_in_the_digest() {
    let (mut a, _, _) = stage(3000);
    let (mut b, _, _) = stage(3000);
    // Writing the defaults is no state change: the digests agree tick for tick.
    assert!(script(&mut a, MissionSwitch::AllowWarp, true));
    assert!(script(&mut a, MissionSwitch::CollisionPrevention, true));
    b.step(&[]);
    b.step(&[]);
    assert_eq!(a.digest(), b.digest());
    assert!(script(&mut a, MissionSwitch::AllowWarp, false));
    b.step(&[]);
    assert_ne!(a.digest(), b.digest());
    assert!(script(&mut a, MissionSwitch::AllowWarp, true));
    b.step(&[]);
    assert_eq!(a.digest(), b.digest(), "back to the default: nothing hashed");
}

/// Whole-state digest of a sim with every switch off, pinned (LUA-41).
#[test]
fn a_sim_with_the_switches_off_has_a_pinned_digest() {
    let (mut sim, _, _) = stage(3000);
    for w in [MissionSwitch::AllowWarp, MissionSwitch::CollisionPrevention, MissionSwitch::ObstacleAvoidance] {
        assert!(script(&mut sim, w, false));
    }
    assert!(script(&mut sim, MissionSwitch::PathPlanning, true));
    for _ in 0..10 {
        sim.step(&[]);
    }
    assert_eq!(sim.digest(), PINNED_OFF, "digest {:#x}", sim.digest());
    let s = sim.digest();
    let back = Sim::from_snapshot(&sim.snapshot(), sim.content().cloned()).unwrap();
    assert_eq!(back.digest(), s);
}

const PINNED_OFF: u64 = 0x53c9_321b_40b0_0338;
