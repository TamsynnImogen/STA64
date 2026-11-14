//! `IsDerelict(object)` for Armada II and Armada I (LUA-26), and the derelict test
//! `IsCompromised` shares.
//!
//! Rule (read from the executable, static; not measured in game; same in A1 and A2): true for
//! an object that is missing, dying or not a craft or station; otherwise the craft's derelict
//! latch. The latch is set by the engine's crew check (crew under 1 with a positive maximum;
//! the `CraftCannotDie` top-up without `ButItCanBeDerelicted` gives crew back instead) and
//! cleared when crew is back at 1 or more.
//!
//! The sim keeps that latch in `SubsystemRow::derelict`, which the mission view does not
//! carry (`UnitFlags` is a full byte; a view field is left to the package that adds them), so
//! the binding reads the view's crew pair, the same test the engine's check makes. Where the
//! casualty model runs (`Features::crew_loss_on_hit`) a latched unit has crew 0 and an
//! unlatched one has crew of 1 or more, and `Sim::step` clears the latch before it builds the
//! view, so the two agree in the same tick. Divergences, all pinned or documented here:
//!
//! - Where `crew_loss_on_hit` is off (the Armada I profile default, `Profile::Sta1Classic`) there
//!   is no casualty model: the sim never calls the check, so combat never latches and `SetCrew(h, 0)` on a `CraftCannotDie` craft
//!   is not topped up. The crew pair then reads derelict where the engine's rule says not.
//! - A `maxCrew` edit never runs the check: `maxCrew 0` on a latched unit reads not derelict
//!   while the sim keeps the latch, and a maximum just under 1 reads derelict with no latch.
//! - A unit spawned with crew under 1 and a positive maximum is not checked at spawn and keeps
//!   working in the sim, but reads derelict. No stock definition is known to have one (read from
//!   data, Armada II; Armada I not measured).
//!
//! A `BuildObject` token that is pending raises, so the gap is tainted: the engine's object
//! exists at once and is crewed, so no constant is right. In A2 a failed or unknown token raises
//! too (via `dead_handle`); in A1 a failed or unknown one answers true (the lenient alias).

use mlua::{Lua, Table};
use sta_core::Fixed;
use sta_sim::view::{UnitFlags, UnitView};
use sta_sim::world::EntityKind;

use super::{handle, query};
use crate::api::{Ctx, Shared};

/// The derelict latch of a live craft or station, as the view shows it.
pub(crate) fn latched(u: &UnitView) -> bool {
    u.crew.is_some_and(|[now, max]| now < Fixed::ONE && max > Fixed::ZERO)
}

/// Not a craft or station by STA64's class mapping (`EntityKind::of_def`), an approximation of
/// the engine's class test; with no content a unit is a craft.
pub(crate) fn fixture(c: &Ctx, u: &UnitView) -> bool {
    c.def(u).is_some_and(|d| !matches!(EntityKind::of_def(d), EntityKind::Craft | EntityKind::Station))
}

/// `IsDerelict` of an object id: true when it is missing, dying or a fixture, else the latch.
pub(crate) fn is_derelict(c: &Ctx, id: sta_core::EntityId) -> bool {
    match c.unit(id) {
        Some(u) if !u.flags.contains(UnitFlags::DYING) => fixture(c, u) || latched(u),
        _ => true,
    }
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    table.set("IsDerelict", query(lua, ctx, "IsDerelict", 1, |c, a| {
        let id = handle("IsDerelict", a, 0)?;
        if c.alive(id).is_none() {
            // Missing or dying: true; a pending token raises so the gap is tainted.
            c.dead_handle("A2", "IsDerelict", id)?;
            return Ok(true);
        }
        c.note_query(id, "IsDerelict");
        Ok(is_derelict(c, id))
    })?)?;
    Ok(())
}

#[cfg(test)]
mod tests;
