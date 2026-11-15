//! Wormhole gates (`classLabel = "wormhole_obj"`): a craft sent into a gate comes out of
//! its partner (LUA-17; Armada II and Fleet Operations since the stateless step, on two-gate maps).
//!
//! Rules (read from the Armada I and Armada II executables, static; not measured on the
//! original):
//! - **Partners (Armada II family).** Only when the map file placed exactly two gates (labelled
//!   entities) are they a pair, in file order; with any other number nothing travels, and a
//!   gate a script adds joins no pair, until LUA-33 reads the maps' partner pointers. Read from
//!   the stock, Roots and mod maps' data: every pointer pair of a two-gate map is mutual (see the
//!   LUA-24 travel PR); not measured in the original.
//! - **Partners (Armada I).** Armada I pairs gates by creation order: each new gate pairs with the
//!   one left unpaired, so the 1st pairs with the 2nd, the 3rd with the 4th. Here the
//!   live gates, in id (creation) order, pair in twos; this matches the original until a
//!   paired gate is destroyed (its partner is then re-paired in the original, not here).
//! - **Who goes through.** A craft flying its last leg onto the gate (its target within
//!   [`ON_GATE`] of the centre and no further legs queued: a move ordered onto the gate
//!   entity) enters it; craft passing by, or on an earlier leg of a longer course, do not.
//!   Any team, any craft. Asked before routing and avoidance replace the target with a detour.
//! - **Transport.** Once it is within its `tooCloseToTurn` plus [`GATE_REACH`] of the
//!   centre (the original queues it there and flies it along an entry spline into the
//!   centre; the spline is not modelled), it is moved to its partner at rest with no
//!   course (an order onto the gate ends idle on the far side, as the original's). The
//!   Armada I profile sets it down [`GATE_REACH`] ahead of the partner along the partner's
//!   facing (not measured); the Armada II family sets it at the partner's centre, which is
//!   where the original's transport puts it before the exit queue carries it out (read from
//!   the executable, static; the exit spline is not modelled). The Roots gameplay tutorial
//!   tests a box 71 units around its gate, which the 75 offset would miss.
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

/// The least space, in units, between craft set down at one gate (not measured).
const MIN_SPACING: i32 = 30;

const WORMHOLE_CLASS: &str = "wormhole_obj";

fn flat_distance(a: Vec3, b: Vec3) -> Fixed {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

impl Sim {
    /// Live gates in id order, each with its partner (Armada I pairing), as `(gate, partner)`.
    fn gate_pairs(&self) -> Vec<(EntityId, EntityId)> {
        let Some(db) = self.content.as_deref() else { return Vec::new() };
        let a1 = db.features().a1_compat;
        let gates: Vec<EntityId> = self
            .units
            .iter()
            .filter(|e| e.kind == crate::world::EntityKind::Prop && !e.flags.contains(UnitFlags::DYING))
            .filter(|e| db.unit(e.def).is_some_and(|d| d.class.0 == WORMHOLE_CLASS))
            // The Armada II family pairs only the gates the map file placed (the only labelled
            // entities); a gate a script adds is not paired.
            .filter(|e| a1 || self.labels.get(e.id).is_some())
            .map(|e| e.id)
            .collect();
        // Armada II family: exactly two map gates are a pair; any other number is not paired
        // (file order is not the pointers there, e.g. Roots `nirvana6`, A2 `fed05`, `fed06`,
        // `borg10`) until LUA-33 carries the partner pointers.
        if !a1 && gates.len() != 2 {
            return Vec::new();
        }
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
        let out = if self.content.as_deref().is_some_and(|db| db.features().a1_compat) { reach } else { Fixed::ZERO };
        for &(id, gate, partner) in goers {
            let (Some(g), Some(p)) = (self.units.get(gate), self.units.get(partner)) else { continue };
            let (gate_pos, exit) = (g.pos, p.pos);
            let ahead = p.heading;
            let Some(u) = self.units.get(id) else { continue };
            if u.flags.contains(UnitFlags::DYING) || flat_distance(u.pos, gate_pos) > u.stats.too_close_to_turn + reach {
                continue;
            }
            let mut to = Vec3 { x: exit.x + ahead.0 * out, y: exit.y, z: exit.z + ahead.1 * out };
            if out == Fixed::ZERO {
                to = self.free_exit_spot(id, to);
            }
            if let Some(u) = self.units.get_mut(id) {
                u.pos = to;
                u.target = None;
                u.speed = Fixed::ZERO;
                u.omega = Fixed::ZERO;
            }
            self.movement.clear_waypoints(id);
        }
    }

    /// The nearest free spot to `centre` for `id` to be set down: the centre itself, else the
    /// first of eight directions per ring, rings one spacing apart (the original queues craft out
    /// one at a time; STA64 sets each down at once, so a craft that finds the mouth held stands
    /// beside it). Free is no other craft within the spacing.
    fn free_exit_spot(&self, id: EntityId, centre: Vec3) -> Vec3 {
        let spacing = (self.units.get(id).map_or(Fixed::ZERO, |u| self.body_radius(u.def)) * Fixed::from_int(2)).max(Fixed::from_int(MIN_SPACING));
        // Unit vectors of eight directions, in thousandths.
        const DIRS: [(i32, i32); 8] = [(1000, 0), (707, 707), (0, 1000), (-707, 707), (-1000, 0), (-707, -707), (0, -1000), (707, -707)];
        let held = |p: Vec3| {
            self.units
                .iter()
                .any(|o| o.id != id && o.kind == crate::world::EntityKind::Craft && !o.flags.contains(UnitFlags::DYING) && flat_distance(o.pos, p) < spacing)
        };
        for k in 0..64i32 {
            let p = if k == 0 {
                centre
            } else {
                let (ring, dir) = (Fixed::from_int((k - 1) / 8 + 1), DIRS[((k - 1) % 8) as usize]);
                Vec3 {
                    x: centre.x + Fixed::from_int(dir.0) / Fixed::from_int(1000) * ring * spacing,
                    y: centre.y,
                    z: centre.z + Fixed::from_int(dir.1) / Fixed::from_int(1000) * ring * spacing,
                }
            };
            if !held(p) {
                return p;
            }
        }
        centre
    }
}

#[cfg(test)]
#[path = "wormholes_tests.rs"]
mod tests;
