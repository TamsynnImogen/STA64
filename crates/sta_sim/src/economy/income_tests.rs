//! SIM-72: orbital station income. Authored numbers only.

use super::tests::{game_with_content, spawn, v};
use super::*;
use crate::WorldSource;
use crate::world::NEUTRAL;
use sta_content::{ContentBuilder, DefRef, Hardpoint, NodeIncome, Rules, StationMining, UnitDef, WeaponDef, WeaponKind};

const PERIOD: u32 = 45;
const ALIVE: u32 = 37;

struct Case {
    node: NodeIncome,
    station: StationMining,
    race: Option<Fixed>,
}

fn case() -> Case {
    Case {
        node: NodeIncome { mining_rate: Some(Fixed::from_int(6)), amounts: [None, None, Some(Fixed::from_int(1000)), None], ..NodeIncome::default() },
        station: StationMining { multiplier: Some(Fixed::from_f64(0.5)), can_handle: Some([false, false, true, false]) },
        race: Some(Fixed::from_int(2)),
    }
}

fn db(c: &Case, node: bool, beam: bool) -> sta_content::ContentDb {
    let mut weapon = WeaponDef::new("o_beam", "minebm", WeaponKind::Beam);
    weapon.refire = u64::from(PERIOD);
    weapon.duration = u64::from(ALIVE);
    let mut station = UnitDef::new("o_station", "p_mining");
    station.orbit_time = Some(Fixed::ZERO);
    station.station_mining = Some(c.station);
    if beam {
        station.hardpoints = vec![Hardpoint { weapon: DefRef::new("o_beam"), node: "hp01".into(), arc: None }];
    }
    let mut world = UnitDef::new("o_world", "planet");
    world.radius = Fixed::from_int(200);
    world.resource = Some(2);
    world.can_have_orbital_station = true;
    world.node_income = node.then_some(c.node);
    let rules = Rules::default().with_race_mining(c.race.map(|m| vec![("fx".to_owned(), m)]).unwrap_or_default());
    ContentBuilder::default().weapon(weapon).unit(station).unit(world).build().unwrap().with_rules(rules)
}

/// A planet at the origin and a station at `at` for `owner`; ticks run; the owner's metal and the node.
fn run(db: sta_content::ContentDb, at: Vec3, owner: PlayerId, ticks: u32) -> (crate::Sim, Fixed, Option<Fixed>) {
    let mut sim = game_with_content(db);
    let world = spawn(&mut sim, "o_world", NEUTRAL, v(0, 0));
    spawn(&mut sim, "o_station", owner, at);
    for _ in 0..ticks {
        sim.step(&[]);
    }
    let metal = sim.view().players.iter().find(|p| p.id == PlayerId(0)).unwrap().resources.get(2).copied().unwrap_or(Fixed::ZERO);
    let left = sim.economy.nodes.get(world).copied();
    (sim, metal, left)
}

fn near(a: Fixed, b: f64) -> bool {
    (f64::from(a.to_f32()) - b).abs() < 0.3
}

const RING: i32 = 275;

#[test]
fn a_beam_pulse_pays_rate_times_multiplier_times_race_modifier() {
    // 6 x 0.5 x 2 = 6 a second: 0.2 a tick for 37 of every 45 ticks; ten whole periods.
    let (_, metal, left) = run(db(&case(), true, true), v(RING, 0), PlayerId(0), PERIOD * 10);
    assert!(near(metal, 74.2), "{metal:?}");
    assert!(near(left.unwrap(), 1000.0 - 74.2), "{left:?}");
}

#[test]
fn a_race_without_a_modifier_counts_one() {
    let mut c = case();
    c.race = None;
    let (_, metal, _) = run(db(&c, true, true), v(RING, 0), PlayerId(0), PERIOD * 10);
    assert!(near(metal, 37.1), "{metal:?}");
}

#[test]
fn the_node_limits_the_take_and_holds_at_the_trickle_amount() {
    let mut c = case();
    c.node.amounts = [None, None, Some(Fixed::from_int(50)), None];
    // No trickle data: the node runs dry and the stock stops at what it held.
    let (_, metal, left) = run(db(&c, true, true), v(RING, 0), PlayerId(0), PERIOD * 40);
    assert!(near(metal, 50.0), "{metal:?}");
    assert_eq!(left, Some(Fixed::ZERO));
    // A floor of 20 and a fraction of a quarter: it holds there and keeps paying a trickle.
    c.node.trickle_amount = Some(Fixed::from_int(20));
    c.node.trickle_fraction = Some(Fixed::from_f64(0.25));
    let (_, metal, left) = run(db(&c, true, true), v(RING, 0), PlayerId(0), PERIOD * 40);
    assert_eq!(left, Some(Fixed::from_int(20)));
    assert!(f64::from(metal.to_f32()) > 31.0 && f64::from(metal.to_f32()) < 120.0, "{metal:?}");
}

#[test]
fn an_infinite_node_never_falls() {
    let mut c = case();
    c.node.infinite = Some(true);
    let (_, metal, left) = run(db(&c, true, true), v(RING, 0), PlayerId(0), PERIOD * 10);
    assert!(near(metal, 74.2), "{metal:?}");
    assert!(left.is_none());
}

#[test]
fn nothing_is_paid_without_a_reason_to() {
    let base = case();
    let paid = |db: sta_content::ContentDb, at: Vec3, owner: PlayerId| run(db, at, owner, PERIOD * 10).1;
    // Off the ring, owned by nobody, no beam, no node data: nothing.
    assert_eq!(paid(db(&base, true, true), v(RING + 40, 0), PlayerId(0)), Fixed::ZERO);
    assert_eq!(paid(db(&base, true, true), v(RING, 0), NEUTRAL), Fixed::ZERO);
    assert_eq!(paid(db(&base, true, false), v(RING, 0), PlayerId(0)), Fixed::ZERO);
    assert_eq!(paid(db(&base, false, true), v(RING, 0), PlayerId(0)), Fixed::ZERO);
    // A station that does not handle the node's resource, or has no multiplier.
    let mut other = case();
    other.station.can_handle = Some([true, false, false, false]);
    assert_eq!(paid(db(&other, true, true), v(RING, 0), PlayerId(0)), Fixed::ZERO);
    let mut none = case();
    none.station.multiplier = None;
    assert_eq!(paid(db(&none, true, true), v(RING, 0), PlayerId(0)), Fixed::ZERO);
}

#[test]
fn a_station_with_its_weapons_off_earns_nothing() {
    use crate::command::{Command, CommandKind, Edit};
    let mut sim = game_with_content(db(&case(), true, true));
    spawn(&mut sim, "o_world", NEUTRAL, v(0, 0));
    let station = spawn(&mut sim, "o_station", PlayerId(0), v(RING, 0));
    sim.step(&[Command { player: PlayerId::BENCH, seq: 1, kind: CommandKind::Edit(Edit::Weapons(station, false)) }]);
    for _ in 0..PERIOD * 4 {
        sim.step(&[]);
    }
    assert_eq!(sim.view().players.iter().find(|p| p.id == PlayerId(0)).unwrap().resources.get(2).copied().unwrap_or(Fixed::ZERO), Fixed::ZERO);
    // Switched back on, it earns again.
    sim.step(&[Command { player: PlayerId::BENCH, seq: 2, kind: CommandKind::Edit(Edit::Weapons(station, true)) }]);
    for _ in 0..PERIOD * 4 {
        sim.step(&[]);
    }
    assert!(sim.view().players.iter().find(|p| p.id == PlayerId(0)).unwrap().resources[2] > Fixed::ZERO);
}

#[test]
fn extreme_mod_values_cannot_wrap_into_a_negative_amount() {
    let mut c = case();
    c.node.mining_rate = Some(Fixed::from_int(2_000_000_000));
    c.node.amounts = [None, None, Some(Fixed::from_int(2_000_000_000)), None];
    c.station.multiplier = Some(Fixed::from_int(1_000_000_000));
    c.race = Some(Fixed::from_int(1_000_000_000));
    let (_, metal, _) = run(db(&c, true, true), v(RING, 0), PlayerId(0), PERIOD);
    // At most the cap a second over the living ticks.
    assert!(metal > Fixed::ZERO && metal < Fixed::from_int(1_300_000), "{metal:?}");
}

#[test]
fn income_needs_no_new_saved_state() {
    let (sim, metal, left) = run(db(&case(), true, true), v(RING, 0), PlayerId(0), 100);
    assert!(metal > Fixed::ZERO && left.is_some());
    // The stock and the node table are what the snapshot already carries.
    assert_eq!(Economy::from_rec(&sim.economy.to_rec()), sim.economy);
}

/// A new pin: the whole-state digest of a station earning over a run. Everything it depends on
/// is authored above, so a change here is a change of the income rule.
#[test]
fn the_digest_of_a_station_earning_over_a_run_is_pinned() {
    let (sim, _, _) = run(db(&case(), true, true), v(RING, 0), PlayerId(0), 300);
    assert_eq!(sim.digest(), 1_440_923_353_902_248_347);
}
