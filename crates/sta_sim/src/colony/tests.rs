//! SIM-30: colonization and planet population, against authored definitions.

use std::sync::Arc;

use sta_content::{ColonizePodDef, ContentDb, CrewRules, DefRef, Hardpoint, MapDef, MoveStats, PlanetRole, PopulationRules, Rules, UnitDef, WeaponDef, WeaponKind};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use super::*;
use crate::command::{Command, CommandKind, Edit, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::world::NEUTRAL;
use crate::{Sim, WorldSource};

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

fn at(x: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: Fixed::ZERO, z: n(z) }
}

fn planet(odf: &str, level: u8, civ: f64, colonizable: bool, crew_gain: i32) -> UnitDef {
    let mut p = UnitDef::new(odf, "planet");
    p.planet_role = Some(PlanetRole::Producer);
    p.planet_colonizable = colonizable;
    p.planet_max_population = level;
    p.planet_civilian_population_modifier = Fixed::from_f64(civ);
    p.planet_crew_gain = crew_gain;
    p.hull = n(10_000);
    p.radius = n(100);
    p
}

fn ship(odf: &str, weapon: &str) -> UnitDef {
    let mut s = UnitDef::new(odf, "colony");
    s.race = "fx".into();
    s.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    s.hull = n(250);
    s.radius = n(10);
    s.hardpoints = vec![Hardpoint { weapon: DefRef::new(weapon), node: "hp01".into(), arc: None }];
    s
}

fn content(with_rules: bool) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
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
    let mut beam = WeaponDef::new("fx_colbeam", "colonybm", WeaponKind::Beam);
    beam.refire = 150;
    let mut base = UnitDef::new("fx_starbase", "starbase");
    base.race = "fx".into();
    base.hull = n(500);
    let mut rules = Rules::default();
    if with_rules {
        rules = rules
            .with_population(PopulationRules { levels: [n(1000), n(2000), n(3000), n(5000)], civilian_rate_modifier: Fixed::from_f64(0.2) })
            .with_crew(CrewRules { starting_capacity: Some(250), races: vec![("fx".into(), n(4))] });
    }
    let db = b
        .weapon(pod)
        .weapon(beam)
        .unit(planet("fx_pl_d", 0, 1.0, false, 0))
        .unit(planet("fx_pl_h", 1, 1.0, true, 1000))
        .unit(planet("fx_pl_k", 2, 0.5, true, 2000))
        .unit(planet("fx_pl_m", 4, 1.5, true, 5000))
        .unit(ship("fx_colony", "fx_pod"))
        .unit(ship("fx_borg_colony", "fx_colbeam"))
        .unit(base)
        .build()
        .unwrap();
    Arc::new(db.with_rules(rules))
}

fn game(db: Arc<ContentDb>) -> Sim {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Human };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 1,
        map: Arc::new(MapDef::empty()),
        content: db,
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    sim.economy.set_personnel(PlayerId(0), Some(n(750)), None);
    sim.economy.set_personnel(PlayerId(1), Some(n(750)), None);
    sim
}

fn bench(sim: &mut Sim, kind: CommandKind) -> bool {
    run_cmd(sim, PlayerId::BENCH, kind)
}

fn run_cmd(sim: &mut Sim, player: PlayerId, kind: CommandKind) -> bool {
    let out = sim.step(&[Command { player, seq: 0, kind }]);
    !out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. }))
}

fn spawn(sim: &mut Sim, odf: &str, owner: PlayerId, pos: Vec3) -> EntityId {
    let key = sim.content().unwrap().key(odf).unwrap();
    assert!(bench(sim, CommandKind::Edit(Edit::Spawn { odf: key, player: owner, pos, heading: (Fixed::ZERO, Fixed::ONE) })));
    sim.units.iter().map(|u| u.id).max().unwrap()
}

fn run(sim: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        sim.step(&[]);
    }
}

fn colonize(sim: &mut Sim, player: u8, ship: EntityId, planet: EntityId) -> bool {
    run_cmd(sim, PlayerId(player), CommandKind::Order(Order::Colonize { units: vec![ship], planet }))
}

fn set_pop(sim: &mut Sim, planet: EntityId, people: i32) {
    assert!(run_cmd(sim, PlayerId::SCRIPT, CommandKind::Edit(Edit::PlanetPopulation(planet, n(people)))));
}

fn pop(sim: &Sim, planet: EntityId) -> Fixed {
    sim.planet_population(planet).unwrap()
}

fn owner(sim: &Sim, id: EntityId) -> PlayerId {
    sim.unit(id).unwrap().owner
}

fn close(a: Fixed, b: f64, tolerance: f64) {
    let got = a.to_bits() as f64 / (1u64 << 32) as f64;
    assert!((got - b).abs() <= tolerance, "{got} vs {b}");
}

// ---- the small rules ----

#[test]
fn the_range_counts_the_planet_radius_twice() {
    // 150 + 2 x 100 + 10 - 15 = 345
    assert!(in_colonize_range(n(345), n(150), n(100), n(10)));
    assert!(!in_colonize_range(n(346), n(150), n(100), n(10)));
    let p = approach_point(at(0, 0), at(1000, 0), (Fixed::ONE, Fixed::ZERO), n(100), n(10), n(150));
    // 2 x 100 + 10 + 150 - 100 = 260 from the planet centre, toward the ship.
    assert_eq!((p.x, p.z), (n(260), Fixed::ZERO));
}

#[test]
fn payload_growth_crew_and_cancel_numbers() {
    assert_eq!(pod_payload(n(1000), n(100)), n(100));
    assert_eq!(pod_payload(n(40), n(100)), n(40));
    // (4 x 1000 / 5000 + 1) x 4 x 1.5 x 0.2 x 0.2 = 0.432 per second
    close(growth_per_second(n(1000), n(5000), n(4), Fixed::from_f64(1.5), Fixed::from_f64(0.2)), 0.432, 1e-6);
    close(growth_per_second(n(5000), n(5000), n(4), Fixed::from_f64(1.5), Fixed::from_f64(0.2)), 1.2, 1e-6);
    assert_eq!(growth_per_second(n(500), Fixed::ZERO, n(4), Fixed::ONE, Fixed::ONE), Fixed::ZERO);
    // 1000 / 5000 x 4 x 4 = 3.2 crew per second; 5000 people give 16.
    close(crew_per_second(n(1000), n(5000), n(4)), 3.2, 1e-6);
    close(crew_per_second(n(5000), n(5000), n(4)), 16.0, 1e-6);
    assert_eq!(cancelled_population(n(300), n(700), n(400)), n(300));
    assert_eq!(cancelled_population(n(0), n(700), n(300)), n(400));
}

// ---- the order ----

#[test]
fn a_full_run_takes_ten_pods_over_forty_five_seconds_and_consumes_the_ship() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_h", NEUTRAL, at(0, 0));
    let s = spawn(&mut sim, "fx_colony", PlayerId(0), at(300, 0));
    assert!(colonize(&mut sim, 0, s, planet));
    // The first pod left on the tick of the order; 4 pods later 400 people.
    assert_eq!(pop(&sim, planet), n(100));
    assert_eq!(sim.colonies().row(planet).unwrap().colonizer, Some(s));
    run(&mut sim, 150 * 3);
    assert_eq!(pop(&sim, planet), n(400));
    run(&mut sim, 150 * 6 - 1);
    assert_eq!(pop(&sim, planet), n(900));
    assert!(sim.unit(s).is_some());
    assert_eq!(owner(&sim, planet), NEUTRAL, "unowned until the last pod");
    run(&mut sim, 1);
    assert_eq!(pop(&sim, planet), n(1000));
    assert_eq!(owner(&sim, planet), PlayerId(0));
    assert!(sim.unit(s).is_none(), "consumed");
    assert_eq!(sim.colonies().row(planet).unwrap().colonizer, None);
    assert!(sim.colonies().process(s).is_none());
    // The view shows it.
    let view = sim.step(&[]).view;
    let c = view.colonies.iter().find(|c| c.id == planet).unwrap();
    assert_eq!((c.population, c.max_population, c.colonizer), (n(1000), n(1000), None));
}

#[test]
fn a_ship_out_of_range_travels_there_first() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_h", NEUTRAL, at(0, 0));
    let s = spawn(&mut sim, "fx_colony", PlayerId(0), at(3000, 0));
    assert!(colonize(&mut sim, 0, s, planet));
    assert!(sim.colonies().row(planet).is_none(), "not a colonizer yet");
    run(&mut sim, 30 * 10);
    assert!(sim.unit(s).unwrap().pos.x < n(3000));
    run(&mut sim, 30 * 60);
    assert!(sim.colonies().row(planet).is_some());
    run(&mut sim, 30 * 50);
    assert_eq!(owner(&sim, planet), PlayerId(0));
    assert_eq!(pop(&sim, planet), n(1000));
    assert!(sim.unit(s).is_none());
}

#[test]
fn colonize_is_rejected_for_the_wrong_ship_planet_or_owner() {
    let mut sim = game(content(true));
    let sparse = spawn(&mut sim, "fx_pl_h", NEUTRAL, at(0, 0));
    let dead = spawn(&mut sim, "fx_pl_d", NEUTRAL, at(2000, 0));
    let theirs = spawn(&mut sim, "fx_pl_k", PlayerId(1), at(-2000, 0));
    let mine = spawn(&mut sim, "fx_colony", PlayerId(0), at(300, 100));
    let other = spawn(&mut sim, "fx_colony", PlayerId(1), at(300, -100));
    let borg = spawn(&mut sim, "fx_borg_colony", PlayerId(0), at(300, 200));
    let cruiser = spawn(&mut sim, "fx_cruiser", PlayerId(0), at(300, 300));
    assert!(!colonize(&mut sim, 0, mine, dead), "not colonizable");
    assert!(!colonize(&mut sim, 0, mine, theirs), "owned by another player");
    assert!(!colonize(&mut sim, 0, other, sparse), "another player's ship");
    assert!(!colonize(&mut sim, 0, borg, sparse), "Borg beam ships are not implemented");
    assert!(!colonize(&mut sim, 0, cruiser, sparse), "not a colony ship");
    assert!(!colonize(&mut sim, 0, mine, mine), "not a planet");
    assert!(!run_cmd(&mut sim, PlayerId(0), CommandKind::Order(Order::Colonize { units: vec![mine], planet: EntityId(9999) })));
    assert!(colonize(&mut sim, 0, mine, sparse));
    assert!(!colonize(&mut sim, 1, other, sparse), "the planet is already being colonized");
    assert!(sim.colonies().process(other).is_none());
    // Without the population rules nothing colonizes.
    let mut off = game(content(false));
    let p = spawn(&mut off, "fx_pl_h", NEUTRAL, at(0, 0));
    let s = spawn(&mut off, "fx_colony", PlayerId(0), at(300, 0));
    assert!(!colonize(&mut off, 0, s, p));
}

#[test]
fn a_ship_may_colonize_its_own_planet_but_a_script_needs_the_special_forces_flag() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_k", PlayerId(0), at(0, 0));
    set_pop(&mut sim, planet, 500);
    let s = spawn(&mut sim, "fx_colony", PlayerId(0), at(300, 0));
    let script = |sim: &mut Sim| run_cmd(sim, PlayerId::SCRIPT, CommandKind::Order(Order::Colonize { units: vec![s], planet }));
    assert!(!script(&mut sim), "no special-forces flag");
    assert!(run_cmd(&mut sim, PlayerId::SCRIPT, CommandKind::Edit(Edit::SpecialForces(s, true))));
    assert!(script(&mut sim));
    run(&mut sim, 150 * 5);
    // Light cap 2000: 500 + 6 pods of 100 so far.
    close(pop(&sim, planet), 1100.0, 2.0);
    assert_eq!(owner(&sim, planet), PlayerId(0));
}

#[test]
fn a_second_ship_in_range_is_stopped_by_the_first() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_h", NEUTRAL, at(0, 0));
    let a = spawn(&mut sim, "fx_colony", PlayerId(0), at(300, 100));
    let b = spawn(&mut sim, "fx_colony", PlayerId(0), at(300, -100));
    assert!(run_cmd(&mut sim, PlayerId(0), CommandKind::Order(Order::Colonize { units: vec![a, b], planet })));
    assert_eq!(sim.colonies().row(planet).unwrap().colonizer, Some(a));
    assert!(sim.colonies().process(b).is_none());
    assert!(sim.unit(b).is_some());
}

// ---- interrupts ----

#[test]
fn stop_takes_back_the_colonists_launched_but_not_below_the_start() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_h", NEUTRAL, at(0, 0));
    set_pop(&mut sim, planet, 300);
    let s = spawn(&mut sim, "fx_colony", PlayerId(0), at(300, 0));
    assert!(colonize(&mut sim, 0, s, planet));
    run(&mut sim, 150 * 3);
    assert_eq!(pop(&sim, planet), n(700));
    assert!(run_cmd(&mut sim, PlayerId(0), CommandKind::Order(Order::Stop { units: vec![s] })));
    assert_eq!(pop(&sim, planet), n(300));
    assert_eq!(sim.colonies().row(planet).unwrap().colonizer, None);
    assert!(sim.colonies().process(s).is_none());
    assert!(sim.unit(s).is_some());
    run(&mut sim, 150 * 3);
    assert_eq!(pop(&sim, planet), n(300), "no more pods");
    assert_eq!(owner(&sim, planet), NEUTRAL);
}

#[test]
fn stop_with_a_low_start_keeps_what_the_pods_cannot_explain() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_h", NEUTRAL, at(0, 0));
    let s = spawn(&mut sim, "fx_colony", PlayerId(0), at(300, 0));
    assert!(colonize(&mut sim, 0, s, planet));
    run(&mut sim, 150 * 2);
    // A script adds 250 people while the pods run: 300 + 250 - 300 launched.
    set_pop(&mut sim, planet, 550);
    assert!(run_cmd(&mut sim, PlayerId(0), CommandKind::Order(Order::Stop { units: vec![s] })));
    assert_eq!(pop(&sim, planet), n(250));
}

#[test]
fn another_order_leaves_the_landed_colonists_and_frees_the_planet() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_h", NEUTRAL, at(0, 0));
    let s = spawn(&mut sim, "fx_colony", PlayerId(0), at(300, 0));
    assert!(colonize(&mut sim, 0, s, planet));
    run(&mut sim, 150 * 3);
    assert!(run_cmd(&mut sim, PlayerId(0), CommandKind::Order(Order::Move { units: vec![s], to: at(2000, 2000), queue: false })));
    assert_eq!(pop(&sim, planet), n(400));
    assert_eq!(sim.colonies().row(planet).unwrap().colonizer, None);
    assert!(sim.colonies().process(s).is_none());
    // A queued move does not replace the colonization.
    let t = spawn(&mut sim, "fx_colony", PlayerId(1), at(-300, 0));
    assert!(colonize(&mut sim, 1, t, planet));
    assert!(run_cmd(&mut sim, PlayerId(1), CommandKind::Order(Order::Move { units: vec![t], to: at(-2000, 0), queue: true })));
    assert!(sim.colonies().process(t).is_some());
}

#[test]
fn a_dying_ship_clears_the_colonizer_without_rolling_back() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_h", NEUTRAL, at(0, 0));
    let s = spawn(&mut sim, "fx_colony", PlayerId(0), at(300, 0));
    assert!(colonize(&mut sim, 0, s, planet));
    run(&mut sim, 150 * 2);
    assert!(bench(&mut sim, CommandKind::Edit(Edit::Kill(s))));
    assert_eq!(pop(&sim, planet), n(300));
    assert_eq!(sim.colonies().row(planet).unwrap().colonizer, None);
    assert!(sim.colonies().process(s).is_none());
    assert_eq!(owner(&sim, planet), NEUTRAL);
    // Another ship may now finish it.
    let t = spawn(&mut sim, "fx_colony", PlayerId(1), at(-300, 0));
    assert!(colonize(&mut sim, 1, t, planet));
    run(&mut sim, 150 * 8);
    assert_eq!((owner(&sim, planet), pop(&sim, planet)), (PlayerId(1), n(1000)));
}

// ---- growth, crew, neutralisation ----

#[test]
fn a_heavy_planet_grows_by_the_law() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_m", PlayerId(0), at(0, 0));
    set_pop(&mut sim, planet, 1000);
    let start = pop(&sim, planet);
    run(&mut sim, 30 * 60);
    // dP/dt = (4P/5000 + 1) x 4 x 1.5 x 0.2 x 0.2 = 0.24 + 0.000192 P, per tick (1/30 s) Euler.
    let mut p = 1000.0f64;
    for _ in 0..30 * 60 + 1 {
        p += (4.0 * p / 5000.0 + 1.0) * 4.0 * 1.5 * 0.2 * 0.2 / 30.0;
    }
    close(pop(&sim, planet), p, 0.01);
    assert!(pop(&sim, planet) > start);
}

#[test]
fn growth_stops_at_the_cap_and_cuts_a_script_value_above_it() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_m", PlayerId(0), at(0, 0));
    set_pop(&mut sim, planet, 4999);
    run(&mut sim, 30 * 10);
    assert_eq!(pop(&sim, planet), n(5000));
    set_pop(&mut sim, planet, 9000);
    assert_eq!(pop(&sim, planet), n(5000), "cut on the tick");
    let k = spawn(&mut sim, "fx_pl_k", PlayerId(0), at(5000, 0));
    set_pop(&mut sim, k, 3000);
    assert_eq!(pop(&sim, k), n(2000));
    let d = spawn(&mut sim, "fx_pl_d", PlayerId(0), at(-5000, 0));
    set_pop(&mut sim, d, 800);
    run(&mut sim, 30);
    assert_eq!(pop(&sim, d), Fixed::ZERO, "a class that holds nobody");
}

#[test]
fn a_planet_below_the_sparse_level_or_without_an_owner_does_not_grow() {
    let mut sim = game(content(true));
    let low = spawn(&mut sim, "fx_pl_m", PlayerId(0), at(0, 0));
    let free = spawn(&mut sim, "fx_pl_m", NEUTRAL, at(4000, 0));
    set_pop(&mut sim, low, 999);
    set_pop(&mut sim, free, 3000);
    run(&mut sim, 30 * 30);
    assert_eq!(pop(&sim, low), n(999));
    assert_eq!(pop(&sim, free), n(3000));
}

#[test]
fn people_give_crew_at_the_heavy_proportion() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_m", PlayerId(0), at(0, 0));
    set_pop(&mut sim, planet, 1000);
    let before = sim.economy.crew_of(PlayerId(0)).unwrap();
    run(&mut sim, 30 * 10);
    let gained = sim.economy.crew_of(PlayerId(0)).unwrap() - before;
    // 1000 / 5000 x 4 x 4 = 3.2 per second (the people grow by about 4 meanwhile).
    close(gained, 32.0, 0.3);
    assert_eq!(sim.economy.crew_of(PlayerId(1)).unwrap(), n(750), "another seat gains nothing");
    // A planet nobody owns feeds nobody.
    let free = spawn(&mut sim, "fx_pl_m", NEUTRAL, at(4000, 0));
    set_pop(&mut sim, free, 5000);
    let before = sim.economy.crew_of(PlayerId(1)).unwrap();
    run(&mut sim, 30 * 5);
    assert_eq!(sim.economy.crew_of(PlayerId(1)).unwrap(), before);
}

#[test]
fn an_owned_planet_without_people_goes_neutral_and_keeps_its_row() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_m", PlayerId(0), at(0, 0));
    let bare = spawn(&mut sim, "fx_pl_m", PlayerId(0), at(4000, 0));
    run(&mut sim, 5);
    assert_eq!(owner(&sim, bare), PlayerId(0), "no row: population 0 but not neutralised");
    set_pop(&mut sim, planet, 0);
    run(&mut sim, 2);
    assert_eq!(owner(&sim, planet), NEUTRAL);
    assert_eq!(pop(&sim, planet), Fixed::ZERO);
    // A negative value is stored raw, then floored by the next tick.
    let other = spawn(&mut sim, "fx_pl_m", PlayerId(1), at(-4000, 0));
    assert!(run_cmd(&mut sim, PlayerId::SCRIPT, CommandKind::Edit(Edit::PlanetPopulation(other, n(-5)))));
    run(&mut sim, 2);
    assert_eq!((owner(&sim, other), pop(&sim, other)), (NEUTRAL, Fixed::ZERO));
}

#[test]
fn planet_population_edit_takes_producer_planets_from_scripts_and_the_bench_only() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_m", NEUTRAL, at(0, 0));
    let s = spawn(&mut sim, "fx_colony", PlayerId(0), at(300, 0));
    let cruiser = spawn(&mut sim, "fx_cruiser", PlayerId(0), at(900, 0));
    assert!(!run_cmd(&mut sim, PlayerId(0), CommandKind::Edit(Edit::PlanetPopulation(planet, n(10)))), "a player may not");
    assert!(!run_cmd(&mut sim, PlayerId::SCRIPT, CommandKind::Edit(Edit::PlanetPopulation(s, n(10)))));
    assert!(!run_cmd(&mut sim, PlayerId::SCRIPT, CommandKind::Edit(Edit::PlanetPopulation(cruiser, n(10)))));
    assert!(!run_cmd(&mut sim, PlayerId::SCRIPT, CommandKind::Edit(Edit::PlanetPopulation(EntityId(777), n(10)))));
    assert!(sim.colonies().row(planet).is_none());
    assert!(run_cmd(&mut sim, PlayerId::BENCH, CommandKind::Edit(Edit::PlanetPopulation(planet, n(1234)))));
    assert_eq!(pop(&sim, planet), n(1234));
    assert!(run_cmd(&mut sim, PlayerId::SCRIPT, CommandKind::Edit(Edit::PlanetPopulation(planet, Fixed::from_int(i32::MAX)))));
    assert_eq!(pop(&sim, planet), n(5000), "clamped to 1e9, then cut to the cap");
}

#[test]
fn weapon_damage_costs_people_and_spares_the_hull_of_a_planet_with_a_row() {
    let mut sim = game(content(true));
    let planet = spawn(&mut sim, "fx_pl_m", PlayerId(1), at(0, 0));
    let bare = spawn(&mut sim, "fx_pl_k", PlayerId(1), at(4000, 0));
    set_pop(&mut sim, planet, 1000);
    let hit = |target| SimEvent::Hit {
        target,
        by: EntityId(999),
        weapon: sta_core::ContentKey(0),
        facing: 0,
        shield: Fixed::ZERO,
        hull: n(200),
        at: Vec3::ZERO,
    };
    sim.events.push(hit(planet));
    sim.events.push(hit(bare));
    let db = sim.content().cloned().unwrap();
    let killed = sim.with_planet_population_loss(0, |sim| crate::combat::damage::apply(&mut sim.units, &db, &mut sim.events, 0));
    assert!(killed.is_empty());
    close(pop(&sim, planet), 980.0, 0.05);
    assert_eq!(sim.unit(planet).unwrap().hull[0], n(10_000), "hull untouched");
    assert_eq!(sim.unit(bare).unwrap().hull[0], n(9_800), "a planet without a row takes hull damage as before");
}

// ---- persistence ----

#[test]
fn snapshot_round_trip_mid_colonization_continues_identically() {
    let db = content(true);
    let mut live = game(Arc::clone(&db));
    let planet = spawn(&mut live, "fx_pl_h", NEUTRAL, at(0, 0));
    let other = spawn(&mut live, "fx_pl_m", PlayerId(1), at(4000, 0));
    set_pop(&mut live, other, 2500);
    let s = spawn(&mut live, "fx_colony", PlayerId(0), at(300, 0));
    assert!(colonize(&mut live, 0, s, planet));
    run(&mut live, 77 + 150 * 2);
    let mut copy = Sim::from_snapshot(&live.snapshot(), Some(Arc::clone(&db))).unwrap();
    assert_eq!(copy.digest(), live.digest());
    assert_eq!(copy.colonies(), live.colonies());
    for _ in 0..200 {
        live.step(&[]);
        copy.step(&[]);
        assert_eq!(copy.digest(), live.digest());
    }
    assert_eq!(pop(&copy, planet), pop(&live, planet));
    assert!(pop(&live, planet) > n(300));
    assert!(copy.snapshot() == live.snapshot());
}

#[test]
fn a_snapshot_with_a_dangling_colony_row_is_refused() {
    let db = content(true);
    let mut sim = game(Arc::clone(&db));
    let planet = spawn(&mut sim, "fx_pl_h", NEUTRAL, at(0, 0));
    set_pop(&mut sim, planet, 10);
    let mut bytes = sim.snapshot();
    // The colony record is the last: rewrite its planet id to one that does not exist.
    let rec = sim.colonies().to_rec();
    let good = postcard::to_allocvec(&rec).unwrap();
    assert!(bytes.ends_with(&good));
    let mut bad = rec;
    bad.planets[0].0 = EntityId(4000);
    let cut = bytes.len() - good.len();
    bytes.truncate(cut);
    bytes.extend(postcard::to_allocvec(&bad).unwrap());
    assert!(Sim::from_snapshot(&bytes, Some(db)).is_err());
}

#[test]
fn two_runs_agree_and_colony_state_reaches_the_digest() {
    let run_once = || {
        let mut sim = game(content(true));
        let planet = spawn(&mut sim, "fx_pl_h", NEUTRAL, at(0, 0));
        let s = spawn(&mut sim, "fx_colony", PlayerId(0), at(300, 0));
        assert!(colonize(&mut sim, 0, s, planet));
        run(&mut sim, 500);
        sim
    };
    let (a, b) = (run_once(), run_once());
    assert_eq!(a.digest(), b.digest());
    let mut c = run_once();
    let planet = *a.colonies().planets.keys().next().unwrap();
    set_pop(&mut c, planet, 1);
    assert_ne!(a.digest(), c.digest());
    // No colony state, no change to the digest of the same world.
    let plain = game(content(true));
    assert!(plain.colonies().is_empty());
}
