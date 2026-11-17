//! Whether a shot hits, and what a hit does to this target (SIM-73).
//!
//! Both rules are read from the stock Armada II executable, static; neither is
//! measured, so both are behind feature switches that are off in every
//! profile (`weapon_hit_roll`, `weapon_target_damage`).
//!
//! **Hit roll.** A shot at a craft that can move hits when a uniform draw in
//! `0..1` is at most
//!
//! ```text
//! hitChance(target) + s * impulseToHitModifier + (1 - s) * stopToHitModifier
//!                   + t * toHitImpulseModifier + (1 - t) * toHitStopModifier
//! ```
//!
//! where `hitChance(target)` is the `hitChance` row for the target's ODF, else
//! the key's own value, else 1; `s` and `t` are the shooter's and the target's
//! speed as a fraction of their top speed, capped at 1. A shooter or target
//! with no top speed contributes neither of its two terms. A target that
//! cannot move is always hit (our reading of the original's "always hit"
//! test, which checks two flags we have not identified). Not reproduced: the
//! original subtracts a sensor-scrambler amount when the target carries that
//! effect; the sim has no such effect yet.
//!
//! **Damage.** A hit does the ordnance's `damageBase` row for the target's
//! ODF when it has one, else the plain value.
//!
//! **Damage variance.** Under the same switch, where a hit's damage varies
//! (the casualty model), the spread is the ordnance's `damageVariance` row
//! for the target's ODF when it has one, else the plain value.
//!
//! **Firing from a cloak** (`weapon_cloak_fire`, off everywhere). A fully
//! cloaked craft fires only weapons with `cloakEnable` set. The original
//! also lets a cloaked craft fire with a further unit flag and energy we
//! have not identified; not reproduced.
//!
//! **Not sim rules.** `ignoreShield` only decides whether an ordnance stops
//! at the shield bubble (with a shield-hit effect) or flies on to the hull;
//! the damage it does is the same either way (read from the executable,
//! static), so it belongs to the renderer. `needTarget` and `warpEnable`
//! belong to targeted specials and warp, which the sim does not have yet.
//!
//! **Target masks** (`weapon_target_masks`, also off everywhere). A weapon
//! with `targetLocation` set is not limited. Otherwise:
//!
//! - a shooter without `canAttackPlanets` does not fire at a planet, and one
//!   with `canOnlyAttackPlanets` fires at nothing else;
//! - at an enemy: a station needs `targetEnemyBuildings` (default on), any
//!   enemy needs `targetEnemyCraft` (default on), and `targetEnemyPlanetsOnly`
//!   limits the weapon to planets;
//! - at an ally: a station needs `targetAlliedBuildings` and any ally needs
//!   `targetAlliedCraft` (both default off);
//! - a neutral target is not limited by the masks.
//!
//! "Station" is the sim's station kind, standing in for the original's
//! building flag of the target's class (our reading). The original also
//! refuses an allied target carrying one further flag we have not identified,
//! and then asks the ordnance class; neither is reproduced.
//!
//! A table row names a unit ODF, with or without `.odf`, in any case.

use sta_content::{ContentDb, Lookup, WeaponDef};
use sta_core::Fixed;

use crate::world::{Entity, EntityKind};

/// The row of `table` for the unit ODF `odf`, if any. The last matching row
/// wins, as a later row replaces an earlier one of the same name.
fn row(table: &Lookup, odf: &str) -> Option<Fixed> {
    let want = odf.trim_end_matches(".odf");
    table
        .iter()
        .filter(|(name, _)| name.strip_suffix(".odf").unwrap_or(name).eq_ignore_ascii_case(want))
        .map(|(_, v)| v)
        .last()
}

/// Speed as a fraction of top speed, capped at 1; `None` for a unit with no top speed.
fn speed_fraction(u: &Entity) -> Option<Fixed> {
    let top = u.stats.max_speed;
    (top > Fixed::ZERO).then(|| (u.speed.abs() / top).min(Fixed::ONE))
}

/// The chance, before the draw, that `weapon` fired by `shooter` hits `target`.
/// `None`: the target is always hit and no draw is made.
pub fn hit_chance(db: &ContentDb, weapon: &WeaponDef, shooter: &Entity, target: &Entity) -> Option<Fixed> {
    let t = speed_fraction(target)?;
    let base = &weapon.base;
    let target_odf = db.unit(target.def).map_or("", |d| d.odf.as_str());
    let mut chance = base.hit_chance.as_ref().map_or(Fixed::ONE, |l| row(l, target_odf).unwrap_or(l.base));
    let term = |m: Option<Fixed>, share: Fixed| m.unwrap_or(Fixed::ZERO) * share;
    if let Some(s) = speed_fraction(shooter) {
        chance = chance + term(base.to_hit.impulse_to_hit, s) + term(base.to_hit.stop_to_hit, Fixed::ONE - s);
    }
    chance = chance + term(base.to_hit.to_hit_impulse, t) + term(base.to_hit.to_hit_stop, Fixed::ONE - t);
    Some(chance)
}

/// Does the shot hit? Draws once from `draw` only when the target can be missed.
pub fn will_hit(db: &ContentDb, weapon: &WeaponDef, shooter: &Entity, target: &Entity, draw: impl FnOnce() -> Fixed) -> bool {
    match hit_chance(db, weapon, shooter, target) {
        None => true,
        Some(chance) => draw() <= chance,
    }
}

/// What a hit of `weapon` does to `target` before shields: its `damageBase`
/// row for the target's ODF, else the plain value.
pub fn damage_for(db: &ContentDb, weapon: &WeaponDef, target: &Entity) -> Fixed {
    let target_odf = db.unit(target.def).map_or("", |d| d.odf.as_str());
    weapon.base.damage_base.as_ref().and_then(|l| row(l, target_odf)).unwrap_or(weapon.damage)
}

/// The spread of a hit of `weapon` on `target`: its `damageVariance` row
/// for the target's ODF, else the plain value.
pub fn variance_for(db: &ContentDb, weapon: &WeaponDef, target: &Entity) -> Fixed {
    let target_odf = db.unit(target.def).map_or("", |d| d.odf.as_str());
    weapon.base.damage_variance.as_ref().and_then(|l| row(l, target_odf)).unwrap_or(weapon.damage_variance)
}

/// May `weapon` be fired by a fully cloaked craft (`cloakEnable`)?
pub fn fires_from_cloak(weapon: &WeaponDef) -> bool {
    weapon.base.cloak_enable == Some(true)
}

/// How the shooter stands to a target, for [`can_target`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stance {
    Enemy,
    Ally,
    Neutral,
}

/// May `weapon` on `shooter` be fired at `target` (the target masks)?
pub fn can_target(db: &ContentDb, weapon: &WeaponDef, shooter: &Entity, target: &Entity, stance: Stance) -> bool {
    let m = &weapon.base.targets;
    if m.location == Some(true) {
        return true;
    }
    let planet = db.unit(target.def).is_some_and(|d| matches!(d.class.0.as_str(), "planet" | "planet_obj"));
    let stats = db.unit(shooter.def).map(|d| &d.stats);
    if planet && stats.and_then(|s| s.can_attack_planets) != Some(true) {
        return false;
    }
    if !planet && stats.and_then(|s| s.can_only_attack_planets) == Some(true) {
        return false;
    }
    let building = target.kind == EntityKind::Station;
    match stance {
        Stance::Enemy => {
            !(building && m.enemy_buildings == Some(false))
                && m.enemy_craft != Some(false)
                && !(m.enemy_planets_only == Some(true) && !planet)
        }
        Stance::Ally => !(building && m.allied_buildings != Some(true)) && m.allied_craft == Some(true),
        Stance::Neutral => true,
    }
}
