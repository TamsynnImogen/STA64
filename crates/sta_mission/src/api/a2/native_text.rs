//! Exact A2 title calls: `ShowTitle`, `ShowSubtitle`, `RemoveTitle`,
//! `RemoveSubtitle`. (The `SetCommandWhat*VO` calls live in `orders.rs`.)
//!
//! `ShowTitle(x, y, label, fadeIn)` and `ShowSubtitle(...)` return a message id
//! (an `int`); the scripts keep it and pass it to `RemoveTitle(id, fadeOut)` /
//! `RemoveSubtitle(id, fadeOut)`. Both kinds share one overlay-text id counter
//! that starts at 0 (a zero-initialised engine counter, read from the
//! executable, static; whether a mission load resets it is not measured). The
//! label is a string-table key the client localizes (stock scripts pass keys
//! such as a mission title id, never prose).
//!
//! Each show queues the typed `Presentation::Title { id, text: label,
//! subtitle, seconds: None }` and then `Call A2.ShowTitle|ShowSubtitle
//! [Int id, Int x, Int y, Bool fade_in]`, as the A1 bridge does: the typed
//! seam has no placement or fade field. A negative `x` means "centred" (the
//! engine replaces it by half the screen width and centre-justifies; the stock
//! subtitles pass -1); the client applies it, the bridge keeps the raw value.
//! The final flag starts the message transparent so it fades in, else fully
//! visible. A title uses the title font and the Federation light-blue colour, a
//! subtitle the default font and white: client styling, not carried here.
//! Removal queues `RemoveTitle { id }` plus `Call A2.RemoveTitle|RemoveSubtitle
//! [Int id, Bool fade]` (true fades the message out, false removes it at
//! once). The two removal calls have identical bodies. An id that is not (or no
//! longer) on screen is ignored, as in the engine: stock scripts remove
//! titles they may already have removed.

use std::cell::RefCell;
use std::collections::BTreeSet;

use mlua::{Lua, MultiValue, Table, Value};
use sta_core::PlayerId;
use sta_sim::command::{CommandKind, Edit};
use sta_sim::present::{Arg, Presentation};

use super::native_camera::{arity, boolean, submit, text};
use crate::api::Shared;

#[derive(Default)]
struct Titles {
    next: u32,
    live: BTreeSet<u32>,
}

fn titles<R>(lua: &Lua, f: impl FnOnce(&mut Titles) -> R) -> R {
    if lua.app_data_ref::<RefCell<Titles>>().is_none() {
        lua.set_app_data(RefCell::new(Titles::default()));
    }
    let cell = lua.app_data_ref::<RefCell<Titles>>().expect("title state");
    let mut state = cell.borrow_mut();
    f(&mut state)
}

fn int32(name: &str, what: &str, value: &Value) -> mlua::Result<i64> {
    match value {
        Value::Integer(n) if i32::try_from(*n).is_ok() => Ok(*n),
        _ => Err(mlua::Error::runtime(format!("A2.{name} {what} must be a signed 32-bit integer"))),
    }
}

fn flag(name: &str, value: &Value) -> mlua::Result<bool> {
    match boolean(name, value)? {
        Arg::Bool(b) => Ok(b),
        _ => unreachable!("boolean() returns Bool"),
    }
}

fn push(ctx: &Shared, name: &str, event: Presentation) -> mlua::Result<()> {
    ctx.borrow_mut()
        .try_push(PlayerId::SCRIPT, CommandKind::Edit(Edit::Present(event)))
        .map(|_| ())
        .map_err(|reason| mlua::Error::runtime(format!("A2.{name}: {reason}")))
}

fn show(lua: &Lua, ctx: &Shared, name: &'static str, subtitle: bool) -> mlua::Result<mlua::Function> {
    let c = ctx.clone();
    lua.create_function(move |lua, args: MultiValue| {
        arity(name, &args, 4)?;
        let x = int32(name, "x", &args[0])?;
        let y = int32(name, "y", &args[1])?;
        let Arg::Text(label) = text(name, &args[2])? else { unreachable!("text() returns Text") };
        let fade_in = flag(name, &args[3])?;
        let id = titles(lua, |t| {
            let id = t.next;
            t.next = id
                .checked_add(1)
                .filter(|n| i32::try_from(*n).is_ok())
                .ok_or_else(|| mlua::Error::runtime(format!("A2.{name}: title ids exhausted")))?;
            Ok::<_, mlua::Error>(id)
        })?;
        push(&c, name, Presentation::Title { id, text: label, subtitle, seconds: None })?;
        submit(
            &c,
            name,
            vec![Arg::Int(i64::from(id)), Arg::Int(x), Arg::Int(y), Arg::Bool(fade_in)],
        )?;
        titles(lua, |t| t.live.insert(id));
        Ok(i64::from(id))
    })
}

fn remove(lua: &Lua, ctx: &Shared, name: &'static str) -> mlua::Result<mlua::Function> {
    let c = ctx.clone();
    lua.create_function(move |lua, args: MultiValue| {
        arity(name, &args, 2)?;
        let id = int32(name, "id", &args[0])?;
        let fade = flag(name, &args[1])?;
        let live = u32::try_from(id).is_ok_and(|id| titles(lua, |t| t.live.remove(&id)));
        if live {
            push(&c, name, Presentation::RemoveTitle { id: id as u32 })?;
            submit(&c, name, vec![Arg::Int(id), Arg::Bool(fade)])?;
        }
        Ok(())
    })
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    table.set("ShowTitle", show(lua, ctx, "ShowTitle", false)?)?;
    table.set("ShowSubtitle", show(lua, ctx, "ShowSubtitle", true)?)?;
    table.set("RemoveTitle", remove(lua, ctx, "RemoveTitle")?)?;
    table.set("RemoveSubtitle", remove(lua, ctx, "RemoveSubtitle")?)?;
    Ok(())
}

#[cfg(test)]
#[path = "native_text/tests.rs"]
mod tests;
