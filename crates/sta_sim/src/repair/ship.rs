//! Repair ships (SIM-41, first slice). Evidence grades as in [`super::service`]: **S** read
//! from the executable, static; **D** read from data; **ours** STA64's own choice.
//!
//! - *Who (D, S).* A unit whose class label is `repairship` (stock `classLabel = "RepairShip"`).
//! - *Order (S).* The same repair order as a yard repair (`Order::Repair` with a target): given
//!   to a repair ship and naming a ship that is not a yard it could dock at, the repair ship
//!   repairs that ship. The target must be a live ship (not a station: ours, as the original's
//!   test is a craft method), not the repair ship itself, of an allied player (confirmed
//!   allies, the own player included), and need repair: hull or a system (not shields or
//!   crew, which the beam does not restore).
//! - *Approach (S).* More than [`BEAM_APPROACH`] apart (surface to surface) the repair ship
//!   moves to [`BEAM_APPROACH`] off the target's surface on its own side; nearer, it holds.
//! - *Beam (S, D).* While the surface gap is under the range of its first weapon (stock
//!   `gRepairBeam`, `range = 200`), each tick it does a yard's systems and hull work times its
//!   `repairRate` (stock 0.25): [`Sim::repair_systems_and_hull`](crate::Sim).
//! - *End (S).* A target that no longer qualifies (repaired, dead, gone, no longer allied)
//!   ends the order and the repair ship stops. Any other order ends it too.
//!
//! - *On its own (S; Roots from the changelog).* Every [`AUTO_REPAIR_SECS`] an idle repair ship
//!   (no order, course, attack or beam) on **high** movement autonomy takes the first damaged
//!   allied ship within its scan range (`rangeScan`), its own player's before an ally's (lowest
//!   id first: ours), and goes to it. On Roots (the Fo4 profile) one on **medium** autonomy takes
//!   only a ship already inside its beam's range and repairs it without moving (the beam ends if
//!   the ship leaves the range); stock leaves medium alone. Not measured.
//!
//! - *Shields (D; effect inferred from the key's name, not measured).* A repair ship with
//!   `repairsShields = 1` (FO/Roots) also restores shields, at a yard's shield rate times its
//!   `repairRate`, and counts low shields as damage.
//!
//! Not done (SIM-41's later slices): `alliedRepairUpgrade`, the `fedrepairteam`
//! / `esconverter` auto-fire gates, the beam's own effect on screen. Not measured: rate and range
//! against the original (WORKPLAN's paired scene).

use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId, Vec3};

use super::service::{alive, surface};
use crate::Sim;
use crate::world::EntityKind;

/// Seconds between an idle repair ship's looks for work (S).
pub const AUTO_REPAIR_SECS: u32 = 4;

/// Surface gap a repair ship closes to before it holds (S).
pub const BEAM_APPROACH: Fixed = Fixed::from_bits(50 << 32);

impl Sim {
    /// A live repair ship and its `repairRate` and beam range.
    fn repair_ship(&self, db: &ContentDb, id: EntityId) -> Option<(Fixed, Fixed)> {
        let u = self.units.get(id).filter(|u| alive(u) && u.kind == EntityKind::Craft)?;
        let def = db.unit(u.def).filter(|d| d.class.0 == "repairship")?;
        let range = def.hardpoints.first().and_then(|hp| db.weapon(hp.weapon.key)).map_or(Fixed::ZERO, |w| w.range);
        Some((def.repair.repair_rate.unwrap_or(Fixed::ZERO), range))
    }

    /// Whether the repair ship's beam also restores shields (`repairsShields = 1`).
    fn beam_shields(&self, db: &ContentDb, ship: EntityId) -> bool {
        self.units.get(ship).and_then(|u| db.unit(u.def)).is_some_and(|d| d.repair.repairs_shields == Some(true))
    }

    /// Hull or a system short, or shields when the beam restores them (what the beam restores).
    fn beam_needed(&self, db: &ContentDb, id: EntityId, shields: bool) -> bool {
        let Some(u) = self.units.get(id) else { return false };
        let Some(def) = db.unit(u.def) else { return false };
        let row = self.subsystems.get(id).unwrap_or_default();
        let pools = if u.shields.directional { 4 } else { 1 };
        u.hull[0] < u.hull[1]
            || (0..5).any(|n| def.system_hit_points[n] > Fixed::ZERO && row.damage[n] > 0)
            || (shields && u.shields.max > Fixed::ZERO && u.shields.now[..pools].iter().any(|s| *s < u.shields.max))
    }

    /// Whether `ship` may beam `target` now.
    fn beam_target_ok(&self, db: &ContentDb, ship: EntityId, target: EntityId) -> bool {
        let (Some(s), Some(t)) = (self.units.get(ship), self.units.get(target)) else { return false };
        ship != target
            && t.kind == EntityKind::Craft
            && alive(t)
            && crate::relations::Diplomacy::new(&self.players, &self.relations).confirmed_ally(s.owner, t.owner)
            && self.beam_needed(db, target, self.beam_shields(db, ship))
    }

    /// `Order::Repair` naming `target`: the repair ships among `units` that may beam it (and for
    /// which it is not a yard) take it as their beam target. Returns them.
    pub(super) fn assign_beams(&mut self, db: &ContentDb, player: PlayerId, units: &[EntityId], target: EntityId) -> Vec<EntityId> {
        let mut taken = Vec::new();
        for &id in units {
            let Some(owner) = self.units.get(id).map(|u| u.owner) else { continue };
            if (player.is_player() && owner != player) || self.repair_ship(db, id).is_none() || self.repair_yard_ok(db, target, owner) || !self.beam_target_ok(db, id, target) {
                continue;
            }
            self.repairs.set_beam(id, target, false);
            taken.push(id);
        }
        if !taken.is_empty() {
            // Like every order that replaces what a unit is doing.
            self.weapons.stop_attack(&self.units, player, &taken);
            self.economy.cancel(&self.units, player, &taken);
            self.production.cancel(db, &self.units, &mut self.economy, player, &taken);
            self.clear_scripted(player, &taken);
        }
        taken
    }

    /// Idle repair ships look for work on their own (see the module docs).
    pub(crate) fn auto_repair(&mut self) {
        if !self.tick.is_multiple_of(u64::from(AUTO_REPAIR_SECS * sta_core::TICK_HZ)) {
            return;
        }
        let Some(db) = self.content.clone() else { return };
        let roots = db.profile() == sta_content::Profile::Fo4;
        let ships: Vec<EntityId> = self.units.iter().map(|u| u.id).collect();
        for ship in ships {
            let Some((rate, range)) = self.repair_ship(&db, ship) else { continue };
            let level = self.autonomy.levels(ship)[0];
            let hold = match level {
                2 => false,
                1 if roots => true,
                _ => continue,
            };
            let Some(u) = self.units.get(ship) else { continue };
            let idle = rate > Fixed::ZERO
                && self.repairs.beam_of(ship).is_none()
                && self.repairs.yard_of(ship).is_none()
                && self.scripted.job(ship).is_none()
                && u.target.is_none()
                && self.weapons.attack_of(ship).is_none()
                && self.movement.course(ship).is_none_or(|c| c.waypoints.is_empty());
            if !idle {
                continue;
            }
            let owner = u.owner;
            let reach = if hold { range } else { db.unit(u.def).map_or(Fixed::ZERO, |d| d.sensor) };
            let sr = db.unit(u.def).map_or(Fixed::ZERO, |d| d.radius);
            let mut best: Option<(bool, EntityId)> = None;
            for t in self.units.iter() {
                if !self.beam_target_ok(&db, ship, t.id) {
                    continue;
                }
                let tr = db.unit(t.def).map_or(Fixed::ZERO, |d| d.radius);
                if surface(u, sr, t, tr) >= reach {
                    continue;
                }
                let own = t.owner == owner;
                if best.is_none_or(|(b_own, _)| own && !b_own) {
                    best = Some((own, t.id));
                }
            }
            if let Some((_, target)) = best {
                self.repairs.set_beam(ship, target, hold);
            }
        }
    }

    /// One tick of every repair ship's beam.
    pub(super) fn step_beams(&mut self, db: &ContentDb) {
        for (ship, target, hold) in self.repairs.beams.clone() {
            let rs = self.repair_ship(db, ship);
            if rs.is_none() || !self.beam_target_ok(db, ship, target) {
                self.repairs.end(ship);
                if rs.is_some() {
                    self.set_target(ship, None);
                }
                continue;
            }
            let (rate, range) = rs.unwrap_or_default();
            let (Some(s), Some(t)) = (self.units.get(ship), self.units.get(target)) else { continue };
            let radius = |e: &crate::world::Entity| db.unit(e.def).map_or(Fixed::ZERO, |d| d.radius);
            let (sr, tr) = (radius(s), radius(t));
            let gap = surface(s, sr, t, tr);
            if hold {
                // Taken where it stands: out of range it lets the target go instead of following.
                if gap >= range {
                    self.repairs.end(ship);
                    continue;
                }
            } else if gap > BEAM_APPROACH {
                let to = approach_point(t.pos, s.pos, sr + tr + BEAM_APPROACH);
                self.set_target(ship, Some(to));
            } else {
                self.set_target(ship, None);
            }
            if gap < range && rate > Fixed::ZERO {
                self.repair_systems_and_hull(db, target, rate);
                if self.beam_shields(db, ship) {
                    self.restore_shields(db, target, rate);
                }
            }
        }
    }
}

/// The point `reach` from `target` towards `from` on the plane (straight ahead of the target
/// when they coincide).
fn approach_point(target: Vec3, from: Vec3, reach: Fixed) -> Vec3 {
    let (dx, dz) = (from.x - target.x, from.z - target.z);
    let sq = ((i128::from(dx.to_bits()) * i128::from(dx.to_bits()) + i128::from(dz.to_bits()) * i128::from(dz.to_bits())) >> 32).min(i128::from(i64::MAX)) as i64;
    let d = Fixed::from_bits(sq).sqrt();
    if d <= Fixed::ZERO {
        return Vec3 { x: target.x, y: target.y, z: target.z + reach };
    }
    Vec3 { x: target.x + dx * reach / d, y: target.y, z: target.z + dz * reach / d }
}
