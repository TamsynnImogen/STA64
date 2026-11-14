//! Armada I teams, relations, AI switches and team resources.
//!
//! Team numbers follow the configured player numbering (converted missions
//! use `ArmadaTeams`: 1..8 are slots 0..7, 0 is neutral).
//!
//! `SetRelation(a, b, relation)` takes `Team::TeamRelation` as the DLLs pass
//! it, an int: 0 enemy, 1 neutral, 2 ally (read from the executable, static).
//! It sends `Edit::SetRelation` (SIM-63), one direction, as the original does
//! (see `api::set_relation`).

use mlua::{Lua, Table, Value};
use sta_sim::command::{Edit, TeamOp};
use sta_sim::team_pool::{crew_pool_after, officer_count_after};

use super::{Env, args_of, def};
use crate::api::ai_switch;

/// Resource ids in the world view and in `Edit::Cargo`: 0 dilithium.
const DILITHIUM: u8 = 0;

fn dilithium(e: &Env, name: &str, team: i64, n: i64, add: bool) -> mlua::Result<Value> {
    let player = e.team(name, team)?;
    if !player.is_player() {
        return Err(mlua::Error::runtime(format!("A1.{name} requires a player team")));
    }
    let amount = i32::try_from(n).map_err(|_| mlua::Error::runtime(format!("A1.{name} amount out of range")))?;
    let tick = e.ctx.borrow().view.tick;
    // Writes are absolute and the view is a snapshot: within one tick, build on
    // the value this call last wrote instead of the stale view.
    let base = {
        let c = e.ctx.borrow();
        match e.st.borrow().dilithium.get(&player) {
            Some((t, v)) if *t == tick => *v,
            _ => c
                .view
                .player(player)
                .and_then(|p| p.resources.get(usize::from(DILITHIUM)))
                .copied()
                .unwrap_or(sta_core::Fixed::ZERO),
        }
    };
    let new = if add { base + sta_core::Fixed::from_int(amount) } else { sta_core::Fixed::from_int(amount) };
    e.edit(name, Edit::Cargo(player, DILITHIUM, new))?;
    e.st.borrow_mut().dilithium.insert(player, (tick, new));
    Ok(Value::Integer((new.to_f32() as f64).round() as i64))
}

pub(super) fn install(lua: &Lua, env: &Env, a2: &Table, table: &Table) -> mlua::Result<()> {
    table.set("GetRealTeam", super::alias(lua, env, a2, "GetRealTeam")?)?;
    table.set("SetRealTeam", super::alias(lua, env, a2, "SetRealTeam")?)?;
    table.set("SetPerceivedTeam", super::alias(lua, env, a2, "SetPerceivedTeam")?)?;

    table.set("SetRelation", def(lua, env, "SetRelation", &["iii"], |_, e, _, a| {
        let (from, to) = (e.team("SetRelation", a[0].int())?, e.team("SetRelation", a[1].int())?);
        let rel = crate::api::set_relation::from_code(a[2].int())
            .ok_or_else(|| mlua::Error::runtime("A1.SetRelation requires a relation value 0, 1 or 2"))?;
        crate::api::set_relation::queue(&mut e.ctx.borrow_mut(), "A1.SetRelation", from, to, rel)?;
        Ok(Value::Nil)
    })?)?;

    table.set("AI_Team_Load_AIP", def(lua, env, "AI_Team_Load_AIP", &["is"], |_, e, _, a| {
        let player = e.team("AI_Team_Load_AIP", a[0].int())?;
        let mut c = e.ctx.borrow_mut();
        c.ai_requests.push(sta_ai::AiRequest::Load(player, a[1].text().to_owned()));
        Ok(Value::Nil)
    })?)?;

    table.set("SetDilithium", def(lua, env, "SetDilithium", &["ii"], |_, e, _, a| {
        dilithium(e, "SetDilithium", a[0].int(), a[1].int(), false)
    })?)?;
    table.set("AddDilithium", def(lua, env, "AddDilithium", &["ii"], |_, e, _, a| {
        dilithium(e, "AddDilithium", a[0].int(), a[1].int(), true)
    })?)?;

    // The AI switches (LUA-20): requests for the host's AI hub, in call order (`api::ai_switch`).
    // Each also retains `Presentation::Call("A1.<name>")` as before (and as A2 does), so a recorded
    // command stream does not change; none is counted as unsupported any more.
    // `ResetStrategyCycle` is not exported by Armada I.
    table.set("DisableAllAITeams", def(lua, env, "DisableAllAITeams", &[""], |_, e, _, a| {
        e.call("DisableAllAITeams", args_of(a))?;
        ai_switch::enable_all(&mut e.ctx.borrow_mut(), false);
        Ok(Value::Nil)
    })?)?;
    table.set("EnableAllAITeams", def(lua, env, "EnableAllAITeams", &[""], |_, e, _, a| {
        e.call("EnableAllAITeams", args_of(a))?;
        ai_switch::enable_all(&mut e.ctx.borrow_mut(), true);
        Ok(Value::Nil)
    })?)?;
    for (name, on) in [("DisableTeam", false), ("EnableTeam", true)] {
        table.set(name, def(lua, env, name, &["i"], move |_, e, _, a| {
            let team = e.team(name, a[0].int())?;
            e.call(name, args_of(a))?;
            ai_switch::enable(&mut e.ctx.borrow_mut(), Some(team), on);
            Ok(Value::Nil)
        })?)?;
    }
    table.set("SetIsAITeam", def(lua, env, "SetIsAITeam", &["ib"], |_, e, _, a| {
        let team = e.team("SetIsAITeam", a[0].int())?;
        e.call("SetIsAITeam", args_of(a))?;
        ai_switch::set_is_ai(&mut e.ctx.borrow_mut(), "A1.SetIsAITeam", Some(team), a[1].boolean());
        Ok(Value::Nil)
    })?)?;
    table.set("SetRace", def(lua, env, "SetRace", &["hs"], |_, e, _, a| {
        let id = a[0].id();
        e.unit("SetRace", id)?;
        e.edit("SetRace", sta_sim::command::Edit::UnitRace(id, a[1].text().to_owned()))?;
        Ok(Value::Nil)
    })?)?;
    table.set("SetWormholeTeam", def(lua, env, "SetWormholeTeam", &["hi"], |_, e, _, a| {
        e.unit("SetWormholeTeam", a[0].id())?;
        e.team("SetWormholeTeam", a[1].int())?;
        e.gap("SetWormholeTeam", "no wormhole team edit in the sim seam", args_of(a))?;
        Ok(Value::Nil)
    })?)?;
    // SIM-54: the team calls clamp to the team's maximum (Armada I `Team` calls, read from the
    // executable, static; nothing measured, A1 has no bench). Crew is the stock, clamped to the
    // team's crew capacity; officers are the count in use, clamped to the cap. No floor at 0.
    // The edit lands on the next tick, so the returned number is the rule applied to the view.
    for (name, op) in [
        ("SetOfficers", TeamOp::OfficersSetA1),
        ("AddOfficers", TeamOp::OfficersAddA1),
        ("SetTeamCrew", TeamOp::CrewSetA1),
        ("AddCrew", TeamOp::CrewAddA1),
    ] {
        table.set(name, def(lua, env, name, &["if"], move |_, e, _, a| {
            let team = e.team(name, a[0].int())?;
            if !a[1].num().is_finite() {
                return Err(mlua::Error::runtime(format!("A1.{name} requires a finite number")));
            }
            let x = sta_core::Fixed::from_f64(a[1].num().clamp(-1.0e9, 1.0e9));
            let (pool, cap, used, officer_cap, has_limit) = {
                let c = e.ctx.borrow();
                let view = c.view.player(team);
                (
                    view.and_then(|p| p.crew).unwrap_or(sta_core::Fixed::ZERO),
                    c.view.attrs.seats.iter().find(|s| s.player == team).and_then(|s| s.crew_capacity),
                    view.map_or(sta_core::Fixed::ZERO, |p| p.officers[0]),
                    view.map_or(sta_core::Fixed::ZERO, |p| p.officers[1]),
                    c.view.attrs.seats.iter().any(|s| s.player == team && s.officer_limit.is_some()),
                )
            };
            let after = match op {
                // The sim refuses an officer edit for a seat with no limit: the count is unchanged.
                TeamOp::OfficersSetA1 | TeamOp::OfficersAddA1 => {
                    has_limit.then(|| officer_count_after(op, used, x, officer_cap)).flatten()
                }
                _ => cap.and_then(|cap| crew_pool_after(op, pool, x, cap)),
            };
            e.edit(name, Edit::TeamPool(team, op, x))?;
            Ok(Value::Number(f64::from(after.unwrap_or(match op {
                TeamOp::OfficersSetA1 | TeamOp::OfficersAddA1 => used,
                _ => pool,
            }).to_f32())))
        })?)?;
    }
    table.set("SetCrew", def(lua, env, "SetCrew", &["hf"], |_, e, _, a| {
        let id = a[0].id();
        e.unit("SetCrew", id)?;
        if !a[1].num().is_finite() { return Err(mlua::Error::runtime("A1.SetCrew requires a finite number")); }
        e.edit("SetCrew", sta_sim::command::Edit::UnitCrew(id, sta_core::Fixed::from_f64(a[1].num())))?;
        Ok(Value::Nil)
    })?)?;
    table.set("GetDilithiumOnFreighter", def(lua, env, "GetDilithiumOnFreighter", &["h"], |_, e, _, a| {
        e.unit("GetDilithiumOnFreighter", a[0].id())?;
        Err(e.no_data("GetDilithiumOnFreighter", "the world view carries no cargo contents"))
    })?)?;
    Ok(())
}
