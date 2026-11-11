//! Exact adapters backed by shared ownership, perception and confinement.

use mlua::{Lua, MultiValue, Table, Value};
use sta_sim::command::Edit;
use sta_sim::world::NEUTRAL;

use super::objects::{handle, integer, live};
use crate::api::{Shared, edit_feedback};

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let c = ctx.clone();
    table.set(
        "SetForceOntoMap",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("SetForceOntoMap", &args, 2)?;
            let Value::Boolean(enabled) = args[1] else {
                return Err(mlua::Error::runtime("A2.SetForceOntoMap requires a boolean"));
            };
            live(&c, "SetForceOntoMap", id)?;
            edit_feedback::queue(
                &mut c.borrow_mut(),
                "A2.SetForceOntoMap",
                Edit::ForceOntoMap { id, enabled },
            )
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "SetRealTeam",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("SetRealTeam", &args, 2)?;
            let number = integer("SetRealTeam", &args[1])?;
            let destination = u8::try_from(number)
                .ok()
                .and_then(|number| c.borrow().player_numbering.decode(number))
                .filter(|p| p.is_player() || *p == NEUTRAL)
                .ok_or_else(|| mlua::Error::runtime("A2.SetRealTeam requires a real player number or neutral"))?;
            live(&c, "SetRealTeam", id)?;
            edit_feedback::queue(&mut c.borrow_mut(), "A2.SetRealTeam", Edit::Team(id, destination))
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "SetPerceivedTeam",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("SetPerceivedTeam", &args, 2)?;
            let number = integer("SetPerceivedTeam", &args[1])?;
            let destination = u8::try_from(number)
                .ok()
                .and_then(|number| c.borrow().player_numbering.decode(number))
                .filter(|p| p.is_player() || *p == NEUTRAL)
                .ok_or_else(|| mlua::Error::runtime("A2.SetPerceivedTeam requires a real player number or neutral"))?;
            live(&c, "SetPerceivedTeam", id)?;
            edit_feedback::queue(&mut c.borrow_mut(), "A2.SetPerceivedTeam", Edit::PerceivedTeam(id, destination))
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "GetPerceivedTeam",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("GetPerceivedTeam", &args, 1)?;
            let c = c.borrow();
            c.alive(id)
                .and_then(|_| c.view.perceived_owner(id))
                .map(|owner| c.player_numbering.encode(owner))
                .ok_or_else(|| mlua::Error::runtime(format!("A2.GetPerceivedTeam: object {} is not alive", id.0)))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "controls/perception_tests.rs"]
mod perception_tests;
