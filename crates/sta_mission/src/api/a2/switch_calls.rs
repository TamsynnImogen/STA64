//! A2 mission-wide switches (LUA-41): `AllowWarp(int)`, `CollisionPrevention(bool)` and
//! `PathPlanning(bool)`, each an `Edit::MissionSwitch` (global, no team).
//!
//! Read from the executable, static (Armada II 1.1); nothing measured in game:
//!
//! - `AllowWarp(n)` stores the integer in one global that is read by every team's craft, starts
//!   at 1, and returns the previous value. The sim only needs on or off, so the edit sends
//!   `n != 0`; the call remembers the integer it was last given (a script that stores 2 and
//!   asks again reads 2 back). A value never written by this binding reads as 1 or 0 from the
//!   view (a mission loaded from a save).
//! - `CollisionPrevention(b)` and `PathPlanning(b)` return nothing (see the sim's
//!   `mission_switches` for what they gate). `ObstacleAvoidance` writes a flag nothing reads, so
//!   it stays a no-op in `shared_calls`.

use std::cell::Cell;
use std::rc::Rc;

use mlua::{Lua, MultiValue, Table, Value};
use sta_sim::command::{Edit, MissionSwitch};

use crate::api::{Shared, edit_feedback};

fn int(name: &str, v: &Value) -> mlua::Result<i64> {
    match v {
        Value::Integer(n) if i32::try_from(*n).is_ok() => Ok(*n),
        Value::Number(n) if n.fract() == 0.0 && n.abs() < 2_147_483_648.0 => Ok(*n as i64),
        _ => Err(mlua::Error::runtime(format!("A2.{name} requires a signed 32-bit integer"))),
    }
}

fn boolean(name: &str, v: &Value) -> mlua::Result<bool> {
    match v {
        Value::Boolean(b) => Ok(*b),
        _ => Err(mlua::Error::runtime(format!("A2.{name} requires a boolean"))),
    }
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let last: Rc<Cell<Option<i64>>> = Rc::default();
    let c = ctx.clone();
    table.set(
        "AllowWarp",
        lua.create_function(move |_, args: MultiValue| {
            if args.len() != 1 {
                return Err(mlua::Error::runtime("A2.AllowWarp requires exactly 1 argument(s)"));
            }
            let n = int("AllowWarp", &args[0])?;
            let mut c = c.borrow_mut();
            let old = last.get().unwrap_or_else(|| i64::from(c.view.attrs.mission_switches.allow_warp));
            edit_feedback::queue(&mut c, "AllowWarp", Edit::MissionSwitch(MissionSwitch::AllowWarp, n != 0))?;
            last.set(Some(n));
            Ok(old)
        })?,
    )?;
    for (name, which) in [("CollisionPrevention", MissionSwitch::CollisionPrevention), ("PathPlanning", MissionSwitch::PathPlanning)] {
        let c = ctx.clone();
        table.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                if args.len() != 1 {
                    return Err(mlua::Error::runtime(format!("A2.{name} requires exactly 1 argument(s)")));
                }
                let on = boolean(name, &args[0])?;
                edit_feedback::queue(&mut c.borrow_mut(), name, Edit::MissionSwitch(which, on))
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sta_content::{MapDef, fake};
    use sta_core::{PlayerId, TeamId};
    use sta_sim::command::{Command, CommandKind, Edit, MissionSwitch};
    use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use sta_sim::{Sim, WorldSource};

    use crate::api::MissionConfig;
    use crate::runtime::MissionRuntime;

    fn world() -> (Sim, MissionRuntime) {
        let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Idle };
        let mut sim = Sim::from_setup(&GameSetup {
            seed: 3,
            map: Arc::new(MapDef::empty()),
            content: Arc::new(fake::db()),
            players: vec![seat(0), seat(1)],
            options: GameOptions::default(),
        });
        let view = (*sim.step(&[]).view).clone();
        let cfg = MissionConfig { content: Some(Arc::new(fake::db())), view: Some(view), ..MissionConfig::default() };
        let rt = MissionRuntime::from_source_with("sw.lua", "", 3, cfg);
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        (sim, rt)
    }

    #[test]
    fn allow_warp_returns_the_old_value_and_reaches_the_sim() {
        let (mut sim, mut rt) = world();
        assert_eq!(rt.eval::<i64>("return A2.AllowWarp(0)").unwrap(), 1, "starts on");
        assert_eq!(rt.eval::<i64>("return A2.AllowWarp(2)").unwrap(), 0, "the call before it wrote 0");
        assert_eq!(rt.eval::<i64>("return A2.AllowWarp(0)").unwrap(), 2, "the integer is kept");
        let commands: Vec<Command> = rt.take_commands();
        assert_eq!(commands.len(), 3);
        assert!(matches!(commands[0].kind, CommandKind::Edit(Edit::MissionSwitch(MissionSwitch::AllowWarp, false))));
        assert!(matches!(commands[1].kind, CommandKind::Edit(Edit::MissionSwitch(MissionSwitch::AllowWarp, true))));
        sim.step(&commands);
        assert!(!sim.mission_switches().allow_warp, "the last call wins");
        assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
    }

    #[test]
    fn the_other_switches_send_their_edit_and_check_their_argument() {
        let (mut sim, mut rt) = world();
        rt.eval::<()>("A2.CollisionPrevention(false); A2.PathPlanning(true)").unwrap();
        for bad in ["A2.AllowWarp()", "A2.AllowWarp(true)", "A2.AllowWarp(1.5)", "A2.CollisionPrevention(1)", "A2.PathPlanning()"] {
            assert!(rt.eval::<()>(bad).is_err(), "{bad}");
        }
        let commands = rt.take_commands();
        assert_eq!(commands.len(), 2);
        sim.step(&commands);
        let s = sim.mission_switches();
        assert!(!s.collision_prevention && s.allow_warp);
        assert_eq!(s.path_planning, Some(true));
        assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
    }
}
