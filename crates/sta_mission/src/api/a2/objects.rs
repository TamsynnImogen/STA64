//! DLL-named object calls. Handles refer to the latest mission view; writes
//! become commands and are visible only after the simulation advances.

use mlua::{Lua, MultiValue, Table, Value};
use sta_core::{EntityId, Fixed};
use sta_sim::command::{Edit, Order};

use crate::api::{Shared, pos_f64, vector};

pub(super) fn integer(name: &str, value: &Value) -> mlua::Result<u32> {
    let n = match value {
        Value::Integer(n) => u32::try_from(*n).ok(),
        Value::Number(n)
            if n.is_finite() && *n >= 0.0 && *n <= u32::MAX as f64 && n.fract() == 0.0 =>
        {
            Some(*n as u32)
        }
        _ => None,
    };
    n.ok_or_else(|| mlua::Error::runtime(format!("A2.{name} requires an unsigned integer")))
}

pub(super) fn handle(name: &str, args: &MultiValue, arity: usize) -> mlua::Result<EntityId> {
    if args.len() != arity {
        return Err(mlua::Error::runtime(format!(
            "A2.{name} requires exactly {arity} argument(s)"
        )));
    }
    Ok(EntityId(integer(name, &args[0])?))
}

pub(super) fn live(ctx: &Shared, name: &str, id: EntityId) -> mlua::Result<()> {
    if ctx.borrow().alive(id).is_some() {
        Ok(())
    } else {
        Err(mlua::Error::runtime(format!(
            "A2.{name}: object {} is not alive",
            id.0
        )))
    }
}

fn finite_amount(name: &str, label: &str, value: &Value) -> mlua::Result<Fixed> {
    let amount = match value {
        Value::Integer(n) => *n as f64,
        Value::Number(n) if n.is_finite() => *n,
        _ => {
            return Err(mlua::Error::runtime(format!(
                "A2.{name} requires a finite numeric {label} value"
            )));
        }
    };
    Ok(Fixed::from_f64(amount))
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let c = ctx.clone();
    table.set(
        "Exists",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("Exists", &args, 1)?;
            c.borrow().note_query(id, "Exists");
            Ok(c.borrow().unit(id).is_some())
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "DoesNotExist",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("DoesNotExist", &args, 1)?;
            c.borrow().note_query(id, "DoesNotExist");
            Ok(c.borrow().unit(id).is_none())
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "IsAlive",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("IsAlive", &args, 1)?;
            c.borrow().note_query(id, "IsAlive");
            Ok(c.borrow().alive(id).is_some())
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "GetLocation",
        lua.create_function(move |lua, args: MultiValue| {
            let id = handle("GetLocation", &args, 1)?;
            let pos = c
                .borrow()
                .alive(id)
                .map(|u| pos_f64(u.pos))
                .ok_or_else(|| {
                    mlua::Error::runtime(format!("A2.GetLocation: object {} is not alive", id.0))
                })?;
            vector(lua, pos)
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "GetRealTeam",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("GetRealTeam", &args, 1)?;
            let c = c.borrow();
            c.alive(id).map(|u| c.player_numbering.encode(u.owner)).ok_or_else(|| {
                mlua::Error::runtime(format!("A2.GetRealTeam: object {} is not alive", id.0))
            })
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "RemoveObject",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("RemoveObject", &args, 1)?;
            live(&c, "RemoveObject", id)?;
            c.borrow_mut().edit(Edit::Kill(id));
            Ok(())
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "SetCurrentHealth",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("SetCurrentHealth", &args, 2)?;
            let hull = finite_amount("SetCurrentHealth", "hull", &args[1])?;
            live(&c, "SetCurrentHealth", id)?;
            c.borrow_mut().edit(Edit::Hull(id, hull));
            Ok(())
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "SetSpecialEnergy",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("SetSpecialEnergy", &args, 2)?;
            let energy = finite_amount("SetSpecialEnergy", "energy", &args[1])?;
            {
                let c = c.borrow();
                let unit = c.alive(id)
                    .filter(|unit| unit.hull[0] > Fixed::ZERO)
                    .ok_or_else(|| mlua::Error::runtime(format!(
                        "A2.SetSpecialEnergy: object {} is not a live positive-hull unit", id.0
                    )))?;
                if c.def(unit).and_then(sta_sim::specials::energy_def).is_none() {
                    return Err(mlua::Error::runtime(format!(
                        "A2.SetSpecialEnergy: object {} has no defined energy pool", id.0
                    )));
                }
            }
            c.borrow_mut().edit(Edit::SetEnergy(id, energy));
            Ok(())
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "CraftCannotDie",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("CraftCannotDie", &args, 2)?;
            let Value::Boolean(on) = &args[1] else {
                return Err(mlua::Error::runtime("A2.CraftCannotDie requires a boolean"));
            };
            live(&c, "CraftCannotDie", id)?;
            c.borrow_mut().edit(Edit::Immortal(id, *on));
            Ok(())
        })?,
    )?;

    table.set("Cloak", cloak_function(lua, ctx, "Cloak", true)?)?;
    table.set("Decloak", cloak_function(lua, ctx, "Decloak", false)?)?;
    Ok(())
}

fn cloak_function(
    lua: &Lua,
    ctx: &Shared,
    name: &'static str,
    on: bool,
) -> mlua::Result<mlua::Function> {
    let c = ctx.clone();
    lua.create_function(move |_, args: MultiValue| {
        let id = handle(name, &args, 1)?;
        live(&c, name, id)?;
        if !c.borrow().unit(id).is_some_and(|u| u.owner.is_player()) {
            return Err(mlua::Error::runtime(format!(
                "A2.{name}: object {} has no player owner",
                id.0
            )));
        }
        c.borrow_mut().order(id, |units| Order::Cloak { units, on });
        Ok(())
    })
}

#[cfg(test)]
#[path = "objects/tests.rs"]
mod tests;
