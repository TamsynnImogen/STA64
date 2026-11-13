//! Special abilities and special energy (SIM-14), with Cloak the first.
//!
//! What content gives us (typed `UnitDef`/`WeaponDef` fields, read from the
//! ODF keys named here by the content loader; the sim reads no raw ODF, D9;
//! gaps listed below):
//! - Unit: `maxSpecialEnergy` and `specialEnergyRate` (per second;
//!   `special_energy_max`, `special_energy_rate`). A unit
//!   with `maxSpecialEnergy > 0` has an energy pool, full at its first tick
//!   unless set by `Edit::SetEnergy`, refilled at the rate within 0..max.
//!   Mission/bench energy edits apply before that tick's refill/cloak drain.
//! - A special is a hardpoint whose weapon's class label the [`SpecialKind`]
//!   dispatch knows (`cloakingdevice`, `cloakingdeviceimp`). Special weapons
//!   never fire as weapons (`weapons.rs` skips them). An `Order::Special`
//!   names a special by `slot`, counting the unit's specials in hardpoint
//!   order; `Order::Cloak` engages or disengages the unit's cloak directly.
//! - Cloak weapon keys: `effectTimer` (seconds to engage, and to disengage;
//!   default 0), `energyDrainRate` (per second while engaging or cloaked),
//!   `specialEnergyCost` (paid when ordered; the order is rejected without
//!   it), `disableWeapons` (default 1: the unit cannot fire from engage to
//!   the end of disengage; with 0 it can, and firing decloaks it).
//!   `decloakOnHit` is ours, not in stock ODFs: 1 makes a hit decloak.
//!
//! Cloak states: engaging (still visible, timer), cloaked
//! (`UnitFlags::CLOAKED`, so `sensors.rs` hides it from every other team),
//! disengaging (visible again, weapons still off until the timer ends).
//! `SimEvent::CloakChanged { on: true }` comes when the unit becomes cloaked,
//! `{ on: false }` when it stops being so (the start of disengaging). Running
//! out of energy disengages. Cloaked units are not picked as targets by other
//! teams (`weapons.rs`).
//!
//! Weapons are blocked through `UnitFlags::WEAPONS_OFF`, which the bench's
//! `Weapons` edit also uses; this module clears it only if it set it.
//!
//! Not yet (no data in `UnitDef`): `initialCloak`, `decloakShieldDelay`,
//! `disableShield`, `unsafeCloakDistance`, `warpEnable`, detectors
//! (`cloakdetector`), the energy cost of other specials, effect names.
//!
//! State is in the digest and the snapshot only once any exists, so sims with
//! no special-capable unit keep their digests.

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, UnitDef, WeaponDef};
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, Tick};

use crate::event::SimEvent;
use crate::view::UnitFlags;
use crate::world::{Entities, Fnv, Table};

/// The special abilities the sim knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpecialKind {
    Cloak,
    /// A point-defence weapon, toggled like a special when
    /// `Features::pd_energy` is on (SIM-34; `combat/pd.rs` runs it).
    PointDefence,
}

impl SpecialKind {
    /// The ability a weapon definition stands for, if any.
    pub fn of(w: &WeaponDef) -> Option<SpecialKind> {
        match w.class.0.as_str() {
            "cloakingdevice" | "cloakingdeviceimp" => Some(SpecialKind::Cloak),
            _ if w.kind == sta_content::WeaponKind::PointDefence => Some(SpecialKind::PointDefence),
            _ => None,
        }
    }
}

/// Is this weapon a special rather than something that fires?
pub fn is_special(w: &WeaponDef) -> bool {
    SpecialKind::of(w).is_some()
}

/// A unit's special energy limits, from its ODF.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnergyDef {
    pub max: Fixed,
    /// Per tick.
    pub rate: Fixed,
}

pub fn energy_def(u: &UnitDef) -> Option<EnergyDef> {
    (u.special_energy_max > Fixed::ZERO).then(|| EnergyDef {
        max: u.special_energy_max,
        rate: per_tick(u.special_energy_rate),
    })
}

fn per_tick(per_second: Fixed) -> Fixed {
    per_second / Fixed::from_int(TICK_HZ as i32)
}

/// A unit's cloak, from its cloaking device's ODF.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CloakDef {
    /// Ticks to engage, and to disengage.
    pub ticks: Tick,
    /// Energy per tick while engaging or cloaked.
    pub drain: Fixed,
    pub cost: Fixed,
    pub disable_weapons: bool,
    pub decloak_on_hit: bool,
}

/// The unit's specials in hardpoint order.
pub fn specials_of<'a>(
    db: &'a ContentDb,
    u: &'a UnitDef,
) -> impl Iterator<Item = (SpecialKind, &'a WeaponDef)> {
    specials_indexed(db, u).map(|(_, k, w)| (k, w))
}

/// The unit's specials with their hardpoint index, in hardpoint order.
pub fn specials_indexed<'a>(
    db: &'a ContentDb,
    u: &'a UnitDef,
) -> impl Iterator<Item = (usize, SpecialKind, &'a WeaponDef)> {
    u.hardpoints.iter().enumerate().filter_map(|(i, h)| {
        let w = db.weapon(h.weapon.key)?;
        let kind = SpecialKind::of(w)?;
        // A PD weapon is a special (and takes a slot) only under `pd_energy` with
        // `point_defense_cycles`, the bits its toggle needs,
        // so slot numbers of other content do not shift.
        (kind != SpecialKind::PointDefence || { let f = db.weapon_features(w); f.pd_energy && f.point_defense_cycles }).then_some((i, kind, w))
    })
}

/// The `Order::Special` slot of the unit's point-defence weapon, when point
/// defence is a toggled special for it (`Features::pd_energy`). A HUD button
/// or a controller sends `Order::Special { slot, .. }` with it (SIM-37).
pub fn point_defence_slot(db: &ContentDb, u: &UnitDef) -> Option<u8> {
    specials_indexed(db, u).position(|(_, k, _)| k == SpecialKind::PointDefence).map(|n| n as u8)
}

pub fn cloak_def(db: &ContentDb, u: &UnitDef) -> Option<CloakDef> {
    let (_, w) = specials_of(db, u).find(|s| s.0 == SpecialKind::Cloak)?;
    Some(CloakDef {
        ticks: w.effect_ticks,
        drain: per_tick(w.energy_drain),
        cost: w.energy_cost,
        disable_weapons: w.disable_weapons,
        decloak_on_hit: w.decloak_on_hit,
    })
}

/// Where a unit is in its cloak. No row: not cloaked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloakState {
    /// Ticks left.
    Engaging(Tick),
    Cloaked,
    /// Ticks left.
    Disengaging(Tick),
}

/// Specials state of the whole sim.
#[derive(Debug, Clone, Default)]
pub struct Specials {
    /// Special energy of units that have a pool.
    energy: Table<Fixed>,
    cloak: Table<CloakState>,
    /// Units whose `WEAPONS_OFF` this module set.
    muted: Table<()>,
}

/// [`Specials`] as the snapshot stores it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpecialsRec {
    pub energy: Vec<(EntityId, Fixed)>,
    pub cloak: Vec<(EntityId, CloakState)>,
    pub muted: Vec<EntityId>,
}

impl Specials {
    pub fn is_empty(&self) -> bool {
        self.energy.is_empty() && self.cloak.is_empty() && self.muted.is_empty()
    }

    pub fn energy_of(&self, id: EntityId) -> Option<Fixed> {
        self.energy.get(id).copied()
    }

    pub(crate) fn set_energy(&mut self, units: &Entities, db: &ContentDb, id: EntityId, to: Fixed) -> bool {
        let Some(def) = units
            .get(id)
            .filter(|unit| unit.hull[0] > Fixed::ZERO)
            .and_then(|unit| db.unit(unit.def))
            .and_then(energy_def)
        else {
            return false;
        };
        self.energy.insert(id, to.max(Fixed::ZERO).min(def.max));
        true
    }

    /// The unit's special-energy pool, for the point-defence drain.
    pub(crate) fn energy_mut(&mut self, id: EntityId) -> Option<&mut Fixed> {
        self.energy.get_mut(id)
    }

    /// Make the unit's pool now if it has one and it has not been made yet
    /// (pools start full at the first step; an order may come first).
    pub(crate) fn ensure_pool(&mut self, id: EntityId, def: &UnitDef) {
        if self.energy.get(id).is_none()
            && let Some(e) = energy_def(def)
        {
            self.energy.insert(id, e.max);
        }
    }

    pub fn cloak_state(&self, id: EntityId) -> Option<CloakState> {
        self.cloak.get(id).copied()
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.u64(self.energy.len() as u64);
        for (id, e) in self.energy.iter() {
            h.u64(id.0 as u64);
            h.fixed(*e);
        }
        h.u64(self.cloak.len() as u64);
        for (id, c) in self.cloak.iter() {
            h.u64(id.0 as u64);
            let (tag, n) = match *c {
                CloakState::Engaging(n) => (1, n),
                CloakState::Cloaked => (2, 0),
                CloakState::Disengaging(n) => (3, n),
            };
            h.u64(tag);
            h.u64(n);
        }
        h.u64(self.muted.len() as u64);
        for (id, _) in self.muted.iter() {
            h.u64(id.0 as u64);
        }
    }

    pub(crate) fn to_rec(&self) -> SpecialsRec {
        SpecialsRec {
            energy: self.energy.iter().map(|(i, e)| (i, *e)).collect(),
            cloak: self.cloak.iter().map(|(i, c)| (i, *c)).collect(),
            muted: self.muted.iter().map(|(i, _)| i).collect(),
        }
    }

    /// `None` if rows are not ascending by id.
    pub(crate) fn from_rec(r: SpecialsRec) -> Option<Specials> {
        Some(Specials {
            energy: Table::from_rows(r.energy)?,
            cloak: Table::from_rows(r.cloak)?,
            muted: Table::from_rows(r.muted.into_iter().map(|i| (i, ())).collect())?,
        })
    }

    /// An `Order::Cloak`: engage or disengage the listed units of `player`
    /// (any, for scripts and the bench). True when at least one took it.
    pub(crate) fn order_cloak(
        &mut self,
        db: &ContentDb,
        units: &Entities,
        player: PlayerId,
        ids: &[EntityId],
        on: bool,
    ) -> bool {
        let mut any = false;
        for &id in ids {
            any |= self.cloak_one(db, units, player, id, Some(on));
        }
        any
    }

    /// An `Order::Special`: `slot` counts the unit's specials in hardpoint order.
    pub(crate) fn order_special(
        &mut self,
        db: &ContentDb,
        units: &Entities,
        player: PlayerId,
        ids: &[EntityId],
        slot: u8,
    ) -> bool {
        let mut any = false;
        for &id in ids {
            let kind = units
                .get(id)
                .and_then(|u| db.unit(u.def))
                .and_then(|d| specials_of(db, d).nth(slot as usize))
                .map(|s| s.0);
            any |= match kind {
                Some(SpecialKind::Cloak) => self.cloak_one(db, units, player, id, None),
                // Toggled by `Weapons::order_special`, which routes it.
                Some(SpecialKind::PointDefence) | None => false,
            };
        }
        any
    }

    /// `on`: `None` toggles.
    fn cloak_one(
        &mut self,
        db: &ContentDb,
        units: &Entities,
        player: PlayerId,
        id: EntityId,
        on: Option<bool>,
    ) -> bool {
        let Some(u) = units
            .get(id)
            .filter(|u| (!player.is_player() || u.owner == player) && u.hull[0] > Fixed::ZERO)
        else {
            return false;
        };
        let Some(def) = db.unit(u.def).and_then(|d| cloak_def(db, d)) else {
            return false;
        };
        if self.energy.get(id).is_none() {
            // Pools start full; the first step would have made it.
            if let Some(e) = db.unit(u.def).and_then(energy_def) {
                self.energy.insert(id, e.max);
            }
        }
        let state = self.cloak.get(id).copied();
        let engage = on.unwrap_or(matches!(state, None | Some(CloakState::Disengaging(_))));
        if engage {
            if state.is_some() {
                return false;
            }
            match self.energy.get_mut(id) {
                Some(e) if *e >= def.cost => *e = *e - def.cost,
                Some(_) => return false,
                None if def.cost > Fixed::ZERO => return false,
                None => {}
            }
            self.cloak.insert(id, CloakState::Engaging(def.ticks));
            if def.disable_weapons && !u.flags.contains(UnitFlags::WEAPONS_OFF) {
                self.muted.insert(id, ());
            }
            true
        } else {
            match state {
                Some(CloakState::Engaging(_)) => {
                    // Never became visible to the sensors as cloaked: just stop.
                    self.cloak.remove(id);
                    true
                }
                Some(CloakState::Cloaked) => {
                    // The flag drops and the event comes with the next step.
                    self.cloak.insert(id, CloakState::Disengaging(def.ticks));
                    true
                }
                _ => false,
            }
        }
    }
}

impl Specials {
    /// One tick, before weapons: energy, cloak timers, drain, and the flags
    /// the other systems read. An order made this tick takes effect here.
    pub(crate) fn step(
        &mut self,
        units: &mut Entities,
        db: &ContentDb,
        events: &mut Vec<SimEvent>,
    ) {
        let gone: Vec<EntityId> = self
            .energy
            .iter()
            .map(|r| r.0)
            .chain(self.cloak.iter().map(|r| r.0))
            .chain(self.muted.iter().map(|r| r.0))
            .filter(|id| !units.contains(*id))
            .collect();
        for id in gone {
            self.energy.remove(id);
            self.cloak.remove(id);
            self.muted.remove(id);
        }
        // Pools start full and refill.
        let ids: Vec<EntityId> = units.iter().map(|u| u.id).collect();
        for &id in &ids {
            let Some(e) = units
                .get(id)
                .and_then(|u| db.unit(u.def))
                .and_then(energy_def)
            else {
                continue;
            };
            match self.energy.get_mut(id) {
                Some(v) => {
                    // Widen before adding: Fixed addition wraps before a cap
                    // can apply, including when a mission sets a full pool.
                    let next = i128::from(v.to_bits()) + i128::from(e.rate.to_bits());
                    *v = Fixed::from_bits(next.clamp(0, i128::from(e.max.to_bits())) as i64);
                }
                None => {
                    self.energy.insert(id, e.max);
                }
            }
        }
        let cloaking: Vec<(EntityId, CloakState)> =
            self.cloak.iter().map(|(i, c)| (i, *c)).collect();
        for (id, mut state) in cloaking {
            let Some(def) = units
                .get(id)
                .and_then(|u| db.unit(u.def))
                .and_then(|d| cloak_def(db, d))
            else {
                self.cloak.remove(id);
                continue;
            };
            let draining = matches!(state, CloakState::Engaging(_) | CloakState::Cloaked)
                && def.drain > Fixed::ZERO;
            if draining && let Some(e) = self.energy.get_mut(id) {
                *e = *e - def.drain;
                if *e <= Fixed::ZERO {
                    *e = Fixed::ZERO;
                    state = CloakState::Disengaging(def.ticks);
                }
            }
            match state {
                CloakState::Engaging(n) => {
                    if n <= 1 {
                        self.cloak.insert(id, CloakState::Cloaked);
                        set_flag(units, id, UnitFlags::CLOAKED, true);
                        events.push(SimEvent::CloakChanged { id, on: true });
                    } else {
                        self.cloak.insert(id, CloakState::Engaging(n - 1));
                    }
                }
                CloakState::Cloaked => {
                    self.cloak.insert(id, state);
                }
                CloakState::Disengaging(n) => {
                    self.settle_visible(units, id, events);
                    if n <= 1 {
                        self.cloak.remove(id);
                    } else {
                        self.cloak.insert(id, CloakState::Disengaging(n - 1));
                    }
                }
            }
        }
        // Weapons stay off for as long as a unit that must not fire has a cloak state.
        let muted: Vec<EntityId> = self.muted.iter().map(|r| r.0).collect();
        for id in muted {
            let on = self.cloak.get(id).is_some();
            set_flag(units, id, UnitFlags::WEAPONS_OFF, on);
            if !on {
                self.muted.remove(id);
            }
        }
    }

    /// A disengaging unit shows again: drop the flag and say so, once.
    fn settle_visible(&mut self, units: &mut Entities, id: EntityId, events: &mut Vec<SimEvent>) {
        if units
            .get(id)
            .is_some_and(|u| u.flags.contains(UnitFlags::CLOAKED))
        {
            set_flag(units, id, UnitFlags::CLOAKED, false);
            events.push(SimEvent::CloakChanged { id, on: false });
        }
    }

    /// After combat: units that fired from a cloak that allows it, or were hit
    /// while cloaked and decloak on hit, drop the cloak. Looks at `events[from..]`.
    pub(crate) fn react(
        &mut self,
        units: &mut Entities,
        db: &ContentDb,
        events: &mut Vec<SimEvent>,
        from: usize,
    ) {
        if self.cloak.is_empty() {
            return;
        }
        let mut drop: Vec<EntityId> = Vec::new();
        for ev in &events[from..] {
            match *ev {
                SimEvent::Fired { shooter, .. } => drop.push(shooter),
                SimEvent::Hit { target, .. } => {
                    let hit_decloaks = units
                        .get(target)
                        .and_then(|u| db.unit(u.def))
                        .and_then(|d| cloak_def(db, d))
                        .is_some_and(|d| d.decloak_on_hit);
                    if hit_decloaks {
                        drop.push(target);
                    }
                }
                _ => {}
            }
        }
        drop.sort();
        drop.dedup();
        for id in drop {
            let Some(def) = units
                .get(id)
                .and_then(|u| db.unit(u.def))
                .and_then(|d| cloak_def(db, d))
            else {
                continue;
            };
            if matches!(
                self.cloak.get(id),
                Some(CloakState::Engaging(_) | CloakState::Cloaked)
            ) {
                self.cloak.insert(id, CloakState::Disengaging(def.ticks));
                self.settle_visible(units, id, events);
            }
        }
    }
}

fn set_flag(units: &mut Entities, id: EntityId, f: UnitFlags, on: bool) {
    if let Some(u) = units.get_mut(id) {
        u.flags = u.flags.with(f, on);
    }
}

pub mod fixture;
#[cfg(test)]
mod tests;
