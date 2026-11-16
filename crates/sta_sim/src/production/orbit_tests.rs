//! SIM-62: orbital stations snap to a planet's orbit slot, one per planet.
//! SIM-65: a finished station circles its planet (the second half of the file).
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

/// `orbit_time` seconds per orbit; zero keeps a station at its slot (the SIM-62 tests).
fn content(orbit_time: i32) -> Arc<ContentDb> {
    content_with_time(Fixed::from_int(orbit_time))
}

fn content_with_time(orbit_time: Fixed) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.pay_on_queue = true;
    let mut rig = UnitDef::new("fx_rig", "constructionrig");
    rig.race = "fx".into();
    rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.hull = Fixed::from_int(100);
    // An orbital station: class p_mining.
    let mut orbital = UnitDef::new("fx_orbital", "p_mining");
    orbital.race = "fx".into();
    orbital.hull = Fixed::from_int(200);
    orbital.build.cost = vec![Fixed::from_int(100)];
    orbital.build.ticks = 60;
    orbital.orbit_time = Some(orbit_time);
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
    game_with(0)
}

fn game_with(orbit_time: i32) -> Sim {
    game_with_content(content(orbit_time))
}

fn game_with_content(content: Arc<ContentDb>) -> Sim {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Human };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content,
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
    assert_eq!(orbitals(&sim).len(), 2, "a refused replacement keeps the second rig's valid reservation");
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

// SIM-65: orbiting.

fn at_ring(sim: &Sim, world: Vec3) -> Vec3 {
    let (pos, _) = orbitals(sim)[0];
    Vec3 { x: pos.x - world.x, y: pos.y - world.y, z: pos.z - world.z }
}

/// A finished orbital station facing +Z of a planet at (1000, 0): bearing 0.
fn finished(sim: &mut Sim) -> (Vec3, EntityId) {
    let world = v(1000, 0);
    spawn(sim, "fx_world", world);
    let ring = RADIUS + orbit::ALTITUDE;
    let station = spawn(sim, "fx_orbital", v(1000, ring));
    (world, station)
}

#[test]
fn a_station_circles_its_planet_one_orbit_per_orbit_time() {
    let mut sim = game_with(120);
    let (world, station) = finished(&mut sim);
    let ring = f64::from(RADIUS + orbit::ALTITUDE);
    // A quarter orbit (30 s): the bearing runs from +Z toward +X.
    for _ in 0..900 {
        sim.step(&[]);
        let r = dist(sim.unit(station).unwrap().pos, world);
        assert!((r - ring).abs() < 0.01, "stays on the orbit: {r}");
    }
    let off = at_ring(&sim, world);
    assert!(off.x.to_f32() > 274.0 && off.z.to_f32().abs() < 3.0, "a quarter turn toward +X: {off:?}");
    let (_, heading) = orbitals(&sim)[0];
    assert!(heading.0.to_f32() > 0.999 && heading.1.to_f32().abs() < 0.01, "faces outward: {heading:?}");
    // Another three quarters: back at the start, then the same again.
    for _ in 0..2700 {
        sim.step(&[]);
    }
    let off = at_ring(&sim, world);
    assert!(off.x.to_f32().abs() < 3.0 && off.z.to_f32() > 274.0, "one full orbit: {off:?}");
    assert_eq!(sim.unit(station).unwrap().pos.y, world.y);
}

#[test]
fn no_orbit_time_a_building_station_or_open_space_keeps_the_station_still() {
    // Zero seconds: no orbiting.
    let mut sim = game_with(0);
    let (_, station) = finished(&mut sim);
    let at = sim.unit(station).unwrap().pos;
    for _ in 0..300 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(station).unwrap().pos, at);
    // No planet near: nothing to circle.
    let mut sim = game_with(120);
    let far = spawn(&mut sim, "fx_orbital", v(5000, 5000));
    for _ in 0..300 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(far).unwrap().pos, v(5000, 5000));
    // Under construction: it stays at its site until finished.
    let mut sim = game_with(120);
    let (_, station) = finished(&mut sim);
    sim.units.get_mut(station).unwrap().progress = Some(Fixed::ZERO);
    let at = sim.unit(station).unwrap().pos;
    for _ in 0..300 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(station).unwrap().pos, at);
}

#[test]
fn a_station_off_the_ring_stays_where_a_script_or_map_put_it() {
    let mut sim = game_with(120);
    let world = v(1000, 0);
    spawn(&mut sim, "fx_world", world);
    // Within reach of the planet but 20 units off its circle.
    let station = spawn(&mut sim, "fx_orbital", v(1000, RADIUS + 20));
    // On the circle, then moved off it by a script (`SetPosition`).
    let moved = spawn(&mut sim, "fx_orbital", v(1000, -(RADIUS + orbit::ALTITUDE)));
    for _ in 0..30 {
        sim.step(&[]);
    }
    assert!(sim.unit(moved).unwrap().pos != v(1000, -(RADIUS + orbit::ALTITUDE)), "on the circle it orbits");
    let away = v(1000, 3 * RADIUS);
    sim.units.get_mut(moved).unwrap().pos = away;
    for _ in 0..300 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(station).unwrap().pos, v(1000, RADIUS + 20), "never on the ring: never pulled onto it");
    assert_eq!(sim.unit(moved).unwrap().pos, away, "moved off the ring: stops orbiting, not pulled back");
}

#[test]
fn a_built_station_circles_from_the_bearing_it_was_built_at_and_the_slot_stays_taken() {
    let mut sim = game_with(120);
    let world = v(1000, 0);
    spawn(&mut sim, "fx_world", world);
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    assert!(build(&mut sim, rig, v(960, 0)));
    for _ in 0..1800 {
        sim.step(&[]);
    }
    assert_eq!(orbitals(&sim).len(), 1, "built");
    let r = dist(orbitals(&sim)[0].0, world);
    assert!((r - f64::from(RADIUS + orbit::ALTITUDE)).abs() < 0.01);
    // Wherever it is now, it still owns the planet and the cursor sees it taken.
    assert!(!build(&mut sim, rig, v(1100, 80)), "taken while circling");
}

#[test]
fn circling_is_deterministic_and_survives_a_snapshot() {
    let run = |cut: bool| {
        let mut sim = game_with(120);
        finished(&mut sim);
        for _ in 0..500 {
            sim.step(&[]);
        }
        if cut {
            let bytes = sim.snapshot();
            sim = Sim::from_snapshot(&bytes, sim.content().cloned()).unwrap();
        }
        for _ in 0..1000 {
            sim.step(&[]);
        }
        sim.digest()
    };
    assert_eq!(run(false), run(false));
    assert_eq!(run(false), run(true), "a mid-orbit save restores to the same course");
}

#[test]
fn the_step_angle_is_two_pi_over_the_orbit_time() {
    let a = orbit::step_angle(Fixed::from_int(120)).to_f32();
    assert!((a - std::f32::consts::TAU / 3600.0).abs() < 1e-6, "{a}");
    assert_eq!(orbit::step_angle(Fixed::ZERO), Fixed::ZERO);
    assert_eq!(orbit::step_angle(-Fixed::ONE), Fixed::ZERO);
}

#[test]
fn an_extreme_orbit_time_costs_one_step_and_never_hangs() {
    // The smallest positive time spins it by a bounded angle; a huge one does not move it.
    for t in [Fixed::from_bits(1), Fixed::from_bits(30), Fixed::from_bits(i64::MAX)] {
        let a = orbit::step_angle(t);
        assert!(a >= Fixed::ZERO && a.to_f32() < std::f32::consts::TAU, "{t:?}: {a:?}");
    }
    assert_eq!(orbit::step_angle(Fixed::from_bits(i64::MAX)), Fixed::ZERO);
    let ring = f64::from(RADIUS + orbit::ALTITUDE);
    for t in [Fixed::from_bits(1), Fixed::from_bits(i64::MAX)] {
        let mut s = game_with_content(content_with_time(t));
        let (w, st) = finished(&mut s);
        for _ in 0..30 {
            s.step(&[]);
        }
        let r = dist(s.unit(st).unwrap().pos, w);
        assert!((r - ring).abs() < 0.05, "{t:?}: stays on the ring: {r}");
    }
}

/// The station's position and heading bits after 3600 and 108000 ticks of a
/// 100 s orbit round a planet at (1000, 0): constants, so every OS is held to
/// the same numbers and not only to itself.
#[test]
fn the_orbit_math_is_pinned_to_constants() {
    let mut sim = game_with(100);
    let (_, station) = finished(&mut sim);
    let bits = |sim: &Sim| {
        let u = sim.unit(station).unwrap();
        [u.pos.x.to_bits(), u.pos.z.to_bits(), u.heading.0.to_bits(), u.heading.1.to_bits()]
    };
    for _ in 0..3600 {
        sim.step(&[]);
    }
    assert_eq!(bits(&sim), PIN_3600, "after 3600 ticks");
    for _ in 3600..108_000 {
        sim.step(&[]);
    }
    assert_eq!(bits(&sim), PIN_108000, "after 108000 ticks");
}

const PIN_3600: [i64; 4] = [5_419_037_378_300, 362_631_314_925, 4_087_527_572, 1_318_659_327];
const PIN_108000: [i64; 4] = [4_297_444_042_525, 1_181_113_409_575, 9_006_351, 4_294_957_853];
