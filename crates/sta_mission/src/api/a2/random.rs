//! DLL-shaped bounded integers on the runtime's existing deterministic RNG.

use mlua::{Function, Lua, MultiValue, Table};

use super::objects::integer;
use crate::api::Shared;

pub(super) fn install(lua: &Lua, _ctx: &Shared, table: &Table) -> mlua::Result<()> {
    // MissionRuntime installs deterministic math before API bindings and runs
    // mission code afterward. Capture that function rather than a mutable
    // global lookup on every draw; randomseed still updates its shared state.
    let math: Table = lua.globals().get("math")?;
    let random: Function = math.get("random")?;
    table.set(
        "RandomGetUnsigned",
        lua.create_function(move |_, args: MultiValue| {
            if args.len() != 1 {
                return Err(mlua::Error::runtime("A2.RandomGetUnsigned requires exactly one argument"));
            }
            let upper = integer("RandomGetUnsigned", &args[0])?;
            if upper == 0 {
                return Err(mlua::Error::runtime("A2.RandomGetUnsigned requires a positive upper bound"));
            }
            random.call::<u32>((0i64, i64::from(upper) - 1))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "random/tests.rs"]
mod tests;
