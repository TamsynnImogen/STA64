//! DLL-named queries the stock rule DLLs (`R.drl`) call from their rule
//! conditions: counts, resources, time, hit and playback state.
//!
//! Every call reads the latest mission view (or the events the runtime kept)
//! and validates its arguments strictly: exact arity, integral handles and
//! team numbers, finite numbers, strict booleans. A `char*` argument is a
//! string, or `nil` / `0` for the null pointer the DLLs pass.
//!
//! Calls whose data the sim does not carry (cargo contents, trade totals,
//! capital-ship class, off-map state) raise a
//! Lua error and add `A2.<Name>: <why>` to `unsupported()`; they never return
//! a guessed value. That list is this package's missing-seam report.
//!
//! Evidence: signatures and argument constants read from the stock rule DLLs
//! (static). The readings of the team / radius arguments follow the `A1`
//! bindings and are not measured; `UnderAttack` (a `Hit` within the last N
//! seconds) and `HasTimeElapsed` (mission time at or past N) are not measured.

use mlua::{Function, Lua, MultiValue, Table, Value};
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ};
use sta_sim::world::EntityKind;
use sta_sim::view::UnitFlags;

use crate::api::search::{Query, near};
use crate::api::{Ctx, Shared, dist, pos_f64};

/// Resource ids in the world view: 0 dilithium, 1 latinum, 2 metal.
const DILITHIUM: usize = 0;
const LATINUM: usize = 1;
const METAL: usize = 2;

fn arity(name: &str, args: &MultiValue, n: usize) -> mlua::Result<()> {
    if args.len() == n {
        Ok(())
    } else {
        Err(mlua::Error::runtime(format!("A2.{name} requires exactly {n} argument(s)")))
    }
}

fn bad(name: &str, index: usize, want: &str) -> mlua::Error {
    mlua::Error::runtime(format!("A2.{name} argument {} must be {want}", index + 1))
}

fn int(name: &str, args: &MultiValue, i: usize) -> mlua::Result<i64> {
    match &args[i] {
        Value::Integer(n) if i32::try_from(*n).is_ok() => Ok(*n),
        Value::Number(n) if n.is_finite() && n.fract() == 0.0 && n.abs() < 2_147_483_648.0 => Ok(*n as i64),
        _ => Err(bad(name, i, "a signed 32-bit integer")),
    }
}

fn handle(name: &str, args: &MultiValue, i: usize) -> mlua::Result<EntityId> {
    u32::try_from(int(name, args, i).map_err(|_| bad(name, i, "an unsigned integer handle"))?)
        .map(EntityId)
        .map_err(|_| bad(name, i, "an unsigned integer handle"))
}

fn num(name: &str, args: &MultiValue, i: usize) -> mlua::Result<f64> {
    match &args[i] {
        Value::Integer(n) => Ok(*n as f64),
        Value::Number(n) if n.is_finite() => Ok(*n),
        _ => Err(bad(name, i, "a finite number")),
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

fn team(c: &Ctx, name: &str, n: i64) -> mlua::Result<PlayerId> {
    u8::try_from(n)
        .ok()
        .and_then(|n| c.player_numbering.decode(n))
        .filter(|p| p.is_player() || *p == sta_sim::world::NEUTRAL)
        .ok_or_else(|| mlua::Error::runtime(format!("A2.{name} requires a valid team number")))
}

/// A team filter where -1 means any team.
fn team_filter(c: &Ctx, name: &str, n: i64) -> mlua::Result<Option<PlayerId>> {
    if n == -1 { Ok(None) } else { team(c, name, n).map(Some) }
}

fn alive<'c>(c: &'c Ctx, name: &str, id: EntityId) -> mlua::Result<&'c sta_sim::view::UnitView> {
    c.alive(id)
        .ok_or_else(|| mlua::Error::runtime(format!("A2.{name}: object {} is not alive", id.0)))
}

fn time(c: &Ctx) -> f64 {
    c.view.tick as f64 / TICK_HZ as f64
}

/// A bound call with no data behind it: raises, never guesses, and is counted.
fn no_data_fn(lua: &Lua, ctx: &Shared, name: &'static str, why: &'static str) -> mlua::Result<Function> {
    let c = ctx.clone();
    lua.create_function(move |_, _: MultiValue| -> mlua::Result<Value> {
        c.borrow_mut().unsupported(&format!("A2.{name}: {why}"));
        Err(mlua::Error::runtime(format!("A2.{name} is not available: {why}")))
    })
}

/// `name(args...)` with `f` run on the validated arguments.
fn query<R: mlua::IntoLua>(
    lua: &Lua,
    ctx: &Shared,
    name: &'static str,
    n: usize,
    f: impl Fn(&Ctx, &MultiValue) -> mlua::Result<R> + 'static,
) -> mlua::Result<Function> {
    let c = ctx.clone();
    lua.create_function(move |_, args: MultiValue| {
        arity(name, &args, n)?;
        f(&c.borrow(), &args)
    })
}

fn resource(lua: &Lua, ctx: &Shared, name: &'static str, slot: usize) -> mlua::Result<Function> {
    query(lua, ctx, name, 1, move |c, a| {
        let p = team(c, name, int(name, a, 0)?)?;
        if !p.is_player() {
            return Err(mlua::Error::runtime(format!("A2.{name} requires a player team")));
        }
        // A listed player with no entry for the resource holds none.
        let view = c
            .view
            .player(p)
            .ok_or_else(|| mlua::Error::runtime(format!("A2.{name}: the view has no such team")))?;
        let stock = view.resources.get(slot).copied().unwrap_or_default();
        Ok((f64::from(stock.to_f32())).round() as i64)
    })
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let is_station: Function = table.get("IsStation")?;

    // Searches over live units: `-1` as a team means any; a null ODF any ODF.
    table.set("CountUnits", query(lua, ctx, "CountUnits", 2, |c, a| {
        let owner = team_filter(c, "CountUnits", int("CountUnits", a, 0)?)?;
        c.note_team_query(owner, "CountUnits");
        let odf = text("CountUnits", a, 1)?;
        let n = c
            .view
            .units
            .iter()
            .filter(|u| c.alive(u.id).is_some() && owner.is_none_or(|o| u.owner == o))
            .filter(|u| crate::api::search::odf_ok(c, u, odf.as_deref()))
            .count();
        Ok(n as i64)
    })?)?;
    table.set("CountUnitsNearObject", query(lua, ctx, "CountUnitsNearObject", 4, |c, a| {
        let name = "CountUnitsNearObject";
        let id = handle(name, a, 0)?;
        c.note_query(id, name);
        let centre = alive(c, name, id)?;
        let odf = text(name, a, 3)?;
        let q = Query {
            center: pos_f64(centre.pos),
            radius: num(name, a, 1)?,
            owner: team_filter(c, name, int(name, a, 2)?)?,
            odf: odf.as_deref(),
            exclude: centre.id,
            seen_by: None,
        };
        Ok(near(c, &q).len() as i64)
    })?)?;
    // The bool is not measured and ignored.
    table.set("CountVisibleUnits", query(lua, ctx, "CountVisibleUnits", 4, |c, a| {
        let name = "CountVisibleUnits";
        let observer = team(c, name, int(name, a, 0)?)?;
        boolean(name, a, 2)?;
        let odf = text(name, a, 3)?;
        let owner = team_filter(c, name, int(name, a, 1)?)?;
        c.note_team_query(owner, name);
        let q = Query {
            center: (0.0, 0.0, 0.0),
            radius: f64::INFINITY,
            owner,
            odf: odf.as_deref(),
            exclude: EntityId::NONE,
            seen_by: Some(observer),
        };
        Ok(near(c, &q).len() as i64)
    })?)?;
    // Units near the object that the object's owner can see (reading not measured).
    table.set("CountVisibleUnitsNearObject", query(lua, ctx, "CountVisibleUnitsNearObject", 5, |c, a| {
        let name = "CountVisibleUnitsNearObject";
        let centre = alive(c, name, handle(name, a, 0)?)?;
        boolean(name, a, 3)?;
        let odf = text(name, a, 4)?;
        let q = Query {
            center: pos_f64(centre.pos),
            radius: num(name, a, 1)?,
            owner: team_filter(c, name, int(name, a, 2)?)?,
            odf: odf.as_deref(),
            exclude: centre.id,
            seen_by: Some(centre.owner),
        };
        Ok(near(c, &q).len() as i64)
    })?)?;

    // Stations of a team, by the `IsStation` classification (its errors propagate).
    let c = ctx.clone();
    table.set("CountStations", lua.create_function(move |_, args: MultiValue| {
        arity("CountStations", &args, 1)?;
        let candidates: Vec<EntityId> = {
            let c = c.borrow();
            let owner = team_filter(&c, "CountStations", int("CountStations", &args, 0)?)?;
            c.note_team_query(owner, "CountStations");
            c.view.units.iter().filter(|u| c.alive(u.id).is_some() && owner.is_none_or(|o| u.owner == o)).map(|u| u.id).collect()
        };
        let mut n = 0i64;
        for id in candidates {
            if is_station.call::<bool>(i64::from(id.0)).map_err(|e| mlua::Error::runtime(e.to_string()))? {
                n += 1;
            }
        }
        Ok(n)
    })?)?;
    table.set("CountCapitalShips", no_data_fn(lua, ctx, "CountCapitalShips", "no capital-ship classification in the content data")?)?;

    // Unit state.
    table.set("GetCrew", {
        let c = ctx.clone();
        lua.create_function(move |_, args: MultiValue| {
            arity("GetCrew", &args, 1)?;
            let id = handle("GetCrew", &args, 0)?;
            let mut c = c.borrow_mut();
            // A handle naming nothing live answers -1 (LUA-21; read from the executable, static).
            let Some(unit) = c.alive(id) else {
                c.dead_handle("A2", "GetCrew", id)?;
                return Ok(crate::api::ABSENT_AMOUNT);
            };
            match unit.crew {
                Some(crew) => Ok(f64::from(crew[0].to_f32())),
                None => {
                    let why = "A2.GetCrew: the unit has no initialised crew";
                    c.unsupported(why);
                    Err(mlua::Error::runtime(why))
                }
            }
        })?
    })?;
    // `GetPlanetCivPop(planet)` (SIM-30): the population truncated to an integer, -1
    // for a handle that is gone (read from the executable, static). A unit with no
    // colony row reads 0.
    table.set("GetPlanetCivPop", query(lua, ctx, "GetPlanetCivPop", 1, |c, a| {
        let id = handle("GetPlanetCivPop", a, 0)?;
        if c.alive(id).is_none() {
            return Ok(-1.0);
        }
        let pop = c.view.colonies.iter().find(|r| r.id == id).map_or(0.0, |r| (r.population.to_bits() as f64 / 4_294_967_296.0).trunc());
        Ok(pop)
    })?)?;
    // An object gone from the view is not exploding.
    table.set("IsExploding", query(lua, ctx, "IsExploding", 1, |c, a| {
        Ok(c.unit(handle("IsExploding", a, 0)?).is_some_and(|u| u.flags.contains(UnitFlags::DYING)))
    })?)?;
    // A hit within the last `seconds` of mission time (reading not measured).
    table.set("UnderAttack", query(lua, ctx, "UnderAttack", 2, |c, a| {
        let id = handle("UnderAttack", a, 0)?;
        let window = num("UnderAttack", a, 1)?;
        if window < 0.0 {
            return Err(bad("UnderAttack", 1, "a non-negative number of seconds"));
        }
        alive(c, "UnderAttack", id)?;
        let within = (window * TICK_HZ as f64).round() as u64;
        Ok(c.last_hit.get(&id).is_some_and(|t| c.view.tick.saturating_sub(*t) <= within))
    })?)?;
    // IsCompromised(object, team): true when the object has been lost to `team`,
    // i.e. it is gone or dying, is not a craft or station, is a derelict, or
    // its permanent owner is not `team` (LUA-12). Read from the executable,
    // static (A2 and A1; nothing measured). The derelict latch is
    // approximated by a crew of under one with a positive maximum (not
    // modelled: the `CraftCannotDie` exemption and the latch clearing only
    // when crew returns to one), and a temporary takeover (mind control) shows
    // as the new owner here, where the engine keeps the original (SIM-59).
    table.set("IsCompromised", query(lua, ctx, "IsCompromised", 2, |c, a| {
        let id = handle("IsCompromised", a, 0)?;
        c.note_query(id, "IsCompromised");
        // Any integer is accepted, as the engine's own compare does: a team
        // number that no seat has just makes every object compromised.
        let team = int("IsCompromised", a, 1)?;
        let Some(u) = c.alive(id) else { return Ok(true) };
        // Craft or station by STA64's class mapping (`EntityKind::of_def`), an
        // approximation of the engine's class test; no content: a craft.
        let fixture = c.def(u).is_some_and(|d| !matches!(EntityKind::of_def(d), EntityKind::Craft | EntityKind::Station));
        let derelict = u.crew.is_some_and(|[now, max]| now < Fixed::ONE && max > Fixed::ZERO);
        let owner = c.player_numbering.encode(u.owner).map(i64::from);
        Ok(fixture || derelict || owner != Some(team))
    })?)?;
    table.set("IsShipAliveAndOffMap", no_data_fn(lua, ctx, "IsShipAliveAndOffMap", "the world view carries no off-map state")?)?;
    table.set("IsCargoShipReadyToLoad", no_data_fn(lua, ctx, "IsCargoShipReadyToLoad", "the world view carries no cargo state")?)?;
    table.set("IsCargoShipFullAndReadyToSend", no_data_fn(lua, ctx, "IsCargoShipFullAndReadyToSend", "the world view carries no cargo state")?)?;

    // Team stock and trade.
    table.set("GetDilithium", resource(lua, ctx, "GetDilithium", DILITHIUM)?)?;
    table.set("GetLatinum", resource(lua, ctx, "GetLatinum", LATINUM)?)?;
    table.set("GetMetal", resource(lua, ctx, "GetMetal", METAL)?)?;
    table.set("GetLatinumTraded", no_data_fn(lua, ctx, "GetLatinumTraded", "the world view carries no trade totals")?)?;

    // Mission time.
    table.set("GetTime", query(lua, ctx, "GetTime", 0, |c, _| Ok(time(c)))?)?;
    table.set("HasTimeElapsed", query(lua, ctx, "HasTimeElapsed", 1, |c, a| {
        Ok(time(c) >= num("HasTimeElapsed", a, 0)?)
    })?)?;

    // Playback: started by the mission, finished when the client reports it
    // (`A2.AudioMessageDone(id)`, `A2.MovieDone()`).
    table.set("IsAudioMessageDone", query(lua, ctx, "IsAudioMessageDone", 1, |c, a| {
        let id = int("IsAudioMessageDone", a, 0)?;
        Ok(!u32::try_from(id).is_ok_and(|id| c.playback.sounds.contains(&id)))
    })?)?;
    table.set("IsMoviePlaying", query(lua, ctx, "IsMoviePlaying", 0, |c, _| Ok(c.playback.movie))?)?;
    let c = ctx.clone();
    table.set("AudioMessageDone", lua.create_function(move |_, args: MultiValue| {
        arity("AudioMessageDone", &args, 1)?;
        let id = int("AudioMessageDone", &args, 0)?;
        if let Ok(id) = u32::try_from(id) {
            c.borrow_mut().playback.sounds.remove(&id);
        }
        Ok(())
    })?)?;
    let c = ctx.clone();
    table.set("MovieDone", lua.create_function(move |_, args: MultiValue| {
        arity("MovieDone", &args, 0)?;
        c.borrow_mut().playback.movie = false;
        Ok(())
    })?)?;
    Ok(())
}

/// `A2.GetDistance(object, path, point)`: the object to a point of
/// `A2.paths[path]`, a table the host fills from the map.
pub(super) fn path_distance(c: &mut Ctx, lua: &Lua, args: &MultiValue) -> mlua::Result<f64> {
    let name = "GetDistance";
    let id = handle(name, args, 0)?;
    let path = text(name, args, 1)?.ok_or_else(|| bad(name, 1, "a path name"))?;
    let index = int(name, args, 2)?;
    let from = pos_f64(alive(c, name, id)?.pos);
    let missing = |c: &mut Ctx, why: String| {
        let why = format!("A2.{name}: {why}");
        c.unsupported(&why);
        mlua::Error::runtime(why)
    };
    let a2: Table = lua.globals().raw_get("A2")?;
    let Value::Table(paths) = a2.raw_get::<Value>("paths")? else {
        return Err(missing(c, "A2.paths is missing".into()));
    };
    let Value::Table(points) = paths.raw_get::<Value>(path.as_str())? else {
        return Err(missing(c, format!("path '{path}' is not in A2.paths (the host fills it from the map)")));
    };
    let point = usize::try_from(index).ok().and_then(|i| points.raw_get::<Value>(i + 1).ok());
    let Some(Value::Table(p)) = point else {
        return Err(missing(c, format!("path '{path}' has no point {index}")));
    };
    let get = |k: &str| p.raw_get::<f64>(k).ok().filter(|v| v.is_finite());
    match (get("x"), get("y"), get("z")) {
        (Some(x), Some(y), Some(z)) => Ok(dist(from, (x, y, z))),
        _ => Err(missing(c, format!("path '{path}' has a malformed point"))),
    }
}

#[cfg(test)]
#[path = "conditions/tests.rs"]
mod tests;
