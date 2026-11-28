//! The dying phase (issue #1100, `Features::dying_phase`, on for stock Armada II).
//!
//! A craft or station whose hull reaches zero from hits (weapons, the nebula, a script's
//! damage) is not removed at once. It enters a dying phase and is removed, with
//! [`SimEvent::Destroyed`], on the tick the phase ends. Rules and grades:
//!
//! - Length: [`DYING_STANDING_TICKS`] when its speed at hull zero is at most
//!   [`DYING_MOVING_SPEED`], else [`DYING_MOVING_TICKS`]; removal comes [`DYING_LATE_TICKS`]
//!   after that (measured, repeated). Craft and stations alike (read from the executable,
//!   static).
//! - Each further hit that lands during the phase does no damage (the hull is at zero, so
//!   `damage::apply` sends it with a zero split) and takes [`DYING_HIT_STEP_TICKS`] off what
//!   is left; a hit with less than that left removes the unit on the next tick (measured,
//!   repeated). A hit that lands on the tick the hull reaches zero does not shorten the phase
//!   (read from the executable, static; not measured).
//! - The unit is flagged `DYING` at once and [`SimEvent::Dying`] is sent with the first lethal
//!   hitter; `Destroyed` at removal names the same one.
//! - Its orders end and it takes no new ones (read from the executable, static); its attack
//!   ends, it fires nothing (read from the executable, static), and its beams in flight are lost
//!   (measured, repeated; `weapons::beam_holds`). No shooter picks it as a new target and an
//!   attack ordered on it ends (measured once: no new shot at it); shots and beams already on
//!   their way still land (measured, repeated).
//! - It drifts on its heading without thrust or turning, its speed decaying by
//!   [`DYING_DRIFT_DECAY`] (measured, repeated).
//! - Other craft do not collide with it (measured, repeated); see `avoid::separate_units`.
//! - It is not selectable in the view (a selected craft is deselected at hull zero: measured,
//!   once).
//! - Victory and elimination are counted at removal (read from the executable, static).
//! - A scripted kill (`Edit::Kill`) removes at once, also during the phase (read from the
//!   executable, static; seen on the bench).
//!
//! INTERIM (not settled; STA64's behaviour of today, where the phase changes nothing):
//! - a unit set to zero hull by a script (`Edit::Hull`) and the kills of special weapons,
//!   self-destruct, decommission, mines and freighters are removed at once;
//! - a dying unit stays a solid for craft when it is a station, and is still pushed out of
//!   solids itself;
//! - the speed is STA64's speed along the heading, read at hull zero;
//! - every `Hit` on a dying unit counts as a hit, whatever its weapon;
//! - a dying yard's build queue holds (as in stasis) until removal;
//! - the `isHero` class key, which gives the long phase whatever the speed (read from the
//!   executable, static), is not read: every unit follows its speed;
//! - the late ticks are counted in STA64 ticks, as many as the bench counted at its finer tick;
//! - a unit killed at warp drifts from its impulse speed (dying was measured at impulse-range
//!   speeds only).

use serde::{Deserialize, Serialize};
use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, Tick};

use crate::command::Order;
use crate::event::SimEvent;
use crate::view::UnitFlags;
use crate::world::{Entities, EntityKind, Fnv, Table};
use crate::Sim;

/// Phase length of a unit at rest or slow at hull zero (measured, repeated).
pub const DYING_STANDING_TICKS: Tick = 3 * TICK_HZ as Tick;
/// Phase length of a unit faster than [`DYING_MOVING_SPEED`] at hull zero (measured, repeated).
pub const DYING_MOVING_TICKS: Tick = 5 * TICK_HZ as Tick;
/// Removal comes this many ticks after the phase length (measured, repeated). INTERIM: the bench
/// counted them at a finer tick than STA64's and attributes them to counting and rounding; the
/// same count of STA64 ticks is a longer time.
pub const DYING_LATE_TICKS: Tick = 2;
/// What each further hit takes off the phase (measured, repeated).
pub const DYING_HIT_STEP_TICKS: Tick = TICK_HZ as Tick / 2;
/// A unit faster than this, in units per second, takes the long phase (read from the
/// executable, static; both sides of it measured).
pub const DYING_MOVING_SPEED: Fixed = Fixed::ONE;
/// The dying drift's speed decay, a share of the speed per second (measured, repeated: a fit
/// to a roughly exponential decay).
pub const DYING_DRIFT_DECAY: Fixed = Fixed::from_bits(19 * (1i64 << 32) / 100);

/// One unit in its dying phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Row {
    /// The tick it is removed on (at the start of that tick's step).
    pub(crate) ends: Tick,
    /// The first lethal hitter, named again by `Destroyed`.
    pub(crate) by: Option<EntityId>,
}

/// The units in their dying phase, by id. Saved, and hashed once any exists.
#[derive(Debug, Clone, Default)]
pub(crate) struct Dying {
    rows: Table<Row>,
}

impl Dying {
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn contains(&self, id: EntityId) -> bool {
        self.rows.get(id).is_some()
    }

    pub(crate) fn remove(&mut self, id: EntityId) {
        self.rows.remove(id);
    }

    /// The tick `id` is removed on, while it is dying.
    pub(crate) fn ends(&self, id: EntityId) -> Option<Tick> {
        self.rows.get(id).map(|r| r.ends)
    }

    pub(crate) fn to_rows(&self) -> Vec<(EntityId, Row)> {
        self.rows.iter().map(|(id, r)| (id, *r)).collect()
    }

    /// The rows a snapshot taken at the end of a step holds (`tick` is the next tick):
    /// ascending, each of a craft or station flagged `DYING`, none ended before `tick`.
    /// `None` otherwise. (Its hull is not tested: a bench `Heal` may refill a dying unit.)
    pub(crate) fn from_rows(rows: Vec<(EntityId, Row)>, tick: Tick, units: &Entities) -> Option<Dying> {
        let valid = rows.iter().all(|(id, r)| {
            r.ends >= tick
                && units.get(*id).is_some_and(|u| {
                    matches!(u.kind, EntityKind::Craft | EntityKind::Station) && u.flags.contains(UnitFlags::DYING)
                })
        });
        valid.then(|| Table::from_rows(rows)).flatten().map(|rows| Dying { rows })
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("dying-v1");
        h.u64(self.rows.len() as u64);
        for (id, r) in self.rows.iter() {
            h.u64(id.0 as u64);
            h.u64(r.ends);
            h.u64(r.by.map_or(0, |b| u64::from(b.0) + 1));
        }
    }
}

/// Phase length plus the late ticks, by the unit's speed at hull zero.
pub(crate) fn phase_ticks(speed: Fixed) -> Tick {
    let length = if speed.abs() > DYING_MOVING_SPEED { DYING_MOVING_TICKS } else { DYING_STANDING_TICKS };
    length + DYING_LATE_TICKS
}

/// What an order becomes once the dying units it names are taken out.
pub(crate) enum Living {
    /// It names no dying unit.
    All,
    /// It names some: this is the order without them.
    Some(Order),
    /// It names only dying units: refused.
    None,
}

/// The units an order is given to.
enum Actors<'a> {
    List(&'a mut Vec<EntityId>),
    One(EntityId),
}

fn actors(order: &mut Order) -> Actors<'_> {
    use Order::*;
    match order {
        Move { units, .. }
        | Stop { units }
        | Attack { units, .. }
        | Formation { units, .. }
        | Special { units, .. }
        | Cloak { units, .. }
        | Mine { units, .. }
        | Autonomy { units, .. }
        | Patrol { units, .. }
        | SearchAndDestroy { units }
        | Scout { units }
        | AttackAny { units }
        | AttackPoint { units, .. }
        | MoveWarp { units, .. }
        | Repair { units, .. }
        | Colonize { units, .. }
        | Transport { units, .. }
        | Recrew { units, .. }
        | Guard { units, .. }
        | Decommission { units, .. }
        | Alert { units, .. }
        | Trade { units, .. } => Actors::List(units),
        Rally { builders, .. } => Actors::List(builders),
        Build { builder, .. } | CancelBuild { builder, .. } => Actors::One(*builder),
    }
}

impl Sim {
    /// Whether `id` is in its dying phase.
    pub fn is_dying(&self, id: EntityId) -> bool {
        self.dying.contains(id)
    }

    /// The tick `id` is removed on, while it is dying.
    pub fn dying_ends(&self, id: EntityId) -> Option<Tick> {
        self.dying.ends(id)
    }

    /// A dying unit takes no orders: `order` without the dying units it names.
    pub(crate) fn order_for_the_living(&self, order: &Order) -> Living {
        if self.dying.is_empty() {
            return Living::All;
        }
        let mut order = order.clone();
        match actors(&mut order) {
            Actors::One(id) if self.dying.contains(id) => Living::None,
            Actors::One(_) => Living::All,
            Actors::List(units) => {
                let before = units.len();
                units.retain(|id| !self.dying.contains(*id));
                if units.len() == before {
                    Living::All
                } else if units.is_empty() {
                    Living::None
                } else {
                    Living::Some(order)
                }
            }
        }
    }

    /// Start the dying phase of those `killed` (unit, first lethal hitter; in kill order) whose
    /// definition has `dying_phase`, and return the others, which are removed at once.
    pub(crate) fn start_dying(&mut self, db: &ContentDb, killed: Vec<(EntityId, Option<EntityId>)>) -> Vec<(EntityId, Option<EntityId>)> {
        let mut now = Vec::new();
        for (id, by) in killed {
            let Some(u) = self.units.get(id) else {
                now.push((id, by));
                continue;
            };
            if self.dying.contains(id) {
                continue;
            }
            let phased = matches!(u.kind, EntityKind::Craft | EntityKind::Station)
                && db.unit(u.def).is_some_and(|d| db.unit_features(d).dying_phase);
            if !phased {
                now.push((id, by));
                continue;
            }
            let ends = self.tick.saturating_add(phase_ticks(u.speed));
            self.dying.rows.insert(id, Row { ends, by });
            let u = self.units.get_mut(id).expect("present");
            u.flags = u.flags.with(UnitFlags::DYING, true);
            u.target = None;
            u.omega = Fixed::ZERO;
            // INTERIM: a unit killed at warp drifts from its impulse speed, not its warp speed;
            // dying was measured at impulse-range speeds only.
            u.speed = u.speed.min(u.stats.max_speed);
            // Its orders end: attack, scripted job, course, mining and the stuck-order watch.
            self.weapons.stop_attack(&self.units, PlayerId::SCRIPT, &[id]);
            self.scripted.remove(id);
            self.movement.clear(id);
            self.approach_warp.remove(id);
            self.watchdog.remove(id);
            self.economy.cancel(&self.units, PlayerId::SCRIPT, &[id]);
            self.events.push(SimEvent::Dying { id, by });
        }
        now
    }

    /// A script's lethal damage (`Edit::Damage`): the dying phase under the switch, else
    /// removal at once. False when nothing happened.
    pub(crate) fn kill_by_damage(&mut self, id: EntityId) -> bool {
        let Some(db) = self.content.clone() else { return self.despawn(id, None) };
        let now = self.start_dying(&db, vec![(id, None)]);
        match now.first() {
            Some(&(id, by)) => self.despawn(id, by),
            None => true,
        }
    }

    /// Every `Hit` in `self.events[from..]` on a unit that was already dying takes
    /// [`DYING_HIT_STEP_TICKS`] off its phase, never past the next tick. Call before this
    /// tick's kills start their phases.
    pub(crate) fn shorten_dying(&mut self, from: usize) {
        if self.dying.is_empty() {
            return;
        }
        let next = self.tick + 1;
        for e in &self.events[from..] {
            if let SimEvent::Hit { target, .. } = e
                && let Some(r) = self.dying.rows.get_mut(*target)
            {
                r.ends = r.ends.saturating_sub(DYING_HIT_STEP_TICKS).max(next);
            }
        }
    }

    /// Remove the units whose phase ends on this tick, ascending id, with `Destroyed` naming
    /// their killer; returns their owners for the elimination check.
    pub(crate) fn remove_dead(&mut self) -> Vec<PlayerId> {
        if self.dying.is_empty() {
            return Vec::new();
        }
        let due: Vec<(EntityId, Option<EntityId>)> =
            self.dying.rows.iter().filter(|(_, r)| r.ends <= self.tick).map(|(id, r)| (id, r.by)).collect();
        let mut owners = Vec::new();
        for (id, by) in due {
            owners.extend(self.units.get(id).map(|u| u.owner));
            self.despawn(id, by);
        }
        owners
    }

    /// Move each dying craft along its heading at its decaying speed, without turning.
    pub(crate) fn drift_dying(&mut self) {
        if self.dying.is_empty() {
            return;
        }
        let hz = Fixed::from_int(TICK_HZ as i32);
        let obstacles = if self.confinement.is_active() { self.obstacles() } else { Vec::new() };
        let ids: Vec<EntityId> = self.dying.rows.iter().map(|(id, _)| id).collect();
        for id in ids {
            let radius = self.units.get(id).map_or(Fixed::ZERO, |u| self.full_radius(u.def) * Fixed::from_int(3) / Fixed::from_int(4));
            let dock = self.economy.trade_dock_of(id);
            let Some(u) = self.units.get_mut(id).filter(|u| u.kind == EntityKind::Craft || u.mobile_station) else { continue };
            let previous = u.pos;
            u.omega = Fixed::ZERO;
            u.pos.x = u.pos.x + u.heading.0 * u.speed / hz;
            u.pos.z = u.pos.z + u.heading.1 * u.speed / hz;
            u.speed = u.speed - u.speed * DYING_DRIFT_DECAY / hz;
            if self.confinement.is_active() && u.pos != previous {
                crate::avoid::constrain_position(&mut self.confinement, u, previous, radius, &obstacles, dock);
            }
        }
    }
}
