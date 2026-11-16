//! SIM-73: the hit roll and per-target damage, through the whole sim on the
//! fake content with authored weapon keys.

use std::sync::Arc;

use sta_content::{ContentDb, Features, Lookup, WeaponDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Order};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn table(base: f64, rows: &[(&str, f64)]) -> Lookup {
    Lookup { base: Fixed::from_f64(base), rows: rows.iter().map(|(n, v)| (Arc::<str>::from(*n), Fixed::from_f64(*v))).collect() }
}

/// `fake::db()` with the two switches set and `patch` applied to every weapon.
fn db(hit_roll: bool, target_damage: bool, patch: impl Fn(&mut WeaponDef)) -> ContentDb {
    let fake = sta_content::fake::db();
    let mut features: Features = fake.features().clone();
    features.weapon_hit_roll = hit_roll;
    features.weapon_target_damage = target_damage;
    let mut b = ContentDb::builder().profile(fake.profile()).features(features);
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    for w in fake.weapons() {
        let mut w = w.clone();
        patch(&mut w);
        b = b.weapon(w);
    }
    b.build().unwrap()
}

/// A cruiser ordered to attack `target_def`, 300 units ahead, both at rest.
/// Returns `(shots fired, hits, damage of the first hit)` of the cruiser over
/// `ticks` ticks, and the final digest.
fn fight(db: ContentDb, target_def: &str, ticks: u64) -> (usize, usize, Option<Fixed>, u64) {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let (cruiser, target) = (db.key("fx_cruiser").unwrap(), db.key(target_def).unwrap());
    let setup = GameSetup { seed: 7, map: Arc::new(sta_content::MapDef::empty()), content: Arc::new(db), players: vec![seat(1), seat(2)], options: GameOptions::default() };
    let mut sim = Sim::from_setup(&setup);
    let shooter: EntityId = sim.spawn_def(cruiser, PlayerId(1), v(2000, 2000), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let target: EntityId = sim.spawn_def(target, PlayerId(2), v(2000, 2300), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let order = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![shooter], target }) };
    let (mut fired, mut hits, mut first) = (0, 0, None);
    for t in 0..ticks {
        let cmds = if t == 0 { vec![order.clone()] } else { Vec::new() };
        for e in sim.step(&cmds).events {
            match e {
                SimEvent::Fired { shooter: s, .. } if s == shooter => fired += 1,
                SimEvent::Hit { by, shield, hull, .. } if by == shooter => {
                    hits += 1;
                    first.get_or_insert(shield + hull);
                }
                _ => {}
            }
        }
    }
    (fired, hits, first, sim.digest())
}

const TICKS: u64 = 600;

#[test]
fn switches_off_change_nothing_whatever_the_keys_say() {
    let plain = fight(db(false, false, |_| {}), "fx_frigate", TICKS);
    let keyed = fight(
        db(false, false, |w| {
            w.base.hit_chance = Some(table(0.0, &[]));
            w.base.damage_base = Some(table(5.0, &[("fx_frigate", 1.0)]));
        }),
        "fx_frigate",
        TICKS,
    );
    assert!(plain.0 > 3 && plain.1 > 3, "the cruiser fires and hits");
    assert_eq!(plain, keyed, "same shots, hits, damage and digest");
}

#[test]
fn a_zero_chance_never_hits_a_craft_that_can_move() {
    let (fired, hits, ..) = fight(db(true, false, |w| w.base.hit_chance = Some(table(0.0, &[]))), "fx_frigate", TICKS);
    assert!(fired > 3, "{fired} shots");
    assert_eq!(hits, 0);
}

#[test]
fn the_row_for_the_target_replaces_the_base_chance() {
    let patch = |w: &mut WeaponDef| w.base.hit_chance = Some(table(0.0, &[("other.odf", 0.0), ("FX_Frigate.odf", 2.0)]));
    let (fired, hits, ..) = fight(db(true, false, patch), "fx_frigate", TICKS);
    assert!(fired > 3 && hits > 0, "{fired} shots, {hits} hits");
    // Every beam hits on its tick; a torpedo still in flight at the end is the only shortfall.
    assert!(fired - hits <= 2, "{fired} shots, {hits} hits");
}

#[test]
fn the_target_at_rest_takes_its_stop_modifier() {
    let patch = |w: &mut WeaponDef| {
        w.base.hit_chance = Some(table(0.0, &[]));
        w.base.to_hit.to_hit_stop = Some(Fixed::from_int(2));
        // The moving-state terms do not count for units at rest.
        w.base.to_hit.to_hit_impulse = Some(Fixed::from_int(-5));
        w.base.to_hit.impulse_to_hit = Some(Fixed::from_int(-5));
    };
    let (fired, hits, ..) = fight(db(true, false, patch), "fx_frigate", TICKS);
    assert!(fired > 3 && fired - hits <= 2, "{fired} shots, {hits} hits");
    let never = |w: &mut WeaponDef| {
        w.base.hit_chance = Some(table(1.0, &[]));
        w.base.to_hit.stop_to_hit = Some(Fixed::from_int(-2));
    };
    assert_eq!(fight(db(true, false, never), "fx_frigate", TICKS).1, 0, "the shooter at rest takes its own stop modifier");
}

#[test]
fn a_target_that_cannot_move_is_always_hit() {
    // Exactly as with the switch off: no draw is made for it.
    let rolled = fight(db(true, false, |w| w.base.hit_chance = Some(table(0.0, &[]))), "fx_station", TICKS);
    let plain = fight(db(false, false, |w| w.base.hit_chance = Some(table(0.0, &[]))), "fx_station", TICKS);
    assert!(rolled.1 > 3, "{} hits", rolled.1);
    assert_eq!((rolled.0, rolled.1, rolled.2), (plain.0, plain.1, plain.2));
}

#[test]
fn a_half_chance_hits_some_and_misses_some_and_repeats() {
    let run = || fight(db(true, false, |w| w.base.hit_chance = Some(table(0.5, &[]))), "fx_frigate", 3000);
    let (fired, hits, _, digest) = run();
    assert!(hits > fired / 5 && hits < fired * 4 / 5, "{fired} shots, {hits} hits");
    assert_eq!(run().3, digest, "the roll is deterministic");
}

#[test]
fn a_damage_row_for_the_target_replaces_the_plain_damage() {
    let plain = fight(db(false, true, |_| {}), "fx_frigate", TICKS).2.unwrap();
    let rowed = fight(db(false, true, |w| w.base.damage_base = Some(table(99.0, &[("fx_frigate.odf", 1.0)]))), "fx_frigate", TICKS).2.unwrap();
    assert_eq!(rowed, Fixed::ONE);
    assert!(plain > Fixed::ONE);
    // A table with no row for this target leaves the plain value.
    let other = fight(db(false, true, |w| w.base.damage_base = Some(table(99.0, &[("fx_station.odf", 1.0)]))), "fx_frigate", TICKS).2.unwrap();
    assert_eq!(other, plain);
}

#[test]
fn the_keys_are_digested_only_under_their_switch() {
    let chance = |w: &mut WeaponDef| w.base.hit_chance = Some(table(0.5, &[("fx_frigate", 0.25)]));
    let rows = |w: &mut WeaponDef| w.base.damage_base = Some(table(9.0, &[("fx_frigate", 1.0)]));
    // Off: the keys are typed only and the pinned fake hash holds.
    assert_eq!(db(false, false, chance).hash(), sta_content::fake::db().hash());
    assert_eq!(db(false, false, rows).hash(), sta_content::fake::db().hash());
    // On: a key the rule reads moves the hash; the other rule's key does not.
    assert_ne!(db(true, false, chance).hash(), db(true, false, |_| {}).hash());
    assert_eq!(db(true, false, rows).hash(), db(true, false, |_| {}).hash());
    assert_ne!(db(false, true, rows).hash(), db(false, true, |_| {}).hash());
    assert_eq!(db(false, true, chance).hash(), db(false, true, |_| {}).hash());
    let modifier = |w: &mut WeaponDef| w.base.to_hit.to_hit_stop = Some(Fixed::ONE);
    assert_ne!(db(true, false, modifier).hash(), db(true, false, |_| {}).hash());
}
