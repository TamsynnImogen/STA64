//! SIM-41: repair ships beam allies.

use std::sync::Arc;

use sta_content::{ContentDb, DefRef, Hardpoint, WeaponDef, WeaponKind};
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, TeamId, Vec3};

use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, SimEvent, WorldSource};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// The fake content plus `fx_rship`: a repair ship with `repairRate` 0.25 and a 200-range beam;
/// `fx_cruiser` gets hull 1000 and `healthRate` 2 (100 x 2 x 0.25 = 50 hull a second by beam).
fn db() -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    let mut beam = WeaponDef::new("fx_repairbeam", "cannon", WeaponKind::Beam);
    beam.range = Fixed::from_int(200);
    b = b.weapon(beam);
    for u in fake.units() {
        let mut u = u.clone();
        if u.odf == "fx_cruiser" {
            u.hull = Fixed::from_int(1000);
            u.stats.health_rate = Some(Fixed::from_int(2));
            let mut r = u.clone();
            r.odf = "fx_rship".into();
            r.class = sta_content::ClassLabel::new("repairship");
            r.repair.repair_rate = Some(Fixed::from_bits(1 << 30));
            r.hardpoints = vec![Hardpoint { weapon: DefRef::new("fx_repairbeam"), node: "hp01".into(), arc: None }];
            b = b.unit(r);
        }
        b = b.unit(u);
    }
    Arc::new(b.build().unwrap())
}

struct W {
    sim: Sim,
    rship: EntityId,
    ship: EntityId,
    foe: EntityId,
}

/// A repair ship of player 1 at the origin, a damaged cruiser of player 1 `gap` away along z,
/// a damaged cruiser of player 2 far off.
fn world(gap: i32) -> W {
    let d = db();
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 3, map: Arc::new(sta_content::MapDef::empty()), content: Arc::clone(&d), players: vec![seat(1), seat(2)], options: GameOptions::default() });
    let ahead = (Fixed::ZERO, Fixed::ONE);
    let rship = sim.spawn_def(d.key("fx_rship").unwrap(), PlayerId(1), v(0, 0), ahead).unwrap();
    let ship = sim.spawn_def(d.key("fx_cruiser").unwrap(), PlayerId(1), v(0, gap), ahead).unwrap();
    let foe = sim.spawn_def(d.key("fx_cruiser").unwrap(), PlayerId(2), v(5000, 5000), ahead).unwrap();
    for id in [ship, foe] {
        sim.units.get_mut(id).unwrap().hull[0] = Fixed::from_int(600);
    }
    W { sim, rship, ship, foe }
}

impl W {
    fn order(&mut self, units: Vec<EntityId>, target: EntityId) -> Vec<SimEvent> {
        let c = Command { player: PlayerId(1), seq: 9, kind: CommandKind::Order(Order::Repair { units, yard: Some(target) }) };
        self.sim.step(&[c]).events
    }
    fn hull(&self, id: EntityId) -> Fixed {
        self.sim.unit(id).unwrap().hull[0]
    }
    fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.sim.step(&[]);
        }
    }
}

fn rejected(ev: &[SimEvent]) -> bool {
    ev.iter().any(|e| matches!(e, SimEvent::Rejected { seq: 9, .. }))
}

#[test]
fn a_repair_ship_beams_an_ally_in_range_at_its_repair_rate_and_stops_when_it_is_whole() {
    let mut w = world(150);
    let (rship, ship) = (w.rship, w.ship);
    assert!(!rejected(&w.order(vec![rship], ship)));
    assert_eq!(w.sim.repairs().beam_of(rship), Some(ship));
    let before = w.hull(ship);
    w.run(TICK_HZ);
    let gained = (w.hull(ship) - before).to_f32();
    assert!((gained - 50.0).abs() < 2.0_f32, "50 hull a second (100 x healthRate 2 x repairRate 0.25), got {gained}");
    w.run(8 * TICK_HZ);
    assert_eq!(w.hull(ship), Fixed::from_int(1000));
    assert!(w.sim.repairs().beam_of(rship).is_none(), "the order ends when the target is whole");
}

#[test]
fn out_of_range_the_repair_ship_approaches_and_does_not_repair_yet() {
    let mut w = world(3000);
    let (rship, ship) = (w.rship, w.ship);
    assert!(!rejected(&w.order(vec![rship], ship)));
    w.run(1);
    assert_eq!(w.hull(ship), Fixed::from_int(600), "3000 apart: out of the beam's 200");
    let z0 = w.sim.unit(rship).unwrap().pos.z;
    w.run(2 * TICK_HZ);
    assert!(w.sim.unit(rship).unwrap().pos.z > z0, "it moves toward the target");
    w.run(60 * TICK_HZ);
    assert!(w.hull(ship) > Fixed::from_int(600), "once near, the beam repairs");
}

#[test]
fn enemies_whole_ships_and_itself_are_refused() {
    let mut w = world(150);
    let (rship, ship, foe) = (w.rship, w.ship, w.foe);
    assert!(rejected(&w.order(vec![rship], foe)), "an enemy is not repaired");
    assert!(rejected(&w.order(vec![rship], rship)), "not itself");
    w.sim.units.get_mut(ship).unwrap().hull[0] = Fixed::from_int(1000);
    assert!(rejected(&w.order(vec![rship], ship)), "nothing to repair");
    // A ship that is not a repair ship cannot beam: the order is a yard repair, refused (no yard).
    w.sim.units.get_mut(ship).unwrap().hull[0] = Fixed::from_int(600);
    assert!(rejected(&w.order(vec![ship], rship)));
}

#[test]
fn a_snapshot_mid_beam_continues_alike() {
    let mut w = world(150);
    let (rship, ship) = (w.rship, w.ship);
    assert!(!rejected(&w.order(vec![rship], ship)));
    w.run(10);
    let bytes = w.sim.snapshot();
    let mut copy = Sim::from_snapshot(&bytes, w.sim.content.clone()).unwrap();
    assert_eq!(copy.repairs().beam_of(rship), Some(ship));
    for _ in 0..TICK_HZ {
        w.sim.step(&[]);
        copy.step(&[]);
    }
    assert_eq!(copy.digest(), w.sim.digest());
}

#[test]
fn the_record_refuses_bad_beams() {
    use crate::repair::{Repairs, RepairsRec};
    let ok = |b: Vec<(EntityId, EntityId)>, q: Vec<(EntityId, Vec<EntityId>)>| Repairs::from_rec(RepairsRec { version: 2, queues: q, beams: b }, |id| id.0 < 10).is_some();
    assert!(ok(vec![(EntityId(1), EntityId(2)), (EntityId(3), EntityId(2))], vec![]));
    assert!(!ok(vec![(EntityId(3), EntityId(2)), (EntityId(1), EntityId(2))], vec![]), "unordered");
    assert!(!ok(vec![(EntityId(1), EntityId(1))], vec![]), "on itself");
    assert!(!ok(vec![(EntityId(1), EntityId(20))], vec![]), "unknown target");
    assert!(!ok(vec![(EntityId(1), EntityId(2))], vec![(EntityId(5), vec![EntityId(1)])]), "also queued at a yard");
}
