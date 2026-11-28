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
    // The flag call on a station returns 0 and queues nothing (a Craft only, as in the engine); on
    // a craft it returns the handle.
    assert_eq!(rt.eval::<i64>("return A2.SetSpecialForcesFlag(s, true)").unwrap(), 0);
    assert!(rt.take_commands().is_empty());
    assert_eq!(rt.eval::<i64>("return A2.SetSpecialForcesFlag(a, true)").unwrap(), i64::from(s.ids[0].0));
    rt.eval::<()>("A1.SetSpecialForcesFlag(s, true)").unwrap();
    assert_eq!(rt.take_commands().len(), 1, "only the A2 craft flag was queued");
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
    let mv = Command { player: PlayerId(0), seq: 1, kind: CommandKind::Order(Order::Move { units: vec![s.ids[0]], to: Vec3::from_f32([50.0, 0.0, 0.0]), queue: false, mode: None }) };
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

#[test]
fn a_flag_call_or_recruit_on_a_craft_removed_this_tick_is_no_object_on_both_games() {
    let mut s = scene();
    let mut rt = runtime(&s);
    // The view still shows the craft after `RemoveObject` (the kill applies at the next step), and
    // the sim rejects a flag edit on a unit that is gone by then: the bindings answer as for no
    // object (A2 returns 0), queue no flag, and recruit skips it.
    rt.eval::<()>("A2.RemoveObject(b)").unwrap();
    assert_eq!(rt.eval::<i64>("return A2.SetSpecialForcesFlag(b, true)").unwrap(), 0);
    rt.eval::<()>("A1.SetSpecialForcesFlag(b, true)").unwrap();
    let r: (bool, i64) = rt.eval("return A2.RecruitSpecialForces('fx_cruiser', 1, 0)").unwrap();
    assert_eq!(r, (false, 0));
    let r: (bool, i64) = rt.eval("return A1.RecruitSpecialForces('fx_cruiser', 1, 0)").unwrap();
    assert_eq!(r, (false, 0));
    let commands = rt.take_commands();
    assert!(commands.iter().all(|c| !matches!(c.kind, CommandKind::Edit(Edit::SpecialForces(..)))), "{commands:?}");
    // A craft that was not removed still takes the flag, and the sim accepts it (no rejection).
    assert_eq!(rt.eval::<i64>("return A2.SetSpecialForcesFlag(a, true)").unwrap(), i64::from(s.ids[0].0));
    let flag = rt.take_commands();
    let out = s.sim.step(&flag);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })), "{:?}", out.events);
}

#[test]
fn a_team_number_equal_to_a_live_unflagged_crafts_id_is_not_an_order_refusal_on_a1_attack_main_enemy_base() {
    let s = scene();
    let rt = runtime(&s);
    // A1's `AttackMainEnemyBase(team, bool)` takes a team number first, not a craft: the gate
    // must not read it as the unflagged craft `a` whose id happens to be the same number.
    rt.eval::<()>(&format!("A1.AttackMainEnemyBase({}, true)", s.ids[0].0)).unwrap();
    assert_eq!(gated(&rt, "AttackMainEnemyBase"), 0, "not an order refusal");
    // A2's version is per craft and stays gated.
    rt.eval::<()>("A2.AttackMainEnemyBase(a, true)").unwrap();
    assert_eq!(gated(&rt, "AttackMainEnemyBase"), 1);
}

#[test]
fn a_team_change_queued_after_the_flag_in_the_same_tick_closes_the_gate_again() {
    let s = scene();
    let mut rt = runtime(&s);
    rt.eval::<()>("A2.SetSpecialForcesFlag(a, true); A2.Stop(a, 0)").unwrap();
    assert_eq!(orders(&rt.take_commands()).len(), 1, "flagged: the order goes");
    // The sim applies the edits in order and a team change clears the flag.
    rt.eval::<()>("A2.SetSpecialForcesFlag(a, true); A2.SetRealTeam(a, 1); A2.Stop(a, 0)").unwrap();
    assert!(orders(&rt.take_commands()).is_empty());
    assert_eq!(gated(&rt, "Stop"), 1);
    // A flag set after the team change counts again.
    rt.eval::<()>("A2.SetRealTeam(a, 1); A2.SetSpecialForcesFlag(a, true); A2.Stop(a, 0)").unwrap();
    assert_eq!(orders(&rt.take_commands()).len(), 1);
}

#[test]
fn a1_flag_call_on_an_absent_or_dead_craft_does_nothing_without_raising() {
    let mut s = scene();
    let mut rt = runtime(&s);
    rt.eval::<()>("A1.SetSpecialForcesFlag(987654, true)").unwrap();
    assert!(rt.take_commands().is_empty());
    // A craft the sim has destroyed.
    let kill = Command { player: PlayerId::SCRIPT, seq: 20, kind: CommandKind::Edit(Edit::Kill(s.ids[1])) };
    let out = s.sim.step(&[kill]);
    for _ in 0..5 {
        s.sim.step(&[]);
    }
    let view = s.sim.view();
    assert!(view.unit(s.ids[1]).is_none_or(|u| u.flags.contains(sta_sim::view::UnitFlags::DYING)), "gone or dying");
    rt.think(&s.sim.view(), &out.events, &mut Vec::new());
    rt.eval::<()>("A1.SetSpecialForcesFlag(b, true)").unwrap();
    assert!(rt.take_commands().is_empty());
}

#[test]
fn recruit_skips_a_craft_still_in_production() {
    let s = scene();
    let mut view = (*s.sim.view()).clone();
    for u in &mut view.units {
        if u.id == s.ids[1] {
            u.progress = Some(Fixed::from_f64(0.5));
        }
    }
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some(view), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("sf.lua", "", 3, cfg);
    rt.eval::<()>(&format!("b = {}", s.ids[1].0)).unwrap();
    for call in ["A2.RecruitSpecialForces('fx_cruiser', 1, 0)", "A1.RecruitSpecialForces('fx_cruiser', 1, 0)"] {
        let r: (bool, i64) = rt.eval(&format!("return {call}")).unwrap();
        assert_eq!(r, (false, 0), "{call}: the only cruiser of team 1 is not finished");
    }
    assert!(rt.take_commands().is_empty());
}

mod colonize {
    use super::*;
    use sta_content::{ColonizePodDef, CrewRules, DefRef, Hardpoint, MoveStats, PlanetRole, PopulationRules, Rules, UnitDef, WeaponDef, WeaponKind};

    fn n(v: i32) -> Fixed {
        Fixed::from_int(v)
    }

    fn content() -> Arc<ContentDb> {
        let fake = fake::db();
        let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for w in fake.weapons().cloned() {
            b = b.weapon(w);
        }
        for u in fake.units().cloned() {
            b = b.unit(u);
        }
        let mut pod = WeaponDef::new("fx_pod", "colonizerpod", WeaponKind::Projectile);
        pod.refire = 150;
        pod.colonize = Some(ColonizePodDef { party_size: n(100), orbit_range: n(150) });
        let mut planet = UnitDef::new("fx_pl", "planet");
        planet.planet_role = Some(PlanetRole::Producer);
        planet.planet_colonizable = true;
        planet.planet_max_population = 1;
        planet.planet_civilian_population_modifier = Fixed::ONE;
        planet.planet_crew_gain = 1000;
        planet.hull = n(10_000);
        planet.radius = n(100);
        let mut ship = UnitDef::new("fx_colony", "colony");
        ship.race = "fx".into();
        ship.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
        ship.hull = n(250);
        ship.radius = n(10);
        ship.hardpoints = vec![Hardpoint { weapon: DefRef::new("fx_pod"), node: "hp01".into(), arc: None }];
        let rules = Rules::default()
            .with_population(PopulationRules { levels: [n(1000), n(2000), n(3000), n(5000)], civilian_rate_modifier: Fixed::from_f64(0.2) })
            .with_crew(CrewRules { starting_capacity: Some(250), max_crew: None, races: vec![("fx".into(), n(4))] });
        Arc::new(b.weapon(pod).unit(planet).unit(ship).build().unwrap().with_rules(rules))
    }

    #[test]
    fn a_flagged_a2_colonize_reaches_the_sim_and_is_accepted_and_an_unflagged_one_is_refused_by_the_gate() {
        let db = content();
        let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Human };
        let mut sim = Sim::from_setup(&GameSetup {
            seed: 1,
            map: Arc::new(MapDef::empty()),
            content: db.clone(),
            players: vec![seat(0), seat(1)],
            options: GameOptions::default(),
        });
        let spawn = |seq, odf: &str, x: f32| Command {
            player: PlayerId::BENCH,
            seq,
            kind: CommandKind::Edit(Edit::Spawn {
                odf: db.key(odf).unwrap(),
                player: PlayerId(0),
                pos: Vec3::from_f32([x, 0.0, 0.0]),
                heading: (Fixed::ZERO, Fixed::ONE),
            }),
        };
        let out = sim.step(&[spawn(1, "fx_pl", 0.0), spawn(2, "fx_colony", 300.0)]);
        let at = |x: f32| out.view.units.iter().find(|u| u.pos == Vec3::from_f32([x, 0.0, 0.0])).unwrap().id;
        let (planet, ship) = (at(0.0), at(300.0));
        let cfg = MissionConfig { content: Some(db.clone()), view: Some((*out.view).clone()), ..MissionConfig::default() };
        let mut rt = MissionRuntime::from_source_with("sf.lua", "", 3, cfg);
        rt.eval::<()>(&format!("p, c = {}, {}", planet.0, ship.0)).unwrap();
        // Unflagged: the gate swallows it.
        rt.eval::<()>("A2.Colonize(c, p)").unwrap();
        assert!(rt.take_commands().is_empty());
        assert_eq!(gated(&rt, "Colonize"), 1);
        // Flagged in the same tick: the order and the flag edit reach the sim, which accepts both.
        rt.eval::<()>("A2.SetSpecialForcesFlag(c, true); A2.Colonize(c, p)").unwrap();
        let commands = rt.take_commands();
        assert!(commands.iter().any(|c| matches!(&c.kind, CommandKind::Order(Order::Colonize { planet: p, .. }) if *p == planet)), "{commands:?}");
        let out = sim.step(&commands);
        assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })), "{:?}", out.events);
        for _ in 0..10 {
            sim.step(&[]);
        }
        assert!(sim.colonies().process(ship).is_some(), "the ship is colonizing the planet");
    }
}
