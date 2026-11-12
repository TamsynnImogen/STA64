//! Exact A2 presentation requests that need no simulation state, retained in
//! the presentation command stream like the [camera requests](super::native_camera).
//!
//! Each successful call queues exactly one SCRIPT `Edit::Present` carrying a
//! `Presentation::Call` named `A2.<method>` with the typed arguments in call
//! order. Raw ints, floats, flags and strings are retained, never
//! reinterpreted (a minimap message's second int is not guessed to be a colour
//! or an id). There is no client consumer yet: queuing a request is not
//! playback. Script-pointer calls (`script *` first argument), handle-returning
//! calls and completion queries need a lifecycle contract and are not here.

use mlua::{Lua, MultiValue, Table, Value};
use sta_sim::present::Arg;

use super::native_camera::{arity, boolean, entity, number, submit, text};
use crate::api::Shared;

/// A signed 32-bit `int` argument that is not an object handle.
fn int32(name: &str, value: &Value) -> mlua::Result<Arg> {
    match value {
        Value::Integer(v) if i32::try_from(*v).is_ok() => Ok(Arg::Int(*v)),
        _ => Err(mlua::Error::runtime(format!(
            "A2.{name} requires a signed-32-bit integer"
        ))),
    }
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let c = ctx.clone();
    table.set(
        "AddTransporterEffect",
        lua.create_function(move |_, args: MultiValue| {
            arity("AddTransporterEffect", &args, 1)?;
            let arguments = vec![entity(&c, "AddTransporterEffect", &args[0])?];
            submit(&c, "AddTransporterEffect", arguments)
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "RemoveBillboardEffect",
        lua.create_function(move |_, args: MultiValue| {
            arity("RemoveBillboardEffect", &args, 1)?;
            let arguments = vec![int32("RemoveBillboardEffect", &args[0])?];
            submit(&c, "RemoveBillboardEffect", arguments)
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "GridVisible",
        lua.create_function(move |_, args: MultiValue| {
            arity("GridVisible", &args, 1)?;
            let arguments = vec![boolean("GridVisible", &args[0])?];
            submit(&c, "GridVisible", arguments)
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "RemoveFromAdLog",
        lua.create_function(move |_, args: MultiValue| {
            arity("RemoveFromAdLog", &args, 1)?;
            let arguments = vec![int32("RemoveFromAdLog", &args[0])?];
            submit(&c, "RemoveFromAdLog", arguments)
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "Starfield_Load_New_Background_Geometry",
        lua.create_function(move |_, args: MultiValue| {
            let name = "Starfield_Load_New_Background_Geometry";
            arity(name, &args, 2)?;
            let arguments = vec![text(name, &args[0])?, text(name, &args[1])?];
            submit(&c, name, arguments)
        })?,
    )?;

    // `MinimapMessage(int, int, float)` and `MinimapMessage(char *, int,
    // float, int)`: the argument count selects the overload.
    let c = ctx.clone();
    table.set(
        "MinimapMessage",
        lua.create_function(move |_, args: MultiValue| {
            let name = "MinimapMessage";
            let arguments = match args.len() {
                3 => vec![int32(name, &args[0])?, int32(name, &args[1])?, number(name, &args[2])?],
                4 => vec![
                    text(name, &args[0])?,
                    int32(name, &args[1])?,
                    number(name, &args[2])?,
                    int32(name, &args[3])?,
                ],
                _ => {
                    return Err(mlua::Error::runtime(
                        "A2.MinimapMessage requires exactly 3 or 4 argument(s)",
                    ));
                }
            };
            submit(&c, name, arguments)
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "native_presentation/tests.rs"]
mod tests;
