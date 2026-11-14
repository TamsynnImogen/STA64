//! A2 team resource writes (LUA-22): `SetMetal`, `AddMetal`, `SetDilithium`, `AddDilithium`,
//! `SetLatinum`, `AddLatinum`, `SetBiomatter`, `AddBiomatter`, all `(team, amount)`.
//!
//! Read from the executable, static (Armada II 1.1 stock `ScriptInterfaceImp` and `Team`);
//! nothing measured in game:
//!
//! - The first argument is an A2 team number (the numbering of `AddCrew` and `GetMetal`), the
//!   amount a signed 32-bit int that the original converts to float; the calls return nothing.
//! - `Add*`: stock + amount; `Set*`: amount; both clamped to `0..MAX_<resource>`, so a negative
//!   add cannot go below 0. A team flagged "unlimited" ignores them (STA64 has no such setting).
//! - The `MAX_*` cap (`RTS_CFG.h`) is not applied: the rules have no accessor for it, so stored
//!   values clamp to `0..1e9` only. That the cap is the `RTS_CFG.h` value of the same resource is
//!   inferred, not established.
//! - A team number that no seat holds raises, like the sibling team calls (the original indexes
//!   a null team there, which is undefined: not established).
//!
//! The edit is `Edit::Cargo` (an absolute set of one seat's stock; resource ids 0 dilithium,
//! 1 latinum, 2 metal, 3 biomatter). The view is a snapshot and the edit lands next tick, so an
//! add builds on the value this binding last wrote for the seat in the same tick.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mlua::{Lua, MultiValue, Table, Value};
use sta_core::{Fixed, PlayerId, Tick};
use sta_sim::command::Edit;

use crate::api::{Shared, edit_feedback};

/// `(name, resource id, add)`; ids as in `economy::RESOURCES`.
const CALLS: [(&str, u8, bool); 8] = [
    ("SetDilithium", 0, false),
    ("AddDilithium", 0, true),
    ("SetLatinum", 1, false),
    ("AddLatinum", 1, true),
    ("SetMetal", 2, false),
    ("AddMetal", 2, true),
    ("SetBiomatter", 3, false),
    ("AddBiomatter", 3, true),
];

/// The most a stored value can hold here (no `MAX_*` accessor, see the module note).
const LIMIT: f64 = 1.0e9;

/// What each seat's stock was last set to by these calls, and in which tick.
type Written = Rc<RefCell<BTreeMap<(PlayerId, u8), (Tick, Fixed)>>>;

fn int(name: &str, i: usize, v: &Value) -> mlua::Result<i64> {
    match v {
        Value::Integer(n) if i32::try_from(*n).is_ok() => Ok(*n),
        Value::Number(n) if n.fract() == 0.0 && n.abs() < 2_147_483_648.0 => Ok(*n as i64),
        _ => Err(mlua::Error::runtime(format!("A2.{name} argument {} must be a signed 32-bit integer", i + 1))),
    }
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let written: Written = Rc::default();
    for (name, res, add) in CALLS {
        let c = ctx.clone();
        let written = written.clone();
        table.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                if args.len() != 2 {
                    return Err(mlua::Error::runtime(format!("A2.{name} requires exactly 2 argument(s)")));
                }
                let (team, amount) = (int(name, 0, &args[0])?, int(name, 1, &args[1])?);
                let mut c = c.borrow_mut();
                let player = u8::try_from(team)
                    .ok()
                    .and_then(|n| c.player_numbering.decode(n))
                    .filter(|p| p.is_player() && c.view.player(*p).is_some())
                    .ok_or_else(|| mlua::Error::runtime(format!("A2.{name} requires a player team number")))?;
                let tick = c.view.tick;
                let new = if add {
                    let base = match written.borrow().get(&(player, res)) {
                        Some((t, v)) if *t == tick => *v,
                        _ => c
                            .view
                            .player(player)
                            .and_then(|v| v.resources.get(usize::from(res)).copied())
                            .unwrap_or(Fixed::ZERO),
                    };
                    f64::from(base.to_f32()) + amount as f64
                } else {
                    amount as f64
                };
                let new = Fixed::from_f64(new.clamp(0.0, LIMIT));
                edit_feedback::queue(&mut c, name, Edit::Cargo(player, res, new))?;
                written.borrow_mut().insert((player, res), (tick, new));
                Ok(())
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "resource_calls/tests.rs"]
mod tests;
