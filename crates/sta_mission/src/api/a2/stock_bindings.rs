//! Three stock-mission calls of the Armada II `ScriptInterface` (LUA-19):
//! `AllStationsDestroyed`, `GetNextCraft` and `CommandAllShips`.
//!
//! Rules, each from the executable's own `ScriptInterfaceImp` method body
//! (read from the executable, static; nothing measured in game) unless a line
//! says `not measured`. Every call is served by an existing seam: no new
//! order or edit, no persisted state.
//!
//! - `AllStationsDestroyed(team)` is `CountStations(team) == 0`: the engine
//!   method calls its own `CountStations` and tests the result for zero. It
//!   therefore shares that call's classification here (`IsStation`), and its
//!   team handling (an unknown team raises).
//! - `GetFirstCraft()` / `GetNextCraft()` walk one list with one cursor that
//!   lives in the script interface, not in the script: `GetFirstCraft` puts
//!   the cursor on the first element, `GetNextCraft` moves it on by one and
//!   returns the handle there, or 0 once it is past the last. The list is the
//!   engine's craft list in its insertion order; STA64 takes it as the world
//!   view's order (ascending id) and its membership from `GetFirstCraft` (live,
//!   complete, not a planet-like object). Not measured: the list's membership
//!   for dying or unfinished objects (the engine does not skip them), and what
//!   the engine does after the end or with no `GetFirstCraft` before it (its
//!   cursor is then stale): here both return 0.
//! - `CommandAllShips(command, team, odf, flag_a, flag_b)` issues the command
//!   to every craft of the list that is not dying and not a derelict, whose
//!   real team is `team` (-1: any), whose class is `odf` (null: any), that is
//!   not a station when `flag_b` is set, and that has an unidentified
//!   per-craft state byte set when `flag_a` is set. The command is given with
//!   no target, the same way `Stop(unit)` and `AI_Attack(unit)` give theirs
//!   (read from the executable, static), so only `STOP` (3) and `ATTACK` (6,
//!   without a target: `Order::AttackAny`) have a seam; any other command raises and is
//!   counted before anything is queued. `flag_a == true` raises and is
//!   counted too (the state byte is not identified). The derelict test is the
//!   crew approximation `IsCompromised` uses. Craft with no player owner (the
//!   neutral seat) cannot be ordered and are skipped. Orders go out in
//!   ascending id as each craft's owner, like `Stop`.

use std::cell::Cell;
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table, Value};
use sta_core::{EntityId, Fixed};
use sta_sim::command::Order;
use sta_sim::view::{UnitFlags, UnitView};

use super::queries::craft_family;
use crate::api::search::odf_ok;
use crate::api::{Ctx, Shared};

/// `AiCommand` values of the executable's name table.
const CMD_STOP: i64 = 3;
const CMD_ATTACK: i64 = 6;
/// Highest `AiCommand` value.
const CMD_LAST: i64 = 56;

/// What `GetNextCraft` returns when no list walk is in progress (before any
/// `GetFirstCraft`, or after the end). Not measured: the engine's cursor is
/// stale or past the end there.
const NO_CRAFT: i64 = 0;

/// Where the script interface's craft cursor stands.
#[derive(Clone, Copy)]
enum Cursor {
    /// No walk started.
    Idle,
    /// The element last returned.
    At(EntityId),
    /// Past the last element.
    End,
}

fn gap(ctx: &Shared, name: &str, why: &str) -> mlua::Error {
    ctx.borrow_mut().unsupported(&format!("A2.{name}: {why}"));
    mlua::Error::runtime(format!("A2.{name} is not available: {why}"))
}

fn bad(name: &str, i: usize, want: &str) -> mlua::Error {
    mlua::Error::runtime(format!("A2.{name} argument {} must be {want}", i + 1))
}

fn arity(name: &str, args: &MultiValue, n: usize) -> mlua::Result<()> {
    if args.len() == n {
        Ok(())
    } else {
        Err(mlua::Error::runtime(format!("A2.{name} requires exactly {n} argument(s)")))
    }
}

fn int(name: &str, args: &MultiValue, i: usize) -> mlua::Result<i64> {
    match &args[i] {
        Value::Integer(n) if i32::try_from(*n).is_ok() => Ok(*n),
        Value::Number(n) if n.is_finite() && n.fract() == 0.0 && n.abs() < f64::from(i32::MAX) => Ok(*n as i64),
        _ => Err(bad(name, i, "a signed 32-bit integer")),
    }
}

fn boolean(name: &str, args: &MultiValue, i: usize) -> mlua::Result<bool> {
    match &args[i] {
        Value::Boolean(b) => Ok(*b),
        _ => Err(bad(name, i, "a boolean")),
    }
}

/// A `char*`: a string, or nil / 0 for the null pointer.
fn text(name: &str, args: &MultiValue, i: usize) -> mlua::Result<Option<String>> {
    match &args[i] {
        Value::Nil | Value::Integer(0) => Ok(None),
        Value::Number(n) if *n == 0.0 => Ok(None),
        Value::String(s) => {
            let s = s.to_str().map_err(|_| bad(name, i, "a UTF-8 string"))?;
            if s.contains('\0') {
                return Err(bad(name, i, "a string without NUL"));
            }
            Ok(Some(s.to_owned()))
        }
        _ => Err(bad(name, i, "a string, nil or 0")),
    }
}

/// A member of the craft list: what `GetFirstCraft` returns (keep in step
/// with it; not measured for dying or unfinished objects).
fn listed(c: &Ctx, u: &UnitView) -> bool {
    !u.flags.contains(UnitFlags::DYING)
        && u.progress.is_none()
        && c.def(u).is_none_or(|d| craft_family(&d.class.0) != Some(false))
}

/// The crew approximation of the engine's derelict byte (as `IsCompromised`).
fn derelict(u: &UnitView) -> bool {
    u.crew.is_some_and(|[now, max]| now < Fixed::ONE && max > Fixed::ZERO)
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    all_stations_destroyed(lua, table)?;
    craft_walk(lua, ctx, table)?;
    command_all_ships(lua, ctx, table)
}

/// `bool AllStationsDestroyed(int team)`. Needs `CountStations` installed.
fn all_stations_destroyed(lua: &Lua, table: &Table) -> mlua::Result<()> {
    let count: Function = table.get("CountStations")?;
    table.set(
        "AllStationsDestroyed",
        lua.create_function(move |_, args: MultiValue| {
            arity("AllStationsDestroyed", &args, 1)?;
            int("AllStationsDestroyed", &args, 0)?;
            let n: i64 = count.call(args).map_err(|e| mlua::Error::runtime(e.to_string()))?;
            Ok(n == 0)
        })?,
    )
}

/// `int GetNextCraft(void)`, with `GetFirstCraft` wrapped to start the cursor.
/// Needs `GetFirstCraft` installed.
fn craft_walk(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let cursor = Rc::new(Cell::new(Cursor::Idle));
    let first: Function = table.get("GetFirstCraft")?;
    let k = cursor.clone();
    table.set(
        "GetFirstCraft",
        lua.create_function(move |_, args: MultiValue| {
            let id: i64 = first.call(args)?;
            k.set(u32::try_from(id).ok().filter(|n| *n != 0).map_or(Cursor::End, |n| Cursor::At(EntityId(n))));
            Ok(id)
        })?,
    )?;
    let c = ctx.clone();
    table.set(
        "GetNextCraft",
        lua.create_function(move |_, args: MultiValue| {
            arity("GetNextCraft", &args, 0)?;
            let Cursor::At(after) = cursor.get() else { return Ok(NO_CRAFT) };
            let c = c.borrow();
            // The view lists units in ascending id, so the next member is the
            // first listed one above the cursor (a unit gone since is skipped).
            match c.view.units.iter().find(|u| u.id > after && listed(&c, u)) {
                Some(u) => {
                    cursor.set(Cursor::At(u.id));
                    Ok(i64::from(u.id.0))
                }
                None => {
                    cursor.set(Cursor::End);
                    Ok(NO_CRAFT)
                }
            }
        })?,
    )
}

/// `void CommandAllShips(AiCommand, int team, char* odf, bool, bool)`.
/// Needs `IsStation` installed.
fn command_all_ships(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    const NAME: &str = "CommandAllShips";
    let is_station: Function = table.get("IsStation")?;
    let c = ctx.clone();
    table.set(
        NAME,
        lua.create_function(move |_, args: MultiValue| {
            arity(NAME, &args, 5)?;
            let command = int(NAME, &args, 0)?;
            let team = int(NAME, &args, 1)?;
            let odf = text(NAME, &args, 2)?;
            let only_flagged = boolean(NAME, &args, 3)?;
            let skip_stations = boolean(NAME, &args, 4)?;
            if !(0..=CMD_LAST).contains(&command) {
                return Err(bad(NAME, 0, &format!("an AiCommand value 0..={CMD_LAST}")));
            }
            let owner = if team == -1 {
                None
            } else {
                let b = c.borrow();
                let seat = u8::try_from(team)
                    .ok()
                    .and_then(|n| b.player_numbering.decode(n))
                    .filter(|p| p.is_player() || *p == sta_sim::world::NEUTRAL)
                    .ok_or_else(|| mlua::Error::runtime(format!("A2.{NAME} requires a valid team number")))?;
                Some(seat)
            };
            let order: fn(Vec<EntityId>) -> Order = match command {
                CMD_STOP => |units| Order::Stop { units },
                CMD_ATTACK => |units| Order::AttackAny { units },
                n => return Err(gap(&c, NAME, &format!("command {n} has no order without a target"))),
            };
            if only_flagged {
                return Err(gap(&c, NAME, "the per-craft state the first flag selects on is not identified"));
            }
            let candidates: Vec<EntityId> = {
                let b = c.borrow();
                b.view
                    .units
                    .iter()
                    .filter(|u| listed(&b, u) && !derelict(u) && u.owner.is_player())
                    .filter(|u| owner.is_none_or(|o| u.owner == o))
                    .filter(|u| odf_ok(&b, u, odf.as_deref()))
                    .map(|u| u.id)
                    .collect()
            };
            // The station filter can raise (an unclassified class), so it
            // runs here, while the list is built; orders are queued only once
            // the whole list is known.
            let mut targets = Vec::with_capacity(candidates.len());
            for id in candidates {
                if skip_stations && is_station.call::<bool>(i64::from(id.0)).map_err(|e| mlua::Error::runtime(e.to_string()))? {
                    continue;
                }
                targets.push(id);
            }
            for id in targets {
                c.borrow_mut().order(id, order);
            }
            Ok(())
        })?,
    )
}

#[cfg(test)]
mod tests;
