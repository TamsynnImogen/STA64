//! Read-only DLL-shaped object queries over the latest mission view.

use mlua::{Lua, MultiValue, Table, Value};

use super::objects::handle;
use crate::api::Shared;

fn odf_stem(name: &str) -> &str {
    match name.get(name.len().saturating_sub(4)..) {
        Some(suffix) if suffix.eq_ignore_ascii_case(".odf") => &name[..name.len() - 4],
        _ => name,
    }
}

fn health(lua: &Lua, ctx: &Shared, name: &'static str, max: bool) -> mlua::Result<mlua::Function> {
    let c = ctx.clone();
    lua.create_function(move |_, args: MultiValue| {
        let id = handle(name, &args, 1)?;
        c.borrow()
            .alive(id)
            .map(|u| u.hull[if max { 1 } else { 0 }].to_f32() as f64)
            .ok_or_else(|| mlua::Error::runtime(format!("A2.{name}: object {} is not alive", id.0)))
    })
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    table.set("GetCurrentHealth", health(lua, ctx, "GetCurrentHealth", false)?)?;
    table.set("GetMaxHealth", health(lua, ctx, "GetMaxHealth", true)?)?;
    let c = ctx.clone();
    table.set(
        "IsOdf",
        lua.create_function(move |_, args: MultiValue| {
            let id = handle("IsOdf", &args, 2)?;
            let Value::String(odf) = &args[1] else {
                return Err(mlua::Error::runtime("A2.IsOdf requires an ODF string"));
            };
            let odf = odf.to_str()?;
            let c = c.borrow();
            let Some(unit) = c.alive(id) else { return Ok(false) };
            if c.def(unit).is_none() {
                return Err(mlua::Error::runtime(format!(
                    "A2.IsOdf: object {} has no unit definition",
                    id.0
                )));
            }
            Ok(odf_stem(&c.odf_name(unit)).eq_ignore_ascii_case(odf_stem(&odf)))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "queries/tests.rs"]
mod tests;
