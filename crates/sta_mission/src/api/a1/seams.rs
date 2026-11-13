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
    // SetPosition and PointAt (SIM-57): the Armada II rules (instant, speed
    // zeroed, orders kept; yaw only), read from the Armada II executable and
    // measured there; the Armada I executable is not measured.
    table.set("SetPosition", def(lua, env, "SetPosition", &["hs"], |lua, e, _, a| {
        let unit = e.unit("SetPosition", a[0].id())?;
        let to = e.path_point(lua, "SetPosition", a[1].text(), 0)?;
        e.edit("SetPosition", sta_sim::command::Edit::Position(unit.id, to))?;
        Ok(Value::Nil)
    })?)?;
    table.set("SetWarpSpeed", gap_write(lua, env, "SetWarpSpeed", &["hf"], "no warp-speed seam in the sim")?)?;
    table.set("ResetWarpSpeed", gap_write(lua, env, "ResetWarpSpeed", &["h"], "no warp-speed seam in the sim")?)?;
    // Patrol and SearchAndDestroy (SIM-56): the Armada II rules; the Armada I
    // executable's script `Patrol` also sets the path's loop type and issues
    // GO (read, static); its command handlers are not read.
    table.set("Patrol", def(lua, env, "Patrol", &["hs"], |lua, e, _, a| {
        let id = a[0].id();
        if !e.unit("Patrol", id)?.owner.is_player() {
            return Err(mlua::Error::runtime("A1.Patrol: object has no player owner"));
        }
        let points = e.path(lua, "Patrol", a[1].text())?;
        if !e.ctx.borrow_mut().order(id, |units| sta_sim::command::Order::Patrol { units, points }) {
            return Err(mlua::Error::runtime("A1.Patrol: the order could not be queued"));
        }
        Ok(Value::Nil)
    })?)?;
    table.set("SearchAndDestroy", def(lua, env, "SearchAndDestroy", &["h"], |_, e, _, a| {
        let id = a[0].id();
        if !e.unit("SearchAndDestroy", id)?.owner.is_player() {
            return Err(mlua::Error::runtime("A1.SearchAndDestroy: object has no player owner"));
        }
        if !e.ctx.borrow_mut().order(id, |units| sta_sim::command::Order::SearchAndDestroy { units }) {
            return Err(mlua::Error::runtime("A1.SearchAndDestroy: the order could not be queued"));
        }
        Ok(Value::Nil)
    })?)?;
    table.set("PointAt", def(lua, env, "PointAt", &["hh", "hs"], |lua, e, ov, a| {
        let unit = e.unit("PointAt", a[0].id())?;
        let to = if ov == 0 { let t = e.unit("PointAt", a[1].id())?; e.ctx.borrow().placed(t.id, t.pos) } else { e.path_point(lua, "PointAt", a[1].text(), 0)? };
        let from = e.ctx.borrow().placed(unit.id, unit.pos);
        let dir = (to.x - from.x, to.z - from.z);
        if dir == (sta_core::Fixed::ZERO, sta_core::Fixed::ZERO) {
            e.ctx.borrow_mut().unsupported("A1.PointAt: the unit is at the target (no direction to face)");
            return Ok(Value::Nil);
        }
        let heading = dir;
        e.edit("PointAt", sta_sim::command::Edit::Heading(unit.id, heading))?;
        Ok(Value::Nil)
    })?)?;
    table.set("Special_Attack", gap_write(lua, env, "Special_Attack", &["hvs", "hhs", "hs"], "special weapons are named; the order takes a slot index")?)?;
    table.set("SetSpecialWeaponAttackInterval", gap_write(lua, env, "SetSpecialWeaponAttackInterval", &["if"], "no special-weapon interval seam in the sim")?)?;
    // AI team orders.
    table.set("AI_Attack", gap_write(lua, env, "AI_Attack", &["ii", "iv", "i"], "no AI team attack seam in the sim")?)?;
    table.set("AttackMainEnemyBase", gap_write(lua, env, "AttackMainEnemyBase", &["ib"], "no AI team attack seam in the sim")?)?;
    // Damage and shields.
    // SIM-55: a fraction of the maximum, and the new maximum.
    table.set("SetShieldPercent", def(lua, env, "SetShieldPercent", &["hf"], |_, e, _, a| {
        let id = a[0].id();
        e.unit("SetShieldPercent", id)?;
        if !a[1].num().is_finite() { return Err(mlua::Error::runtime("A1.SetShieldPercent requires a finite number")); }
        e.edit("SetShieldPercent", sta_sim::command::Edit::ShieldPercent(id, sta_core::Fixed::from_f64(a[1].num())))?;
        Ok(Value::Nil)
    })?)?;
    table.set("SetMaxShields", def(lua, env, "SetMaxShields", &["hf"], |_, e, _, a| {
        let id = a[0].id();
        e.unit("SetMaxShields", id)?;
        if !a[1].num().is_finite() { return Err(mlua::Error::runtime("A1.SetMaxShields requires a finite number")); }
        e.edit("SetMaxShields", sta_sim::command::Edit::MaxShields(id, sta_core::Fixed::from_f64(a[1].num())))?;
        Ok(Value::Nil)
    })?)?;
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
    // `RecruitSpecialForces(name, team, out)` returns success and leaves the
    // out-reference unchanged: no seam, so (false, out).
    let recruit_env = env.clone();
    table.set("RecruitSpecialForces", lua.create_function(move |_, args: mlua::MultiValue| {
        let (_, a) = recruit_env.parse("RecruitSpecialForces", &["sii"], &args)?;
        recruit_env.team("RecruitSpecialForces", a[1].int())?;
        recruit_env.gap("RecruitSpecialForces", "no special-forces recruitment in the sim seam", args_of(&a))?;
        Ok((false, a[2].int()))
    })?)?;
    // Loading rule and action-script DLLs by name: a converted mission is one
    // Lua file, so nothing is loaded; 0 is the failure status.
    table.set("Load_Rules", def(lua, env, "Load_Rules", &["s"], |_, e, _, a| {
        e.gap("Load_Rules", "converted missions are one Lua file; no rule DLL is loaded", args_of(a))?;
        Ok(Value::Integer(0))
    })?)?;
    table.set("Load_Action_Scripts", def(lua, env, "Load_Action_Scripts", &["s"], |_, e, _, a| {
        e.gap("Load_Action_Scripts", "converted missions are one Lua file; no action script is loaded", args_of(a))?;
        Ok(Value::Integer(0))
    })?)?;
    // Script groups of teams: plain bookkeeping.
    table.set("GroupNew", def(lua, env, "GroupNew", &["s"], |_, e, _, a| {
        let mut st = e.st.borrow_mut();
        st.next_group += 1;
        let id = st.next_group;
        st.groups.insert(id, (a[0].text().to_owned(), Default::default()));
        Ok(Value::Integer(i64::from(id)))
    })?)?;
    table.set("GroupDelete", def(lua, env, "GroupDelete", &["i"], |_, e, _, a| {
        let id = u32::try_from(a[0].int()).unwrap_or(0);
        e.st.borrow_mut().groups.remove(&id).map(|_| Value::Nil).ok_or_else(|| mlua::Error::runtime("A1.GroupDelete: unknown group"))
    })?)?;
    table.set("Add_Team_To_Group", def(lua, env, "Add_Team_To_Group", &["ii"], |_, e, _, a| {
        e.team("Add_Team_To_Group", a[0].int())?;
        let id = u32::try_from(a[1].int()).unwrap_or(0);
        let mut st = e.st.borrow_mut();
        let (_, members) = st.groups.get_mut(&id).ok_or_else(|| mlua::Error::runtime("A1.Add_Team_To_Group: unknown group"))?;
        members.insert(a[0].int());
        Ok(Value::Nil)
    })?)?;
    // Team strength variables: no strength metric in the world view.
    table.set("strength_team_1", no_data(lua, env, "strength_team_1", &[""], "no team strength metric in the world view")?)?;
    table.set("strength_team_2", no_data(lua, env, "strength_team_2", &[""], "no team strength metric in the world view")?)?;
    table.set("hot_float_value", no_data(lua, env, "hot_float_value", &[""], "no team strength metric in the world view")?)?;

    // Queries with no data behind them.
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
