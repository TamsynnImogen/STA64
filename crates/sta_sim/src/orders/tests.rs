//! SIM-56 scripted orders and SIM-58 switches, on the SIM-36 stand-in ships
//! (`approach_tests::db`: a 140-impulse destroyer with a 400-range beam, a
//! far-seeing copy `st_seer` and an unarmed `st_target`).

use std::sync::Arc;

use sta_content::Profile;
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, Vec3};

use crate::command::{Command, CommandKind, Edit, Order};
use crate::event::SimEvent;
use crate::movement::approach_tests;
use crate::{Sim, WorldSource};

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}
fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: Fixed::ZERO, z: n(z) }
}
const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);
const HZ: u32 = TICK_HZ;

fn game(profile: Profile) -> (Sim, Arc<sta_content::ContentDb>) {
    let sim = approach_tests::sim(approach_tests::db(profile, Some(1100)));
    let db = Arc::clone(sim.content.as_ref().unwrap());
    (sim, db)
}

/// [`game`] on a map of this half-size (the map's bounds, as a real game has them).
fn game_on(half: i32) -> (Sim, Arc<sta_content::ContentDb>) {
    use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    let db = Arc::new(approach_tests::db(Profile::Sta2Classic, Some(1100)));
    let mut map = sta_content::MapDef::empty();
    map.bounds = (v(-half, -half), v(half, half));
    let seat = |id| PlayerSetup { id: PlayerId(id), team: sta_core::TeamId(id), race: "st".into(), control: Control::Idle };
    let sim = Sim::from_setup(&GameSetup { seed: 5, map: Arc::new(map), content: Arc::clone(&db), players: vec![seat(1), seat(2)], options: GameOptions::default() });
    (sim, db)
}

fn order(player: u8, seq: u32, o: Order) -> Command {
    Command { player: PlayerId(player), seq, kind: CommandKind::Order(o) }
}
fn edit(seq: u32, e: Edit) -> Command {
    Command { player: PlayerId::SCRIPT, seq, kind: CommandKind::Edit(e) }
}

/// Step `ticks` ticks, the commands going in on the first; every event.
fn run(sim: &mut Sim, first: Vec<Command>, ticks: u32) -> Vec<SimEvent> {
    let mut out = Vec::new();
    for i in 0..ticks {
        let cmds = if i == 0 { first.clone() } else { Vec::new() };
        out.extend(WorldSource::step(sim, &cmds).events);
    }
    out
}

fn fired_by(events: &[SimEvent], who: EntityId) -> usize {
    events.iter().filter(|e| matches!(e, SimEvent::Fired { shooter, .. } if *shooter == who)).count()
}

fn pos(sim: &Sim, id: EntityId) -> Vec3 {
    sim.unit(id).unwrap().pos
}

fn flat(a: Vec3, b: Vec3) -> Fixed {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

#[test]
fn patrol_loops_the_path_from_its_first_point_for_ever_without_being_ended_as_stuck() {
    let (mut sim, db) = game(Profile::Sta2Classic);
    let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    let pts = vec![v(0, 400), v(400, 400), v(400, 0)];
    let o = order(1, 0, Order::Patrol { units: vec![s], points: pts.clone() });
    let mut laps = 0;
    let mut last_leg = 0;
    let mut events = Vec::new();
    // Ten minutes: far past the stuck-order watch's two-minute limit.
    for i in 0..(600 * HZ) {
        events.extend(WorldSource::step(&mut sim, &if i == 0 { vec![o.clone()] } else { vec![] }).events);
        if let Some(crate::orders::Job::Patrol { leg, .. }) = sim.scripted.job(s) {
            if *leg == 0 && last_leg != 0 {
                laps += 1;
            }
            last_leg = *leg;
        }
    }
    assert!(laps >= 20, "laps {laps}");
    assert!(sim.unit(s).unwrap().target.is_some());
    assert!(!events.iter().any(|e| matches!(e, SimEvent::OrderAbandoned { .. })), "ended as stuck");
    assert!(matches!(sim.scripted.job(s), Some(crate::orders::Job::Patrol { .. })));
}

#[test]
fn a_new_order_ends_the_patrol_and_a_one_point_patrol_is_a_move() {
    let (mut sim, db) = game(Profile::Sta2Classic);
    let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    run(&mut sim, vec![order(1, 0, Order::Patrol { units: vec![s], points: vec![v(0, 300), v(300, 300)] })], 30);
    assert!(sim.scripted.job(s).is_some());
    run(&mut sim, vec![order(1, 1, Order::Stop { units: vec![s] })], 2);
    assert!(sim.scripted.job(s).is_none() && sim.unit(s).unwrap().target.is_none());
    run(&mut sim, vec![order(1, 2, Order::Patrol { units: vec![s], points: vec![v(0, 200)] })], 60 * HZ);
    assert!(sim.scripted.job(s).is_none());
    assert!(flat(pos(&sim, s), v(0, 200)) < n(30));
    // An order from someone else's seat is refused.
    assert!(!matches!(
        run(&mut sim, vec![order(2, 0, Order::Patrol { units: vec![s], points: vec![v(0, 0), v(5, 5)] })], 1).as_slice(),
        []
    ) || sim.scripted.job(s).is_none());
}

#[test]
fn a_patrol_fires_at_an_enemy_in_range_without_leaving_its_path() {
    let (mut sim, db) = game(Profile::Sta2Classic);
    let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(200, 500), AHEAD).unwrap();
    let ev = run(&mut sim, vec![order(1, 0, Order::Patrol { units: vec![s], points: vec![v(0, 1000), v(0, 0)] })], 20 * HZ);
    assert!(fired_by(&ev, s) > 0);
    assert!(pos(&sim, s).x.abs() < n(150), "left the path: {:?}", pos(&sim, s));
    assert!(sim.scripted.job(s).is_some());
}

#[test]
fn search_and_destroy_fights_what_it_sees_and_roams_when_nothing_is() {
    let (mut sim, db) = game_on(3000);
    let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    let hunt = order(1, 0, Order::SearchAndDestroy { units: vec![s] });
    // Nothing to see: it picks a destination and flies.
    run(&mut sim, vec![hunt.clone()], 10 * HZ);
    assert!(sim.unit(s).unwrap().target.is_some() || flat(pos(&sim, s), v(0, 0)) > n(100));
    run(&mut sim, vec![], 30 * HZ);
    assert!(flat(pos(&sim, s), v(0, 0)) > n(200));
    // An enemy turns up: it closes in and fires.
    let t = sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(0, 2500), AHEAD).unwrap();
    let ev = run(&mut sim, vec![], 120 * HZ);
    assert!(fired_by(&ev, s) > 0, "never fired");
    assert!(flat(pos(&sim, s), pos(&sim, t)) < n(450));
    assert!(matches!(sim.scripted.job(s), Some(crate::orders::Job::SearchAndDestroy)));
}

#[test]
fn attack_without_a_target_engages_then_stops_when_nothing_is_seen() {
    let (mut sim, db) = game(Profile::Sta2Classic);
    let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    // No enemy: the order ends at once.
    run(&mut sim, vec![order(1, 0, Order::AttackAny { units: vec![s] })], 3);
    assert!(sim.scripted.job(s).is_none() && sim.unit(s).unwrap().target.is_none());
    let t = sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(0, 1500), AHEAD).unwrap();
    let ev = run(&mut sim, vec![order(1, 1, Order::AttackAny { units: vec![s] })], 40 * HZ);
    assert!(fired_by(&ev, s) > 0);
    assert!(flat(pos(&sim, s), pos(&sim, t)) < n(450));
    assert!(sim.scripted.job(s).is_some(), "keeps fighting while it sees the enemy");
}

#[test]
fn attack_at_a_point_flies_there_firing_nothing_then_stops() {
    let (mut sim, db) = game(Profile::Sta2Classic);
    let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(100, 600), AHEAD).unwrap();
    let go = order(1, 0, Order::AttackPoint { units: vec![s], at: v(0, 1500) });
    let mut fired_on_the_way = 0;
    let mut ev = Vec::new();
    for i in 0..(60 * HZ) {
        ev.extend(WorldSource::step(&mut sim, &if i == 0 { vec![go.clone()] } else { vec![] }).events);
        if sim.scripted.job(s).is_some() {
            fired_on_the_way = fired_by(&ev, s);
        }
    }
    assert_eq!(fired_on_the_way, 0);
    assert!(sim.scripted.job(s).is_none());
    let d = flat(pos(&sim, s), v(0, 1500));
    assert!(d <= n(360) && d > n(150), "stopped {d:?} from the point");
}

#[test]
fn a_scout_never_fires_stays_near_its_start_and_never_ends() {
    let (mut sim, db) = game_on(4000);
    let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(0, 300), AHEAD).unwrap();
    let ev = run(&mut sim, vec![order(1, 0, Order::Scout { units: vec![s] })], 120 * HZ);
    assert_eq!(fired_by(&ev, s), 0);
    assert!(sim.scripted.job(s).is_some());
    assert!(flat(pos(&sim, s), v(0, 0)) < n(1500), "wandered off: {:?}", pos(&sim, s));
    assert!(flat(pos(&sim, s), v(0, 0)) > n(20));
}

#[test]
fn scripted_orders_replay_deterministically_and_survive_a_snapshot() {
    let make = || {
        let (mut sim, db) = game_on(3000);
        let a = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
        let b = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(50, 0), AHEAD).unwrap();
        let c = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(100, 0), AHEAD).unwrap();
        sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(0, 900), AHEAD).unwrap();
        let cmds = vec![
            order(1, 0, Order::Patrol { units: vec![a], points: vec![v(0, 400), v(400, 400)] }),
            order(1, 1, Order::Scout { units: vec![b] }),
            order(1, 2, Order::SearchAndDestroy { units: vec![c] }),
        ];
        run(&mut sim, cmds, 20 * HZ);
        (sim, db)
    };
    let (mut x, db) = make();
    let (mut y, _) = make();
    assert_eq!(x.digest(), y.digest());
    let mut z = Sim::from_snapshot(&x.snapshot(), Some(db)).unwrap();
    assert_eq!(z.digest(), x.digest());
    for tick in 0..400 {
        let (a, b, c) = (x.step(&[]), y.step(&[]), z.step(&[]));
        assert_eq!(a.view.digest, b.view.digest, "tick {tick}");
        assert_eq!(a.view.digest, c.view.digest, "tick {tick}");
    }
    assert_eq!(x.snapshot(), z.snapshot());
}

#[test]
fn a_warp_move_warps_on_a_long_leg_a_plain_move_does_not() {
    let top = |warp: bool| {
        let (mut sim, db) = game(Profile::Sta2Classic);
        let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
        let o = if warp {
            Order::MoveWarp { units: vec![s], to: v(0, 6000), queue: false }
        } else {
            Order::Move { units: vec![s], to: v(0, 6000), queue: false, mode: None }
        };
        let mut best = Fixed::ZERO;
        for i in 0..(20 * HZ) {
            WorldSource::step(&mut sim, &if i == 0 { vec![order(1, 0, o.clone())] } else { vec![] });
            best = best.max(sim.unit(s).unwrap().speed);
        }
        (best, flat(pos(&sim, s), v(0, 6000)))
    };
    let (w, wd) = top(true);
    let (p, pd) = top(false);
    assert!(w > n(300), "no warp {w:?}");
    assert!(p <= n(141));
    // 6000 units in 20 s: warp (400) is there, impulse (140) is not.
    assert!(wd < n(100) && pd > n(2000), "{wd:?} {pd:?}");
}

#[test]
fn a_warp_move_drops_out_before_turning_and_reenters_once_when_aligned() {
    use crate::movement::approach::{IN_WARP, WARP_OUT, warp_leg};

    let (mut sim, db) = game(Profile::Sta2Classic);
    let ship = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    let move_warp = |seq, to| order(1, seq, Order::MoveWarp { units: vec![ship], to, queue: false });
    run(&mut sim, vec![move_warp(0, v(0, 5000))], 4 * HZ);
    assert_eq!(sim.approach_warp.phase(ship), Some(IN_WARP));
    let impulse = sim.unit(ship).unwrap().stats.max_speed;
    assert!(sim.unit(ship).unwrap().speed > impulse);

    sim.step(&[move_warp(1, v(0, 7000))]);
    assert_eq!(sim.approach_warp.phase(ship), Some(IN_WARP), "aligned extension keeps its warp phase");

    let to = Vec3 { x: n(4000), y: Fixed::ZERO, z: sim.unit(ship).unwrap().pos.z };
    let new_course = [move_warp(2, to)];
    let mut previous_phase = Some(IN_WARP);
    let mut transitions = Vec::new();
    let mut turned = false;
    let mut cruise_ticks = 0;
    for tick in 0..60 * HZ {
        let unit = sim.unit(ship).unwrap();
        let previous_heading = unit.heading;
        let previous_speed = unit.speed;
        let previously_aligned = warp_leg(unit, to).1;
        sim.step(if tick == 0 { &new_course } else { &[] });
        let phase = sim.approach_warp.phase(ship);
        let unit = sim.unit(ship).unwrap();
        if unit.heading != previous_heading {
            assert!(previous_speed <= impulse, "turned above impulse at tick {tick}");
            turned = true;
        }
        if phase != previous_phase {
            if phase == Some(IN_WARP) {
                assert!(turned && previously_aligned && previous_speed <= impulse);
            }
            transitions.push(phase);
            previous_phase = phase;
        }
        if phase == Some(IN_WARP) {
            cruise_ticks += 1;
        }
        if unit.target.is_none() && unit.speed == Fixed::ZERO {
            break;
        }
    }
    let unit = sim.unit(ship).unwrap();
    assert!(unit.target.is_none() && unit.speed == Fixed::ZERO, "the redirected order finishes");
    assert_eq!(transitions, [Some(WARP_OUT), None, Some(IN_WARP), Some(WARP_OUT), None]);
    assert!(cruise_ticks >= 2 * HZ);
}

#[test]
fn a_warp_move_brakes_when_avoidance_plans_a_turn() {
    use crate::movement::approach::{IN_WARP, WARP_OUT};
    use sta_content::{Region, RegionKind, RegionShape};

    let (mut sim, db) = game(Profile::Sta2Classic);
    let ship = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    let goal = v(0, 6000);
    let start = order(1, 0, Order::MoveWarp { units: vec![ship], to: goal, queue: false });
    run(&mut sim, vec![start], 4 * HZ);
    assert_eq!(sim.approach_warp.phase(ship), Some(IN_WARP));

    let before = sim.unit(ship).unwrap().clone();
    sim.regions.push(Region {
        kind: RegionKind::AsteroidField,
        odf: "turn_fixture".into(),
        shape: RegionShape::Sphere { centre: Vec3 { z: before.pos.z + n(600), ..before.pos }, radius: n(280) },
    });
    sim.step(&[]);
    let after = sim.unit(ship).unwrap();
    assert_eq!(sim.approach_warp.phase(ship), Some(WARP_OUT));
    assert_eq!(after.heading, before.heading, "braking holds the warp heading");
    assert!(after.speed < before.speed);
    assert_eq!(after.target, Some(goal), "the detour keeps the original order");
}

fn shooter_vs_target(profile: Profile) -> (Sim, EntityId, EntityId) {
    let (mut sim, db) = game(profile);
    let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    let t = sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(0, 300), AHEAD).unwrap();
    (sim, s, t)
}

#[test]
fn allow_as_target_blocks_automatic_picks_by_that_seat_only() {
    for profile in [Profile::Sta2Classic, Profile::Fo4] {
        let block = |t, player| edit(0, Edit::AllowAsTarget { id: t, player: PlayerId(player), allowed: false });
        let (mut sim, s, _) = shooter_vs_target(profile);
        assert!(fired_by(&run(&mut sim, vec![], 3 * HZ), s) > 0, "{profile:?}: baseline fires");
        // Blocked for the shooter's seat from the start: it never picks it.
        let (mut sim, s, t) = shooter_vs_target(profile);
        assert_eq!(fired_by(&run(&mut sim, vec![block(t, 1)], 8 * HZ), s), 0, "{profile:?}: blocked");
        // An explicit attack order still fires at it.
        let atk = order(1, 0, Order::Attack { units: vec![s], target: t });
        assert!(fired_by(&run(&mut sim, vec![atk], 5 * HZ), s) > 0, "{profile:?}: attack order not blocked");
        // Another seat's mask is not this seat's.
        let (mut sim, s, t) = shooter_vs_target(profile);
        assert!(fired_by(&run(&mut sim, vec![block(t, 2)], 3 * HZ), s) > 0, "{profile:?}: other seat");
        // Allowed again: the mask clears and the row goes with it.
        let (mut sim, _, t) = shooter_vs_target(profile);
        run(&mut sim, vec![block(t, 1), edit(1, Edit::AllowAsTarget { id: t, player: PlayerId(1), allowed: true })], 1);
        assert_eq!(sim.switches(t).unwrap(), crate::switches::Switches::default());
    }
}

#[test]
fn switch_edits_store_apply_and_survive_a_snapshot() {
    let (mut sim, s, t) = shooter_vs_target(Profile::Sta2Classic);
    let before = sim.digest();
    let cmds = vec![
        edit(0, Edit::CraftCanRepair(s, false)),
        edit(1, Edit::DerelictAllowed(s, true)),
        edit(2, Edit::CanRecrew(s, false)),
        edit(3, Edit::Hidden(s, true)),
        edit(4, Edit::Name(s, "Voyager".into())),
        edit(5, Edit::SpecialForces(s, true)),
    ];
    let ev = run(&mut sim, cmds, 1);
    assert!(!ev.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    let sw = sim.switches(s).unwrap();
    assert!(sw.no_repair && sw.derelict_ok && sw.recrew == Some(false) && sw.hidden && sw.special_forces);
    assert_eq!(sw.name.as_deref(), Some("Voyager"));
    assert_ne!(sim.digest(), before);
    // Over-long names, dead handles and non-seat masks are rejected.
    let bad = run(&mut sim, vec![edit(6, Edit::Name(s, "x".repeat(65))), edit(7, Edit::CraftCanRepair(EntityId(999), true)), edit(8, Edit::AllowAsTarget { id: t, player: PlayerId::SCRIPT, allowed: false })], 1);
    assert_eq!(bad.iter().filter(|e| matches!(e, SimEvent::Rejected { .. })).count(), 3);
    let db = Arc::clone(sim.content.as_ref().unwrap());
    let loaded = Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap();
    assert_eq!(loaded.switches(s), sim.switches(s));
    assert_eq!(loaded.digest(), sim.digest());
    // A change of owner clears the special-forces flag (`Craft::SetTeam`).
    run(&mut sim, vec![edit(9, Edit::Team(s, PlayerId(2)))], 1);
    assert!(!sim.switches(s).unwrap().special_forces && sim.switches(s).unwrap().hidden);
    // Players may not edit.
    let by_player = Command { player: PlayerId(1), seq: 10, kind: CommandKind::Edit(Edit::Hidden(t, true)) };
    run(&mut sim, vec![by_player], 1);
    assert!(!sim.switches(t).unwrap().hidden);
}

#[test]
fn craft_can_repair_false_stops_shield_regeneration() {
    let shields = |can: bool| {
        let db = Arc::new(sta_content::fake::db());
        let mut sim = approach_tests::sim(sta_content::fake::db());
        let _ = &db;
        let id = sim.spawn_def(sim.content.as_ref().unwrap().key("fx_cruiser").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
        sim.units.get_mut(id).unwrap().shields.now = [Fixed::ZERO; 4];
        run(&mut sim, vec![edit(0, Edit::CraftCanRepair(id, can))], 2 * HZ);
        sim.unit(id).unwrap().shields.now[0]
    };
    assert!(shields(true) > Fixed::ZERO);
    assert_eq!(shields(false), Fixed::ZERO);
}

#[test]
fn another_players_order_cannot_cancel_a_scripted_order_and_assist_skips_scripted_crafts() {
    let (mut sim, db) = game_on(3000);
    let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    run(&mut sim, vec![order(1, 0, Order::Scout { units: vec![s] })], 2);
    // Seat 2 orders seat 1's craft: refused, the scout keeps its order.
    run(&mut sim, vec![order(2, 0, Order::Stop { units: vec![s] }), order(2, 1, Order::Move { units: vec![s], to: v(5, 5), queue: false, mode: None })], 2);
    assert!(matches!(sim.scripted.job(s), Some(crate::orders::Job::Scout { .. })));
    // A queued move replaces a patrol.
    run(&mut sim, vec![order(1, 1, Order::Patrol { units: vec![s], points: vec![v(0, 300), v(300, 0)] })], 2);
    run(&mut sim, vec![order(1, 2, Order::Move { units: vec![s], to: v(5, 5), queue: true, mode: None })], 1);
    assert!(sim.scripted.job(s).is_none());
    // An empty path changes nothing.
    run(&mut sim, vec![order(1, 3, Order::Scout { units: vec![s] })], 1);
    run(&mut sim, vec![order(1, 4, Order::Patrol { units: vec![s], points: vec![] })], 1);
    assert!(matches!(sim.scripted.job(s), Some(crate::orders::Job::Scout { .. })));
}

/// SIM-64: an Armada I scout (impulse 20, warp 100, `tooCloseForWarp` 10,
/// `headingCloseEnoughForWarp` 30) on a map-wide plain move warps; the same
/// ship without the keys (Armada II physics) does not.
mod auto_warp {
    use super::*;
    use sta_content::{ContentDb, MoveStats};

    fn scout_game(keys: bool) -> (Sim, EntityId) {
        let base = sta_content::fake::db();
        let mut b = ContentDb::builder().profile(base.profile()).features(base.features().clone());
        for w in base.weapons().cloned() {
            b = b.weapon(w);
        }
        for mut u in base.units().cloned() {
            if u.odf == "fx_frigate" {
                u.movement = MoveStats::from_f32(20.0, 1.0, 1.0, 1.0, 1.0);
                u.movement.warp_speed = n(100);
                if keys {
                    u.movement.too_close_for_warp = n(10);
                    u.movement.heading_close_for_warp = n(30);
                }
            }
            b = b.unit(u);
        }
        let db = Arc::new(b.build().unwrap());
        let mut sim = approach_tests::sim_with(Arc::clone(&db));
        let s = sim.spawn_def(db.key("fx_frigate").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
        (sim, s)
    }

    /// Top speed and ticks to arrive (within 60) for a plain move of `dist`.
    fn go(keys: bool, dist: i32, secs: u32) -> (Fixed, Option<u32>) {
        let (mut sim, s) = scout_game(keys);
        let to = v(0, dist);
        let (mut top, mut arrived) = (Fixed::ZERO, None);
        for i in 0..secs * HZ {
            WorldSource::step(&mut sim, &if i == 0 { vec![order(1, 0, Order::Move { units: vec![s], to, queue: false, mode: None })] } else { vec![] });
            top = top.max(sim.unit(s).unwrap().speed);
            if arrived.is_none() && flat(pos(&sim, s), to) < n(60) {
                arrived = Some(i);
            }
        }
        (top, arrived)
    }

    #[test]
    fn a_long_plain_move_warps_with_the_keys_and_never_without() {
        let (top, arrived) = go(true, 3000, 60);
        assert!(top > n(90), "no warp {top:?}");
        // 3000 units at warp 100: about 30 s plus the ramp; at impulse 20 it would take 150 s.
        let secs = arrived.expect("arrives") / HZ;
        assert!((25..=45).contains(&secs), "{secs} s");
        let (top, arrived) = go(false, 3000, 60);
        assert!(top <= n(21) && arrived.is_none(), "{top:?} {arrived:?}");
    }

    #[test]
    fn a_short_plain_move_stays_at_impulse() {
        // Inside the engine's drop-out lead the craft never warps.
        let (top, _) = go(true, 50, 20);
        assert!(top <= n(21), "{top:?}");
    }

    #[test]
    fn a_goal_off_the_bow_is_reached_not_missed() {
        let (mut sim, s) = scout_game(true);
        let to = v(1500, 3000); // about 27 degrees off the bow
        for i in 0..80 * HZ {
            WorldSource::step(&mut sim, &if i == 0 { vec![order(1, 0, Order::Move { units: vec![s], to, queue: false, mode: None })] } else { vec![] });
        }
        assert!(flat(pos(&sim, s), to) < n(80), "{:?}", flat(pos(&sim, s), to));
        assert!(sim.scripted.job(s).is_none(), "warp job outlives the move");
    }

    #[test]
    fn a_new_order_ends_the_warp_and_a_snapshot_keeps_it() {
        let (mut sim, s) = scout_game(true);
        let cmd = order(1, 0, Order::Move { units: vec![s], to: v(0, 3000), queue: false, mode: None });
        for i in 0..10 * HZ {
            WorldSource::step(&mut sim, &if i == 0 { vec![cmd.clone()] } else { vec![] });
        }
        assert!(sim.unit(s).unwrap().speed > n(90));
        let db = Arc::clone(sim.content.as_ref().unwrap());
        let mut loaded = Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap();
        assert_eq!(loaded.digest(), sim.digest());
        for _ in 0..HZ {
            assert_eq!(WorldSource::step(&mut sim, &[]).view.digest, WorldSource::step(&mut loaded, &[]).view.digest);
        }
        // A stop leaves warp: the speed falls to nothing and stays there.
        run(&mut sim, vec![order(1, 1, Order::Stop { units: vec![s] })], 30 * HZ);
        assert!(sim.unit(s).unwrap().speed < n(1));
    }
}

/// The legs a craft still has: its current target, then its queued legs.
fn legs(sim: &Sim, id: EntityId) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = sim.unit(id).and_then(|u| u.target).into_iter().collect();
    out.extend(sim.movement.course(id).map(|c| c.waypoints.clone()).unwrap_or_default());
    out
}

/// #1173: a queued warp move is appended; only an unqueued one replaces the course.
#[test]
fn a_queued_warp_move_keeps_the_earlier_legs_and_appends() {
    let (mut sim, db) = game(Profile::Sta2Classic);
    let ship = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    let warp = |seq, to, queue| order(1, seq, Order::MoveWarp { units: vec![ship], to, queue });
    let (a, b, c) = (v(0, 5000), v(3000, 5000), v(3000, 0));

    // An idle craft starts for a queued leg at once.
    sim.step(&[warp(0, a, true)]);
    assert_eq!(legs(&sim, ship), [a]);
    assert!(sim.scripted.is_warp(ship));

    // Under way: each queued warp move is added behind what is there.
    run(&mut sim, vec![warp(1, b, true)], HZ);
    assert_eq!(legs(&sim, ship), [a, b], "the first leg is kept");
    run(&mut sim, vec![warp(2, c, true)], HZ);
    assert_eq!(legs(&sim, ship), [a, b, c]);

    // The craft flies them in order, warping on the way, and ends at the last.
    let impulse = sim.unit(ship).unwrap().stats.max_speed;
    let (mut reached, mut warped) = (Vec::new(), false);
    for _ in 0..180 * HZ {
        let before = legs(&sim, ship);
        sim.step(&[]);
        let now = legs(&sim, ship);
        if now.len() < before.len() {
            reached.push(before[0]);
        }
        let unit = sim.unit(ship).unwrap();
        warped |= unit.speed > impulse;
        if now.is_empty() && unit.speed == Fixed::ZERO {
            break;
        }
    }
    assert_eq!(reached, [a, b, c], "the legs in the order they were given");
    assert!(warped, "the course was flown at warp");
    let end = sim.unit(ship).unwrap().pos;
    assert!((end.x - c.x).abs() < n(150) && (end.z - c.z).abs() < n(150), "ended at {end:?}");
    assert!(!sim.scripted.is_warp(ship), "the warp move is over with its last leg");

    // Not queued: the course is replaced, as before.
    run(&mut sim, vec![warp(3, a, false)], HZ);
    run(&mut sim, vec![warp(4, b, true)], 1);
    assert_eq!(legs(&sim, ship), [a, b]);
    sim.step(&[warp(5, c, false)]);
    assert_eq!(legs(&sim, ship), [c], "an unqueued warp move drops the queued legs");
}

/// A queued warp move cancels nothing: a plain move under way keeps its leg and gets the new
/// one behind it, and the craft's whole course is a warp course from then on.
#[test]
fn a_queued_warp_move_behind_a_plain_move_keeps_that_move() {
    let (mut sim, db) = game(Profile::Sta2Classic);
    let ship = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    let (a, b) = (v(0, 5000), v(3000, 5000));
    run(&mut sim, vec![order(1, 0, Order::Move { units: vec![ship], to: a, queue: false, mode: None })], HZ);
    assert!(!sim.scripted.is_warp(ship));
    sim.step(&[order(1, 1, Order::MoveWarp { units: vec![ship], to: b, queue: true })]);
    assert_eq!(legs(&sim, ship), [a, b]);
    assert!(sim.scripted.is_warp(ship));
}

/// A game with queued warp moves replays to the same state.
#[test]
fn a_replay_with_queued_warp_moves_is_stable() {
    use crate::replay::{Recorder, Replay};
    use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};

    let db = Arc::new(approach_tests::db(Profile::Sta2Classic, Some(1100)));
    let seat = |id| PlayerSetup { id: PlayerId(id), team: sta_core::TeamId(id), race: "st".into(), control: Control::Idle };
    let setup = GameSetup { seed: 5, map: Arc::new(sta_content::MapDef::empty()), content: Arc::clone(&db), players: vec![seat(1), seat(2)], options: GameOptions::default() };
    let mut recorder = Recorder::with_interval(&setup, 1);
    recorder.step(&[edit(0, Edit::Spawn { odf: db.key("st_seer").unwrap(), player: PlayerId(1), pos: v(0, 0), heading: AHEAD })]);
    let ship = recorder.sim().units.iter().next().unwrap().id;
    let warp = |seq, to, queue| order(1, seq, Order::MoveWarp { units: vec![ship], to, queue });
    let (a, b, c) = (v(0, 5000), v(3000, 5000), v(3000, 0));
    recorder.step(&[warp(0, a, false)]);
    for (i, to) in [b, c].into_iter().enumerate() {
        for _ in 0..HZ {
            recorder.step(&[]);
        }
        recorder.step(&[warp(1 + i as u32, to, true)]);
    }
    assert_eq!(legs(recorder.sim(), ship), [a, b, c]);
    for _ in 0..20 * HZ {
        recorder.step(&[]);
    }
    let (digest, left) = (recorder.sim().digest(), legs(recorder.sim(), ship));
    assert!(!left.is_empty() && left.len() < 3, "recorded mid-course: {left:?}");
    let played = Replay::decode(&recorder.finish().encode()).unwrap().play(db).unwrap();
    assert_eq!(played.digest(), digest);
    assert_eq!(legs(&played, ship), left);
}
