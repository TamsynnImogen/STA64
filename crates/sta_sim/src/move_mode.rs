//! The move mode a move order carries (SIM-76 seam, ADR 0011 G4).
//!
//! `Order::Move` names a [`MoveMode`] or leaves it to the content's default
//! (`cfgDEFAULT_TO_FORMATION_MOVE`, `DEFAULT_TO_FORMATION_MOVE` in Armada I,
//! resolved per profile by `sta_content::GroupRules`; read from data). This
//! module keeps what the last move order of each unit named, so it survives
//! the queue, a snapshot and a replay, and answers which mode a unit's move
//! runs under. Nothing steers by it yet: SIM-76's next PR (the group record) and
//! SIM-77 and SIM-79 read it. Evidence for the two modes: read from the
//! executable, static, not measured (TOOL-26).
//!
//! - A move order sets the mode of the units that took it (`None` removes the
//!   row: the content's default); `Stop` removes it. A queued move replaces
//!   the unit's row too: it is the mode of the last move order, not of each leg.
//! - Rows of units that are gone are pruned each tick; the digest, the section
//!   digest and the snapshot cover only live units' rows, and only once a row
//!   exists, so a sim whose orders never named a mode hashes as before.

use sta_core::{EntityId, PlayerId};

use crate::command::MoveMode;
use crate::world::{Fnv, Table};
use crate::Sim;

/// The explicit move mode of each unit's last move order, ascending by id.
#[derive(Debug, Clone, Default)]
pub struct MoveModes {
    rows: Table<MoveMode>,
}

impl MoveModes {
    pub(crate) fn set(&mut self, id: EntityId, mode: Option<MoveMode>) {
        match mode {
            Some(m) => {
                self.rows.insert(id, m);
            }
            None => {
                self.rows.remove(id);
            }
        }
    }

    pub(crate) fn get(&self, id: EntityId) -> Option<MoveMode> {
        self.rows.get(id).copied()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The rows of the units `alive` accepts, ascending by id.
    pub(crate) fn rows(&self, alive: impl Fn(EntityId) -> bool) -> Vec<(EntityId, MoveMode)> {
        self.rows.iter().filter(|(id, _)| alive(*id)).map(|(id, m)| (id, *m)).collect()
    }

    pub(crate) fn from_rows(rows: Vec<(EntityId, MoveMode)>) -> Option<MoveModes> {
        Some(MoveModes { rows: Table::from_rows(rows)? })
    }

    pub(crate) fn hash_rows(rows: &[(EntityId, MoveMode)], h: &mut Fnv) {
        if rows.is_empty() {
            return;
        }
        h.str("move modes");
        h.u64(rows.len() as u64);
        for (id, mode) in rows {
            h.u64(id.0 as u64);
            h.u64(match mode {
                MoveMode::Formation => 1,
                MoveMode::Group => 2,
            });
        }
    }
}

impl Sim {
    /// The live units' saved rows.
    pub(crate) fn move_mode_rows(&self) -> Vec<(EntityId, MoveMode)> {
        self.move_modes.rows(|id| self.units.get(id).is_some())
    }

    pub(crate) fn hash_move_modes(&self, h: &mut Fnv) {
        if !self.move_modes.is_empty() {
            MoveModes::hash_rows(&self.move_mode_rows(), h);
        }
    }

    /// Record the mode of a move order for the units that took it. They are the
    /// ones `apply_movement` accepts: orderable, and the player's own.
    pub(crate) fn record_move_mode(&mut self, player: PlayerId, ids: &[EntityId], mode: Option<MoveMode>) {
        for &id in ids {
            if self.unit(id).is_some_and(|u| u.orderable() && (!player.is_player() || u.owner == player)) {
                self.move_modes.set(id, mode);
            }
        }
    }

    pub(crate) fn clear_move_modes(&mut self, ids: &[EntityId]) {
        for &id in ids {
            self.move_modes.set(id, None);
        }
    }

    /// Rows of units that are gone, dropped (run once per tick).
    pub(crate) fn prune_move_modes(&mut self) {
        if self.move_modes.is_empty() {
            return;
        }
        let live = self.move_mode_rows();
        self.move_modes = MoveModes::from_rows(live).unwrap_or_default();
    }

    /// The move mode `id`'s current move runs under: the mode its order named,
    /// else the content's default for the profile (ADR 0011 G4). `None` for a unit
    /// with nothing to fly to, and for a sim without content (no rules to read).
    /// Nothing steers by it yet (SIM-76 seam).
    pub fn move_mode(&self, id: EntityId) -> Option<MoveMode> {
        let db = self.content.as_ref()?;
        let unit = self.unit(id)?;
        let moving = unit.target.is_some() || self.movement.course(id).is_some_and(|c| !c.waypoints.is_empty());
        moving.then(|| MoveMode::resolve(self.move_modes.get(id), &db.group_rules()))
    }
}
