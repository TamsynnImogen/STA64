//! LUA-31: Armada I scripted warp speed (`SetWarpSpeed`, `ResetWarpSpeed`,
//! `GetWarpSpeed`) and `GetImpulseSpeed`.
//!
//! Rule (read from the executable, static; not measured in game): the calls
//! act on a live craft only (the ship-bit guard, not dying); anything else,
//! an absent or dying handle included, does nothing and a getter answers 0.
//! `SetWarpSpeed(h, v)` sets the absolute warp speed, `ResetWarpSpeed(h)`
//! restores the class value; both are typed `Edit::WarpSpeed`s the sim applies
//! (see `sta_sim::warp_override`: kept between 0 and its ceiling, saved, reset
//! by the end of a cineractive). `GetWarpSpeed` is the craft's current absolute
//! warp speed; `GetImpulseSpeed` is the Armada II getter under the A1 name.

use mlua::{Lua, Table, Value};
use sta_sim::command::{CommandKind, Edit};
use sta_sim::present::Presentation;
use sta_sim::Fixed;

use super::{Env, alias, def};

/// A live craft's class warp speed, or `None` for anything else.
fn live_craft(e: &Env, id: sta_sim::EntityId) -> Option<(sta_sim::view::UnitView, Fixed)> {
    let c = e.ctx.borrow();
    let unit = c.alive(id)?;
    let d = c.def(unit)?;
    (sta_sim::world::EntityKind::of_def(d) == sta_sim::world::EntityKind::Craft).then(|| (unit.clone(), d.movement.warp_speed))
}

pub(super) fn install(lua: &Lua, env: &Env, a2: &Table, table: &Table) -> mlua::Result<()> {
    table.set("SetWarpSpeed", def(lua, env, "SetWarpSpeed", &["hf"], |_, e, _, a| {
        let id = a[0].id();
        if live_craft(e, id).is_some() {
            e.edit("SetWarpSpeed", Edit::WarpSpeed(id, Some(Fixed::from_f64(a[1].num()))))?;
        }
        Ok(Value::Nil)
    })?)?;
    table.set("ResetWarpSpeed", def(lua, env, "ResetWarpSpeed", &["h"], |_, e, _, a| {
        let id = a[0].id();
        if live_craft(e, id).is_some() {
            e.edit("ResetWarpSpeed", Edit::WarpSpeed(id, None))?;
        }
        Ok(Value::Nil)
    })?)?;
    // What a call queued this tick set (or a cineractive end queued after it reset), else the
    // view's override, else the class value.
    table.set("GetWarpSpeed", def(lua, env, "GetWarpSpeed", &["h"], |_, e, _, a| {
        let Some((unit, class)) = live_craft(e, a[0].id()) else { return Ok(Value::Number(0.0)) };
        let c = e.ctx.borrow();
        let queued = c.commands.iter().rev().find_map(|cmd| match &cmd.kind {
            CommandKind::Edit(Edit::WarpSpeed(who, to)) if *who == unit.id => {
                Some(to.map_or(class, |v| v.max(Fixed::ZERO).min(sta_sim::warp_override::MAX_WARP)))
            }
            CommandKind::Edit(Edit::Present(Presentation::Cineractive { on: false, .. })) => Some(class),
            _ => None,
        });
        let viewed = || c.view.attrs.warp.binary_search_by_key(&unit.id, |r| r.0).ok().map(|i| c.view.attrs.warp[i].1);
        let speed = queued.or_else(viewed).unwrap_or(class);
        Ok(Value::Number(f64::from(speed.to_f32())))
    })?)?;
    table.set("GetImpulseSpeed", alias(lua, env, a2, "GetImpulseSpeed")?)?;
    Ok(())
}
