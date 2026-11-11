//! Global perceived identity, separate from authoritative ownership.

use sta_core::{EntityId, PlayerId, TeamId};

use crate::Sim;
use crate::view::UnitFlags;
use crate::world::{Entities, NEUTRAL, Table};

fn valid_destination(players: &[(PlayerId, TeamId)], to: PlayerId) -> bool {
    to == NEUTRAL || (to.is_player() && (players.is_empty() || players.iter().any(|&(p, _)| p == to)))
}

pub(crate) fn representable(owner: PlayerId) -> bool {
    owner.is_player() || owner == NEUTRAL
}

impl Sim {
    /// Global identity used when another actor evaluates this target's team.
    /// Command authority and resource ownership always use the real owner.
    pub fn perceived_owner(&self, id: EntityId) -> Option<PlayerId> {
        let unit = self.units.get(id)?;
        Some(self.perception.get(id).copied().unwrap_or(unit.owner))
    }

    pub(crate) fn set_perceived_owner(&mut self, id: EntityId, to: PlayerId) -> bool {
        if !valid_destination(&self.players, to) {
            return false;
        }
        let Some(unit) = self.units.get(id) else {
            return false;
        };
        if unit.flags.contains(UnitFlags::DYING) {
            return false;
        }
        if to == unit.owner {
            self.perception.remove(id);
        } else {
            self.perception.insert(id, to);
        }
        true
    }
}

/// Snapshot rows must be canonical; silently repairing corrupt identity would
/// change the saved simulation and conceal a desync.
pub(crate) fn restore(
    units: &Entities,
    rows: Vec<(EntityId, PlayerId)>,
) -> Result<Table<PlayerId>, &'static str> {
    for &(id, to) in &rows {
        let unit = units.get(id).ok_or("perceived owner references missing entity")?;
        // A map's prior real owner may be an unconfigured real slot. Transfer
        // preserves that identity even though explicit writes require a seat.
        if !representable(to) {
            return Err("invalid perceived owner");
        }
        if to == unit.owner {
            return Err("redundant perceived owner");
        }
    }
    Table::from_rows(rows).ok_or("perceived owners out of order")
}
