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
    let base = sta_content::fake::db();
    let mut b = sta_content::ContentDb::builder().profile(base.profile()).features(base.features().clone());
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
    beam.range = Fixed::from_int(RANGE);
    beam.refire = 45;
    let mut orbiter = base.units().find(|u| u.odf == "fx_miner").unwrap().clone();
    orbiter.odf = "fx_orbiter".into();
    orbiter.radius = Fixed::from_int(10);
    orbiter.hardpoints = vec![Hardpoint { weapon: DefRef::new("fx_minebeam"), node: "hp01".into(), arc: None }];
    let mut moon = base.units().find(|u| u.odf == "fx_moon").unwrap().clone();
    moon.radius = Fixed::from_int(40);
    orbit_content(b.weapon(beam).unit(orbiter).unit(moon))
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
fn several_miners_share_one_resource_and_a_digest_repeats() {
    let digest = || {
        let mut sim = orbit_game();
        let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
        spawn(&mut sim, "fx_station", PlayerId(0), v(1500, 0));
        let ms: Vec<_> = (0..3).map(|i| spawn(&mut sim, "fx_orbiter", PlayerId(0), v(500 + i * 40, i * 60))).collect();
        let o = Order::Mine { units: ms.clone(), node: moon };
        sim.step(&[Command { player: PlayerId(0), seq: 1, kind: CommandKind::Order(o) }]);
        let mut delivered = 0;
        for _ in 0..4500 {
            delivered += sim.step(&[]).events.iter().filter(|e| matches!(e, SimEvent::Delivered { .. })).count();
        }
        assert!(delivered >= 6, "{delivered}");
        sim.digest()
    };
    assert_eq!(digest(), digest());
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
            for e in &out.events {
                if let SimEvent::MinerPhase { phase, .. } = e {
                    if *phase == MinerPhase::Mining { mining = Some(t); last = flat(&sim, m, c).1; }
                    if *phase == MinerPhase::ToDock && mining.is_some() { break; }
                }
            }
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

/// A full hold keeps the freighter circling until the station is on its outer side.
#[test]
fn it_peels_off_the_orbit_when_the_station_is_on_its_outer_side() {
    let mut sim = orbit_game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(2000, 0));
    let m = spawn(&mut sim, "fx_orbiter", PlayerId(0), v(500, 0));
    assert!(mine(&mut sim, 0, m, moon));
    until(&mut sim, m, MinerPhase::ToDock, 3000);
    let c = sim.unit(moon).unwrap().pos;
    let mut left_band = None;
    for t in 0..600u32 {
        sim.step(&[]);
        let (r, _) = flat(&sim, m, c);
        let p = sim.unit(m).unwrap().pos;
        // Station along +x: outer side is x beyond the freighter's radial direction.
        let outer = (2000.0 - p.x.to_f32()) * p.x.to_f32() + (0.0 - p.z.to_f32()) * p.z.to_f32() > 0.0;
        if r > 1.5 * RANGE as f32 { left_band = Some((t, outer)); break; }
    }
    let (_, outer_when_leaving) = left_band.expect("it never left the orbit");
    assert!(outer_when_leaving, "it left with the station on the inner side");
}
