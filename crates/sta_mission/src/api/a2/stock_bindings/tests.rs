//! Tests of the LUA-19 calls on authored scenes (fake content, two players);
//! no installed game data.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::WorldView;
use sta_sim::{Sim, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

struct Scene {
    sim: Sim,
    view: WorldView,
    db: Arc<ContentDb>,
    /// Cruisers of player 0 (x=0) and 1 (x=100), a player-0 station (x=300),
    /// a far player-1 frigate (x=5000); ascending ids.
    ids: [EntityId; 4],
}

fn game(db: &Arc<ContentDb>) -> GameSetup {
    let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Idle };
    GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    }
}

fn scene() -> Scene {
    // The fake station carries the native `is_starbase` flag.
    let fake_db = fake::db();
    let mut builder = ContentDb::builder().profile(fake_db.profile()).features(fake_db.features().clone());
    for unit in fake_db.units() {
        let mut unit = unit.clone();
        unit.is_starbase |= unit.odf == "fx_station";
        builder = builder.unit(unit);
    }
    for weapon in fake_db.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    let db = Arc::new(builder.build().unwrap());
    let mut sim = Sim::from_setup(&game(&db));
    let spawn = |seq, odf: &str, player, x: f32| Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: db.key(odf).expect("fake odf"),
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
    Scene { ids: [at(0.0), at(100.0), at(300.0), at(5000.0)], view: (*out.view).clone(), sim, db }
}

fn runtime(s: &Scene) -> MissionRuntime {
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some(s.view.clone()), ..MissionConfig::default() };
    let rt = MissionRuntime::from_source_with("stock_bindings.lua", "", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.eval::<()>(&format!("a, b, s, far = {}, {}, {}, {}", s.ids[0].0, s.ids[1].0, s.ids[2].0, s.ids[3].0)).unwrap();
    rt
}

fn stops(commands: &[Command]) -> Vec<(PlayerId, Order)> {
    commands
        .iter()
        .map(|c| match &c.kind {
            CommandKind::Order(o) => (c.player, o.clone()),
            other => panic!("not an order: {other:?}"),
        })
        .collect()
}

fn counted(rt: &MissionRuntime, what: &str) -> bool {
    rt.unsupported().iter().any(|(k, _)| k.contains(what))
}

#[test]
fn all_stations_destroyed_is_a_zero_station_count() {
    let mut s = scene();
    let mut rt = runtime(&s);
    assert!(!rt.eval::<bool>("return A2.AllStationsDestroyed(0)").unwrap(), "team 0 has a station");
    assert!(rt.eval::<bool>("return A2.AllStationsDestroyed(1)").unwrap(), "team 1 has none, only craft");
    // The station dies: the call turns true on the next view.
    let kill = Command { player: PlayerId::SCRIPT, seq: 10, kind: CommandKind::Edit(Edit::Kill(s.ids[2])) };
    let out = s.sim.step(&[kill]);
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert!(rt.eval::<bool>("return A2.AllStationsDestroyed(0)").unwrap());
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
    for bad in ["A2.AllStationsDestroyed()", "A2.AllStationsDestroyed(0, 1)", "A2.AllStationsDestroyed('0')", "A2.AllStationsDestroyed(0.5)", "A2.AllStationsDestroyed(300)"] {
        assert!(rt.eval::<()>(bad).is_err(), "{bad}");
    }
}

#[test]
fn next_craft_walks_the_list_in_ascending_id_and_ends_with_zero() {
    let s = scene();
    let rt = runtime(&s);
    // No walk started: nothing (the engine's cursor would be stale).
    assert_eq!(rt.eval::<i64>("return A2.GetNextCraft()").unwrap(), 0);
    let joined: String = rt
        .eval(
            "local out = {}; local h = A2.GetFirstCraft(); while h ~= 0 do out[#out + 1] = h; h = A2.GetNextCraft() end; return table.concat(out, ',')",
        )
        .unwrap();
    let walk: Vec<i64> = joined.split(',').map(|n| n.parse().unwrap()).collect();
    let mut want: Vec<i64> = s.ids.iter().map(|id| i64::from(id.0)).collect();
    want.sort_unstable();
    assert_eq!(walk, want);
    // Past the end it stays 0; a new GetFirstCraft starts over.
    assert_eq!(rt.eval::<i64>("return A2.GetNextCraft()").unwrap(), 0);
    assert_eq!(rt.eval::<i64>("return A2.GetFirstCraft()").unwrap(), want[0]);
    assert_eq!(rt.eval::<i64>("return A2.GetNextCraft()").unwrap(), want[1]);
    assert!(rt.eval::<()>("A2.GetNextCraft(1)").is_err());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn next_craft_skips_an_object_that_died_during_the_walk() {
    let mut s = scene();
    let mut rt = runtime(&s);
    let first = rt.eval::<i64>("return A2.GetFirstCraft()").unwrap();
    assert_eq!(first, i64::from(s.ids[0].0));
    let kill = Command { player: PlayerId::SCRIPT, seq: 10, kind: CommandKind::Edit(Edit::Kill(s.ids[1])) };
    let out = s.sim.step(&[kill]);
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert_eq!(rt.eval::<i64>("return A2.GetNextCraft()").unwrap(), i64::from(s.ids[2].0));
}

#[test]
fn command_all_ships_stop_of_a_team_orders_its_craft_in_id_order_as_their_owner() {
    let s = scene();
    let mut rt = runtime(&s);
    // The stock form: STOP, team, null odf (the integer 0), both flags false.
    rt.eval::<()>("A2.CommandAllShips(3, 0, 0, false, false)").unwrap();
    assert_eq!(
        stops(&rt.take_commands()),
        vec![
            (PlayerId(0), Order::Stop { units: vec![s.ids[0]] }),
            (PlayerId(0), Order::Stop { units: vec![s.ids[2]] }),
        ]
    );
    // Any team, one class.
    rt.eval::<()>("A2.CommandAllShips(3, -1, 'fx_cruiser.odf', false, false)").unwrap();
    assert_eq!(
        stops(&rt.take_commands()),
        vec![
            (PlayerId(0), Order::Stop { units: vec![s.ids[0]] }),
            (PlayerId(1), Order::Stop { units: vec![s.ids[1]] }),
        ]
    );
    // The second flag leaves the stations out.
    rt.eval::<()>("A2.CommandAllShips(3, 0, nil, false, true)").unwrap();
    assert_eq!(stops(&rt.take_commands()), vec![(PlayerId(0), Order::Stop { units: vec![s.ids[0]] })]);
    // A team with nothing to command queues nothing.
    rt.eval::<()>("A2.CommandAllShips(3, 1, 'fx_station', false, false)").unwrap();
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn command_all_ships_attack_without_a_target_is_attack_any() {
    let s = scene();
    let mut rt = runtime(&s);
    rt.eval::<()>("A2.CommandAllShips(6, 1, nil, false, true)").unwrap();
    assert_eq!(
        stops(&rt.take_commands()),
        vec![
            (PlayerId(1), Order::AttackAny { units: vec![s.ids[1]] }),
            (PlayerId(1), Order::AttackAny { units: vec![s.ids[3]] }),
        ]
    );
}

#[test]
fn command_all_ships_raises_and_counts_what_it_cannot_serve_before_queueing() {
    let s = scene();
    let mut rt = runtime(&s);
    // A command with no order without a target.
    assert!(rt.eval::<()>("A2.CommandAllShips(4, 0, nil, false, false)").is_err());
    assert!(counted(&rt, "CommandAllShips: command 4"));
    // The unidentified first flag.
    assert!(rt.eval::<()>("A2.CommandAllShips(3, 0, nil, true, false)").is_err());
    assert!(counted(&rt, "CommandAllShips: the per-craft state"));
    assert!(rt.take_commands().is_empty());
    for bad in [
        "A2.CommandAllShips(3, 0, nil, false)",
        "A2.CommandAllShips(57, 0, nil, false, false)",
        "A2.CommandAllShips(3, 99, nil, false, false)",
        "A2.CommandAllShips(3, 0, 5, false, false)",
        "A2.CommandAllShips(3, 0, nil, 0, false)",
        "A2.CommandAllShips(3, 0, nil, false, 1)",
    ] {
        assert!(rt.eval::<()>(bad).is_err(), "{bad}");
    }
    assert!(rt.take_commands().is_empty());
}
