//! Shared presentation bindings for converted A2 missions (LUA-05).
//!
//! Observed call shapes come from Immy's clean-room `armada_missions`
//! conversions and `A2FOMissions/missions/Recovery.lua`, not MMM bodies.
//! Presentation output uses `Edit::Present`; this module does not play media
//! or move the client's camera. A bounded DLL-named `A2` object API maps
//! integer handles to the current mission view and sim commands.
//!
//! The typed seam cannot yet describe positioned titles or camera path
//! parameters. Those requests use `Presentation::Call`, retaining arguments
//! for a future consumer rather than pretending they have rendered.

use mlua::Lua;

use super::Shared;

mod camera;
mod conditions;
mod controls;
mod media;
mod native_camera;
mod native_presentation;
mod objectives;
mod objects;
mod orders;
mod queries;
mod random;
mod subsystems;
#[cfg(test)]
mod tests;
mod titles;

pub(super) fn cineractive_active(lua: &Lua) -> mlua::Result<bool> {
    camera::active(lua)
}

pub(super) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    camera::install(lua, ctx)?;
    media::install(lua, ctx)?;
    titles::install(lua, ctx)?;
    let a2 = lua.create_table()?;
    native_camera::install(lua, ctx, &a2)?;
    native_presentation::install(lua, ctx, &a2)?;
    controls::install(lua, ctx, &a2)?;
    objectives::install(lua, ctx, &a2)?;
    objects::install(lua, ctx, &a2)?;
    queries::install(lua, ctx, &a2)?;
    random::install(lua, ctx, &a2)?;
    a2.set("paths", lua.create_table()?)?;
    conditions::install(lua, ctx, &a2)?;
    subsystems::install(lua, ctx, &a2)?;
    // Last: it wraps every function installed above to resolve `BuildObject` handles.
    orders::install(lua, ctx, &a2)?;
    lua.globals().raw_set("A2", a2)?;
    Ok(())
}
