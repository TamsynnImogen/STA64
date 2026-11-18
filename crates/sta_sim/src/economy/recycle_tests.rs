//! SIM-45: decommissioning.

use std::sync::Arc;

use sta_content::{ContentDb, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Edit, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, SimEvent, WorldSource};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// `fx_cruiser` costs 600 dilithium, 300 metal and 50 biomatter and takes 300 ticks to build;
/// `fx_yard` takes decommissioned ships (`patch` adjusts any unit).
fn db(patch: impl Fn(&mut UnitDef)) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        let mut u = u.clone();
        match u.odf.as_str() {
            "fx_cruiser" => {
                u.build.cost = vec![Fixed::from_int(600), Fixed::ZERO, Fixed::from_int(300), Fixed::from_int(50)];
                u.build.ticks = 300;
            }
            "fx_yard" => u.repair.decommission_facility = Some(true),
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

fn world(d: Arc<ContentDb>, gap: i32) -> W {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 4, map: Arc::new(sta_content::MapDef::empty()), content: Arc::clone(&d), players: vec![seat(1), seat(2)], options: GameOptions::default() });
    let ahead = (Fixed::ZERO, Fixed::ONE);
    let yard = sim.spawn_def(d.key("fx_yard").unwrap(), PlayerId(1), v(0, 0), ahead).unwrap();
    let ship = sim.spawn_def(d.key("fx_cruiser").unwrap(), PlayerId(1), v(0, gap), ahead).unwrap();
    W { sim, yard, ship }
}

impl W {
    fn order(&mut self, units: Vec<EntityId>, yard: Option<EntityId>) -> Vec<SimEvent> {
        let c = Command { player: PlayerId(1), seq: 9, kind: CommandKind::Order(Order::Decommission { units, yard }) };
        self.sim.step(&[c]).events
    }
    fn stock(&self, slot: usize) -> Fixed {
        self.sim.economy.stock_of(PlayerId(1)).get(slot).copied().unwrap_or(Fixed::ZERO)
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

fn near(a: Fixed, b: f64) -> bool {
    (a.to_f32() as f64 - b).abs() < 1.0
}

#[test]
fn in_place_it_takes_the_deconstruct_share_of_build_time_and_returns_the_race_fraction() {
    let d = db(|u| if u.odf == "fx_cruiser" { u.repair.recycles_in_place = Some(true) });
    let mut w = world(d, 600);
    let (ship, before) = (w.ship, [w.stock(0), w.stock(2), w.stock(3)]);
    assert!(!rejected(&w.order(vec![ship], None)));
    // 300 build ticks x 0.67 (no header: stock A2's) = 201 ticks.
    w.run(195);
    assert!(w.sim.unit(ship).is_some(), "still recycling");
    w.run(10);
    assert!(w.sim.unit(ship).is_none(), "gone after the recycle time");
    // The race sets no fraction: the engine's 0.8; biomatter has no header fraction: 0.
    assert!(near(w.stock(0) - before[0], 480.0), "{:?}", w.stock(0) - before[0]);
    assert!(near(w.stock(2) - before[1], 240.0));
    assert!(near(w.stock(3) - before[2], 0.0));
}

#[test]
fn at_a_yard_it_flies_there_and_returns_the_yards_fraction() {
    let d = db(|u| if u.odf == "fx_yard" { u.repair.dilithium_recycling = Some(Fixed::from_bits(1 << 31)) });
    let mut w = world(d, 900);
    let (ship, yard) = (w.ship, w.yard);
    let before = (w.stock(0), w.stock(2));
    assert!(!rejected(&w.order(vec![ship], None)), "the sim picks the yard");
    assert_eq!(w.sim.recycles.job_of(ship).and_then(|j| j.yard), Some(yard));
    w.run(2);
    assert!(w.sim.recycles.job_of(ship).is_some_and(|j| j.left.is_none()), "travelling first");
    w.run(60 * 30);
    assert!(w.sim.unit(ship).is_none(), "docked and recycled");
    assert!(near(w.stock(0) - before.0, 300.0), "the yard's 0.5 of 600: {:?}", w.stock(0) - before.0);
    assert!(near(w.stock(2) - before.1, 240.0), "metal: the race's 0.8");
}

#[test]
fn without_a_yard_banned_or_not_ones_own_it_is_refused() {
    let d = db(|u| if u.odf == "fx_yard" { u.repair.decommission_facility = None });
    let mut w = world(d, 600);
    assert!(rejected(&w.order(vec![w.ship], None)), "no decommission facility");
    let mut w = world(db(|_| {}), 600);
    let ship = w.ship;
    let ban = Command { player: PlayerId::SCRIPT, seq: 1, kind: CommandKind::Edit(Edit::Decommissionable(ship, false)) };
    w.sim.step(&[ban]);
    assert!(rejected(&w.order(vec![ship], None)), "a script forbade it");
    let mut w = world(db(|_| {}), 600);
    let d = w.sim.content.clone().unwrap();
    let foe = w.sim.spawn_def(d.key("fx_cruiser").unwrap(), PlayerId(2), v(300, 300), (Fixed::ZERO, Fixed::ONE)).unwrap();
    assert!(rejected(&w.order(vec![foe], None)), "not ours");
}

#[test]
fn another_order_stops_it_and_nothing_is_returned() {
    let d = db(|u| if u.odf == "fx_cruiser" { u.repair.recycles_in_place = Some(true) });
    let mut w = world(d, 600);
    let ship = w.ship;
    let before = w.stock(0);
    assert!(!rejected(&w.order(vec![ship], None)));
    w.run(50);
    let stop = Command { player: PlayerId(1), seq: 2, kind: CommandKind::Order(Order::Stop { units: vec![ship] }) };
    w.sim.step(&[stop]);
    assert!(w.sim.recycles.job_of(ship).is_none());
    w.run(300);
    assert!(w.sim.unit(ship).is_some());
    assert_eq!(w.stock(0), before);
}

#[test]
fn a_snapshot_mid_recycle_continues_alike_and_bad_records_are_refused() {
    let d = db(|u| if u.odf == "fx_cruiser" { u.repair.recycles_in_place = Some(true) });
    let mut w = world(d, 600);
    assert!(!rejected(&w.order(vec![w.ship], None)));
    w.run(40);
    let bytes = w.sim.snapshot();
    let mut copy = Sim::from_snapshot(&bytes, w.sim.content.clone()).unwrap();
    assert_eq!(copy.recycles, w.sim.recycles);
    for _ in 0..300 {
        w.sim.step(&[]);
        copy.step(&[]);
    }
    assert_eq!(copy.digest(), w.sim.digest());
    use super::recycle::{Job, Recycles, RecyclesRec};
    let ok = |jobs: Vec<(EntityId, Job)>| Recycles::from_rec(RecyclesRec { version: 1, jobs }, |id| id.0 < 10).is_some();
    let j = |yard: Option<u32>| Job { yard: yard.map(EntityId), left: None };
    assert!(ok(vec![(EntityId(1), j(Some(2))), (EntityId(3), j(None))]));
    assert!(!ok(vec![(EntityId(1), j(None)), (EntityId(1), j(None))]), "twice");
    assert!(!ok(vec![(EntityId(1), j(Some(1)))]), "its own yard");
    assert!(!ok(vec![(EntityId(1), j(Some(20)))]), "unknown yard");
}
