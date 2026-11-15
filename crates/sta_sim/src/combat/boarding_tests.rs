//! SIM-46 P1: the transporter order (`combat/boarding.rs`). The scenarios use
//! authored values on typed definitions (crews, radii, shields, race factors);
//! only the stub rule constants (`TransportRules::of`, the defender factor) are
//! the rules under test, and expectations are written from them. No game file is read.

use super::boarding::{CARRIER_HOLD, DEFENCE_FACTOR, Party, TARGET_HOLD, TransportRules, Transports};
use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::world::NEUTRAL;
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, MapDef, Profile, RaceRepair, Rules, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

/// Authored crew of every test ship and its radius.
const CREW: f64 = 143.0;
const RADIUS: i32 = 17;

fn fx(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

fn num(v: Fixed) -> f64 {
    v.to_bits() as f64 / 4294967296.0
}

fn rules(db: &ContentDb) -> TransportRules {
    TransportRules::of(db)
}

fn party(db: &ContentDb) -> f64 {
    num(rules(db).party)
}

fn db(crew_red: Option<f64>, on: bool) -> Arc<ContentDb> {
    let mut features = Profile::Sta2Classic.features();
    features.crew_loss_on_hit = on;
    let mut b = ContentDb::builder().profile(Profile::Sta2Classic).features(features);
    for (key, race) in [("alpha", "alpha"), ("beta", "beta"), ("gamma", "beta")] {
        let mut u = UnitDef::new(key, "craft");
        u.race = race.into();
        u.hull = Fixed::from_int(100_000);
        u.shields.max = Fixed::from_int(90);
        // Only the carrier and `gamma` regenerate; a shot-down `beta` stays down until it is boarded.
        u.shields.recharge = if key == "beta" { Fixed::ZERO } else { Fixed::from_int(2) };
        u.crew = fx(CREW);
        u.maximum_crew = Some(fx(CREW));
        u.radius = Fixed::from_int(RADIUS);
        b = b.unit(u);
    }
    let mut rules = Rules::default();
    if let Some(red) = crew_red {
        let repair = RaceRepair { crew_red_status: Some(fx(red)), ..RaceRepair::default() };
        rules = rules.with_race_repairs(vec![("alpha".into(), repair)]);
    }
    Arc::new(b.build().unwrap().with_rules(rules))
}

struct World {
    sim: Sim,
    db: Arc<ContentDb>,
    carrier: EntityId,
    target: EntityId,
}

fn at(x: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::ZERO }
}

/// A carrier (player 0, race alpha) and a target of `kind` owned by `owner`, `gap` units apart (centres).
fn world_with(db: Arc<ContentDb>, owner: PlayerId, kind: &str, gap: i32) -> World {
    // Seats 0 and 1 are enemies (own team each); seat 2 is on team 0.
    let seat = |id: u8, team: u8| PlayerSetup { id: PlayerId(id), team: TeamId(team), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 7, map: Arc::new(MapDef::empty()), content: Arc::clone(&db), players: vec![seat(0, 0), seat(1, 1), seat(2, 0)], options: GameOptions::default() });
    let carrier = sim.spawn_def(db.key("alpha").unwrap(), PlayerId(0), at(0), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let target = sim.spawn_def(db.key(kind).unwrap(), owner, at(gap), (Fixed::ZERO, Fixed::ONE)).unwrap();
    World { sim, db, carrier, target }
}

fn world(owner: u8, gap: i32) -> World {
    world_with(db(None, true), PlayerId(owner), "beta", gap)
}

impl World {
    fn order(&mut self) -> bool {
        let c = Command { player: PlayerId(0), seq: 0, kind: CommandKind::Order(Order::Transport { units: vec![self.carrier], target: self.target, attack: false }) };
        self.sim.step(&[c]);
        self.sim.transports().target_of(self.carrier).is_some()
    }
    fn crew(&self, id: EntityId) -> f64 {
        num(self.sim.units.get(id).unwrap().crew.unwrap()[0])
    }
    fn set_crew(&mut self, id: EntityId, now: f64, max: f64) {
        self.sim.units.get_mut(id).unwrap().crew = Some([fx(now), fx(max)]);
    }
    fn shields(&self, id: EntityId) -> f64 {
        num(self.sim.units.get(id).unwrap().shields.now[0])
    }
    fn lower_shields(&mut self, id: EntityId) {
        self.sim.units.get_mut(id).unwrap().shields.now = [Fixed::ZERO; 4];
    }
    fn owner(&self, id: EntityId) -> PlayerId {
        self.sim.units.get(id).unwrap().owner
    }
    fn derelict(&self, id: EntityId) -> bool {
        self.sim.is_derelict(id).unwrap()
    }
    fn make_derelict(&mut self, id: EntityId) {
        self.set_crew(id, 0.0, CREW);
        self.lower_shields(id);
        self.sim.subsystems.edit(id, |r| r.derelict = true);
    }
    fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.sim.step(&[]);
        }
    }
    fn resolve(&mut self, party: Party) {
        let db = Arc::clone(&self.db);
        self.sim.resolve_party(&db, party);
    }
    fn party(&self, size: f64, strength: f64) -> Party {
        Party { arrive: 0, seq: 0, target: self.target, size: fx(size), strength: fx(strength), race: "alpha".into(), team: PlayerId(0), source: self.carrier }
    }
}

/// Kills of one equal-strength party against the defender factor.
fn kills_of(db: &ContentDb) -> Fixed {
    rules(db).party * Fixed::ONE / DEFENCE_FACTOR
}

#[test]
fn the_first_pulse_is_on_the_order_tick_then_one_per_delay_and_each_costs_the_party() {
    let mut w = world(2, 300); // a team mate with room for crew
    let (p, delay) = (party(&w.db), rules(&w.db).delay);
    w.set_crew(w.target, 40.0, CREW);
    assert!(w.order());
    assert_eq!(w.crew(w.carrier), CREW - p, "the pulse of the order tick");
    assert_eq!(w.crew(w.target), 40.0 + p, "a friend gets the party at once");
    w.run(delay - 1);
    assert_eq!((w.crew(w.carrier), w.crew(w.target)), (CREW - p, 40.0 + p), "nothing between pulses");
    w.run(1);
    assert_eq!((w.crew(w.carrier), w.crew(w.target)), (CREW - 2.0 * p, 40.0 + 2.0 * p));
}

#[test]
fn friendly_overflow_is_lost_and_the_transport_ends_at_a_full_target() {
    let mut w = world(2, 300);
    let (p, delay) = (party(&w.db), rules(&w.db).delay);
    w.set_crew(w.target, CREW - 2.0, CREW);
    assert!(w.order());
    assert_eq!(w.crew(w.target), CREW, "capped at the maximum");
    assert_eq!(w.crew(w.carrier), CREW - p, "the carrier still paid the whole party");
    w.run(delay);
    assert_eq!(w.crew(w.carrier), CREW - p, "full: no second pulse");
    assert!(w.sim.transports().target_of(w.carrier).is_none());
}

#[test]
fn range_is_surface_to_surface_and_out_of_range_does_nothing() {
    let reach = num(rules(&w_db()).range) as i32 + 2 * RADIUS;
    let p = party(&w_db());
    let mut inside = world(2, reach);
    inside.set_crew(inside.target, 40.0, CREW);
    assert!(inside.order());
    assert_eq!(inside.crew(inside.carrier), CREW - p);

    let mut outside = world(2, reach + 1);
    outside.set_crew(outside.target, 40.0, CREW);
    let before = outside.sim.unit(outside.carrier).unwrap().pos;
    assert!(!outside.order(), "no transport starts");
    outside.run(120);
    assert_eq!(outside.crew(outside.carrier), CREW);
    assert_eq!(outside.crew(outside.target), 40.0);
    assert_eq!(outside.sim.unit(outside.carrier).unwrap().pos, before, "no approach");
    assert!(outside.sim.transports().is_empty());
}

fn w_db() -> Arc<ContentDb> {
    db(None, true)
}

#[test]
fn an_enemy_with_shields_up_is_not_boarded() {
    let mut w = world(1, 300);
    assert!(w.shields(w.target) > 0.0);
    w.order();
    w.run(120);
    assert_eq!((w.crew(w.carrier), w.crew(w.target)), (CREW, CREW));
    assert_eq!(w.owner(w.target), PlayerId(1));
}

#[test]
fn each_enemy_party_arrives_an_action_delay_after_its_pulse_and_kills_size_over_the_defender_factor() {
    let mut w = world(1, 300);
    let (p, action) = (party(&w.db), rules(&w.db).action_delay);
    w.lower_shields(w.target);
    assert!(w.order());
    assert_eq!(w.crew(w.carrier), CREW - p);
    assert_eq!(w.crew(w.target), CREW, "the party is still in flight");
    w.run(action - 1);
    assert_eq!(w.crew(w.target), CREW);
    w.run(1);
    let want = fx(CREW) - kills_of(&w.db);
    assert_eq!(w.sim.units.get(w.target).unwrap().crew.unwrap()[0], want);
    assert_eq!(w.owner(w.target), PlayerId(1), "one party does not take a ship of that crew");
}

#[test]
fn the_target_cannot_recharge_during_the_transport_and_the_carrier_shields_return_in_full_after_the_last_pulse() {
    let mut w = world_with(db(None, true), PlayerId(1), "gamma", 300);
    let r = rules(&w.db);
    w.lower_shields(w.target);
    // The target was shot down just before: its recharge is still held for the order tick.
    w.sim.transports.holds.insert((w.target, TARGET_HOLD), 1);
    assert!(w.shields(w.carrier) > 0.0);
    assert!(w.order());
    assert_eq!(w.shields(w.carrier), 0.0, "down at the first pulse");
    let last = 2 * r.delay; // pulses at 0, delay, 2 x delay
    w.run(last);
    assert_eq!((w.shields(w.carrier), w.shields(w.target)), (0.0, 0.0), "both stay down during the transport");
    let c = Command { player: PlayerId(0), seq: 1, kind: CommandKind::Order(Order::Stop { units: vec![w.carrier] }) };
    w.sim.step(&[c]);
    assert!(w.sim.transports().target_of(w.carrier).is_none());
    // The target is held until last + shield_hold, then regrows by its rate; the carrier is
    // held until last + carrier_shield_return, then is back at its maximum in one step.
    let now = last + 1;
    w.run(last + r.target_shield_hold - now - 1);
    assert_eq!(w.shields(w.target), 0.0);
    w.run(1);
    assert!(w.shields(w.target) > 0.0 && w.shields(w.target) < 90.0, "regrowing, not full");
    w.run(last + r.carrier_shield_return - (last + r.target_shield_hold) - 1);
    assert_eq!(w.shields(w.carrier), 0.0);
    w.run(1);
    assert_eq!(w.shields(w.carrier), 90.0, "back in full in one step");
}

#[test]
fn one_unit_that_is_a_carrier_and_an_enemys_target_keeps_both_holds() {
    let mut w = world(1, 300);
    // Both holds on one unit, different ends: neither overwrites the other.
    w.sim.transports.holds.insert((w.carrier, TARGET_HOLD), 40);
    w.sim.transports.holds.insert((w.carrier, CARRIER_HOLD), 70);
    w.lower_shields(w.carrier);
    w.run(41);
    assert_eq!(w.shields(w.carrier), 0.0, "the target hold ended; the carrier hold still holds");
    assert!(w.sim.transports().shields_held(w.carrier));
    w.run(30);
    assert_eq!(w.shields(w.carrier), 90.0, "the carrier hold's end restores in full");
    assert!(!w.sim.transports().shields_held(w.carrier));
}

#[test]
fn the_carrier_stops_at_the_races_red_status_from_the_content() {
    // Another pulse must leave more than red x max crew. Default 0.25 of 143 is 35.75: the
    // crew goes 43 (goes on), 38 (another pulse would leave 33: stop).
    let mut w = world(2, 300);
    w.set_crew(w.target, 0.0, 100_000.0);
    w.order();
    w.run(40 * 30);
    assert_eq!(w.crew(w.carrier), 38.0);
    assert!(w.sim.transports().target_of(w.carrier).is_none());
    // Race `crewRedStatus` 0.4 from the content, max 135: level 54. From 134 the crew lands on
    // 59, where another pulse leaves exactly 54: equality stops, despite the 32.32 rounding of 0.4.
    let mut w = world_with(db(Some(0.4), true), PlayerId(2), "beta", 300);
    w.set_crew(w.carrier, 134.0, 135.0);
    w.set_crew(w.target, 0.0, 100_000.0);
    w.order();
    w.run(40 * 30);
    assert_eq!(w.crew(w.carrier), 59.0);
}

#[test]
fn a_tie_leaves_a_derelict_that_keeps_its_owner_and_stops_its_order_and_the_next_party_captures_it() {
    let mut w = world(1, 300);
    let action = rules(&w.db).action_delay;
    w.lower_shields(w.target);
    w.sim.units.get_mut(w.target).unwrap().target = Some(at(900)); // it was under way
    w.sim.units.get_mut(w.target).unwrap().crew = Some([kills_of(&w.db), fx(CREW)]);
    assert!(w.order());
    w.run(action); // the first party arrives
    assert_eq!(w.crew(w.target), 0.0);
    assert!(w.derelict(w.target));
    assert_eq!(w.owner(w.target), PlayerId(1), "kills == crew is not a capture");
    assert!(w.sim.units.get(w.target).unwrap().target.is_none(), "its order is Stop");
    w.run(rules(&w.db).delay); // the second party arrives at the derelict
    assert_eq!(w.owner(w.target), PlayerId(0));
}

#[test]
fn a_party_that_empties_the_crew_captures_and_owner_race_and_order_change_on_one_tick() {
    let mut w = world(1, 300);
    let action = rules(&w.db).action_delay;
    w.lower_shields(w.target);
    w.sim.units.get_mut(w.target).unwrap().target = Some(at(900)); // it was under way
    w.set_crew(w.target, 2.5, CREW);
    assert!(w.order());
    w.run(action - 1);
    assert_eq!((w.owner(w.target), w.sim.unit_race(w.target)), (PlayerId(1), Some("beta")));
    w.run(1);
    // survivors = (kills - crew) x D / A
    assert_eq!(w.owner(w.target), PlayerId(0));
    assert_eq!(w.sim.unit_race(w.target), Some("alpha"));
    assert!(w.sim.units.get(w.target).unwrap().target.is_none(), "Stop");
    assert!(!w.derelict(w.target));
    let want = (kills_of(&w.db) - fx(2.5)) * DEFENCE_FACTOR / Fixed::ONE;
    assert_eq!(w.sim.units.get(w.target).unwrap().crew.unwrap()[0], want);
    // The carrier keeps transporting to the now-friendly ship.
    w.run(1);
    assert_eq!(w.sim.transports().target_of(w.carrier), Some(w.target));
}

#[test]
fn survivors_are_the_surplus_times_defence_over_attack() {
    let near = |v: Fixed, want: f64| assert!((num(v) - want).abs() < 0.0001, "{} vs {want}", num(v));
    // size 4, attack 2.5, defence 1.0: kills 10; crew 6 leaves an excess of 4, survivors 4 x 0.4.
    let (kills, survivors) = super::boarding::outcome(fx(4.0), fx(2.5), fx(1.0), fx(6.0));
    near(kills, 10.0);
    near(survivors, 1.6);
    // size 3, attack 0.8, defence 1.6: kills 1.5; crew 1 leaves 0.5, survivors 0.5 x 2.
    let (kills, survivors) = super::boarding::outcome(fx(3.0), fx(0.8), fx(1.6), fx(1.0));
    near(kills, 1.5);
    near(survivors, 1.0);
    // size 7, attack 1.2, defence 0.9: kills 9.3333; crew 4 leaves 5.3333, survivors 5.3333 x 0.75.
    let (kills, survivors) = super::boarding::outcome(fx(7.0), fx(1.2), fx(0.9), fx(4.0));
    near(kills, 9.3333);
    near(survivors, 4.0);
    // Capture needs kills strictly over the crew.
    assert_eq!(super::boarding::outcome(fx(4.0), fx(2.5), fx(1.0), fx(10.0)).1, Fixed::ZERO);
}

#[test]
fn survivors_under_one_leave_the_captured_unit_derelict() {
    let mut w = world(1, 300);
    w.lower_shields(w.target);
    w.set_crew(w.target, 3.0, CREW);
    let p = w.party(5.0, 1.0);
    w.resolve(p);
    // kills 3.333 over crew 3: survivors 0.333 x 1.5 = 0.5.
    assert_eq!(w.owner(w.target), PlayerId(0));
    assert!((w.crew(w.target) - 0.5).abs() < 0.001);
    w.run(1);
    assert!(w.derelict(w.target));
}

#[test]
fn strong_boarders_leave_survivors_as_crew() {
    let mut w = world(1, 300);
    w.set_crew(w.target, 9.0, CREW);
    let p = w.party(4.0, 3.0);
    w.resolve(p);
    // kills 4 x 3 / 1.5 = 8 is not over 9: nothing is captured.
    assert_eq!(w.owner(w.target), PlayerId(1));
    assert_eq!(w.crew(w.target), 1.0);
    let mut w = world(1, 300);
    w.set_crew(w.target, 6.0, CREW);
    let p = w.party(4.0, 3.0);
    w.resolve(p);
    // kills 8 over 6: survivors 2 x 1.5 / 3 = 1.
    assert_eq!(w.owner(w.target), PlayerId(0));
    assert!((w.crew(w.target) - 1.0).abs() < 0.001);
    w.run(1);
    assert!(!w.derelict(w.target), "one crew clears the latch");
}

#[test]
fn a_derelict_with_shields_up_is_refused_like_any_shielded_enemy() {
    let mut w = world(1, 300);
    w.make_derelict(w.target);
    w.sim.units.get_mut(w.target).unwrap().shields.now = [Fixed::from_int(50); 4];
    w.order();
    w.run(60);
    assert_eq!((w.crew(w.carrier), w.owner(w.target)), (CREW, PlayerId(1)));
}

#[test]
fn an_enemy_derelict_is_captured_by_one_party() {
    let mut w = world(1, 300);
    let (p, action) = (party(&w.db), rules(&w.db).action_delay);
    w.make_derelict(w.target);
    assert!(w.order());
    w.run(action - 1);
    assert_eq!(w.owner(w.target), PlayerId(1));
    w.run(1);
    assert_eq!(w.owner(w.target), PlayerId(0));
    assert_eq!(w.sim.unit_race(w.target), Some("alpha"));
    assert_eq!(w.crew(w.target), p);
    assert!(!w.derelict(w.target), "the latch clears on the same tick");
}

#[test]
fn an_own_derelict_is_recrewed_and_takes_the_carriers_race_but_a_neutral_one_keeps_its_owner() {
    let mut own = world(2, 300);
    let p = party(&own.db);
    own.make_derelict(own.target);
    assert!(own.order());
    assert_eq!((own.crew(own.target), own.owner(own.target)), (p, PlayerId(2)));
    assert_eq!(own.sim.unit_race(own.target), Some("alpha"));
    assert!(!own.derelict(own.target));

    let mut neutral = world(NEUTRAL.0, 300);
    neutral.make_derelict(neutral.target);
    assert!(neutral.order());
    assert_eq!((neutral.crew(neutral.target), neutral.owner(neutral.target)), (p, NEUTRAL));
    assert_eq!(neutral.sim.unit_race(neutral.target), Some("beta"));
    assert!(!neutral.derelict(neutral.target));
}

#[test]
fn a_party_whose_carrier_is_gone_is_lost_and_the_order_needs_the_casualty_model() {
    let mut w = world(1, 300);
    w.lower_shields(w.target);
    assert!(w.order());
    w.sim.despawn(w.carrier, None);
    w.run(40);
    assert_eq!(w.crew(w.target), CREW);
    assert!(w.sim.transports().target_of(w.carrier).is_none());

    // Without the derelict latch (crew_loss_on_hit off) the order is refused.
    let mut off = world_with(db(None, false), PlayerId(2), "beta", 300);
    assert!(!off.order());
}

#[test]
fn another_order_ends_the_transport_and_the_attack_mode_is_refused() {
    let mut w = world(2, 300);
    w.set_crew(w.target, 0.0, 100_000.0);
    assert!(w.order());
    let stop = Command { player: PlayerId(0), seq: 1, kind: CommandKind::Order(Order::Stop { units: vec![w.carrier] }) };
    w.sim.step(&[stop]);
    assert!(w.sim.transports().target_of(w.carrier).is_none());
    let attack = Command { player: PlayerId(0), seq: 2, kind: CommandKind::Order(Order::Transport { units: vec![w.carrier], target: w.target, attack: true }) };
    w.sim.step(&[attack]);
    assert!(w.sim.transports().target_of(w.carrier).is_none());
}

#[test]
fn transport_state_round_trips_through_a_snapshot_and_keeps_stepping_alike() {
    let mut w = world(1, 300);
    w.lower_shields(w.target);
    assert!(w.order());
    w.run(10);
    let copy = Sim::from_snapshot(&w.sim.snapshot(), Some(Arc::clone(&w.db))).unwrap();
    assert_eq!(copy.digest(), w.sim.digest());
    let (target, mut a, mut b) = (w.target, w.sim, copy);
    for _ in 0..80 {
        a.step(&[]);
        b.step(&[]);
        assert_eq!(a.digest(), b.digest());
    }
    assert_eq!(a.units.get(target).unwrap().crew, b.units.get(target).unwrap().crew);
}

/// The full digest after an authored scripted run: a carrier boards a shot-down enemy of low
/// crew, the second party arrives, the unit is captured (owner, race, order Stop) and
/// the carrier goes on. The layout of every transport field is in it.
#[test]
fn a_scripted_capture_run_is_pinned() {
    let mut w = world(1, 300);
    w.lower_shields(w.target);
    w.sim.units.get_mut(w.target).unwrap().target = Some(at(900));
    w.set_crew(w.target, 2.5, CREW);
    assert!(w.order());
    w.run(45);
    assert_eq!(w.owner(w.target), PlayerId(0));
    assert!(w.sim.units.get(w.target).unwrap().target.is_none());
    assert!(!w.sim.transports().is_empty());
    assert_eq!(w.sim.digest(), CAPTURE_RUN_DIGEST, "{:#x}", w.sim.digest());
}

const CAPTURE_RUN_DIGEST: u64 = 0x162a_82c0_cff7_7054;

#[test]
fn the_sample_state_is_well_formed() {
    let s = Transports::sample_for_test();
    let rec = s.to_rec();
    let back = Transports::from_rec(rec.clone(), |id| id.0 <= 2).unwrap();
    assert_eq!(back.to_rec().parties, rec.parties);
    assert_eq!(back.to_rec().holds, rec.holds);
    assert!(Transports::from_rec(rec.clone(), |_| false).is_none());
    let mut bad = rec;
    bad.holds.swap(0, 1);
    assert!(Transports::from_rec(bad, |id| id.0 <= 2).is_none(), "holds must ascend by (entity, kind)");
}
