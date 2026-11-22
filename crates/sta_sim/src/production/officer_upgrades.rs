//! Armada I officer quarters complete on the starbase. The live base and
//! its completed quarters contribute capacity to its owner.

use super::{Production, finished};
use crate::economy::Economy;
use crate::view::UnitFlags;
use crate::world::{Entities, Entity, Fnv, Table};
use sta_content::{ContentDb, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId};

pub type OfficerUpgradeRec = Vec<(EntityId, u32)>;

pub(super) fn crew_cost(def: &UnitDef) -> Fixed {
    if def.a1_officer_upgrade_gain.is_some() { def.crew.max(Fixed::ZERO) } else { Fixed::ZERO }
}

pub(super) fn credit_crew(eco: &mut Economy, owner: PlayerId, amount: Fixed) {
    if amount > Fixed::ZERO {
        let crew = eco.crew_of(owner).unwrap_or(Fixed::ZERO);
        let total = Fixed::from_bits(crew.to_bits().saturating_add(amount.to_bits()));
        eco.set_personnel(owner, Some(total), eco.officer_base(owner));
    }
}

impl Production {
    pub(crate) fn officer_limit(&self, units: &Entities, db: &ContentDb, eco: &Economy, owner: PlayerId) -> Option<Fixed> {
        let configured = eco.officer_limit(owner);
        if !db.a1_officers() {
            return configured;
        }
        let baseline = configured.unwrap_or_else(|| Fixed::from_int(db.a1_starting_officers().unwrap_or(0)));
        let mut cap = baseline.to_bits();
        for unit in units.iter().filter(|u| u.owner == owner && finished(u) && !u.flags.contains(UnitFlags::DYING)) {
            let Some(base) = db.unit(unit.def).and_then(|d| d.a1_base_officers) else { continue };
            cap = cap.saturating_add(Fixed::from_int(base.gain).to_bits());
            if let Some(gain) = base.upgrade.and_then(|key| db.unit(key)).and_then(|d| d.a1_officer_upgrade_gain) {
                let count = self.officer_upgrades.get(unit.id).copied().unwrap_or(0);
                let bonus = Fixed::from_int(gain).to_bits().saturating_mul(i64::from(count));
                cap = cap.saturating_add(bonus);
            }
        }
        Some(Fixed::from_bits(cap))
    }

    pub(super) fn officer_upgrade_view(&self, yard: &Entity, def: &UnitDef) -> Option<super::view::OfficerUpgradeView> {
        let base = def.a1_base_officers?;
        let completed = self.officer_upgrades.get(yard.id).copied().unwrap_or(0);
        let inactive = super::view::OfficerUpgradeView { completed, remaining: 0 };
        if !finished(yard) || yard.flags.contains(UnitFlags::DYING) {
            return Some(inactive);
        }
        let Some(upgrade) = base.upgrade else { return Some(inactive) };
        let queued = self.queues.get(yard.id).into_iter().flatten().filter(|item| item.def == upgrade).count() as u32;
        let remaining = base.maximum_upgrades.saturating_sub(completed).saturating_sub(queued);
        Some(super::view::OfficerUpgradeView { completed, remaining })
    }

    pub(super) fn admits_officer_upgrade(&self, yard: &Entity, base: &UnitDef, product: &UnitDef) -> bool {
        product.a1_officer_upgrade_gain.is_none()
            || (base.a1_base_officers.is_some_and(|b| b.upgrade == Some(product.key))
                && self.officer_upgrade_view(yard, base).is_some_and(|v| v.remaining > 0))
    }

    pub(super) fn finish_officer_upgrade(&mut self, yard: &Entity, base: &UnitDef, product: &UnitDef) -> bool {
        let Some(policy) = base.a1_base_officers.filter(|b| b.upgrade == Some(product.key)) else {
            return false;
        };
        let count = self.officer_upgrades.get(yard.id).copied().unwrap_or(0);
        if count >= policy.maximum_upgrades || yard.flags.contains(UnitFlags::DYING) {
            return false;
        }
        self.officer_upgrades.insert(yard.id, count + 1);
        true
    }

    pub(crate) fn clear_officer_upgrades(&mut self, yard: EntityId) {
        self.officer_upgrades.remove(yard);
    }

    pub(super) fn hash_officer_upgrades(&self, h: &mut Fnv) {
        if !self.officer_upgrades.is_empty() {
            h.u64(u64::from_le_bytes(*b"a1offs01"));
            h.u64(self.officer_upgrades.iter().count() as u64);
            for (id, count) in self.officer_upgrades.iter() {
                h.u64(u64::from(id.0));
                h.u64(u64::from(*count));
            }
        }
    }

    pub(crate) fn officer_upgrade_rows(&self) -> OfficerUpgradeRec {
        self.officer_upgrades.iter().map(|(id, count)| (id, *count)).collect()
    }

    pub(crate) fn restore_officer_upgrades(&mut self, rows: OfficerUpgradeRec, units: &Entities, db: Option<&ContentDb>) -> Option<()> {
        for &(id, count) in &rows {
            let yard = units.get(id)?;
            let base = db?.unit(yard.def)?.a1_base_officers?;
            if base.upgrade.is_none() || !finished(yard) || count == 0 || count > base.maximum_upgrades {
                return None;
            }
        }
        self.officer_upgrades = Table::from_rows(rows)?;
        Some(())
    }
}
