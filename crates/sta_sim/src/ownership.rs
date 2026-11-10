//! SIM-19: guarded ownership edits through the shared simulation.
//!
//! Active production and unfinished units remain rejected. Entirely
//! unstarted station queues have checked cleanup; see OWNERSHIP_TRANSFER.md.

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
            || (to.is_player()
                && (self.players.is_empty() || self.players.iter().any(|&(p, _)| p == to)));
        if !destination || self.victory.is_eliminated(to) {
            return false;
        }
        let Some(unit) = self.units.get(id) else {
            return false;
        };
        if !matches!(unit.kind, EntityKind::Craft | EntityKind::Station)
            || unit.hull[0] <= Fixed::ZERO
            || unit.flags.contains(UnitFlags::DYING)
            || unit.progress.is_some()
            || self
                .content
                .as_deref()
                .and_then(|db| db.unit(unit.def))
                .is_some_and(|def| def.resource.is_some())
        {
            return false;
        }
        let Some(production) =
            self.production
                .prepare_transfer(self.content.as_deref(), &self.economy, id, unit)
        else {
            return false;
        };

        // All rejection checks precede cleanup, including same-owner requests.
        // This consumes the checked unstarted-queue plan; ordinary Stop/cancel
        // has a separate contract, particularly for active production.
        self.production
            .finish_transfer(&mut self.economy, production);
        self.weapons.stop_attack(&self.units, from, &[id]);
        self.economy.cancel(&self.units, from, &[id]);
        self.movement.clear(id);
        self.production.clear_builder_rally(id);
        let unit = self.units.get_mut(id).expect("validated live unit");
        unit.target = None;
        unit.owner = to;
        true
    }
}

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ownership_queue_tests.rs"]
mod queue_tests;
