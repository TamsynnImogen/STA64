//! Calls whose simulation seam does not exist yet, and mission flow.
//!
//! A **write** here validates its arguments (every handle must be a live
//! object), queues `Presentation::Call` named `A1.<Name>` with the arguments
//! in call order, adds `A1.<Name>: <why>` to `unsupported()` and changes
//! nothing else. A **query** raises a Lua error naming the missing data. The
//! reason strings are the PR's "missing seams" list.

use mlua::{Function, Lua, Table, Value};
use sta_sim::TICK_HZ;

use super::{Env, args_of, def};

fn gap_write(
    lua: &Lua,
    env: &Env,
    name: &'static str,
    specs: &'static [&'static str],
    why: &'static str,
) -> mlua::Result<Function> {
    def(lua, env, name, specs, move |_, e, _, a| {
        for value in a {
            if let super::A::Handle(id) = value {
                e.unit(name, *id)?;
            }
        }
        e.gap(name, why, args_of(a))?;
        Ok(Value::Nil)
    })
}

fn no_data(lua: &Lua, env: &Env, name: &'static str, spec: &'static [&'static str], why: &'static str) -> mlua::Result<Function> {
    def(lua, env, name, spec, move |_, e, _, a| {
        for value in a {
            if let super::A::Handle(id) = value {
                e.unit(name, *id)?;
            }
        }
        Err(e.no_data(name, why))
    })
}

pub(super) fn install(lua: &Lua, env: &Env, table: &Table) -> mlua::Result<()> {
    // Movement and position.
    table.set("SetPosition", gap_write(lua, env, "SetPosition", &["hs", "hsi"], "no teleport edit in the sim seam")?)?;
    table.set("SetWarpSpeed", gap_write(lua, env, "SetWarpSpeed", &["hf"], "no warp-speed seam in the sim")?)?;
    table.set("ResetWarpSpeed", gap_write(lua, env, "ResetWarpSpeed", &["h"], "no warp-speed seam in the sim")?)?;
    table.set("Patrol", gap_write(lua, env, "Patrol", &["hs"], "no patrol order in the sim seam")?)?;
    table.set("SearchAndDestroy", gap_write(lua, env, "SearchAndDestroy", &["h"], "no search-and-destroy order in the sim seam")?)?;
    table.set("PointAt", gap_write(lua, env, "PointAt", &["hh", "hs"], "no facing order in the sim seam")?)?;
    table.set("Special_Attack", gap_write(lua, env, "Special_Attack", &["hvs", "hhs", "hs"], "special weapons are named; the order takes a slot index")?)?;
    table.set("SetSpecialWeaponAttackInterval", gap_write(lua, env, "SetSpecialWeaponAttackInterval", &["if"], "no special-weapon interval seam in the sim")?)?;
    // AI team orders.
    table.set("AI_Attack", gap_write(lua, env, "AI_Attack", &["ii", "iv", "i"], "no AI team attack seam in the sim")?)?;
    table.set("AttackMainEnemyBase", gap_write(lua, env, "AttackMainEnemyBase", &["ib"], "no AI team attack seam in the sim")?)?;
    // Damage and shields.
    table.set("SetShieldPercent", gap_write(lua, env, "SetShieldPercent", &["hf"], "no shield edit in the sim seam")?)?;
    table.set("SetMaxShields", gap_write(lua, env, "SetMaxShields", &["hf"], "no maximum-shield edit in the sim seam")?)?;
    table.set("DestroyCritical", gap_write(lua, env, "DestroyCritical", &["h"], "no critical-system damage in the sim seam")?)?;
    table.set("DestroyShieldGenerator", gap_write(lua, env, "DestroyShieldGenerator", &["h"], "no subsystem destruction in the sim seam")?)?;
    table.set("DestroyLifeSupport", gap_write(lua, env, "DestroyLifeSupport", &["h"], "no subsystem destruction in the sim seam")?)?;
    table.set("DestroyWeapons", gap_write(lua, env, "DestroyWeapons", &["h"], "no subsystem destruction in the sim seam")?)?;
    table.set("DestroyEngines", gap_write(lua, env, "DestroyEngines", &["h"], "no subsystem destruction in the sim seam")?)?;
    table.set("DestroySensors", gap_write(lua, env, "DestroySensors", &["h"], "no subsystem destruction in the sim seam")?)?;
    table.set("AllowMultipleSpecialWeaponFire", gap_write(lua, env, "AllowMultipleSpecialWeaponFire", &["hb"], "no special-weapon rule seam in the sim")?)?;
    // Objects, flags and wormholes.
    table.set("SetSpecialForcesFlag", gap_write(lua, env, "SetSpecialForcesFlag", &["hb"], "no special-forces marker in the sim seam")?)?;
    table.set("TransferFlag", gap_write(lua, env, "TransferFlag", &["ii"], "flag transfer between ships has no sim seam")?)?;
    table.set("LinkWormHoles", gap_write(lua, env, "LinkWormHoles", &["hh"], "no wormhole link edit in the sim seam")?)?;
    table.set("EnableWormholeTravel", gap_write(lua, env, "EnableWormholeTravel", &["hb"], "no wormhole travel switch in the sim seam")?)?;
    table.set("ObstacleAvoidence", gap_write(lua, env, "ObstacleAvoidence", &["b"], "no obstacle-avoidance switch in the sim seam")?)?;
    table.set("PauseSimulation", gap_write(lua, env, "PauseSimulation", &[""], "no pause request in the sim seam")?)?;
    table.set("UnpauseSimulation", gap_write(lua, env, "UnpauseSimulation", &[""], "no pause request in the sim seam")?)?;
    table.set("EnableLight", gap_write(lua, env, "EnableLight", &["hb"], "no light switch in the sim seam")?)?;

    // `ReplaceObject(object, odf)` and `RecruitSpecialForces(name, team, out)`
    // return a handle / success: with no seam they return 0 / false.
    table.set("ReplaceObject", def(lua, env, "ReplaceObject", &["hs"], |_, e, _, a| {
        e.unit("ReplaceObject", a[0].id())?;
        e.gap("ReplaceObject", "no object replacement in the sim seam", args_of(a))?;
        Ok(Value::Integer(0))
    })?)?;
    table.set("RecruitSpecialForces", def(lua, env, "RecruitSpecialForces", &["sii"], |_, e, _, a| {
        e.team("RecruitSpecialForces", a[1].int())?;
        e.gap("RecruitSpecialForces", "no special-forces recruitment in the sim seam", args_of(a))?;
        Ok(Value::Boolean(false))
    })?)?;

    // Queries with no data behind them.
    table.set("IsCompromised", no_data(lua, env, "IsCompromised", &["hi"], "the meaning of compromised (and its data) is not known")?)?;
    table.set("IsDerelict", no_data(lua, env, "IsDerelict", &["h"], "the world view carries no derelict state")?)?;
    table.set("IsThereARomulanSpyOnShip", no_data(lua, env, "IsThereARomulanSpyOnShip", &["h"], "the world view carries no boarding or spy state")?)?;
    table.set("GetTranswarpExitWormhole", no_data(lua, env, "GetTranswarpExitWormhole", &["h"], "the world view carries no transwarp links")?)?;
    table.set("GetNearestTranswarpGate", no_data(lua, env, "GetNearestTranswarpGate", &["h"], "no transwarp gate classification in the world view")?)?;

    // Mission flow. `FailMission(seconds, text)` and `SucceedMission(seconds,
    // text)` retain the delay and text for the host's outcome screen and mark
    // the mission finished at once; the delay is not applied here.
    table.set("FailMission", def(lua, env, "FailMission", &["fs"], |_, e, _, a| {
        e.call("FailMission", args_of(a))?;
        e.ctx.borrow_mut().finished = true;
        Ok(Value::Nil)
    })?)?;
    table.set("SucceedMission", def(lua, env, "SucceedMission", &["fs"], |_, e, _, a| {
        e.call("SucceedMission", args_of(a))?;
        e.ctx.borrow_mut().finished = true;
        Ok(Value::Nil)
    })?)?;
    table.set("GetTime", def(lua, env, "GetTime", &[""], |_, e, _, _| {
        Ok(Value::Number(e.ctx.borrow().view.tick as f64 / TICK_HZ as f64))
    })?)?;
    Ok(())
}
