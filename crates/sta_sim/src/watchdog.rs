//! Move orders that make no progress (a stuck detector).
//!
//! Steering, the path planner and the kinematics models each end an order
//! they can serve; this is the safety net under all of them. A craft with a
//! move target keeps a row and an order ends, with its target and queued legs
//! cleared (as a Stop does) and [`SimEvent::OrderBlocked`] raised unless it
//! already was, on either of two counts:
//!
//! - **still**: the craft has not moved [`GAIN`] units from where it was for
//!   [`STILL`] ticks (two minutes): walled in, held by SIM-22, or a deadlock
//!   between the models;
//! - **going round**: the craft has moved but come no [`GAIN`] units nearer
//!   than its best for as long as it takes to fly [`TRAVEL`] units at its
//!   own top speed (never less than [`STILL`]): an orbit round a destination
//!   it cannot reach, or a detour that never ends. [`TRAVEL`] is above the
//!   longest wall of blocking fields in the stock Fleet Operations and Roots
//!   maps (36 000 units, `nirvana2`: measured from the maps' objects), so a
//!   craft flying round a whole wall is not cut off.
//!
//! A new target restarts the row, so a craft chasing a moving target or
//! flying queued legs never trips it; so does every move, stop or formation
//! order, even to the same point. Welding builders and deploying rigs are not
//! watched. Systems that re-aim a craft every tick (attack approach, mining)
//! have their target cleared like any other and may raise the event again
//! after another interval.
//!
//! The rule is STA64's own (the originals' data has no key for it, and no
//! original-game measurement of how a stuck craft ends was made: not
//! measured).
//!
//! The rows are bookkeeping that follows from the orders and positions, so
//! they are in snapshots (a restored sim must time out on the same tick) but
//! not in the digest: a divergence shows in positions long before.

use serde::{Deserialize, Serialize};
use sta_core::{EntityId, Fixed, Vec3};

use crate::world::{EntityKind, Table};
use crate::{Sim, SimEvent, TICK_HZ};

/// Ticks a craft may stand still on a live move order: two minutes.
const STILL: u64 = 120 * TICK_HZ as u64;
/// Units of flight without getting nearer that a craft is given.
const TRAVEL: u64 = 40_000;
/// Units that count as moved, or as nearer than the best so far.
const GAIN: i32 = 8;

/// One craft's watch on its current move order.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct Row {
    /// The order's target; another one starts a new row.
    pub target: Vec3,
    /// The nearest the craft has been to it (`distance`).
    pub best: Fixed,
    /// The tick that set `best`.
    pub since: u64,
    /// `OrderBlocked` was already raised for this order.
    pub reported: bool,
    /// Where the craft last moved [`GAIN`] units from, and the tick it got there.
    pub anchor: Vec3,
    pub anchored: u64,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct Watchdog {
    rows: Table<Row>,
}

/// An overflow-free stand-in for the flat distance: the larger offset plus
/// half the smaller (within 12% of it), monotone in the way that matters.
fn distance(a: Vec3, b: Vec3) -> Fixed {
    let (dx, dz) = ((a.x - b.x).abs(), (a.z - b.z).abs());
    dx.max(dz) + dx.min(dz) / Fixed::from_int(2)
}

impl Watchdog {
    pub(crate) fn remove(&mut self, id: EntityId) {
        self.rows.remove(id);
    }

    pub(crate) fn to_rec(&self) -> Vec<(EntityId, Row)> {
        self.rows.iter().map(|(id, r)| (id, *r)).collect()
    }

    pub(crate) fn from_rec(rows: Vec<(EntityId, Row)>) -> Option<Watchdog> {
        Some(Watchdog { rows: Table::from_rows(rows)? })
    }

    /// Mark the order of `id` as reported; true when it was not before.
    /// (Not keyed on the target: the planner may have swapped in an aim point.)
    pub(crate) fn report(&mut self, id: EntityId) -> bool {
        match self.rows.get_mut(id) {
            Some(r) if !r.reported => {
                r.reported = true;
                true
            }
            _ => false,
        }
    }
}

impl Sim {
    /// Start of every tick: update the rows and end the orders that timed out.
    pub(crate) fn watch_orders(&mut self) {
        let tick = self.tick;
        let mut ended = Vec::new();
        let ids: Vec<EntityId> = self.units.iter().map(|u| u.id).collect();
        for id in ids {
            let Some(u) = self.units.get(id) else { continue };
            let watched = u.kind == EntityKind::Craft && u.mobile() && !self.production.is_welding(id) && !self.production.is_deploying(id);
            let Some(target) = u.target.filter(|_| watched) else {
                self.watchdog.remove(id);
                continue;
            };
            let here = distance(u.pos, target);
            let fresh = Row { target, best: here, since: tick, reported: false, anchor: u.pos, anchored: tick };
            let (pos, speed) = (u.pos, u.stats.max_speed);
            let row = match self.watchdog.rows.get_mut(id) {
                Some(r) if r.target == target => r,
                _ => {
                    self.watchdog.rows.insert(id, fresh);
                    continue;
                }
            };
            if distance(pos, row.anchor) >= Fixed::from_int(GAIN) {
                row.anchor = pos;
                row.anchored = tick;
            }
            // Ticks to fly TRAVEL units at top speed (speed > 0: mobile).
            let travel = ((TRAVEL as u128) << 32) * TICK_HZ as u128 / (speed.to_bits().max(1) as u128);
            let going_round = travel.min(u64::MAX as u128) as u64;
            if here + Fixed::from_int(GAIN) <= row.best {
                row.best = here;
                row.since = tick;
            } else if tick.saturating_sub(row.anchored) >= STILL
                || tick.saturating_sub(row.since) >= going_round.max(STILL)
            {
                ended.push((id, row.reported));
            }
        }
        for (id, reported) in ended {
            self.watchdog.remove(id);
            self.movement.clear_waypoints(id);
            if let Some(u) = self.units.get_mut(id) {
                u.target = None;
            }
            if !reported {
                self.events.push(SimEvent::OrderBlocked { id });
            }
        }
    }
}
