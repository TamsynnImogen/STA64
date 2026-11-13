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
const HZ: u32 = TICK_HZ as u32;

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
            Order::Move { units: vec![s], to: v(0, 6000), queue: false }
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
