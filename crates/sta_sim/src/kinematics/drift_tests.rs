//! `engines_off_drift` (#1031): a craft whose engines go offline coasts on its heading with
//! decaying speed, neither turns nor moves under its own power, keeps its order and carries it
//! out when the engines return. On the approach scene's ships (`approach_tests::db`: impulse
//! 140, warp 400), stock Armada II rules; nothing here is game data.

use std::sync::Arc;

use sta_content::{ContentDb, Profile};
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, Vec3};

use super::DRIFT_DECAY;
use crate::command::{Command, CommandKind, Edit, Order};
use crate::movement::approach::IN_WARP;
use crate::movement::approach_tests;
use crate::view::UnitFlags;
use crate::{Sim, WorldSource};

const HZ: u32 = TICK_HZ;
const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}
fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: Fixed::ZERO, z: n(z) }
}

/// The approach scene under `profile`, with the switch as that profile has it.
fn game(profile: Profile) -> (Sim, Arc<ContentDb>, EntityId) {
    let mut sim = approach_tests::sim(approach_tests::db(profile, Some(1100)));
    let db = Arc::clone(sim.content.as_ref().unwrap());
    let ship = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    (sim, db, ship)
}

fn order(o: Order) -> Command {
    Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(o) }
}
fn engines(id: EntityId, on: bool) -> Command {
    Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(Edit::Engines(id, on)) }
}
fn run(sim: &mut Sim, first: Vec<Command>, ticks: u32) {
    for i in 0..ticks {
        sim.step(if i == 0 { &first } else { &[] });
    }
}

#[test]
fn only_stock_armada_ii_has_the_switch_on() {
    let on: Vec<Profile> = [Profile::Native, Profile::Fo4, Profile::Fo3, Profile::Sta2Classic, Profile::Sta1Classic].into_iter().filter(|p| p.features().engines_off_drift).collect();
    assert_eq!(on, [Profile::Sta2Classic]);
}

#[test]
fn a_craft_under_way_coasts_on_its_heading_with_falling_speed_and_keeps_its_order() {
    let (mut sim, _, ship) = game(Profile::Sta2Classic);
    // Under way to a goal ahead and to the side, so a craft under power would be turning.
    let goal = v(3000, 6000);
    run(&mut sim, vec![order(Order::Move { units: vec![ship], to: goal, queue: false, mode: None })], 6 * HZ);
    let before = sim.unit(ship).unwrap().clone();
    assert!(before.speed > n(100), "under way: {:?}", before.speed);

    sim.step(&[engines(ship, false)]);
    let off = sim.unit(ship).unwrap().clone();
    assert!(off.flags.contains(UnitFlags::ENGINES_OFF));
    assert_eq!(off.target, Some(goal), "the order is kept");
    assert!(off.speed > Fixed::ZERO, "it does not stop at once");

    let (mut last, start) = (off.clone(), off.clone());
    for tick in 0..HZ {
        sim.step(&[]);
        let u = sim.unit(ship).unwrap();
        assert_eq!(u.heading, start.heading, "tick {tick}: it does not turn");
        assert!(u.speed < last.speed && u.speed > Fixed::ZERO, "tick {tick}: the speed falls smoothly");
        // Straight on along the heading: the step is the speed it had, nothing sideways.
        let (dx, dz) = (u.pos.x - last.pos.x, u.pos.z - last.pos.z);
        let along = dx * start.heading.0 + dz * start.heading.1;
        let across = dx * start.heading.1 - dz * start.heading.0;
        assert!((along - last.speed / n(HZ as i32)).abs() < Fixed::from_f64(0.001), "tick {tick}: {along:?}");
        assert!(across.abs() < Fixed::from_f64(0.001), "tick {tick}: {across:?}");
        assert_eq!(u.target, Some(goal), "tick {tick}");
        last = u.clone();
    }
    // After one second: the share of the constant is gone, to within the tick-wise rounding.
    let kept = (last.speed / start.speed).to_f32();
    let per_tick = 1.0 - DRIFT_DECAY.to_f32() / HZ as f32;
    let expected = (0..HZ).fold(1.0f32, |kept, _| kept * per_tick);
    assert!((kept - expected).abs() < 0.002, "kept {kept} of the speed in a second, expected {expected}");
}

#[test]
fn the_kept_order_is_carried_out_when_the_engines_return() {
    let (mut sim, _, ship) = game(Profile::Sta2Classic);
    let goal = v(0, 3000);
    run(&mut sim, vec![order(Order::Move { units: vec![ship], to: goal, queue: false, mode: None })], 4 * HZ);
    run(&mut sim, vec![engines(ship, false)], 40 * HZ);
    let adrift = sim.unit(ship).unwrap().clone();
    assert!(adrift.speed < n(1), "it has all but come to rest: {:?}", adrift.speed);
    assert!(adrift.pos.z < n(2500), "short of the goal: {:?}", adrift.pos.z);
    assert_eq!(adrift.target, Some(goal), "still ordered there, however long the engines were off");

    run(&mut sim, vec![engines(ship, true)], 40 * HZ);
    let u = sim.unit(ship).unwrap();
    assert!(u.target.is_none() && (u.pos.z - goal.z).abs() < n(100) && u.pos.x.abs() < n(100), "arrived: {:?}", u.pos);
}

#[test]
fn a_warp_move_resumes_at_warp_by_itself() {
    let (mut sim, _, ship) = game(Profile::Sta2Classic);
    run(&mut sim, vec![order(Order::MoveWarp { units: vec![ship], to: v(0, 7500), queue: false })], 4 * HZ);
    assert_eq!(sim.approach_warp.phase(ship), Some(IN_WARP));
    run(&mut sim, vec![engines(ship, false)], 2 * HZ);
    let u = sim.unit(ship).unwrap();
    assert!(sim.approach_warp.phase(ship) != Some(IN_WARP), "not at warp with the engines off");
    assert!(u.speed <= u.stats.max_speed, "it coasts from impulse, not from warp: {:?}", u.speed);
    assert!(u.speed > Fixed::ZERO && u.target.is_some() && sim.scripted.is_warp(ship), "coasting, the warp move kept");
    let mut warped = false;
    sim.step(&[engines(ship, true)]);
    for _ in 0..8 * HZ {
        sim.step(&[]);
        warped |= sim.approach_warp.phase(ship) == Some(IN_WARP);
    }
    assert!(warped, "back at warp without a new order");
}

#[test]
fn a_craft_at_rest_with_its_engines_off_does_not_move_or_turn() {
    let (mut sim, _, ship) = game(Profile::Sta2Classic);
    sim.step(&[engines(ship, false)]);
    let at = sim.unit(ship).unwrap().clone();
    run(&mut sim, Vec::new(), 2 * HZ);
    let u = sim.unit(ship).unwrap();
    assert_eq!((u.pos, u.heading, u.speed), (at.pos, at.heading, Fixed::ZERO));
}

#[test]
fn engines_that_went_off_without_a_script_coast_the_same() {
    // The flag as damage sets it (engines destroyed), not through the script edit.
    let (mut sim, _, ship) = game(Profile::Sta2Classic);
    run(&mut sim, vec![order(Order::Move { units: vec![ship], to: v(0, 6000), queue: false, mode: None })], 4 * HZ);
    let u = sim.units.get_mut(ship).unwrap();
    u.flags = u.flags.with(UnitFlags::ENGINES_OFF, true);
    let (speed, z) = (u.speed, u.pos.z);
    run(&mut sim, Vec::new(), HZ);
    let u = sim.unit(ship).unwrap();
    assert!(u.speed < speed && u.speed > Fixed::ZERO && u.pos.z > z + n(50), "{:?} at {:?}", u.speed, u.pos.z);
}

#[test]
fn with_the_switch_off_the_craft_stops_where_it_is_and_loses_its_order() {
    for profile in [Profile::Native, Profile::Fo4] {
        let (mut sim, db, ship) = game(profile);
        assert!(!db.features().engines_off_drift);
        run(&mut sim, vec![order(Order::Move { units: vec![ship], to: v(0, 6000), queue: false, mode: None })], 4 * HZ);
        assert!(sim.unit(ship).unwrap().speed > Fixed::ZERO, "{profile:?}: under way");
        sim.step(&[engines(ship, false)]);
        let at = sim.unit(ship).unwrap().clone();
        assert_eq!((at.speed, at.target), (Fixed::ZERO, None), "{profile:?}");
        run(&mut sim, Vec::new(), HZ);
        assert_eq!(sim.unit(ship).unwrap().pos, at.pos, "{profile:?}: it stays where it stopped");
    }
}

#[test]
fn a_snapshot_taken_mid_drift_reloads_to_the_same_flight() {
    let (mut sim, db, ship) = game(Profile::Sta2Classic);
    run(&mut sim, vec![order(Order::Move { units: vec![ship], to: v(2000, 6000), queue: false, mode: None })], 5 * HZ);
    run(&mut sim, vec![engines(ship, false)], HZ);
    let mut back = Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap();
    assert_eq!(back.digest(), sim.digest());
    for tick in 0..3 * HZ {
        // The engines come back half way: the reloaded game takes the order up the same way.
        let cmds = if tick == HZ { vec![engines(ship, true)] } else { Vec::new() };
        sim.step(&cmds);
        back.step(&cmds);
        assert_eq!(back.digest(), sim.digest(), "tick {tick}");
    }
}

/// A craft at warp whose engines go off is at impulse speed on the next tick and coasts from
/// there: it does not carry its warp speed across the map.
#[test]
fn a_craft_at_warp_drops_to_impulse_before_it_coasts() {
    let (mut sim, _, ship) = game(Profile::Sta2Classic);
    run(&mut sim, vec![order(Order::MoveWarp { units: vec![ship], to: v(0, 7500), queue: false })], 4 * HZ);
    let at_warp = sim.unit(ship).unwrap().clone();
    assert!(at_warp.speed > at_warp.stats.max_speed, "at warp: {:?}", at_warp.speed);
    sim.step(&[engines(ship, false)]);
    sim.step(&[]);
    let u = sim.unit(ship).unwrap();
    assert!(u.speed <= u.stats.max_speed && u.speed > Fixed::ZERO, "one tick on: {:?}", u.speed);
    run(&mut sim, Vec::new(), 60 * HZ);
    // The whole coast is what impulse speed gives under the decay: the speed over the rate.
    let far = sim.unit(ship).unwrap().pos.z - at_warp.pos.z;
    let limit = at_warp.stats.max_speed / DRIFT_DECAY + n(50);
    assert!(far > n(100) && far < limit, "coasted {far:?}, at most {limit:?} from impulse");
}

/// Puts the craft at `at`, heading along +z at impulse speed with its engines off: a craft
/// that is coasting, with no order.
fn adrift(sim: &mut Sim, ship: EntityId, at: Vec3) {
    let u = sim.units.get_mut(ship).unwrap();
    u.pos = at;
    u.heading = AHEAD;
    u.speed = u.stats.max_speed;
    u.flags = u.flags.with(UnitFlags::ENGINES_OFF, true);
}

/// The approach scene under the native rules with the drift switch forced on: rules under
/// which no map confinement is active.
fn game_without_confinement() -> (Sim, EntityId) {
    let base = approach_tests::db(Profile::Native, Some(1100));
    let mut features = base.features().clone();
    features.engines_off_drift = true;
    let mut b = ContentDb::builder().profile(base.profile()).features(features);
    for w in base.weapons() {
        b = b.weapon(w.clone());
    }
    for u in base.units() {
        b = b.unit(u.clone());
    }
    let db = Arc::new(b.build().unwrap().with_rules(base.rules().clone()));
    let mut sim = approach_tests::sim_with(Arc::clone(&db));
    let ship = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    (sim, ship)
}

/// A coasting craft is kept out of an asteroid field, as a craft under power is: flying
/// straight at one, it comes to the field's edge and no further, with the map confinement
/// active (stock Armada II rules) and without it. The coast adds no rule of its own for
/// fields: what already keeps units out of them holds a coasting craft too.
#[test]
fn a_coasting_craft_does_not_drift_into_an_asteroid_field() {
    use sta_content::{Region, RegionKind, RegionShape};
    let (stock, _, stock_ship) = game(Profile::Sta2Classic);
    let (bare, bare_ship) = game_without_confinement();
    assert!(stock.confinement.is_active() && !bare.confinement.is_active());
    for (what, mut sim, ship) in [("with confinement", stock, stock_ship), ("without confinement", bare, bare_ship)] {
        // A field straight ahead, well inside the distance the coast covers.
        let (centre, radius) = (v(0, 1000), 300);
        sim.regions.push(Region { kind: RegionKind::AsteroidField, odf: "drift_rocks".into(), shape: RegionShape::Sphere { centre, radius: n(radius) } });
        adrift(&mut sim, ship, v(0, 400));
        let mut nearest = Fixed::from_int(i32::MAX);
        for _ in 0..40 * HZ {
            sim.step(&[]);
            let p = sim.unit(ship).unwrap().pos;
            let (dx, dz) = (p.x - centre.x, p.z - centre.z);
            nearest = nearest.min((dx * dx + dz * dz).sqrt());
        }
        assert!(nearest < n(radius + 50), "{what}: the coast reached the field's edge: {nearest:?} from its centre");
        assert!(nearest >= n(radius), "{what}: and stayed out of the field: {nearest:?} from its centre");
    }
}

/// A coasting craft is held on the map like every other unit (the map's own confinement; the
/// coast adds no rule of its own for the edge).
#[test]
fn a_coasting_craft_stays_on_the_map() {
    let (mut sim, _, ship) = game(Profile::Sta2Classic);
    let (_, hi) = sim.bounds.expect("the map has bounds");
    adrift(&mut sim, ship, Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: hi.z - n(600) });
    let mut farthest = Fixed::ZERO;
    for _ in 0..40 * HZ {
        sim.step(&[]);
        farthest = farthest.max(sim.unit(ship).unwrap().pos.z);
    }
    assert!(farthest > hi.z - n(300), "the coast reached the edge region: {farthest:?}");
    assert!(farthest <= hi.z, "and never left the map: {farthest:?} beyond {:?}", hi.z);
}

/// A dying craft whose engines are off as well is moved once a tick, by the dying drift alone:
/// the movement step leaves a dying craft to it. Both drifts are the one function, so the step
/// is the speed along the heading and the decay is the one rate.
#[test]
fn a_dying_craft_with_its_engines_off_is_moved_once_a_tick() {
    let (mut sim, db, ship) = game(Profile::Sta2Classic);
    assert!(db.features().dying_phase && db.features().engines_off_drift);
    run(&mut sim, vec![order(Order::Move { units: vec![ship], to: v(0, 6000), queue: false, mode: None })], 4 * HZ);
    sim.step(&[engines(ship, false)]);
    let kill = Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(Edit::Damage(ship, n(1_000_000))) };
    sim.step(&[kill]);
    assert!(sim.dying_ends(ship).is_some(), "the craft is in its dying phase");
    assert!(sim.unit(ship).unwrap().flags.contains(UnitFlags::ENGINES_OFF));
    let hz = n(HZ as i32);
    let mut last = sim.unit(ship).unwrap().clone();
    assert!(last.speed > Fixed::ZERO);
    for tick in 0..HZ {
        sim.step(&[]);
        let Some(u) = sim.unit(ship).cloned() else { break };
        assert_eq!(u.pos.z, last.pos.z + last.heading.1 * last.speed / hz, "tick {tick}: one step of the speed, not two");
        assert_eq!(u.speed, last.speed - last.speed * DRIFT_DECAY / hz, "tick {tick}: the decay applied once");
        last = u;
    }
}
