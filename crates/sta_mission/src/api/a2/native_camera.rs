//! Exact A2 camera requests retained in the presentation command stream.

use mlua::{Lua, MultiValue, Table, Value};
use sta_core::{EntityId, Fixed, PlayerId};
use sta_sim::command::{CommandKind, Edit};
use sta_sim::present::{Arg, Presentation};

use super::objects::{integer, live};
use crate::api::Shared;

pub(super) fn arity(name: &str, args: &MultiValue, expected: usize) -> mlua::Result<()> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(mlua::Error::runtime(format!(
            "A2.{name} requires exactly {expected} argument(s)"
        )))
    }
}

pub(super) fn entity(ctx: &Shared, name: &str, value: &Value) -> mlua::Result<Arg> {
    let id = EntityId(integer(name, value)?);
    live(ctx, name, id)?;
    Ok(Arg::Entity(id))
}

pub(super) fn number(name: &str, value: &Value) -> mlua::Result<Arg> {
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

pub(super) fn boolean(name: &str, value: &Value) -> mlua::Result<Arg> {
    match value {
        Value::Boolean(value) => Ok(Arg::Bool(*value)),
        _ => Err(mlua::Error::runtime(format!("A2.{name} requires a boolean"))),
    }
}

pub(super) fn text(name: &str, value: &Value) -> mlua::Result<Arg> {
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

pub(super) fn submit(ctx: &Shared, name: &str, args: Vec<Arg>) -> mlua::Result<()> {
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
            // Two overloads (decorated names, static): an object, or a path name and its point.
            let arguments = match args.len() {
                1 => vec![entity(&c, "CenterCamera", &args[0])?],
                2 => vec![text("CenterCamera", &args[0])?, point_index("CenterCamera", &args[1])?],
                _ => return Err(mlua::Error::runtime("A2.CenterCamera requires exactly 1 or 2 argument(s)")),
            };
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

    // Twelve overloads in the executable (decorated names, static): the first
    // argument is a handle or a path name, then a path point (int) or
    // parameter (float), then a handle, a second path with its point or
    // parameter, a flag, or nothing. Each shape retains its arguments.
    let c = ctx.clone();
    table.set(
        "CineractiveShot",
        lua.create_function(move |_, args: MultiValue| {
            let is_text = |v: &Value| matches!(v, Value::String(_));
            let is_num = |v: &Value| matches!(v, Value::Integer(_) | Value::Number(_));
            let is_bool = |v: &Value| matches!(v, Value::Boolean(_));
            let param = |v: &Value| match v {
                Value::Integer(_) => point_index("CineractiveShot", v),
                _ => number("CineractiveShot", v),
            };
            let n = "CineractiveShot";
            let a: Vec<&Value> = args.iter().collect();
            let arguments = match a.as_slice() {
                [x, y] if is_bool(y) => vec![entity(&c, n, x)?, boolean(n, y)?],
                [x, y] => vec![entity(&c, n, x)?, entity(&c, n, y)?],
                [p, t, z] if is_text(p) && is_num(t) && is_bool(z) => vec![text(n, p)?, param(t)?, boolean(n, z)?],
                [p, t, z] if is_text(p) && is_num(t) => vec![text(n, p)?, param(t)?, entity(&c, n, z)?],
                [h, p, t] if is_text(p) && is_num(t) => vec![entity(&c, n, h)?, text(n, p)?, param(t)?],
                [p, t, q, u] if is_text(p) && is_num(t) && is_text(q) && is_num(u) => {
                    vec![text(n, p)?, param(t)?, text(n, q)?, param(u)?]
                }
                _ => {
                    return Err(mlua::Error::runtime(
                        "A2.CineractiveShot: no overload takes these arguments (2 to 4 of handle/path/number/flag)",
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
