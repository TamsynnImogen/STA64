//! Damage (SIM-05): applies the `Hit`s the weapons emitted this tick.
//!
//! - Shields take a hit first: the facing it struck with directional shields,
//!   the single pool otherwise. What the shield cannot absorb goes to the hull.
//! - [`apply`] rewrites each `Hit` in place so `shield` and `hull` are the
//!   damage each really took (weapons emit the raw damage as `hull`), and
//!   follows it with `ShieldsDown { facing }` when that hit emptied a pool.
//! - A hull at zero destroys the entity at the end of the tick's hits. The
//!   killer is the `by` of the first lethal `Hit`; kills are returned in that
//!   order, for `Sim::despawn` (`Destroyed`, removal). Later hits that tick on
//!   a unit already dead do no damage.
//! - [`recharge`] gives every shield pool its definition's `recharge` per tick
//!   (capped at `max`), before the tick's hits. No delay after a hit.
//!
//! - Planets with a population row (SIM-30): the hull damage of a hit becomes
//!   population loss (`0.1 x damage`) and the hull is untouched, hit by hit.
//!
//! - Crew (SIM-39, `Features::crew_loss_on_hit`): the engine's casualty model
//!   (damage variance, shield protection, one location roll per hit, system
//!   hit points, crew loss, life-support drain, derelict latch) is
//!   [`super::casualty`]; without the feature a hit is shields, then hull.
//!
//! - Mission edits (SIM-55): [`Sim::edit_shield_percent`] and
//!   [`Sim::edit_max_shields`] write the shield pools, [`Subsystems`] keeps the
//!   switches and damage points of the shield generator, life support and
//!   sensors (the engines and weapons switches stay unit flags), and damage
//!   points of all five systems.
//!
//! Shields, hull and crew live on the entity, so snapshots and the digest
//! already cover them; [`Subsystems`] is the one state of its own.
//!
//! Not yet: what a switched-off or damaged system does. The sim records it
//! and nothing else reads it: the original's effect (a dead shield generator
//! absorbing nothing, life support draining crew, sensors) is not measured.
//! Systems damage from hits (weapons or engines offline below a hull
//! threshold) is not modelled either.

use serde::{Deserialize, Serialize};
use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId};

use crate::command::Subsystem;
use crate::event::SimEvent;
use crate::view::UnitFlags;
use crate::world::{Entities, EntityKind, Fnv, Table};
use crate::Sim;

/// Recharge every unit's shields by its definition's rate.
pub(crate) fn recharge(units: &mut Entities, db: &ContentDb, switches: &crate::switches::SwitchTable, subsystems: &Subsystems) {
    for u in units.iter_mut() {
        // `CraftCanRepair(false)` (SIM-58) stops the shields coming back too.
        if u.hull[0] <= Fixed::ZERO || switches.no_repair(u.id) || subsystems.get(u.id).is_some_and(|r| r.derelict) {
            continue;
        }
        let Some(def) = db.unit(u.def) else { continue };
        let (rate, max) = (def.shields.recharge, u.shields.max);
        if rate <= Fixed::ZERO {
            continue;
        }
        let pools = if u.shields.directional { 4 } else { 1 };
        for now in &mut u.shields.now[..pools] {
            if *now < max {
                *now = (*now + rate).min(max);
            }
        }
    }
}

/// Apply the `Hit`s in `events[from..]`; returns `(killed, killer)` in the
/// order the lethal hits came.
pub(crate) fn apply(
    units: &mut Entities,
    db: &ContentDb,
    events: &mut Vec<SimEvent>,
    from: usize,
    ctx: &mut super::casualty::Ctx,
) -> Vec<(EntityId, Option<EntityId>)> {
    let mut killed = Vec::new();
    let tail = events.split_off(from);
    for e in tail {
        let SimEvent::Hit {
            target,
            by,
            weapon,
            facing,
            hull: damage,
            at,
            ..
        } = e
        else {
            events.push(e);
            continue;
        };
        let (mut shield, mut hull, mut down) = (Fixed::ZERO, Fixed::ZERO, false);
        if let Some(t) = units.get_mut(target).filter(|t| t.hull[0] > Fixed::ZERO) {
            let pool = if t.shields.directional {
                usize::from(facing).min(3)
            } else {
                0
            };
            // SIM-39: the engine's casualty model for ordnance that has it.
            let model = db.unit(t.def).zip(db.weapon(weapon)).filter(|(_, w)| db.weapon_features(w).crew_loss_on_hit);
            let floor = if t.flags.contains(UnitFlags::IMMORTAL) { Fixed::ONE } else { Fixed::ZERO };
            let wanted = match model {
                Some((def, w)) => {
                    let (s, h, d) = super::casualty::hit(t, def, w, db, ctx, pool, damage);
                    (shield, down) = (s, d);
                    h
                }
                None => {
                    let held = t.shields.now[pool];
                    shield = damage.min(held);
                    t.shields.now[pool] = held - shield;
                    down = shield > Fixed::ZERO && t.shields.now[pool] == Fixed::ZERO;
                    damage - shield
                }
            };
            if ctx.colonies.has_row(target) {
                // SIM-30, a planet with a population row: damage that reaches the
                // hull costs `0.1 x damage` people and the hull stays, so every hit
                // of the tick counts in full (read from the executable, static).
                hull = wanted.max(Fixed::ZERO);
                ctx.colonies.lose_population(target, hull);
            } else {
                hull = wanted.min((t.hull[0] - floor).max(Fixed::ZERO));
                t.hull[0] = t.hull[0] - hull;
                if t.hull[0] <= Fixed::ZERO {
                    killed.push((target, Some(by)));
                }
            }
        }
        events.push(SimEvent::Hit {
            target,
            by,
            weapon,
            facing,
            shield,
            hull,
            at,
        });
        if down {
            events.push(SimEvent::ShieldsDown { id: target, facing });
        }
    }
    killed
}

/// The five systems, in the order of `UnitDef::system_hit_points`.
fn system(sub: Subsystem) -> usize {
    match sub {
        Subsystem::Engines => 0,
        Subsystem::Weapons => 1,
        Subsystem::Shields => 2,
        Subsystem::LifeSupport => 3,
        Subsystem::Sensors => 4,
    }
}

/// One unit's switched-off systems (bit `n` of `off` is system `n`, only the
/// shield generator, life support and sensors) and damage points per system,
/// as fixed-point bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubsystemRow {
    pub off: u8,
    /// The derelict latch (SIM-39): crew fell under 1.
    pub derelict: bool,
    pub damage: [i64; 5],
}

/// The units that have a switched-off or damaged system (SIM-55); a unit with
/// neither has no row. In the digest and the snapshot once any exists.
#[derive(Debug, Clone, Default)]
pub struct Subsystems {
    rows: Table<SubsystemRow>,
}

impl Subsystems {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn get(&self, id: EntityId) -> Option<SubsystemRow> {
        self.rows.get(id).copied()
    }

    pub(super) fn edit(&mut self, id: EntityId, f: impl FnOnce(&mut SubsystemRow)) {
        let mut row = self.get(id).unwrap_or_default();
        f(&mut row);
        if row == SubsystemRow::default() {
            self.rows.remove(id);
        } else {
            self.rows.insert(id, row);
        }
    }

    pub(crate) fn remove(&mut self, id: EntityId) {
        self.rows.remove(id);
    }

    #[cfg(test)]
    pub(crate) fn set_off_for_test(&mut self, id: EntityId) {
        self.edit(id, |r| r.off |= 0b100);
    }

    /// Mend the damage points (a bench `Heal`); the switches stay as set.
    pub(crate) fn repair(&mut self, id: EntityId) {
        self.edit(id, |r| r.damage = [0; 5]);
    }

    pub(crate) fn to_rows(&self) -> Vec<(EntityId, SubsystemRow)> {
        self.rows.iter().map(|(id, r)| (id, *r)).collect()
    }

    /// `None` for rows out of order, with unknown switch bits or negative damage.
    pub(crate) fn from_rows(rows: Vec<(EntityId, SubsystemRow)>) -> Option<Subsystems> {
        let valid = rows.iter().all(|(_, r)| r.off & !0b1_1100 == 0 && r.damage.iter().all(|d| *d >= 0) && *r != SubsystemRow::default());
        valid.then(|| Table::from_rows(rows)).flatten().map(|rows| Subsystems { rows })
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("subsystems-v1");
        h.u64(self.rows.len() as u64);
        for (id, r) in self.rows.iter() {
            h.u64(id.0 as u64);
            h.u64(u64::from(r.off) | u64::from(r.derelict) << 8);
            for d in r.damage {
                h.u64(d as u64);
            }
        }
    }
}

impl Sim {
    /// A live craft or station, which is what has shields and systems.
    fn live_ship(&self, id: EntityId) -> bool {
        self.units.get(id).is_some_and(|u| matches!(u.kind, EntityKind::Craft | EntityKind::Station) && u.hull[0] > Fixed::ZERO)
    }

    /// `SetShieldPercent`: every pool in use to `fraction` of the maximum.
    /// Rule read from the executable (static): current = max x fraction. For a
    /// directional unit every facing gets it (not measured).
    pub(crate) fn edit_shield_percent(&mut self, id: EntityId, fraction: Fixed) -> bool {
        if !self.live_ship(id) {
            return false;
        }
        let u = self.units.get_mut(id).expect("live ship");
        let level = u.shields.max * fraction.max(Fixed::ZERO).min(Fixed::ONE);
        let pools = if u.shields.directional { 4 } else { 1 };
        u.shields.now[..pools].fill(level);
        true
    }

    /// `SetMaxShields`: the maximum and every pool in use to `max` (read from
    /// the executable, static; negative reads as 0).
    pub(crate) fn edit_max_shields(&mut self, id: EntityId, max: Fixed) -> bool {
        if !self.live_ship(id) {
            return false;
        }
        let u = self.units.get_mut(id).expect("live ship");
        let max = max.max(Fixed::ZERO).min(Fixed::from_int(1_000_000_000));
        u.shields.max = max;
        let pools = if u.shields.directional { 4 } else { 1 };
        u.shields.now[..pools].fill(max);
        true
    }

    /// `Disable<System>` for the systems that are not unit flags.
    pub(crate) fn edit_subsystem_switch(&mut self, id: EntityId, sub: Subsystem, on: bool) -> bool {
        match sub {
            Subsystem::Engines => self.apply_edit(PlayerId::SCRIPT, &crate::command::Edit::Engines(id, on)),
            Subsystem::Weapons => self.apply_edit(PlayerId::SCRIPT, &crate::command::Edit::Weapons(id, on)),
            _ => {
                let n = system(sub);
                if !self.live_ship(id) {
                    return false;
                }
                self.subsystems.edit(id, |r| {
                    r.off = if on { r.off & !(1 << n) } else { r.off | (1 << n) };
                });
                true
            }
        }
    }

    /// `Damage<System>`: add `points` (negative repairs), clamped to 0..the
    /// system's hit points. A unit whose ODF has none for it rejects.
    pub(crate) fn edit_subsystem_damage(&mut self, id: EntityId, sub: Subsystem, points: Fixed) -> bool {
        let n = system(sub);
        if !self.live_ship(id) {
            return false;
        }
        let Some(hp) = self.content.as_deref().and_then(|db| db.unit(self.units.get(id)?.def)).map(|d| d.system_hit_points[n]) else {
            return false;
        };
        if hp <= Fixed::ZERO {
            return false;
        }
        self.subsystems.edit(id, |r| {
            let now = Fixed::from_bits(r.damage[n]);
            r.damage[n] = (now.to_bits().saturating_add(points.to_bits())).clamp(0, hp.to_bits());
        });
        true
    }

    /// Whether the system is switched on (by `Disable<System>`), and its damage
    /// points. Engines and weapons answer from the unit flags.
    pub fn subsystem_state(&self, id: EntityId, sub: Subsystem) -> Option<(bool, Fixed)> {
        let n = system(sub);
        let u = self.units.get(id)?;
        let row = self.subsystems.get(id).unwrap_or_default();
        let on = match sub {
            Subsystem::Engines => !u.flags.contains(UnitFlags::ENGINES_OFF),
            Subsystem::Weapons => !u.flags.contains(UnitFlags::WEAPONS_OFF),
            _ => row.off & (1 << n) == 0,
        };
        Some((on, Fixed::from_bits(row.damage[n])))
    }
}
