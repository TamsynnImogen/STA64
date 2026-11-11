//! Exact A2 camera requests retained in the presentation command stream.

use mlua::{Lua, MultiValue, Table, Value};
use sta_core::{EntityId, Fixed, PlayerId};
use sta_sim::command::{CommandKind, Edit};
use sta_sim::present::{Arg, Presentation};

use super::objects::{integer, live};
use crate::api::Shared;

fn arity(name: &str, args: &MultiValue, expected: usize) -> mlua::Result<()> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(mlua::Error::runtime(format!(
            "A2.{name} requires exactly {expected} argument(s)"
        )))
    }
}

fn entity(ctx: &Shared, name: &str, value: &Value) -> mlua::Result<Arg> {
    let id = EntityId(integer(name, value)?);
    live(ctx, name, id)?;
    Ok(Arg::Entity(id))
}

fn number(name: &str, value: &Value) -> mlua::Result<Arg> {
    let n = match value {
        Value::Integer(n) => *n as f64,
        Value::Number(n) => *n,
        _ => {
            return Err(mlua::Error::runtime(format!(
                "A2.{name} requires a finite numeric value"
            )))
        }
    };
    if !n.is_finite() || !(-2_147_483_648.0..2_147_483_648.0).contains(&n) {
        return Err(mlua::Error::runtime(format!(
            "A2.{name} requires a number within signed 32.32 range"
        )));
    }
    Ok(Arg::Num(Fixed::from_f64(n)))
}

fn boolean(name: &str, value: &Value) -> mlua::Result<Arg> {
    match value {
        Value::Boolean(value) => Ok(Arg::Bool(*value)),
        _ => Err(mlua::Error::runtime(format!("A2.{name} requires a boolean"))),
    }
}

fn text(name: &str, value: &Value) -> mlua::Result<Arg> {
    let Value::String(value) = value else {
        return Err(mlua::Error::runtime(format!("A2.{name} requires a string")));
    };
    let value = value.to_str()?;
    if value.contains('\0') {
        return Err(mlua::Error::runtime(format!("A2.{name} string contains NUL")));
    }
    Ok(Arg::Text(value.to_owned()))
}

fn point_index(name: &str, value: &Value) -> mlua::Result<Arg> {
    match value {
        Value::Integer(value) if (0..=i32::MAX as i64).contains(value) => Ok(Arg::Int(*value)),
        _ => Err(mlua::Error::runtime(format!(
            "A2.{name} requires a nonnegative signed-32-bit path point index"
        ))),
    }
}

fn submit(ctx: &Shared, name: &str, args: Vec<Arg>) -> mlua::Result<()> {
    let presentation = Presentation::Call {
        name: format!("A2.{name}"),
        args,
    };
    ctx.borrow_mut()
        .try_push(PlayerId::SCRIPT, CommandKind::Edit(Edit::Present(presentation)))
        .map_err(|reason| mlua::Error::runtime(format!("A2.{name}: {reason}")))?;
    Ok(())
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let c = ctx.clone();
    table.set(
        "CenterCamera",
        lua.create_function(move |_, args: MultiValue| {
            arity("CenterCamera", &args, 1)?;
            let arguments = vec![entity(&c, "CenterCamera", &args[0])?];
            submit(&c, "CenterCamera", arguments)
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "SetCameraZoom",
        lua.create_function(move |_, args: MultiValue| {
            arity("SetCameraZoom", &args, 1)?;
            let arguments = vec![number("SetCameraZoom", &args[0])?];
            submit(&c, "SetCameraZoom", arguments)
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "CineractiveConstantSpeed",
        lua.create_function(move |_, args: MultiValue| {
            arity("CineractiveConstantSpeed", &args, 1)?;
            let arguments = vec![boolean("CineractiveConstantSpeed", &args[0])?];
            submit(&c, "CineractiveConstantSpeed", arguments)
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "CineractiveCameraOffset",
        lua.create_function(move |_, args: MultiValue| {
            let arguments = match args.len() {
                4 => vec![
                    number("CineractiveCameraOffset", &args[0])?,
                    number("CineractiveCameraOffset", &args[1])?,
                    number("CineractiveCameraOffset", &args[2])?,
                    boolean("CineractiveCameraOffset", &args[3])?,
                ],
                3 => vec![
                    text("CineractiveCameraOffset", &args[0])?,
                    number("CineractiveCameraOffset", &args[1])?,
                    boolean("CineractiveCameraOffset", &args[2])?,
                ],
                _ => {
                    return Err(mlua::Error::runtime(
                        "A2.CineractiveCameraOffset requires exactly 3 or 4 argument(s)",
                    ));
                }
            };
            submit(&c, "CineractiveCameraOffset", arguments)
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "CineractiveShot",
        lua.create_function(move |_, args: MultiValue| {
            let arguments = match args.len() {
                2 => vec![
                    entity(&c, "CineractiveShot", &args[0])?,
                    entity(&c, "CineractiveShot", &args[1])?,
                ],
                3 => {
                    let path = text("CineractiveShot", &args[0])?;
                    let parameter = match &args[1] {
                        Value::Integer(_) => point_index("CineractiveShot", &args[1])?,
                        Value::Number(_) => number("CineractiveShot", &args[1])?,
                        _ => {
                            return Err(mlua::Error::runtime(
                                "A2.CineractiveShot requires an integer path index or floating path parameter",
                            ));
                        }
                    };
                    vec![path, parameter, entity(&c, "CineractiveShot", &args[2])?]
                }
                _ => {
                    return Err(mlua::Error::runtime(
                        "A2.CineractiveShot requires exactly 2 or 3 argument(s)",
                    ));
                }
            };
            submit(&c, "CineractiveShot", arguments)
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "native_camera/tests.rs"]
mod tests;
