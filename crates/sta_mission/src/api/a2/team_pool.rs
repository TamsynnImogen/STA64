//! A2 team crew and officer calls (SIM-54), with the engine's clamps.
//!
//! Read from the executable, static (Armada II 1.1 `ScriptInterfaceImp`,
//! identical in the Roots and Fleet Operations 3.2.7 engines); nothing
//! measured in game:
//!
//! - `AddCrew(team, x)` is `Team::AddCrew(x)`: over capacity a positive add is
//!   dropped and a negative one applied, otherwise the sum clamped to the
//!   team's crew capacity, no floor at 0. `SetTeamCrew(team, v)` is
//!   `AddCrew(v - pool)`. Both return the pool afterwards. (Under the
//!   "unlimited" starting resources the original does nothing; the sim has no
//!   such setting.) The edit lands on the next tick, so the returned number is
//!   what the rule gives from the pool in the current view.
//! - `GetMaxCrew(h)` ignores its handle and returns the header's `MAX_CREW`.
//! - `AddOfficers` and `SetOfficers` are stubs that return 0.0 and change
//!   nothing; `GetOfficers(team)` returns the *vacancies* (cap minus officers
//!   in use) and `GetOfficerCapacity(team)` the cap.

use mlua::{Lua, MultiValue, Table, Value};
use sta_core::{Fixed, PlayerId};
use sta_sim::command::{Edit, TeamOp};
use sta_sim::team_pool::crew_pool_after;

use crate::api::{Ctx, Shared, edit_feedback};

fn arg_error(name: &str, i: usize, want: &str) -> mlua::Error {
    mlua::Error::runtime(format!("A2.{name} argument {} must be {want}", i + 1))
}

fn check(name: &str, args: &MultiValue, kinds: &str) -> mlua::Result<()> {
    if args.len() != kinds.len() {
        return Err(mlua::Error::runtime(format!("A2.{name} requires exactly {} argument(s)", kinds.len())));
    }
    for (i, (k, v)) in kinds.chars().zip(args.iter()).enumerate() {
        let ok = match (k, v) {
            ('f', Value::Integer(_)) => true,
            ('f', Value::Number(n)) => n.is_finite(),
            (_, Value::Integer(n)) => i32::try_from(*n).is_ok(),
            (_, Value::Number(n)) => n.is_finite() && n.fract() == 0.0 && n.abs() < 2_147_483_648.0,
            _ => false,
        };
        if !ok {
            return Err(arg_error(name, i, if k == 'f' { "a finite number" } else { "a signed 32-bit integer" }));
        }
    }
    Ok(())
}

fn number(v: &Value) -> f64 {
    match v {
        Value::Integer(n) => *n as f64,
        Value::Number(n) => *n,
        _ => 0.0,
    }
}

fn team(c: &Ctx, name: &str, args: &MultiValue) -> mlua::Result<PlayerId> {
    u8::try_from(number(&args[0]) as i64)
        .ok()
        .and_then(|n| c.player_numbering.decode(n))
        .filter(|p| p.is_player() && c.view.player(*p).is_some())
        .ok_or_else(|| mlua::Error::runtime(format!("A2.{name} requires a player team number")))
}

fn capacity(c: &Ctx, p: PlayerId) -> Option<Fixed> {
    c.view.attrs.seats.iter().find(|s| s.player == p).and_then(|s| s.crew_capacity)
}

fn floats(f: Fixed) -> f64 {
    f64::from(f.to_f32())
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let c = ctx.clone();
    table.set(
        "GetTeamCrew",
        lua.create_function(move |_, args: MultiValue| {
            check("GetTeamCrew", &args, "i")?;
            let c = c.borrow();
            let p = team(&c, "GetTeamCrew", &args)?;
            Ok(c.view.player(p).and_then(|v| v.crew).map_or(0.0, floats))
        })?,
    )?;
    for (name, op) in [("SetTeamCrew", TeamOp::CrewSetA2), ("AddCrew", TeamOp::CrewAddA2)] {
        let c = ctx.clone();
        table.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                check(name, &args, "if")?;
                let mut c = c.borrow_mut();
                let p = team(&c, name, &args)?;
                let x = Fixed::from_f64(number(&args[1]).clamp(-1.0e9, 1.0e9));
                let pool = c.view.player(p).and_then(|v| v.crew).unwrap_or(Fixed::ZERO);
                let after = capacity(&c, p).and_then(|cap| crew_pool_after(op, pool, x, cap)).unwrap_or(pool);
                edit_feedback::queue(&mut c, if name == "AddCrew" { "A2.AddCrew" } else { "A2.SetTeamCrew" }, Edit::TeamPool(p, op, x))?;
                Ok(floats(after))
            })?,
        )?;
    }
    let c = ctx.clone();
    table.set(
        "GetMaxCrew",
        lua.create_function(move |_, args: MultiValue| {
            check("GetMaxCrew", &args, "i")?;
            let mut c = c.borrow_mut();
            match c.content.as_ref().and_then(|db| db.rules().crew()).and_then(|r| r.max_crew) {
                Some(max) => Ok(max as f64),
                None => {
                    c.unsupported("A2.GetMaxCrew: the content has no MAX_CREW");
                    Err(mlua::Error::runtime("A2.GetMaxCrew is not available: the content has no MAX_CREW"))
                }
            }
        })?,
    )?;
    for name in ["AddOfficers", "SetOfficers"] {
        table.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                check(name, &args, "if")?;
                Ok(0.0)
            })?,
        )?;
    }
    for (name, vacancies) in [("GetOfficers", true), ("GetOfficerCapacity", false)] {
        let c = ctx.clone();
        table.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                check(name, &args, "i")?;
                let c = c.borrow();
                let p = team(&c, name, &args)?;
                let [used, cap] = c.view.player(p).map_or([Fixed::ZERO; 2], |v| v.officers);
                Ok(floats(if vacancies { Fixed::from_bits(cap.to_bits().saturating_sub(used.to_bits())) } else { cap }))
            })?,
        )?;
    }
    Ok(())
}
