//! LUA-32: alert levels, the last enemy hit and the ally attack counters (rules in `alert`).

use std::sync::Arc;

use sta_content::Profile;
use sta_core::{ContentKey, TeamId};

use crate::command::{AlertState, Command, CommandKind, Edit, Order};
use crate::event::SimEvent;
use crate::movement::approach_tests;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::warp_override_tests::script;
use crate::{EntityId, Fixed, PlayerId, Sim, Vec3, WorldSource};

const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);
const HZ: u64 = sta_core::TICK_HZ as u64;

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// Seats `(id, team)` over the approach tests' content (`st_seer`: armed, sees the stage;
/// `st_target`: unarmed), stock rules.
pub(crate) fn game(seats: &[(u8, u8)]) -> Sim {
    let seat = |&(id, team): &(u8, u8)| PlayerSetup { id: PlayerId(id), team: TeamId(team), race: "st".into(), control: Control::Idle };
    Sim::from_setup(&GameSetup {
        seed: 5,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(approach_tests::db(Profile::Sta2Classic, None)),
        players: seats.iter().map(seat).collect(),
        options: GameOptions::default(),
    })
}

pub(crate) fn put(sim: &mut Sim, odf: &str, owner: u8, at: Vec3) -> EntityId {
    let key = sim.content().unwrap().key(odf).unwrap();
    sim.spawn_def(key, PlayerId(owner), at, AHEAD).unwrap()
}

fn fired_by(sim: &mut Sim, who: EntityId, ticks: u64, first: Vec<Command>) -> usize {
    let mut n = 0;
    for i in 0..ticks {
        let cmds = if i == 0 { first.clone() } else { Vec::new() };
        n += WorldSource::step(sim, &cmds).events.iter().filter(|e| matches!(e, SimEvent::Fired { shooter, .. } if *shooter == who)).count();
    }
    n
}

fn attack(player: u8, units: Vec<EntityId>, target: EntityId) -> Command {
    Command { player: PlayerId(player), seq: 0, kind: CommandKind::Order(Order::Attack { units, target }) }
}

fn hit(sim: &mut Sim, target: EntityId, by: EntityId) {
    let from = sim.events.len();
    sim.events.push(SimEvent::Hit { target, by, weapon: ContentKey::NONE, facing: 0, shield: Fixed::ZERO, hull: Fixed::ONE, at: Vec3::ZERO });
    sim.record_enemy_hits(from);
    sim.events.truncate(from);
}

/// A shooter at the origin and an unarmed enemy 300 ahead, in beam range.
fn duel() -> (Sim, EntityId, EntityId) {
    let mut sim = game(&[(1, 1), (2, 2)]);
    let s = put(&mut sim, "st_seer", 1, v(0, 0));
    let t = put(&mut sim, "st_target", 2, v(0, 300));
    (sim, s, t)
}

#[test]
fn red_fires_at_targets_of_opportunity_green_never_yellow_only_after_a_hit() {
    let (mut sim, s, _) = duel();
    assert!(fired_by(&mut sim, s, 180, vec![]) > 0, "red");
    let (mut sim, s, _) = duel();
    assert!(script(&mut sim, Edit::Alert(s, AlertState::Green)));
    assert_eq!(fired_by(&mut sim, s, 180, vec![]), 0, "green");
    let (mut sim, s, _) = duel();
    assert!(script(&mut sim, Edit::Alert(s, AlertState::Yellow)));
    assert_eq!(fired_by(&mut sim, s, 180, vec![]), 0, "yellow without a hit");
}

#[test]
fn the_ordered_target_is_fired_at_under_every_level() {
    for level in [AlertState::Green, AlertState::Yellow, AlertState::Red] {
        let (mut sim, s, t) = duel();
        assert!(script(&mut sim, Edit::Alert(s, level)));
        assert!(fired_by(&mut sim, s, 180, vec![attack(1, vec![s], t)]) > 0, "{level:?}");
    }
}

#[test]
fn a_green_unit_does_not_fire_at_an_opportunity_even_with_an_order_on_something_out_of_reach() {
    // The order's target is far away: the opportunity target in range stays unshot.
    let (mut sim, s, _) = duel();
    let far = put(&mut sim, "st_target", 2, v(0, 3000));
    assert!(script(&mut sim, Edit::Alert(s, AlertState::Green)));
    assert_eq!(fired_by(&mut sim, s, 120, vec![attack(1, vec![s], far)]), 0);
}

#[test]
fn yellow_fires_for_fifteen_seconds_after_an_enemy_hit_strictly() {
    let (mut sim, s, t) = duel();
    sim.tick = 10_000;
    assert!(script(&mut sim, Edit::Alert(s, AlertState::Yellow)));
    let now = sim.tick;
    sim.alerts.stamp(s, now - (15 * HZ - 1));
    assert!(sim.opportunity_fire_allowed(s), "14.97 s after the hit");
    sim.alerts.stamp(s, now - 15 * HZ);
    assert!(!sim.opportunity_fire_allowed(s), "exactly 15 s");
    // And through the weapons: a fresh hit lets it shoot, an old one does not.
    let now = sim.tick;
    sim.alerts.stamp(s, now);
    assert!(fired_by(&mut sim, s, 60, vec![]) > 0);
    let (mut sim, s, _) = duel();
    sim.tick = 10_000;
    assert!(script(&mut sim, Edit::Alert(s, AlertState::Yellow)));
    let now = sim.tick;
    sim.alerts.stamp(s, now - 20 * HZ);
    assert_eq!(fired_by(&mut sim, s, 60, vec![]), 0);
    let _ = t;
}

#[test]
fn the_gate_levels_follow_the_alert_edits_and_red_removes_the_row() {
    let mut sim = game(&[(1, 1), (2, 2)]);
    let s = put(&mut sim, "st_seer", 1, v(0, 0));
    assert_eq!(sim.alert_of(s), AlertState::Red);
    assert!(sim.opportunity_fire_allowed(s));
    assert!(script(&mut sim, Edit::Alert(s, AlertState::Green)));
    assert!(!sim.opportunity_fire_allowed(s));
    assert!(script(&mut sim, Edit::Alert(s, AlertState::Red)));
    assert!(sim.opportunity_fire_allowed(s));
    assert!(sim.alerts.is_default(), "red removes the row");
}

#[test]
fn alert_edits_apply_to_live_craft_and_stations_only() {
    let mut sim = game(&[(1, 1), (2, 2)]);
    let s = put(&mut sim, "st_seer", 1, v(0, 0));
    assert!(!script(&mut sim, Edit::Alert(EntityId(999), AlertState::Green)), "unknown");
    sim.units.get_mut(s).unwrap().flags = crate::view::UnitFlags::DYING;
    assert!(!script(&mut sim, Edit::Alert(s, AlertState::Green)), "dying");
    // A player may not send it.
    let c = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Edit(Edit::Alert(s, AlertState::Green)) };
    assert!(WorldSource::step(&mut sim, &[c]).events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    let (mut sim, s, _) = duel();
    assert!(script(&mut sim, Edit::Alert(s, AlertState::Yellow)));
    assert_eq!(sim.alert_of(s), AlertState::Yellow);
    assert_eq!(sim.view().attrs.alerts, vec![(s, AlertState::Yellow)]);
}

#[test]
fn only_enemy_hits_stamp_and_a_team_change_or_a_cloak_forgets_it() {
    // Seats 1 and 3 are one team, 2 and 4 are others; 5 is another team's seat made neutral by relation.
    let mut sim = game(&[(1, 1), (2, 2), (3, 1)]);
    let a = put(&mut sim, "st_seer", 1, v(0, 0));
    let friend = put(&mut sim, "st_seer", 3, v(0, 50));
    let foe = put(&mut sim, "st_seer", 2, v(0, 100));
    hit(&mut sim, a, friend);
    hit(&mut sim, a, a);
    assert_eq!(sim.alerts.since_hit(a, 0), None, "ally and own hits do not count");
    hit(&mut sim, a, foe);
    assert!(sim.alerts.since_hit(a, 0).is_some(), "an enemy hit does");
    // A change of owner forgets it.
    assert!(script(&mut sim, Edit::Team(a, PlayerId(2))));
    assert_eq!(sim.alerts.since_hit(a, 0), None);
    // So does cloaking.
    hit(&mut sim, friend, foe);
    assert!(sim.alerts.since_hit(friend, 0).is_some());
    let from = sim.events.len();
    sim.events.push(SimEvent::CloakChanged { id: friend, on: true });
    sim.forget_hits_of_cloakers(from);
    assert_eq!(sim.alerts.since_hit(friend, 0), None);
}

#[test]
fn a_neutral_relation_hit_does_not_stamp() {
    let mut sim = game(&[(1, 1), (2, 2)]);
    let a = put(&mut sim, "st_seer", 1, v(0, 0));
    let b = put(&mut sim, "st_seer", 2, v(0, 50));
    assert!(script(&mut sim, Edit::SetRelation { from: PlayerId(1), to: PlayerId(2), relation: crate::view::Relation::Neutral }));
    hit(&mut sim, a, b);
    assert_eq!(sim.alerts.since_hit(a, 0), None);
}

#[test]
fn a_hit_in_a_fight_stamps_the_victim_and_removal_drops_the_row() {
    let mut sim = game(&[(1, 1), (2, 2)]);
    let a = put(&mut sim, "st_seer", 1, v(0, 0));
    let b = put(&mut sim, "st_seer", 2, v(0, 200));
    for _ in 0..200 {
        sim.step(&[]);
    }
    assert!(sim.alerts.since_hit(a, sim.tick).is_some() && sim.alerts.since_hit(b, sim.tick).is_some());
    assert!(sim.despawn(a, None));
    assert_eq!(sim.alerts.since_hit(a, sim.tick), None);
    assert_eq!(sim.view().attrs.enemy_hit.iter().map(|r| r.0).collect::<Vec<_>>(), vec![b]);
}

/// Seer A (0,0), seer H (0,100) of seat 1; an enemy seer E (0,350) that shoots the nearer H.
fn assist_scene() -> (Sim, EntityId, EntityId) {
    let mut sim = game(&[(1, 1), (2, 2)]);
    put(&mut sim, "st_seer", 1, v(0, 0));
    let h = put(&mut sim, "st_seer", 1, v(0, 100));
    let e = put(&mut sim, "st_seer", 2, v(0, 350));
    (sim, h, e)
}

fn ever_ordered_on(sim: &mut Sim, h: EntityId, e: EntityId) -> bool {
    (0..240).any(|_| {
        sim.step(&[]);
        sim.weapons.attack_of(h) == Some(e)
    })
}

#[test]
fn idle_assist_green_and_special_forces_stay_red_and_yellow_chase() {
    let (mut sim, h, e) = assist_scene();
    assert!(ever_ordered_on(&mut sim, h, e), "red assists");
    let (mut sim, h, e) = assist_scene();
    assert!(script(&mut sim, Edit::Alert(h, AlertState::Yellow)));
    assert!(ever_ordered_on(&mut sim, h, e), "yellow assists as red does");
    let (mut sim, h, e) = assist_scene();
    assert!(script(&mut sim, Edit::Alert(h, AlertState::Green)));
    assert!(!ever_ordered_on(&mut sim, h, e), "green never assists");
    let (mut sim, h, e) = assist_scene();
    assert!(script(&mut sim, Edit::SpecialForces(h, true)));
    assert!(!ever_ordered_on(&mut sim, h, e), "special forces never assist");
}

#[test]
fn ally_attack_counters_count_orders_against_non_enemies_with_a_shared_throttle() {
    let mut sim = game(&[(1, 1), (2, 2), (3, 1)]);
    let a = put(&mut sim, "st_seer", 1, v(0, 0));
    let friend = put(&mut sim, "st_target", 3, v(0, 300));
    let foe = put(&mut sim, "st_target", 2, v(0, -300));
    WorldSource::step(&mut sim, &[attack(1, vec![a], foe)]);
    assert_eq!(sim.ally_attacks(PlayerId(2), PlayerId(1)), (0, 0), "an enemy target does not count");
    WorldSource::step(&mut sim, &[attack(1, vec![a], friend)]);
    assert_eq!(sim.ally_attacks(PlayerId(3), PlayerId(1)), (1, 0));
    WorldSource::step(&mut sim, &[attack(1, vec![a], friend)]);
    assert_eq!(sim.ally_attacks(PlayerId(3), PlayerId(1)), (1, 0), "within 1.0 s");
    for _ in 0..HZ + 1 {
        sim.step(&[]);
    }
    WorldSource::step(&mut sim, &[attack(1, vec![a], friend)]);
    assert_eq!(sim.ally_attacks(PlayerId(3), PlayerId(1)), (2, 0));
    // Only the two accepted orders above counted (the scene's targets are unarmed, so no assist ran).
    assert_eq!(sim.view().attrs.ally_attacks, vec![(3, 1, 2, 0)]);
}

#[test]
fn the_throttle_is_one_second_for_ships_and_a_twentieth_for_stations_from_one_shared_time() {
    let mut a = crate::alert::Alerts::default();
    assert!(a.count(100, 3, 1, false));
    assert!(!a.count(100 + HZ, 3, 1, false), "exactly 1.0 s is not more");
    assert!(a.count(101 + HZ, 3, 1, false));
    let t = 101 + HZ;
    assert!(!a.count(t + 1, 3, 1, true), "0.033 s is not more than 0.05 s");
    assert!(a.count(t + 2, 3, 1, true), "0.067 s is");
    assert!(!a.count(t + 3, 3, 1, false), "the station record moved the shared time");
    assert!(a.count(t + 2 + HZ + 1, 3, 1, false));
    assert!(a.count(t, 4, 2, false), "another attacker has its own time");
    assert_eq!(a.ally_rows(), &[(3, 1, 3, 1), (4, 2, 1, 0)]);
}

#[test]
fn a_built_unit_copies_its_producers_alert() {
    let mut sim = game(&[(1, 1), (2, 2)]);
    let yard = put(&mut sim, "st_seer", 1, v(0, 0));
    let child = put(&mut sim, "st_seer", 1, v(0, 50));
    let other = put(&mut sim, "st_seer", 1, v(0, 90));
    assert!(script(&mut sim, Edit::Alert(yard, AlertState::Green)));
    let from = sim.events.len();
    sim.events.push(SimEvent::Built { builder: yard, id: child });
    sim.inherit_builder_alert(from);
    assert_eq!(sim.alert_of(child), AlertState::Green);
    assert_eq!(sim.alert_of(other), AlertState::Red, "a script spawn starts at the default");
}

#[test]
fn decommission_is_a_stored_switch_and_a_spin_after_the_end_is_accepted() {
    let mut sim = game(&[(1, 1), (2, 2)]);
    let s = put(&mut sim, "st_seer", 1, v(0, 0));
    assert!(script(&mut sim, Edit::Decommissionable(s, false)));
    assert!(sim.switches(s).unwrap().no_decommission);
    assert!(script(&mut sim, Edit::Decommissionable(s, true)));
    assert!(!sim.switches(s).unwrap().no_decommission);
    // A dying craft takes the spin; a gone one is accepted without effect; one never allocated is not.
    sim.units.get_mut(s).unwrap().flags = crate::view::UnitFlags::DYING;
    assert!(script(&mut sim, Edit::AngularVelocity(s, Fixed::ONE)));
    sim.units.get_mut(s).unwrap().flags = crate::view::UnitFlags(0);
    assert!(sim.despawn(s, None));
    assert!(script(&mut sim, Edit::AngularVelocity(s, Fixed::ONE)), "gone");
    assert!(!script(&mut sim, Edit::AngularVelocity(EntityId(9999), Fixed::ONE)), "never allocated");
}

#[test]
fn nothing_new_is_hashed_at_the_default_and_the_enemy_hit_table_only_while_a_level_exists() {
    // Twin sims stepped alike; only the alert state differs.
    let twin = |f: &dyn Fn(&mut Sim, EntityId)| {
        let (mut sim, s, _) = duel();
        sim.step(&[]);
        f(&mut sim, s);
        sim.digest()
    };
    let plain = twin(&|_, _| {});
    assert_eq!(twin(&|sim, s| sim.alerts.stamp(s, 5)), plain, "a stamp alone hashes nothing while every unit is at red");
    assert_eq!(twin(&|sim, s| assert!(sim.edit_alert(s, AlertState::Red))), plain, "red is the default");
    // Decommissionable(true) is the default switch row: no row, no hash.
    assert_eq!(twin(&|sim, s| assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::Decommissionable(s, true)))), plain);
    let yellow = twin(&|sim, s| assert!(sim.edit_alert(s, AlertState::Yellow)));
    let yellow_hit = twin(&|sim, s| {
        assert!(sim.edit_alert(s, AlertState::Yellow));
        sim.alerts.stamp(s, 5);
    });
    assert_ne!(yellow, plain);
    assert_ne!(yellow_hit, yellow, "once a level exists the hit is part of the digest");
    let off = twin(&|sim, s| assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::Decommissionable(s, false))));
    assert_ne!(off, plain, "a set switch bit is hashed");
}

#[test]
fn a_whole_state_digest_with_non_default_alert_state_is_pinned() {
    let mut sim = game(&[(1, 1), (2, 2), (3, 1)]);
    let a = put(&mut sim, "st_seer", 1, v(0, 0));
    let b = put(&mut sim, "st_seer", 1, v(0, 60));
    let friend = put(&mut sim, "st_target", 3, v(0, 300));
    assert!(script(&mut sim, Edit::Alert(a, AlertState::Green)));
    assert!(script(&mut sim, Edit::Alert(b, AlertState::Yellow)));
    assert!(script(&mut sim, Edit::Decommissionable(b, false)));
    sim.alerts.stamp(b, 3);
    WorldSource::step(&mut sim, &[attack(1, vec![a], friend)]);
    for _ in 0..40 {
        sim.step(&[]);
    }
    assert_eq!(sim.ally_attacks(PlayerId(3), PlayerId(1)), (1, 0));
    assert_eq!(sim.digest(), PINNED, "{:#x}", sim.digest());
}

const PINNED: u64 = 0x62c8_9c40_006b_ad9f;

fn fake_game() -> Sim {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(sta_content::fake::db()),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    })
}

#[test]
fn a_starbase_follows_the_same_gate_and_takes_alert_edits() {
    let fires = |level: Option<AlertState>, stamped: bool| {
        let mut sim = fake_game();
        let key = sim.content().unwrap().key("fx_station").unwrap();
        let station = sim.spawn_def(key, PlayerId(0), v(0, 0), AHEAD).unwrap();
        put(&mut sim, "fx_miner", 1, v(0, 100));
        if let Some(l) = level {
            assert!(script(&mut sim, Edit::Alert(station, l)), "a station takes an alert edit");
        }
        if stamped {
            let now = sim.tick;
            sim.alerts.stamp(station, now);
        }
        fired_by(&mut sim, station, 240, vec![])
    };
    assert!(fires(None, false) > 0, "red");
    assert_eq!(fires(Some(AlertState::Green), false), 0, "green");
    assert_eq!(fires(Some(AlertState::Yellow), false), 0, "yellow without a hit");
    assert!(fires(Some(AlertState::Yellow), true) > 0, "yellow after a hit");
}

#[test]
fn player_alert_applies_only_to_owned_live_actors_and_uses_the_combat_gate() {
    let (mut sim, own, enemy) = duel();
    let order = Command {
        player: PlayerId(1),
        seq: 1,
        kind: CommandKind::Order(Order::Alert { units: vec![enemy, own, EntityId(999)], level: AlertState::Green }),
    };
    assert_eq!(fired_by(&mut sim, own, 180, vec![order]), 0);
    assert_eq!(sim.alert_of(enemy), AlertState::Red);
    assert_eq!(sim.alert_of(own), AlertState::Green);
}
