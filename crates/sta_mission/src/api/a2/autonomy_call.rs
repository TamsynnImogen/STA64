//! `SetCommandMoveAutonomyLow` (LUA-36).
//!
//! Rule, read from the executable, static (Armada II 1.1 stock): the call looks the handle up
//! and, for a live object that is not dying, issues the engine command MOVE_LOW (49) to it. It
//! tests no owner and no special-forces flag, unlike the script's order calls. The craft
//! process's new-command handler then writes the craft's movement autonomy: 49 sets low (0),
//! the next two commands medium (1) and high (2). Here: an `Order::Autonomy` movement level
//! `Low` from the script seat, which the sim applies to a living craft only. A handle that
//! names nothing live does nothing. What a low movement autonomy changes in a running game is
//! not measured (SIM-36's rules read the stored level).

use mlua::{Lua, Table};
use sta_sim::command::{AutonomyKind, AutonomyLevel, Order};

use super::orders::{Env, def, nothing};
use crate::api::Shared;

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let env = Env { ctx: ctx.clone() };
    table.set(
        "SetCommandMoveAutonomyLow",
        def(lua, &env, "SetCommandMoveAutonomyLow", &["h"], |_, e, _, a| {
            let Some(unit) = e.alive(a[0].id()) else { return nothing() };
            e.script_order(
                "SetCommandMoveAutonomyLow",
                Order::Autonomy { units: vec![unit.id], kind: AutonomyKind::Movement, level: AutonomyLevel::Low },
            )?;
            nothing()
        })?,
    )
}
