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
//! - [`recharge`] gives each unheld pool its definition's `recharge` per tick
//!   (capped at `max`), before the tick's hits. Stock A2 holds an emptied pool
//!   for `shieldDelay` after the last hit while empty (measured).
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
//! Switched-off shield generators drop their pools and prevent recharge (UI-47; static rule,
//! timing not measured). Armada I's hull recharge also reads generator damage (SIM-68).
//! Other subsystem combat consequences remain incomplete; recording a switch alone does not
//! establish parity for destroyed generators, life support or sensors.
//! Systems damage from hits (weapons or engines offline below a hull
//! threshold) is not modelled either.

use serde::{Deserialize, Serialize};
use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId, Tick};

use crate::command::Subsystem;
use crate::event::SimEvent;
use crate::view::UnitFlags;
use crate::world::{Entities, EntityKind, Fnv, Table};
use crate::Sim;

/// Empty shield pools held until an absolute tick (exclusive), by unit and facing.
/// A zero entry has no hold; inactive rows are removed before hashing or saving.
#[derive(Debug, Clone, Default)]
pub(crate) struct ZeroShieldHolds {
    rows: Table<[Tick; 4]>,
}

impl ZeroShieldHolds {
    pub(crate) fn is_empty(&self) -> bool { self.rows.is_empty() }
    pub(crate) fn remove(&mut self, id: EntityId) { self.rows.remove(id); }
    pub(crate) fn until(&self, id: EntityId, pool: usize) -> Tick {
        self.rows.get(id).map_or(0, |r| r[pool])
    }
    pub(crate) fn hold(&mut self, id: EntityId, pool: usize, until: Tick) {
        let mut row = self.rows.get(id).copied().unwrap_or([0; 4]);
        row[pool] = row[pool].max(until);
        self.rows.insert(id, row);
    }
    pub(crate) fn clear_pool(&mut self, id: EntityId, pool: usize) {
        let Some(mut row) = self.rows.get(id).copied() else { return };
        row[pool] = 0;
        if row == [0; 4] { self.rows.remove(id); } else { self.rows.insert(id, row); }
    }
    pub(crate) fn to_rows(&self) -> Vec<(EntityId, [Tick; 4])> {
        self.rows.iter().map(|(id, r)| (id, *r)).collect()
    }
    pub(crate) fn from_rows(rows: Vec<(EntityId, [Tick; 4])>, tick: Tick, units: &Entities, db: Option<&ContentDb>) -> Option<Self> {
        let valid = rows.iter().all(|(id, row)| {
            let Some(u) = units.get(*id) else { return false };
            let Some(def) = db.and_then(|db| db.unit(u.def)) else { return false };
            let enabled = db.is_some_and(|db| db.unit_features(def).shield_zero_hold);
            let pools = if u.shields.directional { 4 } else { 1 };
            // The step increments its tick after damage. A one-tick hold can
            // therefore have `until == tick` in a valid end-of-step snapshot.
            enabled && u.kind == EntityKind::Craft && row[..pools].iter().any(|&t| t >= tick && t != 0)
                && row[..pools].iter().all(|&t| t == 0 || t >= tick)
                && row[pools..].iter().all(|&t| t == 0)
        });
        valid.then(|| Table::from_rows(rows)).flatten().map(|rows| Self { rows })
    }
    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("shield-zero-holds-v1");
        h.u64(self.rows.len() as u64);
        for (id, row) in self.rows.iter() {
            h.u64(id.0 as u64);
            for until in row { h.u64(*until); }
        }
    }
}

/// Recharge every unit's shields by its definition's rate.
#[allow(clippy::too_many_arguments)] // separate system state borrowed by the tick
pub(crate) fn recharge(units: &mut Entities, db: &ContentDb, switches: &crate::switches::SwitchTable, subsystems: &Subsystems, transports: &super::boarding::Transports, holds: &mut ZeroShieldHolds, tick: Tick, specials: &crate::specials::Specials) {
    for u in units.iter_mut() {
        let pools = if u.shields.directional { 4 } else { 1 };
        let mut held = [false; 4];
        #[allow(clippy::needless_range_loop)] // `pool` addresses the parallel per-pool arrays
        for pool in 0..pools {
            let until = holds.until(u.id, pool);
            if until > tick && u.shields.now[pool] == Fixed::ZERO {
                held[pool] = true;
            } else if until != 0 {
                holds.clear_pool(u.id, pool);
            }
        }
        if specials.stasis(u.id, tick) || specials.shields_held(u.id, tick) {
            continue;
        }
        // Hold disabled shields at zero even after loading an older snapshot which stored the
        // off bit alongside charged pools, or a script tried to refill a disabled generator.
        if subsystems.get(u.id).is_some_and(|r| r.off & (1 << system(Subsystem::Shields)) != 0) {
            u.shields.now.fill(Fixed::ZERO);
            continue;
        }
        // `CraftCanRepair(false)` (SIM-58) stops the shields coming back too.
        if u.hull[0] <= Fixed::ZERO || switches.no_repair(u.id) || subsystems.get(u.id).is_some_and(|r| r.derelict) || transports.shields_held(u.id) {
            continue;
        }
        let Some(def) = db.unit(u.def) else { continue };
        let (rate, max) = (def.shields.recharge, u.shields.max);
        if rate <= Fixed::ZERO {
            continue;
        }
        // SIM-68, Armada I (`a1_compat`): the only pool is `maxHealth`, held in the hull. It
        // regenerates at `shieldRate` per second with no delay above zero (measured on the
        // original), only with the shield generator switched on and not destroyed (read from the
        // executable, static). Armada II, FO and Native hulls never regenerate.
        if db.unit_features(def).a1_compat {
            let row = subsystems.get(u.id).unwrap_or_default();
            let gen_hp = def.system_hit_points[system(Subsystem::Shields)];
            let generator_dead = gen_hp > Fixed::ZERO && row.damage[system(Subsystem::Shields)] >= gen_hp.to_bits();
            if row.off & (1 << system(Subsystem::Shields)) == 0 && !generator_dead && u.hull[0] < u.hull[1] {
                // Guarded so a very large maximum cannot wrap.
                u.hull[0] = if u.hull[1] - u.hull[0] <= rate { u.hull[1] } else { u.hull[0] + rate };
            }
        }
        #[allow(clippy::needless_range_loop)] // `pool` addresses the parallel per-pool arrays
        for pool in 0..pools {
            let now = &mut u.shields.now[pool];
            if !held[pool] && *now < max {
                *now = (*now + rate).min(max);
            }
        }
    }
}

/// Apply the `Hit`s in `events[from..]`; returns `(killed, killer)` in the
/// order the lethal hits came.
#[allow(clippy::too_many_arguments)] // separate system state borrowed by the tick
pub(crate) fn apply(
    units: &mut Entities,
    db: &ContentDb,
    events: &mut Vec<SimEvent>,
    from: usize,
    ctx: &mut super::casualty::Ctx,
    holds: &mut ZeroShieldHolds,
    tick: Tick,
    specials: &crate::specials::Specials,
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
        if let Some(t) = units.get_mut(target).filter(|t| {
            t.hull[0] > Fixed::ZERO
                && (damage > Fixed::ZERO || db.weapon(weapon).is_none_or(|w| !crate::specials::is_special(w)))
                && !specials.stasis(target, tick)
                && !specials.a2_invulnerable(target, tick) && !specials.phased(db, t)
        }) {
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
                    let bypass = specials.shields_held(target, tick);
                    let saved = t.shields.now[pool];
                    if bypass { t.shields.now[pool] = Fixed::ZERO; }
                    let (s, h, d) = super::casualty::hit(t, def, w, db, ctx, pool, damage);
                    if bypass { t.shields.now[pool] = saved; }
                    (shield, down) = (s, d);
                    h
                }
                None => {
                    if !specials.shields_held(target, tick) {
                        let held = t.shields.now[pool];
                        shield = damage.min(held);
                        t.shields.now[pool] = held - shield;
                        down = shield > Fixed::ZERO && t.shields.now[pool] == Fixed::ZERO;
                    }
                    damage - shield
                }
            };
            #[allow(clippy::collapsible_if)] // the inner `if let` stays separate
            if damage > Fixed::ZERO && t.kind == EntityKind::Craft && t.shields.max > Fixed::ZERO && t.shields.now[pool] == Fixed::ZERO {
                if let Some(def) = db.unit(t.def).filter(|def| db.unit_features(def).shield_zero_hold) {
                    // The emptying hit and every later hit at zero restart the timer.
                    let delay = def.transport.shield_delay.map_or(
                        super::boarding::TransportRules::of(db).target_shield_hold,
                        super::boarding::ticks,
                    );
                    let until = tick.saturating_add(delay);
                    if until > tick { holds.hold(target, pool, until); }
                }
            }
            if ctx.colonies.has_row(target) {
                // SIM-30, a planet with a population row: damage that reaches the
                // hull costs `0.1 x damage` people and the hull stays, so every hit
                // of the tick counts in full (read from the executable, static).
                // The event reports what the hull took: nothing; the people lost are the row's.
                ctx.colonies.lose_population(target, wanted.max(Fixed::ZERO));
                hull = Fixed::ZERO;
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

/// One unit's systems as the interface shows them, in [`UnitDef::system_hit_points`] order
/// (engines, weapons, shield generator, life support, sensors): the share of hit points left
/// (1 for a system without hit points) and whether it is switched on.
///
/// [`UnitDef::system_hit_points`]: sta_content::UnitDef::system_hit_points
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemView {
    pub entity: EntityId,
    pub health: [Fixed; 5],
    pub on: [bool; 5],
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
                if sub == Subsystem::Shields && !on {
                    self.units.get_mut(id).expect("live ship").shields.now.fill(Fixed::ZERO);
                }
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

    /// SIM-40: a yard takes `healed` damage points off system `n`. A system that was
    /// dead and lives again is switched back on, as the original re-enables a repaired
    /// system (read from the executable, static); a derelict unit stays off.
    pub(crate) fn yard_repair_system(&mut self, id: EntityId, n: usize, healed: Fixed) {
        let Some(hp) = self.content.as_deref().and_then(|db| db.unit(self.units.get(id)?.def)).map(|d| d.system_hit_points[n]) else {
            return;
        };
        let before = self.subsystems.get(id).unwrap_or_default();
        let dead_before = hp > Fixed::ZERO && before.damage[n] >= hp.to_bits();
        self.subsystems.edit(id, |r| r.damage[n] = r.damage[n].saturating_sub(healed.to_bits().max(0)).max(0));
        let after = self.subsystems.get(id).unwrap_or_default();
        if !dead_before || after.damage[n] >= hp.to_bits() || after.derelict {
            return;
        }
        match n {
            0 | 1 => {
                if let Some(u) = self.units.get_mut(id) {
                    let flag = if n == 0 { UnitFlags::ENGINES_OFF } else { UnitFlags::WEAPONS_OFF };
                    u.flags = u.flags.with(flag, false);
                }
            }
            _ => self.subsystems.edit(id, |r| r.off &= !(1 << n)),
        }
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
        let held = match sub {
            Subsystem::Engines => self.specials.engines_held(id, self.tick),
            Subsystem::Shields => self.specials.shields_held(id, self.tick),
            _ => self.specials.a2_held(id, sub, self.tick),
        };
        Some((on && !held && !self.specials.stasis(id, self.tick), Fixed::from_bits(row.damage[n])))
    }

    /// Read-only state of the five systems of every unit with damaged or switched-off systems
    /// (UI-48: the info panel's system icons). Units not listed have every system whole and on.
    pub fn system_view(&self) -> Vec<SystemView> {
        let Some(db) = self.content.as_deref() else { return Vec::new() };
        let mut ids: Vec<EntityId> = self.subsystems.to_rows().into_iter().map(|(id, _)| id).collect();
        ids.extend(self.units.iter().filter(|u| u.flags.contains(UnitFlags::ENGINES_OFF) || u.flags.contains(UnitFlags::WEAPONS_OFF)).map(|u| u.id));
        ids.extend(self.specials.a1.disabled_units());
        ids.extend(self.specials.a2_disabled_units());
        ids.sort();
        ids.dedup();
        ids.into_iter()
            .filter_map(|id| {
                let def = db.unit(self.units.get(id)?.def)?;
                let mut out = SystemView { entity: id, health: [Fixed::ONE; 5], on: [true; 5] };
                for (n, sub) in [Subsystem::Engines, Subsystem::Weapons, Subsystem::Shields, Subsystem::LifeSupport, Subsystem::Sensors].into_iter().enumerate() {
                    let (on, damage) = self.subsystem_state(id, sub)?;
                    let hp = def.system_hit_points[n];
                    out.on[n] = on;
                    if hp > Fixed::ZERO {
                        out.health[n] = ((hp - damage) / hp).max(Fixed::ZERO).min(Fixed::ONE);
                    }
                }
                Some(out)
            })
            .collect()
    }

    /// The derelict latch of a live unit (SIM-39): `None` for a unit that is not in the world.
    pub fn is_derelict(&self, id: EntityId) -> Option<bool> {
        self.units.get(id).map(|_| self.subsystems.get(id).is_some_and(|r| r.derelict))
    }
}
