//! Shared presentation bindings for converted A2 missions (LUA-05).
//!
//! Observed call shapes come from Immy's clean-room `armada_missions`
//! conversions and `A2FOMissions/missions/Recovery.lua`, not MMM bodies.
//! All output uses `Edit::Present`; this module does not play media or move
//! the client's camera. The DLL-named `A2` table is still pending.
//!
//! The typed seam cannot yet describe positioned titles or camera path
//! parameters. Those requests use `Presentation::Call`, retaining arguments
//! for a future consumer rather than pretending they have rendered.

use mlua::Lua;

use super::Shared;

mod camera;
mod media;
#[cfg(test)]
mod tests;
mod titles;

pub(super) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    camera::install(lua, ctx)?;
    media::install(lua, ctx)?;
    titles::install(lua, ctx)?;
    Ok(())
}
