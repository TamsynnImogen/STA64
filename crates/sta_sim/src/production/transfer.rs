//! Checked station-queue ownership cleanup (SIM-19).

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
        for (index, item) in queue.iter().enumerate() {
            if item.owner != unit.owner {
                return None;
            }
            let def = db?.unit(item.def)?;
            if item.started {
                if index != 0 || !item.paid || item.done >= def.build.ticks.max(1) {
                    return None;
                }
            } else if item.done != 0 {
                return None;
            }
            if !item.paid {
                continue;
            }
            let refundable = if item.started {
                let db = db?;
                let builder = db.unit(unit.def)?;
                if EntityKind::of_def(builder) != EntityKind::Station {
                    return None;
                }
                // Native ownership clear forfeits a deferred active charge;
                // FO's precharged queue-clear loop includes the active head.
                // Even done=0 has already crossed the payment boundary.
                db.unit_features(builder).pay_on_queue
            } else {
                // Preserve the unstarted stage's explicit paid-receipt rule.
                true
            };
            for (slot, &cost) in def.build.cost.iter().enumerate() {
                if cost <= Fixed::ZERO {
                    continue;
                }
                // Unsupported positive payments remain guarded even when
                // forfeited. Never expose neutral's Core player0 alias.
                if slot >= RESOURCES || !unit.owner.is_player() {
                    return None;
                }
                if !refundable {
                    continue;
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
