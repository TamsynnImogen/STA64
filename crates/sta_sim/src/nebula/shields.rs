//! Shields after leaving a shield-disabling nebula (SIM-75, slice 2).
//!
//! Rule (Armada II family; every profile but Armada I; the Native profile
//! follows it by STA64's own choice). Grade: measured on stock Armada II 1.1,
//! single runs, two nebula classes; **interim**, because the delay below is a
//! single-run measurement and not a data key (it may derive from a per-class
//! `shieldDelay` key plus a system off-timer: inferred, not proven). That the
//! shields return to the value on entry and not to the maximum, and that
//! recharge then continues from it, is **measured** (stock Armada II 1.1, five
//! partial-shield samples over two classes). The record is taken on the first
//! tick the shields are switched off and kept: not overwritten while the craft
//! stays inside or comes back during the delay.
//! - While a craft (or station, which never leaves) is inside a kind with `disableShields`, its shields are held
//!   at 0 (slice 1) and the value it had on entering is remembered.
//! - After it is out of every such kind, the shields stay at 0 for
//!   [`RETURN_DELAY`], then return in one step to the remembered value (not a
//!   recharge from 0).
//!
//! Edge cases, all **not measured**; the simplest reading of the above:
//! - Back inside during the delay: the remembered value is kept, the delay
//!   starts over at the next leaving.
//! - Entering with the shields already at 0: that 0 is the remembered value,
//!   so nothing comes back (the normal recharge goes on).
//! - Shield generator switched off, or the unit a derelict, when the delay ends:
//!   nothing is restored and the remembered value is dropped. (The original
//!   sometimes gave nothing back at all; a generator below full hit points that
//!   stays off is the likely cause, read from the executable, static, not
//!   measured, and not implemented: here the generator is never switched off by
//!   a nebula.)
//! - Destroyed or removed during the delay: the state goes with it. A change of
//!   owner or a capture does not touch it.
//! - Two overlapping kinds with `disableShields`: it is "out" when out of both.
//! - A maximum that fell meanwhile caps the restored value at it.
//! - Armada I (`Sta1Classic`): none of this; shields recharge from 0 as before
//!   (interim, not measured).
//!
//! The state is in the digest and the snapshot (format 39) once any exists.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, Profile};
use sta_core::{EntityId, Fixed, TICK_HZ, Tick};

use crate::command::Subsystem;
use crate::world::Fnv;
use crate::Sim;

/// How long the shields stay at 0 after the craft is out, in ticks. Measured on
/// stock Armada II 1.1 (single runs, two classes); interim, not a data key.
pub const RETURN_DELAY: Tick = 4 * TICK_HZ as Tick;

/// One craft's remembered shields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Row {
    /// The four pools as they were on entering.
    pub entry: [Fixed; 4],
    /// The first tick the craft was out of every shield-disabling kind; none while inside.
    pub left: Option<Tick>,
}

/// Craft whose shields a nebula took and has yet to give back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NebulaShields {
    rows: BTreeMap<EntityId, Row>,
}

/// The snapshot form of [`NebulaShields`] (format 39).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NebulaShieldsRec {
    pub version: u8,
    /// `(craft, pool bits x4, left + 1 or 0)`, ascending craft.
    pub rows: Vec<(EntityId, [i64; 4], Tick)>,
}

impl NebulaShields {
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn remove_entity(&mut self, id: EntityId) {
        self.rows.remove(&id);
    }

    /// The craft is inside a shield-disabling kind this tick, its pools as they are now.
    pub(crate) fn inside(&mut self, id: EntityId, now: [Fixed; 4]) {
        let row = self.rows.entry(id).or_insert(Row { entry: now, left: None });
        row.left = None;
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        if self.rows.is_empty() {
            return;
        }
        h.str("nebula-shields-v1");
        h.u64(self.rows.len() as u64);
        for (id, r) in &self.rows {
            h.u64(u64::from(id.0));
            for p in r.entry {
                h.u64(p.to_bits() as u64);
            }
            h.u64(r.left.map_or(0, |t| t + 1));
        }
    }

    pub(crate) fn to_rec(&self) -> NebulaShieldsRec {
        NebulaShieldsRec { version: 1, rows: self.rows.iter().map(|(id, r)| (*id, r.entry.map(Fixed::to_bits), r.left.map_or(0, |t| t + 1))).collect() }
    }

    /// Ascending craft that exist; pools in `0..=1_000_000_000` units; no leave tick after `tick`.
    pub(crate) fn from_rec(rec: NebulaShieldsRec, tick: Tick, exists: impl Fn(EntityId) -> bool) -> Option<NebulaShields> {
        const LIMIT: i64 = 1_000_000_000i64 << 32;
        if rec.version != 1 || !rec.rows.windows(2).all(|w| w[0].0 < w[1].0) {
            return None;
        }
        let ok = rec.rows.iter().all(|(id, pools, left)| exists(*id) && pools.iter().all(|p| (0..=LIMIT).contains(p)) && left.saturating_sub(1) <= tick);
        ok.then(|| NebulaShields {
            rows: rec.rows.into_iter().map(|(id, p, left)| (id, Row { entry: p.map(Fixed::from_bits), left: left.checked_sub(1) })).collect(),
        })
    }
}

impl Sim {
    /// Before the tick's recharge: remembers the shields of every craft inside a
    /// shield-disabling kind (the value on entry is the one before this tick's
    /// recharge). Returns those craft, ascending.
    pub(crate) fn nebula_shields_enter(&mut self, db: &ContentDb, inside: &super::Inside) -> Vec<EntityId> {
        let mut now = Vec::new();
        for (id, keys) in &inside.rows {
            if !keys.iter().filter_map(|k| super::effects(db, *k)).any(|e| e.disable_shields) {
                continue;
            }
            let Some(u) = self.units.get(*id).filter(|u| super::affected(u)) else { continue };
            now.push(*id);
            if db.profile() != Profile::Sta1Classic {
                self.nebula_shields.inside(*id, u.shields.now);
            }
        }
        now
    }

    /// After the nebula effects of the tick: craft that are out of every
    /// shield-disabling kind (`inside` lists those still in, ascending) wait,
    /// then get their entry value back.
    pub(crate) fn nebula_shields_tail(&mut self, inside: &[EntityId]) {
        if self.nebula_shields.is_empty() {
            return;
        }
        let tick = self.tick;
        let ids: Vec<EntityId> = self.nebula_shields.rows.keys().copied().filter(|id| inside.binary_search(id).is_err()).collect();
        for id in ids {
            if !self.units.get(id).is_some_and(super::affected) {
                self.nebula_shields.remove_entity(id);
                continue;
            }
            let Some(row) = self.nebula_shields.rows.get_mut(&id) else { continue };
            let left = *row.left.get_or_insert(tick);
            let entry = row.entry;
            if tick < left.saturating_add(RETURN_DELAY) {
                if let Some(u) = self.units.get_mut(id) {
                    u.shields.now = [Fixed::ZERO; 4];
                }
                continue;
            }
            self.nebula_shields.remove_entity(id);
            let derelict = self.subsystems.get(id).is_some_and(|r| r.derelict);
            let on = self.subsystem_state(id, Subsystem::Shields).is_some_and(|(on, _)| on);
            if let Some(u) = self.units.get_mut(id).filter(|_| on && !derelict) {
                let pools = if u.shields.directional { 4 } else { 1 };
                let max = u.shields.max;
                for (now, was) in u.shields.now[..pools].iter_mut().zip(entry) {
                    *now = was.min(max);
                }
            }
        }
    }
}
