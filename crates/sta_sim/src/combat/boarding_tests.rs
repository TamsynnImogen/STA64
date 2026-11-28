//! SIM-46 P1: the transporter order (`combat/boarding.rs`). The scenarios use
//! authored values on typed definitions (crews, radii, shields, race factors);
//! the rule numbers come from the content's keys (`TransportRules::of`) and the
//! defender factor, and expectations are written from them. No game file is read.

use super::boarding::{CARRIER_HOLD, DEFENCE_FACTOR, Party, TARGET_HOLD, TransportRules, Transports};
use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::world::NEUTRAL;
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, MapDef, Profile, RaceBoarding, RaceRepair, Rules, TransporterRules, UnitDef};
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
    db_with(crew_red, on, &|_, _| {}, |r| r)
}

/// `tweak` edits each unit by key (`alpha`, `beta`, `gamma`); `more` the rules.
fn db_with(crew_red: Option<f64>, on: bool, tweak: &dyn Fn(&str, &mut UnitDef), more: impl Fn(Rules) -> Rules) -> Arc<ContentDb> {
    db_for(Profile::Sta2Classic, crew_red, on, tweak, more)
}

fn db_for(profile: Profile, crew_red: Option<f64>, on: bool, tweak: &dyn Fn(&str, &mut UnitDef), more: impl Fn(Rules) -> Rules) -> Arc<ContentDb> {
    let mut features = profile.features();
    features.crew_loss_on_hit = on;
    let mut b = ContentDb::builder().profile(profile).features(features);
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
        tweak(key, &mut u);
        b = b.unit(u);
    }
    let mut rules = more(Rules::default());
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
fn the_red_status_row_is_found_by_the_race_name_when_the_file_stem_differs() {
    // The rows are keyed by the race file's stem, a unit's race key is the race's name.
    let repair = RaceRepair { crew_red_status: Some(fx(0.4)), ..RaceRepair::default() };
    let rows = move |r: Rules| r.with_race_repairs(vec![("racea".into(), repair)]).with_race_repair_stems(vec![("alpha".into(), "RaceA.odf".into())]);
    let mut w = world_with(db_with(None, true, &|_, _| {}, rows), PlayerId(2), "beta", 300);
    w.set_crew(w.carrier, 134.0, 135.0);
    w.set_crew(w.target, 0.0, 100_000.0);
    w.order();
    w.run(40 * 30);
    assert_eq!(w.crew(w.carrier), 59.0, "stops at the 0.4 level, not the 0.25 default");
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
/// crew, the first party arrives, the unit is captured (owner, race, order Stop) and
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

const CAPTURE_RUN_DIGEST: u64 = 0xc83f_ff24_434e_83b6;

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

// SIM-69: the numbers come from the content's keys. The base database has no
// header, so the scenarios above run on the engine defaults.

/// The carrier at 0 and a team mate with room at `gap`: crew paid and gained after the order tick.
fn friend(db: Arc<ContentDb>, gap: i32) -> World {
    let mut w = world_with(db, PlayerId(2), "beta", gap);
    w.set_crew(w.target, 40.0, CREW);
    w
}

#[test]
fn the_header_sets_range_party_and_delays() {
    let hdr = TransporterRules { range: fx(120.0), delay: fx(2.0), action_delay: fx(0.5), max: 7 };
    let d = db_with(None, true, &|_, _| {}, |r| r.with_transporter(hdr));
    let r = rules(&d);
    assert_eq!((r.range, r.party, r.delay, r.action_delay), (fx(120.0), Fixed::from_int(7), 60, 15));
    // Reach is the range plus both radii.
    let mut inside = friend(Arc::clone(&d), 120 + 2 * RADIUS);
    assert!(inside.order());
    assert_eq!((inside.crew(inside.carrier), inside.crew(inside.target)), (CREW - 7.0, 47.0));
    inside.run(59);
    assert_eq!(inside.crew(inside.carrier), CREW - 7.0, "one pulse per two seconds");
    inside.run(1);
    assert_eq!(inside.crew(inside.carrier), CREW - 14.0);
    let mut outside = friend(Arc::clone(&d), 120 + 2 * RADIUS + 1);
    assert!(!outside.order());
    // A header that leaves the keys out gives the engine defaults.
    let e = rules(&db(None, true));
    assert_eq!((e.range, e.party, e.delay, e.action_delay), (Fixed::from_int(300), Fixed::from_int(5), 30, 30));
}

#[test]
fn the_transporter_key_decides_who_may_transport_and_absent_means_crewed() {
    let off = |k: &str, u: &mut UnitDef| {
        if k == "alpha" {
            u.transport.transporter = Some(false);
        }
    };
    let mut w = friend(db_with(None, true, &off, |r| r), 100);
    assert!(!w.order(), "transporter = 0");
    let on = |k: &str, u: &mut UnitDef| {
        if k == "alpha" {
            u.transport.transporter = Some(true);
        }
    };
    assert!(friend(db_with(None, true, &on, |r| r), 100).order());
    assert!(friend(db(None, true), 100).order(), "no key: a crewed ship transports");
}

#[test]
fn party_and_race_strengths_come_from_the_keys() {
    let party_key = |k: &str, u: &mut UnitDef| {
        if k == "alpha" {
            u.transport.boarding_party_strength = fx(3.0);
        }
    };
    let race = |r: Rules| {
        r.with_race_boarding(vec![("alpha".into(), RaceBoarding { boarding_strength: fx(1.5), ..RaceBoarding::default() }), ("beta".into(), RaceBoarding { boarding_strength: fx(0.5), ..RaceBoarding::default() })])
    };
    let d = db_with(None, true, &party_key, race);
    let mut w = world_with(Arc::clone(&d), PlayerId(1), "beta", 300);
    w.lower_shields(w.target);
    w.order();
    w.run(30); // the first party has landed
    // size 5 x (3.0 x 1.5) / (1.0 x 0.5 x 1.5) = 30 kills
    assert_eq!(w.crew(w.target), CREW - 30.0);
}

#[test]
fn can_commandeer_and_can_be_commandeered_gate_the_order() {
    let none = |r: Rules| r.with_race_boarding(vec![("alpha".into(), RaceBoarding { can_commandeer: false, ..RaceBoarding::default() })]);
    // A race that cannot commandeer may still help a team mate but not take an enemy.
    assert!(friend(db_with(None, true, &|_, _| {}, none), 100).order());
    let mut enemy = world_with(db_with(None, true, &|_, _| {}, none), PlayerId(1), "beta", 100);
    enemy.lower_shields(enemy.target);
    assert!(!enemy.order());
    let safe = |r: Rules| r.with_race_boarding(vec![("beta".into(), RaceBoarding { can_be_commandeered: false, ..RaceBoarding::default() })]);
    assert!(!friend(db_with(None, true, &|_, _| {}, safe), 100).order());
}

#[test]
fn disable_shields_and_shield_delay_come_from_the_keys() {
    let keys = |k: &str, u: &mut UnitDef| match k {
        "alpha" => u.transport.disable_shields_on_transport = false,
        "beta" => u.transport.shield_delay = Some(fx(2.0)),
        _ => {}
    };
    let hold = |d: Arc<ContentDb>| {
        let mut w = world_with(d, PlayerId(1), "beta", 300);
        w.lower_shields(w.target);
        w.order();
        let until = *w.sim.transports.holds.get(&(w.target, TARGET_HOLD)).expect("the target is held");
        (w.shields(w.carrier), w.sim.transports().shields_held(w.carrier), until)
    };
    let (shields, held, until) = hold(db_with(None, true, &keys, |r| r));
    let (_, default_held, default_until) = hold(db(None, true));
    assert_eq!(shields, 90.0, "disableShieldsOnTransport = 0 keeps the carrier's shields");
    assert!(!held && default_held);
    assert_eq!(default_until - until, 30, "the target's shieldDelay of 2.0 s instead of the 3.0 s default");
}

#[test]
fn the_public_answer_follows_the_family_the_key_and_the_crew_floors() {
    use super::boarding::{can_transport, transport_allowed};
    let off = |k: &str, u: &mut UnitDef| {
        if k == "beta" {
            u.transport.transporter = Some(false);
        }
    };
    let d = db_with(None, true, &off, |r| r);
    let (alpha, beta) = (d.unit(d.key("alpha").unwrap()).unwrap(), d.unit(d.key("beta").unwrap()).unwrap());
    let crew = |now: f64| [fx(now), fx(CREW)];
    assert!(transport_allowed(&d, alpha, fx(CREW)) && !transport_allowed(&d, beta, fx(CREW)), "transporter = 0");
    assert!(can_transport(&d, alpha, crew(10.0)));
    assert!(!can_transport(&d, alpha, crew(9.0)), "the floor is 9");
    assert!(!can_transport(&d, beta, crew(100.0)));
    // A family without the derelict latch is refused, whatever the crew.
    let native = db(None, false);
    assert!(!transport_allowed(&native, native.unit(native.key("alpha").unwrap()).unwrap(), fx(CREW)));
}

// SIM-71: the Armada I profile (read from the executable, static; not measured). Its units hold the
// single health pool as hull and have no shield maximum; header and race numbers are authored.

const A1_HULL: i32 = 100_000;

/// An Armada I database: no casualty model, one pool, the race strengths and header given.
fn a1_db(strengths: &[(&str, f64)], range: f64, tweak: &dyn Fn(&str, &mut UnitDef)) -> Arc<ContentDb> {
    let rows: Vec<(String, RaceBoarding)> = strengths.iter().map(|(r, v)| ((*r).to_owned(), RaceBoarding { boarding_strength: fx(*v), ..RaceBoarding::default() })).collect();
    let hdr = TransporterRules { range: fx(range), delay: Fixed::ONE, action_delay: Fixed::ONE, max: 5 };
    db_for(
        Profile::Sta1Classic,
        None,
        false,
        &|k, u| {
            u.shields.max = Fixed::ZERO;
            u.shields.recharge = Fixed::ZERO;
            tweak(k, u);
        },
        move |r| r.with_transporter(hdr).with_race_boarding(rows.clone()),
    )
}

fn set_pool(w: &mut World, now: i32) {
    let id = w.target;
    w.sim.units.get_mut(id).unwrap().hull = [Fixed::from_int(now), Fixed::from_int(A1_HULL)];
}

#[test]
fn armada_one_kills_use_only_the_two_races_strengths() {
    // A 2.0, D 0.5: kills = 5 x 2.0 / 0.5 = 20. The unit multiplier and the 1.5 factor of Armada II do not apply.
    let party_key = |k: &str, u: &mut UnitDef| {
        if k == "alpha" || k == "beta" {
            u.transport.boarding_party_strength = fx(7.0);
        }
    };
    let mut w = world_with(a1_db(&[("alpha", 2.0), ("beta", 0.5)], 400.0, &party_key), PlayerId(1), "beta", 300);
    set_pool(&mut w, A1_HULL / 100);
    assert!(w.order());
    w.run(30); // the first party has landed
    assert_eq!(w.crew(w.target), CREW - 20.0);
}

#[test]
fn armada_one_range_is_centre_to_centre_from_the_cfg_key() {
    let d = a1_db(&[], 120.0, &|_, _| {});
    let mut inside = world_with(Arc::clone(&d), PlayerId(2), "beta", 120);
    inside.set_crew(inside.target, 40.0, CREW);
    assert!(inside.order());
    assert_eq!(inside.crew(inside.target), 45.0);
    // One unit further is out of range although the two radii would reach (Armada II adds them).
    let mut outside = world_with(d, PlayerId(2), "beta", 121);
    outside.set_crew(outside.target, 40.0, CREW);
    assert!(!outside.order());
}

#[test]
fn armada_one_boards_an_enemy_whose_pool_is_at_or_below_one_percent_and_a_derelict() {
    let d = a1_db(&[], 400.0, &|_, _| {});
    let boarded = |pool: i32| {
        let mut w = world_with(Arc::clone(&d), PlayerId(1), "beta", 300);
        set_pool(&mut w, pool);
        w.order();
        w.run(30);
        w.crew(w.target) < CREW
    };
    assert!(boarded(A1_HULL / 100), "exactly 1 %");
    assert!(!boarded(A1_HULL / 100 + 1000), "above 1 %");
    // A latched derelict passes whatever the sim holds as its pool.
    let mut w = world_with(d, PlayerId(1), "beta", 300);
    w.make_derelict(w.target);
    w.order();
    w.run(30);
    assert_eq!(w.owner(w.target), PlayerId(0), "an enemy derelict is captured");
}

#[test]
fn armada_one_has_no_unit_key_no_commandeer_keys_and_keeps_the_carriers_shields() {
    let keys = |k: &str, u: &mut UnitDef| {
        u.transport.transporter = Some(false);
        if k == "alpha" {
            u.shields.max = fx(90.0); // this carrier has a shield pool of its own
        }
    };
    let rows = vec![("alpha".to_owned(), RaceBoarding { can_commandeer: false, ..RaceBoarding::default() }), ("beta".to_owned(), RaceBoarding { can_be_commandeered: false, ..RaceBoarding::default() })];
    let hdr = TransporterRules { range: fx(400.0), delay: Fixed::ONE, action_delay: Fixed::ONE, max: 5 };
    let d = db_for(Profile::Sta1Classic, None, false, &|k, u| { u.shields.max = Fixed::ZERO; keys(k, u) }, move |r| r.with_transporter(hdr).with_race_boarding(rows.clone()));
    let mut w = world_with(d, PlayerId(1), "beta", 300);
    set_pool(&mut w, A1_HULL / 100);
    assert!(w.order(), "transporter = 0, canCommandeer = 0 and canBeCommandeered = 0 are not Armada I keys");
    w.sim.units.get_mut(w.carrier).unwrap().shields.now = [fx(90.0), Fixed::ZERO, Fixed::ZERO, Fixed::ZERO];
    w.run(30);
    assert_eq!(w.shields(w.carrier), 90.0, "the carrier keeps its shields");
    assert!(!w.sim.transports().shields_held(w.carrier));
    // No planets, and the capture command stays refused.
    assert!(!super::boarding::can_transport(&w.db, w.db.unit(w.db.key("alpha").unwrap()).unwrap(), [fx(8.0), fx(CREW)]), "the crew floor holds");
}

#[test]
fn armada_one_clears_the_latch_of_a_recrewed_derelict_that_keeps_its_owner() {
    let mut w = world_with(a1_db(&[], 400.0, &|_, _| {}), PlayerId(2), "beta", 100);
    w.make_derelict(w.target);
    assert!(w.order());
    assert!(!w.derelict(w.target), "crew of 1 or more clears the latch (no casualty model runs)");
    assert_eq!((w.owner(w.target), w.crew(w.target)), (PlayerId(2), 5.0));
}

#[test]
fn an_allied_targets_shields_drop_for_the_transfer_and_return_in_full_after_the_hold() {
    let mut w = world_with(db(None, true), PlayerId(2), "beta", 100);
    w.set_crew(w.target, 40.0, CREW);
    w.sim.units.get_mut(w.target).unwrap().shields.now = [fx(90.0), Fixed::ZERO, Fixed::ZERO, Fixed::ZERO];
    assert!(w.order());
    assert_eq!(w.shields(w.target), 0.0, "off at the first pulse");
    assert!(w.sim.transports().shields_held(w.target));
    w.sim.transports.end(w.carrier);
    w.run(rules(&w.db).carrier_shield_return - 1);
    assert_eq!(w.shields(w.target), 0.0, "still off during the hold");
    w.run(2);
    assert_eq!(w.shields(w.target), 90.0, "back in full one step after the hold");
    assert!(!w.sim.transports().shields_held(w.target));
    // An enemy is not given this hold kind (its recharge delay is set instead), and the carrier's own key rules.
    let keep = |k: &str, u: &mut UnitDef| {
        if k == "alpha" {
            u.transport.disable_shields_on_transport = false;
        }
    };
    let mut x = world_with(db_with(None, true, &keep, |r| r), PlayerId(2), "beta", 100);
    x.set_crew(x.target, 40.0, CREW);
    assert!(x.order());
    assert_eq!(x.shields(x.target), 90.0, "disableShieldsOnTransport = 0 on the carrier keeps the ally's too");
}

/// The full digest after an authored Armada I run: a carrier boards a nearly down enemy of low crew,
/// the second party arrives and captures it. New pin (SIM-71).
#[test]
fn an_armada_one_boarding_run_is_pinned() {
    let mut w = world_with(a1_db(&[("alpha", 1.25), ("beta", 0.75)], 400.0, &|_, _| {}), PlayerId(1), "beta", 300);
    set_pool(&mut w, 900);
    w.sim.units.get_mut(w.target).unwrap().target = Some(at(900));
    w.set_crew(w.target, 2.5, CREW);
    assert!(w.order());
    w.run(45);
    assert_eq!(w.owner(w.target), PlayerId(0));
    assert!(!w.sim.transports().is_empty());
    assert_eq!(w.sim.digest(), A1_RUN_DIGEST, "{:#x}", w.sim.digest());
}

const A1_RUN_DIGEST: u64 = 0x85ed_7753_b044_1716;

// SIM-46 transport attack (Roots, `transport_attack_by_weld`).

/// Fo4 content; the carrier `alpha` has `weldingAnimSpeed` as given.
fn roots_db(weld: Option<f64>) -> Arc<ContentDb> {
    db_for(Profile::Fo4, None, true, &|k, u| if k == "alpha" { u.transport.welding_anim_speed = weld.map(fx) }, |r| r)
}

impl World {
    fn attack(&mut self, player: u8) -> bool {
        let c = Command { player: PlayerId(player), seq: 0, kind: CommandKind::Order(Order::Transport { units: vec![self.carrier], target: self.target, attack: true }) };
        self.sim.step(&[c]);
        self.sim.transports().target_of(self.carrier).is_some()
    }
}

#[test]
fn a_zero_weld_carrier_on_roots_transport_attacks_a_shielded_enemy() {
    let mut w = world_with(roots_db(Some(0.0)), PlayerId(1), "beta", 300);
    assert!(w.shields(w.target) > 0.0);
    w.set_crew(w.target, 2.5, CREW);
    assert!(w.attack(0));
    w.run(45);
    assert_eq!(w.owner(w.target), PlayerId(0), "captured through its shields");
    // The plain order on the same content still stops at the shields.
    let mut plain = world_with(roots_db(Some(0.0)), PlayerId(1), "beta", 300);
    plain.set_crew(plain.target, 2.5, CREW);
    plain.order();
    plain.run(45);
    assert_eq!(plain.owner(plain.target), PlayerId(1));
}

#[test]
fn transport_attack_is_refused_without_the_weld_key_the_switch_or_from_a_non_player() {
    for weld in [None, Some(1.0)] {
        let mut w = world_with(roots_db(weld), PlayerId(1), "beta", 300);
        assert!(!w.attack(0), "weldingAnimSpeed {weld:?}");
    }
    let classic = db_with(None, true, &|k, u| if k == "alpha" { u.transport.welding_anim_speed = Some(Fixed::ZERO) }, |r| r);
    assert!(!world_with(classic, PlayerId(1), "beta", 300).attack(0), "switch off in Sta2Classic");
    let mut w = world_with(roots_db(Some(0.0)), PlayerId(1), "beta", 300);
    let c = Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Order(Order::Transport { units: vec![w.carrier], target: w.target, attack: true }) };
    w.sim.step(&[c]);
    assert!(w.sim.transports().target_of(w.carrier).is_none(), "a script is not a player");
}

#[test]
fn an_ai_seat_may_not_transport_attack() {
    let db = roots_db(Some(0.0));
    let seat = |id: u8, control: Control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 7,
        map: Arc::new(MapDef::empty()),
        content: Arc::clone(&db),
        players: vec![seat(0, Control::Ai("test".into())), seat(1, Control::Idle)],
        options: GameOptions::default(),
    });
    let carrier = sim.spawn_def(db.key("alpha").unwrap(), PlayerId(0), at(0), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let target = sim.spawn_def(db.key("beta").unwrap(), PlayerId(1), at(300), (Fixed::ZERO, Fixed::ONE)).unwrap();
    // Shields down, so the plain order keeps going.
    sim.units.get_mut(target).unwrap().shields.now = [Fixed::ZERO; 4];
    let c = |attack| Command { player: PlayerId(0), seq: 0, kind: CommandKind::Order(Order::Transport { units: vec![carrier], target, attack }) };
    sim.step(&[c(true)]);
    assert!(sim.transports().target_of(carrier).is_none());
    sim.step(&[c(false)]);
    assert!(sim.transports().target_of(carrier).is_some(), "the plain order is the AI's to give");
}

#[test]
fn a_transport_attack_round_trips_through_a_snapshot() {
    let mut w = world_with(roots_db(Some(0.0)), PlayerId(1), "beta", 300);
    assert!(w.attack(0));
    w.run(3);
    let copy = Sim::from_snapshot(&w.sim.snapshot(), Some(Arc::clone(&w.db))).unwrap();
    assert_eq!(copy.digest(), w.sim.digest());
    let (target, mut a, mut b) = (w.target, w.sim, copy);
    for _ in 0..60 {
        a.step(&[]);
        b.step(&[]);
        assert_eq!(a.digest(), b.digest());
    }
    assert_eq!(a.units.get(target).unwrap().owner, b.units.get(target).unwrap().owner);
}

#[test]
fn a_hold_kind_above_carrier_and_an_attack_without_a_row_are_refused_on_load() {
    let rec = Transports::sample_for_test().to_rec();
    let mut bad = rec.clone();
    bad.holds[0].1 = 2;
    assert!(Transports::from_rec(bad, |id| id.0 <= 2).is_none());
    let mut stray = rec;
    stray.attacks = vec![EntityId(99)];
    assert!(Transports::from_rec(stray, |id| id.0 <= 2).is_none());
}

/// SIM-46 (#774 review): a captured yard loses its queue and nobody is refunded; its
/// cargo-free stock is untouched on both sides (measured rule, see `capture`).
#[test]
fn a_captured_yard_loses_its_queue_and_no_one_is_refunded() {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.crew_loss_on_hit = true;
    let mut b = ContentDb::builder().profile(fake.profile()).features(features);
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        let mut u = u.clone();
        // Shot down and staying down, as `beta` in the scenes above.
        if u.odf == "fx_yard" {
            u.shields.recharge = Fixed::ZERO;
        }
        b = b.unit(u);
    }
    let db = Arc::new(b.build().unwrap());
    let seat = |id: u8| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 7, map: Arc::new(MapDef::empty()), content: Arc::clone(&db), players: vec![seat(0), seat(1)], options: GameOptions::default() });
    let carrier = sim.spawn_def(db.key("fx_cruiser").unwrap(), PlayerId(0), at(0), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let yard = sim.spawn_def(db.key("fx_yard").unwrap(), PlayerId(1), at(300), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let build = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Build { builder: yard, what: db.key("fx_frigate").unwrap(), at: None }) };
    sim.step(&[build]);
    assert!(!sim.production.queue(yard).is_empty(), "the yard is building");
    let (paid1, paid0) = (sim.economy.stock_of(PlayerId(1)), sim.economy.stock_of(PlayerId(0)));
    let y = sim.units.get_mut(yard).unwrap();
    y.shields.now = [Fixed::ZERO; 4];
    y.crew = Some([fx(2.5), fx(500.0)]);
    let order = Command { player: PlayerId(0), seq: 1, kind: CommandKind::Order(Order::Transport { units: vec![carrier], target: yard, attack: false }) };
    sim.step(&[order]);
    for _ in 0..45 {
        sim.step(&[]);
    }
    assert_eq!(sim.units.get(yard).unwrap().owner, PlayerId(0), "captured");
    assert!(sim.production.queue(yard).is_empty(), "the queue is lost");
    assert_eq!(sim.economy.stock_of(PlayerId(1)), paid1, "the old owner gets nothing back");
    assert_eq!(sim.economy.stock_of(PlayerId(0)), paid0, "the captor gets nothing either");
}
