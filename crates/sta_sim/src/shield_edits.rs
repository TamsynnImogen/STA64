//! Privileged writes to the existing per-instance shield storage.

use sta_core::{EntityId, Fixed};

use crate::Sim;
use crate::view::UnitFlags;

impl Sim {
    pub(crate) fn set_shield_fraction(&mut self, id: EntityId, fraction: Fixed) -> bool {
        let Some(unit) = self
            .units
            .get_mut(id)
            .filter(|unit| !unit.flags.contains(UnitFlags::DYING))
        else {
            return false;
        };
        // Match Fixed multiplication's arithmetic shift, but reject a result
        // outside its storage instead of truncating it back into i64.
        let product = (i128::from(unit.shields.max.to_bits()) * i128::from(fraction.to_bits())) >> 32;
        let Ok(bits) = i64::try_from(product) else {
            return false;
        };
        let current = Fixed::from_bits(bits);
        let pools = if unit.shields.directional { 4 } else { 1 };
        unit.shields.now[..pools].fill(current);
        true
    }

    pub(crate) fn set_max_shields(&mut self, id: EntityId, maximum: Fixed) -> bool {
        let Some(unit) = self
            .units
            .get_mut(id)
            .filter(|unit| !unit.flags.contains(UnitFlags::DYING))
        else {
            return false;
        };
        let pools = if unit.shields.directional { 4 } else { 1 };
        unit.shields.max = maximum;
        unit.shields.now[..pools].fill(maximum);
        true
    }
}

#[cfg(test)]
#[path = "shield_edits_tests.rs"]
mod tests;
