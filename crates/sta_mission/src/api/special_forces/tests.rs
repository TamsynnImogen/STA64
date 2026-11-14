//! LUA-28: the flag round trip through the sim, the script order gate, recruitment and
//! `CommandAllShips`, on both games' bindings. Authored fake content; no installed data.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

struct Scene {
    sim: Sim,
    db: Arc<ContentDb>,
    /// Cruisers of player 0 (x=0) and 1 (x=100), a player-0 station (x=300), a player-1 frigate.
    ids: [EntityId; 4],
}

fn scene() -> Scene {
    let db = Arc::new(fake::db());
    let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let spawn = |seq, odf: &str, player, x: f32| Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: db.key(odf).unwrap(),
            player: PlayerId(player),
            pos: Vec3::from_f32([x, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    };
    let out = sim.step(&[
        spawn(1, "fx_cruiser", 0, 0.0),
        spawn(2, "fx_cruiser", 1, 100.0),
        spawn(3, "fx_station", 0, 300.0),
        spawn(4, "fx_frigate", 1, 5000.0),
    ]);
    let at = |x: f32| out.view.units.iter().find(|u| u.pos == Vec3::from_f32([x, 0.0, 0.0])).unwrap().id;
    let ids = [at(0.0), at(100.0), at(300.0), at(5000.0)];
    Scene { sim, db, ids }
}

fn runtime(s: &Scene) -> MissionRuntime {
    let cfg = MissionConfig {
        content: Some(s.db.clone()),
        view: Some((*s.sim.view()).clone()),
        ..MissionConfig::default()
    };
    let rt = MissionRuntime::from_source_with("sf.lua", "", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.eval::<()>(&format!(
        "a, b, s, far = {}, {}, {}, {}; A2.paths.P = {{ {{x=10,y=0,z=0}}, {{x=20,y=0,z=0}} }}; A1.paths.P = A2.paths.P",
        s.ids[0].0, s.ids[1].0, s.ids[2].0, s.ids[3].0
    ))
    .unwrap();
    rt
}

/// Deliver a step to the sim and its result to the mission, as the host does.
fn advance(rt: &mut MissionRuntime, sim: &mut Sim) -> Vec<SimEvent> {
    let commands = rt.take_commands();
    let out = sim.step(&commands);
    rt.think(&out.view, &out.events, &mut Vec::new());
    out.events
}

fn gated(rt: &MissionRuntime, name: &str) -> u32 {
    rt.absent_paths().iter().filter(|(k, _)| k.contains("not special forces") && k.contains(name)).map(|(_, n)| *n).sum()
}

fn orders(commands: &[Command]) -> Vec<Order> {
    commands
        .iter()
        .filter_map(|c| match &c.kind {
            CommandKind::Order(o) => Some(o.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_script_order_on_an_unflagged_craft_does_nothing_and_is_counted_on_both_games() {
    let s = scene();
    let mut rt = runtime(&s);
    for call in [
        "A2.Goto(a, 'P', false, 0)",
        "A2.Goto(a, b, false, 0)",
        "A2.Stop(a, 0)",
        "A2.Attack(a, b, 0)",
        "A2.AI_Attack(a, b)",
        "A2.AI_Attack(a)",
        "A2.Mine(a, b)",
        "A2.Patrol(a, 'P')",
        "A2.SearchAndDestroy(a)",
        "A2.Scout(a)",
        "A2.Colonize(a, s)",
        "A2.Cloak(a)",
        "A2.Decloak(a)",
        "A1.Goto(a, 'P', 0)",
        "A1.Goto(a, b, 0)",
        "A1.Stop(a, 0)",
        "A1.Attack(a, b, 0)",
        "A1.Mine(a, b)",
        "A1.Patrol(a, 'P')",
        "A1.SearchAndDestroy(a)",
        "A1.AI_Attack(a, b)",
        "A1.Cloak(a)",
        "A1.Decloak(a)",
        "A1.ImmediateCloak(a)",
    ] {
        rt.eval::<()>(call).unwrap_or_else(|e| panic!("{call}: {e}"));
    }
    assert!(rt.take_commands().is_empty(), "nothing was queued");
    for name in ["Goto", "Stop", "Attack", "AI_Attack", "Mine", "Patrol", "SearchAndDestroy", "Scout", "Colonize", "Cloak", "Decloak"] {
        assert!(gated(&rt, name) > 0, "{name} is counted");
    }
    // Nothing is missing from the seam: a gated call is not an unsupported one.
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn flagging_in_the_same_tick_opens_the_gate_and_the_flag_reaches_the_sim() {
    let mut s = scene();
    let mut rt = runtime(&s);
    assert_eq!(rt.eval::<i64>("return A2.SetSpecialForcesFlag(a, true)").unwrap(), i64::from(s.ids[0].0));
    rt.eval::<()>("A2.Goto(a, 'P', false, 0); A1.Goto(a, b, 0)").unwrap();
    let commands = rt.take_commands();
    assert!(matches!(&commands[0].kind, CommandKind::Edit(Edit::SpecialForces(id, true)) if *id == s.ids[0]));
    let moves = orders(&commands);
    assert_eq!(moves.len(), 3, "two path legs and one object leg: {moves:?}");
    assert!(moves.iter().all(|o| matches!(o, Order::Move { units, .. } if *units == vec![s.ids[0]])));
    // The sim holds the switch and the next view lists it.
    let out = s.sim.step(&commands);
    assert!(s.sim.switches(s.ids[0]).unwrap().special_forces);
    assert!(out.view.is_special_forces(s.ids[0]) && !out.view.is_special_forces(s.ids[1]));
    // A later tick reads the view, and clearing closes the gate again.
    rt.think(&out.view, &out.events, &mut Vec::new());
    rt.eval::<()>("A2.Stop(a, 0)").unwrap();
    assert_eq!(orders(&rt.take_commands()).len(), 1);
    rt.eval::<()>("A1.SetSpecialForcesFlag(a, false); A2.Stop(a, 0); A1.Stop(a, 0)").unwrap();
    let commands = rt.take_commands();
    assert!(orders(&commands).is_empty() && gated(&rt, "Stop") == 2);
    advance_commands(&mut rt, &mut s.sim, commands);
    assert!(!s.sim.switches(s.ids[0]).unwrap().special_forces);
}

fn advance_commands(rt: &mut MissionRuntime, sim: &mut Sim, commands: Vec<Command>) {
    let out = sim.step(&commands);
    rt.think(&out.view, &out.events, &mut Vec::new());
}

#[test]
fn stations_and_unknown_objects_are_not_gated_and_the_flag_calls_reject_them() {
    let s = scene();
    let mut rt = runtime(&s);
    // A station is not a craft: the order gate does not read a flag it can never have.
    rt.eval::<()>("A2.Stop(s, 0)").unwrap();
    assert_eq!(orders(&rt.take_commands()).len(), 1);
    assert_eq!(rt.eval::<i64>("return A2.SetSpecialForcesFlag(s, true)").unwrap(), 0 + i64::from(s.ids[2].0), "the A2 call returns the handle");
    let commands = rt.take_commands();
    // The sim rejects the edit for a station: it is queued and reported, the flag never sets.
    let mut sim = s.sim;
    let out = sim.step(&commands);
    assert!(out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })) || !sim.switches(s.ids[2]).unwrap().special_forces);
    assert!(!sim.switches(s.ids[2]).unwrap().special_forces);
}

#[test]
fn recruit_takes_the_first_unflagged_craft_of_the_team_and_class_on_both_games() {
    let mut s = scene();
    let mut rt = runtime(&s);
    let r: (bool, i64) = rt.eval("return A2.RecruitSpecialForces('fx_cruiser', 1, 0)").unwrap();
    assert_eq!(r, (true, i64::from(s.ids[1].0)));
    // The same tick: the queued flag already counts.
    let r: (bool, i64) = rt.eval("return A1.RecruitSpecialForces('fx_cruiser', 1, 7)").unwrap();
    assert_eq!(r, (false, 0), "out is 0 on failure, not the old value");
    advance(&mut rt, &mut s.sim);
    assert!(s.sim.switches(s.ids[1]).unwrap().special_forces);
    let r: (bool, i64) = rt.eval("return A2.RecruitSpecialForces('fx_cruiser.odf', 1)").unwrap();
    assert_eq!(r, (false, 0), "now flagged");
    // A1: team 0 owns cruiser a; wrong team, unknown and empty ODF report false.
    let r: (bool, i64) = rt.eval("return A1.RecruitSpecialForces('FX_CRUISER', 0, 0)").unwrap();
    assert_eq!(r, (true, i64::from(s.ids[0].0)));
    for call in ["A1.RecruitSpecialForces('fx_cruiser', 5, 0)", "A1.RecruitSpecialForces('nothing', 0, 0)", "A1.RecruitSpecialForces('', 0, 0)"] {
        let r: (bool, i64) = rt.eval(&format!("return {call}")).unwrap_or_else(|e| panic!("{call}: {e}"));
        assert!(!r.0 && r.1 == 0, "{call}");
    }
    // Clearing frees it; a station is never recruited (not a craft class).
    rt.eval::<()>("A2.SetSpecialForcesFlag(b, false)").unwrap();
    advance(&mut rt, &mut s.sim);
    let r: (bool, i64) = rt.eval("return A2.RecruitSpecialForces('fx_cruiser', 1, 0)").unwrap();
    assert_eq!(r, (true, i64::from(s.ids[1].0)));
    let r: (bool, i64) = rt.eval("return A2.RecruitSpecialForces('fx_station', 0, 0)").unwrap();
    assert!(!r.0);
}

#[test]
fn the_flag_is_cleared_by_a_team_change_and_the_owner_can_still_order_a_flagged_craft() {
    let mut s = scene();
    let flag = Command { player: PlayerId::SCRIPT, seq: 10, kind: CommandKind::Edit(Edit::SpecialForces(s.ids[0], true)) };
    s.sim.step(&[flag]);
    assert!(s.sim.view().is_special_forces(s.ids[0]));
    // The player's own order is not gated by the flag.
    let mv = Command { player: PlayerId(0), seq: 1, kind: CommandKind::Order(Order::Move { units: vec![s.ids[0]], to: Vec3::from_f32([50.0, 0.0, 0.0]), queue: false }) };
    let out = s.sim.step(&[mv]);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    // A team change clears it (the original's SetTeam does).
    let give = Command { player: PlayerId::SCRIPT, seq: 11, kind: CommandKind::Edit(Edit::Team(s.ids[0], PlayerId(1))) };
    let out = s.sim.step(&[give]);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })), "{:?}", out.events);
    for _ in 0..3 {
        s.sim.step(&[]);
    }
    assert_eq!(s.sim.view().unit(s.ids[0]).unwrap().owner, PlayerId(1), "the craft changed hands");
    assert!(!s.sim.view().is_special_forces(s.ids[0]), "and lost the flag");
}

#[test]
fn command_all_ships_with_the_flag_argument_selects_only_flagged_craft() {
    let mut s = scene();
    let mut rt = runtime(&s);
    rt.eval::<()>("A2.SetSpecialForcesFlag(b, true)").unwrap();
    advance(&mut rt, &mut s.sim);
    // Team -1: every craft; only the flagged one when the fourth argument is true.
    rt.eval::<()>("A2.CommandAllShips(3, -1, nil, true, false)").unwrap();
    let only = orders(&rt.take_commands());
    assert_eq!(only, vec![Order::Stop { units: vec![s.ids[1]] }]);
    rt.eval::<()>("A2.CommandAllShips(3, -1, nil, false, true)").unwrap();
    assert!(orders(&rt.take_commands()).len() > 1, "without the argument, every craft is ordered");
}

#[test]
fn a_built_object_is_flagged_and_ordered_in_call_order_once_it_exists() {
    let db = Arc::new(fake::db());
    let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let cfg = MissionConfig { content: Some(db), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let src = "A2.paths.Q = { {x=40,y=0,z=0}, {x=50,y=0,z=0} }; h = A2.BuildObject('fx_cruiser.odf', 1, 'Q', 1); \
               f = A2.SetSpecialForcesFlag(h, true); A2.Goto(h, 'Q', false, 0)";
    let mut rt = MissionRuntime::from_source_with("build.lua", src, 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let token: u32 = rt.eval("return h").unwrap();
    assert_eq!(rt.eval::<u32>("return f").unwrap(), token, "the flag call returns the handle it was given");
    let first = rt.take_commands();
    assert!(orders(&first).is_empty(), "the order waits for the object");
    let out = sim.step(&first);
    let id = out.events.iter().find_map(|e| match e { SimEvent::ScriptSpawned { id, .. } => Some(*id), _ => None }).unwrap();
    rt.think(&out.view, &out.events, &mut Vec::new());
    // Any A2 call releases the held ones, in order: the flag, then the move it permits.
    rt.eval::<()>("A2.GetLocation(h)").unwrap();
    let commands = rt.take_commands();
    assert!(matches!(&commands[0].kind, CommandKind::Edit(Edit::SpecialForces(who, true)) if *who == id), "{commands:?}");
    assert!(orders(&commands).iter().all(|o| matches!(o, Order::Move { .. })) && orders(&commands).len() == 2, "{commands:?}");
    assert!(rt.absent_paths().iter().all(|(k, _)| !k.contains("not special forces")));
}
