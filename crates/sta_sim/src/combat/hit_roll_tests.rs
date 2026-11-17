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
    db_with(
        |f| {
            f.weapon_hit_roll = hit_roll;
            f.weapon_target_damage = target_damage;
        },
        patch,
        |_| {},
    )
}

/// `fake::db()` with `switches` applied to its features and the patches to every weapon and unit.
fn db_with(switches: impl Fn(&mut Features), weapon: impl Fn(&mut WeaponDef), unit: impl Fn(&mut sta_content::UnitDef)) -> ContentDb {
    let fake = sta_content::fake::db();
    let mut features: Features = fake.features().clone();
    switches(&mut features);
    let mut b = ContentDb::builder().profile(fake.profile()).features(features);
    for u in fake.units() {
        let mut u = u.clone();
        unit(&mut u);
        b = b.unit(u);
    }
    for w in fake.weapons() {
        let mut w = w.clone();
        weapon(&mut w);
        b = b.weapon(w);
    }
    b.build().unwrap()
}

/// As [`db`], with only the target-mask switch as given.
fn masks(on: bool, weapon: impl Fn(&mut WeaponDef), unit: impl Fn(&mut sta_content::UnitDef)) -> ContentDb {
    db_with(|f| f.weapon_target_masks = on, weapon, unit)
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

#[test]
fn the_mask_switch_off_changes_nothing() {
    let plain = fight(masks(false, |_| {}, |_| {}), "fx_frigate", TICKS);
    let keyed = fight(masks(false, |w| w.base.targets.enemy_craft = Some(false), |u| u.stats.can_only_attack_planets = Some(true)), "fx_frigate", TICKS);
    assert_eq!(plain, keyed);
    // On, with no key set: the defaults allow enemy craft and stations, as before.
    // (The digest differs: the switch is part of the content's feature set.)
    let events = |r: (usize, usize, Option<Fixed>, u64)| (r.0, r.1, r.2);
    assert_eq!(events(fight(masks(true, |_| {}, |_| {}), "fx_frigate", TICKS)), events(plain));
    assert_eq!(events(fight(masks(true, |_| {}, |_| {}), "fx_station", TICKS)), events(fight(masks(false, |_| {}, |_| {}), "fx_station", TICKS)));
}

#[test]
fn a_weapon_that_may_not_target_enemy_craft_does_not_fire() {
    let no_craft = |w: &mut WeaponDef| w.base.targets.enemy_craft = Some(false);
    assert_eq!(fight(masks(true, no_craft, |_| {}), "fx_frigate", TICKS).0, 0);
    // The same flag is asked of an enemy station too.
    assert_eq!(fight(masks(true, no_craft, |_| {}), "fx_station", TICKS).0, 0);
    // `targetLocation` lifts every mask.
    let located = |w: &mut WeaponDef| {
        w.base.targets.enemy_craft = Some(false);
        w.base.targets.location = Some(true);
    };
    assert!(fight(masks(true, located, |_| {}), "fx_frigate", TICKS).0 > 3);
}

#[test]
fn enemy_buildings_and_planets_only_limit_the_weapon() {
    let no_buildings = |w: &mut WeaponDef| w.base.targets.enemy_buildings = Some(false);
    assert_eq!(fight(masks(true, no_buildings, |_| {}), "fx_station", TICKS).0, 0);
    assert!(fight(masks(true, no_buildings, |_| {}), "fx_frigate", TICKS).0 > 3, "craft are still targets");
    let planets_only = |w: &mut WeaponDef| w.base.targets.enemy_planets_only = Some(true);
    assert_eq!(fight(masks(true, planets_only, |_| {}), "fx_frigate", TICKS).0, 0);
}

#[test]
fn a_shooter_that_only_attacks_planets_fires_at_nothing_else() {
    let only = |u: &mut sta_content::UnitDef| u.stats.can_only_attack_planets = Some(true);
    assert_eq!(fight(masks(true, |_| {}, only), "fx_frigate", TICKS).0, 0);
}

#[test]
fn the_mask_keys_are_digested_only_under_their_switch() {
    let weapon = |w: &mut WeaponDef| w.base.targets.enemy_buildings = Some(false);
    let unit = |u: &mut sta_content::UnitDef| u.stats.can_attack_planets = Some(true);
    assert_eq!(masks(false, weapon, unit).hash(), sta_content::fake::db().hash());
    assert_ne!(masks(true, weapon, |_| {}).hash(), masks(true, |_| {}, |_| {}).hash());
    assert_ne!(masks(true, |_| {}, unit).hash(), masks(true, |_| {}, |_| {}).hash());
}

/// All hits (shield + hull) of the cruiser on a frigate under the casualty model.
fn hit_amounts(target_damage: bool, weapon: impl Fn(&mut WeaponDef)) -> Vec<Fixed> {
    let db = db_with(
        |f| {
            f.crew_loss_on_hit = true;
            f.weapon_target_damage = target_damage;
        },
        weapon,
        |_| {},
    );
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let (cruiser, target) = (db.key("fx_cruiser").unwrap(), db.key("fx_station").unwrap());
    let setup = GameSetup { seed: 7, map: Arc::new(sta_content::MapDef::empty()), content: Arc::new(db), players: vec![seat(1), seat(2)], options: GameOptions::default() };
    let mut sim = Sim::from_setup(&setup);
    let shooter = sim.spawn_def(cruiser, PlayerId(1), v(2000, 2000), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let target = sim.spawn_def(target, PlayerId(2), v(2000, 2300), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let order = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![shooter], target }) };
    let mut out = Vec::new();
    for t in 0..TICKS {
        let cmds = if t == 0 { vec![order.clone()] } else { Vec::new() };
        for e in sim.step(&cmds).events {
            if let SimEvent::Hit { by, shield, hull, .. } = e
                && by == shooter
            {
                out.push(shield + hull);
            }
        }
    }
    out
}

#[test]
fn a_variance_row_for_the_target_replaces_the_plain_spread() {
    let flat = |w: &mut WeaponDef| {
        w.damage_variance = Fixed::ZERO;
        w.base.damage_variance = Some(table(0.0, &[("fx_station.odf", 5.0)]));
    };
    let distinct = |v: &[Fixed]| {
        let mut v = v.to_vec();
        v.sort();
        v.dedup();
        v.len()
    };
    let off = hit_amounts(false, flat);
    let on = hit_amounts(true, flat);
    assert!(off.len() > 3 && on.len() > 3, "{off:?} {on:?}");
    // Per weapon kind the plain spread is zero, so off gives one amount per weapon.
    assert!(distinct(&on) > distinct(&off), "the row spreads the damage: {on:?} vs {off:?}");
}

#[test]
fn variance_rows_and_cloak_enable_are_digested_only_under_their_switch() {
    let rows = |w: &mut WeaponDef| w.base.damage_variance = Some(table(1.0, &[("fx_frigate", 3.0)]));
    let cloak = |w: &mut WeaponDef| w.base.cloak_enable = Some(true);
    assert_eq!(db(false, false, rows).hash(), sta_content::fake::db().hash());
    assert_ne!(db(false, true, rows).hash(), db(false, true, |_| {}).hash());
    let cloak_db = |on: bool, patch: &dyn Fn(&mut WeaponDef)| db_with(|f| f.weapon_cloak_fire = on, patch, |_| {});
    assert_eq!(cloak_db(false, &cloak).hash(), sta_content::fake::db().hash());
    assert_ne!(cloak_db(true, &cloak).hash(), cloak_db(true, &|_| {}).hash());
}
