//! `beam_damage_at_end` (issue #1068): a beam's hit lands on the tick its life ends and is
//! lost in the cases the rule names. Through the whole sim, on the fake content with an
//! authored beam (life, range and reload invented for these tests).

use std::sync::Arc;

use sta_content::{ContentDb, Lookup, WeaponDef, WeaponKind};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use super::weapons::BEAM_BREAK_RANGE_FACTOR;
use crate::command::{Command, CommandKind, Order};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

/// The authored beam: its life in ticks, its range, and a reload longer than any test runs,
/// so exactly one beam is fired.
const LIFE: u64 = 20;
const RANGE: i32 = 400;
const RELOAD: u32 = 100_000;

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// `fake::db()` with the switch as given, the authored beam, and a shooter (`fx_beamer`)
/// that carries nothing else. `patch` is applied to the beam last.
fn db(switch: bool, patch: impl Fn(&mut sta_content::Features, &mut WeaponDef)) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.beam_damage_at_end = switch;
    let mut beam = None;
    for w in fake.weapons() {
        let mut w = w.clone();
        if w.odf == "fx_beam" {
            assert_eq!(w.kind, WeaponKind::Beam);
            w.duration = LIFE as _;
            w.range = Fixed::from_int(RANGE);
            w.refire = RELOAD as _;
            patch(&mut features, &mut w);
        }
        beam.get_or_insert_with(Vec::new).push(w);
    }
    let mut b = ContentDb::builder().profile(fake.profile()).features(features);
    for w in beam.unwrap() {
        b = b.weapon(w);
    }
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    let mut beamer = fake.units().find(|u| u.odf == "fx_cruiser").unwrap().clone();
    beamer.odf = "fx_beamer".into();
    beamer.hardpoints.retain(|h| h.weapon.odf == "fx_beam");
    assert_eq!(beamer.hardpoints.len(), 1);
    Arc::new(b.unit(beamer).build().unwrap())
}

struct Game {
    sim: Sim,
    db: Arc<ContentDb>,
    shooter: EntityId,
    target: EntityId,
    /// Ticks stepped so far.
    ticks: u64,
    /// The tick index of every `Fired` and every `Hit` of the shooter so far.
    fired: Vec<u64>,
    hits: Vec<(u64, Fixed)>,
}

/// The shooter and a frigate 300 units ahead of it, both at rest, the attack ordered.
fn game(db: Arc<ContentDb>) -> Game {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let setup = GameSetup { seed: 7, map: Arc::new(sta_content::MapDef::empty()), content: db.clone(), players: vec![seat(1), seat(2)], options: GameOptions::default() };
    let mut sim = Sim::from_setup(&setup);
    let shooter = sim.spawn_def(db.key("fx_beamer").unwrap(), PlayerId(1), v(2000, 2000), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let target = sim.spawn_def(db.key("fx_frigate").unwrap(), PlayerId(2), v(2000, 2300), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let mut g = Game { sim, db, shooter, target, ticks: 0, fired: Vec::new(), hits: Vec::new() };
    let order = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![shooter], target }) };
    g.step_with(&[order]);
    g
}

impl Game {
    fn step_with(&mut self, cmds: &[Command]) {
        for e in self.sim.step(cmds).events {
            match e {
                SimEvent::Fired { shooter, .. } if shooter == self.shooter => self.fired.push(self.ticks),
                SimEvent::Hit { by, shield, hull, .. } if by == self.shooter => self.hits.push((self.ticks, shield + hull)),
                _ => {}
            }
        }
        self.ticks += 1;
    }

    fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.step_with(&[]);
        }
    }

    /// Steps until the beam has fired; the tick it fired on.
    fn until_fired(&mut self) -> u64 {
        for _ in 0..200 {
            if let Some(t) = self.fired.first() {
                return *t;
            }
            self.step_with(&[]);
        }
        panic!("the beam never fired");
    }

    fn beams(&self) -> usize {
        self.sim.weapons.beams_in_flight()
    }

    /// Puts the target `distance` ahead of the shooter.
    fn target_at(&mut self, distance: i32) {
        self.sim.units.get_mut(self.target).unwrap().pos = v(2000, 2000 + distance);
    }
}

#[test]
fn the_hit_lands_on_the_tick_the_beams_life_ends() {
    let mut g = game(db(true, |_, _| {}));
    let fired = g.until_fired();
    assert_eq!(g.beams(), 1, "the beam is in flight from the tick it fired");
    g.run(LIFE - 1);
    assert!(g.hits.is_empty(), "no hit before the life ends: {:?}", g.hits);
    assert_eq!(g.beams(), 1);
    g.run(1);
    assert_eq!(g.hits.iter().map(|h| h.0).collect::<Vec<_>>(), [fired + LIFE]);
    assert!(g.hits[0].1 > Fixed::ZERO, "the hit carries the damage");
    assert_eq!(g.beams(), 0);
    g.run(2 * LIFE);
    assert_eq!((g.fired.len(), g.hits.len()), (1, 1), "one beam, one hit");
}

#[test]
fn with_the_switch_off_a_beam_hits_on_the_tick_it_fires_and_nothing_is_kept() {
    let mut on = game(db(true, |_, _| {}));
    let mut off = game(db(false, |_, _| {}));
    let fired = off.until_fired();
    assert_eq!(on.until_fired(), fired, "the switch does not move the shot");
    assert_eq!(off.hits.iter().map(|h| h.0).collect::<Vec<_>>(), [fired]);
    assert_eq!(off.beams(), 0);
    on.run(LIFE);
    off.run(LIFE);
    assert_eq!(on.hits[0].1, off.hits[0].1, "the same damage, later");
}

#[test]
fn a_beam_with_no_life_hits_at_once_under_the_switch() {
    let mut g = game(db(true, |_, w| w.duration = 0));
    let fired = g.until_fired();
    assert_eq!(g.hits.iter().map(|h| h.0).collect::<Vec<_>>(), [fired]);
    assert_eq!(g.beams(), 0);
}

#[test]
fn no_hit_when_the_shooter_is_gone_before_the_end() {
    let mut g = game(db(true, |_, _| {}));
    g.until_fired();
    g.run(LIFE / 2);
    assert!(g.sim.despawn(g.shooter, None));
    g.run(1);
    assert_eq!(g.beams(), 0, "the beam is dropped on the next tick");
    g.run(2 * LIFE);
    assert!(g.hits.is_empty(), "{:?}", g.hits);
}

#[test]
fn no_hit_when_the_target_is_gone_before_the_end() {
    let mut g = game(db(true, |_, _| {}));
    g.until_fired();
    g.run(LIFE / 2);
    assert!(g.sim.despawn(g.target, None));
    g.run(2 * LIFE);
    assert!(g.hits.is_empty(), "{:?}", g.hits);
    assert_eq!(g.beams(), 0);
}

#[test]
fn a_target_beyond_the_break_distance_loses_the_beam_for_good() {
    let limit = BEAM_BREAK_RANGE_FACTOR * RANGE;
    // Out of the weapon's range but inside the break distance: the beam holds and lands.
    let mut held = game(db(true, |_, _| {}));
    let fired = held.until_fired();
    held.target_at(limit);
    held.run(LIFE);
    assert_eq!(held.hits.iter().map(|h| h.0).collect::<Vec<_>>(), [fired + LIFE], "exactly at the limit still lands");

    // One unit beyond it for a single tick, then back in range before the life ends.
    let mut lost = game(db(true, |_, _| {}));
    lost.until_fired();
    lost.run(LIFE / 4);
    lost.target_at(limit + 1);
    lost.run(1);
    assert_eq!(lost.beams(), 0, "tested on every tick of the life, not only at its end");
    lost.target_at(300);
    lost.run(2 * LIFE);
    assert!(lost.hits.is_empty(), "a lost beam never lands later: {:?}", lost.hits);
}

#[test]
fn a_missed_roll_keeps_no_beam_and_applies_nothing() {
    // A chance of zero against a craft that can move: every roll misses.
    let miss = |f: &mut sta_content::Features, w: &mut WeaponDef| {
        f.weapon_hit_roll = true;
        w.base.hit_chance = Some(Lookup { base: Fixed::ZERO, rows: std::iter::empty::<(Arc<str>, Fixed)>().collect() });
    };
    let mut g = game(db(true, miss));
    g.until_fired();
    assert_eq!(g.beams(), 0, "a miss is not kept");
    g.run(2 * LIFE);
    assert_eq!(g.fired.len(), 1);
    assert!(g.hits.is_empty(), "{:?}", g.hits);
}

#[test]
fn a_snapshot_taken_mid_beam_reloads_to_the_same_result() {
    let mut live = game(db(true, |_, _| {}));
    let fired = live.until_fired();
    live.run(LIFE / 2);
    let bytes = live.sim.snapshot();
    let sim = Sim::from_snapshot(&bytes, Some(live.db.clone())).unwrap();
    assert_eq!(sim.weapons.beams_in_flight(), 1);
    assert_eq!(sim.snapshot(), bytes, "save, load, save");
    assert_eq!(sim.digest(), live.sim.digest());
    let mut back = Game { sim, db: live.db.clone(), shooter: live.shooter, target: live.target, ticks: live.ticks, fired: Vec::new(), hits: Vec::new() };
    for _ in 0..2 * LIFE {
        live.run(1);
        back.run(1);
        assert_eq!(back.sim.digest(), live.sim.digest(), "tick {}", live.ticks);
    }
    assert_eq!(live.hits.iter().map(|h| h.0).collect::<Vec<_>>(), [fired + LIFE]);
    assert_eq!(back.hits, live.hits, "the reloaded game lands the same hit on the same tick");
}

#[test]
fn a_beam_in_flight_is_in_the_digest_and_an_empty_list_is_not() {
    let mut g = game(db(true, |_, _| {}));
    g.until_fired();
    g.run(LIFE / 2);
    let with_beam = g.sim.digest();
    // The same world without its beam in flight is another state.
    let mut emptied = Sim::from_snapshot(&g.sim.snapshot(), Some(g.db.clone())).unwrap();
    assert_eq!(emptied.digest(), with_beam);
    emptied.weapons.set_beams(Vec::new());
    assert_ne!(emptied.digest(), with_beam, "the beam in flight is hashed");
    // Once the list is empty the weapons digest has no beams block: a snapshot from the
    // format before the record (the same bytes without it) loads to the same digest.
    g.run(LIFE);
    assert_eq!(g.beams(), 0);
    let bytes = g.sim.snapshot();
    assert_eq!(&bytes[bytes.len() - 2..], [1, 0], "the record: version 1, no rows");
    let mut old = bytes[..bytes.len() - 2].to_vec();
    old[4..6].copy_from_slice(&crate::snapshot::PRE_BEAMS_FORMAT.to_le_bytes());
    let previous = Sim::from_snapshot(&old, Some(g.db.clone())).unwrap();
    assert_eq!(previous.digest(), g.sim.digest());
    assert_eq!(previous.snapshot(), bytes, "and is written back in the current format");
}
