//! Armada I teams, relations, AI switches and team resources.
//!
//! Team numbers follow the configured player numbering (converted missions
//! use `ArmadaTeams`: 1..8 are slots 0..7, 0 is neutral).
//!
//! `SetRelation(a, b, relation)` takes `Team::TeamRelation` as the DLLs pass
//! it, an int. The meaning of the values is NOT measured; `relation_of` is the
//! one provisional mapping (0 enemy, 1 ally, 2 neutral) and is easy to change.
//! It only sets the mission-local override (`Player:setRelation` does the
//! same); the sim has no relation edit, which `unsupported()` records.

use mlua::{Lua, Table, Value};
use sta_sim::command::Edit;
use sta_sim::view::Relation;

use super::{Env, args_of, def};

fn relation_of(n: i64) -> Option<Relation> {
    match n {
        0 => Some(Relation::Enemy),
        1 => Some(Relation::Ally),
        2 => Some(Relation::Neutral),
        _ => None,
    }
}

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
        let rel = relation_of(a[2].int())
            .ok_or_else(|| mlua::Error::runtime("A1.SetRelation requires a relation value 0, 1 or 2"))?;
        let mut c = e.ctx.borrow_mut();
        c.relations.insert((from, to), rel);
        c.unsupported("A1.SetRelation: mission-local override only; no relation edit in the sim seam");
        Ok(Value::Nil)
    })?)?;

    table.set("AI_Team_Load_AIP", def(lua, env, "AI_Team_Load_AIP", &["is"], |_, e, _, a| {
        let player = e.team("AI_Team_Load_AIP", a[0].int())?;
        let mut c = e.ctx.borrow_mut();
        c.loaded_aips.push((player, a[1].text().to_owned()));
        Ok(Value::Nil)
    })?)?;

    table.set("SetDilithium", def(lua, env, "SetDilithium", &["ii"], |_, e, _, a| {
        dilithium(e, "SetDilithium", a[0].int(), a[1].int(), false)
    })?)?;
    table.set("AddDilithium", def(lua, env, "AddDilithium", &["ii"], |_, e, _, a| {
        dilithium(e, "AddDilithium", a[0].int(), a[1].int(), true)
    })?)?;

    // Writes with no seam: AI team switches, race change, officers, crew.
    table.set("DisableAllAITeams", def(lua, env, "DisableAllAITeams", &[""], |_, e, _, a| {
        e.gap("DisableAllAITeams", "no AI enable seam in the sim", args_of(a))?;
        Ok(Value::Nil)
    })?)?;
    table.set("EnableAllAITeams", def(lua, env, "EnableAllAITeams", &[""], |_, e, _, a| {
        e.gap("EnableAllAITeams", "no AI enable seam in the sim", args_of(a))?;
        Ok(Value::Nil)
    })?)?;
    table.set("DisableTeam", def(lua, env, "DisableTeam", &["i"], |_, e, _, a| {
        e.team("DisableTeam", a[0].int())?;
        e.gap("DisableTeam", "no AI enable seam in the sim", args_of(a))?;
        Ok(Value::Nil)
    })?)?;
    table.set("EnableTeam", def(lua, env, "EnableTeam", &["i"], |_, e, _, a| {
        e.team("EnableTeam", a[0].int())?;
        e.gap("EnableTeam", "no AI enable seam in the sim", args_of(a))?;
        Ok(Value::Nil)
    })?)?;
    table.set("SetIsAITeam", def(lua, env, "SetIsAITeam", &["ib"], |_, e, _, a| {
        e.team("SetIsAITeam", a[0].int())?;
        e.gap("SetIsAITeam", "the AI flag of a seat is fixed by the game setup", args_of(a))?;
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
    table.set("SetOfficers", def(lua, env, "SetOfficers", &["if"], |_, e, _, a| {
        e.team("SetOfficers", a[0].int())?;
        e.gap("SetOfficers", "no officer-stock edit in the sim seam", args_of(a))?;
        Ok(Value::Number(a[1].num()))
    })?)?;
    table.set("AddOfficers", def(lua, env, "AddOfficers", &["if"], |_, e, _, a| {
        e.team("AddOfficers", a[0].int())?;
        e.gap("AddOfficers", "no officer-stock edit in the sim seam", args_of(a))?;
        Ok(Value::Number(a[1].num()))
    })?)?;
    table.set("SetTeamCrew", def(lua, env, "SetTeamCrew", &["if"], |_, e, _, a| {
        let team = e.team("SetTeamCrew", a[0].int())?;
        if !a[1].num().is_finite() { return Err(mlua::Error::runtime("A1.SetTeamCrew requires a finite number")); }
        e.edit("SetTeamCrew", sta_sim::command::Edit::TeamCrew(team, sta_core::Fixed::from_f64(a[1].num())))?;
        Ok(Value::Number(a[1].num()))
    })?)?;
    table.set("AddCrew", def(lua, env, "AddCrew", &["if"], |_, e, _, a| {
        e.team("AddCrew", a[0].int())?;
        e.gap("AddCrew", "no crew-stock edit in the sim seam", args_of(a))?;
        Ok(Value::Number(a[1].num()))
    })?)?;
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
