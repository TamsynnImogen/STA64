//! Armada I object calls: handles, queries over the mission view, counts and
//! nearest searches, shields, switches, flags and creation.
//!
//! Argument meanings that are not evidence-backed (the extra `int` of the
//! nearest searches, the `bool` of `RemoveAllCraft`, the team-or-relation
//! reading of the third argument of the `...Near...` searches) are taken as
//! documented in each function's comment and listed in README.md as not
//! measured.

use mlua::{Lua, Table, Value};
use sta_core::{EntityId, Fixed};
use sta_sim::command::{AlertState, Edit, Subsystem};
use sta_sim::present::Presentation;
use sta_sim::view::{Relation, UnitView};

use super::{A, Env, Flag, TOKEN_BASE, alias, def};
use crate::api::{ABSENT_AMOUNT, ABSENT_DISTANCE, ABSENT_PATH_LOCATION, ABSENT_SHIELD_PERCENT, dist, pos_f64, vector};

pub(super) use crate::api::search::stem;
use crate::api::search::{Query, near, odf_ok};

/// The ODF filter of an argument: nil or empty matches every unit.
fn filter(a: &A) -> Option<&str> {
    match a {
        A::Str(s) => Some(s),
        _ => None,
    }
}

/// The A2 station classification; its errors (unclassified class) propagate.
fn station(is_station: &mlua::Function, id: EntityId) -> mlua::Result<bool> {
    is_station.call::<bool>(i64::from(id.0)).map_err(super::rename_error)
}

fn handle_value(id: Option<EntityId>) -> Value {
    Value::Integer(id.map_or(0, |id| i64::from(id.0)))
}

fn shield_fraction(unit: &UnitView) -> f64 {
    let (now, max) = unit
        .shields
        .iter()
        .fold((0.0, 0.0), |(n, m), s| (n + f64::from(s[0].to_f32()), m + f64::from(s[1].to_f32())));
    if max > 0.0 { now / max } else { 0.0 }
}

pub(super) fn install(lua: &Lua, env: &Env, a2: &Table, table: &Table) -> mlua::Result<()> {
    // The same operation as the A2 binding.
    table.set("RemoveObject", alias(lua, env, a2, "RemoveObject")?)?;
    table.set("CraftCannotDie", alias(lua, env, a2, "CraftCannotDie")?)?;
    table.set("IsAlive", alias(lua, env, a2, "IsAlive")?)?;
    table.set("IsCompromised", alias(lua, env, a2, "IsCompromised")?)?;
    table.set("Exists", alias(lua, env, a2, "Exists")?)?;
    table.set("DoesNotExist", alias(lua, env, a2, "DoesNotExist")?)?;
    table.set("Cloak", alias(lua, env, a2, "Cloak")?)?;
    table.set("Decloak", alias(lua, env, a2, "Decloak")?)?;
    // No separate immediate-cloak seam: the same cloak order.
    table.set("ImmediateCloak", super::alias_as(lua, env, a2, "ImmediateCloak", "Cloak")?)?;
    table.set("DisableWeapons", alias(lua, env, a2, "DisableWeapons")?)?;
    table.set("DisableEngines", alias(lua, env, a2, "DisableEngines")?)?;
    table.set("SetSpecialEnergy", alias(lua, env, a2, "SetSpecialEnergy")?)?;
    table.set("IsVisibleToTeam", alias(lua, env, a2, "IsVisibleToTeam")?)?;
    table.set("UnderAttack", alias(lua, env, a2, "UnderAttack")?)?;

    table.set("GetHandle", def(lua, env, "GetHandle", &["s"], |_, e, _, a| {
        let c = e.ctx.borrow();
        let wanted = a[0].text();
        let id = c
            .labels
            .iter()
            .find(|(_, l)| l.as_str() == wanted)
            .or_else(|| c.labels.iter().find(|(_, l)| l.eq_ignore_ascii_case(wanted)))
            .map(|(id, _)| *id);
        Ok(handle_value(id))
    })?)?;

    table.set("IsValid", def(lua, env, "IsValid", &["H"], |_, e, _, a| {
        Ok(Value::Boolean(e.ctx.borrow().unit(a[0].id()).is_some()))
    })?)?;

    table.set("IsExploding", def(lua, env, "IsExploding", &["h"], |_, e, _, a| {
        // An object that is gone from the view is not exploding.
        let c = e.ctx.borrow();
        Ok(Value::Boolean(c.unit(a[0].id()).is_some_and(|u| u.flags.contains(sta_sim::view::UnitFlags::DYING))))
    })?)?;

    // `GetLocation(handle)` and `GetLocation(path, point)`.
    table.set("GetLocation", def(lua, env, "GetLocation", &["h", "si"], |lua, e, ov, a| {
        let p = if ov == 0 {
            // A handle naming nothing live: the engine's constant zero vector (LUA-21).
            let c = e.ctx.borrow();
            match c.alive(a[0].id()) {
                Some(u) => u.pos,
                None => {
                    c.dead_handle("A1", "GetLocation", a[0].id())?;
                    return Ok(Value::Table(vector(lua, ABSENT_PATH_LOCATION)?));
                }
            }
        } else {
            // An absent path or point: the engine returns a fixed default vector.
            match e.find_path_point(lua, "GetLocation", a[0].text(), a[1].int())? {
                Some(p) => p,
                None => return Ok(Value::Table(vector(lua, crate::api::ABSENT_PATH_LOCATION)?)),
            }
        };
        Ok(Value::Table(vector(lua, pos_f64(p))?))
    })?)?;

    // `GetDistance(a, b)` and `GetDistance(object, path, point)`.
    table.set("GetDistance", def(lua, env, "GetDistance", &["hh", "hsi"], |lua, e, ov, a| {
        // An object naming nothing live, on either side: the engine answers 1e30 (LUA-21).
        let from = e.ctx.borrow().alive(a[0].id()).map(|u| pos_f64(u.pos));
        let Some(from) = from else {
            e.ctx.borrow().dead_handle("A1", "GetDistance", a[0].id())?;
            return Ok(Value::Number(ABSENT_DISTANCE));
        };
        let to = if ov == 0 {
            let c = e.ctx.borrow();
            match c.alive(a[1].id()) {
                Some(u) => pos_f64(u.pos),
                None => {
                    c.dead_handle("A1", "GetDistance", a[1].id())?;
                    return Ok(Value::Number(ABSENT_DISTANCE));
                }
            }
        } else {
            pos_f64(e.path_point(lua, "GetDistance", a[1].text(), a[2].int())?)
        };
        Ok(Value::Number(dist(from, to)))
    })?)?;

    // Shield strength now over maximum across the facings, 0..1 (the
    // SetShieldPercent takes the same fraction).
    // The "real" variant has no separate source: perception does not alter shields.
    table.set("GetShieldPercent", def(lua, env, "GetShieldPercent", &["h"], |_, e, _, a| {
        Ok(Value::Number(shield_fraction(&e.unit("GetShieldPercent", a[0].id())?)))
    })?)?;
    table.set("GetRealShieldPercent", def(lua, env, "GetRealShieldPercent", &["h"], |_, e, _, a| {
        // A handle naming nothing live answers -33 (LUA-21; read from the executable, static).
        let unit = e.ctx.borrow().alive(a[0].id()).cloned();
        let Some(unit) = unit else {
            e.ctx.borrow().dead_handle("A1", "GetRealShieldPercent", a[0].id())?;
            return Ok(Value::Number(ABSENT_SHIELD_PERCENT));
        };
        Ok(Value::Number(shield_fraction(&unit)))
    })?)?;

    table.set("GetCrew", def(lua, env, "GetCrew", &["h"], |_, e, _, a| {
        // A handle naming nothing live answers -1 (LUA-21; read from the executable, static).
        let unit = e.ctx.borrow().alive(a[0].id()).cloned();
        let Some(unit) = unit else {
            e.ctx.borrow().dead_handle("A1", "GetCrew", a[0].id())?;
            return Ok(Value::Number(ABSENT_AMOUNT));
        };
        let crew = unit.crew.ok_or_else(|| e.no_data("GetCrew", "the unit has no initialised crew"))?;
        Ok(Value::Number(f64::from(crew[0].to_f32())))
    })?)?;

    // `CountUnits(team, odf)`: live units of `team` (-1: any) of that ODF (nil: any).
    table.set("CountUnits", def(lua, env, "CountUnits", &["iS"], |_, e, _, a| {
        let owner = e.team_filter("CountUnits", a[0].int())?;
        let c = e.ctx.borrow();
        let n = c
            .view
            .units
            .iter()
            .filter(|u| c.alive(u.id).is_some() && owner.is_none_or(|o| u.owner == o) && odf_ok(&c, u, filter(&a[1])))
            .count();
        Ok(Value::Integer(n as i64))
    })?)?;

    // `CountUnitsNearObject(object, radius, team, odf)`; the third argument is
    // read as a team (-1: any), as in CountUnits (not measured). The centre
    // object itself is not counted.
    table.set("CountUnitsNearObject", def(lua, env, "CountUnitsNearObject", &["hfiS"], |_, e, _, a| {
        let owner = e.team_filter("CountUnitsNearObject", a[2].int())?;
        let centre = e.unit("CountUnitsNearObject", a[0].id())?;
        let c = e.ctx.borrow();
        let q = Query { center: pos_f64(centre.pos), radius: a[1].num(), owner, odf: filter(&a[3]), exclude: centre.id, seen_by: None };
        Ok(Value::Integer(near(&c, &q).len() as i64))
    })?)?;

    // `CountUnitsNearPoint(path, radius, team, odf, int)`: around the path's
    // first point; the trailing int is not measured and ignored.
    table.set("CountUnitsNearPoint", def(lua, env, "CountUnitsNearPoint", &["sfiSi"], |lua, e, _, a| {
        let owner = e.team_filter("CountUnitsNearPoint", a[2].int())?;
        let point = e.path_point(lua, "CountUnitsNearPoint", a[0].text(), 0)?;
        let c = e.ctx.borrow();
        let q = Query { center: pos_f64(point), radius: a[1].num(), owner, odf: filter(&a[3]), exclude: EntityId::NONE, seen_by: None };
        Ok(Value::Integer(near(&c, &q).len() as i64))
    })?)?;

    // `CountVisibleUnits(observer, team, bool, odf)`: units of `team` that
    // `observer` can see; the bool is not measured and ignored.
    table.set("CountVisibleUnits", def(lua, env, "CountVisibleUnits", &["iibS"], |_, e, _, a| {
        let observer = e.team("CountVisibleUnits", a[0].int())?;
        let owner = e.team_filter("CountVisibleUnits", a[1].int())?;
        let c = e.ctx.borrow();
        let q = Query { center: (0.0, 0.0, 0.0), radius: f64::INFINITY, owner, odf: filter(&a[3]), exclude: EntityId::NONE, seen_by: Some(observer) };
        Ok(Value::Integer(near(&c, &q).len() as i64))
    })?)?;

    // Units near the object that its owner can see (reading not measured).
    table.set("CountVisibleUnitsNearObject", def(lua, env, "CountVisibleUnitsNearObject", &["hfibS"], |_, e, _, a| {
        let owner = e.team_filter("CountVisibleUnitsNearObject", a[2].int())?;
        let centre = e.unit("CountVisibleUnitsNearObject", a[0].id())?;
        let c = e.ctx.borrow();
        let q = Query { center: pos_f64(centre.pos), radius: a[1].num(), owner, odf: filter(&a[4]), exclude: centre.id, seen_by: Some(centre.owner) };
        Ok(Value::Integer(near(&c, &q).len() as i64))
    })?)?;

    // `GetNearestEnemy(object, radius)`: nearest live unit whose owner is an
    // enemy of the object's owner; 0 when none.
    table.set("GetNearestEnemy", def(lua, env, "GetNearestEnemy", &["hf"], |_, e, _, a| {
        let me = e.unit("GetNearestEnemy", a[0].id())?;
        let c = e.ctx.borrow();
        let q = Query { center: pos_f64(me.pos), radius: a[1].num(), owner: None, odf: None, exclude: me.id, seen_by: None };
        let found = near(&c, &q).into_iter().find(|u| c.relation(me.owner, u.owner) == Relation::Enemy);
        Ok(handle_value(found.map(|u| u.id)))
    })?)?;

    // `GetNearestStarbase(object, team)`: nearest live station of `team`
    // (-1: any), by the A2 station classification; the second argument's
    // reading as a team is not measured.
    let is_station: mlua::Function = a2.get("IsStation")?;
    let is_station_all = is_station.clone();
    table.set("GetNearestStarbase", def(lua, env, "GetNearestStarbase", &["hi"], move |_, e, _, a| {
        let me = e.unit("GetNearestStarbase", a[0].id())?;
        let owner = e.team_filter("GetNearestStarbase", a[1].int())?;
        let candidates: Vec<EntityId> = {
            let c = e.ctx.borrow();
                let q = Query { center: pos_f64(me.pos), radius: f64::INFINITY, owner, odf: None, exclude: me.id, seen_by: None };
            near(&c, &q).into_iter().map(|u| u.id).collect()
        };
        for id in candidates {
            if station(&is_station, id)? {
                return Ok(handle_value(Some(id)));
            }
        }
        Ok(handle_value(None))
    })?)?;

    // `GetNearestObjectNearMe(object, radius, team, odf, int, bool)`. The int
    // (always -1 in the stock DLLs) and the bool (always false) are not measured.
    table.set("GetNearestObjectNearMe", def(lua, env, "GetNearestObjectNearMe", &["hfiSib"], |_, e, _, a| {
        let owner = e.team_filter("GetNearestObjectNearMe", a[2].int())?;
        let me = e.unit("GetNearestObjectNearMe", a[0].id())?;
        let c = e.ctx.borrow();
        let q = Query { center: pos_f64(me.pos), radius: a[1].num(), owner, odf: filter(&a[3]), exclude: me.id, seen_by: None };
        Ok(handle_value(near(&c, &q).first().map(|u| u.id)))
    })?)?;

    table.set("GetNearestObjectNearPosition", def(lua, env, "GetNearestObjectNearPosition", &["vfiSibi"], |_, e, _, a| {
        let owner = e.team_filter("GetNearestObjectNearPosition", a[2].int())?;
        let A::Vec(p) = &a[0] else { unreachable!("spec v") };
        let c = e.ctx.borrow();
        let q = Query { center: pos_f64(*p), radius: a[1].num(), owner, odf: filter(&a[3]), exclude: EntityId::NONE, seen_by: None };
        Ok(handle_value(near(&c, &q).first().map(|u| u.id)))
    })?)?;

    table.set("RemoveAllObjects", def(lua, env, "RemoveAllObjects", &[""], |_, e, _, _| {
        let ids: Vec<EntityId> = {
            let c = e.ctx.borrow();
            c.view.units.iter().filter(|u| c.alive(u.id).is_some()).map(|u| u.id).collect()
        };
        for id in ids {
            e.edit("RemoveAllObjects", Edit::Kill(id))?;
        }
        Ok(Value::Nil)
    })?)?;

    // `RemoveAllCraft(bool, team)`: every live non-station unit of `team`
    // (-1: any). The bool is not measured and ignored.
    table.set("RemoveAllCraft", def(lua, env, "RemoveAllCraft", &["bi"], move |_, e, _, a| {
        let owner = e.team_filter("RemoveAllCraft", a[1].int())?;
        let candidates: Vec<EntityId> = {
            let c = e.ctx.borrow();
            c.view
                .units
                .iter()
                .filter(|u| c.alive(u.id).is_some() && owner.is_none_or(|o| u.owner == o))
                .map(|u| u.id)
                .collect()
        };
        // Craft are what is not a station, by the same classification as GetNearestStarbase.
        let mut ids = Vec::new();
        for id in candidates {
            if !station(&is_station_all, id)? {
                ids.push(id);
            }
        }
        for id in ids {
            e.edit("RemoveAllCraft", Edit::Kill(id))?;
        }
        Ok(Value::Nil)
    })?)?;

    table.set("SetImpulseSpeed", def(lua, env, "SetImpulseSpeed", &["hf"], |_, e, _, a| {
        let id = a[0].id();
        e.unit("SetImpulseSpeed", id)?;
        if a[1].num() < 0.0 {
            return Err(mlua::Error::runtime("A1.SetImpulseSpeed requires a non-negative speed"));
        }
        e.edit("SetImpulseSpeed", Edit::MaxSpeed(id, Some(Fixed::from_f64(a[1].num()))))?;
        Ok(Value::Nil)
    })?)?;

    table.set("SetRedAlert", alert(lua, env, "SetRedAlert", AlertState::Red)?)?;
    table.set("SetYellowAlert", alert(lua, env, "SetYellowAlert", AlertState::Yellow)?)?;
    table.set("SetGreenAlert", alert(lua, env, "SetGreenAlert", AlertState::Green)?)?;

    // The boolean means disabled; the edit takes `on`.
    table.set("DisableShieldGenerator", def(lua, env, "DisableShieldGenerator", &["hb"], |_, e, _, a| {
        let id = a[0].id();
        e.unit("DisableShieldGenerator", id)?;
        e.edit("DisableShieldGenerator", Edit::Subsystem(id, Subsystem::Shields, !a[1].boolean()))?;
        Ok(Value::Nil)
    })?)?;

    // `BuildObject(odf, team, path, int)` and `BuildObject(odf, team, int, x, y, z)`:
    // queue a spawn and return a pending handle (see the module docs). The
    // first point of the path is the position; the int is not measured.
    table.set("BuildObject", def(lua, env, "BuildObject", &["sisi", "siifff"], |lua, e, ov, a| {
        let (odf, team) = (a[0].text(), a[1].int());
        let owner = e.team("BuildObject", team)?;
        let pos = if ov == 0 {
            e.path_point(lua, "BuildObject", a[2].text(), 0)?
        } else {
            sta_core::Vec3 { x: Fixed::from_f64(a[3].num()), y: Fixed::from_f64(a[4].num()), z: Fixed::from_f64(a[5].num()) }
        };
        let handle = {
            let mut c = e.ctx.borrow_mut();
            let key = c
                .content
                .as_ref()
                .and_then(|db| db.key(stem(odf)))
                .ok_or_else(|| mlua::Error::runtime(format!("A1.BuildObject: unknown ODF '{odf}'")))?;
            crate::api::spawn::queue_spawn(&mut c, key, owner, pos).map_err(mlua::Error::runtime)?
        };
        let crate::api::entity_ref::EntityRef::Spawn(spawn) = handle else {
            return Err(mlua::Error::runtime("A1.BuildObject: spawn was not queued"));
        };
        let token = u32::try_from(spawn.request)
            .ok()
            .and_then(|r| TOKEN_BASE.checked_add(r))
            .ok_or_else(|| mlua::Error::runtime("A1.BuildObject: spawn handles exhausted"))?;
        e.st.borrow_mut().pending.insert(token, spawn);
        Ok(Value::Integer(i64::from(token)))
    })?)?;

    // Special flags: a named hero tied to one unit. `CreateSpecialFlag(object, name)`
    // returns the flag id. Restrictions are recorded (the DLLs pass bit masks
    // 0x20 and 0x400; their meaning is not measured) and have no effect.
    table.set("CreateSpecialFlag", def(lua, env, "CreateSpecialFlag", &["hs"], |_, e, _, a| {
        let id = a[0].id();
        e.unit("CreateSpecialFlag", id)?;
        let mut st = e.st.borrow_mut();
        st.next_flag += 1;
        let flag = st.next_flag;
        st.flags.insert(flag, Flag { unit: id, restrictions: 0 });
        Ok(Value::Integer(i64::from(flag)))
    })?)?;
    table.set("RestrictSpecialFlag", def(lua, env, "RestrictSpecialFlag", &["ii", "iii"], |_, e, _, a| {
        let mut st = e.st.borrow_mut();
        let flag = st.flags.get_mut(&(a[0].int() as u32)).ok_or_else(|| mlua::Error::runtime("A1.RestrictSpecialFlag: unknown flag"))?;
        flag.restrictions |= a[1].int();
        Ok(Value::Nil)
    })?)?;
    table.set("ClearRestrictions", def(lua, env, "ClearRestrictions", &["i"], |_, e, _, a| {
        let mut st = e.st.borrow_mut();
        let flag = st.flags.get_mut(&(a[0].int() as u32)).ok_or_else(|| mlua::Error::runtime("A1.ClearRestrictions: unknown flag"))?;
        flag.restrictions = 0;
        Ok(Value::Nil)
    })?)?;
    // The unit carrying the flag, and the owners of that unit.
    table.set("GetLocationOfFlag", def(lua, env, "GetLocationOfFlag", &["i"], |_, e, _, a| {
        let unit = e.st.borrow().flags.get(&(a[0].int() as u32)).map(|f| f.unit)
            .ok_or_else(|| mlua::Error::runtime("A1.GetLocationOfFlag: unknown flag"))?;
        e.unit("GetLocationOfFlag", unit)?;
        Ok(handle_value(Some(unit)))
    })?)?;
    table.set("GetRealTeamOfFlag", def(lua, env, "GetRealTeamOfFlag", &["i"], |_, e, _, a| flag_team(e, "GetRealTeamOfFlag", a[0].int(), false))?)?;
    table.set("GetPercievedTeamOfFlag", def(lua, env, "GetPercievedTeamOfFlag", &["i"], |_, e, _, a| flag_team(e, "GetPercievedTeamOfFlag", a[0].int(), true))?)?;
    Ok(())
}

fn flag_team(e: &Env, name: &str, flag: i64, perceived: bool) -> mlua::Result<Value> {
    let unit = e
        .st
        .borrow()
        .flags
        .get(&(flag as u32))
        .map(|f| f.unit)
        .ok_or_else(|| mlua::Error::runtime(format!("A1.{name}: unknown flag")))?;
    let c = e.ctx.borrow();
    c.alive(unit)
        .and_then(|u| if perceived { c.view.perceived_owner(unit) } else { Some(u.owner) })
        .and_then(|owner| c.player_numbering.encode(owner))
        .map(|n| Value::Integer(i64::from(n)))
        .ok_or_else(|| mlua::Error::runtime(format!("A1.{name}: the flag's unit is not alive")))
}

fn alert(lua: &Lua, env: &Env, name: &'static str, state: AlertState) -> mlua::Result<mlua::Function> {
    def(lua, env, name, &["h"], move |_, e, _, a| {
        let id = a[0].id();
        e.unit(name, id)?;
        e.edit(name, Edit::Alert(id, state))?;
        Ok(Value::Nil)
    })
}

/// `AddTransporterEffect(object)`.
pub(super) fn transporter(e: &Env, id: EntityId) -> mlua::Result<()> {
    e.unit("AddTransporterEffect", id)?;
    e.present("AddTransporterEffect", Presentation::Effect { kind: "transporter".into(), on: id })
}
