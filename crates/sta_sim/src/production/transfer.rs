//! Checked queue/deploy/weld ownership cleanup (SIM-19).

use super::Production;
use crate::economy::{Economy, RESOURCES};
use crate::view::UnitFlags;
use crate::world::{Entities, Entity, EntityKind};
use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId};

/// Validated, single-use cleanup. No simulation step may intervene between
/// preparation and consumption. This plan is neither persisted nor cloned.
pub(crate) struct TransferPlan {
    builder: EntityId,
    refund: Option<(PlayerId, [Fixed; RESOURCES])>,
    children: Vec<EntityId>,
}

impl Production {
    pub(crate) fn prepare_transfer(
        &self,
        db: Option<&ContentDb>,
        eco: &Economy,
        units: &Entities,
        id: EntityId,
        unit: &Entity,
    ) -> Option<TransferPlan> {
        if self.welds.get(id).is_some() {
            return None;
        }
        let mut plan = TransferPlan {
            builder: id,
            refund: None,
            children: Vec::new(),
        };
        let stock = eco.stock_of(unit.owner);
        let mut balance = std::array::from_fn(|i| stock.get(i).copied().unwrap_or(Fixed::ZERO));
        if let Some(deploy) = self.deploys.get(id) {
            if unit.kind != EntityKind::Craft || deploy.owner != unit.owner {
                return None;
            }
            let def = db?.unit(deploy.def)?;
            if deploy.paid {
                plan.credit(&mut balance, unit.owner, &def.build.cost, true)?;
            }
        }
        for (child, weld) in self.welds.iter().filter(|(_, w)| w.builder == id) {
            let child_unit = units.get(child)?;
            let def = db?.unit(child_unit.def)?;
            // Core creates and pays a weld together, with the same immutable
            // definition and owner as the builder. No supported edit changes
            // those while unfinished. This checks that model invariant, not a
            // recovered historical payment receipt from an arbitrary save.
            if unit.kind != EntityKind::Craft
                || child == id
                || child_unit.owner != unit.owner
                || child_unit.flags.contains(UnitFlags::DYING)
                || child_unit.progress.is_none_or(|p| p < Fixed::ZERO || p >= Fixed::ONE)
                || weld.ticks != def.build.ticks.max(1)
                || weld.done >= weld.ticks
                || self.queues.get(child).is_some()
                || self.deploys.get(child).is_some()
                || self.welds.iter().any(|(_, w)| w.builder == child)
            {
                return None;
            }
            let done = i32::try_from(weld.done).ok()?;
            let ticks = i32::try_from(weld.ticks).ok()?;
            if child_unit.progress != Some(Fixed::from_int(done) / Fixed::from_int(ticks)) {
                return None;
            }
            let prepaid = db?.unit_features(db?.unit(unit.def)?).pay_on_queue;
            plan.credit(&mut balance, unit.owner, &def.build.cost, prepaid)?;
            plan.children.push(child);
        }
        let Some(queue) = self.queues.get(id) else {
            return Some(plan);
        };
        if unit.kind != EntityKind::Station && !db?.unit(unit.def).is_some_and(EntityKind::producer_def) {
            return None;
        }
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
                if !EntityKind::producer_def(builder) {
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
            plan.credit(&mut balance, unit.owner, &def.build.cost, refundable)?;
        }
        Some(plan)
    }

    pub(crate) fn finish_transfer(&mut self, eco: &mut Economy, plan: TransferPlan) -> Vec<EntityId> {
        self.queues.remove(plan.builder);
        self.deploys.remove(plan.builder);
        for &child in &plan.children {
            self.welds.remove(child);
        }
        if let Some((owner, stock)) = plan.refund {
            eco.set_stock(owner, stock);
        }
        plan.children
    }
}

impl TransferPlan {
    fn credit(
        &mut self,
        balance: &mut [Fixed; RESOURCES],
        owner: PlayerId,
        costs: &[Fixed],
        refundable: bool,
    ) -> Option<()> {
        for (slot, &cost) in costs.iter().enumerate() {
            if cost <= Fixed::ZERO {
                continue;
            }
            // Even forfeited payments must fit the supported ledger. Never
            // credit a neutral or reserved owner through a player-slot alias.
            if slot >= RESOURCES || !owner.is_player() {
                return None;
            }
            if refundable {
                balance[slot] = Fixed::from_bits(balance[slot].to_bits().checked_add(cost.to_bits())?);
                self.refund = Some((owner, *balance));
            }
        }
        Some(())
    }
}
