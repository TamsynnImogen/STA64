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
//! Not done (SIM-41's later slices): the automatic search on high movement autonomy and Roots'
//! medium-autonomy rule, `repairsShields`, `alliedRepairUpgrade`, the `fedrepairteam` /
//! `esconverter` auto-fire gates, the beam's own effect on screen. Not measured: rate and range
//! against the original (WORKPLAN's paired scene).

use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId, Vec3};

use super::service::{alive, surface};
use crate::Sim;
use crate::world::EntityKind;

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

    /// Hull or a system short (what the beam restores).
    fn beam_needed(&self, db: &ContentDb, id: EntityId) -> bool {
        let Some(u) = self.units.get(id) else { return false };
        let Some(def) = db.unit(u.def) else { return false };
        let row = self.subsystems.get(id).unwrap_or_default();
        u.hull[0] < u.hull[1] || (0..5).any(|n| def.system_hit_points[n] > Fixed::ZERO && row.damage[n] > 0)
    }

    /// Whether `ship` may beam `target` now.
    fn beam_target_ok(&self, db: &ContentDb, ship: EntityId, target: EntityId) -> bool {
        let (Some(s), Some(t)) = (self.units.get(ship), self.units.get(target)) else { return false };
        ship != target
            && t.kind == EntityKind::Craft
            && alive(t)
            && crate::relations::Diplomacy::new(&self.players, &self.relations).confirmed_ally(s.owner, t.owner)
            && self.beam_needed(db, target)
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
            self.repairs.set_beam(id, target);
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

    /// One tick of every repair ship's beam.
    pub(super) fn step_beams(&mut self, db: &ContentDb) {
        for (ship, target) in self.repairs.beams.clone() {
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
            if gap > BEAM_APPROACH {
                let to = approach_point(t.pos, s.pos, sr + tr + BEAM_APPROACH);
                self.set_target(ship, Some(to));
            } else {
                self.set_target(ship, None);
            }
            if gap < range && rate > Fixed::ZERO {
                self.repair_systems_and_hull(db, target, rate);
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
