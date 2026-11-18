//! SIM-40: yard repair through the sim, on `fake::db()` with authored repair
//! keys. Expectations follow from the module's rules and the authored values.

use std::sync::Arc;

use sta_content::{ContentDb, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3, TICK_HZ};

use crate::command::{Command, CommandKind, Order, Subsystem};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// `fx_yard` repairs; `fx_cruiser` has 100 engine hit points repaired at 0.5 s
/// per point, hull 1000, shields 200 recharging 1 per tick, `healthRate` 2.
fn db(patch: impl Fn(&mut UnitDef)) -> Arc<ContentDb> {
    db_on(None, patch)
}

/// [`db`] under `profile` (the fake's own when `None`).
fn db_on(profile: Option<sta_content::Profile>, patch: impl Fn(&mut UnitDef)) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut b = match profile {
        Some(p) => ContentDb::builder().profile(p).features(p.features()),
        None => ContentDb::builder().profile(fake.profile()).features(fake.features().clone()),
    };
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        let mut u = u.clone();
        match u.odf.as_str() {
            "fx_yard" => u.repair.repair_facility = Some(true),
            "fx_cruiser" => {
                u.hull = Fixed::from_int(1000);
                u.shields.max = Fixed::from_int(200);
                u.shields.recharge = Fixed::ONE;
                u.system_hit_points[0] = Fixed::from_int(100);
                u.repair.system_times[0] = Some(Fixed::from_bits(1 << 31));
                u.stats.health_rate = Some(Fixed::from_int(2));
            }
            _ => {}
        }
        patch(&mut u);
        b = b.unit(u);
    }
    Arc::new(b.build().unwrap())
}

struct W {
    sim: Sim,
    yard: EntityId,
    ship: EntityId,
}

fn world(db: Arc<ContentDb>, gap: i32) -> W {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 5, map: Arc::new(sta_content::MapDef::empty()), content: Arc::clone(&db), players: vec![seat(1), seat(2)], options: GameOptions::default() });
    let yard = sim.spawn_def(db.key("fx_yard").unwrap(), PlayerId(1), v(0, 0), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let ship = sim.spawn_def(db.key("fx_cruiser").unwrap(), PlayerId(1), v(0, gap), (Fixed::ZERO, Fixed::ONE)).unwrap();
    W { sim, yard, ship }
}

impl W {
    fn damage(&mut self, ship: EntityId) {
        let u = self.sim.units.get_mut(ship).unwrap();
        u.hull[0] = Fixed::from_int(600);
        u.shields.now = [Fixed::ZERO; 4];
        assert!(self.sim.edit_subsystem_damage(ship, Subsystem::Engines, Fixed::from_int(100)));
    }
    fn order(&mut self, units: Vec<EntityId>, yard: Option<EntityId>) -> Vec<SimEvent> {
        let c = Command { player: PlayerId(1), seq: 9, kind: CommandKind::Order(Order::Repair { units, yard }) };
        self.sim.step(&[c]).events
    }
    fn whole(&self, ship: EntityId) -> bool {
        let u = self.sim.unit(ship).unwrap();
        u.hull[0] == u.hull[1] && u.shields.now[0] == u.shields.max && self.sim.subsystem_state(ship, Subsystem::Engines).unwrap().1 == Fixed::ZERO
    }
    fn run(&mut self, secs: u32) {
        for _ in 0..secs * TICK_HZ {
            self.sim.step(&[]);
        }
    }
}

fn rejected(ev: &[SimEvent]) -> bool {
    ev.iter().any(|e| matches!(e, SimEvent::Rejected { seq: 9, .. }))
}

#[test]
fn a_damaged_ship_goes_to_the_yard_and_leaves_whole_at_the_rules_rates() {
    let mut w = world(db(|_| {}), 600);
    let (ship, yard) = (w.ship, w.yard);
    w.damage(ship);
    assert!(!rejected(&w.order(vec![ship], None)));
    assert_eq!(w.sim.repairs().yard_of(ship), Some(yard));
    // Engines dead (100 of 100 points) and off, as a hit leaves them; the ship is brought to the dock.
    let u = w.sim.units.get_mut(ship).unwrap();
    u.flags = u.flags.with(crate::view::UnitFlags::ENGINES_OFF, true);
    u.pos = v(0, 20);
    // Hull 400 missing at 100 x 2 per second: 2 s; engines 100 points at 100 / 0.5 per second: 0.5 s;
    // shields 200 at 50 x 30 per second: under a second.
    w.run(3);
    assert!(w.whole(ship), "{:?}", w.sim.unit(ship));
    assert!(w.sim.repairs().yard_of(ship).is_none(), "it leaves the queue");
    assert!(!w.sim.unit(ship).unwrap().flags.contains(crate::view::UnitFlags::ENGINES_OFF), "a dead system repaired comes back on");
}

#[test]
fn the_hull_rate_defaults_to_a_fifth_of_the_shield_rate() {
    let mut w = world(db(|u| if u.odf == "fx_cruiser" { u.stats.health_rate = None }), 20);
    let ship = w.ship;
    w.sim.units.get_mut(ship).unwrap().hull[0] = Fixed::from_int(900);
    w.order(vec![ship], None);
    // shieldRate 30 per second, healthRate 6: 600 hull per second, so 100 missing take 1/6 s.
    let before = w.sim.unit(ship).unwrap().hull[0];
    w.sim.step(&[]);
    let gain = w.sim.unit(ship).unwrap().hull[0] - before;
    let want = Fixed::from_int(600) / Fixed::from_int(TICK_HZ as i32);
    // 0.2 is not exact in fixed point: a few raw bits short.
    assert!((gain - want).abs().to_bits() < 1 << 10, "{gain:?} vs {want:?}");
}

#[test]
fn refused_when_whole_not_a_yard_or_the_ship_may_not_use_one() {
    let mut w = world(db(|_| {}), 120);
    let (ship, yard) = (w.ship, w.yard);
    assert!(rejected(&w.order(vec![ship], None)), "nothing to repair");
    w.damage(ship);
    assert!(rejected(&w.order(vec![ship], Some(ship))), "a craft is no yard");
    let mut w2 = world(db(|u| if u.odf == "fx_yard" { u.repair.repair_facility = None }), 120);
    let s2 = w2.ship;
    w2.damage(s2);
    assert!(rejected(&w2.order(vec![s2], None)), "no repair facility");
    let mut w3 = world(db(|u| if u.odf == "fx_cruiser" { u.repair.can_shipyard_repair = Some(false) }), 120);
    let s3 = w3.ship;
    w3.damage(s3);
    assert!(rejected(&w3.order(vec![s3], None)), "canShipyardRepair = 0");
    let _ = yard;
}

#[test]
fn ships_are_serviced_one_at_a_time_in_order_and_stop_takes_one_out() {
    let d = db(|_| {});
    let mut w = world(Arc::clone(&d), 20);
    let second = w.sim.spawn_def(d.key("fx_cruiser").unwrap(), PlayerId(1), v(0, -20), (Fixed::ZERO, Fixed::ONE)).unwrap();
    for s in [w.ship, second] {
        w.sim.units.get_mut(s).unwrap().hull[0] = Fixed::from_int(600);
    }
    w.order(vec![w.ship], None);
    w.order(vec![second], None);
    assert_eq!(w.sim.repairs().queue(w.yard), &[w.ship, second]);
    w.sim.step(&[]);
    assert!(w.sim.unit(w.ship).unwrap().hull[0] > Fixed::from_int(600));
    assert_eq!(w.sim.unit(second).unwrap().hull[0], Fixed::from_int(600), "the second waits");
    let stop = Command { player: PlayerId(1), seq: 10, kind: CommandKind::Order(Order::Stop { units: vec![w.ship] }) };
    w.sim.step(&[stop]);
    assert_eq!(w.sim.repairs().queue(w.yard), &[second]);
    w.run(3);
    assert!(w.sim.repairs().is_empty());
    assert_eq!(w.sim.unit(second).unwrap().hull[0], Fixed::from_int(1000));
}

#[test]
fn the_yards_loss_ends_the_queue() {
    let mut w = world(db(|_| {}), 120);
    let (ship, yard) = (w.ship, w.yard);
    w.sim.units.get_mut(ship).unwrap().hull[0] = Fixed::from_int(600);
    w.order(vec![ship], None);
    assert!(w.sim.despawn(yard, None));
    w.sim.step(&[]);
    assert!(w.sim.repairs().is_empty());
}

#[test]
fn a_snapshot_mid_repair_continues_alike() {
    let d = db(|_| {});
    let mut w = world(Arc::clone(&d), 400);
    let ship = w.ship;
    w.sim.units.get_mut(ship).unwrap().hull[0] = Fixed::from_int(300);
    w.order(vec![ship], None);
    for _ in 0..20 {
        w.sim.step(&[]);
    }
    let copy = Sim::from_snapshot(&w.sim.snapshot(), Some(d)).unwrap();
    assert_eq!(copy.digest(), w.sim.digest());
    let (mut a, mut b) = (w.sim, copy);
    for _ in 0..200 {
        a.step(&[]);
        b.step(&[]);
        assert_eq!(a.digest(), b.digest());
    }
    assert_eq!(a.unit(ship).unwrap().hull, b.unit(ship).unwrap().hull);
}

#[test]
fn the_record_refuses_bad_queues() {
    use super::service::{Repairs, RepairsRec};
    let ok = |q: Vec<(EntityId, Vec<EntityId>)>| Repairs::from_rec(RepairsRec { version: 1, queues: q }, |id| id.0 < 10).is_some();
    assert!(ok(vec![(EntityId(1), vec![EntityId(2), EntityId(3)])]));
    assert!(!ok(vec![(EntityId(1), vec![])]), "empty queue");
    assert!(!ok(vec![(EntityId(3), vec![EntityId(2)]), (EntityId(1), vec![EntityId(4)])]), "yards out of order");
    assert!(!ok(vec![(EntityId(1), vec![EntityId(2)]), (EntityId(3), vec![EntityId(2)])]), "a ship twice");
    assert!(!ok(vec![(EntityId(1), vec![EntityId(20)])]), "an unknown ship");
}

#[test]
fn roots_lets_a_ship_dock_fifty_further_out() {
    use super::service::{DOCK_REACH, ROOTS_EXTRA_REACH, reaches};
    let stock = sta_content::fake::db();
    assert_eq!(reaches(&stock).0, DOCK_REACH);
    let roots = ContentDb::builder().profile(sta_content::Profile::Fo4).features(sta_content::Profile::Fo4.features()).build().unwrap();
    assert_eq!(reaches(&roots), (DOCK_REACH + ROOTS_EXTRA_REACH, super::service::WAIT_REACH + ROOTS_EXTRA_REACH));
}

#[test]
fn roots_refills_special_energy_when_the_repair_ends() {
    for (profile, refilled) in [(Some(sta_content::Profile::Fo4), true), (None, false)] {
        let mut w = world(db_on(profile, |u| if u.odf == "fx_cruiser" {
            u.special_energy_max = Fixed::from_int(100);
            u.special_energy_rate = Fixed::ZERO;
        }), 600);
        let ship = w.ship;
        w.damage(ship);
        w.sim.step(&[]);
        let db = w.sim.content.clone().unwrap();
        assert!(w.sim.specials.set_energy(&w.sim.units, &db, ship, Fixed::from_int(10)));
        assert!(!rejected(&w.order(vec![ship], None)));
        w.sim.units.get_mut(ship).unwrap().pos = v(0, 20);
        w.run(3);
        assert!(w.sim.repairs().yard_of(ship).is_none(), "{profile:?}: repaired and gone");
        let energy = w.sim.specials.energy_of(ship);
        assert_eq!(energy, Some(if refilled { Fixed::from_int(100) } else { Fixed::from_int(10) }), "{profile:?}");
    }
}

#[test]
fn roots_keeps_a_queued_ships_place_on_a_second_repair_order() {
    for (profile, kept) in [(Some(sta_content::Profile::Fo4), true), (None, false)] {
        let d = db_on(profile, |_| {});
        let mut w = world(Arc::clone(&d), 600);
        let first = w.ship;
        let second = w.sim.spawn_def(d.key("fx_cruiser").unwrap(), PlayerId(1), v(0, 700), (Fixed::ZERO, Fixed::ONE)).unwrap();
        w.damage(first);
        w.damage(second);
        assert!(!rejected(&w.order(vec![first, second], None)));
        let place = |w: &W| w.sim.repairs().queue(w.yard).iter().position(|s| *s == first);
        assert_eq!(place(&w), Some(0), "{profile:?}");
        assert!(!rejected(&w.order(vec![first], None)), "{profile:?}: a second order is accepted");
        assert_eq!(place(&w), Some(if kept { 0 } else { 1 }), "{profile:?}");
    }
}
