//! A2 calls with no sim seam behind them that converted missions make at
//! start-up and during intros (LUA-11): global switches, the culling distance,
//! sound muting, the background geometry, the fade removal, team relations and
//! resource writes. Each validates its arguments by the DLL's signature (read
//! from the executable, static: the decorated names), retains the request as a
//! `Presentation::Call` for a future consumer and, for the ones that change
//! game state, is counted in `unsupported()`; none raises. What each does in
//! the original is not measured; nothing here claims an effect.

use mlua::{Lua, MultiValue, ObjectLike, Table, Value};
use sta_sim::present::Arg;

use super::native_camera::{arity, boolean, number, submit, text};
use crate::api::{Shared, ai_switch};

/// One argument by its signature letter: `n` float, `b` bool, `s` string,
/// `i` signed 32-bit integer.
fn arg(name: &str, kind: char, v: &Value) -> mlua::Result<Arg> {
    match kind {
        'n' => number(name, v),
        'b' => boolean(name, v),
        's' => text(name, v),
        _ => match v {
            Value::Integer(n) if i32::try_from(*n).is_ok() => Ok(Arg::Int(*n)),
            Value::Number(n) if n.fract() == 0.0 && n.abs() < 2_147_483_648.0 => Ok(Arg::Int(*n as i64)),
            _ => Err(mlua::Error::runtime(format!("A2.{name} requires a signed 32-bit integer"))),
        },
    }
}

/// `why`: `None` for a presentation hint, else the reason it has no effect
/// (counted as unsupported).
fn call(lua: &Lua, ctx: &Shared, table: &Table, name: &'static str, sig: &'static str, why: Option<&'static str>) -> mlua::Result<()> {
    let c = ctx.clone();
    table.set(
        name,
        lua.create_function(move |_, args: MultiValue| {
            arity(name, &args, sig.len())?;
            let args = sig.chars().zip(args.iter()).map(|(k, v)| arg(name, k, v)).collect::<mlua::Result<Vec<_>>>()?;
            submit(&c, name, args)?;
            if let Some(why) = why {
                c.borrow_mut().unsupported(&format!("A2.{name}: {why}"));
            }
            Ok(())
        })?,
    )
}

/// The AI switches (LUA-20): validated by the DLL's signature and retained as a presentation call
/// like the others here, and queued as one request each for the host's AI hub
/// (`api::ai_switch`; unlike the calls above they are not counted as unsupported).
fn ai_switches(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    type Effect = fn(&mut crate::api::Ctx, &[Arg]);
    let switches: [(&'static str, &'static str, Effect); 6] = [
        ("DisableAllAITeams", "", |c, _| ai_switch::enable_all(c, false)),
        ("EnableAllAITeams", "", |c, _| ai_switch::enable_all(c, true)),
        ("EnableTeam", "i", |c, a| ai_switch::enable(c, team_arg(c, a), true)),
        ("DisableTeam", "i", |c, a| ai_switch::enable(c, team_arg(c, a), false)),
        ("SetIsAITeam", "ib", |c, a| {
            let team = team_arg(c, a);
            ai_switch::set_is_ai(c, "A2.SetIsAITeam", team, matches!(a.get(1), Some(Arg::Bool(true))));
        }),
        ("ResetStrategyCycle", "", |c, _| ai_switch::replan(c)),
    ];
    for (name, sig, effect) in switches {
        let c = ctx.clone();
        table.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                arity(name, &args, sig.len())?;
                let args = sig.chars().zip(args.iter()).map(|(k, v)| arg(name, k, v)).collect::<mlua::Result<Vec<_>>>()?;
                submit(&c, name, args.clone())?;
                effect(&mut c.borrow_mut(), &args);
                Ok(())
            })?,
        )?;
    }
    Ok(())
}

fn team_arg(c: &crate::api::Ctx, a: &[Arg]) -> Option<sta_core::PlayerId> {
    match a.first() {
        Some(Arg::Int(n)) => ai_switch::seat(c, *n),
        _ => None,
    }
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    call(lua, ctx, table, "MuteSounds", "n", None)?;
    call(lua, ctx, table, "FadeRemove", "", None)?;
    call(lua, ctx, table, "Starfield_Load_Background_Geometry", "s", None)?;
    call(lua, ctx, table, "SetObjectCullingDistance", "n", None)?;
    call(lua, ctx, table, "RestoreObjectCullingDistance", "", None)?;
    for name in ["ObstacleAvoidance", "PathPlanning", "CollisionPrevention", "FerengiCanSteal", "FerengiCanTrade"] {
        call(lua, ctx, table, name, "b", Some("no global switch in the sim seam"))?;
    }
    ai_switches(lua, ctx, table)?;
    for name in ["SetDilithium", "AddDilithium", "SetMetal", "AddMetal"] {
        call(lua, ctx, table, name, "ii", Some("no resource edit for A2 team numbering in the mission API yet"))?;
    }
    call(lua, ctx, table, "SetClippingDistance", "n", None)?;
    call(lua, ctx, table, "MessageDisplay_Set_Text_From_File", "s", None)?;
    for (name, sig, why) in [
        ("AllowWarp", "i", "no warp permission edit in the sim seam"),
        ("DisableChat", "b", "chat is not a sim feature"),
        ("DisableSave", "", "no save lock in the mission API"),
        ("EnableSave", "", "no save lock in the mission API"),
        ("EnableWormholeTravel", "ib", "no wormhole travel switch in the sim seam"),
        ("AllowMultipleSpecialWeaponFire", "ib", "no special-weapon switch in the sim seam"),
        ("ShowTradesTo", "i", "no trade display in the mission API"),
        ("SetTeamDistressCallRange", "in", "no AI distress range in the sim seam"),
        ("ClearShroud", "", "no shroud edit in the sim seam"),
        ("SetLatinum", "ii", "no resource edit for A2 team numbering in the mission API yet"),
        ("AddLatinum", "ii", "no resource edit for A2 team numbering in the mission API yet"),
        ("SetBiomatter", "ii", "no resource edit for A2 team numbering in the mission API yet"),
        ("AddBiomatter", "ii", "no resource edit for A2 team numbering in the mission API yet"),
    ] {
        call(lua, ctx, table, name, sig, Some(why))?;
    }
    let c = ctx.clone();
    table.set(
        "StopMovie",
        lua.create_function(move |_, args: MultiValue| {
            arity("StopMovie", &args, 0)?;
            c.borrow_mut()
                .try_push(sta_core::PlayerId::SCRIPT, sta_sim::command::CommandKind::Edit(sta_sim::command::Edit::Present(sta_sim::present::Presentation::StopMovie)))
                .map_err(|reason| mlua::Error::runtime(format!("A2.StopMovie: {reason}")))?;
            c.borrow_mut().playback.movie = false;
            Ok(())
        })?,
    )?;
    // The end of a cineractive: the same singleton `Cineractive.active` drives.
    table.set(
        "CineractiveEnd",
        lua.create_function(move |lua, args: MultiValue| {
            arity("CineractiveEnd", &args, 0)?;
            let singleton: mlua::AnyUserData = lua.globals().raw_get("Cineractive")?;
            singleton.set("active", false)?;
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::api::MissionConfig;
    use crate::runtime::MissionRuntime;

    #[test]
    fn calls_are_checked_by_signature_and_counted_when_they_change_state() {
        let mut rt = MissionRuntime::from_source_with("s.lua", "", 1, MissionConfig::default());
        rt.eval::<()>("A2.MuteSounds(5.0); A2.FadeRemove(); A2.ObstacleAvoidance(false); A2.SetRelation(1, 8, 2); A2.CineractiveEnd()")
            .unwrap();
        for bad in ["A2.MuteSounds()", "A2.ObstacleAvoidance(0)", "A2.SetRelation(1, 2)", "A2.Starfield_Load_Background_Geometry(3)"] {
            assert!(rt.eval::<()>(bad).is_err(), "{bad}");
        }
        assert_eq!(rt.take_commands().len(), 5, "every valid call retains its request (the cineractive end is the typed event)");
        let counted: Vec<String> = rt.unsupported().into_iter().map(|(k, _)| k).collect();
        assert!(counted.iter().any(|k| k.contains("A2.ObstacleAvoidance")));
        assert!(!counted.iter().any(|k| k.contains("SetRelation")), "SetRelation is a sim edit now");
        assert!(!counted.iter().any(|k| k.contains("MuteSounds")), "a presentation hint is not a gap");
    }
}
