//! Wormhole gates (`classLabel = "wormhole_obj"`): a craft sent into a gate comes out of
//! its partner (LUA-17).
//!
//! Rules (read from the Armada I and Armada II executables, static; not measured on the
//! original):
//! - **Partners.** Armada I pairs gates by creation order: each new gate pairs with the
//!   one left unpaired, so the 1st pairs with the 2nd, the 3rd with the 4th. Here the
//!   live gates, in id (creation) order, pair in twos; this matches the original until a
//!   paired gate is destroyed (its partner is then re-paired in the original, not here).
//!   Armada II pairs only through map data and `LinkWormholes`, neither of which the sim
//!   carries yet, so gates move nothing outside the Armada I profile (`a1_compat`).
//! - **Who goes through.** A craft flying its last leg onto the gate (its target within
//!   [`ON_GATE`] of the centre and no further legs queued: a move ordered onto the gate
//!   entity) enters it; craft passing by, or on an earlier leg of a longer course, do not.
//!   Any team, any craft. Asked before routing and avoidance replace the target with a detour.
//! - **Transport.** Once it is within its `tooCloseToTurn` plus [`GATE_REACH`] of the
//!   centre (the original queues it there and flies it along an entry spline into the
//!   centre; the spline is not modelled), it is moved to its partner, set down
//!   [`GATE_REACH`] ahead of the partner along the partner's facing, at rest with no
//!   course (an order onto the gate ends idle on the far side, as the original's).
//!   Gate opening and closing (`AnimationTime`, `ClosedScale`, `OpenScale`, sounds) is
//!   presentation and not modelled; `DisableTimer` is never armed in either game.
//!
//! Nothing here is kept between ticks: the pairing and the gate goers are derived from
//! the entities and courses, so snapshots and the digest are unchanged.

use sta_core::{EntityId, Fixed, Vec3};

use crate::Sim;
use crate::view::UnitFlags;

/// A move whose last point is this close to a gate centre is a move into the gate.
const ON_GATE: i32 = 5;
/// How far from the centre a craft is taken in, beyond its `tooCloseToTurn`, and how far
/// ahead of the exit gate it comes out (the gate's reach; not measured).
const GATE_REACH: i32 = 75;

const WORMHOLE_CLASS: &str = "wormhole_obj";

fn flat_distance(a: Vec3, b: Vec3) -> Fixed {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

impl Sim {
    /// Live gates in id order, each with its partner (Armada I pairing), as `(gate, partner)`.
    fn gate_pairs(&self) -> Vec<(EntityId, EntityId)> {
        let Some(db) = self.content.as_deref() else { return Vec::new() };
        if !db.features().a1_compat {
            return Vec::new();
        }
        let gates: Vec<EntityId> = self
            .units
            .iter()
            .filter(|e| !e.flags.contains(UnitFlags::DYING))
            .filter(|e| db.unit(e.def).is_some_and(|d| d.class.0 == WORMHOLE_CLASS))
            .map(|e| e.id)
            .collect();
        gates.chunks_exact(2).flat_map(|p| [(p[0], p[1]), (p[1], p[0])]).collect()
    }

    /// Craft flying their last leg onto a paired gate: `(craft, gate, partner)`. Asked before
    /// the tick's routing and movement, while the target is still the ordered one.
    pub(crate) fn wormhole_goers(&self) -> Vec<(EntityId, EntityId, EntityId)> {
        let pairs = self.gate_pairs();
        if pairs.is_empty() {
            return Vec::new();
        }
        let on_gate = Fixed::from_int(ON_GATE);
        let mut out = Vec::new();
        for e in self.units.iter() {
            if e.kind != crate::world::EntityKind::Craft || e.flags.contains(UnitFlags::DYING) {
                continue;
            }
            if self.movement.course(e.id).is_some_and(|c| !c.waypoints.is_empty()) {
                continue;
            }
            let Some(goal) = e.target else { continue };
            for &(gate, partner) in &pairs {
                if self.units.get(gate).is_some_and(|g| flat_distance(g.pos, goal) <= on_gate) {
                    out.push((e.id, gate, partner));
                    break;
                }
            }
        }
        out
    }

    /// Move each goer that has reached its gate to the gate's partner.
    pub(crate) fn step_wormholes(&mut self, goers: &[(EntityId, EntityId, EntityId)]) {
        let reach = Fixed::from_int(GATE_REACH);
        for &(id, gate, partner) in goers {
            let (Some(g), Some(p)) = (self.units.get(gate), self.units.get(partner)) else { continue };
            let (gate_pos, exit) = (g.pos, p.pos);
            let ahead = p.heading;
            let Some(u) = self.units.get(id) else { continue };
            if u.flags.contains(UnitFlags::DYING) || flat_distance(u.pos, gate_pos) > u.stats.too_close_to_turn + reach {
                continue;
            }
            let to = Vec3 { x: exit.x + ahead.0 * reach, y: exit.y, z: exit.z + ahead.1 * reach };
            if let Some(u) = self.units.get_mut(id) {
                u.pos = to;
                u.target = None;
                u.speed = Fixed::ZERO;
                u.omega = Fixed::ZERO;
            }
            self.movement.clear_waypoints(id);
        }
    }
}

#[cfg(test)]
#[path = "wormholes_tests.rs"]
mod tests;
