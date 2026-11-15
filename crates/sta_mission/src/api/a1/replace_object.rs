//! `ReplaceObject(object, odf)` (LUA-30).
//!
//! Rule (read from the Armada I executable, static; not measured in play): for an
//! object that is present and not dying, the call copies its placement and owner,
//! removes it, builds `odf` for that owner at that place and returns the new
//! object's handle. A handle that names nothing live returns 0 and does nothing.
//! Here the removal is `Edit::Kill` (the sim's one destroy edit, as for
//! `RemoveObject`; the original removes silently, this one emits the Destroyed
//! event) and the build is a queued spawn, so the result is a pending handle exactly
//! as `BuildObject` returns (usable once the receipt is in). The spawn is queued
//! first: when it cannot be (unknown ODF, an owner that cannot hold the new
//! kind) the call raises and the old object stays. If the Kill then cannot be queued
//! (sequence exhausted) the call raises with the new object queued and the old alive. The old heading and the old
//! map label are not carried over (not read).

use mlua::{Lua, Table, Value};
use sta_sim::command::Edit;

use super::{Env, TOKEN_BASE, def};
use crate::api::entity_ref::EntityRef;

pub(super) fn install(lua: &Lua, env: &Env, table: &Table) -> mlua::Result<()> {
    table.set(
        "ReplaceObject",
        def(lua, env, "ReplaceObject", &["hs"], |_, e, _, a| {
            let id = a[0].id();
            let (owner, pos) = {
                let c = e.ctx.borrow();
                match c.alive(id) {
                    Some(u) => (u.owner, c.placed(id, u.pos)),
                    None => return Ok(Value::Integer(0)),
                }
            };
            let odf = a[1].text();
            let handle = {
                let mut c = e.ctx.borrow_mut();
                let key = c
                    .content
                    .as_ref()
                    .and_then(|db| db.key(super::objects::stem(odf)))
                    .ok_or_else(|| mlua::Error::runtime(format!("A1.ReplaceObject: unknown ODF '{odf}'")))?;
                let handle = crate::api::spawn::queue_spawn(&mut c, key, owner, pos).map_err(mlua::Error::runtime)?;
                c.try_push(sta_core::PlayerId::SCRIPT, sta_sim::command::CommandKind::Edit(Edit::Kill(id)))
                    .map_err(|reason| mlua::Error::runtime(format!("A1.ReplaceObject: {reason}")))?;
                // The original removes silently: the client shows no death effect for this Kill.
                crate::api::spawn::note_removed(&mut c, id);
                handle
            };
            let EntityRef::Spawn(spawn) = handle else {
                return Err(mlua::Error::runtime("A1.ReplaceObject: spawn was not queued"));
            };
            let token = u32::try_from(spawn.request)
                .ok()
                .and_then(|r| TOKEN_BASE.checked_add(r))
                .ok_or_else(|| mlua::Error::runtime("A1.ReplaceObject: spawn handles exhausted"))?;
            e.st.borrow_mut().pending.insert(token, spawn);
            Ok(Value::Integer(i64::from(token)))
        })?,
    )?;
    Ok(())
}
