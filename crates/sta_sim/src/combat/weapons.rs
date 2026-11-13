//! Weapons (SIM-04): targeting, hardpoints, fire arcs, refire, beams and
//! projectiles, emitting `Fired` and `Hit` (S4).
//!
//! - Each armed entity has one mount per hardpoint of its definition. A mount
//!   fires when its weapon is ready, the target is alive, inside the weapon's
//!   range and inside the hardpoint's arc ([`in_arc`]).
//!   In a seated game the target must also be in the shooter's team's current
//!   sensor visibility; an allied spotter can supply that vision.
//! - A beam hits on the tick it fires and is drawn for `duration` ticks (the
//!   client derives `fire.end` as the `Fired` tick plus `duration`). A
//!   projectile flies at `shot_speed` toward its target's current position and
//!   hits on the tick it arrives; its first move is the tick after it fired.
//! - Targets: an `Attack` order, else the nearest enemy in the weapon's range
//!   (ties to the lowest id). Point-defence weapons do not shoot at units (`pd.rs` runs them).
//! - `Hit` carries the weapon's damage as `hull` and zero `shield`: applying
//!   it (shields, hull, destruction) is `damage.rs`, which rewrites the split.
//!
//! Order each tick: shots in flight move (ascending shot id), then shooters
//! fire (ascending entity id, hardpoints in definition order).
//! Not yet: bursts (no field in `WeaponDef`), mount positions (shots leave
//! the entity's centre), entity radii.

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, FireArc, WeaponKind};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TICK_HZ, TeamId, Vec3};

use super::pd::{Pd, PdRec};
use crate::Rng;
use crate::specials::{self, Specials};
use crate::event::{SimEvent, Target};
use crate::view::UnitFlags;
use crate::world::{Entities, Entity, Fnv, Table};

/// The XZ-plane direction from `from` to `to`, normalised, and the distance.
pub(super) fn flat(from: Vec3, to: Vec3) -> ((Fixed, Fixed), Fixed) {
    let (dx, dz) = (to.x - from.x, to.z - from.z);
    let d = (dx * dx + dz * dz).sqrt();
    if d == Fixed::ZERO {
        ((Fixed::ZERO, Fixed::ZERO), d)
    } else {
        ((dx / d, dz / d), d)
    }
}

/// Is `to` inside a hardpoint's arc, for a unit at `from` facing `heading`
/// (a unit vector `(x, z)`)? The arc is in the unit's frame, +z ahead, +x to
/// starboard; `None` is all round. A target at the same spot is inside.
pub fn in_arc(heading: (Fixed, Fixed), arc: Option<FireArc>, from: Vec3, to: Vec3) -> bool {
    let Some(arc) = arc else { return true };
    let (dir, d) = flat(from, to);
    if d == Fixed::ZERO {
        return true;
    }
    // The arc's direction in world space: x along starboard, z along heading.
    let right = (heading.1, -heading.0);
    let ax = arc.dir.0 * right.0 + arc.dir.1 * heading.0;
    let az = arc.dir.0 * right.1 + arc.dir.1 * heading.1;
    ax * dir.0 + az * dir.1 >= arc.min_dot
}

/// Which side of a target (heading `heading`, at `at`) `from` is on:
/// 0 fore, 1 aft, 2 port, 3 starboard (the `Hit.facing` of S4).
pub fn facing(heading: (Fixed, Fixed), at: Vec3, from: Vec3) -> u8 {
    let (dir, _) = flat(at, from);
    let fwd = dir.0 * heading.0 + dir.1 * heading.1;
    let side = dir.0 * heading.1 - dir.1 * heading.0;
    if fwd.abs() >= side.abs() {
        if fwd >= Fixed::ZERO { 0 } else { 1 }
    } else if side >= Fixed::ZERO {
        3
    } else {
        2
    }
}

/// A draw from the sim's RNG in [0, 1).
pub(super) fn draw(rng: &mut Rng) -> Fixed {
    Fixed::from_bits((rng.next_u64() >> 32) as i64)
}

/// Whole ticks of `refire` ticks scaled by `factor`.
fn scaled(refire: u64, factor: Fixed) -> u64 {
    let t = Fixed::from_int(refire.min(i32::MAX as u64) as i32) * factor;
    (t.to_bits() >> 32).max(0) as u64
}

/// Ticks of initial reload (SIM-35). `initialShotDelayModifier` m: absent
/// reads as -1 (the engine's default; stock has no such key), 0 -> none,
/// m > 0 -> `shotDelay` x m, m < 0 -> U[0, -m) x `shotDelay`. Evidence: a
/// special launcher measured on stock and Roots; for regular weapons the
/// rule is read from the engine's code only (not measured).
fn initial_reload(w: &sta_content::WeaponDef, rng: &mut Rng) -> u64 {
    if w.refire == 0 {
        return 0;
    }
    let m = w.initial_shot_delay_modifier.unwrap_or(-Fixed::ONE);
    if m == Fixed::ZERO {
        0
    } else if m > Fixed::ZERO {
        scaled(w.refire, m)
    } else {
        scaled(w.refire, draw(rng) * -m)
    }
}

/// Ticks until the weapon may shoot again: `shotDelay` and, under
/// `weapon_shot_timing`, 0 to 5 % jitter from the sim's RNG unless the
/// weapon sets `ignoreShotDelayEffects`. Team modifier and crew penalty are
/// not modelled. At least one tick.
fn shot_reload(timing: bool, w: &sta_content::WeaponDef, rng: &mut Rng) -> u64 {
    if timing && w.refire > 0 && !w.ignore_shot_delay_effects {
        let jitter = draw(rng) * Fixed::from_int(5) / Fixed::from_int(100);
        scaled(w.refire, Fixed::ONE + jitter).max(1)
    } else {
        w.refire.max(1)
    }
}

/// One hardpoint's firing state: the first tick its weapon may fire again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Mount {
    ready: u64,
}

/// A projectile in flight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Shot {
    pub(super) id: u32,
    pub(super) shooter: EntityId,
    pub(super) weapon: ContentKey,
    pub(super) target: EntityId,
    pub(super) pos: Vec3,
    /// Units per tick.
    pub(super) step: Fixed,
    pub(super) damage: Fixed,
}

/// Weapon state of the whole sim.
#[derive(Debug, Clone, Default)]
pub struct Weapons {
    /// What each unit was ordered to attack.
    attack: Table<EntityId>,
    mounts: Table<Vec<Mount>>,
    shots: Vec<Shot>,
    next_shot: u32,
    pd: Pd,
}

/// [`Weapons`] as the snapshot stores it (SIM-10).
#[derive(Serialize, Deserialize)]
pub(crate) struct WeaponsRec {
    attack: Vec<(EntityId, EntityId)>,
    mounts: Vec<(EntityId, Vec<Mount>)>,
    shots: Vec<Shot>,
    next_shot: u32,
    /// Point-defence cycle timers (SIM-06); a format change for snapshot.rs.
    pd: PdRec,
}

impl Weapons {
    pub(crate) fn to_rec(&self) -> WeaponsRec {
        WeaponsRec {
            attack: self.attack.iter().map(|(id, t)| (id, *t)).collect(),
            mounts: self.mounts.iter().map(|(id, m)| (id, m.clone())).collect(),
            shots: self.shots.clone(),
            next_shot: self.next_shot,
            pd: self.pd.to_rec(),
        }
    }

    /// `None` if the rows are not ascending by id.
    pub(crate) fn from_rec(r: WeaponsRec) -> Option<Weapons> {
        Some(Weapons {
            attack: Table::from_rows(r.attack)?,
            mounts: Table::from_rows(r.mounts)?,
            shots: r.shots,
            next_shot: r.next_shot,
            pd: Pd::from_rec(r.pd)?,
        })
    }

    /// Order `unit` to attack `target` until it is gone.
    pub fn set_attack(&mut self, unit: EntityId, target: EntityId) {
        self.attack.insert(unit, target);
    }

    pub fn attack_of(&self, unit: EntityId) -> Option<EntityId> {
        self.attack.get(unit).copied()
    }

    /// Make a unit's mounts with their initial reload when any of its weapons
    /// uses `weapon_shot_timing` and it has none yet.
    fn init_mounts(&mut self, tick: u64, db: &ContentDb, id: EntityId, def: &sta_content::UnitDef, rng: &mut Rng) {
        if self.mounts.get(id).is_some() {
            return;
        }
        let timed = |hp: &sta_content::Hardpoint| {
            db.weapon(hp.weapon.key)
                .filter(|w| w.kind != WeaponKind::PointDefence && db.weapon_features(w).weapon_shot_timing)
        };
        if !def.hardpoints.iter().any(|hp| timed(hp).is_some()) {
            return;
        }
        let ready = def
            .hardpoints
            .iter()
            .map(|hp| Mount { ready: tick + timed(hp).map_or(0, |w| initial_reload(w, rng)) })
            .collect();
        self.mounts.insert(id, ready);
    }

    /// Is the PD weapon at hardpoint `i` of `unit` switched on (energy model)?
    pub fn pd_on(&self, unit: EntityId, i: usize) -> bool {
        self.pd.is_on(unit, i)
    }

    /// The point-defence rows, as the snapshot stores them.
    #[cfg(test)]
    pub(crate) fn pd_rec(&self) -> PdRec {
        self.pd.to_rec()
    }

    /// Projectiles in flight.
    pub fn shots_in_flight(&self) -> usize {
        self.shots.len()
    }

    /// Nothing to hash: the digest then equals the one from before weapons.
    pub(crate) fn is_empty(&self) -> bool {
        self.attack.is_empty()
            && self.mounts.is_empty()
            && self.shots.is_empty()
            && self.next_shot == 0
            && self.pd.is_empty()
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.u64(self.attack.len() as u64);
        for (id, t) in self.attack.iter() {
            h.u64(id.0 as u64);
            h.u64(t.0 as u64);
        }
        h.u64(self.mounts.len() as u64);
        for (id, ms) in self.mounts.iter() {
            h.u64(id.0 as u64);
            for m in ms {
                h.u64(m.ready);
            }
        }
        h.u64(self.shots.len() as u64);
        for s in &self.shots {
            h.u64(s.id as u64);
            h.u64(s.shooter.0 as u64);
            h.u64(s.weapon.0 as u64);
            h.u64(s.target.0 as u64);
            for f in [s.pos.x, s.pos.y, s.pos.z, s.step, s.damage] {
                h.fixed(f);
            }
        }
        h.u64(self.next_shot as u64);
        if !self.pd.is_empty() {
            self.pd.hash_into(h);
        }
    }

    /// One tick of weapons at tick `tick` (the tick being computed).
    ///
    /// Standalone form: it has no special-energy pools and draws from a
    /// throwaway RNG, so it is only right for content without
    /// `pd_energy` and `weapon_shot_timing`; the sim uses
    /// [`Weapons::step_with_perception`].
    pub fn step(
        &mut self,
        tick: u64,
        units: &Entities,
        db: &ContentDb,
        teams: &[(PlayerId, TeamId)],
        events: &mut Vec<SimEvent>,
    ) {
        let (mut specials, mut rng) = (Specials::default(), Rng::new(1));
        self.step_with_perception(tick, units, db, teams, &Table::new(), &mut specials, &mut rng, events);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn step_with_perception(
        &mut self,
        tick: u64,
        units: &Entities,
        db: &ContentDb,
        teams: &[(PlayerId, TeamId)],
        perception: &Table<PlayerId>,
        specials: &mut Specials,
        rng: &mut Rng,
        events: &mut Vec<SimEvent>,
    ) {
        let alive = |id: EntityId| units.get(id).filter(|u| u.hull[1] > Fixed::ZERO);
        let gone: Vec<EntityId> = self
            .attack
            .iter()
            .filter(|(id, t)| !units.contains(*id) || alive(**t).is_none())
            .map(|(id, _)| id)
            .collect();
        for id in gone {
            self.attack.remove(id);
        }
        let gone: Vec<EntityId> = self
            .mounts
            .iter()
            .filter(|(id, _)| !units.contains(*id))
            .map(|(id, _)| id)
            .collect();
        for id in gone {
            self.mounts.remove(id);
        }

        self.pd
            .step(tick, units, db, teams, &mut self.shots, specials, rng, events);

        // Shots in flight home on the target's current position.
        let mut flying = Vec::with_capacity(self.shots.len());
        for mut s in std::mem::take(&mut self.shots) {
            let Some(t) = alive(s.target) else { continue };
            let (dir, d) = flat(s.pos, t.pos);
            if d <= s.step {
                let face = facing(t.heading, t.pos, s.pos);
                events.push(SimEvent::Hit {
                    target: s.target,
                    by: s.shooter,
                    weapon: s.weapon,
                    facing: face,
                    shield: Fixed::ZERO,
                    hull: s.damage,
                    at: t.pos,
                });
            } else {
                s.pos.x = s.pos.x + dir.0 * s.step;
                s.pos.z = s.pos.z + dir.1 * s.step;
                flying.push(s);
            }
        }
        self.shots = flying;

        // Compute after this tick's movement, using the same shared-vision rule
        // as WorldView (which then also leaves out `hidden = 1` units).
        // `invalidAsTarget = 1` units are never picked automatically (SIM-28); an explicit order is not refused (unmeasured).
        // Do not use last tick's view or explored fog.
        let visibility = crate::sensors::compute(units, Some(db), teams);
        for u in units.iter() {
            let Some(def) = db.unit(u.def) else { continue };
            // Effect objects (`u_ball_obj`) carry effect spawners, not guns (SIM-28).
            if def.hardpoints.is_empty() || def.class.0 == "u_ball_obj" {
                continue;
            }
            // SIM-35: a weapon is created with an initial reload, also when
            // its unit starts with weapons off. A unit's mounts are made the
            // first tick it is seen, the creation tick for all practical
            // purposes; ascending id keeps the draws stable.
            self.init_mounts(tick, db, u.id, def, rng);
            if u.flags.contains(UnitFlags::WEAPONS_OFF) {
                continue;
            }
            for (i, hp) in def.hardpoints.iter().enumerate() {
                if self.mounts.get(u.id).is_some_and(|m| m[i].ready > tick) {
                    continue;
                }
                let Some(w) = db.weapon(hp.weapon.key) else {
                    continue;
                };
                if w.kind == WeaponKind::PointDefence || crate::specials::is_special(w) || w.resource.is_some() {
                    continue;
                }
                let arc = if db.features().fire_arcs {
                    hp.arc
                } else {
                    None
                };
                // Seatless legacy/bench sources are omniscient, as their views
                // are. An unknown owner in a configured game sees nothing.
                let seen = (!teams.is_empty()).then(|| visibility.iter()
                    .find(|(player, _)| *player == u.owner)
                    .map_or(&[][..], |(_, ids)| ids.as_slice()));
                let Some(t) = self.pick(db, u, w.range, arc, units, teams, perception, seen) else {
                    continue;
                };
                if self.mounts.get(u.id).is_none() {
                    self.mounts
                        .insert(u.id, vec![Mount { ready: 0 }; def.hardpoints.len()]);
                }
                if let Some(m) = self.mounts.get_mut(u.id) {
                    m[i].ready = tick + shot_reload(db.weapon_features(w).weapon_shot_timing, w, rng);
                }
                events.push(SimEvent::Fired {
                    shooter: u.id,
                    weapon: w.key,
                    hardpoint: i as u8,
                    target: Target::Entity(t.id),
                });
                if w.kind == WeaponKind::Projectile {
                    self.next_shot += 1;
                    let step = w.shot_speed / Fixed::from_int(TICK_HZ as i32);
                    self.shots.push(Shot {
                        id: self.next_shot,
                        shooter: u.id,
                        weapon: w.key,
                        target: t.id,
                        pos: u.pos,
                        step,
                        damage: w.damage,
                    });
                } else {
                    let face = facing(t.heading, t.pos, u.pos);
                    events.push(SimEvent::Hit {
                        target: t.id,
                        by: u.id,
                        weapon: w.key,
                        facing: face,
                        shield: Fixed::ZERO,
                        hull: w.damage,
                        at: t.pos,
                    });
                }
            }
        }
    }

    /// An `Order::Special` (SIM-34, SIM-35): `slot` counts the unit's
    /// specials in hardpoint order. A point-defence slot is toggled here; any
    /// other goes to `specials`. Under `weapon_shot_timing` an order that
    /// arrives while the special's weapon is reloading is refused, not
    /// queued; an accepted one starts the reload. True when one unit took it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn order_special(
        &mut self,
        tick: u64,
        db: &ContentDb,
        units: &Entities,
        specials: &mut Specials,
        rng: &mut Rng,
        player: PlayerId,
        ids: &[EntityId],
        slot: u8,
    ) -> bool {
        let mut any = false;
        for &id in ids {
            let Some(u) = units.get(id).filter(|u| (!player.is_player() || u.owner == player) && u.hull[0] > Fixed::ZERO) else {
                continue;
            };
            let Some(def) = db.unit(u.def) else { continue };
            let Some((i, kind, w)) = specials::specials_indexed(db, def).nth(slot as usize) else { continue };
            self.init_mounts(tick, db, id, def, rng);
            let timing = db.weapon_features(w).weapon_shot_timing;
            if timing && self.mounts.get(id).is_some_and(|m| m[i].ready > tick) {
                continue;
            }
            let ok = match kind {
                specials::SpecialKind::PointDefence => {
                    specials.ensure_pool(id, def);
                    self.pd.toggle(db, specials, id, def, i)
                }
                _ => specials.order_special(db, units, player, &[id], slot),
            };
            if ok {
                any = true;
                if timing && kind != specials::SpecialKind::PointDefence {
                    if self.mounts.get(id).is_none() {
                        self.mounts.insert(id, vec![Mount { ready: 0 }; def.hardpoints.len()]);
                    }
                    if let Some(m) = self.mounts.get_mut(id) {
                        m[i].ready = tick + shot_reload(true, w, rng);
                    }
                }
            }
        }
        any
    }

    /// Drop the attack orders of the listed units that `player` may order.
    pub(crate) fn stop_attack(&mut self, units: &Entities, player: PlayerId, ids: &[EntityId]) {
        for &id in ids {
            if units
                .get(id)
                .is_some_and(|u| !player.is_player() || u.owner == player)
            {
                self.attack.remove(id);
            }
        }
    }

    /// Apply an `Attack` order: every listed unit of `player` (any, for
    /// scripts and the bench) that can shoot is set on `target`. False when
    /// the target is not a valid one or no unit took the order.
    pub(crate) fn order_attack(
        &mut self,
        db: &ContentDb,
        units: &Entities,
        player: PlayerId,
        ids: &[EntityId],
        target: EntityId,
    ) -> bool {
        if !units.get(target).is_some_and(|t| t.hull[1] > Fixed::ZERO) {
            return false;
        }
        let mut any = false;
        for &id in ids {
            let armed = |u: &Entity| db.unit(u.def).is_some_and(|d| !d.hardpoints.is_empty());
            if id != target
                && units
                    .get(id)
                    .is_some_and(|u| (!player.is_player() || u.owner == player) && armed(u))
            {
                self.set_attack(id, target);
                any = true;
            }
        }
        any
    }

    /// The target a mount of `u` shoots at, if any: the ordered one when it
    /// can be hit, else the nearest enemy that can.
    #[allow(clippy::too_many_arguments)]
    fn pick<'a>(
        &self,
        db: &ContentDb,
        u: &Entity,
        range: Fixed,
        arc: Option<FireArc>,
        units: &'a Entities,
        teams: &[(PlayerId, TeamId)],
        perception: &Table<PlayerId>,
        seen: Option<&[EntityId]>,
    ) -> Option<&'a Entity> {
        let team = |p: PlayerId| teams.iter().find(|x| x.0 == p).map(|x| x.1);
        // A cloaked unit is only a target for its own team.
        let unseen = |t: &Entity| t.flags.contains(UnitFlags::CLOAKED) && team(t.owner) != team(u.owner);
        let hittable = |t: &Entity| {
            t.id != u.id
                && seen.is_none_or(|ids| ids.binary_search(&t.id).is_ok())
                && !unseen(t)
                && t.hull[1] > Fixed::ZERO
                && flat(u.pos, t.pos).1 <= range
                && in_arc(u.heading, arc, u.pos, t.pos)
        };
        if let Some(t) = self.attack.get(u.id).and_then(|&t| units.get(t)) {
            return hittable(t).then_some(t);
        }
        let mine = team(u.owner)?;
        let mut best: Option<(Fixed, &Entity)> = None;
        for t in units.iter() {
            let perceived = perception.get(t.id).copied().unwrap_or(t.owner);
            if team(perceived).is_none_or(|x| x == mine) || !hittable(t) || db.unit(t.def).is_some_and(|d| d.invalid_as_target) {
                continue;
            }
            let d = flat(u.pos, t.pos).1;
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, t));
            }
        }
        best.map(|b| b.1)
    }
}
