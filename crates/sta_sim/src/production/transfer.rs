//! Capture cleanup for entirely unstarted station queues (SIM-19).

use super::Production;
use crate::economy::{Economy, RESOURCES};
use crate::world::{Entity, EntityKind};
use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId};

/// Validated, single-use cleanup. No simulation step may intervene between
/// preparation and consumption. This plan is neither persisted nor cloned.
pub(crate) struct TransferPlan {
    builder: EntityId,
    refund: Option<(PlayerId, [Fixed; RESOURCES])>,
}

impl Production {
    pub(crate) fn prepare_transfer(
        &self,
        db: Option<&ContentDb>,
        eco: &Economy,
        id: EntityId,
        unit: &Entity,
    ) -> Option<TransferPlan> {
        if self.deploys.get(id).is_some()
            || self.welds.get(id).is_some()
            || self.welds.iter().any(|(_, w)| w.builder == id)
        {
            return None;
        }
        let mut plan = TransferPlan {
            builder: id,
            refund: None,
        };
        let Some(queue) = self.queues.get(id) else {
            return Some(plan);
        };
        if unit.kind != EntityKind::Station {
            return None;
        }
        let stock = eco.stock_of(unit.owner);
        let mut balance = std::array::from_fn(|i| stock.get(i).copied().unwrap_or(Fixed::ZERO));
        for item in queue {
            if item.owner != unit.owner || item.started || item.done != 0 {
                return None;
            }
            let def = db?.unit(item.def)?;
            if !item.paid {
                continue;
            }
            for (slot, &cost) in def.build.cost.iter().enumerate() {
                if cost <= Fixed::ZERO {
                    continue;
                }
                // Neutral's native precharged branch is not an ordinary
                // player's refund. Never redirect its credit to Core player0.
                if slot >= RESOURCES || !unit.owner.is_player() {
                    return None;
                }
                balance[slot] =
                    Fixed::from_bits(balance[slot].to_bits().checked_add(cost.to_bits())?);
                plan.refund = Some((unit.owner, balance));
            }
        }
        Some(plan)
    }

    pub(crate) fn finish_transfer(&mut self, eco: &mut Economy, plan: TransferPlan) {
        self.queues.remove(plan.builder);
        if let Some((owner, stock)) = plan.refund {
            eco.set_stock(owner, stock);
        }
    }
}
