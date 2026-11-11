//! SIM-19: guarded ownership edits through the shared simulation.
//!
//! Station queues have checked cleanup, including a paid active head.
//! Rig cleanup and typed planet roles share the same transaction. Direct
//! unfinished-child transfer remains unsupported; see OWNERSHIP_TRANSFER.md.

use sta_core::{EntityId, Fixed, PlayerId};

use crate::Sim;
use crate::view::UnitFlags;
use crate::world::{EntityKind, NEUTRAL};

impl Sim {
    pub(crate) fn transfer_owner(&mut self, from: PlayerId, id: EntityId, to: PlayerId) -> bool {
        if from != PlayerId::SCRIPT && from != PlayerId::BENCH {
            return false;
        }
        let destination = to == NEUTRAL
            || (to.is_player() && (self.players.is_empty() || self.players.iter().any(|&(p, _)| p == to)));
        if !destination || self.victory.is_eliminated(to) {
            return false;
        }
        let Some(unit) = self.units.get(id) else {
            return false;
        };
        let def = self.content.as_deref().and_then(|db| db.unit(unit.def));
        let planet = def.is_some_and(|d| d.planet_role.is_some());
        let background = def.is_some_and(|d| d.planet_role == Some(sta_content::PlanetRole::Background));
        if (!planet && !matches!(unit.kind, EntityKind::Craft | EntityKind::Station))
            || (unit.hull[0] <= Fixed::ZERO && !(background && unit.hull == [Fixed::ZERO; 2]))
            || unit.flags.contains(UnitFlags::DYING)
            || unit.progress.is_some()
            || (!planet && def.is_some_and(|def| def.resource.is_some()))
        {
            return false;
        }
        let Some(production) =
            self.production
                .prepare_transfer(self.content.as_deref(), &self.economy, &self.units, id, unit)
        else {
            return false;
        };

        // All rejection checks precede cleanup, including same-owner requests.
        // Consume the checked production plan; ordinary Stop/cancel has a separate
        // refund contract, particularly for deferred active production.
        let children = self.production.finish_transfer(&mut self.economy, production);
        for child in children {
            self.despawn(child, None);
        }
        self.weapons.stop_attack(&self.units, from, &[id]);
        self.economy.cancel(&self.units, from, &[id]);
        self.movement.clear(id);
        self.production.clear_builder_rally(id);
        let unit = self.units.get_mut(id).expect("validated live unit");
        unit.target = None;
        unit.owner = to;
        true
    }

    /// Planet contribution only, not total team crew capacity or crew stock.
    /// Derived from immutable definitions and current owners, so repeated swaps
    /// cannot double-credit it and snapshots need no mutable accumulator.
    pub fn planet_crew_capacity(&self, player: PlayerId) -> i64 {
        if !player.is_player() {
            return 0;
        }
        let Some(db) = self.content.as_deref() else { return 0 };
        self.units
            .iter()
            .filter(|u| u.owner == player)
            .filter_map(|u| db.unit(u.def))
            .filter(|d| d.planet_role == Some(sta_content::PlanetRole::Producer))
            .map(|d| i64::from(d.planet_crew_gain))
            .sum()
    }
}

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ownership_queue_tests.rs"]
mod queue_tests;

#[cfg(test)]
#[path = "ownership_completion_tests.rs"]
mod completion_tests;
