//! Move orders that make no progress (a stuck detector).
//!
//! Steering, the path planner and the kinematics models each end an order
//! they can serve; this is the safety net under all of them. A craft with a
//! move target keeps a row: the target, the nearest it has come to it and the
//! tick that last improved. When [`LIMIT`] ticks pass without the craft
//! coming [`GAIN`] units nearer than its best, the order ends (target and
//! queued legs cleared, as a Stop does) and [`SimEvent::OrderBlocked`] is
//! raised unless it already was for this order. That covers a craft walled in
//! (held by SIM-22 with its order kept), one that circles a destination it
//! cannot reach, and any deadlock between the models that a later change
//! introduces. A new target restarts the row, so a craft chasing a moving
//! target or flying queued legs never trips it. Craft that are welding are
//! not watched.
//!
//! The rule is STA64's own (the originals' data has no key for it, and no
//! original-game measurement of how a stuck craft ends was made: not
//! measured). It is conservative: two minutes of game time without getting
//! nearer (a way round a field or wall is a few thousand units at most; a
//! longer one is not planned for). Rows restart on every move, stop or
//! formation order, even to the same point. A deploying rig is not watched.
//! Systems that re-aim a craft every tick (attack approach, mining) have
//! their target cleared like any other and may raise the event again after
//! another interval.
//!
//! The rows are bookkeeping that follows from the orders and positions, so
//! they are in snapshots (a restored sim must time out on the same tick) but
//! not in the digest: a divergence shows in positions long before.

use serde::{Deserialize, Serialize};
use sta_core::{EntityId, Fixed, Vec3};

use crate::world::{EntityKind, Table};
use crate::{Sim, SimEvent, TICK_HZ};

/// Ticks without getting nearer before the order ends: two minutes.
const LIMIT: u64 = 120 * TICK_HZ as u64;
/// Units nearer than the best so far that count as progress.
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
            let fresh = Row { target, best: here, since: tick, reported: false };
            let row = match self.watchdog.rows.get_mut(id) {
                Some(r) if r.target == target => r,
                _ => {
                    self.watchdog.rows.insert(id, fresh);
                    continue;
                }
            };
            if here + Fixed::from_int(GAIN) <= row.best {
                row.best = here;
                row.since = tick;
            } else if tick.saturating_sub(row.since) >= LIMIT {
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
