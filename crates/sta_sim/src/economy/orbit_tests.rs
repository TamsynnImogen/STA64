//! A freighter with a mining beam circles its resource for the whole harvest (issue #345).
#![allow(clippy::disallowed_methods)] // measuring floats in a test, not sim state
use super::tests::*;
use super::*;
use crate::command::{Command, CommandKind, Order};
use crate::event::MinerPhase;
use crate::world::NEUTRAL;
use crate::{Sim, WorldSource};
use sta_content::{DefRef, Hardpoint, WeaponDef, WeaponKind};

const RANGE: i32 = 180;

/// The fake content plus a freighter with a mining beam (`fx_orbiter`) and a moon with a radius.
fn orbit_game() -> Sim {
    orbit_game_with(None)
}

fn orbit_game_with(profile: Option<sta_content::Profile>) -> Sim {
    orbit_game_tuned(profile, RANGE, None)
}

/// As [`orbit_game_with`], with the beam's range and the freighter's `tooCloseToTurn` set.
fn orbit_game_tuned(profile: Option<sta_content::Profile>, range: i32, too_close: Option<i32>) -> Sim {
    orbit_content(orbit_builder(profile, range, too_close))
}

fn orbit_builder(profile: Option<sta_content::Profile>, range: i32, too_close: Option<i32>) -> sta_content::ContentBuilder {
    let base = sta_content::fake::db();
    let mut b = sta_content::ContentDb::builder().profile(profile.unwrap_or(base.profile())).features(base.features().clone());
    for u in base.units().filter(|u| u.odf != "fx_moon") {
        let mut u = u.clone();
        if u.odf == "fx_station" {
            u.radius = Fixed::from_int(150);
        }
        b = b.unit(u);
    }
    for w in base.weapons() {
        b = b.weapon(w.clone());
    }
    let mut beam = WeaponDef::new("fx_minebeam", "minebm", WeaponKind::Projectile);
    beam.range = Fixed::from_int(range);
    beam.refire = 45;
    let mut orbiter = base.units().find(|u| u.odf == "fx_miner").unwrap().clone();
    orbiter.odf = "fx_orbiter".into();
    orbiter.radius = Fixed::from_int(10);
    orbiter.hardpoints = vec![Hardpoint { weapon: DefRef::new("fx_minebeam"), node: "hp01".into(), arc: None }];
    if let Some(t) = too_close {
        orbiter.movement.too_close_to_turn = Fixed::from_int(t);
        orbiter.movement.path_lead = Fixed::from_int(300);
    }
    let mut moon = base.units().find(|u| u.odf == "fx_moon").unwrap().clone();
    moon.radius = Fixed::from_int(40);
    b.weapon(beam).unit(orbiter).unit(moon)
}

fn orbit_content(b: sta_content::ContentBuilder) -> Sim {
    game_with_content(b.build().unwrap())
}

fn flat(sim: &Sim, id: EntityId, c: Vec3) -> (f32, f32) {
    let p = sim.unit(id).unwrap().pos;
    let (dx, dz) = ((p.x - c.x).to_f32(), (p.z - c.z).to_f32());
    ((dx * dx + dz * dz).sqrt(), dz.atan2(dx))
}

/// Run until `phase` is announced; the tick count.
fn until(sim: &mut Sim, id: EntityId, phase: MinerPhase, max: u32) -> u32 {
    for t in 0..max {
        if sim.step(&[]).events.iter().any(|e| matches!(e, SimEvent::MinerPhase { id: i, phase: p, .. } if *i == id && *p == phase)) {
            return t;
        }
    }
    panic!("{phase:?} never announced");
}

#[test]
fn a_miner_circles_the_resource_at_its_weapons_range_for_the_whole_harvest() {
    let mut sim = orbit_game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(3000, 0));
    let m = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(600, 0));
    assert!(mine(&mut sim, 0, m, moon));
    until(&mut sim, m, MinerPhase::Mining, 3000);
    let c = sim.unit(moon).unwrap().pos;
    let (mut swept, mut last, mut ticks) = (0.0f32, flat(&sim, m, c).1, 0u32);
    let mut radii = Vec::new();
    loop {
        let out = sim.step(&[]);
        ticks += 1;
        let (r, a) = flat(&sim, m, c);
        let mut d = a - last;
        if d > std::f32::consts::PI { d -= 2.0 * std::f32::consts::PI }
        if d < -std::f32::consts::PI { d += 2.0 * std::f32::consts::PI }
        swept += d;
        last = a;
        if ticks > 150 { radii.push(r); }
        if out.events.iter().any(|e| matches!(e, SimEvent::MinerPhase { phase: MinerPhase::ToDock, .. })) { break; }
        assert!(ticks < 1000);
    }
    // Harvest time unchanged: 150 at 15 a second.
    assert!((290..=310).contains(&ticks), "{ticks} ticks");
    // It starts working just inside the weapon range plus both radii and spirals onto the circle.
    assert!(radii.iter().all(|r| *r < RANGE as f32 + 80.0), "radii {radii:?}");
    assert!((radii[radii.len() - 1] - RANGE as f32).abs() < 10.0, "radii {radii:?}");
    assert!(swept.abs() > 1.0, "swept {swept}");
    // One direction: clockwise (the angle falls).
    assert!(swept < 0.0);
}

#[test]
fn the_trip_is_unchanged_and_repeats() {
    let mut sim = orbit_game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(1500, 0));
    let m = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(600, 0));
    assert!(mine(&mut sim, 0, m, moon));
    let mut delivered = 0;
    for _ in 0..8000 {
        for e in sim.step(&[]).events {
            if let SimEvent::Delivered { amount, .. } = e {
                assert_eq!(amount, Fixed::from_int(150));
                delivered += 1;
            }
        }
    }
    assert!(delivered >= 3, "{delivered} trips");
}

#[test]
fn one_freighter_at_a_time_mines_a_node_across_repeated_deliveries() {
    let digest = || {
        let mut sim = orbit_game();
        let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
        spawn(&mut sim, "fx_station", PlayerId(0), v(1500, 0));
        let miners = [0, 1, 2].map(|i| spawn(&mut sim, "fx_orbiter", PlayerId(0), v(500 + i * 30, i * 50)));
        let order = Order::Mine { units: miners.to_vec(), node: moon };
        sim.step(&[Command { player: PlayerId(0), seq: 1, kind: CommandKind::Order(order) }]);
        let mut delivered = 0;
        let mut took_a_turn = [false; 3];
        for _ in 0..6000 {
            let before = miners.map(|id| sim.economy.cargo(id).unwrap_or(Fixed::ZERO));
            let out = sim.step(&[]);
            let mut active = 0;
            let mut extracting = 0;
            for (index, id) in miners.into_iter().enumerate() {
                let miner = sim.economy.miners.get(id).unwrap();
                active += usize::from(miner.phase == Phase::Mining);
                if miner.cargo > before[index] {
                    extracting += 1;
                    took_a_turn[index] = true;
                }
            }
            assert!(active <= 1, "{active} active miners");
            assert!(extracting <= 1, "{extracting} freighters extracted in one tick");
            delivered += out.events.iter().filter(|e| matches!(e, SimEvent::Delivered { .. })).count();
        }
        assert!(delivered >= 6, "{delivered} deliveries");
        assert!(took_a_turn.into_iter().all(|mined| mined), "miners that extracted: {took_a_turn:?}");
        sim.digest()
    };
    assert_eq!(digest(), digest());
}

#[test]
fn active_higher_id_keeps_the_node_until_cancelled_then_waiter_takes_over() {
    let mut sim = orbit_game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(1500, 0));
    let lower = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(500, 0));
    let incumbent = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(500, 50));
    assert!(mine(&mut sim, 0, incumbent, moon));
    until(&mut sim, incumbent, MinerPhase::Mining, 3000);
    assert!(mine(&mut sim, 0, lower, moon));
    for _ in 0..60 {
        sim.step(&[]);
        assert_eq!(sim.economy.miners.get(incumbent).unwrap().phase, Phase::Mining);
        assert_eq!(sim.economy.cargo(lower), Some(Fixed::ZERO));
        assert_ne!(sim.economy.miners.get(lower).unwrap().phase, Phase::Mining);
    }
    let out = sim.step(&[Command { player: PlayerId(0), seq: 9, kind: CommandKind::Order(Order::Stop { units: vec![incumbent] }) }]);
    assert!(out.events.iter().any(|e| matches!(e, SimEvent::MinerPhase { id, phase: MinerPhase::Idle, .. } if *id == incumbent)));
    until(&mut sim, lower, MinerPhase::Mining, 3000);
    assert!(sim.economy.cargo(lower).unwrap() > Fixed::ZERO);
}

#[test]
fn duplicate_restored_mining_phases_resolve_before_extraction() {
    let mut sim = orbit_game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    let first = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(500, 0));
    let second = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(500, 50));
    assert!(mine(&mut sim, 0, first, moon));
    assert!(mine(&mut sim, 0, second, moon));
    until(&mut sim, first, MinerPhase::Mining, 3000);
    let mut duplicate = *sim.economy.miners.get(second).unwrap();
    duplicate.phase = Phase::Mining;
    sim.economy.miners.insert(second, duplicate);
    let mut sim = Sim::from_snapshot(&sim.snapshot(), sim.content.clone()).unwrap();
    let before = sim.economy.cargo(first).unwrap();
    sim.step(&[]);
    assert!(sim.economy.cargo(first).unwrap() > before);
    assert_eq!(sim.economy.cargo(second), Some(Fixed::ZERO));
    assert_eq!(sim.economy.miners.get(second).unwrap().phase, Phase::ToNode);
}

#[test]
fn a_resource_that_is_gone_mid_orbit_sends_the_miner_home() {
    let mut sim = orbit_game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(1500, 0));
    let m = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(600, 0));
    assert!(mine(&mut sim, 0, m, moon));
    until(&mut sim, m, MinerPhase::Mining, 3000);
    for _ in 0..100 { sim.step(&[]); }
    sim.despawn(moon, None);
    let mut seen = Vec::new();
    for _ in 0..2500 {
        for e in sim.step(&[]).events {
            match e {
                SimEvent::MinerPhase { phase, .. } => seen.push(phase),
                SimEvent::Delivered { amount, .. } => assert!(amount > Fixed::ZERO && amount < Fixed::from_int(150)),
                _ => {}
            }
        }
    }
    assert!(seen.contains(&MinerPhase::ToDock) && seen.contains(&MinerPhase::Unloading), "{seen:?}");
}

#[test]
fn a_stop_mid_orbit_is_announced_and_the_miner_stops_circling() {
    let mut sim = orbit_game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(1500, 0));
    let m = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(600, 0));
    assert!(mine(&mut sim, 0, m, moon));
    until(&mut sim, m, MinerPhase::Mining, 3000);
    for _ in 0..60 { sim.step(&[]); }
    let out = sim.step(&[Command { player: PlayerId(0), seq: 9, kind: CommandKind::Order(Order::Stop { units: vec![m] }) }]);
    assert!(out.events.iter().any(|e| matches!(e, SimEvent::MinerPhase { id, phase: MinerPhase::Idle, .. } if *id == m)));
    for _ in 0..200 { sim.step(&[]); }
    let p = sim.unit(m).unwrap().pos;
    for _ in 0..60 { sim.step(&[]); }
    assert_eq!(sim.unit(m).unwrap().pos, p, "it kept circling after the stop");
}


/// The first trip (ordered from wherever the freighter stands, arriving with momentum) circles
/// and beams exactly like a later one: from several directions and distances, from inside the
/// work range, and ordered while flying fast.
#[test]
fn the_first_trip_circles_whatever_the_approach() {
    // (start x, start z, a move first so it is flying fast when the mine order comes)
    let starts: [(i32, i32, bool); 7] = [(700, 0, false), (-900, 300, false), (0, 450, false), (0, -1500, false), (100, 0, false), (60, 60, false), (900, 0, true)];
    for (sx, sz, fast) in starts {
        let mut sim = orbit_game();
        let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
        spawn(&mut sim, "fx_station", PlayerId(0), v(1500, 0));
        let m = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(sx, sz));
        if fast {
            sim.step(&[Command { player: PlayerId(0), seq: 5, kind: CommandKind::Order(Order::Move { units: vec![m], to: v(-2000, 0), queue: false }) }]);
            for _ in 0..90 { sim.step(&[]); }
        }
        assert!(mine(&mut sim, 0, m, moon));
        let c = sim.unit(moon).unwrap().pos;
        let (mut mining, mut swept, mut last) = (None::<u32>, 0.0f32, 0.0f32);
        let mut late_radii = Vec::new();
        for t in 0..3000u32 {
            let out = sim.step(&[]);
            let mut harvested = false;
            for e in &out.events {
                if let SimEvent::MinerPhase { phase, .. } = e {
                    if *phase == MinerPhase::Mining { mining = Some(t); last = flat(&sim, m, c).1; }
                    harvested |= *phase == MinerPhase::ToDock && mining.is_some();
                }
            }
            if harvested { break; }
            if let Some(t0) = mining {
                let (r, a) = flat(&sim, m, c);
                let mut d = a - last;
                if d > std::f32::consts::PI { d -= 2.0 * std::f32::consts::PI }
                if d < -std::f32::consts::PI { d += 2.0 * std::f32::consts::PI }
                swept += d;
                last = a;
                if t > t0 + 200 { late_radii.push(r); }
                if t > t0 + 330 { break; }
            }
        }
        assert!(mining.is_some(), "start ({sx},{sz}) never mined");
        assert!(swept < -1.0, "start ({sx},{sz}) swept {swept} rad: it did not circle");
        assert!(late_radii.iter().all(|r| *r > 0.6 * RANGE as f32 && *r < 1.45 * RANGE as f32), "start ({sx},{sz}) radii {late_radii:?}");
    }
}

/// The way back is ordinary movement: never above top speed (a tick's travel), round a solid in
/// the way, and no position jump.
#[test]
fn the_return_leg_is_ordinary_movement_round_obstacles() {
    let mut sim = orbit_game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    let st = spawn(&mut sim, "fx_station", PlayerId(0), v(2400, 0));
    // An enemy station in the straight line home.
    let wall = spawn(&mut sim, "fx_station", PlayerId(1), v(1200, 0));
    let wall_r = sim.content().unwrap().unit(sim.content().unwrap().key("fx_station").unwrap()).unwrap().radius.to_f32();
    let m = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(500, 300));
    let max_speed = sim.content().unwrap().unit(sim.content().unwrap().key("fx_orbiter").unwrap()).unwrap().movement.max_speed.to_f32();
    assert!(mine(&mut sim, 0, m, moon));
    let (wc, sc) = (sim.unit(wall).unwrap().pos, sim.unit(st).unwrap().pos);
    let mut prev = sim.unit(m).unwrap().pos;
    let (mut homeward, mut unloading, mut closest) = (false, false, f32::MAX);
    for _ in 0..4000 {
        let out = sim.step(&[]);
        let p = sim.unit(m).unwrap().pos;
        let step = (((p.x - prev.x).to_f32()).powi(2) + ((p.z - prev.z).to_f32()).powi(2)).sqrt();
        assert!(step <= max_speed / 30.0 + 0.05, "moved {step} in a tick (top speed {max_speed})");
        prev = p;
        for e in out.events {
            if let SimEvent::MinerPhase { phase, .. } = e {
                homeward |= phase == MinerPhase::ToDock;
                unloading |= phase == MinerPhase::Unloading;
            }
        }
        if homeward {
            closest = closest.min((((p.x - wc.x).to_f32()).powi(2) + ((p.z - wc.z).to_f32()).powi(2)).sqrt());
        }
        if unloading { break; }
    }
    assert!(unloading, "it never reached the station");
    assert!(closest > wall_r * 0.75, "flew through the enemy station: {closest} from its centre (radius {wall_r}, solid at three quarters)");
    // It docks within the last short stretch of the station's origin.
    let p = sim.unit(m).unwrap().pos;
    assert!((((p.x - sc.x).to_f32()).powi(2) + ((p.z - sc.z).to_f32()).powi(2)).sqrt() < 120.0);
}

/// A full hold keeps the freighter circling while the station is on its inner side, and it
/// leaves only once the station is on the outer side.
#[test]
fn it_peels_off_the_orbit_when_the_station_is_on_its_outer_side() {
    let mut tried = 0;
    for (sx, sz) in [(-500, 0), (0, 500), (0, -500), (500, 300), (-300, -400), (-400, 300), (300, -450)] {
        let mut sim = orbit_game();
        let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
        spawn(&mut sim, "fx_station", PlayerId(0), v(2000, 0));
        let m = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(sx, sz));
        assert!(mine(&mut sim, 0, m, moon));
        until(&mut sim, m, MinerPhase::ToDock, 3000);
        let c = sim.unit(moon).unwrap().pos;
        let outer = |sim: &Sim| {
            let p = sim.unit(m).unwrap().pos;
            (2000.0 - p.x.to_f32()) * p.x.to_f32() - p.z.to_f32() * p.z.to_f32() + 0.0 > 0.0
        };
        // Only a start whose hold filled with the station on the inner side tells us anything.
        if outer(&sim) {
            continue;
        }
        tried += 1;
        let (mut waited, mut left_with_outer) = (0, None);
        for _ in 0..900u32 {
            sim.step(&[]);
            if flat(&sim, m, c).0 > 1.5 * RANGE as f32 {
                left_with_outer = Some(outer(&sim));
                break;
            }
            waited += 1;
        }
        assert!(waited > 15, "start ({sx},{sz}): it left at once ({waited} ticks) with the station on its inner side");
        assert_eq!(left_with_outer, Some(true), "start ({sx},{sz}): it left with the station on the inner side");
    }
    assert!(tried > 0, "no start filled its hold with the station on the inner side");
}

/// A station inside the orbit band cannot be waited for: no endless orbit.
#[test]
fn a_station_inside_the_orbit_band_does_not_trap_a_full_freighter() {
    let mut sim = orbit_game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(0, 150));
    let m = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(500, 0));
    assert!(mine(&mut sim, 0, m, moon));
    let mut delivered = false;
    for _ in 0..3000 {
        delivered |= sim.step(&[]).events.iter().any(|e| matches!(e, SimEvent::Delivered { .. }));
    }
    assert!(delivered);
}

/// On the smooth movement model of the real profiles (a craft nearly at its point stops: "too
/// close to turn"), several freighters on one station all get to unload.
#[test]
fn several_freighters_unload_at_one_station_on_the_smooth_model() {
    let mut sim = orbit_game_with(Some(sta_content::Profile::Sta2Classic));
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(1500, 0));
    let ms: Vec<_> = (0..4).map(|i| spawn(&mut sim, "fx_orbiter", PlayerId(0), v(500 + i * 30, i * 50))).collect();
    sim.step(&[Command { player: PlayerId(0), seq: 1, kind: CommandKind::Order(Order::Mine { units: ms, node: moon }) }]);
    let mut delivered = 0;
    for _ in 0..9000 {
        delivered += sim.step(&[]).events.iter().filter(|e| matches!(e, SimEvent::Delivered { .. })).count();
    }
    assert!(delivered >= 8, "{delivered} deliveries: some freighter never unloaded");
}
/// A freighter without a mining beam whose station is destroyed on the way back keeps its target
/// (as on main: no rule measured); only beam-armed miners drop theirs with the work.
#[test]
fn a_beamless_freighter_keeps_its_target_when_the_station_is_destroyed_on_the_way_back() {
    let mut sim = orbit_game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    let st = spawn(&mut sim, "fx_station", PlayerId(0), v(3000, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(100, 0));
    assert!(mine(&mut sim, 0, m, moon));
    until(&mut sim, m, MinerPhase::ToDock, 3000);
    for _ in 0..3 { sim.step(&[]); }
    assert_eq!(sim.unit(m).unwrap().target, Some(v(3000, 0)));
    sim.despawn(st, None);
    for _ in 0..5 {
        sim.step(&[]);
        assert_eq!(sim.unit(m).unwrap().target, Some(v(3000, 0)), "the target was cleared");
    }
}

/// SIM-65: the only station is an orbital facility that circles its planet (about 15 units a
/// second on a 120 s orbit). Freighters steer at the station's place every tick, so they follow it
/// round and still dock: with and without a mining beam.
#[test]
fn freighters_still_dock_at_an_orbital_station_that_circles_its_planet() {
    let mut orbital = sta_content::UnitDef::new("fx_orbital", "p_mining");
    orbital.race = "fx".into();
    orbital.hull = Fixed::from_int(200);
    orbital.orbit_time = Some(Fixed::from_int(120));
    let mut world = sta_content::UnitDef::new("fx_world", "planet");
    world.radius = Fixed::from_int(200);
    world.resource = Some(2);
    world.can_have_orbital_station = true;
    let mut sim = orbit_content(orbit_builder(None, RANGE, None).unit(orbital).unit(world));
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_world", NEUTRAL, v(1500, 0));
    let station = spawn(&mut sim, "fx_orbital", PlayerId(0), v(1500, 275));
    let ms = vec![spawn(&mut sim, "fx_orbiter", PlayerId(0), v(500, 0)), spawn(&mut sim, "fx_miner", PlayerId(0), v(500, 60))];
    let at = sim.unit(station).unwrap().pos;
    sim.step(&[Command { player: PlayerId(0), seq: 1, kind: CommandKind::Order(Order::Mine { units: ms.clone(), node: moon }) }]);
    let mut delivered = 0;
    // Unloads per freighter: each one docks, not just one of the two.
    let mut unloads = [0usize; 2];
    for _ in 0..9000 {
        for e in sim.step(&[]).events {
            match e {
                SimEvent::Delivered { .. } => delivered += 1,
                SimEvent::MinerPhase { id, phase: MinerPhase::Unloading, .. } => {
                    if let Some(i) = ms.iter().position(|m| *m == id) {
                        unloads[i] += 1;
                    }
                }
                _ => {}
            }
        }
    }
    assert!(sim.unit(station).unwrap().pos != at, "the station circled meanwhile");
    assert!(delivered >= 4, "{delivered} deliveries at a circling station");
    assert!(unloads.iter().all(|&n| n >= 2), "unloads per freighter (beam-armed, beamless): {unloads:?}");
}

/// Fleet Operations 4.0's miners (every race) have a 150 beam and `tooCloseToTurn 120`: the 30 degree
/// pursuit point lies inside that, so the smooth model halted the miner and it sat still, beam on,
/// instead of circling. The point is pushed out to `tooCloseToTurn + 10` from the miner. Pure pursuit
/// of that point settles on a circle of about 0.87 of the range (130 for 150).
#[test]
fn a_miner_with_a_large_too_close_to_turn_still_circles_at_the_beams_range() {
    // Start far away flying in, and start at rest inside the circle (68..119 from the centre).
    for (start, at_rest) in [(v(900, 100), false), (v(90, 0), true), (v(0, -110), true)] {
        let mut sim = orbit_game_tuned(Some(sta_content::Profile::Fo4), 150, Some(120));
        let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
        spawn(&mut sim, "fx_station", PlayerId(0), v(3000, 0));
        let m = spawn(&mut sim, "fx_orbiter", PlayerId(0), start);
        assert!(mine(&mut sim, 0, m, moon));
        if !at_rest { until(&mut sim, m, MinerPhase::Mining, 3000); }
        let c = sim.unit(moon).unwrap().pos;
        let (mut swept, mut last) = (0.0f32, flat(&sim, m, c).1);
        let mut late = Vec::new();
        for t in 0..250u32 {
            sim.step(&[]);
            let (r, a) = flat(&sim, m, c);
            let mut d = a - last;
            if d > std::f32::consts::PI { d -= 2.0 * std::f32::consts::PI }
            if d < -std::f32::consts::PI { d += 2.0 * std::f32::consts::PI }
            swept += d;
            last = a;
            if t >= 150 { late.push((r, sim.unit(m).unwrap().speed.to_f32())); }
        }
        // Moving at more than half the combat speed (the stop threshold), clockwise, on the circle.
        assert!(swept < -3.0, "start {start:?}: swept {swept} rad, the miner did not circle");
        assert!(late.iter().all(|(r, v)| (*r - 130.0).abs() < 15.0 && *v > 50.0), "start {start:?}: {late:?}");
    }
}
