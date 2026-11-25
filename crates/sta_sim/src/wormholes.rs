//! Wormhole gates (`classLabel = "wormhole_obj"`): a craft sent onto a gate comes out of its
//! partner (LUA-17 for Armada I; the stateless interim step for the Armada II family).
//!
//! Rules (read from the Armada I and Armada II executables, static; not measured on the
//! original):
//! - **Who has it.** Armada I (`a1_compat`): as before, unchanged. The Armada II family (the Fo4,
//!   Fo3 and Sta2Classic profiles: stock Armada II, STA2 Classic, Fleet Operations 3.x, and 4.0
//!   with Roots): the rule below. Native and any other profile: no gate moves anything, as before.
//! - **Partners (Armada II family).** Only when the map file placed exactly two gates, counted by
//!   class (`wormhole_obj`, any case) among the labelled entities the map made, are they a pair, in file
//!   order; with any other number nothing travels, and a gate a script adds joins no pair, until
//!   LUA-33 reads the maps' partner pointers (file order is not the partner pointers on every
//!   map). A destroyed or removed map gate ends travel for the rest of the game (the count is
//!   taken every tick). Grade: pointers read from data; not measured in the original.
//!   A random map places its gates in linked pairs: one pair travels, several do not.
//! - **Partners (Armada I).** Armada I pairs gates by creation order: each new gate pairs with the
//!   one left unpaired, so the 1st pairs with the 2nd, the 3rd with the 4th. Here the
//!   live gates, in id (creation) order, pair in twos; this matches the original until a
//!   paired gate is destroyed (its partner is then re-paired in the original, not here).
//! - **Who goes through.** A craft flying its last leg onto the gate (its target within
//!   [`ON_GATE`] of the centre and no further legs queued: a move ordered onto the gate
//!   entity) enters it; craft passing by, or on an earlier leg of a longer course, do not.
//!   Any team, any craft. Asked before routing and avoidance replace the target with a detour.
//!   Only an unqueued last leg travels: queued legs, patrol, escort and follow orders aimed near
//!   a gate are not carried through, and nothing continues on the far side.
//! - **Transport.** Once it is within its `tooCloseToTurn` plus [`GATE_REACH`] of the
//!   centre (the original queues it there and flies it along an entry spline into the
//!   centre; the spline is not modelled), it is moved to its partner at rest with no
//!   course (an order onto the gate ends idle on the far side, as the original's). The
//!   Armada I profile sets it down [`GATE_REACH`] ahead of the partner along the partner's
//!   facing (not measured). The Armada II family sets it beside the partner's centre, on the
//!   first free ring spot (the original's transport puts it at the centre, then its exit queue
//!   carries it out; read from the executable, static; the spline is not modelled). Never on the
//!   exact centre: a craft parked there would make every later move onto that craft a move onto
//!   the gate. Never outside the map bounds. When no spot qualifies the craft does not travel and
//!   keeps its move, so it may come to rest on the gate's centre (rare).
//!   Gate opening and closing (`AnimationTime`, `ClosedScale`, `OpenScale`, sounds) is
//!   presentation and not modelled; `DisableTimer` is never armed in either game.
//!
//! Generated transwarp endpoints carry an explicit temporary pair. Ordinary
//! map gates and gate goers are still derived from entities and courses.

use sta_content::Profile;
use sta_core::{EntityId, Fixed, Vec3};

use crate::Sim;
use crate::view::UnitFlags;

/// A move whose last point is this close to a gate centre is a move into the gate.
const ON_GATE: i32 = 5;
/// How far from the centre a craft is taken in, beyond its `tooCloseToTurn`, and how far
/// ahead of the exit gate it comes out (the gate's reach; not measured).
const GATE_REACH: i32 = 75;

/// The least step, in units, between craft set down at one gate (STA64's own choice, not
/// measured); a craft's step is twice its body radius, not less than this.
const MIN_SPACING: i32 = 30;
/// The largest ring step (STA64's own choice, not measured), so a set-down stays close to the gate.
const MAX_RING_STEP: i32 = 60;
/// How many candidate exit spots are tried per craft: the ring spots 1 to this (STA64's own
/// choice, not measured). With none free and inside the map, the craft does not travel.
const EXIT_SPOTS: i32 = 64;

const WORMHOLE_CLASS: &str = "wormhole_obj";

fn flat_distance(a: Vec3, b: Vec3) -> Fixed {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

impl Sim {
    /// Live gates in id order, each with its partner, as `(gate, partner)`: Armada I pairs every
    /// gate in creation order; the Armada II family pairs exactly two map gates; nothing else does.
    fn gate_pairs(&self) -> Vec<(EntityId, EntityId)> {
        let Some(db) = self.content.as_deref() else { return Vec::new() };
        let a1 = db.features().a1_compat;
        let family = matches!(db.profile(), Profile::Fo4 | Profile::Fo3 | Profile::Sta2Classic);
        if !a1 && !family {
            return Vec::new();
        }
        let is_gate = |e: &crate::world::Entity| {
            db.unit(e.def).is_some_and(|d| if a1 { d.class.0 == WORMHOLE_CLASS } else { d.class.0.eq_ignore_ascii_case(WORMHOLE_CLASS) })
        };
        let temporary = &self.specials.a2.federation;
        let mut pairs: Vec<_> = temporary.gate_pairs(self.tick).collect();
        let gates: Vec<EntityId> = self
            .units
            .iter()
            .filter(|e| !e.flags.contains(UnitFlags::DYING) && is_gate(e))
            // The Armada II family pairs only the gates the map made, which are the labelled
            // entities (a script's gate has no sim label); a map gate with no label is not
            // counted (none of the stock, Roots and mod maps has one).
            .filter(|e| (a1 || self.labels.get(e.id).is_some()) && !temporary.gate_ends().any(|id| id == e.id))
            .map(|e| e.id)
            .collect();
        // Armada II family: exactly two map gates are a pair; any other number is not paired
        // until LUA-33 carries the partner pointers.
        if !a1 && gates.len() != 2 {
            return pairs;
        }
        pairs.extend(gates.chunks_exact(2).flat_map(|p| [(p[0], p[1]), (p[1], p[0])]));
        pairs.sort_unstable();
        pairs
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
        let a1 = self.content.as_deref().is_some_and(|db| db.features().a1_compat);
        let out = if a1 { reach } else { Fixed::ZERO };
        for &(id, gate, partner) in goers {
            let (Some(g), Some(p)) = (self.units.get(gate), self.units.get(partner)) else { continue };
            let (gate_pos, exit) = (g.pos, p.pos);
            let ahead = p.heading;
            let Some(u) = self.units.get(id) else { continue };
            if u.flags.contains(UnitFlags::DYING) || flat_distance(u.pos, gate_pos) > u.stats.too_close_to_turn + reach {
                continue;
            }
            let mut to = Vec3 { x: exit.x + ahead.0 * out, y: exit.y, z: exit.z + ahead.1 * out };
            if !a1 {
                match self.free_exit_spot(id, exit) {
                    Some(spot) => to = spot,
                    None => continue,
                }
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

    /// The nearest free spot beside `centre` for `id` to be set down, never `centre` itself: the
    /// first of eight directions per ring, rings one step apart (the original queues craft out one
    /// at a time; STA64 sets each down at once, so a craft that finds the mouth held stands beside
    /// it). Free is no other craft within the step and inside the map bounds. `None` when no
    /// candidate qualifies.
    fn free_exit_spot(&self, id: EntityId, centre: Vec3) -> Option<Vec3> {
        let radius2 = self.units.get(id).map_or(Fixed::ZERO, |u| self.body_radius(u.def) * Fixed::from_int(2));
        let step = radius2.max(Fixed::from_int(MIN_SPACING)).min(Fixed::from_int(MAX_RING_STEP));
        // Unit vectors of eight directions, in thousandths.
        const DIRS: [(i32, i32); 8] = [(1000, 0), (707, 707), (0, 1000), (-707, 707), (-1000, 0), (-707, -707), (0, -1000), (707, -707)];
        let inside = |p: Vec3| self.bounds.is_none_or(|(lo, hi)| p.x >= lo.x && p.x <= hi.x && p.z >= lo.z && p.z <= hi.z);
        let held = |p: Vec3| {
            self.units
                .iter()
                .any(|o| o.id != id && o.kind == crate::world::EntityKind::Craft && !o.flags.contains(UnitFlags::DYING) && flat_distance(o.pos, p) < step)
        };
        for k in 1..=EXIT_SPOTS {
            let (ring, dir) = (Fixed::from_int((k - 1) / 8 + 1), DIRS[((k - 1) % 8) as usize]);
            let p = Vec3 {
                x: centre.x + Fixed::from_int(dir.0) / Fixed::from_int(1000) * ring * step,
                y: centre.y,
                z: centre.z + Fixed::from_int(dir.1) / Fixed::from_int(1000) * ring * step,
            };
            if inside(p) && !held(p) {
                return Some(p);
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "wormholes_tests.rs"]
mod tests;
