//! SIM-46 P1: the transporter order (`combat/boarding.rs`). The numbers are the
//! stock Armada II bench figures set as values on typed definitions; no game
//! file is read.

use super::boarding::{Party, Transports};
use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::world::NEUTRAL;
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, MapDef, Profile, RaceRepair, Rules, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

fn fx(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

fn num(v: Fixed) -> f64 {
    v.to_bits() as f64 / 4294967296.0
}

fn db(crew_red: Option<f64>, on: bool) -> Arc<ContentDb> {
    let mut features = Profile::Sta2Classic.features();
    features.crew_loss_on_hit = on;
    let mut b = ContentDb::builder().profile(Profile::Sta2Classic).features(features);
    for (key, race) in [("alpha", "alpha"), ("beta", "beta"), ("gamma", "beta")] {
        let mut u = UnitDef::new(key, "craft");
        u.race = race.into();
        u.hull = Fixed::from_int(100_000);
        u.shields.max = Fixed::from_int(100);
        // Only the carrier and `gamma` regenerate; a shot-down `beta` stays down until it is boarded.
        u.shields.recharge = if key == "beta" { Fixed::ZERO } else { Fixed::ONE };
        u.crew = fx(250.0);
        u.maximum_crew = Some(fx(250.0));
        u.radius = Fixed::from_int(20);
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
        self.set_crew(id, 0.0, 250.0);
        self.lower_shields(id);
        self.sim.subsystems.edit(id, |r| r.derelict = true);
    }
    fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.sim.step(&[]);
        }
    }
    fn _db(&self) -> &ContentDb {
        &self.db
    }
}

const KILLS: f64 = 5.0 / 1.5;

#[test]
fn the_first_pulse_is_on_the_order_tick_then_one_per_second_and_each_costs_the_party() {
    let mut w = world(2, 300); // an ally (same team) with room for crew
    w.set_crew(w.target, 100.0, 250.0);
    assert!(w.order());
    assert_eq!(w.crew(w.carrier), 245.0, "the pulse of the order tick");
    assert_eq!(w.crew(w.target), 105.0, "a friend gets the party at once");
    w.run(29);
    assert_eq!((w.crew(w.carrier), w.crew(w.target)), (245.0, 105.0), "nothing between pulses");
    w.run(1);
    assert_eq!((w.crew(w.carrier), w.crew(w.target)), (240.0, 110.0), "the next pulse, 1.0 s later");
}

#[test]
fn friendly_overflow_is_lost_and_the_transport_ends_at_a_full_target() {
    let mut w = world(2, 300);
    w.set_crew(w.target, 248.0, 250.0);
    assert!(w.order());
    assert_eq!(w.crew(w.target), 250.0, "capped at the maximum");
    assert_eq!(w.crew(w.carrier), 245.0, "the carrier still paid the whole party");
    w.run(30);
    assert_eq!(w.crew(w.carrier), 245.0, "full: no second pulse");
    assert!(w.sim.transports().target_of(w.carrier).is_none());
}

#[test]
fn range_is_surface_to_surface_and_out_of_range_does_nothing() {
    // 500 + 20 + 20 = 540 between centres is in range, one unit more is not.
    let mut inside = world(2, 540);
    inside.set_crew(inside.target, 100.0, 250.0);
    assert!(inside.order());
    assert_eq!(inside.crew(inside.carrier), 245.0);

    let mut outside = world(2, 541);
    outside.set_crew(outside.target, 100.0, 250.0);
    let before = outside.sim.unit(outside.carrier).unwrap().pos;
    assert!(!outside.order(), "no transport starts");
    outside.run(120);
    assert_eq!(outside.crew(outside.carrier), 250.0);
    assert_eq!(outside.crew(outside.target), 100.0);
    assert_eq!(outside.sim.unit(outside.carrier).unwrap().pos, before, "no approach");
    assert!(outside.sim.transports().is_empty());
}

#[test]
fn an_enemy_with_shields_up_is_not_boarded() {
    let mut w = world(1, 300);
    assert!(w.shields(w.target) > 0.0);
    w.order();
    w.run(120);
    assert_eq!((w.crew(w.carrier), w.crew(w.target)), (250.0, 250.0));
    assert_eq!(w.owner(w.target), PlayerId(1));
}

#[test]
fn each_enemy_party_arrives_a_second_after_its_pulse_and_kills_three_and_a_third() {
    let mut w = world(1, 300);
    w.lower_shields(w.target);
    assert!(w.order());
    assert_eq!(w.crew(w.carrier), 245.0);
    assert_eq!(w.crew(w.target), 250.0, "the party is still in flight");
    w.run(29);
    assert_eq!(w.crew(w.target), 250.0);
    w.run(1);
    // The pinned arithmetic: 5 x 1 / (1 x 1 x 1.5) in 32.32.
    let kills = Fixed::from_int(5) / Fixed::from_bits(6_442_450_944);
    let want = Fixed::from_int(250) - kills;
    assert_eq!(w.sim.units.get(w.target).unwrap().crew.unwrap()[0], want);
    assert!((250.0 - w.crew(w.target) - KILLS).abs() < 1e-6);
    assert_eq!(w.owner(w.target), PlayerId(1), "one party does not take a cruiser");
}

#[test]
fn the_target_cannot_recharge_while_the_transport_runs_and_the_carrier_shields_return_3_3_s_after_the_last_pulse() {
    let mut w = world_with(db(None, true), PlayerId(1), "gamma", 300);
    w.lower_shields(w.target);
    // The target was shot down just before: its recharge is still held for the order tick.
    w.sim.transports.holds.insert(w.target, 1);
    assert!(w.shields(w.carrier) > 0.0);
    assert!(w.order());
    assert_eq!(w.shields(w.carrier), 0.0, "down at the first pulse");
    w.run(60);
    assert_eq!((w.shields(w.carrier), w.shields(w.target)), (0.0, 0.0), "both stay down during the transport");
    // End the transport: the last pulse was at tick 60 (pulses at 0, 30, 60).
    let c = Command { player: PlayerId(0), seq: 1, kind: CommandKind::Order(Order::Stop { units: vec![w.carrier] }) };
    w.sim.step(&[c]); // tick 61
    assert!(w.sim.transports().target_of(w.carrier).is_none());
    // Carrier: held until 60 + 99 = 159; target until 60 + 90 = 150.
    w.run(150 - 62); // after the step of tick 149
    assert_eq!(w.shields(w.target), 0.0);
    w.run(1); // tick 150: the hold is over, the pass recharges
    assert!(w.shields(w.target) > 0.0);
    w.run(158 - 150); // after tick 158
    assert_eq!(w.shields(w.carrier), 0.0);
    w.run(1); // tick 159
    assert!(w.shields(w.carrier) > 0.0, "3.3 s after the last pulse");
}

#[test]
fn the_carrier_stops_at_the_races_red_status_from_the_content() {
    // Default 0.25 of 250: another pulse would leave 62.5 or less, so it ends at 65 (37 pulses).
    let mut w = world(2, 300);
    w.set_crew(w.target, 0.0, 100_000.0);
    w.order();
    w.run(40 * 30);
    assert_eq!(w.crew(w.carrier), 65.0);
    assert!(w.sim.transports().target_of(w.carrier).is_none());
    // crewRedStatus 0.2 of the race (the content): 50, so it ends at 55 (39 pulses).
    let mut w = world_with(db(Some(0.2), true), PlayerId(2), "beta", 300);
    w.set_crew(w.target, 0.0, 100_000.0);
    w.order();
    w.run(45 * 30);
    assert_eq!(w.crew(w.carrier), 55.0);
}

#[test]
fn a_tie_leaves_a_derelict_that_keeps_its_owner_and_the_next_party_captures_it() {
    let mut w = world(1, 300);
    w.lower_shields(w.target);
    let kills = Fixed::from_int(5) / Fixed::from_bits(6_442_450_944);
    w.sim.units.get_mut(w.target).unwrap().crew = Some([kills, Fixed::from_int(250)]);
    assert!(w.order());
    w.run(30); // the first party arrives
    assert_eq!(w.crew(w.target), 0.0);
    assert!(w.derelict(w.target));
    assert_eq!(w.owner(w.target), PlayerId(1), "kills == crew is not a capture");
    w.run(30); // the second party arrives at the derelict
    assert_eq!(w.owner(w.target), PlayerId(0));
}

#[test]
fn a_party_that_empties_the_crew_captures_with_survivors_and_owner_and_race_change_on_one_tick() {
    let mut w = world(1, 300);
    w.lower_shields(w.target);
    w.sim.units.get_mut(w.target).unwrap().target = Some(at(900)); // it was under way
    w.set_crew(w.target, 2.0, 250.0);
    assert!(w.order());
    w.run(29);
    assert_eq!((w.owner(w.target), w.sim.unit_race(w.target)), (PlayerId(1), Some("beta")));
    w.run(1);
    // kills 3.333 > crew 2: survivors (3.333 - 2) x D / A (1.5) = 2.0: captured with 2 crew.
    assert_eq!(w.owner(w.target), PlayerId(0));
    assert_eq!(w.sim.unit_race(w.target), Some("alpha"));
    assert!(w.sim.units.get(w.target).unwrap().target.is_none(), "Stop");
    assert!(!w.derelict(w.target), "2 crew is not a derelict");
    let kills = Fixed::from_int(5) / Fixed::from_bits(6_442_450_944);
    let want = (kills - Fixed::from_int(2)) * Fixed::from_bits(6_442_450_944) / Fixed::ONE;
    assert_eq!(w.sim.units.get(w.target).unwrap().crew.unwrap()[0], want);
    assert!((num(want) - 2.0).abs() < 0.001);
    // The carrier keeps transporting to the now-friendly ship: the next pulse re-crews it.
    w.run(1);
    assert_eq!(w.sim.transports().target_of(w.carrier), Some(w.target));
    w.run(30);
    assert!(w.crew(w.target) >= 5.0 && !w.derelict(w.target), "{}", w.crew(w.target));
}

#[test]
fn survivors_are_the_surplus_times_defence_over_attack_for_each_pairing() {
    let size = Fixed::from_int(5);
    let near = |v: Fixed, want: f64| assert!((num(v) - want).abs() < 0.0001, "{} vs {want}", num(v));
    // Strong attacker on a Federation-like defender: D / A = 0.3, an excess of 5 leaves 1.5.
    let (kills, survivors) = super::boarding::outcome(size, Fixed::from_int(5), fx(1.5), fx(16.6667 - 5.0));
    near(kills, 16.6667);
    near(survivors, 1.5);
    // Attacker 1.5 on a defender of 1.125: D / A = 0.75, an excess of 3.333 leaves 2.5.
    let (kills, survivors) = super::boarding::outcome(size, fx(1.5), fx(1.125), fx(6.6667 - 3.3333));
    near(kills, 6.6667);
    near(survivors, 2.5);
    // Equal races: D / A = 1.5, per party 3.333 kills; an excess of 1 leaves 1.5.
    let (kills, survivors) = super::boarding::outcome(size, Fixed::ONE, fx(1.5), fx(3.3333 - 1.0));
    near(kills, 3.3333);
    near(survivors, 1.5);
    // No capture without `kills > crew`.
    assert_eq!(super::boarding::outcome(size, Fixed::ONE, fx(1.5), fx(4.0)).1, Fixed::ZERO);
}

#[test]
fn survivors_under_one_leave_the_captured_unit_derelict() {
    let mut w = world(1, 300);
    w.lower_shields(w.target);
    w.set_crew(w.target, 3.0, 250.0);
    let party = Party { arrive: 0, seq: 0, target: w.target, size: Fixed::from_int(5), strength: Fixed::ONE, race: "alpha".into(), team: PlayerId(0), source: w.carrier };
    let db = Arc::clone(&w.db);
    w.sim.resolve_party(&db, party);
    // kills 3.333 over crew 3: survivors 0.333 x 1.5 = 0.5.
    assert_eq!(w.owner(w.target), PlayerId(0));
    assert!((w.crew(w.target) - 0.5).abs() < 0.001);
    w.run(1);
    assert!(w.derelict(w.target));
}

#[test]
fn strong_boarders_leave_survivors_as_crew() {
    let mut w = world(1, 300);
    w.set_crew(w.target, 10.0, 250.0);
    let party = Party { arrive: 0, seq: 0, target: w.target, size: Fixed::from_int(5), strength: Fixed::from_int(5), race: "alpha".into(), team: PlayerId(0), source: w.carrier };
    let db = Arc::clone(&w.db);
    w.sim.resolve_party(&db, party);
    // kills 16.67 over crew 10: survivors 6.67 x D / A (0.3) = 2.0 become the crew.
    assert_eq!(w.owner(w.target), PlayerId(0));
    assert!((w.crew(w.target) - 2.0).abs() < 0.001);
    w.run(1);
    assert!(w.derelict(w.target) == (w.crew(w.target) < 1.0));
    assert!(!w.derelict(w.target), "2 crew clears the latch");
}

#[test]
fn a_derelict_with_shields_up_is_refused_like_any_shielded_enemy() {
    let mut w = world(1, 300);
    w.make_derelict(w.target);
    w.sim.units.get_mut(w.target).unwrap().shields.now = [Fixed::from_int(50); 4];
    w.order();
    w.run(60);
    assert_eq!((w.crew(w.carrier), w.owner(w.target)), (250.0, PlayerId(1)));
}

#[test]
fn an_enemy_derelict_is_captured_by_one_party() {
    let mut w = world(1, 300);
    w.make_derelict(w.target);
    assert!(w.order());
    w.run(29);
    assert_eq!(w.owner(w.target), PlayerId(1));
    w.run(1);
    assert_eq!(w.owner(w.target), PlayerId(0));
    assert_eq!(w.sim.unit_race(w.target), Some("alpha"));
    assert_eq!(w.crew(w.target), 5.0);
    assert!(!w.derelict(w.target), "the latch clears on the same tick");
}

#[test]
fn an_own_derelict_is_recrewed_and_takes_the_carriers_race_but_a_neutral_one_keeps_its_owner() {
    let mut own = world(2, 300);
    own.make_derelict(own.target);
    assert!(own.order());
    assert_eq!((own.crew(own.target), own.owner(own.target)), (5.0, PlayerId(2)));
    assert_eq!(own.sim.unit_race(own.target), Some("alpha"));
    assert!(!own.derelict(own.target));

    let mut neutral = world(NEUTRAL.0, 300);
    neutral.make_derelict(neutral.target);
    assert!(neutral.order());
    assert_eq!((neutral.crew(neutral.target), neutral.owner(neutral.target)), (5.0, NEUTRAL));
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
    assert_eq!(w.crew(w.target), 250.0);
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
fn transport_state_round_trips_through_a_snapshot_and_the_digest() {
    let mut w = world(1, 300);
    w.lower_shields(w.target);
    assert!(w.order());
    w.run(10);
    let plain = {
        let mut p = world(1, 300);
        p.lower_shields(p.target);
        p.run(11);
        p.sim.digest()
    };
    assert_ne!(w.sim.digest(), plain, "live transport state is in the digest");
    let copy = Sim::from_snapshot(&w.sim.snapshot(), Some(Arc::clone(&w.db))).unwrap();
    assert_eq!(copy.digest(), w.sim.digest());
    let (mut a, mut b) = (w.sim, copy);
    for _ in 0..80 {
        a.step(&[]);
        b.step(&[]);
        assert_eq!(a.digest(), b.digest());
    }
    assert_eq!(a.units.get(w.target).unwrap().crew, b.units.get(w.target).unwrap().crew);
}

#[test]
fn the_sample_state_is_well_formed() {
    let s = Transports::sample_for_test();
    let rec = s.to_rec();
    let back = Transports::from_rec(rec.clone(), |id| id.0 <= 2).unwrap();
    assert_eq!(back.to_rec().parties, rec.parties);
    assert!(Transports::from_rec(rec, |_| false).is_none());
}
