//! `SucceedMission(seconds, text)` and `FailMission(seconds, text)` (LUA-13).
//!
//! Rule (read from the executable, static; not measured): a call stores
//! *done*, the end time and a failed flag in the mission object, overwriting an
//! earlier call, and reads nothing else (the text is unused). The mission keeps
//! running; once mission time is past the end time it ends once with the won
//! flag (`SucceedMission` is won). The stock scripts all pass the current
//! mission time and a null text.
//!
//! STA64 adds one rule of its own (details in [`crate::api::taint`]): an end is
//! *blocked* when an earlier condition query read an object that a failed or
//! unsupported call named (or a handle that never existed after a
//! handle-producing call was a gap, or a team count over such an object). The
//! end may then have been decided by a gap, not by the mission, so it is
//! reported with its reason and is neither a win nor a loss
//! ([`crate::runtime::MissionEndState::Blocked`]). With no such read the end
//! stands, whatever else is unsupported.

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::{MissionEnd, Shared};

fn end(ctx: &Shared, name: &'static str, failed: bool, args: &MultiValue) -> mlua::Result<()> {
    if args.len() != 2 {
        return Err(mlua::Error::runtime(format!("A2.{name} requires exactly 2 arguments (seconds, text)")));
    }
    let time = match &args[0] {
        Value::Integer(n) => *n as f64,
        Value::Number(n) if n.is_finite() => *n,
        _ => return Err(mlua::Error::runtime(format!("A2.{name} argument 1 must be a finite number of seconds"))),
    };
    match &args[1] {
        Value::Nil | Value::Integer(0) | Value::String(_) => {}
        _ => return Err(mlua::Error::runtime(format!("A2.{name} argument 2 must be a string, nil or 0"))),
    }
    let mut c = ctx.borrow_mut();
    let blocked = c.taint.reason().map(|why| format!("{name} after {why}"));
    c.mission_end = Some(MissionEnd { time, failed, blocked });
    Ok(())
}

pub(super) fn install(lua: &Lua, ctx: &Shared, a2: &Table) -> mlua::Result<()> {
    for (name, failed) in [("SucceedMission", false), ("FailMission", true)] {
        let c = ctx.clone();
        a2.set(name, lua.create_function(move |_, args: MultiValue| end(&c, name, failed, &args))?)?;
    }
    Ok(())
}
