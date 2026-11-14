//! LUA-23: which error a call with several overloads reports when none fits.
//! The best-matching overload's: the one whose failing argument is furthest
//! along, and on a tie the one whose argument has the right type but an
//! unusable value (out of range, a handle still being built) rather than the
//! wrong type.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

fn runtime() -> MissionRuntime {
    let db: Arc<ContentDb> = Arc::new(fake::db());
    let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let out = sim.step(&[Command {
        player: PlayerId::SCRIPT,
        seq: 1,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: db.key("fx_cruiser").expect("fake odf"),
            player: PlayerId(0),
            pos: Vec3::from_f32([0.0, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    }]);
    let id = out.view.units[0].id;
    // A script flags the craft it orders (LUA-28): the scene's craft starts flagged.
    let view = crate::api::special_forces::flagged_view(&out.view, &db);
    let cfg = MissionConfig { content: Some(db), view: Some(view), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("overload.lua", "", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.eval::<()>(&format!("a = {}; A1.paths.P = {{ {{x=10,y=0,z=0}} }}", id.0)).unwrap();
    rt.take_commands();
    rt
}

fn error(script: &str) -> String {
    let rt = runtime();
    rt.eval::<()>(script).expect_err(script).to_string()
}

#[test]
fn an_unusable_handle_is_reported_as_that_not_as_a_wrong_string() {
    // `Goto(handle, string, int)` and `Goto(handle, handle, int)`: argument 2 is a
    // handle, so the second overload fits by type and fails on the value.
    let message = error("A1.Goto(a, 1073741999, 1)");
    assert!(message.contains("A1.Goto argument 2") && message.contains("not a known pending object"), "{message}");
    assert!(!message.contains("must be a string"), "{message}");
}

#[test]
fn nil_where_two_overloads_differ_names_both_kinds() {
    // The shape of a call whose earlier lookup raised and left nil behind.
    let message = error("A1.Goto(a, nil, 1)");
    assert!(message.contains("A1.Goto argument 2 must be a string or an unsigned integer handle"), "{message}");
}

#[test]
fn the_overload_that_gets_furthest_names_the_error() {
    // The handle overload accepts arguments 1 and 2 and fails on 3; the string
    // overload fails on 2.
    let message = error("A1.Goto(a, a, 'x')");
    assert!(message.contains("A1.Goto argument 3") && message.contains("integer"), "{message}");
    assert!(!message.contains("must be a string"), "{message}");
}

#[test]
fn a_right_type_with_an_unusable_value_keeps_its_own_error() {
    // 2^40 is an integer, not a handle or string; the handle overload's error is its own.
    let message = error("A1.Goto(a, 1099511627776, 1)");
    assert!(message.contains("A1.Goto argument 2") && !message.contains(" or "), "{message}");
    assert!(message.contains("unsigned integer handle"), "{message}");
    assert!(!message.contains("must be a string"), "{message}");
}

#[test]
fn a_single_kind_error_stays_as_it_was() {
    let message = error("A1.Stop(nil, 1)");
    assert!(message.contains("A1.Stop argument 1 must be an unsigned integer handle"), "{message}");
}

#[test]
fn a_matching_overload_is_still_taken() {
    let mut rt = runtime();
    rt.eval::<()>("A1.Goto(a, 'P', 1)").unwrap();
    assert_eq!(rt.take_commands().len(), 1);
}
