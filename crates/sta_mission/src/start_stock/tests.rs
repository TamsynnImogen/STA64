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
    let t = |v: [f32; 3], crew, officers| sta_bzn::TeamStart { dilithium: v[0], latinum: v[1], metal: v[2], biomatter: 77.0, crew, officers };
    vec![t(human, 1717.0, 321), t(ai, 2828.0, 0)]
}

fn runtime(script: &str, view: &WorldView, db: &Arc<ContentDb>, start: Vec<SeatStart>) -> MissionRuntime {
    let cfg = MissionConfig { content: Some(db.clone()), view: Some(view.clone()), player_numbering: PlayerNumbering::ArmadaTeams, ..MissionConfig::default() };
    let src = script.as_bytes().to_vec();
    let rt = MissionRuntime::load_with_start("m.lua", 3, cfg, Vec::new(), StartStock { seats: start, refused: None }, move |p| (p == "m.lua").then(|| src.clone()));
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
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
    let mut rt = runtime("", &view, &db, from_table(&table([1234.0, 56.0, 789.0], [4321.0, 65.0, 987.0])));
    play(&mut rt, &mut sim, &mut view, 2);
    assert_eq!(&stock(&view, 0)[..4], &[1234, 56, 789, 77], "biomatter is the fourth slot");
    assert_eq!(&stock(&view, 1)[..4], &[4321, 65, 987, 77]);
    let p = view.player(PlayerId(0)).unwrap();
    assert_eq!(p.crew, Some(Fixed::from_int(1717)));
    assert_eq!(p.officers[1], Fixed::from_int(321));
    assert_eq!(view.player(PlayerId(1)).unwrap().officers[1], Fixed::ZERO, "a table limit of 0 is not written");
}

#[test]
fn a_seat_the_table_does_not_cover_is_unchanged() {
    let (mut sim, mut view, db) = bare([Control::Human, Control::Idle]);
    let only_human = from_table(&table([1234.0, 56.0, 789.0], [1.0, 1.0, 1.0]))[..1].to_vec();
    let mut rt = runtime("", &view, &db, only_human);
    play(&mut rt, &mut sim, &mut view, 2);
    assert_eq!(&stock(&view, 0)[..3], &[1234, 56, 789]);
    assert!(view.player(PlayerId(1)).unwrap().resources.is_empty());
}

#[test]
fn a_seat_that_already_has_a_loadout_is_left_alone() {
    let seat = from_table(&table([1234.0, 56.0, 789.0], [0.0; 3]))[0].clone();
    assert!(seat.edits(true).is_empty(), "a skirmish start (crew or officers placed) takes nothing from the table");
    assert!(matches!(seat.edits(false)[..], [Edit::Cargo(..), Edit::Cargo(..), Edit::Cargo(..), Edit::Cargo(..), Edit::TeamCrew(..), Edit::MaxOfficers(..)]));
}

#[test]
fn a_seat_with_a_loadout_in_the_view_gets_no_edit() {
    let (mut sim, mut view, db) = bare([Control::Human, Control::Idle]);
    // A skirmish-like seat: crew already there.
    let p = view.players.iter_mut().find(|p| p.id == PlayerId(0)).unwrap();
    p.crew = Some(Fixed::from_int(750));
    p.resources = vec![Fixed::from_int(9); 4];
    let mut rt = runtime("", &view, &db, from_table(&table([1234.0, 56.0, 789.0], [4321.0, 65.0, 987.0]))[..1].to_vec());
    let sent = play(&mut rt, &mut sim, &mut view, 2);
    assert!(sent.is_empty(), "{sent:?}");
}

#[test]
fn a_script_that_sets_stock_while_it_loads_is_not_overwritten() {
    // Armada team 1 is seat 0; the call runs at load, before any think.
    let (mut sim, mut view, db) = bare([Control::Human, Control::Idle]);
    let mut rt = runtime("A2.SetDilithium(1, 555)\nA2.AddMetal(1, 11)", &view, &db, from_table(&table([1234.0, 56.0, 789.0], [4321.0, 65.0, 987.0])));
    play(&mut rt, &mut sim, &mut view, 3);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(&stock(&view, 0)[..3], &[555, 56, 800], "Set wins and Add builds on the start value");
}

#[test]
fn a_script_write_on_the_first_step_follows_the_start_values() {
    // Armada team 2 is seat 1 (a Set wins), team 1 is seat 0 (an Add builds on the start value).
    let (mut sim, mut view, db) = bare([Control::Human, Control::Idle]);
    let script = "DONE = false\nMMM.register(function(t) if not DONE then DONE = true; A2.SetDilithium(2, 777); A2.AddMetal(1, 500) end end)";
    let mut rt = runtime(script, &view, &db, from_table(&table([1234.0, 56.0, 789.0], [4321.0, 65.0, 987.0])));
    let sent = play(&mut rt, &mut sim, &mut view, 3);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(&stock(&view, 1)[..3], &[777, 65, 987], "the script's SetDilithium wins");
    assert_eq!(&stock(&view, 0)[..3], &[1234, 56, 1289], "AddMetal adds to the start value");
    let cargo: Vec<_> = sent.iter().filter_map(|c| match &c.kind { CommandKind::Edit(Edit::Cargo(p, r, _)) => Some((p.0, *r)), _ => None }).collect();
    let first_script = cargo.iter().position(|&(p, r)| (p, r) == (1, 0)).unwrap();
    assert!(cargo[..first_script].contains(&(0, 2)), "the start values are sent before the script's own writes");
}

#[test]
fn the_start_values_are_ordinary_script_commands_a_replay_carries() {
    let (mut sim, mut view, db) = bare([Control::Human, Control::Idle]);
    let mut rt = runtime("", &view, &db, from_table(&table([1234.0, 56.0, 789.0], [4321.0, 65.0, 987.0])));
    let sent = play(&mut rt, &mut sim, &mut view, 3);
    assert!(sent.iter().all(|c| c.player == PlayerId::SCRIPT && matches!(c.kind, CommandKind::Edit(_))));
    assert_eq!(sent.len(), 4 + 1 + 1 + 4 + 1, "4 cargo + crew + officers for seat 0, 4 cargo + crew for seat 1, once");
}

#[test]
fn a_refused_table_is_listed_once_and_grants_nothing() {
    let (mut sim, mut view, db) = bare([Control::Human, Control::Idle]);
    let cfg = MissionConfig { content: Some(db.clone()), view: Some(view.clone()), ..MissionConfig::default() };
    let start = StartStock { seats: Vec::new(), refused: Some("map team table refused (block runs [9]): no start values".into()) };
    let mut rt = MissionRuntime::load_with_start("m.lua", 3, cfg, Vec::new(), start, |_| Some(Vec::new()));
    let sent = play(&mut rt, &mut sim, &mut view, 2);
    assert!(sent.is_empty());
    let listed: Vec<_> = rt.unsupported().into_iter().filter(|(n, _)| n.contains("team table")).collect();
    assert_eq!(listed.len(), 1, "{listed:?}");
}
