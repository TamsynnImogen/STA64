//! SIM-26: officer reservation and admission. The installed-data rule check is
//! `installed_rule_matches_the_data`, gated on `STA64_TEST_INSTALL`.

use super::*;
use crate::command::{Command, CommandKind, Edit, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, DefRef, MapDef, MoveStats, UnitDef};
use sta_core::TeamId;
use std::sync::Arc;

const OWNER: PlayerId = PlayerId(0);

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

fn content(pay_on_queue: bool) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.pay_on_queue = pay_on_queue;
    let unit = |name: &str, class: &str, officers: i32| {
        let mut u = UnitDef::new(name, class);
        u.race = "fx".into();
        u.hull = Fixed::from_int(100);
        u.build.officers = n(officers);
        u.build.ticks = 20;
        u.features = Some(features.clone());
        u
    };
    let mut yard = unit("off_yard", "shipyard", 0);
    yard.build.list = vec![DefRef::new("off_ship4"), DefRef::new("off_ship0")];
    let mut rig = unit("off_rig", "constructionrig", 0);
    rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.build.list = vec![DefRef::new("off_station5")];
    let mut builder = ContentDb::builder().profile(fake.profile()).features(features.clone());
    for w in fake.weapons() {
        builder = builder.weapon(w.clone());
    }
    Arc::new(
        builder
            .unit(yard)
            .unit(rig)
            .unit(unit("off_ship4", "craft", 4))
            .unit(unit("off_ship0", "craft", 0))
            .unit(unit("off_station5", "shipyard", 5))
            .build()
            .unwrap(),
    )
}

fn game(pay_on_queue: bool, limit: Option<i32>) -> Sim {
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 26,
        map: Arc::new(MapDef::empty()),
        content: content(pay_on_queue),
        players: vec![PlayerSetup { id: OWNER, team: TeamId(0), race: "fx".into(), control: Control::Human }],
        options: GameOptions::default(),
    });
    sim.economy.set_personnel(OWNER, None, limit.map(n));
    sim
}

fn cmd(kind: CommandKind) -> Command {
    Command { player: OWNER, seq: 0, kind }
}

fn spawn(sim: &mut Sim, odf: &str, x: i32) -> EntityId {
    let def = sim.content().unwrap().key(odf).unwrap();
    let at = Vec3 { x: n(x), y: Fixed::ZERO, z: Fixed::ZERO };
    sim.spawn_def(def, OWNER, at, (Fixed::ZERO, Fixed::ONE)).unwrap()
}

fn fund(sim: &mut Sim, amount: i32) {
    sim.step(&[Command {
        player: PlayerId::BENCH,
        seq: 0,
        kind: CommandKind::Edit(Edit::Cargo(OWNER, 0, n(amount))),
    }]);
}

fn order(sim: &mut Sim, builder: EntityId, what: &str, at: Option<Vec3>) -> bool {
    let what = sim.content().unwrap().key(what).unwrap();
    let out = sim.step(&[cmd(CommandKind::Order(Order::Build { builder, what, at }))]);
    !out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. }))
}

fn officers(sim: &Sim) -> [Fixed; 2] {
    sim.view().player(OWNER).unwrap().officers
}

fn stop(sim: &mut Sim, id: EntityId) {
    sim.step(&[cmd(CommandKind::Order(Order::Stop { units: vec![id] }))]);
}

#[test]
fn pay_on_queue_reserves_at_queue_refuses_over_limit_and_cancel_releases() {
    let mut sim = game(true, Some(10));
    let yard = spawn(&mut sim, "off_yard", 0);
    assert_eq!(officers(&sim), [n(0), n(10)]);
    assert!(order(&mut sim, yard, "off_ship4", None));
    assert!(order(&mut sim, yard, "off_ship4", None));
    // Census is live (0) plus both reservations, though only one has started.
    assert_eq!(officers(&sim), [n(8), n(10)]);
    // 8 + 4 > 10: refused, nothing queued, nothing charged, no reservation.
    let before = (sim.production.queue(yard).len(), officers(&sim));
    assert!(!order(&mut sim, yard, "off_ship4", None));
    assert_eq!((sim.production.queue(yard).len(), officers(&sim)), before);
    // A zero-cost product is never blocked.
    assert!(order(&mut sim, yard, "off_ship0", None));
    assert_eq!(officers(&sim), [n(8), n(10)]);
    stop(&mut sim, yard);
    assert_eq!(officers(&sim), [n(0), n(10)]);
    assert!(order(&mut sim, yard, "off_ship4", None));
}

#[test]
fn deferred_head_waits_for_officers_then_starts_uncharged_until_it_does() {
    let mut sim = game(false, Some(6));
    let yard = spawn(&mut sim, "off_yard", 0);
    fund(&mut sim, 1000);
    let cost = |sim: &Sim| sim.content().unwrap().unit(sim.content().unwrap().key("off_ship4").unwrap()).unwrap().build.cost.clone();
    assert!(cost(&sim).iter().all(|&c| c == Fixed::ZERO), "fixture ships cost no resources");
    // The queue accepts both; only the first fits (4 <= 6), reserved at start.
    assert!(order(&mut sim, yard, "off_ship4", None));
    assert!(order(&mut sim, yard, "off_ship4", None));
    assert_eq!(officers(&sim), [n(4), n(6)]);
    assert_eq!(sim.production.to_rec().queues[0].1.len(), 2);
    for _ in 0..40 {
        sim.step(&[]);
    }
    // First finished and is live (4); the second cannot start: 4 + 4 > 6.
    assert_eq!(sim.production.queue(yard).len(), 1);
    let rec = sim.production.to_rec();
    assert!(!rec.queues[0].1[0].3 && !rec.queues[0].1[0].4, "waiting head: not started, not reserved");
    assert_eq!(officers(&sim), [n(4), n(6)]);
    // Snapshot while waiting; both continue identically once room appears.
    let mut loaded = Sim::from_snapshot(&sim.snapshot(), Some(Arc::clone(sim.content().unwrap()))).unwrap();
    assert_eq!(loaded.digest(), sim.digest());
    for s in [&mut sim, &mut loaded] {
        s.economy.set_personnel(OWNER, None, Some(n(8)));
        s.step(&[]);
        assert_eq!(officers(s), [n(8), n(8)], "the live ship plus the started head");
    }
    assert_eq!(loaded.digest(), sim.digest());
    for _ in 0..30 {
        sim.step(&[]);
        loaded.step(&[]);
        assert_eq!(loaded.digest(), sim.digest());
    }
    assert!(sim.production.queue(yard).is_empty());
    assert_eq!(officers(&sim), [n(8), n(8)]);
}

#[test]
fn census_is_live_plus_reserved_and_survives_a_snapshot_mid_queue() {
    let mut sim = game(true, Some(100));
    let yard = spawn(&mut sim, "off_yard", 0);
    let _ship = spawn(&mut sim, "off_ship4", 500);
    assert!(order(&mut sim, yard, "off_ship4", None));
    assert!(order(&mut sim, yard, "off_ship4", None));
    assert_eq!(officers(&sim), [n(12), n(100)]);
    let mut loaded = Sim::from_snapshot(&sim.snapshot(), Some(Arc::clone(sim.content().unwrap()))).unwrap();
    assert_eq!(officers(&loaded), officers(&sim));
    for tick in 0..60 {
        let (a, b) = (sim.step(&[]), loaded.step(&[]));
        assert_eq!(a.events, b.events, "tick {tick}");
        assert_eq!(a.view.digest, b.view.digest, "tick {tick}");
        // Finishing converts a reservation into a live ship in the same tick.
        assert_eq!(officers(&sim)[0], n(12), "tick {tick}");
    }
    assert!(sim.production.queue(yard).is_empty());
}

#[test]
fn no_configured_limit_never_blocks() {
    for pay in [true, false] {
        let mut sim = game(pay, None);
        let yard = spawn(&mut sim, "off_yard", 0);
        for _ in 0..5 {
            assert!(order(&mut sim, yard, "off_ship4", None));
        }
        assert_eq!(officers(&sim)[1], n(0));
    }
}

#[test]
fn rig_deploy_is_admitted_and_reserved_like_a_queue_item() {
    for pay in [true, false] {
        let mut sim = game(pay, Some(8));
        let rig = spawn(&mut sim, "off_rig", 0);
        let site = Vec3 { x: n(4000), y: Fixed::ZERO, z: Fixed::ZERO };
        assert!(order(&mut sim, rig, "off_station5", Some(site)));
        assert_eq!(officers(&sim)[0], if pay { n(5) } else { n(0) }, "reserved when paid");
        // A second rig's station would take 10 > 8.
        let rig2 = spawn(&mut sim, "off_rig", 100);
        let site2 = Vec3 { x: n(5000), y: Fixed::ZERO, z: Fixed::ZERO };
        if pay {
            assert!(!order(&mut sim, rig2, "off_station5", Some(site2)));
            assert!(!sim.production.is_deploying(rig2));
        } else {
            assert!(order(&mut sim, rig2, "off_station5", Some(site2)));
            // Arrive: the first deploys (5), the second no longer fits (10 > 8) and drops.
            for (r, at) in [(rig, site), (rig2, site2)] {
                sim.units.get_mut(r).unwrap().pos = at;
            }
            sim.step(&[]);
            assert!(!sim.production.is_deploying(rig2));
            assert_eq!(sim.production.to_rec().welds.len(), 1);
        }
        // Cancelling the pending deploy releases a prepaid reservation.
        if pay {
            stop(&mut sim, rig);
            assert_eq!(officers(&sim)[0], n(0));
        }
    }
}

/// Real data: the officer keys and limits of an install, and the rule on its
/// own units. `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`) selects the install.
/// The pair tested is the first producer that lists a non-structure product
/// with an `officerCost`; the limit is set to exactly one product's cost, so
/// the first order must be admitted and the second must not be.
#[test]
fn installed_rule_matches_the_data() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let active = std::env::var("STA64_TEST_MOD").ok();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), active.as_deref()).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let structure = |u: &UnitDef| EntityKind::producer_def(u) || u.class.0.to_ascii_lowercase().contains("station");
    let costly: Vec<&UnitDef> = db.units().filter(|u| u.build.officers > Fixed::ZERO).collect();
    let max = costly.iter().map(|u| u.build.officers).max();
    eprintln!(
        "install={root} mod={active:?} profile={:?} pay_on_queue={} officer_limits={:?} units_with_officerCost={} max_cost={max:?}",
        db.profile(),
        db.features().pay_on_queue,
        db.rules().officer_limits,
        costly.len(),
    );
    let pair = db.units().filter(|b| EntityKind::producer_def(b)).find_map(|b| {
        let p = b.build.list.iter().filter_map(|r| db.unit(r.key)).find(|p| p.build.officers > Fixed::ZERO && !structure(p))?;
        Some((b.odf.clone(), p.odf.clone(), p.build.officers))
    });
    let Some((builder_odf, product_odf, cost)) = pair else {
        eprintln!("  no producer with an officer-costing ship in its build list");
        return;
    };
    let content = Arc::new(db);
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 26,
        map: Arc::new(MapDef::empty()),
        content: Arc::clone(&content),
        players: vec![PlayerSetup { id: OWNER, team: TeamId(0), race: String::new(), control: Control::Human }],
        options: GameOptions::default(),
    });
    sim.economy.set_personnel(OWNER, None, Some(cost));
    let bkey = content.key(&builder_odf).unwrap();
    let yard = sim.spawn_def(bkey, OWNER, Vec3::ZERO, (Fixed::ZERO, Fixed::ONE)).unwrap();
    let pay = content.unit_features(content.unit(bkey).unwrap()).pay_on_queue;
    // The yard may itself carry officerCost; the rule counts it.
    let held = officers(&sim)[0];
    sim.economy.set_personnel(OWNER, None, Some(held + cost));
    for slot in 0..crate::economy::RESOURCES as u8 {
        sim.step(&[Command {
            player: PlayerId::BENCH,
            seq: 0,
            kind: CommandKind::Edit(Edit::Cargo(OWNER, slot, n(1_000_000))),
        }]);
    }
    let first = order(&mut sim, yard, &product_odf, None);
    let second = order(&mut sim, yard, &product_odf, None);
    let rec = sim.production.to_rec();
    let items = &rec.queues[0].1;
    eprintln!(
        "  {builder_odf} -> {product_odf}: officerCost {cost:?}, yard holds {held:?}, limit {:?}, pay_on_queue={pay}: first={first} second={second}; queued={} reserved={:?} census={:?}",
        held + cost,
        items.len(),
        items.iter().map(|i| i.4).collect::<Vec<_>>(),
        officers(&sim),
    );
    assert!(first, "the order that fits is admitted");
    if pay {
        assert!(!second, "pay_on_queue refuses the order over the limit");
        assert_eq!(items.len(), 1);
    } else {
        assert!(second, "deferred mode queues it");
        assert_eq!(items.len(), 2);
        assert!(items[0].4 && !items[1].4, "only the head is reserved; the second waits");
    }
    assert_eq!(officers(&sim), [held + cost, held + cost]);
}
