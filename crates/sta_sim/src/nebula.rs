//! Nebula effects (SIM-75): what a nebula does to the craft inside it.
//!
//! Every rule here is read from the executable, static, and the same for
//! every profile, unless marked otherwise. The keys are the nebula definition's
//! ([`NebulaEffects`]), so a mod declares its own kinds of nebula (ADR 0004).
//!
//! **Who is inside.** The map is cut into square cells of [`CELL`] units in
//! the XZ plane, counted from the map's minimum corner. A craft is in a nebula
//! when the centre of its cell, at height 0, lies within the nebula's
//! `effectRadius` of the nebula's position (its height counts, the craft's
//! does not). A craft off the map is in none. Membership is taken once per
//! tick from the positions at the start of the tick.
//!
//! **Several nebulae.** Each kind (definition) that covers a craft acts once,
//! however many of its nebulae overlap there; different kinds all act, in
//! ascending content-key order.
//!
//! **Effects, per tick, scaled by the tick's length.** Live craft and
//! stations:
//! - `damageValue`: a hit of `damageValue x dt` on the first shield pool. Under
//!   the casualty model (`crew_loss_on_hit`) it is the weapon hit path with the
//!   nebula's `damageThreshold`, `damageCrewModifier` and `damageHullModifier`
//!   as the crew law; otherwise shields, then hull. **Interim:** the nebula's
//!   `damageVariance` term is left out (its random range is not established).
//! - `regenValue`: shield points per second, up to the maximum, not on a
//!   derelict.
//! - `repairValue`: system hit points per second on every damaged system, not
//!   on a derelict. **Interim:** a destroyed system is not repaired.
//! - `disableShields`: the pools are held at 0. `disableWeapons`: the craft
//!   holds fire. `disableCrew`: life support is off (the casualty model's
//!   drain). `disableCloak`: an engaging or engaged cloak drops.
//!   **Interim:** in the original the systems stay off a moment after the
//!   craft leaves (read from the executable, static; not measured; not
//!   implemented, it needs a rule the notes do not give); here they are on
//!   again the tick the craft is out. The shields come back at the strength
//!   they had on entry, after a delay, in the Armada II family ([`shields`]);
//!   Armada I recharges from 0. Which unit types besides
//!   freighters the weapons rule spares is not established; none is spared.
//! - Speed (craft only): a throttle fraction that starts at 1 and a cap that
//!   starts at the larger of impulse and warp speed. Each kind lowers the
//!   fraction to `craft_max_speed_fraction` and the cap to that fraction of
//!   impulse speed, for a kind with the key above 0 (measured on stock
//!   Armada II, single runs: impulse speed is that fraction of its value).
//!   Armada II family (every profile but Armada I): a kind without the
//!   fraction does not change speed, whatever `craft_speed_coefficient` and
//!   `craft_speed_limit` say. Grade: measured on stock Armada II 1.1, single
//!   runs (a coefficient far below the measured ones is not). Fleet
//!   Operations 3 and 4 and the Roots lineage take it by engine family (not
//!   measured); the Native profile follows it by STA64's own choice. **Interim, not measured:** warp speed under a fraction is
//!   scaled by it and bounded by the cap. **Interim, not measured:** Armada I
//!   (`Sta1Classic`) keeps the older coefficient rule: a kind without the
//!   fraction lowers the fraction to `craft_speed_coefficient` (0.5 when the
//!   key is absent) and the cap to `craft_speed_limit` when that is 0 or more.
//!   **Interim:** how the original's steering applies these is not
//!   established; here impulse and warp speed are both scaled by the fraction
//!   and bounded by the cap.
//!
//! Shields after leaving, Armada II family: see [`shields`] (snapshot format 39).
//!
//! Not yet: `disableSensors`, `hideContents`, `clearEffects`, the entry
//! voice, the AI's handling (see TODO.md). A map without nebula effects takes
//! the early return: no draws, no change to its digests.

use sta_content::{ContentDb, ContentKey, NebulaEffects, Profile, RegionKind, RegionShape};
use sta_core::{EntityId, Fixed, TICK_HZ, Vec3};

use crate::view::UnitFlags;
use crate::world::{Entity, EntityKind};
use crate::Sim;

mod shields;
pub(crate) use shields::{NebulaShields, NebulaShieldsRec};

/// Width of a containment cell, world units (read from the executable, static).
pub const CELL: i32 = 50;

/// One nebula of the map that has effects.
struct Field {
    key: ContentKey,
    centre: Vec3,
    radius: Fixed,
}

/// The kinds of nebula each craft is in this tick: ascending ids, each with
/// its kinds' keys ascending and unique. Craft in none have no row.
#[derive(Debug, Default)]
pub(crate) struct Inside {
    rows: Vec<(EntityId, Vec<ContentKey>)>,
}

impl Inside {
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Whether the cell of `p` is covered by a nebula at `centre` of `radius`.
/// `origin` is the map's minimum corner, `end` its maximum (none: unbounded).
pub(crate) fn covers(p: Vec3, origin: Vec3, end: Option<Vec3>, centre: Vec3, radius: Fixed) -> bool {
    let cell = i64::from(CELL) << 32;
    let mid = |v: Fixed, lo: Fixed, hi: Option<Fixed>| -> Option<Fixed> {
        let off = (v - lo).to_bits();
        if hi.is_some() && off < 0 {
            return None;
        }
        let i = off.div_euclid(cell);
        if let Some(hi) = hi {
            // Cells start at the corner; the last may stick out past the far edge.
            let span = (hi - lo).to_bits();
            if span <= 0 || i * cell >= span {
                return None;
            }
        }
        Some(lo + Fixed::from_bits(i * cell + cell / 2))
    };
    let (Some(cx), Some(cz)) = (mid(p.x, origin.x, end.map(|e| e.x)), mid(p.z, origin.z, end.map(|e| e.z))) else {
        return false;
    };
    let (dx, dy, dz) = (cx - centre.x, centre.y, cz - centre.z);
    if dx.abs() > radius || dy.abs() > radius || dz.abs() > radius {
        return false;
    }
    dx * dx + dy * dy + dz * dz <= radius * radius
}

/// Tick length in seconds.
fn dt() -> Fixed {
    Fixed::ONE / Fixed::from_int(TICK_HZ as i32)
}

/// Speed caps of a craft: `(impulse, warp)` under the kinds `keys`, or `None`
/// when they leave it as it is.
fn speed_caps(db: &ContentDb, u: &Entity, keys: &[ContentKey]) -> Option<(Fixed, Fixed)> {
    let (impulse, warp) = (u.stats.max_speed, u.stats.warp_speed);
    let mut fraction = Fixed::ONE;
    let mut cap = impulse.max(warp);
    // Interim, not measured: Armada I keeps the coefficient rule; the Armada
    // II family ignores the coefficient and the limit (measured on stock
    // Armada II 1.1 only; the other family profiles by engine family, Native by choice).
    let coefficient_rule = db.profile() == Profile::Sta1Classic;
    for e in keys.iter().filter_map(|k| effects(db, *k)) {
        match e.max_speed_fraction {
            Some(f) => {
                cap = cap.min(impulse * f);
                fraction = fraction.min(f);
            }
            None if coefficient_rule => {
                fraction = fraction.min(e.speed_coefficient);
                if let Some(limit) = e.speed_limit {
                    cap = cap.min(limit);
                }
            }
            None => {}
        }
    }
    let floor = crate::speed_override::HELD_SPEED;
    let new_impulse = (impulse * fraction).min(cap).max(floor);
    let new_warp = (warp * fraction).min(cap).max(Fixed::ZERO);
    (new_impulse < impulse || new_warp < warp).then_some((new_impulse.min(impulse), new_warp.min(warp)))
}

fn effects(db: &ContentDb, key: ContentKey) -> Option<&NebulaEffects> {
    db.unit(key)?.nebula_effects.as_ref()
}

/// Craft and stations alive: the units a nebula acts on.
fn affected(u: &Entity) -> bool {
    matches!(u.kind, EntityKind::Craft | EntityKind::Station) && u.hull[0] > Fixed::ZERO
}

/// What the tick's mid-step effects leave for later steps.
#[derive(Debug, Default)]
pub(crate) struct Holds {
    /// Craft that hold fire this tick, ascending.
    pub held: Vec<EntityId>,
    /// Units whose life support is off this tick, ascending.
    pub life_support_off: Vec<EntityId>,
}

impl Sim {
    /// Who is in which kind of nebula, from the current positions.
    pub(crate) fn nebula_contents(&self) -> Inside {
        let Some(db) = self.content.as_deref() else { return Inside::default() };
        let fields: Vec<Field> = self
            .regions
            .iter()
            .filter(|r| r.kind == RegionKind::Nebula)
            .filter_map(|r| {
                let RegionShape::Sphere { centre, radius } = r.shape else { return None };
                let key = db.key(&r.odf)?;
                effects(db, key)?;
                Some(Field { key, centre, radius })
            })
            .collect();
        if fields.is_empty() {
            return Inside::default();
        }
        let (origin, end) = match self.bounds {
            Some((lo, hi)) => (lo, Some(hi)),
            None => (Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO }, None),
        };
        let mut rows = Vec::new();
        for u in self.units.iter().filter(|u| affected(u)) {
            let mut keys: Vec<ContentKey> = fields.iter().filter(|f| covers(u.pos, origin, end, f.centre, f.radius)).map(|f| f.key).collect();
            if keys.is_empty() {
                continue;
            }
            keys.sort();
            keys.dedup();
            rows.push((u.id, keys));
        }
        Inside { rows }
    }

    /// `(craft, impulse cap, warp cap)` ascending, for the movement step.
    pub(crate) fn nebula_speed_caps(&self, inside: &Inside) -> Vec<(EntityId, (Fixed, Fixed))> {
        let Some(db) = self.content.as_deref() else { return Vec::new() };
        inside
            .rows
            .iter()
            .filter_map(|(id, keys)| {
                let u = self.units.get(*id).filter(|u| u.kind == EntityKind::Craft)?;
                speed_caps(db, u, keys).map(|c| (*id, c))
            })
            .collect()
    }

    /// After shield recharge, before the weapons: regeneration, repair, held
    /// shields and dropped cloaks; returns the holds for the later steps.
    pub(crate) fn nebula_systems(&mut self, db: &ContentDb, inside: &Inside) -> Holds {
        let mut holds = Holds::default();
        let mut decloak = Vec::new();
        for (id, keys) in &inside.rows {
            let id = *id;
            let Some(def) = self.units.get(id).filter(|u| affected(u)).and_then(|u| db.unit(u.def)) else { continue };
            let derelict = self.subsystems.get(id).is_some_and(|r| r.derelict);
            let mut shields_off = false;
            for e in keys.iter().filter_map(|k| effects(db, *k)) {
                if e.regen > Fixed::ZERO && !derelict
                    && let Some(u) = self.units.get_mut(id)
                {
                    let (gain, max) = (e.regen * dt(), u.shields.max);
                    let pools = if u.shields.directional { 4 } else { 1 };
                    for now in &mut u.shields.now[..pools] {
                        if *now < max {
                            *now = (*now + gain).min(max);
                        }
                    }
                }
                if e.repair > Fixed::ZERO && !derelict {
                    let row = self.subsystems.get(id).unwrap_or_default();
                    for n in 0..5 {
                        let hp = def.system_hit_points[n].to_bits();
                        if row.damage[n] > 0 && row.damage[n] < hp {
                            self.yard_repair_system(id, n, e.repair * dt());
                        }
                    }
                }
                shields_off |= e.disable_shields;
                if e.disable_weapons {
                    holds.held.push(id);
                }
                if e.disable_crew {
                    holds.life_support_off.push(id);
                }
                if e.disable_cloak {
                    decloak.push(id);
                }
            }
            if shields_off && let Some(u) = self.units.get_mut(id) {
                u.shields.now = [Fixed::ZERO; 4];
            }
        }
        holds.held.dedup();
        holds.life_support_off.dedup();
        self.specials.force_decloak(&mut self.units, db, &mut self.events, decloak);
        holds
    }

    /// With the tick's hits: the damage of damaging nebulae. Returns the units
    /// it killed, in id order (no killer).
    pub(crate) fn nebula_damage(&mut self, db: &ContentDb, inside: &Inside) -> Vec<(EntityId, Option<EntityId>)> {
        let mut killed = Vec::new();
        for (id, keys) in &inside.rows {
            for e in keys.iter().filter_map(|k| effects(db, *k)) {
                if e.damage <= Fixed::ZERO {
                    continue;
                }
                let Some(t) = self.units.get_mut(*id).filter(|t| affected(t)) else { break };
                let Some(def) = db.unit(t.def) else { break };
                let d = e.damage * dt();
                let wanted = if db.unit_features(def).crew_loss_on_hit {
                    let mut ctx = crate::combat::casualty::Ctx { subsystems: &mut self.subsystems, rng: &mut self.rng, switches: &self.switches, colonies: &mut self.colonies };
                    crate::combat::casualty::after_variance(t, def, db, &mut ctx, 0, d, e.crew_loss).1
                } else {
                    let taken = d.min(t.shields.now[0]);
                    t.shields.now[0] = t.shields.now[0] - taken;
                    d - taken
                };
                let floor = if t.flags.contains(UnitFlags::IMMORTAL) { Fixed::ONE } else { Fixed::ZERO };
                let hull = wanted.min((t.hull[0] - floor).max(Fixed::ZERO));
                t.hull[0] = t.hull[0] - hull;
                if t.hull[0] <= Fixed::ZERO {
                    killed.push((*id, None));
                }
            }
        }
        killed
    }
}

#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod shield_tests;
