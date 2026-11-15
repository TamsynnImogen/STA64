//! LUA-31: scripted Armada I warp speed (`SetWarpSpeed` / `ResetWarpSpeed`).
//!
//! Rule (read from the executable, static; not measured in game): the call
//! sets the craft's absolute warp speed, in world units per second (the
//! executable stores it as a multiplier of the impulse speed, `v / impulse`;
//! `GetWarpSpeed` is multiplier times impulse). `ResetWarpSpeed` restores the
//! class value. A scripted impulse change keeps the absolute warp speed (the
//! multiplier is rescaled), which STA64's absolute `stats.warp_speed` gives
//! for free. The end of any cineractive resets impulse and warp of every craft
//! to the class values. Orders do not touch it. Only a live craft is affected.
//!
//! The warp leg of a keyed (Armada I) craft runs at the absolute warp speed
//! even when it is not above impulse: the aligned far leg then flies at the
//! warp speed (a slower leg), the turning legs at impulse; no warp-in effect.
//!
//! STA64 state: `stats.warp_speed`, which a spawn fills from the definition.
//! Craft whose value differs from the definition are saved (snapshot format 28,
//! the `WarpRec` record) and hashed (nothing is added to the digest of a run
//! that never overrides it).
//!
//! Deviations (STA64's, not read from the executable): the value is kept
//! between 0 and [`MAX_WARP`]; at or below [`HELD_SPEED`](crate::speed_override::HELD_SPEED)
//! the craft has no warp leg at all and flies at impulse (a literal zero
//! throttle would make a long move never arrive). The ceiling keeps the fixed-point
//! products of the warp code from wrapping; stock values are 20 to 100.
//! An owner change leaves it alone (A1 `SetTeam` resetting speed is not
//! established). A warp speed set after a move was ordered does not turn a plain
//! move into a warp move (the order decides at issue time).

use sta_core::{EntityId, Fixed};

use crate::speed_override::HELD_SPEED;
use crate::view::UnitFlags;
use crate::world::{EntityKind, Fnv};
use crate::Sim;

/// The highest scripted warp speed (units per second).
pub const MAX_WARP: Fixed = Fixed::from_int(1_000);

/// The speed of the aligned far leg of a keyed craft whose warp speed is below
/// its impulse speed; `None` when the leg is a real warp (above impulse) or
/// there is no leg (zero, or equal to impulse).
pub(crate) fn slow_leg(warp: Fixed, impulse: Fixed) -> Option<Fixed> {
    (warp > HELD_SPEED && warp < impulse).then_some(warp)
}

impl Sim {
    /// `Edit::WarpSpeed`: a live craft with a definition only (the override is kept as
    /// the difference from it, so a craft without one has nothing to save). `Some(v)` writes `v`
    /// kept between 0 and [`MAX_WARP`], `None` the definition's warp speed.
    pub(crate) fn edit_warp_speed(&mut self, id: EntityId, to: Option<Fixed>) -> bool {
        let Some(class) = self.class_warp(id) else { return false };
        let Some(u) = self.units.get_mut(id).filter(|u| {
            u.kind == EntityKind::Craft && u.hull[0] > Fixed::ZERO && !u.flags.contains(UnitFlags::DYING)
        }) else {
            return false;
        };
        u.stats.warp_speed = to.map_or(class, |v| v.max(Fixed::ZERO).min(MAX_WARP));
        true
    }

    fn class_warp(&self, id: EntityId) -> Option<Fixed> {
        let u = self.units.get(id)?;
        self.content.as_deref()?.unit(u.def).map(|d| d.movement.warp_speed)
    }

    /// The end of a cineractive resets every craft's warp speed (the impulse
    /// half is `reset_all_impulse`).
    pub(crate) fn reset_all_warp(&mut self) {
        let ids: Vec<EntityId> = self.units.iter().filter(|u| u.kind == EntityKind::Craft).map(|u| u.id).collect();
        for id in ids {
            if let (Some(v), Some(u)) = (self.class_warp(id), self.units.get_mut(id)) {
                u.stats.warp_speed = v;
            }
        }
    }

    /// Craft whose warp speed differs from the definition's, ascending id.
    pub(crate) fn warp_overrides(&self) -> Vec<(EntityId, Fixed)> {
        let Some(db) = self.content.as_deref() else { return Vec::new() };
        self.units
            .iter()
            .filter(|u| u.kind == EntityKind::Craft)
            .filter(|u| db.unit(u.def).is_some_and(|d| d.movement.warp_speed != u.stats.warp_speed))
            .map(|u| (u.id, u.stats.warp_speed))
            .collect()
    }

    /// Hash of the overrides, only when there are any (a run that never sends
    /// the edit keeps its digests).
    pub(crate) fn hash_warp_overrides(&self, h: &mut Fnv) {
        let rows = self.warp_overrides();
        if rows.is_empty() {
            return;
        }
        h.str("warp-override-v1");
        h.u64(rows.len() as u64);
        for (id, v) in rows {
            h.u64(id.0 as u64);
            h.fixed(v);
        }
    }
}
