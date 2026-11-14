//! `resource_calls` (LUA-22): each call changes the right seat's stock through `Edit::Cargo`,
//! seen on a real sim after the queued edits are stepped in. Asset-free.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{Fixed, PlayerId, TeamId};
use sta_sim::command::{Command, CommandKind};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::WorldView;
use sta_sim::{Sim, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

fn world() -> (Sim, WorldView, Arc<ContentDb>) {
    let db = Arc::new(fake::db());
    let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Idle };
    let setup = GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let view = (*sim.step(&[]).view).clone();
    (sim, view, db)
}

fn runtime(view: &WorldView, db: &Arc<ContentDb>) -> MissionRuntime {
    let cfg = MissionConfig { content: Some(db.clone()), view: Some(view.clone()), ..MissionConfig::default() };
    let rt = MissionRuntime::from_source_with("res.lua", "", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

/// Run `snippet` on the current view, step its commands into the sim, return the new view.
fn run(sim: &mut Sim, view: &mut WorldView, db: &Arc<ContentDb>, snippet: &str) -> MissionRuntime {
    let mut rt = runtime(view, db);
    rt.eval::<()>(snippet).unwrap_or_else(|e| panic!("{snippet}: {e}"));
    let commands: Vec<Command> = rt.take_commands();
    assert!(commands.iter().all(|c| matches!(c.kind, CommandKind::Edit(_))));
    *view = (*sim.step(&commands).view).clone();
    rt
}

fn stock(view: &WorldView, player: u8) -> [i64; 4] {
    let p = view.player(PlayerId(player)).unwrap();
    std::array::from_fn(|i| p.resources.get(i).copied().unwrap_or(Fixed::ZERO).to_f32().round() as i64)
}

#[test]
fn each_call_changes_the_named_seats_stock() {
    let (mut sim, mut view, db) = world();
    let rt = run(
        &mut sim,
        &mut view,
        &db,
        "A2.SetDilithium(0, 500); A2.SetLatinum(1, 70); A2.SetMetal(0, 300); A2.SetBiomatter(1, 9)",
    );
    assert_eq!(stock(&view, 0)[0], 500);
    assert_eq!(stock(&view, 0)[2], 300);
    assert_eq!(stock(&view, 1)[1], 70);
    assert_eq!(stock(&view, 1)[3], 9);
    assert_eq!([stock(&view, 0)[1], stock(&view, 0)[3], stock(&view, 1)[0], stock(&view, 1)[2]], [0; 4], "other slots and seats unchanged");
    assert!(rt.unsupported().is_empty(), "no longer counted as gaps: {:?}", rt.unsupported());

    run(&mut sim, &mut view, &db, "A2.AddDilithium(0, 25); A2.AddLatinum(1, 5); A2.AddMetal(0, -100); A2.AddBiomatter(1, 1)");
    assert_eq!(stock(&view, 0)[0], 525);
    assert_eq!(stock(&view, 1)[1], 75);
    assert_eq!(stock(&view, 0)[2], 200);
    assert_eq!(stock(&view, 1)[3], 10);
}

#[test]
fn adds_in_one_tick_accumulate_and_the_floor_is_zero() {
    let (mut sim, mut view, db) = world();
    run(&mut sim, &mut view, &db, "A2.SetMetal(1, 100); A2.AddMetal(1, 30); A2.AddMetal(1, 20)");
    assert_eq!(stock(&view, 1)[2], 150);
    run(&mut sim, &mut view, &db, "A2.AddMetal(1, -1000)");
    assert_eq!(stock(&view, 1)[2], 0, "a negative add stops at 0");
    run(&mut sim, &mut view, &db, "A2.SetDilithium(0, -5)");
    assert_eq!(stock(&view, 0)[0], 0, "a negative set stores 0");
}

#[test]
fn arguments_are_checked_and_an_unseated_team_raises() {
    let (_sim, view, db) = world();
    let mut rt = runtime(&view, &db);
    for bad in [
        "A2.SetMetal(0)",
        "A2.SetMetal(0, 1, 2)",
        "A2.AddMetal(0, 1.5)",
        "A2.AddMetal(0, 'x')",
        "A2.AddMetal(0, 3000000000)",
        "A2.AddMetal(-1, 5)",
        "A2.AddMetal(7, 5)",
        "A2.SetLatinum(300, 5)",
    ] {
        assert!(rt.eval::<()>(bad).is_err(), "{bad}");
    }
    assert!(rt.take_commands().is_empty(), "a rejected call sends nothing");
}
