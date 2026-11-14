//! `resource_calls` (LUA-22): each call changes the right seat's stock through `Edit::Cargo`,
//! seen on a real sim after the queued edits are stepped in. Asset-free.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{Fixed, PlayerId, TeamId};
use sta_core::Vec3;
use sta_sim::command::{Command, CommandKind, Edit};
use crate::runtime::MissionEndState;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::WorldView;
use sta_sim::{Sim, WorldSource};

use crate::api::{MissionConfig, PlayerNumbering};
use crate::runtime::MissionRuntime;

fn world() -> (Sim, WorldView, Arc<ContentDb>) {
    world_with(Control::Idle)
}

fn world_with(second: Control) -> (Sim, WorldView, Arc<ContentDb>) {
    let db = Arc::new(fake::db());
    let seat = |n| PlayerSetup {
        id: PlayerId(n),
        team: TeamId(n),
        race: "fx".into(),
        control: if n == 1 { second.clone() } else { Control::Idle },
    };
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
    runtime_numbered(view, db, PlayerNumbering::Slots)
}

fn runtime_numbered(view: &WorldView, db: &Arc<ContentDb>, player_numbering: PlayerNumbering) -> MissionRuntime {
    let cfg = MissionConfig { content: Some(db.clone()), view: Some(view.clone()), player_numbering, ..MissionConfig::default() };
    let rt = MissionRuntime::from_source_with("res.lua", "", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

/// Run `snippet` on the current view, step its commands into the sim, return the new view.
fn run(sim: &mut Sim, view: &mut WorldView, db: &Arc<ContentDb>, snippet: &str) -> MissionRuntime {
    run_numbered(sim, view, db, snippet, PlayerNumbering::Slots)
}

fn run_numbered(sim: &mut Sim, view: &mut WorldView, db: &Arc<ContentDb>, snippet: &str, n: PlayerNumbering) -> MissionRuntime {
    let mut rt = runtime_numbered(view, db, n);
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

#[test]
fn converted_mission_numbering_and_an_ai_seat() {
    let (mut sim, mut view, db) = world_with(Control::Ai("skirmish".into()));
    let mut rt = runtime_numbered(&view, &db, PlayerNumbering::ArmadaTeams);
    assert!(rt.eval::<()>("A2.AddMetal(0, 5)").is_err(), "team 0 is neutral and raises, like AddCrew");
    assert!(rt.take_commands().is_empty());
    run_numbered(&mut sim, &mut view, &db, "A2.SetMetal(1, 40); A2.SetLatinum(2, 60)", PlayerNumbering::ArmadaTeams);
    assert_eq!(stock(&view, 0)[2], 40, "team 1 is slot 0");
    assert_eq!(stock(&view, 1)[1], 60, "team 2 is slot 1, an AI seat");
    assert_eq!([stock(&view, 0)[1], stock(&view, 1)[2]], [0, 0]);
}

#[test]
fn upper_clamp_fraction_and_add_then_set() {
    let (mut sim, mut view, db) = world();
    run(&mut sim, &mut view, &db, "A2.SetMetal(0, 2000000000)");
    assert_eq!(stock(&view, 0)[2], 1_000_000_000, "a set clamps to 1e9");
    run(&mut sim, &mut view, &db, "A2.AddMetal(0, 2147483647)");
    assert_eq!(stock(&view, 0)[2], 1_000_000_000, "an add clamps to 1e9");
    // A stock f32 cannot hold (1234 + 2^-32) survives an add of 0 bit for bit.
    let fraction = Fixed::from_bits((1234 << 32) + 1);
    view = (*sim
        .step(&[Command {
            player: PlayerId::SCRIPT,
            seq: 90,
            kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 3, fraction)),
        }])
        .view)
        .clone();
    assert_eq!(view.player(PlayerId(0)).unwrap().resources[3], fraction);
    run(&mut sim, &mut view, &db, "A2.AddBiomatter(0, 0)");
    assert_eq!(view.player(PlayerId(0)).unwrap().resources[3], fraction, "no float step");
    // Add then Set in one tick: the set wins; Set then Add builds on the set.
    run(&mut sim, &mut view, &db, "A2.AddDilithium(1, 10); A2.SetDilithium(1, 7)");
    assert_eq!(stock(&view, 1)[0], 7);
    run(&mut sim, &mut view, &db, "A2.SetDilithium(1, 7); A2.AddDilithium(1, 3)");
    assert_eq!(stock(&view, 1)[0], 10);
}

#[test]
fn a_failed_resource_write_taints_no_entity_named_by_its_amount() {
    let (mut sim, _, db) = world();
    // Ten units, so the last handle is above 8 and would look like a handle as an amount.
    let spawn: Vec<Command> = (0..10)
        .map(|i| Command {
            player: PlayerId::SCRIPT,
            seq: 100 + i,
            kind: CommandKind::Edit(Edit::Spawn {
                odf: db.key("fx_cruiser").expect("fake odf"),
                player: PlayerId(0),
                pos: Vec3::from_f32([i as f32 * 50.0, 0.0, 0.0]),
                heading: (Fixed::ZERO, Fixed::ONE),
            }),
        })
        .collect();
    let view = (*sim.step(&spawn).view).clone();
    let id = view.units.iter().map(|u| u.id.0).max().unwrap();
    assert!(id > 8);
    let end = |script: String| {
        let rt = runtime(&view, &db);
        rt.eval::<()>(&script).unwrap();
        rt.mission_end(view.tick + 1000)
    };
    let win = format!("if A2.IsAlive({id}) then A2.SucceedMission(0, 0) end");
    // The amount is a value: the end stands.
    assert_eq!(end(format!("A2.NoteGap('AddMetal', 1, {id})\n{win}")), MissionEndState::Ended { won: true });
    // Argument 0 in 1..8 is a team, not a handle.
    assert_eq!(end(format!("A2.NoteGap('AddMetal', 2, 5)\n{win}")), MissionEndState::Ended { won: true });
    // The control: the same id as a handle of another gap taints it.
    assert!(matches!(end(format!("A2.NoteGap('Special_Attack', {id}, 5)\n{win}")), MissionEndState::Blocked(_)));
}
