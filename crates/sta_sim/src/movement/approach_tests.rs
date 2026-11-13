//! SIM-36 attack approach, per profile, on a stand-in for the measured
//! Federation destroyer (impulse 140, combat 70, warp 400, range 400).

use std::sync::Arc;

use sta_content::{ApproachRules, ContentDb, MoveStats, Profile, Rules, UnitDef, WeaponDef, WeaponKind};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: Fixed::ZERO, z: n(z) }
}

/// A destroyer like `fdestroy`, its pulse weapon, and an unarmed target.
pub(crate) fn db(profile: Profile, warp_distance: Option<i32>) -> ContentDb {
    let mut gun = WeaponDef::new("st_pulse", "phaser", WeaponKind::Beam);
    gun.range = n(400);
    gun.damage = n(1);
    gun.refire = 60;
    gun.duration = 5;
    let mut ship = UnitDef::new("st_destroy", "craft");
    ship.race = "st".into();
    ship.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 1.0);
    ship.movement.combat_speed = n(70);
    ship.movement.warp_speed = n(400);
    ship.radius = n(12);
    ship.sensor = n(1200);
    ship.hull = n(100_000);
    ship.hardpoints = vec![sta_content::Hardpoint { weapon: sta_content::DefRef::new("st_pulse"), node: "hp01".into(), arc: None }];
    // The same ship seeing the whole stage, as the measured scenes did (`Fog=0`).
    let mut seer = ship.clone();
    seer.odf = "st_seer".into();
    seer.sensor = n(6000);
    let mut target = ship.clone();
    target.odf = "st_target".into();
    target.hardpoints.clear();
    let mut rules = Rules::default();
    if let Some(w) = warp_distance {
        rules = rules.with_approach(ApproachRules { min_move_warp_distance: n(w) });
    }
    ContentDb::builder()
        .profile(profile)
        .features(profile.features())
        .weapon(gun)
        .unit(ship)
        .unit(seer)
        .unit(target)
        .build()
        .expect("consistent")
        .with_rules(rules)
}

pub(crate) fn sim(db: ContentDb) -> Sim {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "st".into(), control: Control::Idle };
    Sim::from_setup(&GameSetup {
        seed: 5,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(db),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    })
}

const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

/// Shooter at the origin facing the target `dist` ahead; attack ordered on
/// tick 0. `(tick, speed, centre distance)` for `ticks` ticks.
pub(crate) fn run(profile: Profile, warp: Option<i32>, dist: i32, ticks: u32) -> (Sim, Vec<(u32, Fixed, Fixed)>) {
    let mut sim = sim(db(profile, warp));
    let db = Arc::clone(sim.content.as_ref().unwrap());
    let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    let t = sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(0, dist), AHEAD).unwrap();
    let order = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![s], target: t }) };
    let mut out = Vec::new();
    for i in 0..ticks {
        let cmds = if i == 0 { vec![order.clone()] } else { Vec::new() };
        WorldSource::step(&mut sim, &cmds);
        let (a, b) = (sim.unit(s).unwrap(), sim.unit(t).unwrap());
        out.push((i, a.speed, b.pos.z - a.pos.z));
    }
    (sim, out)
}

fn top(trace: &[(u32, Fixed, Fixed)]) -> Fixed {
    trace.iter().map(|r| r.1).max().unwrap()
}

/// Highest speed while the target is nearer than `d` (centre distance).
fn top_within(trace: &[(u32, Fixed, Fixed)], d: i32, after_warp: bool) -> Fixed {
    let start = if after_warp { trace.iter().position(|r| r.1 > n(150)).unwrap_or(0) } else { 0 };
    trace[start..].iter().filter(|r| r.2 < n(d) && r.1 <= n(150)).map(|r| r.1).max().unwrap_or(Fixed::ZERO)
}

fn rest(trace: &[(u32, Fixed, Fixed)]) -> Fixed {
    let last = trace.last().unwrap();
    assert_eq!(last.1, Fixed::ZERO, "still moving");
    last.2
}

#[test]
fn stock_far_attack_warps_then_closes_at_combat_speed_and_stops_short_of_range() {
    let (_, tr) = run(Profile::Sta2Classic, Some(1100), 3000, 30 * 40);
    assert!(top(&tr) > n(390), "no warp: {:?}", top(&tr));
    // Out of warp before the target is 1100 + radii away; combat speed after.
    let first_slow = tr.iter().position(|r| r.1 > n(150)).map(|w| tr[w..].iter().find(|r| r.1 <= n(71)).unwrap()).unwrap();
    assert!(first_slow.2 > n(1000) && first_slow.2 < n(1250), "dropped out at {:?}", first_slow.2);
    assert!(top_within(&tr, 1000, true) <= n(71));
    // Stops where `D > W - buffer` turns false (350 + radii), braking from 70.
    let at = rest(&tr);
    assert!(at > n(340) && at < n(380), "stopped at {:?}", at);
}

#[test]
fn stock_near_attack_runs_at_combat_speed_without_warp() {
    let (_, tr) = run(Profile::Sta2Classic, Some(1100), 1000, 30 * 25);
    assert!(top(&tr) > n(69) && top(&tr) <= n(71), "{:?}", top(&tr));
    let at = rest(&tr);
    assert!(at > n(340) && at < n(380), "stopped at {:?}", at);
}

#[test]
fn roots_classic_warps_to_near_range_and_closes_at_impulse_inside_the_warp_distance() {
    // Roots running STA2 Classic: the Fo4 profile with Classic's 1100.
    let (_, far) = run(Profile::Fo4, Some(1100), 3000, 30 * 30);
    assert!(top(&far) > n(390));
    // Still in warp well inside the warp distance (measured: out at about 450 to 500).
    assert!(far.iter().any(|r| r.1 > n(300) && r.2 < n(900)));
    let (_, near) = run(Profile::Fo4, Some(1100), 1000, 30 * 25);
    assert!(top(&near) > n(130) && top(&near) <= n(140), "impulse: {:?}", top(&near));
    // No buffer: stops once in range (measured 312 centre to centre).
    let at = rest(&near);
    assert!(at > n(280) && at < n(424), "stopped at {:?}", at);
}

#[test]
fn fleet_ops_warp_distance_never_warps() {
    let (_, tr) = run(Profile::Fo4, Some(999_999), 3000, 30 * 40);
    assert!(top(&tr) <= n(140));
    assert!(rest(&tr) < n(424));
}

#[test]
fn an_attacker_in_range_does_not_move_and_its_target_moving_away_restarts_the_approach() {
    let (mut sim, tr) = run(Profile::Sta2Classic, Some(1100), 300, 30 * 3);
    assert_eq!(top(&tr), Fixed::ZERO);
    let t = EntityId(2);
    sim.units.get_mut(t).unwrap().pos = v(0, 900);
    WorldSource::step(&mut sim, &[]);
    WorldSource::step(&mut sim, &[]);
    assert!(sim.unit(EntityId(1)).unwrap().speed > Fixed::ZERO);
}

#[test]
fn an_unseen_or_cloaked_target_is_flown_to_without_warp_and_the_order_stands() {
    let mut sim = sim(db(Profile::Sta2Classic, Some(1100)));
    let db = Arc::clone(sim.content.as_ref().unwrap());
    let s = sim.spawn_def(db.key("st_destroy").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
    let t = sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(0, 3000), AHEAD).unwrap();
    let order = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![s], target: t }) };
    WorldSource::step(&mut sim, &[order]);
    let mut top = Fixed::ZERO;
    for _ in 0..30 * 10 {
        WorldSource::step(&mut sim, &[]);
        top = top.max(sim.unit(s).unwrap().speed);
    }
    assert!(top <= n(140), "no warp toward an unseen target: {:?}", top);
    assert_eq!(sim.weapons.attack_of(s), Some(t));
    // Close by but cloaked: no homing, the craft holds its last goal.
    let at = sim.unit(s).unwrap().pos;
    sim.units.get_mut(t).unwrap().pos = Vec3 { x: at.x + n(300), y: at.y, z: at.z + n(300) };
    let u = sim.units.get_mut(t).unwrap();
    u.flags = u.flags.with(crate::view::UnitFlags::CLOAKED, true);
    let goal = sim.unit(s).unwrap().target;
    WorldSource::step(&mut sim, &[]);
    assert_eq!(sim.weapons.attack_of(s), Some(t));
    assert_eq!(sim.unit(s).unwrap().target, goal);
}

#[test]
fn a_dead_target_leaves_no_course_and_queued_legs_wait_for_the_attack() {
    let (mut sim, _) = run(Profile::Sta2Classic, Some(1100), 1000, 30 * 5);
    let (s, t) = (EntityId(1), EntityId(2));
    let mv = Command { player: PlayerId(1), seq: 1, kind: CommandKind::Order(Order::Move { units: vec![s], to: v(-500, 0), queue: true }) };
    for _ in 0..30 * 10 {
        WorldSource::step(&mut sim, &[]);
    }
    WorldSource::step(&mut sim, &[mv]);
    for _ in 0..30 * 2 {
        WorldSource::step(&mut sim, &[]);
    }
    assert_eq!(sim.weapons.attack_of(s), Some(t));
    assert!(sim.movement.course(s).is_some_and(|c| c.waypoints == [v(-500, 0)]), "the leg waits");
    let kill = Command { player: PlayerId::BENCH, seq: 0, kind: CommandKind::Edit(crate::command::Edit::Kill(t)) };
    WorldSource::step(&mut sim, &[kill]);
    WorldSource::step(&mut sim, &[]);
    assert_eq!(sim.weapons.attack_of(s), None);
    assert_eq!(sim.unit(s).unwrap().target, Some(v(-500, 0)), "on to the queued leg, not the old goal");
}

#[test]
fn a_move_order_replaces_the_attack() {
    let (mut sim, _) = run(Profile::Sta2Classic, Some(1100), 3000, 30);
    let mv = Command { player: PlayerId(1), seq: 1, kind: CommandKind::Order(Order::Move { units: vec![EntityId(1)], to: v(-500, 0), queue: false }) };
    WorldSource::step(&mut sim, &[mv]);
    assert_eq!(sim.weapons.attack_of(EntityId(1)), None);
    assert_eq!(sim.unit(EntityId(1)).unwrap().target, Some(v(-500, 0)));
}

#[test]
fn warp_state_is_in_the_digest_and_survives_a_snapshot() {
    let (sim, tr) = run(Profile::Sta2Classic, Some(1100), 3000, 30 * 4);
    assert!(tr.last().unwrap().1 > n(390), "in warp at the end");
    assert!(sim.approach_warp.contains(EntityId(1)));
    let mut copy = sim.clone();
    copy.approach_warp.remove(EntityId(1));
    assert_ne!(copy.digest(), sim.digest());
    assert_eq!(copy.section_digests().differing(&sim.section_digests()), ["movement"]);
    let back = Sim::from_snapshot(&sim.snapshot(), sim.content.clone()).unwrap();
    assert_eq!(back.digest(), sim.digest());
    assert!(back.approach_warp.contains(EntityId(1)));
}
