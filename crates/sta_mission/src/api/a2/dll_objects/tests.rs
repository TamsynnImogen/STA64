//! Tests of the object-creation, search and flag calls. Fixtures are authored
//! here (fake content, two players); no installed game data.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order, Subsystem};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::WorldView;
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

struct Scene {
    view: WorldView,
    db: Arc<ContentDb>,
    /// Cruisers of player 0 (x=0) and 1 (x=100), a player-0 station (x=300),
    /// a far player-1 frigate (x=5000).
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
    let db = Arc::new(fake::db());
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
    Scene { ids: [at(0.0), at(100.0), at(300.0), at(5000.0)], view: (*out.view).clone(), db }
}

fn runtime(s: &Scene) -> MissionRuntime {
    let cfg = MissionConfig {
        content: Some(s.db.clone()),
        view: Some(s.view.clone()),
        ..MissionConfig::default()
    };
    let rt = MissionRuntime::from_source_with("dll_objects.lua", "", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.eval::<()>(&format!(
        "a, b, s, far = {}, {}, {}, {}; A2.paths.P = {{ {{x=10,y=0,z=0}}, {{x=20,y=0,z=0}} }}",
        s.ids[0].0, s.ids[1].0, s.ids[2].0, s.ids[3].0
    ))
    .unwrap();
    rt
}

fn unsupported(rt: &MissionRuntime, needle: &str) -> u32 {
    rt.unsupported().iter().filter(|(k, _)| k.contains(needle)).map(|(_, n)| *n).sum()
}

/// Deliver a step's receipts to the mission, as the host does.
fn advance(rt: &mut MissionRuntime, sim: &mut Sim) -> Vec<SimEvent> {
    let commands = rt.take_commands();
    let out = sim.step(&commands);
    rt.think(&out.view, &out.events, &mut Vec::new());
    out.events
}

#[test]
fn special_forces_flag_returns_the_handle_and_ignores_invalid_objects() {
    let s = scene();
    let mut rt = runtime(&s);
    assert_eq!(rt.eval::<i64>("return A2.SetSpecialForcesFlag(a, true)").unwrap(), i64::from(s.ids[0].0));
    assert_eq!(rt.eval::<i64>("return A2.SetSpecialForcesFlag(a, false)").unwrap(), i64::from(s.ids[0].0));
    // Native: absent objects return 0 and change nothing.
    assert_eq!(rt.eval::<i64>("return A2.SetSpecialForcesFlag(0, true)").unwrap(), 0);
    assert_eq!(rt.eval::<i64>("return A2.SetSpecialForcesFlag(999999, true)").unwrap(), 0);
    assert!(rt.take_commands().is_empty(), "the flag is mission state, not a command");
    for bad in ["", "a", "a, 1", "a, true, 1", "'a', true", "-1, true", "a, 1"] {
        assert!(rt.eval::<()>(&format!("A2.SetSpecialForcesFlag({bad})")).is_err(), "{bad}");
    }
}

#[test]
fn recruit_takes_the_first_unflagged_craft_of_that_team_and_odf() {
    let s = scene();
    let mut rt = runtime(&s);
    // Team 1 owns the second cruiser and the frigate.
    let r: (bool, i64) = rt.eval("return A2.RecruitSpecialForces('fx_cruiser', 1, 0)").unwrap();
    assert_eq!(r, (true, i64::from(s.ids[1].0)));
    // Now flagged: the same request finds nothing and reports 0.
    let r: (bool, i64) = rt.eval("return A2.RecruitSpecialForces('fx_cruiser.odf', 1)").unwrap();
    assert_eq!(r, (false, 0));
    // A script flag has the same effect; clearing it frees the craft.
    rt.eval::<()>("A2.SetSpecialForcesFlag(far, true)").unwrap();
    let r: (bool, i64) = rt.eval("return A2.RecruitSpecialForces('fx_frigate', 1, 0)").unwrap();
    assert!(!r.0);
    rt.eval::<()>("A2.SetSpecialForcesFlag(far, false)").unwrap();
    let r: (bool, i64) = rt.eval("return A2.RecruitSpecialForces('FX_FRIGATE', 1, 0)").unwrap();
    assert_eq!(r, (true, i64::from(s.ids[3].0)));
    // Wrong team, null and unknown ODF: false and handle 0.
    for call in ["('fx_cruiser', 5, 0)", "(nil, 1, 0)", "('nothing', 1, 0)", "('', 1, 0)"] {
        let r: (bool, i64) = rt.eval(&format!("return A2.RecruitSpecialForces{call}")).unwrap();
        assert_eq!(r, (false, 0), "{call}");
    }
    assert!(rt.eval::<()>("A2.RecruitSpecialForces('fx_cruiser')").is_err());
    assert!(rt.eval::<()>("A2.RecruitSpecialForces('fx_cruiser', 'x', 0)").is_err());
    assert!(rt.take_commands().is_empty());
}

#[test]
fn build_object_queues_a_spawn_and_its_handle_resolves_after_the_receipt() {
    let s = scene();
    let mut sim = Sim::from_setup(&game(&s.db));
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let src = "A2.paths.Q = { {x=40,y=0,z=0}, {x=50,y=0,z=0} }; h = A2.BuildObject('fx_cruiser.odf', 1, 'Q', 1); \
               f = A2.SetSpecialForcesFlag(h, true)";
    let mut rt = MissionRuntime::from_source_with("build.lua", src, 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let token: u32 = rt.eval("return h").unwrap();
    assert!(token >= super::TOKEN_BASE);
    // The pending token is flagged at once and is no object yet.
    assert_eq!(rt.eval::<u32>("return f").unwrap(), token);
    assert!(!rt.eval::<bool>("return A2.Exists(h)").unwrap());
    let commands = rt.take_commands();
    assert!(matches!(&commands[0].kind, CommandKind::Edit(Edit::SpawnRequest { player, pos, .. })
        if *player == PlayerId(1) && *pos == Vec3::from_f32([50.0, 0.0, 0.0])));
    let out = sim.step(&commands);
    let id = out.events.iter().find_map(|e| match e { SimEvent::ScriptSpawned { id, .. } => Some(*id), _ => None }).unwrap();
    rt.think(&out.view, &out.events, &mut Vec::new());
    // Every A2 call now accepts the token as the real object.
    assert!(rt.eval::<bool>("return A2.Exists(h) and A2.IsAlive(h)").unwrap());
    assert_eq!(rt.eval::<f64>("return A2.GetLocation(h).x").unwrap(), 50.0);
    // The flag followed the object: recruiting skips it.
    let r: (bool, i64) = rt.eval("return A2.RecruitSpecialForces('fx_cruiser', 1, 0)").unwrap();
    assert_eq!(r, (false, 0));
    rt.eval::<()>("A2.SetSpecialForcesFlag(h, false)").unwrap();
    let r: (bool, i64) = rt.eval("return A2.RecruitSpecialForces('fx_cruiser', 1, 0)").unwrap();
    assert_eq!(r, (true, i64::from(id.0)));
    rt.eval::<()>("A2.RemoveObject(h)").unwrap();
    assert!(matches!(rt.take_commands()[0].kind, CommandKind::Edit(Edit::Kill(k)) if k == id));
}

#[test]
fn build_object_edge_cases() {
    let s = scene();
    let mut rt = runtime(&s);
    // A point beyond the path returns 0 and queues nothing.
    assert_eq!(rt.eval::<i64>("return A2.BuildObject('fx_cruiser', 0, 'P', 2)").unwrap(), 0);
    assert_eq!(rt.eval::<i64>("return A2.BuildObject('fx_cruiser', 0, 'P', -1)").unwrap(), 0);
    // An unknown ODF returns 0 and is counted.
    assert_eq!(rt.eval::<i64>("return A2.BuildObject('no_such_odf', 0, 'P', 0)").unwrap(), 0);
    assert_eq!(unsupported(&rt, "BuildObject: unknown ODF"), 1);
    assert!(rt.take_commands().is_empty());
    // A path the map does not have: the engine's lookup is null, the call returns 0 (LUA-18).
    assert_eq!(rt.eval::<i64>("return A2.BuildObject('fx_cruiser', 0, 'NOPE', 0)").unwrap(), 0);
    assert_eq!(unsupported(&rt, "not in A2.paths"), 0);
    assert!(rt.take_commands().is_empty());
    // Path names also match without case.
    assert!(rt.eval::<i64>("return A2.BuildObject('fx_cruiser', 0, 'p', 0)").unwrap() >= i64::from(super::TOKEN_BASE));
    // Object form: at the object's position; an offset is not measured.
    rt.take_commands();
    assert!(rt.eval::<i64>("return A2.BuildObject('fx_cruiser', 1, b, 0, 0, 0)").unwrap() >= i64::from(super::TOKEN_BASE));
    let spawned = rt.take_commands();
    assert!(matches!(&spawned[0].kind, CommandKind::Edit(Edit::SpawnRequest { pos, .. }) if *pos == Vec3::from_f32([100.0, 0.0, 0.0])));
    assert!(rt.eval::<()>("A2.BuildObject('fx_cruiser', 1, b, 1, 0, 0)").is_err());
    assert!(rt.eval::<()>("A2.BuildObject('fx_cruiser', 1, 999999, 0, 0, 0)").is_err());
    // Bad shapes.
    for bad in ["", "'fx_cruiser', 0", "'fx_cruiser', 0, 'P'", "nil, 0, 'P', 0", "'fx_cruiser', 9, 'P', 0", "'fx_cruiser', 0, 'P', 'x'"] {
        assert!(rt.eval::<()>(&format!("A2.BuildObject({bad})")).is_err(), "{bad}");
    }
}

#[test]
fn nearest_searches_follow_the_native_filters() {
    let s = scene();
    let mut rt = runtime(&s);
    let near = |rt: &mut MissionRuntime, call: &str| rt.eval::<i64>(&format!("return {call}")).unwrap();
    let (a, b, st, far) = (i64::from(s.ids[0].0), i64::from(s.ids[1].0), i64::from(s.ids[2].0), i64::from(s.ids[3].0));
    let v = |x: i32| format!("Vector({x}, 0, 0)");
    // Nearest of any team, then by team and ODF.
    assert_eq!(near(&mut rt, &format!("A2.GetNearestObjectNearPosition({}, 50, -1, nil, -1, false, 0)", v(90))), b);
    assert_eq!(near(&mut rt, &format!("A2.GetNearestObjectNearPosition({}, 500, 0, nil, -1, false, 0)", v(90))), a);
    assert_eq!(near(&mut rt, &format!("A2.GetNearestObjectNearPosition({}, 500, 0, 'fx_station.odf', -1, false, 0)", v(90))), st);
    // Radius is inclusive; nothing within returns 0; negative is unlimited.
    assert_eq!(near(&mut rt, &format!("A2.GetNearestObjectNearPosition({}, 10, -1, nil, -1, false, 0)", v(90))), b);
    assert_eq!(near(&mut rt, &format!("A2.GetNearestObjectNearPosition({}, 9.9, -1, nil, -1, false, 0)", v(90))), 0);
    assert_eq!(near(&mut rt, &format!("A2.GetNearestObjectNearPosition({}, -1, 1, 'fx_frigate', -1, false, 0)", v(0))), far);
    // Exclusion by handle; a tie goes to the later unit (b and s are 100 from x=200).
    assert_eq!(near(&mut rt, &format!("A2.GetNearestObjectNearPosition({}, 150, -1, nil, -1, false, b)", v(100))), a);
    assert_eq!(near(&mut rt, &format!("A2.GetNearestObjectNearPosition({}, 150, -1, nil, -1, false, 0)", v(200))), st);
    // NearMe never returns the object itself.
    assert_eq!(near(&mut rt, "A2.GetNearestObjectNearMe(a, 500, -1, nil, -1, false)"), b);
    assert_eq!(near(&mut rt, "A2.GetNearestObjectNearMe(a, 50, -1, nil, -1, false)"), 0);
    assert_eq!(near(&mut rt, "A2.GetNearestObjectNearMe(999999, 50, -1, nil, -1, false)"), 0);
    // A mask other than -1 is not measured.
    assert!(rt.eval::<()>("A2.GetNearestObjectNearMe(a, 50, -1, nil, 3, false)").is_err());
    assert!(rt.eval::<()>(&format!("A2.GetNearestObjectNearPosition({}, 50, -1, nil, 3, false, 0)", v(0))).is_err());
    assert_eq!(unsupported(&rt, "class-bit mask"), 2);
    // Shapes.
    assert!(rt.eval::<()>("A2.GetNearestObjectNearMe(a, 50, -1, nil, -1)").is_err());
    assert!(rt.eval::<()>("A2.GetNearestObjectNearPosition(1, 50, -1, nil, -1, false, 0)").is_err());
    assert!(rt.take_commands().is_empty());
}

#[test]
fn the_perceived_flag_also_requires_the_perceived_team() {
    let s = scene();
    let mut sim = Sim::from_setup(&game(&s.db));
    let spawn = Command {
        player: PlayerId::SCRIPT,
        seq: 1,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: s.db.key("fx_cruiser").unwrap(),
            player: PlayerId(0),
            pos: Vec3::from_f32([0.0, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    };
    let first = sim.step(&[spawn]);
    let id = first.view.units[0].id;
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*first.view).clone()), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("p.lua", "", 3, cfg);
    rt.eval::<()>(&format!("u = {}", id.0)).unwrap();
    // Real team 0, perceived 0: matches team 0 either way, never team 1.
    let q = |rt: &mut MissionRuntime, team: i32, perceived: bool| {
        rt.eval::<i64>(&format!("return A2.GetNearestObjectNearPosition(Vector(0,0,0), -1, {team}, nil, -1, {perceived}, 0)")).unwrap()
    };
    assert_eq!(q(&mut rt, 0, false), i64::from(id.0));
    assert_eq!(q(&mut rt, 0, true), i64::from(id.0));
    rt.eval::<()>("A2.SetPerceivedTeam(u, 1)").unwrap();
    let out = sim.step(&rt.take_commands());
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert_eq!(q(&mut rt, 0, false), i64::from(id.0));
    assert_eq!(q(&mut rt, 0, true), 0, "perceived team differs");
    assert_eq!(q(&mut rt, 1, true), 0, "real team differs");
}

#[test]
fn first_craft_is_the_first_live_unit_or_zero() {
    let s = scene();
    let rt = runtime(&s);
    assert_eq!(rt.eval::<i64>("return A2.GetFirstCraft()").unwrap(), i64::from(s.ids[0].0));
    assert!(rt.eval::<()>("A2.GetFirstCraft(1)").is_err());
    let sim = Sim::from_setup(&game(&s.db));
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let empty = MissionRuntime::from_source_with("e.lua", "", 3, cfg);
    assert_eq!(empty.eval::<i64>("return A2.GetFirstCraft()").unwrap(), 0);
}

#[test]
fn path_locations_read_the_host_path_table() {
    let s = scene();
    let rt = runtime(&s);
    assert_eq!(rt.eval::<(f64, f64)>("local p = A2.GetLocation('P', 1); return p.x, p.z").unwrap(), (20.0, 0.0));
    // The handle form is unchanged.
    assert_eq!(rt.eval::<f64>("return A2.GetLocation(b).x").unwrap(), 100.0);
    // LUA-18: an absent path or point (the engine's lookup or bounds test fails): the default vector.
    for call in ["A2.GetLocation('P', 2)", "A2.GetLocation('P', -1)", "A2.GetLocation('NOPE', 0)"] {
        let v: (f64, f64, f64) = rt.eval(&format!("local p = {call}; return p.x, p.y, p.z")).unwrap();
        assert_eq!(v, (0.0, 0.0, 0.0), "{call}");
    }
    assert_eq!(unsupported(&rt, "A2.GetLocation"), 0);
    let noted: Vec<String> = rt.absent_paths().into_iter().map(|(k, _)| k).collect();
    assert!(noted.contains(&"absent path 'NOPE' (A2.GetLocation)".to_owned()), "{noted:?}");
    assert!(noted.contains(&"path 'P' point 2 out of range (A2.GetLocation)".to_owned()), "{noted:?}");
    // a path of no points is still a host gap
    rt.eval::<()>("A2.paths.E = {}").unwrap();
    assert!(rt.eval::<()>("A2.GetLocation('E', 0)").is_err());
    assert_eq!(unsupported(&rt, "has no points"), 1);
    // BuildObject treats it like any index out of range: 0, as before LUA-18.
    assert_eq!(rt.eval::<i64>("return A2.BuildObject('fx_cruiser', 0, 'E', 0)").unwrap(), 0);
    assert_eq!(unsupported(&rt, "has no points"), 1);
    // A point entry that is not a table is malformed, not out of range.
    rt.eval::<()>("A2.paths.M = { 5 }").unwrap();
    assert!(rt.eval::<()>("A2.GetLocation('M', 0)").is_err());
    assert!(rt.eval::<()>("A2.GetLocation('P', 'x')").is_err());
}

#[test]
fn destroy_critical_explode_and_switches_queue_typed_edits() {
    let s = scene();
    let mut rt = runtime(&s);
    rt.eval::<()>("A2.DestroyCritical(a); A2.ExplodeNow(b); A2.SetCanBeDecommissioned(s, false); A2.DisableShieldGenerator(far, true)")
        .unwrap();
    let kinds: Vec<_> = rt.take_commands().into_iter().map(|c| (c.player, c.kind)).collect();
    let edit = |e| (PlayerId::SCRIPT, CommandKind::Edit(e));
    assert_eq!(kinds, vec![
        edit(Edit::Damage(s.ids[0], Fixed::from_f64(100_000.0))),
        edit(Edit::Kill(s.ids[1])),
        edit(Edit::Decommissionable(s.ids[2], false)),
        edit(Edit::Subsystem(s.ids[3], Subsystem::Shields, false)),
    ]);
    // Absent objects and bad shapes raise without queuing.
    for bad in ["A2.DestroyCritical(999999)", "A2.ExplodeNow()", "A2.SetCanBeDecommissioned(a, 0)", "A2.DisableShieldGenerator(a)", "A2.DestroyCritical(a, 1)"] {
        assert!(rt.eval::<()>(bad).is_err(), "{bad}");
    }
    assert!(rt.take_commands().is_empty());
}

#[test]
fn edits_the_sim_rejects_are_reported_and_counted() {
    let s = scene();
    let mut sim = Sim::from_setup(&game(&s.db));
    let first = sim.step(&[Command {
        player: PlayerId::SCRIPT,
        seq: 1,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: s.db.key("fx_cruiser").unwrap(),
            player: PlayerId(0),
            pos: Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    }]);
    let id = first.view.units[0].id;
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*first.view).clone()), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("rej.lua", "", 3, cfg);
    rt.eval::<()>(&format!("A2.SetCanBeDecommissioned({0}, false); A2.DisableShieldGenerator({0}, true)", id.0)).unwrap();
    for _ in 0..3 {
        advance(&mut rt, &mut sim);
    }
    assert_eq!(unsupported(&rt, "SetCanBeDecommissioned: simulation rejected"), 1);
    // SIM-55: the sim applies the shield generator switch now.
    assert_eq!(unsupported(&rt, "DisableShieldGenerator: simulation rejected"), 0);
}

#[test]
fn life_support_and_sensor_switches_queue_subsystem_edits_the_sim_applies() {
    let s = scene();
    let mut sim = Sim::from_setup(&game(&s.db));
    let first = sim.step(&[Command {
        player: PlayerId::SCRIPT,
        seq: 1,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: s.db.key("fx_cruiser").unwrap(),
            player: PlayerId(0),
            pos: Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    }]);
    let id = first.view.units[0].id;
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*first.view).clone()), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("ls.lua", "", 3, cfg);
    rt.eval::<()>(&format!("A2.DisableLifeSupport({0}, true); A2.DisableSensors({0}, false)", id.0)).unwrap();
    let kinds: Vec<CommandKind> = rt.take_commands().into_iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        vec![
            CommandKind::Edit(Edit::Subsystem(id, Subsystem::LifeSupport, false)),
            CommandKind::Edit(Edit::Subsystem(id, Subsystem::Sensors, true)),
        ]
    );
    rt.eval::<()>(&format!("A2.DisableLifeSupport({0}, true); A2.DisableSensors({0}, true)", id.0)).unwrap();
    for _ in 0..3 {
        advance(&mut rt, &mut sim);
    }
    assert_eq!(unsupported(&rt, "simulation rejected"), 0, "the sim applies both switches");
    for bad in ["A2.DisableLifeSupport(1)", "A2.DisableSensors(1, 0)"] {
        assert!(rt.eval::<()>(bad).is_err(), "{bad}");
    }
}

#[test]
fn sim58_switch_calls_queue_typed_edits_and_ignore_dead_handles() {
    let s = scene();
    let mut rt = runtime(&s);
    let a = s.ids[0];
    rt.eval::<()>("A2.AllowAsTarget(a, 1, false); A2.CraftCanRepair(a, false); A2.ButItCanBeDerelicted(a, true); A2.CraftCanRecrew(a, false); A2.HideObject(a, true); A2.SetName(a, 'Voyager')").unwrap();
    let kinds: Vec<_> = rt.take_commands().into_iter().map(|c| (c.player, c.kind)).collect();
    let edit = |e| (PlayerId::SCRIPT, CommandKind::Edit(e));
    assert_eq!(
        kinds,
        vec![
            edit(Edit::AllowAsTarget { id: a, player: PlayerId(1), allowed: false }),
            edit(Edit::CraftCanRepair(a, false)),
            edit(Edit::DerelictAllowed(a, true)),
            edit(Edit::CanRecrew(a, false)),
            edit(Edit::Hidden(a, true)),
            edit(Edit::Name(a, "Voyager".into())),
        ]
    );
    rt.eval::<()>("A2.AllowAsTarget(9999, 1, false); A2.CraftCanRepair(9999, true); A2.SetName(9999, 'x'); A2.HideObject(9999, true)").unwrap();
    assert!(rt.take_commands().is_empty());
    assert!(rt.eval::<()>("A2.AllowAsTarget(a, 99, false)").is_err());
    assert!(rt.eval::<()>("A2.CraftCanRepair(a, 1)").is_err());
}

#[test]
fn seam_gaps_raise_with_a_reason_and_are_counted() {
    let s = scene();
    let mut rt = runtime(&s);
    let names = [
        "SetPlanetCivPop", "StuffIntoWormhole",
        "LinkWormholes", "SetWormholeDeleteOnTransport", "LoadCargoShip", "GetNearestShipyard",
    ];
    for name in names {
        let err = rt.eval::<()>(&format!("A2.{name}(a, 1)")).expect_err(name).to_string();
        assert!(err.contains(&format!("A2.{name} is not available")), "{name}: {err}");
        assert_eq!(unsupported(&rt, &format!("A2.{name}:")), 1, "{name}");
    }
    assert!(rt.take_commands().is_empty());
}

#[test]
fn unresolved_and_failed_tokens_name_no_object() {
    let s = scene();
    let mut sim = Sim::new(3); // no content: the spawn request is rejected
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("fail.lua", "A2.paths.Q = { {x=1,y=0,z=0} }; h = A2.BuildObject('fx_cruiser', 0, 'Q', 0)", 3, cfg);
    let _ = advance(&mut rt, &mut sim);
    assert!(!rt.eval::<bool>("return A2.Exists(h) or A2.IsAlive(h)").unwrap());
    assert!(rt.eval::<bool>("return A2.DoesNotExist(h)").unwrap());
    assert_eq!(rt.eval::<i64>("return A2.SetSpecialForcesFlag(h, true)").unwrap(), 0);
    assert!(rt.eval::<()>("A2.RemoveObject(h)").is_err());
}

#[test]
fn orders_to_a_pending_or_failed_token_raise_and_are_counted() {
    let s = scene();
    // Pending: the spawn is queued but no tick delivered the receipt.
    let sim = Sim::from_setup(&game(&s.db));
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let rt = MissionRuntime::from_source_with(
        "pending.lua",
        "A2.paths.Q = { {x=1,y=0,z=0} }; h = A2.BuildObject('fx_cruiser', 0, 'Q', 0)",
        3,
        cfg,
    );
    // Calls that need the object itself raise; void orders and setters are held (see the next test).
    let err = rt.eval::<()>("A2.GetImpulseSpeed(h)").expect_err("query").to_string();
    assert!(err.contains("still being built"), "{err}");
    assert!(unsupported(&rt, "still being built") >= 1);
    for call in ["A2.Goto(h, 'Q', false, 0)", "A2.Stop(h, 0)", "A2.SetImpulseSpeed(h, 1.0)", "A2.SetRedAlert(h)"] {
        rt.eval::<()>(call).expect(call);
    }
    // A real but absent handle stays a silent no-op.
    rt.eval::<()>("A2.Stop(123456, 0); A2.SetRedAlert(123456)").unwrap();

    // Failed: the sim has no content, so it rejects the request.
    let mut sim = Sim::new(3);
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with(
        "fail.lua",
        "A2.paths.Q = { {x=1,y=0,z=0} }; h = A2.BuildObject('fx_cruiser', 0, 'Q', 0)",
        3,
        cfg,
    );
    let _ = advance(&mut rt, &mut sim);
    let err = rt.eval::<()>("A2.Stop(h, 0)").expect_err("failed token").to_string();
    assert!(err.contains("spawn failed"), "{err}");
    assert_eq!(unsupported(&rt, "A2.Stop: handle names no object: spawn failed"), 1);
}

#[test]
fn an_order_given_to_a_handle_built_this_tick_is_held_and_replayed_after_the_receipt() {
    // Rule (exe, static): BuildObject returns a usable handle at once. Here the spawn is queued
    // and its receipt is one tick later, so a void order on the pending handle waits for it.
    let s = scene();
    let mut sim = Sim::from_setup(&game(&s.db));
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let src = "A2.paths.Q = { {x=40,y=0,z=0}, {x=50,y=0,z=0} }; h = A2.BuildObject('fx_cruiser', 1, 'Q', 1); \
               A2.PointAt(h, 'Q'); A2.SetAngularVelocity(h, Vector(0, 0.5, 0)); A2.SetImpulseSpeed(h, 2.0)";
    let mut rt = MissionRuntime::from_source_with("held.lua", src, 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 1, "only the spawn request is queued: {commands:?}");
    assert_eq!(unsupported(&rt, "still being built"), 0, "holding is not a failure");
    let out = sim.step(&commands);
    let id = out.events.iter().find_map(|e| match e { SimEvent::ScriptSpawned { id, .. } => Some(*id), _ => None }).unwrap();
    rt.think(&out.view, &out.events, &mut Vec::new());
    // The first A2 call after the receipt replays the held orders, in order.
    assert!(rt.eval::<bool>("return A2.Exists(h)").unwrap());
    let kinds: Vec<_> = rt.take_commands().into_iter().map(|c| c.kind).collect();
    assert!(matches!(kinds[0], CommandKind::Edit(Edit::Heading(i, _)) if i == id), "{kinds:?}");
    assert!(matches!(kinds[1], CommandKind::Edit(Edit::AngularVelocity(i, _)) if i == id), "{kinds:?}");
    assert!(matches!(kinds[2], CommandKind::Edit(Edit::MaxSpeed(i, Some(_))) if i == id), "{kinds:?}");
    assert_eq!(kinds.len(), 3);
    // Nothing is replayed twice.
    assert!(rt.eval::<bool>("return A2.Exists(h)").unwrap());
    assert!(rt.take_commands().is_empty());
}

#[test]
fn an_order_held_for_a_failed_spawn_is_dropped() {
    let s = scene();
    let mut sim = Sim::new(3); // no content: the spawn is rejected
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with(
        "dropped.lua",
        "A2.paths.Q = { {x=40,y=0,z=0}, {x=50,y=0,z=0} }; h = A2.BuildObject('fx_cruiser', 1, 'Q', 1); A2.PointAt(h, 'Q')",
        3,
        cfg,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let _ = advance(&mut rt, &mut sim);
    assert!(!rt.eval::<bool>("return A2.Exists(h)").unwrap());
    assert!(rt.take_commands().is_empty(), "no order for an object that was never built");
}

#[test]
fn path_lookup_prefers_exact_then_a_unique_case_insensitive_match() {
    let s = scene();
    let rt = runtime(&s);
    rt.eval::<()>("A2.paths.Ab = { {x=1,y=0,z=0} }; A2.paths.aB = { {x=2,y=0,z=0} }; A2.paths.Solo = { {x=3,y=0,z=0} }").unwrap();
    // Exact name wins even when another key differs only by case.
    assert_eq!(rt.eval::<f64>("return A2.GetLocation('Ab', 0).x").unwrap(), 1.0);
    assert_eq!(rt.eval::<f64>("return A2.GetLocation('aB', 0).x").unwrap(), 2.0);
    // One case-insensitive match is used, in both the order and object calls.
    assert_eq!(rt.eval::<f64>("return A2.GetLocation('solo', 0).x").unwrap(), 3.0);
    rt.eval::<()>("A2.Goto(a, 'solo', false, 0)").unwrap();
    // Two case-insensitive matches and no exact one: an error, deterministically.
    let err = rt.eval::<()>("A2.GetLocation('AB', 0)").expect_err("ambiguous").to_string();
    assert!(err.contains("ambiguous"), "{err}");
    let err = rt.eval::<()>("A2.Goto(a, 'AB', false, 0)").expect_err("ambiguous").to_string();
    assert!(err.contains("ambiguous"), "{err}");
}

/// SIM-54: `SetName` and `SetRace` are per-unit sim edits.
#[test]
fn set_name_and_set_race_queue_typed_edits() {
    let s = scene();
    let mut rt = runtime(&s);
    rt.eval::<()>("A2.SetName(a, 'Relay Station'); A2.SetRace(a, 'fx')").unwrap();
    let edits: Vec<_> = rt.take_commands().into_iter().filter_map(|c| match c.kind { CommandKind::Edit(e) => Some(e), _ => None }).collect();
    assert_eq!(edits, [Edit::Name(s.ids[0], "Relay Station".into()), Edit::UnitRace(s.ids[0], "fx".into())]);
    assert_eq!(unsupported(&rt, "SetName"), 0);
}

#[test]
fn a_fleet_member_built_this_tick_is_ordered_after_its_receipt() {
    // `fleet:AddShip(BuildObject(..))` then `SetFleetDestination` in one step (the stock shape):
    // the member's order is held with the pending handle and replayed once it is built.
    let s = scene();
    let mut sim = Sim::from_setup(&game(&s.db));
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let src = "A2.paths.Q = { {x=40,y=0,z=0}, {x=50,y=0,z=0} }; f = A2.CreateNewSiegeFleet(); \
               f:AddShip(A2.BuildObject('fx_cruiser', 1, 'Q', 1)); f:SetFleetDestination(Vector(500, 0, 0))";
    let mut rt = MissionRuntime::from_source_with("fleet.lua", src, 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 1, "only the spawn request is queued: {commands:?}");
    let out = sim.step(&commands);
    let id = out.events.iter().find_map(|e| match e { SimEvent::ScriptSpawned { id, .. } => Some(*id), _ => None }).unwrap();
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert!(rt.eval::<bool>("return A2.Exists(f.ships[1])").unwrap());
    let kinds: Vec<_> = rt.take_commands().into_iter().map(|c| c.kind).collect();
    assert!(matches!(&kinds[..], [CommandKind::Order(Order::AttackPoint { units, .. })] if units == &vec![id]), "{kinds:?}");
}

#[test]
fn a_fleet_member_whose_spawn_failed_is_skipped_without_raising() {
    let s = scene();
    let mut sim = Sim::new(3); // no content: the spawn is rejected
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let src = "A2.paths.Q = { {x=40,y=0,z=0} }; f = A2.CreateNewSiegeFleet(); f:AddShip(A2.BuildObject('fx_cruiser', 1, 'Q', 0))";
    let mut rt = MissionRuntime::from_source_with("failfleet.lua", src, 3, cfg);
    let out = sim.step(&rt.take_commands());
    rt.think(&out.view, &out.events, &mut Vec::new());
    rt.eval::<()>("f:SetFleetDestination(Vector(5, 0, 0)); f:AddShip(f.ships[1])").unwrap();
    assert!(rt.take_commands().is_empty());
}
