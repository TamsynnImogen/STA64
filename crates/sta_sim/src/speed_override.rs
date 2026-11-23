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
//!
//! Deviations (STA64's, not read from the executable): the value is kept
//! between [`HELD_SPEED`] and [`MAX_OVERRIDE`]. A script that sets 0 holds a
//! ship; a literal 0 would make the unit unorderable and unsteppable (`orderable`
//! and `mobile` test `max_speed > 0`), so 0 is stored as the tiny held speed: the
//! craft drops to a crawl, still takes orders, and a reset frees it. The
//! ceiling keeps the fixed-point products of the movement code (squares of the
//! speed, `forward_accel * speed * distance` in the braking curve) from wrapping;
//! stock values are 35 to 150. A held craft (at or below [`HELD_SPEED`]) never
//! warps, is not watched by the stuck-order timer, and turns at its at-rest rate. An override above the
//! definition's warp speed disables warp for that craft (movement compares the
//! two); not checked against the original.

use sta_core::{EntityId, Fixed};

/// What a script asking for speed 0 gets: a crawl (1/64 unit per second).
pub const HELD_SPEED: Fixed = Fixed::from_bits(1 << 26);
/// The highest override (units per second).
pub const MAX_OVERRIDE: Fixed = Fixed::from_int(1_000);

use crate::Sim;
use crate::view::UnitFlags;
use crate::world::EntityKind;

impl Sim {
    /// `Edit::MaxSpeed`: a live craft only. `Some(v)` writes `v` kept between [`HELD_SPEED`] and [`MAX_OVERRIDE`],
    /// `None` the definition's impulse (rejected when the unit has none).
    pub(crate) fn edit_max_speed(&mut self, id: EntityId, to: Option<Fixed>) -> bool {
        let class = self.class_impulse(id);
        let Some(u) = self.units.get_mut(id).filter(|u| {
            u.kind == EntityKind::Craft && u.hull[0] > Fixed::ZERO && !u.flags.contains(UnitFlags::DYING)
        }) else {
            return false;
        };
        match to.map(|v| v.max(HELD_SPEED).min(MAX_OVERRIDE)).or(class) {
            Some(v) => {
                u.stats.max_speed = v;
                true
            }
            None => false,
        }
    }

    fn class_impulse(&self, id: EntityId) -> Option<Fixed> {
        let u = self.units.get(id)?;
        self.content.as_deref()?.unit(u.def).map(|d| d.movement.max_speed * self.engine_upgrade(u.owner))
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
        let upgrades = crate::ship_upgrades::ShipUpgrades::collect(db, &self.units);
        self.units
            .iter()
            .filter(|u| u.kind == EntityKind::Craft)
            .filter(|u| db.unit(u.def).is_some_and(|d| {
                let factor = upgrades.multiplier(u.owner, crate::ship_upgrades::Effect::Engines);
                d.movement.max_speed * factor != u.stats.max_speed
            }))
            .map(|u| (u.id, u.stats.max_speed))
            .collect()
    }
}
