//! LUA-27: scripted impulse speed (`SetImpulseSpeed` / `ResetImpulseSpeed`).
//!
//! Rule (read from the executable, static; not measured in game): the call
//! overwrites the craft's one impulse-speed field, the same field the class
//! value fills at spawn, so there is no separate cap and no new state: the
//! override lives in `stats.max_speed`, which is already saved and digested.
//! Absolute world units per second, no range check beyond `>= 0`; orders do
//! not touch it. It is reset to the definition's impulse (STA64 has no team
//! engine upgrades, multiplier 1) by `Edit::MaxSpeed(id, None)`, by a change
//! of owner, and by the end of any cineractive for every craft. STA64 has no
//! engine-upgrade research, slowing effect or speed boost; those resets of
//! the executable have nothing to attach to. Warp speed is left alone.

use sta_core::{EntityId, Fixed};

use crate::Sim;
use crate::view::UnitFlags;
use crate::world::EntityKind;

impl Sim {
    /// `Edit::MaxSpeed`: a live craft only. `Some(v)` writes `max(v, 0)`,
    /// `None` the definition's impulse (rejected when the unit has none).
    pub(crate) fn edit_max_speed(&mut self, id: EntityId, to: Option<Fixed>) -> bool {
        let class = self.class_impulse(id);
        let Some(u) = self.units.get_mut(id).filter(|u| {
            u.kind == EntityKind::Craft && u.hull[0] > Fixed::ZERO && !u.flags.contains(UnitFlags::DYING)
        }) else {
            return false;
        };
        match to.map(|v| v.max(Fixed::ZERO)).or(class) {
            Some(v) => {
                u.stats.max_speed = v;
                true
            }
            None => false,
        }
    }

    fn class_impulse(&self, id: EntityId) -> Option<Fixed> {
        let u = self.units.get(id)?;
        self.content.as_deref()?.unit(u.def).map(|d| d.movement.max_speed)
    }

    /// Back to the definition's impulse (team change, cineractive end); a unit
    /// without a definition keeps its value.
    pub(crate) fn reset_impulse(&mut self, id: EntityId) {
        if let (Some(v), Some(u)) = (self.class_impulse(id), self.units.get_mut(id).filter(|u| u.kind == EntityKind::Craft)) {
            u.stats.max_speed = v;
        }
    }

    /// The end of a cineractive resets every craft in the world.
    pub(crate) fn reset_all_impulse(&mut self) {
        let ids: Vec<EntityId> = self.units.iter().filter(|u| u.kind == EntityKind::Craft).map(|u| u.id).collect();
        for id in ids {
            self.reset_impulse(id);
        }
    }

    /// Craft whose current impulse differs from the definition's, ascending id.
    pub(crate) fn impulse_overrides(&self) -> Vec<(EntityId, Fixed)> {
        let Some(db) = self.content.as_deref() else { return Vec::new() };
        self.units
            .iter()
            .filter(|u| u.kind == EntityKind::Craft)
            .filter(|u| db.unit(u.def).is_some_and(|d| d.movement.max_speed != u.stats.max_speed))
            .map(|u| (u.id, u.stats.max_speed))
            .collect()
    }
}
