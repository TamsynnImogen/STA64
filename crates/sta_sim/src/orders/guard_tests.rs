//! LUA-39 guard order, on the SIM-36 stand-in ships (`approach_tests::db`: a
//! 140-impulse destroyer with a 400-range beam, a far-seeing copy `st_seer`
//! and an unarmed `st_target`). All numbers are authored.

use sta_content::Profile;
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, Vec3};

use super::Job;
use crate::command::{Command, CommandKind, Edit, Order};
use crate::event::SimEvent;
use crate::movement::approach_tests;
use crate::{Sim, WorldSource};

const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);
const HZ: u32 = TICK_HZ;

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}
fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: Fixed::ZERO, z: n(z) }
}
fn flat(a: Vec3, b: Vec3) -> Fixed {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}
fn pos(sim: &Sim, id: EntityId) -> Vec3 {
    sim.unit(id).unwrap().pos
}
fn order(player: u8, seq: u32, o: Order) -> Command {
    Command { player: PlayerId(player), seq, kind: CommandKind::Order(o) }
}

fn run(sim: &mut Sim, first: Vec<Command>, ticks: u32) -> Vec<SimEvent> {
    let mut out = Vec::new();
    for i in 0..ticks {
        let cmds = if i == 0 { first.clone() } else { Vec::new() };
        out.extend(WorldSource::step(sim, &cmds).events);
    }
    out
}

/// A guard (`st_seer`, seat 1), its ward (`st_target`, seat 1) and the sim.
fn pair() -> (Sim, std::sync::Arc<sta_content::ContentDb>, EntityId, EntityId) {
    let sim = approach_tests::sim(approach_tests::db(Profile::Sta2Classic, Some(1100)));
    let db = std::sync::Arc::clone(sim.content.as_ref().unwrap());
    let mut sim = sim;
    let ward = sim.spawn_def(db.key("st_target").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    let guard = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(250, 0), AHEAD).unwrap();
    (sim, db, guard, ward)
}

#[test]
fn a_guard_follows_a_moving_ward_and_keeps_its_station() {
    let (mut sim, _db, guard, ward) = pair();
    run(&mut sim, vec![order(1, 0, Order::Guard { units: vec![guard], ward })], 2);
    assert!(matches!(sim.scripted.job(guard), Some(Job::Guard { ward: w }) if *w == ward));
    // The ward flies a long way; the guard keeps within a few spacings of it.
    run(&mut sim, vec![order(1, 1, Order::Move { units: vec![ward], to: v(0, 3000), queue: false, mode: None })], 1);
    let mut worst = Fixed::ZERO;
    for tick in 0..(120 * HZ) {
        run(&mut sim, vec![], 1);
        if tick > 10 * HZ {
            worst = worst.max(flat(pos(&sim, guard), pos(&sim, ward)));
        }
    }
    assert!(pos(&sim, ward).z > n(1000), "the ward did not move: {:?}", pos(&sim, ward));
    assert!(worst < n(600), "the guard lagged {worst:?} behind");
    assert!(matches!(sim.scripted.job(guard), Some(Job::Guard { .. })));
}

#[test]
fn a_guard_takes_the_wards_attacker_and_goes_back_when_it_is_dead() {
    let (mut sim, db, guard, ward) = pair();
    let foe = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(2), v(0, 900), AHEAD).unwrap();
    run(&mut sim, vec![order(1, 0, Order::Guard { units: vec![guard], ward })], 2);
    // The enemy attacks the ward; the guard answers it.
    let ev = run(&mut sim, vec![order(2, 0, Order::Attack { units: vec![foe], target: ward })], 40 * HZ);
    assert!(ev.iter().any(|e| matches!(e, SimEvent::Fired { shooter, .. } if *shooter == guard)), "the guard never fired");
    assert!(sim.weapons.attack_of(guard).is_none_or(|t| t == foe));
    // Kill the attacker: the guard drops the fight and returns to the ward.
    run(&mut sim, vec![Command { player: PlayerId::SCRIPT, seq: 1, kind: CommandKind::Edit(Edit::Kill(foe)) }], 60 * HZ);
    assert!(sim.weapons.attack_of(guard).is_none());
    assert!(flat(pos(&sim, guard), pos(&sim, ward)) < n(400));
    assert!(matches!(sim.scripted.job(guard), Some(Job::Guard { .. })));
}

#[test]
fn a_guard_stops_when_the_ward_dies() {
    let (mut sim, _db, guard, ward) = pair();
    run(&mut sim, vec![order(1, 0, Order::Guard { units: vec![guard], ward })], 2);
    run(&mut sim, vec![order(1, 1, Order::Move { units: vec![ward], to: v(0, 2000), queue: false, mode: None })], 30 * HZ);
    run(&mut sim, vec![Command { player: PlayerId::SCRIPT, seq: 2, kind: CommandKind::Edit(Edit::Kill(ward)) }], 5);
    assert!(sim.scripted.job(guard).is_none(), "the order outlived the ward");
    assert!(sim.unit(guard).unwrap().target.is_none());
    let at = pos(&sim, guard);
    run(&mut sim, vec![], 10 * HZ);
    assert!(flat(at, pos(&sim, guard)) < n(40), "the guard kept flying");
}

#[test]
fn the_order_is_validated_and_replaced_like_the_other_orders() {
    let (mut sim, db, guard, ward) = pair();
    let foe = sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(900, 900), AHEAD).unwrap();
    let take = |sim: &mut Sim, c: Command| WorldSource::step(sim, &[c]).events;
    // Self-guard, an unknown ward and a dead id are refused: no row.
    for (seq, w) in [(0, guard), (1, EntityId(9999))] {
        let ev = take(&mut sim, order(1, seq, Order::Guard { units: vec![guard], ward: w }));
        assert!(sim.scripted.job(guard).is_none());
        assert!(ev.iter().any(|e| matches!(e, SimEvent::Rejected { .. })), "no rejection for ward {w:?}");
    }
    // A player may not guard an enemy craft, nor give the order to another seat's craft.
    take(&mut sim, order(1, 2, Order::Guard { units: vec![guard], ward: foe }));
    assert!(sim.scripted.job(guard).is_none());
    take(&mut sim, order(2, 0, Order::Guard { units: vec![guard], ward: foe }));
    assert!(sim.scripted.job(guard).is_none());
    // The script seat may.
    take(&mut sim, order(PlayerId::SCRIPT.0, 0, Order::Guard { units: vec![guard], ward: foe }));
    assert!(matches!(sim.scripted.job(guard), Some(Job::Guard { ward: w }) if *w == foe));
    // The ward itself in the list is skipped; the rest take it.
    let other = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(-250, 0), AHEAD).unwrap();
    take(&mut sim, order(1, 3, Order::Guard { units: vec![ward, other], ward }));
    assert!(sim.scripted.job(ward).is_none());
    assert!(matches!(sim.scripted.job(other), Some(Job::Guard { .. })));
    // A new move replaces it.
    take(&mut sim, order(1, 4, Order::Move { units: vec![other], to: v(-600, 0), queue: false, mode: None }));
    assert!(sim.scripted.job(other).is_none());
}

#[test]
fn a_guard_order_survives_a_snapshot_byte_for_byte_and_the_loader_refuses_forged_rows() {
    let (mut sim, cdb, guard, ward) = pair();
    run(&mut sim, vec![order(1, 0, Order::Guard { units: vec![guard], ward })], 2);
    run(&mut sim, vec![order(1, 1, Order::Move { units: vec![ward], to: v(0, 700), queue: false, mode: None })], 5 * HZ);
    let db = sim.content.clone();
    let mut loaded = Sim::from_snapshot(&sim.snapshot(), db.clone()).unwrap();
    assert_eq!(loaded.digest(), sim.digest());
    assert_eq!(loaded.snapshot(), sim.snapshot(), "save, load, save");
    for _ in 0..(20 * HZ) {
        assert_eq!(WorldSource::step(&mut sim, &[]).view.digest, WorldSource::step(&mut loaded, &[]).view.digest);
    }
    assert_eq!(loaded.snapshot(), sim.snapshot());
    // Self-guard, an unknown ward and a ward that is no craft are refused on load.
    // A third unit that is no craft: the ward may not be one.
    let other = sim.spawn_def(cdb.key("st_target").unwrap(), PlayerId(1), v(900, 0), AHEAD).unwrap();
    sim.unit_mut(other).unwrap().kind = crate::world::EntityKind::Station;
    assert_ne!(sim.unit(other).unwrap().kind, crate::world::EntityKind::Craft);
    for bad in [guard, EntityId(9999), other] {
        let mut forged = Sim::from_snapshot(&sim.snapshot(), db.clone()).unwrap();
        forged.scripted.rows.insert(guard, Job::Guard { ward: bad });
        assert!(Sim::from_snapshot(&forged.snapshot(), db.clone()).is_err(), "ward {bad:?} accepted");
    }
}

#[test]
fn a_guard_changes_the_digest_and_ending_it_leaves_no_row() {
    let (mut a, _db, guard, ward) = pair();
    let (mut b, ..) = pair();
    run(&mut a, vec![], 30);
    run(&mut b, vec![order(1, 0, Order::Guard { units: vec![guard], ward })], 30);
    assert_ne!(a.digest(), b.digest());
    // Ending the guard again leaves a state without rows hashing as before.
    run(&mut b, vec![order(1, 1, Order::Stop { units: vec![guard] })], 1);
    assert!(b.scripted.job(guard).is_none() && b.scripted.is_empty());
}

/// A pinned whole-state digest sample that contains a guard row (LUA-39). A change of
/// the guard's rules, or of anything the digest covers, moves it on purpose.
#[test]
fn a_whole_state_digest_with_a_guard_is_pinned() {
    let (mut sim, _db, guard, ward) = pair();
    run(&mut sim, vec![order(1, 0, Order::Guard { units: vec![guard], ward })], 2);
    run(&mut sim, vec![order(1, 1, Order::Move { units: vec![ward], to: v(0, 900), queue: false, mode: None })], 20 * HZ);
    assert!(matches!(sim.scripted.job(guard), Some(Job::Guard { .. })));
    assert_eq!(sim.digest(), PINNED_DIGEST);
}

const PINNED_DIGEST: u64 = 0xd038_7997_a920_99f0;
