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
mod dll_objects;
mod controls;
mod handles;
mod media;
mod mission_end;
mod native_camera;
mod native_flow;
mod native_presentation;
mod native_text;
mod objectives;
mod objects;
mod fleets;
mod orders;
mod queries;
mod random;
mod shared_calls;
mod stock_bindings;
mod subsystems;
mod team_pool;
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
    native_flow::install(lua, ctx, &a2)?;
    native_text::install(lua, ctx, &a2)?;
    controls::install(lua, ctx, &a2)?;
    objectives::install(lua, ctx, &a2)?;
    objects::install(lua, ctx, &a2)?;
    queries::install(lua, ctx, &a2)?;
    random::install(lua, ctx, &a2)?;
    // `A2.paths[label] = { {x=, y=, z=}, ... }` from the map's `AiPaths` (CON-47):
    // the first path of a label wins (the engine looks a name up in list order).
    let paths = lua.create_table()?;
    for (label, points) in &ctx.borrow().paths {
        if paths.raw_get::<mlua::Value>(label.as_str())?.is_nil() {
            let list = lua.create_table()?;
            for (i, p) in points.iter().enumerate() {
                let point = lua.create_table()?;
                point.set("x", f64::from(p[0]))?;
                point.set("y", f64::from(p[1]))?;
                point.set("z", f64::from(p[2]))?;
                list.set(i + 1, point)?;
            }
            paths.set(label.as_str(), list)?;
        }
    }
    a2.set("paths", paths)?;
    conditions::install(lua, ctx, &a2)?;
    mission_end::install(lua, ctx, &a2)?;
    subsystems::install(lua, ctx, &a2)?;
    orders::install(lua, ctx, &a2)?;
    dll_objects::install(lua, ctx, &a2)?;
    // After `conditions` (CountStations, IsStation) and `dll_objects` (GetFirstCraft).
    stock_bindings::install(lua, ctx, &a2)?;
    handles::install(lua, ctx, &a2)?;
    shared_calls::install(lua, ctx, &a2)?;
    team_pool::install(lua, ctx, &a2)?;
    super::taint::wrap(lua, ctx, &a2)?;
    // Keep last: wraps every function above (pending BuildObject handles).
    dll_objects::finish(lua, &a2)?;
    lua.globals().raw_set("A2", a2)?;
    Ok(())
}
