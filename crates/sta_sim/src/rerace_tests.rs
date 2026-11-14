//! SIM-54: re-race side effects and the team-pool clamps, against `fake::db()`
//! with two extra races. Rules are read from the executables (static);
//! numbers are the ones the rules give, nothing here was measured in game.

use crate::command::{Command, CommandKind, Edit, Order, TeamOp};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, SimEvent, WorldSource};
use sta_content::{ContentDb, CrewRules, MapDef, Rules, SeatRaceRule, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

fn f(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

fn content(max_crew: Option<i64>) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.pay_on_queue = true;
    let mut b = ContentDb::builder().profile(fake.profile()).features(features);
    for w in fake.weapons().cloned() {
        b = b.weapon(w);
    }
    for mut u in fake.units().cloned() {
        if u.odf == "fx_frigate" {
            u.build.cost = vec![n(250), n(100)];
        }
        b = b.unit(u);
    }
    for (odf, race) in [("alt_ship", "alt"), ("borg_ship", "borg")] {
        let mut u = UnitDef::new(odf, "craft");
        u.race = race.into();
        u.hull = n(100);
        b = b.unit(u);
    }
    let mut base = UnitDef::new("fx_base", "starbase");
    base.race = "fx".into();
    base.hull = n(500);
    base.crew_gain = n(3000);
    b = b.unit(base);
    let rules = Rules::default()
        .with_crew(CrewRules { starting_capacity: Some(250), max_crew, races: vec![("fx".into(), n(4))] })
        .with_seat_races(vec![
            // As the race ODF's float: `0.24f` is a hair under 0.24.
            ("alt".into(), SeatRaceRule { officer_multiplier: f(0.24f32 as f64), latinum_scale: Fixed::ONE }),
            ("borg".into(), SeatRaceRule { officer_multiplier: Fixed::ONE, latinum_scale: Fixed::ZERO }),
        ]);
    Arc::new(b.build().unwrap().with_rules(rules))
}

fn game(max_crew: Option<i64>) -> Sim {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Human };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 1,
        map: Arc::new(MapDef::empty()),
        content: content(max_crew),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    sim.economy.set_personnel(P0, Some(n(200)), Some(n(600)));
    sim
}

fn key(sim: &Sim, odf: &str) -> sta_core::ContentKey {
    sim.content().unwrap().key(odf).unwrap()
}

fn spawn(sim: &mut Sim, odf: &str, player: PlayerId) -> EntityId {
    let id = sim.units.next_id();
    let e = Edit::Spawn { odf: key(sim, odf), player, pos: Vec3 { x: n(100), y: Fixed::ZERO, z: n(100) }, heading: (Fixed::ZERO, Fixed::ONE) };
    sim.step(&[Command { player: PlayerId::BENCH, seq: 0, kind: CommandKind::Edit(e) }]);
    assert!(sim.unit(id).is_some());
    id
}

fn script(sim: &mut Sim, e: Edit) -> bool {
    let out = sim.step(&[Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(e) }]);
    !out.events.iter().any(|ev| matches!(ev, SimEvent::Rejected { .. }))
}

fn pool(sim: &Sim) -> Fixed {
    sim.economy.crew_of(P0).unwrap()
}

fn cap(sim: &Sim) -> Fixed {
    sim.build_view().attrs.seats[0].crew_capacity.unwrap()
}

#[test]
fn the_officer_cap_follows_the_seat_race() {
    let mut sim = game(None);
    let cap_of = |sim: &Sim| sim.build_view().players[0].officers[1];
    assert_eq!(cap_of(&sim), n(600), "multiplier 1");
    assert!(script(&mut sim, Edit::PlayerRace(P0, "alt".into())));
    // `_ftol(0.24f * 600)` truncates 143.99999 to 143, as the original does.
    assert_eq!(cap_of(&sim), n(143), "floor(0.24f * 600)");
    assert_eq!(sim.economy.officer_base(P0), Some(n(600)), "the raw limit is kept");
    assert!(script(&mut sim, Edit::MaxOfficers(P0, n(1000))));
    assert_eq!(cap_of(&sim), n(239), "floor(239.99999)");
    assert!(script(&mut sim, Edit::MaxOfficers(P0, n(599))));
    assert_eq!(cap_of(&sim), n(143), "floor(143.76)");
    assert!(script(&mut sim, Edit::PlayerRace(P0, "fx".into())));
    assert_eq!(cap_of(&sim), n(599), "live when the race changes back");
    // The other seat keeps its own race.
    assert_eq!(sim.seat_race(P1), Some("fx"));
}

#[test]
fn the_cap_survives_a_snapshot() {
    let mut sim = game(None);
    script(&mut sim, Edit::PlayerRace(P0, "alt".into()));
    script(&mut sim, Edit::TeamPool(P0, TeamOp::CapacityRaw, n(777)));
    script(&mut sim, Edit::TeamPool(P0, TeamOp::OfficersAddA1, n(5)));
    let back = Sim::from_snapshot(&sim.snapshot(), sim.content().cloned()).unwrap();
    assert_eq!(back.digest(), sim.digest());
    assert_eq!(back.build_view().players[0].officers, sim.build_view().players[0].officers);
    assert_eq!(back.build_view().players[0].officers, [n(5), n(143)]);
    assert_eq!(cap(&back), n(777));
}

#[test]
fn latinum_is_free_for_a_team_of_a_race_the_data_frees_and_follows_a_rerace() {
    let mut sim = game(None);
    sim.step(&[
        Command { player: PlayerId::BENCH, seq: 1, kind: CommandKind::Edit(Edit::Cargo(P0, 0, n(2000))) },
        Command { player: PlayerId::BENCH, seq: 2, kind: CommandKind::Edit(Edit::Cargo(P0, 1, n(500))) },
    ]);
    let yard = spawn(&mut sim, "fx_yard", P0);
    let build = |sim: &mut Sim, seq| {
        let o = Order::Build { builder: yard, what: key(sim, "fx_frigate"), at: None };
        sim.step(&[Command { player: P0, seq, kind: CommandKind::Order(o) }]);
    };
    let latinum = |sim: &Sim| sim.economy.stock_of(P0)[1];
    build(&mut sim, 5);
    assert_eq!(latinum(&sim), n(400), "100 latinum for an ordinary team");
    assert!(script(&mut sim, Edit::PlayerRace(P0, "borg".into())));
    build(&mut sim, 6);
    assert_eq!(latinum(&sim), n(400), "a Borg team pays no latinum");
    assert_eq!(sim.economy.stock_of(P0)[0], n(2000 - 500), "the other costs stay");
    assert!(script(&mut sim, Edit::PlayerRace(P0, "fx".into())));
    build(&mut sim, 7);
    assert_eq!(latinum(&sim), n(300));
}

#[test]
fn a_unit_race_set_by_a_script_survives_a_change_of_team_and_is_inherited_by_what_it_builds() {
    let mut sim = game(None);
    let yard = spawn(&mut sim, "fx_yard", P0);
    spawn(&mut sim, "fx_cruiser", P0); // keeps the seat alive while the yard is away
    sim.step(&[
        Command { player: PlayerId::BENCH, seq: 1, kind: CommandKind::Edit(Edit::Cargo(P0, 0, n(5000))) },
        Command { player: PlayerId::BENCH, seq: 2, kind: CommandKind::Edit(Edit::Cargo(P0, 1, n(500))) },
    ]);
    assert_eq!(sim.unit_race(yard), Some("fx"));
    assert!(!script(&mut sim, Edit::UnitRace(yard, "nosuch".into())), "unknown race refused");
    assert!(script(&mut sim, Edit::UnitRace(yard, "alt".into())));
    assert_eq!(sim.unit_race(yard), Some("alt"));
    // The seat is untouched, and a change of team does not re-race the unit.
    assert_eq!(sim.seat_race(P0), Some("fx"));
    assert!(script(&mut sim, Edit::Team(yard, P1)));
    assert_eq!(sim.unit_race(yard), Some("alt"));
    assert!(script(&mut sim, Edit::Team(yard, P0)));
    // What it builds takes its race; an unchanged yard's products carry no row.
    let o = Order::Build { builder: yard, what: key(&sim, "fx_frigate"), at: None };
    let out = sim.step(&[Command { player: P0, seq: 3, kind: CommandKind::Order(o) }]);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })), "{:?}", out.events);
    assert_eq!(sim.production.queue(yard).len(), 1);
    let mut built = None;
    for _ in 0..600 {
        for e in sim.step(&[]).events {
            if let SimEvent::Built { id, .. } = e {
                built = Some(id);
            }
        }
    }
    let ship = built.expect("built");
    assert_eq!(sim.unit_race(ship), Some("alt"));
    assert_eq!(sim.build_view().attrs.races, vec![(yard, "alt".to_string()), (ship, "alt".to_string())]);
    let plain = spawn(&mut sim, "fx_yard", P0);
    assert_eq!(sim.unit_race(plain), Some("fx"));
}

#[test]
fn a_starbase_adds_crew_at_the_rate_of_its_own_race() {
    let mut sim = game(None);
    let base = spawn(&mut sim, "fx_base", P0);
    for _ in 0..30 * 10 {
        sim.step(&[]);
    }
    let after_ten = pool(&sim);
    assert!((after_ten.to_bits() - n(240).to_bits()).abs() < f(0.5).to_bits(), "{after_ten:?}");
    assert!(script(&mut sim, Edit::UnitRace(base, "alt".into())));
    for _ in 0..30 * 10 {
        sim.step(&[]);
    }
    assert_eq!(pool(&sim), after_ten, "the race alt has no rate");
}

#[test]
fn a2_team_crew_calls_clamp_to_the_capacity_with_no_floor() {
    let mut sim = game(None);
    assert_eq!(cap(&sim), n(250));
    let op = |sim: &mut Sim, op, x| {
        assert!(script(sim, Edit::TeamPool(P0, op, x)));
        pool(sim)
    };
    assert_eq!(op(&mut sim, TeamOp::CrewAddA2, n(100)), n(250), "sum clamped to capacity");
    script(&mut sim, Edit::TeamCrew(P0, n(300)));
    assert_eq!(op(&mut sim, TeamOp::CrewAddA2, n(10)), n(300), "positive add dropped over capacity");
    assert_eq!(op(&mut sim, TeamOp::CrewAddA2, n(-10)), n(290), "negative add applied");
    assert_eq!(op(&mut sim, TeamOp::CrewSetA2, n(100)), n(100), "SetTeamCrew is AddCrew(v - pool)");
    assert_eq!(op(&mut sim, TeamOp::CrewSetA2, n(-50)), n(-50), "no floor");
    assert_eq!(op(&mut sim, TeamOp::CrewSetA2, n(900)), n(250), "set above capacity clamps");
    assert!(!script(&mut sim, Edit::TeamPool(PlayerId(9), TeamOp::CrewAddA2, n(1))), "no such seat");
}

#[test]
fn a1_team_calls_clamp_to_the_maximum() {
    let mut sim = game(None);
    let mut op = |op, x| {
        assert!(script(&mut sim, Edit::TeamPool(P0, op, x)));
        sim.economy.crew_of(P0).unwrap()
    };
    assert_eq!(op(TeamOp::CrewAddA1, n(100)), n(250));
    assert_eq!(op(TeamOp::CrewSetA1, n(1000)), n(250));
    assert_eq!(op(TeamOp::CrewSetA1, n(-30)), n(-30), "no floor");
}

#[test]
fn a1_officer_calls_clamp_to_the_cap_and_have_no_floor() {
    let mut sim = game(None);
    let used = |sim: &Sim| sim.build_view().players[0].officers[0];
    assert!(script(&mut sim, Edit::TeamPool(P0, TeamOp::OfficersAddA1, f(50.9))));
    assert_eq!(used(&sim), n(50), "ftol truncates");
    assert!(script(&mut sim, Edit::TeamPool(P0, TeamOp::OfficersSetA1, n(5000))));
    assert_eq!(used(&sim), n(600));
    assert!(script(&mut sim, Edit::TeamPool(P0, TeamOp::OfficersAddA1, n(-700))));
    assert_eq!(used(&sim), n(-100));
    // The count is an offset on the live census, so a later actor adds to it.
    assert!(!script(&mut sim, Edit::TeamPool(P1, TeamOp::OfficersSetA1, n(5))), "no limit for the seat");
}

#[test]
fn the_raw_capacity_write_shifts_with_later_capacity_changes_and_max_crew_caps_the_derived_part() {
    let mut sim = game(Some(1000));
    assert!(script(&mut sim, Edit::TeamPool(P0, TeamOp::CapacityRaw, n(400))));
    assert_eq!(cap(&sim), n(400), "written raw");
    spawn(&mut sim, "fx_base", P0);
    // 3000 from the base is capped at MAX_CREW 1000, then the script's +150 stays.
    assert_eq!(cap(&sim), n(1150));
    assert!(script(&mut sim, Edit::TeamPool(P0, TeamOp::CapacityRaw, n(5000))), "a raw write is not clamped");
    assert_eq!(cap(&sim), n(5000));
}

#[test]
fn unit_crew_calls_clamp_to_the_units_own_maximum() {
    let mut sim = game(None);
    let id = spawn(&mut sim, "fx_cruiser", P0);
    let crew = |sim: &Sim| sim.unit(id).unwrap().crew.unwrap();
    let max = crew(&sim)[1];
    assert!(max > Fixed::ZERO);
    assert!(script(&mut sim, Edit::UnitCrew(id, n(-5))));
    assert_eq!(crew(&sim)[0], Fixed::ZERO, "negative reads as 0");
    assert!(script(&mut sim, Edit::UnitCrew(id, max + n(100))));
    assert_eq!(crew(&sim)[0], max, "clamped to the instance maximum, not MAX_CREW");
    assert!(script(&mut sim, Edit::MaxCrew(id, n(7))));
    assert_eq!(crew(&sim), [n(7), n(7)], "the crew is clamped down");
    assert!(script(&mut sim, Edit::MaxCrew(id, n(500))));
    assert_eq!(crew(&sim), [n(7), n(500)], "never raised");
    assert!(script(&mut sim, Edit::UnitCrew(id, n(400))));
    assert_eq!(crew(&sim)[0], n(400), "the larger instance maximum now allows it");
}
