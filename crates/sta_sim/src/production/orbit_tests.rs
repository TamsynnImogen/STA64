//! SIM-62: orbital stations snap to a planet's orbit slot, one per planet.
use super::*;
use crate::command::{Command, CommandKind, Edit, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, MoveStats, UnitDef};
use sta_core::TeamId;
use std::sync::Arc;

const P0: PlayerId = PlayerId(0);
const RADIUS: i32 = 200;

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn content() -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.pay_on_queue = true;
    let mut rig = UnitDef::new("fx_rig", "constructionrig");
    rig.race = "fx".into();
    rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.hull = Fixed::from_int(100);
    // An orbital station: class p_mining, one orbit in 120 s.
    let mut orbital = UnitDef::new("fx_orbital", "p_mining");
    orbital.race = "fx".into();
    orbital.hull = Fixed::from_int(200);
    orbital.build.cost = vec![Fixed::from_int(100)];
    orbital.build.ticks = 60;
    orbital.orbit_time = Some(Fixed::from_int(120));
    // A planet with a slot, and a moon without one.
    let mut world = UnitDef::new("fx_world", "planet");
    world.radius = Fixed::from_int(RADIUS);
    world.resource = Some(2);
    world.can_have_orbital_station = true;
    let mut moon = UnitDef::new("fx_rock", "planet");
    moon.radius = Fixed::from_int(40);
    moon.resource = Some(0);
    let mut b = ContentDb::builder().profile(fake.profile()).features(features).unit(rig).unit(orbital).unit(world).unit(moon);
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    Arc::new(b.build().unwrap())
}

fn game() -> Sim {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Human };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: content(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let e = Edit::Cargo(P0, 0, Fixed::from_int(5000));
    sim.step(&[Command { player: PlayerId::BENCH, seq: 1, kind: CommandKind::Edit(e) }]);
    sim
}

fn spawn(sim: &mut Sim, odf: &str, at: Vec3) -> EntityId {
    let odf = sim.content().unwrap().key(odf).unwrap();
    let id = sim.units.next_id();
    let e = Edit::Spawn { odf, player: P0, pos: at, heading: (Fixed::ZERO, Fixed::ONE) };
    sim.step(&[Command { player: PlayerId::BENCH, seq: 0, kind: CommandKind::Edit(e) }]);
    assert!(sim.unit(id).is_some());
    id
}

fn build(sim: &mut Sim, rig: EntityId, at: Vec3) -> bool {
    let what = sim.content().unwrap().key("fx_orbital").unwrap();
    let o = Order::Build { builder: rig, what, at: Some(at) };
    let out = sim.step(&[Command { player: P0, seq: 7, kind: CommandKind::Order(o) }]);
    !out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. }))
}

fn dist(a: Vec3, b: Vec3) -> f64 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    f64::from((dx * dx + dz * dz).sqrt().to_f32())
}

fn orbitals(sim: &Sim) -> Vec<(Vec3, (Fixed, Fixed))> {
    let key = sim.content().unwrap().key("fx_orbital").unwrap();
    sim.view().units.iter().filter(|u| u.def == key).map(|u| (u.pos, u.heading)).collect()
}

#[test]
fn a_cursor_over_a_planet_snaps_to_its_orbit_and_the_station_faces_outward() {
    let mut sim = game();
    let world = v(1000, 0);
    spawn(&mut sim, "fx_world", world);
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    // On the planet's side nearest the rig, 40 units from the centre: inside it.
    assert!(build(&mut sim, rig, v(960, 0)));
    for _ in 0..900 {
        sim.step(&[]);
    }
    let all = orbitals(&sim);
    assert_eq!(all.len(), 1, "built");
    let (pos, heading) = all[0];
    let ring = (RADIUS + orbit::ALTITUDE) as f64;
    assert!((dist(pos, world) - ring).abs() < 0.01, "on the orbit: {}", dist(pos, world));
    // The bearing of the cursor from the centre: -x.
    assert!(pos.x < world.x && pos.z.abs() < Fixed::ONE);
    assert!(heading.0.to_f32() < -0.99 && heading.1.to_f32().abs() < 0.01, "outward: {heading:?}");
}

#[test]
fn the_reach_is_radius_plus_85_and_open_space_is_refused() {
    let reach = RADIUS + orbit::REACH;
    for (at, ok) in [
        (v(1000 - (reach - 1), 0), true),
        (v(1000 - reach, 0), false),
        (v(1000, 3000), false),
        (v(3000, 3000), false),
        (v(0, 0), false),
    ] {
        let mut sim = game();
        spawn(&mut sim, "fx_world", v(1000, 0));
        let rig = spawn(&mut sim, "fx_rig", v(0, 0));
        assert_eq!(build(&mut sim, rig, at), ok, "{at:?}");
    }
}

#[test]
fn a_planet_takes_one_station_and_a_moon_none() {
    let mut sim = game();
    let a = v(1000, 0);
    spawn(&mut sim, "fx_world", a);
    spawn(&mut sim, "fx_world", v(0, 1500));
    spawn(&mut sim, "fx_rock", v(-1000, 0));
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    let rig2 = spawn(&mut sim, "fx_rig", v(0, 100));
    assert!(build(&mut sim, rig, v(900, 0)));
    // The slot is claimed while the rig is still on its way.
    assert!(!build(&mut sim, rig2, v(1100, 0)), "same planet, other bearing");
    // Another planet is free, a moon has no slot.
    assert!(build(&mut sim, rig2, v(0, 1400)));
    assert!(!build(&mut sim, rig2, v(-1000, 0)), "a moon has no slot");
    for _ in 0..1500 {
        sim.step(&[]);
    }
    assert_eq!(orbitals(&sim).len(), 1, "the second rig's order was replaced by the refused one");
}

#[test]
fn a_finished_station_keeps_its_planet_taken() {
    let mut sim = game();
    spawn(&mut sim, "fx_world", v(1000, 0));
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    assert!(build(&mut sim, rig, v(900, 0)));
    for _ in 0..900 {
        sim.step(&[]);
    }
    assert_eq!(orbitals(&sim).len(), 1);
    assert!(!build(&mut sim, rig, v(1100, 0)), "taken once built");
    assert!(!build(&mut sim, rig, v(1000, 0)), "also on the centre");
}

#[test]
fn a_cursor_on_the_centre_takes_bearing_zero() {
    let p = orbit::Planet { id: EntityId(1), pos: v(10, 20), radius: Fixed::from_int(RADIUS) };
    let s = orbit::slot(&p, v(10, 20));
    assert_eq!(s.heading, (Fixed::ZERO, Fixed::ONE));
    assert_eq!(s.at, v(10, 20 + RADIUS + orbit::ALTITUDE));
}

#[test]
fn the_nearest_surface_wins_where_two_planets_reach_the_cursor() {
    let near = orbit::Planet { id: EntityId(5), pos: v(0, 0), radius: Fixed::from_int(100) };
    let far = orbit::Planet { id: EntityId(2), pos: v(300, 0), radius: Fixed::from_int(100) };
    // 150 from `near`'s centre (50 off its surface), 150 from `far`'s: a tie breaks on id.
    let tie = orbit::pick(&[near, far], v(150, 0)).unwrap();
    assert_eq!(tie.id, EntityId(2));
    let closer = orbit::pick(&[near, far], v(120, 0)).unwrap();
    assert_eq!(closer.id, EntityId(5));
}

#[test]
fn the_ghost_snaps_where_the_sim_builds() {
    let mut sim = game();
    spawn(&mut sim, "fx_world", v(1000, 0));
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    let db = sim.content().unwrap().clone();
    let what = db.key("fx_orbital").unwrap();
    let cursor = v(1100, 80);
    let slot = orbit::snap_view(&db, &sim.view(), what, cursor).expect("orbital").expect("slot");
    assert!(build(&mut sim, rig, cursor));
    for _ in 0..900 {
        sim.step(&[]);
    }
    let all = orbitals(&sim);
    assert_eq!(all, vec![(slot.at, slot.heading)]);
    // And the view now says taken; an ordinary def is not an orbital at all.
    assert_eq!(orbit::snap_view(&db, &sim.view(), what, cursor), Some(Err(orbit::OrbitError::Taken)));
    assert_eq!(orbit::snap_view(&db, &sim.view(), db.key("fx_station").unwrap(), cursor), None);
}

#[test]
fn a_station_belongs_to_the_planet_whose_orbit_it_is_on() {
    let a = orbit::Planet { id: EntityId(1), pos: v(0, 0), radius: Fixed::from_int(100) };
    let b = orbit::Planet { id: EntityId(2), pos: v(260, 0), radius: Fixed::from_int(100) };
    // On A's orbit, but nearer B's surface than A's.
    let station = v(175, 0);
    assert_eq!(orbit::pick(&[a, b], station).unwrap().id, EntityId(2));
    assert_eq!(orbit::owner(&[a, b], station).unwrap().id, EntityId(1));
    // So B stays free and A is taken.
    assert!(orbit::snap(&[a, b], &[station], v(300, 0)).is_ok());
    assert_eq!(orbit::snap(&[a, b], &[station], v(-50, 0)), Err(orbit::OrbitError::Taken));
}

#[test]
fn a_planet_far_away_is_never_picked() {
    // 100000 units: the square would wrap in 32.32 fixed point.
    let far = orbit::Planet { id: EntityId(1), pos: v(100_000, 0), radius: Fixed::from_int(200) };
    assert_eq!(orbit::pick(&[far], v(0, 0)), None);
    let wrap = orbit::Planet { id: EntityId(2), pos: v(65_536, 0), radius: Fixed::from_int(200) };
    assert_eq!(orbit::pick(&[wrap], v(0, 0)), None);
}

#[test]
fn a_rig_on_the_far_side_of_the_planet_still_gets_there() {
    let mut sim = game();
    let world = v(1000, 0);
    spawn(&mut sim, "fx_world", world);
    // The rig is on the +x side, the cursor on the -x side.
    let rig = spawn(&mut sim, "fx_rig", v(2000, 0));
    assert!(build(&mut sim, rig, v(900, 0)));
    for _ in 0..2400 {
        sim.step(&[]);
    }
    let all = orbitals(&sim);
    assert_eq!(all.len(), 1, "built");
    // Its stand point is outside the orbit, so the rig reaches it from either side (planets
    // do not block movement in the sim, so the path itself is not asserted).
}
