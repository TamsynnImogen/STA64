//! Start stock from the map's team table (own-authored table values; no game assets): a bare
//! mission game, converted-mission numbering (Armada team n is seat n - 1), a human seat and an AI seat.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{Fixed, PlayerId, TeamId};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::WorldView;
use sta_sim::{Sim, WorldSource};

use super::*;
use crate::api::{MissionConfig, PlayerNumbering};
use crate::runtime::MissionRuntime;

fn bare(controls: [Control; 2]) -> (Sim, WorldView, Arc<ContentDb>) {
    let db = Arc::new(fake::db());
    let seat = |n: u8, control| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control };
    let [c0, c1] = controls;
    let setup = GameSetup { seed: 3, map: Arc::new(MapDef::empty()), content: db.clone(), players: vec![seat(0, c0), seat(1, c1)], options: GameOptions::default() };
    let sim = Sim::from_setup(&setup);
    let view = (*sim.view()).clone();
    (sim, view, db)
}

fn table(human: [f32; 3], ai: [f32; 3]) -> Vec<sta_bzn::TeamStart> {
    let t = |v: [f32; 3], crew, officers| sta_bzn::TeamStart { dilithium: v[0], latinum: v[1], metal: v[2], crew, officers };
    vec![t(human, 3000.0, 600), t(ai, 4000.0, 0)]
}

fn runtime(script: &str, view: &WorldView, db: &Arc<ContentDb>, start: Vec<SeatStart>) -> MissionRuntime {
    let cfg = MissionConfig { content: Some(db.clone()), view: Some(view.clone()), player_numbering: PlayerNumbering::ArmadaTeams, ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("m.lua", script, 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.set_start_stock(start);
    rt
}

fn play(rt: &mut MissionRuntime, sim: &mut Sim, view: &mut WorldView, ticks: usize) -> Vec<Command> {
    let mut all = Vec::new();
    for _ in 0..ticks {
        let mut out = Vec::new();
        rt.think(view, &[], &mut out);
        all.extend(out.clone());
        *view = (*sim.step(&out).view).clone();
    }
    all
}

fn stock(view: &WorldView, seat: u8) -> Vec<i64> {
    view.player(PlayerId(seat)).unwrap().resources.iter().map(|v| v.to_f32().round() as i64).collect()
}

#[test]
fn the_human_seat_has_the_tables_stock_crew_and_officers_after_the_first_tick() {
    let (mut sim, mut view, db) = bare([Control::Human, Control::Idle]);
    assert!(view.player(PlayerId(0)).unwrap().resources.is_empty(), "a bare mission seat starts with nothing");
    let mut rt = runtime("", &view, &db, from_table(&table([2000.0, 0.0, 1000.0], [3500.0, 1000.0, 12500.0])));
    play(&mut rt, &mut sim, &mut view, 2);
    assert_eq!(&stock(&view, 0)[..3], &[2000, 0, 1000]);
    assert_eq!(&stock(&view, 1)[..3], &[3500, 1000, 12500]);
    let p = view.player(PlayerId(0)).unwrap();
    assert_eq!(p.crew, Some(Fixed::from_int(3000)));
    assert_eq!(p.officers[1], Fixed::from_int(600));
    assert_eq!(view.player(PlayerId(1)).unwrap().officers[1], Fixed::ZERO, "a table limit of 0 is not written");
}

#[test]
fn a_seat_the_table_does_not_cover_is_unchanged() {
    let (mut sim, mut view, db) = bare([Control::Human, Control::Idle]);
    let only_human = from_table(&table([2000.0, 0.0, 1000.0], [1.0, 1.0, 1.0]))[..1].to_vec();
    let mut rt = runtime("", &view, &db, only_human);
    play(&mut rt, &mut sim, &mut view, 2);
    assert_eq!(&stock(&view, 0)[..3], &[2000, 0, 1000]);
    assert!(view.player(PlayerId(1)).unwrap().resources.is_empty());
}

#[test]
fn a_seat_that_already_has_a_loadout_is_left_alone() {
    let seat = from_table(&table([2000.0, 0.0, 1000.0], [0.0; 3]))[0].clone();
    assert!(seat.edits(true).is_empty(), "a skirmish start (crew or officers placed) takes nothing from the table");
    assert!(matches!(seat.edits(false)[..], [Edit::Cargo(..), Edit::Cargo(..), Edit::Cargo(..), Edit::TeamCrew(..), Edit::MaxOfficers(..)]));
}

#[test]
fn a_seat_with_a_loadout_in_the_view_gets_no_edit() {
    let (mut sim, mut view, db) = bare([Control::Human, Control::Idle]);
    // A skirmish-like seat: crew and officer limit already there, stock too.
    let mut rt = runtime("", &view, &db, from_table(&table([2000.0, 0.0, 1000.0], [3500.0, 1000.0, 12500.0]))[..1].to_vec());
    let mut v = view.clone();
    let p = v.players.iter_mut().find(|p| p.id == PlayerId(0)).unwrap();
    p.crew = Some(Fixed::from_int(750));
    p.resources = vec![Fixed::from_int(9); 4];
    let mut out = Vec::new();
    rt.think(&v, &[], &mut out);
    assert!(out.is_empty(), "{out:?}");
    play(&mut rt, &mut sim, &mut view, 1);
}

#[test]
fn a_script_write_on_the_first_step_follows_the_start_values() {
    // Armada team 2 is seat 1 (a Set wins), team 1 is seat 0 (an Add builds on the start value).
    let (mut sim, mut view, db) = bare([Control::Human, Control::Idle]);
    let script = "DONE = false\nMMM.register(function(t) if not DONE then DONE = true; A2.SetDilithium(2, 777); A2.AddMetal(1, 500) end end)";
    let mut rt = runtime(script, &view, &db, from_table(&table([2000.0, 0.0, 1000.0], [3500.0, 1000.0, 12500.0])));
    let sent = play(&mut rt, &mut sim, &mut view, 3);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(&stock(&view, 1)[..3], &[777, 1000, 12500], "the script's SetDilithium wins");
    assert_eq!(&stock(&view, 0)[..3], &[2000, 0, 1500], "AddMetal adds to the start value");
    let cargo: Vec<_> = sent.iter().filter_map(|c| match &c.kind { CommandKind::Edit(Edit::Cargo(p, r, _)) => Some((p.0, *r)), _ => None }).collect();
    let first_script = cargo.iter().position(|&(p, r)| (p, r) == (1, 0)).unwrap();
    assert!(cargo[..first_script].contains(&(0, 2)), "the start values are sent before the script's own writes");
}

#[test]
fn the_start_values_are_ordinary_script_commands_a_replay_carries() {
    let (mut sim, mut view, db) = bare([Control::Human, Control::Idle]);
    let mut rt = runtime("", &view, &db, from_table(&table([2000.0, 0.0, 1000.0], [3500.0, 1000.0, 12500.0])));
    let sent = play(&mut rt, &mut sim, &mut view, 3);
    assert!(sent.iter().all(|c| c.player == PlayerId::SCRIPT && matches!(c.kind, CommandKind::Edit(_))));
    assert_eq!(sent.len(), 3 + 1 + 1 + 3 + 1, "3 cargo + crew + officers for seat 0, 3 cargo + crew for seat 1, once");
}
