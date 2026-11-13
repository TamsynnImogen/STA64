//! Craft autonomy state (SIM-38, the seam for SIM-36). Implements D5, D9.
//!
//! Every craft has a movement autonomy and a special-weapon autonomy, each
//! 0 (low), 1 (medium) or 2 (high), as the original engine reads them. Only
//! levels a player or script has set are stored; any other craft reads the
//! default. Until CON-46 types the `RTS_CFG.h` defaults
//! (`CRAFT_DEFAULT_MOVEMENT_AUTONOMY`, `CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY`)
//! the default is the engine's own for an absent key: movement 1, special
//! weapon 0 (read from the executable, static; the installs set 1 and 2 on Roots and FO 4.0, 1 and 1
//! on Classic Enhanced, which this seam does not read yet).
//!
//! Invariant: rows are relative to [`ENGINE_DEFAULT`], and an absent row
//! always means that constant, in the digest, snapshots and replays alike. When
//! CON-46 reads a profile default from `RTS_CFG.h`, spawning applies it by
//! writing explicit rows for the crafts that differ from the constant; it never
//! changes what "no row" means. A row equal to the constant is never stored
//! (`order` removes it, `from_rec` rejects it), so one behaviour has one digest.
//!
//! This module only holds the state and the order. What autonomy changes
//! (assist, targets of opportunity) is SIM-36's. In the digest and snapshot
//! only once a level has been set.

use serde::{Deserialize, Serialize};
use sta_core::EntityId;

use crate::command::{AutonomyKind, AutonomyLevel};
use crate::world::{Entities, Fnv, Table};

/// Movement 1 (medium), special weapon 0 (low): the engine's defaults for an
/// absent `RTS_CFG.h` key, until CON-46.
pub const ENGINE_DEFAULT: [u8; 2] = [1, 0];

fn level(l: AutonomyLevel) -> u8 {
    match l {
        AutonomyLevel::Low => 0,
        AutonomyLevel::Medium => 1,
        AutonomyLevel::High => 2,
    }
}

/// The levels that were set, by craft; `[movement, special weapon]`.
#[derive(Debug, Clone, Default)]
pub struct Autonomy {
    set: Table<[u8; 2]>,
}

/// [`Autonomy`] as the snapshot stores it: ascending by id.
pub(crate) type AutonomyRec = Vec<(EntityId, [u8; 2])>;

impl Autonomy {
    pub(crate) fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// `[movement, special weapon]` of `id`, 0..=2 each.
    pub fn levels(&self, id: EntityId) -> [u8; 2] {
        self.set.get(id).copied().unwrap_or(ENGINE_DEFAULT)
    }

    #[cfg(test)]
    pub(crate) fn set_for_test(&mut self, id: EntityId, levels: [u8; 2]) {
        self.set.insert(id, levels);
    }

    pub(crate) fn remove(&mut self, id: EntityId) {
        self.set.remove(id);
    }

    /// An `Order::Autonomy`: every listed craft `player` may order (any, for
    /// scripts and the bench) takes the level. True when at least one did.
    pub(crate) fn order(
        &mut self,
        units: &Entities,
        player: sta_core::PlayerId,
        ids: &[EntityId],
        kind: AutonomyKind,
        to: AutonomyLevel,
    ) -> bool {
        let slot = match kind {
            AutonomyKind::Movement => 0,
            AutonomyKind::SpecialWeapon => 1,
        };
        let mut any = false;
        for &id in ids {
            let ok = units.get(id).is_some_and(|u| {
                u.kind == crate::world::EntityKind::Craft
                    && u.hull[0] > sta_core::Fixed::ZERO
                    && (!player.is_player() || u.owner == player)
            });
            if !ok {
                continue;
            }
            let mut now = self.levels(id);
            now[slot] = level(to);
            if now == ENGINE_DEFAULT {
                self.set.remove(id);
            } else {
                self.set.insert(id, now);
            }
            any = true;
        }
        any
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("autonomy-v1");
        h.u64(self.set.len() as u64);
        for (id, l) in self.set.iter() {
            h.u64(id.0 as u64);
            h.u64(l[0] as u64);
            h.u64(l[1] as u64);
        }
    }

    pub(crate) fn to_rec(&self) -> AutonomyRec {
        self.set.iter().map(|(id, l)| (id, *l)).collect()
    }

    /// `None` when rows are unordered, a level is above 2 or a row equals
    /// [`ENGINE_DEFAULT`] (never stored, see the module header).
    pub(crate) fn from_rec(rows: AutonomyRec) -> Option<Autonomy> {
        if rows.iter().any(|(_, l)| l.iter().any(|v| *v > 2) || *l == ENGINE_DEFAULT) {
            return None;
        }
        Some(Autonomy { set: Table::from_rows(rows)? })
    }
}

/// Wire form of a level, for the snapshot record's version byte checks.
#[derive(Serialize, Deserialize)]
pub(crate) struct AutonomyWire {
    pub version: u8,
    pub rows: AutonomyRec,
}

#[cfg(test)]
mod tests;
