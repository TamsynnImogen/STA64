//! `ordnance_flight` (issue #1068): a torpedo's life, its capped turn while it seeks, its straight
//! flight when its target is lost, and what it keeps. Through the whole sim, on the fake content with
//! an authored torpedo (every value invented for these tests).

use std::sync::Arc;

use sta_content::{ContentDb, DefRef, Hardpoint, WeaponDef, WeaponKind};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use super::flight::{DEFAULT_OMEGA_TURN, SEEK_RANGE_FACTOR};
use crate::command::{Command, CommandKind, Order};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::view::UnitFlags;
use crate::{Sim, WorldSource};

/// The authored torpedo: speed (units per second), range, the life, and a reload longer than any
/// test runs, so exactly one torpedo is fired.
pub(crate) const SPEED: i32 = 300;
pub(crate) const RANGE: i32 = 900;
pub(crate) const LIFE_SECONDS: i32 = 4;
pub(crate) const LIFE: u64 = 4 * 30;
const RELOAD: u64 = 100_000;

pub(crate) fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// What a test sets on the authored torpedo besides the switch.
#[derive(Clone, Copy)]
pub(crate) struct Keys {
    pub life: Option<i32>,
    pub seek: Option<f64>,
    pub omega: Option<f64>,
    pub range: i32,
}

impl Default for Keys {
    fn default() -> Keys {
        Keys { life: Some(LIFE_SECONDS), seek: None, omega: None, range: RANGE }
    }
}

/// `fake::db()` with the switch as given, the authored torpedo, a shooter (`fx_torper`) that
/// carries nothing else, a target with an ordinary instant cloak (`fx_cloak_target`) and one with a
/// phase cloak (`fx_phaser`).
pub(crate) fn db(switch: bool, keys: Keys) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.ordnance_flight = switch;
    let mut b = ContentDb::builder().profile(fake.profile()).features(features);
    for w in fake.weapons() {
        let mut w = w.clone();
        if w.odf == "fx_torpedo" {
            w.shot_speed = Fixed::from_int(SPEED);
            w.range = Fixed::from_int(keys.range);
            w.refire = RELOAD;
            w.base.life_span = keys.life.map(Fixed::from_int);
            w.base.seek_time = keys.seek.map(Fixed::from_f64);
            w.base.omega_turn = keys.omega.map(Fixed::from_f64);
        }
        b = b.weapon(w);
    }
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    let mut torper = fake.units().find(|u| u.odf == "fx_cruiser").unwrap().clone();
    torper.odf = "fx_torper".into();
    torper.hardpoints.retain(|h| h.weapon.odf == "fx_torpedo");
    assert_eq!(torper.hardpoints.len(), 1);
    let mut phaser = fake.units().find(|u| u.odf == "fx_frigate").unwrap().clone();
    phaser.odf = "fx_phaser".into();
    phaser.hardpoints = vec![Hardpoint { weapon: DefRef::new("fx_phase_cloak"), node: "hp01".into(), arc: None }];
    let phase = WeaponDef::new("fx_phase_cloak", "phasecloakdevice", WeaponKind::Beam);
    let mut cloak_target = phaser.clone();
    cloak_target.odf = "fx_cloak_target".into();
    cloak_target.hardpoints = vec![Hardpoint { weapon: DefRef::new("fx_instant_cloak"), node: "hp01".into(), arc: None }];
    let instant = WeaponDef::new("fx_instant_cloak", "cloakingdeviceimp", WeaponKind::Beam);
    Arc::new(b.weapon(phase).weapon(instant).unit(torper).unit(phaser).unit(cloak_target).build().unwrap())
}

pub(crate) struct Game {
    pub sim: Sim,
    pub db: Arc<ContentDb>,
    pub shooter: EntityId,
    pub target: EntityId,
    /// Ticks stepped so far.
    pub ticks: u64,
    /// The tick index of every `Fired` of the shooter, and of every `Hit` with its damage.
    pub fired: Vec<u64>,
    pub hits: Vec<(u64, Fixed)>,
}

/// The shooter at (2000, 2000) facing +z and `target_def` at `at`, both at rest, the attack ordered.
pub(crate) fn game_at(db: Arc<ContentDb>, target_def: &str, at: Vec3) -> Game {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let setup = GameSetup { seed: 7, map: Arc::new(sta_content::MapDef::empty()), content: db.clone(), players: vec![seat(1), seat(2)], options: GameOptions::default() };
    let mut sim = Sim::from_setup(&setup);
    let shooter = sim.spawn_def(db.key("fx_torper").unwrap(), PlayerId(1), v(2000, 2000), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let target = sim.spawn_def(db.key(target_def).unwrap(), PlayerId(2), at, (Fixed::ZERO, Fixed::ONE)).unwrap();
    let mut g = Game { sim, db, shooter, target, ticks: 0, fired: Vec::new(), hits: Vec::new() };
    let order = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![shooter], target }) };
    g.step_with(&[order]);
    g
}

/// [`game_at`] with a frigate 300 units ahead.
pub(crate) fn game(db: Arc<ContentDb>) -> Game {
    game_at(db, "fx_frigate", v(2000, 2300))
}

impl Game {
    pub(crate) fn step_with(&mut self, cmds: &[Command]) {
        for e in self.sim.step(cmds).events {
            match e {
                SimEvent::Fired { shooter, .. } if shooter == self.shooter => self.fired.push(self.ticks),
                SimEvent::Hit { by, shield, hull, .. } if by == self.shooter => self.hits.push((self.ticks, shield + hull)),
                _ => {}
            }
        }
        self.ticks += 1;
    }

    pub(crate) fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.step_with(&[]);
        }
    }

    /// Steps until the torpedo has fired; the tick it fired on.
    pub(crate) fn until_fired(&mut self) -> u64 {
        for _ in 0..200 {
            if let Some(t) = self.fired.first() {
                return *t;
            }
            self.step_with(&[]);
        }
        panic!("the torpedo never fired");
    }

    pub(crate) fn shots(&self) -> usize {
        self.sim.weapons.shots_in_flight()
    }

    /// The heading of the one torpedo in flight.
    pub(crate) fn heading(&self) -> (Fixed, Fixed) {
        let rows = self.sim.weapons.flights_rec();
        assert_eq!(rows.len(), 1, "one flown torpedo");
        rows[0].1.heading
    }

    /// Runs until the first hit; its tick.
    fn until_hit(&mut self) -> u64 {
        for _ in 0..400 {
            if let Some(h) = self.hits.first() {
                return h.0;
            }
            self.step_with(&[]);
        }
        panic!("no hit");
    }
}

/// The angle between two unit vectors, radians.
fn angle(a: (Fixed, Fixed), b: (Fixed, Fixed)) -> f64 {
    crate::kinematics::atan2(a.0 * b.1 - a.1 * b.0, a.0 * b.0 + a.1 * b.1).to_f32() as f64
}

const FORWARD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

#[test]
fn a_torpedo_whose_life_runs_out_before_it_arrives_vanishes_with_no_hit() {
    let mut g = game(db(true, Keys { life: Some(1), ..Keys::default() }));
    let fired = g.until_fired();
    // 1 s life: 30 ticks, 300 units a second: the target is 300 away and is reached only after the life ends
    let life = 30;
    g.run(life - 1);
    assert_eq!(g.shots(), 1, "in flight until the tick before its life ends");
    g.run(1);
    assert_eq!(g.shots(), 0, "gone on the tick the life has run out");
    g.run(2 * LIFE);
    assert!(g.hits.is_empty(), "no hit event: {:?}", g.hits);
    assert_eq!(g.fired, [fired], "one torpedo");
}

#[test]
fn with_no_launch_phase_it_homes_from_the_start_and_arrives_when_the_old_flight_would() {
    let mut on = game(db(true, Keys::default()));
    let mut off = game(db(false, Keys::default()));
    on.until_fired();
    off.until_fired();
    assert_eq!(on.until_hit(), off.until_hit(), "no seekTime: perfect homing, as before");
    assert_eq!((on.shots(), off.shots()), (0, 0));
    assert_eq!(on.hits[0].1, off.hits[0].1);
    assert!(off.sim.weapons.flights_rec().is_empty(), "the switch off keeps no flight");
}

#[test]
fn while_it_seeks_the_turn_is_capped_and_afterwards_it_points_at_the_target() {
    // the target is 300 to the side of a shooter facing +z
    let keys = Keys { seek: Some(2.0), omega: Some(0.5), ..Keys::default() };
    let mut straight = game_at(db(false, Keys::default()), "fx_frigate", v(2300, 2000));
    let mut g = game_at(db(true, keys), "fx_frigate", v(2300, 2000));
    straight.until_fired();
    g.until_fired();
    let per_tick = 0.5 / 30.0;
    let mut last = g.heading();
    assert!(angle(FORWARD, last).abs() < 0.01, "launched along the shooter's heading (INTERIM)");
    for k in 1..=60 {
        g.run(1);
        let now = g.heading();
        let turned = angle(last, now);
        assert!((turned.abs() - per_tick).abs() < 0.002, "tick {k}: turned {turned}");
        last = now;
    }
    // the seek time is over: the next tick points straight at the target
    g.run(1);
    let at = g.sim.unit(g.target).unwrap().pos;
    let shot_at_target = g.heading();
    assert!(shot_at_target.0 > Fixed::ZERO, "pointing toward the target's side: {shot_at_target:?} {at:?}");
    assert!(angle(last, shot_at_target).abs() > per_tick * 1.5, "it snaps to the target: no turn limit after the seek time");
    let (a, b) = (g.until_hit(), straight.until_hit());
    assert!(a > b + 20, "the seeking launch takes longer than homing from the start: {a} {b}");
    assert!(a < g.fired[0] + LIFE, "and arrives within its life");
}

#[test]
fn a_target_beyond_the_seek_range_is_not_steered_toward() {
    // short range: the torpedo fires at a target 100 away to the side, which then jumps beyond twice the range
    let keys = Keys { seek: Some(2.0), omega: Some(0.5), range: 150, ..Keys::default() };
    let mut g = game_at(db(true, keys), "fx_frigate", v(2100, 2000));
    g.until_fired();
    g.sim.units.get_mut(g.target).unwrap().pos = v(2000 + 100 * (SEEK_RANGE_FACTOR + 1) + 1000, 2000);
    let before = g.heading();
    g.run(10);
    assert_eq!(g.heading(), before, "flying straight on");
    g.run(LIFE);
    assert!(g.hits.is_empty());
    assert_eq!(g.shots(), 0, "until its life ends");
}

#[test]
fn a_target_that_is_gone_is_lost_and_the_torpedo_flies_straight_on_until_its_life_ends() {
    let keys = Keys { seek: Some(2.0), omega: Some(0.5), ..Keys::default() };
    let mut g = game_at(db(true, keys), "fx_frigate", v(2300, 2000));
    let fired = g.until_fired();
    g.run(10);
    let target = g.target;
    assert!(g.sim.despawn(target, None));
    g.run(1);
    let heading = g.heading();
    g.run(40);
    assert_eq!(g.heading(), heading, "straight on");
    assert_eq!(g.shots(), 1);
    g.run(LIFE - 51 - 1);
    assert_eq!(g.shots(), 1, "still in the air on the tick before its life ends");
    g.run(1);
    assert_eq!(g.shots(), 0);
    assert!(g.hits.is_empty(), "no hit, no event: {:?} (fired {fired})", g.hits);
}

#[test]
fn a_target_that_cloaks_is_kept_and_hit() {
    let mut g = game_at(db(true, Keys::default()), "fx_cloak_target", v(2000, 2300));
    g.until_fired();
    g.run(3);
    let target = g.target;
    g.step_with(&[Command { player: PlayerId(2), seq: 0, kind: CommandKind::Order(Order::Cloak { units: vec![target], on: true }) }]);
    assert_eq!(g.sim.specials.cloak_state(target), Some(crate::specials::CloakState::Cloaked), "fully cloaked at once");
    let target = g.sim.unit(g.target).unwrap().clone();
    assert!(!g.sim.specials.phased(&g.db, &target), "an ordinary cloak does not phase");
    assert!(g.until_hit() > g.fired[0]);
    assert_eq!(g.hits.len(), 1);
}

#[test]
fn a_target_that_starts_to_die_is_still_followed_and_hit() {
    let mut g = game(db(true, Keys::default()));
    g.until_fired();
    g.run(3);
    let dying = g.sim.units.get_mut(g.target).unwrap();
    dying.flags = UnitFlags(dying.flags.0 | UnitFlags::DYING.0);
    dying.hull[0] = Fixed::ZERO;
    g.until_hit();
    assert_eq!(g.hits.len(), 1);
}

#[test]
fn a_phased_target_is_lost() {
    let mut g = game_at(db(true, Keys::default()), "fx_phaser", v(2000, 2300));
    g.until_fired();
    g.run(2);
    let target = g.target;
    g.step_with(&[Command { player: PlayerId(2), seq: 0, kind: CommandKind::Order(Order::Cloak { units: vec![target], on: true }) }]);
    let t = g.sim.unit(g.target).unwrap().clone();
    assert!(g.sim.specials.phased(&g.db, &t));
    g.run(LIFE);
    assert!(g.hits.is_empty(), "{:?}", g.hits);
    assert_eq!(g.shots(), 0);
}

#[test]
fn a_torpedo_with_no_life_flies_as_without_the_switch() {
    let mut g = game(db(true, Keys { life: None, ..Keys::default() }));
    g.until_fired();
    assert_eq!(g.shots(), 1);
    assert!(g.sim.weapons.flights_rec().is_empty(), "no lifeSpan: no flight");
    g.until_hit();
}

#[test]
fn the_turn_rate_defaults_to_a_named_constant_and_the_switch_digests_only_the_keys_it_reads() {
    // the keys enter the content hash for a torpedo under the switch only
    let hash = |switch: bool, keys: Keys| db(switch, keys).hash();
    let other = Keys { omega: Some(1.5), ..Keys::default() };
    assert_ne!(hash(true, Keys::default()), hash(true, other));
    assert_eq!(hash(false, Keys::default()), hash(false, other), "off: the keys are not read");
    assert_eq!(DEFAULT_OMEGA_TURN, Fixed::ONE);
}

#[test]
fn a_snapshot_in_flight_loads_to_the_same_game() {
    let keys = Keys { seek: Some(2.0), omega: Some(0.5), ..Keys::default() };
    let mut g = game_at(db(true, keys), "fx_frigate", v(2300, 2000));
    g.until_fired();
    g.run(20);
    let bytes = g.sim.snapshot();
    let mut back = Sim::from_snapshot(&bytes, g.sim.content.clone()).unwrap();
    assert_eq!(back.snapshot(), bytes, "save, load, save");
    assert_eq!(back.digest(), g.sim.digest());
    assert_eq!(back.weapons.flights_rec(), g.sim.weapons.flights_rec());
    for _ in 0..120 {
        back.step(&[]);
        g.sim.step(&[]);
        assert_eq!(back.digest(), g.sim.digest());
    }
}

/// The snapshot as the format before the flights record wrote it: the same bytes without it.
fn as_previous_format(bytes: &[u8], rows: &[(u32, super::flight::Flight)]) -> Vec<u8> {
    let record = postcard::to_allocvec(&(1u8, rows.to_vec())).unwrap();
    let mut old = bytes[..bytes.len() - record.len()].to_vec();
    old[4..6].copy_from_slice(&crate::snapshot::PRE_FLIGHT_FORMAT.to_le_bytes());
    old
}

#[test]
fn a_snapshot_from_before_the_flights_gives_a_torpedo_in_flight_its_heading_at_the_target_and_a_life_from_the_load() {
    let keys = Keys { seek: Some(2.0), omega: Some(0.5), ..Keys::default() };
    let mut g = game_at(db(true, keys), "fx_frigate", v(2300, 2000));
    g.until_fired();
    g.run(20);
    let rows = g.sim.weapons.flights_rec();
    assert_eq!(rows.len(), 1);
    let old = as_previous_format(&g.sim.snapshot(), &rows);
    let mut back = Sim::from_snapshot(&old, g.sim.content.clone()).expect("the previous format loads");
    let loaded = back.weapons.flights_rec();
    assert_eq!(loaded.len(), 1, "the torpedo in flight has a flight");
    let (id, f) = loaded[0];
    assert_eq!(id, rows[0].0);
    // the shot has flown on ahead of the shooter and its target is behind it, to the side
    assert!(f.heading.0 > Fixed::ZERO && f.heading.1 < Fixed::ZERO, "toward its target: {:?}", f.heading);
    let len = (f.heading.0 * f.heading.0 + f.heading.1 * f.heading.1).sqrt();
    assert!((len.to_f32() - 1.0).abs() < 0.001, "a direction");
    assert!(f.ends > g.sim.tick, "a life counted from the load");
    assert_eq!(f.ends, back.tick + LIFE, "the full life from the load");
    // and it goes on to hit
    let mut hit = false;
    for _ in 0..LIFE {
        hit |= back.step(&[]).events.iter().any(|e| matches!(e, SimEvent::Hit { by, .. } if *by == g.shooter));
    }
    assert!(hit);
    // a game without the switch has nothing to give
    let mut off = game_at(db(false, keys), "fx_frigate", v(2300, 2000));
    off.until_fired();
    off.run(5);
    assert_eq!(off.shots(), 1);
    let old = as_previous_format(&off.sim.snapshot(), &[]);
    let back = Sim::from_snapshot(&old, off.sim.content.clone()).unwrap();
    assert!(back.weapons.flights_rec().is_empty());
    assert_eq!(back.digest(), off.sim.digest());
}

#[test]
fn a_malformed_flights_record_is_refused() {
    let keys = Keys { seek: Some(2.0), omega: Some(0.5), ..Keys::default() };
    let mut g = game_at(db(true, keys), "fx_frigate", v(2300, 2000));
    g.until_fired();
    let bytes = g.sim.snapshot();
    let rows = g.sim.weapons.flights_rec();
    let good = rows[0];
    let with = |rows: Vec<(u32, super::flight::Flight)>| {
        let record = postcard::to_allocvec(&(1u8, g.sim.weapons.flights_rec())).unwrap();
        let mut forged = bytes[..bytes.len() - record.len()].to_vec();
        forged.extend(postcard::to_allocvec(&(1u8, rows)).unwrap());
        Sim::from_snapshot(&forged, g.sim.content.clone()).map(|_| ())
    };
    assert!(with(vec![good]).is_ok());
    assert!(with(vec![(good.0 + 1, good.1)]).is_err(), "no such shot");
    assert!(with(vec![good, good]).is_err(), "ids must ascend");
    let mut still = good.1;
    still.heading = (Fixed::ZERO, Fixed::ZERO);
    assert!(with(vec![(good.0, still)]).is_err(), "a heading is a direction");
    let mut long = good.1;
    long.heading = (Fixed::from_int(3), Fixed::ZERO);
    assert!(with(vec![(good.0, long)]).is_err(), "a long heading would multiply the step");
    let mut near = good.1;
    near.heading = (Fixed::from_f64(0.9997), Fixed::ZERO);
    assert!(with(vec![(good.0, near)]).is_ok(), "within the tolerance a direction");
    let tick = g.sim.tick;
    let mut late = good.1;
    late.ends = tick + LIFE + 1;
    assert!(with(vec![(good.0, late)]).is_err(), "beyond a life from now: a torpedo that would never die");
    late.ends = u64::MAX;
    assert!(with(vec![(good.0, late)]).is_err());
    let mut early = good.1;
    early.ends = tick - 1;
    assert!(with(vec![(good.0, early)]).is_err(), "its life has run out already");
    // the latest and the earliest that a live game can have made load, and stepping on does not underflow
    for ends in [tick, tick + LIFE] {
        let mut edge = good.1;
        edge.ends = ends;
        assert!(with(vec![(good.0, edge)]).is_ok(), "{ends}");
    }
    assert!(with(Vec::new()).is_ok(), "a shot may have none: it homes as without the switch");
    // a row for a shot whose weapon does not fly this way (the switch is off for it)
    let mut off = game_at(db(false, keys), "fx_frigate", v(2300, 2000));
    off.until_fired();
    let bytes = off.sim.snapshot();
    let record = postcard::to_allocvec(&(1u8, Vec::<(u32, super::flight::Flight)>::new())).unwrap();
    let mut forged = bytes[..bytes.len() - record.len()].to_vec();
    forged.extend(postcard::to_allocvec(&(1u8, vec![good])).unwrap());
    assert!(Sim::from_snapshot(&forged, off.sim.content.clone()).is_err());
}

#[test]
fn the_view_lists_a_flown_shot_with_its_place_and_heading_until_it_hits_and_not_without_the_switch() {
    let keys = Keys { seek: Some(2.0), omega: Some(0.5), ..Keys::default() };
    let mut g = game_at(db(true, keys), "fx_frigate", v(2300, 2000));
    g.until_fired();
    g.run(5);
    let view = g.sim.view();
    assert_eq!(view.shot_rows.len(), 1);
    let (row, flight) = (view.shot_rows[0], g.sim.weapons.flights_rec()[0]);
    assert_eq!((row.id, row.shooter, row.target, row.heading), (flight.0, g.shooter, g.target, flight.1.heading));
    assert!(view.shot_row(row.id).is_some() && view.shot_row(row.id + 1).is_none());
    assert!(row.pos.z > Fixed::from_int(2000), "it has flown ahead of its shooter");
    g.until_hit();
    g.run(1);
    assert!(g.sim.view().shot_rows.is_empty(), "no row once it has hit");
    // the switch off: the shot has no flight and no row
    let mut off = game_at(db(false, keys), "fx_frigate", v(2300, 2000));
    off.until_fired();
    off.run(5);
    assert_eq!(off.shots(), 1);
    assert!(off.sim.view().shot_rows.is_empty());
}
