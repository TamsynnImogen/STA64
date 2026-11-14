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
//! carry. Where the casualty model runs, a latched unit has its crew set to 0 and a cleared
//! one has crew of 1 or more, so the view's crew pair is the same test. It differs only for
//! a unit whose features have no casualty model (a crew edit never latches there, the
//! engine would) and for one tick after the crew is restored (the latch is cleared in the
//! tick's upkeep). Reading the latch itself needs a view field (a `UnitView` field or a
//! `UnitFlags` bit: the byte is full), which is left to the package that adds them.

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
        c.note_query(id, "IsDerelict");
        Ok(is_derelict(c, id))
    })?)?;
    Ok(())
}

#[cfg(test)]
mod tests;
