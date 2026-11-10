//! DLL-named engine and weapon switches. The DLL's boolean means disabled;
//! the sim edits take the inverse, `on`.

use mlua::{Lua, MultiValue, Table, Value};
use sta_sim::command::Edit;

use super::objects::{handle, live};
use crate::api::Shared;

fn switch(
    lua: &Lua,
    ctx: &Shared,
    name: &'static str,
    engines: bool,
) -> mlua::Result<mlua::Function> {
    let c = ctx.clone();
    lua.create_function(move |_, args: MultiValue| {
        let id = handle(name, &args, 2)?;
        let Value::Boolean(disabled) = &args[1] else {
            return Err(mlua::Error::runtime(format!(
                "A2.{name} requires a boolean"
            )));
        };
        live(&c, name, id)?;
        let edit = if engines {
            Edit::Engines(id, !*disabled)
        } else {
            Edit::Weapons(id, !*disabled)
        };
        c.borrow_mut().edit(edit);
        Ok(())
    })
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    table.set("DisableEngines", switch(lua, ctx, "DisableEngines", true)?)?;
    table.set("DisableWeapons", switch(lua, ctx, "DisableWeapons", false)?)?;
    Ok(())
}

#[cfg(test)]
#[path = "subsystems/tests.rs"]
mod tests;
