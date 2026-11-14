//! LUA-23: `CraftCannotDie` with the null handle. The engine's guard (read from
//! the executable, static) makes a call on a handle that names no object a
//! silent no-op; a converted script reads an array slot it never set as nil,
//! where the engine reads 0. The flag stays a strict boolean.

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;
use sta_core::{Fixed, PlayerId, Vec3};
use sta_sim::command::{CommandKind, Edit};
use sta_sim::{MoveStats, Sim, WorldSource};

fn runtime() -> (MissionRuntime, sta_core::EntityId) {
    let mut sim = Sim::new(7);
    let a = sim.spawn(0, Vec3::from_f32([1.0, 0.0, 0.0]), (Fixed::ZERO, Fixed::ONE), MoveStats::from_f32(20.0, 1.0, 1.0, 1.0, 0.2));
    let cfg = MissionConfig { view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let rt = MissionRuntime::from_source_with("ccd.lua", "", 7, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    (rt, a)
}

#[test]
fn the_null_handle_is_a_no_op_for_either_flag() {
    let (mut rt, _) = runtime();
    for script in [
        "A2.CraftCannotDie(nil, true)",
        "A2.CraftCannotDie(0, true)",
        "A2.CraftCannotDie(0, false)",
        "A2.CraftCannotDie(0.0, true)",
        "local arr = {}; A2.CraftCannotDie(arr[0], true)",
    ] {
        rt.eval::<()>(script).unwrap_or_else(|e| panic!("{script}: {e}"));
        assert!(rt.take_commands().is_empty(), "{script} queued a command");
        assert!(rt.unsupported().is_empty(), "{script} recorded a gap: {:?}", rt.unsupported());
    }
}

#[test]
fn a_live_handle_still_queues_the_flag() {
    let (mut rt, a) = runtime();
    rt.eval::<()>(&format!("A2.CraftCannotDie({}, true)", a.0)).unwrap();
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 1);
    assert_eq!((commands[0].player, &commands[0].kind), (PlayerId::SCRIPT, &CommandKind::Edit(Edit::Immortal(a, true))));
}

#[test]
fn the_flag_and_other_handles_stay_strict() {
    // A unit that was in the world and is gone from the view.
    let mut sim = Sim::new(8);
    let stats = MoveStats::from_f32(20.0, 1.0, 1.0, 1.0, 0.2);
    let kept = sim.spawn(0, Vec3::from_f32([1.0, 0.0, 0.0]), (Fixed::ZERO, Fixed::ONE), stats);
    let gone = sim.spawn(0, Vec3::from_f32([2.0, 0.0, 0.0]), (Fixed::ZERO, Fixed::ONE), stats);
    let mut view = (*sim.view()).clone();
    view.units.retain(|u| u.id == kept);
    let cfg = MissionConfig { view: Some(view), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("ccd.lua", "", 8, cfg);
    rt.eval::<()>(&format!("gone = {}", gone.0)).unwrap();
    let a = kept;
    for script in [
        "A2.CraftCannotDie(nil, 1)",
        "A2.CraftCannotDie(nil, nil)",
        "A2.CraftCannotDie('x', true)",
        "A2.CraftCannotDie(-1, true)",
        "A2.CraftCannotDie(99999, true)",
        "A2.CraftCannotDie(gone, true)",
        "A2.CraftCannotDie(nil)",
    ] {
        assert!(rt.eval::<()>(script).is_err(), "{script}");
        assert!(rt.take_commands().is_empty(), "{script}");
    }
    assert!(rt.eval::<()>(&format!("A2.CraftCannotDie({}, 1)", a.0)).is_err());
}
