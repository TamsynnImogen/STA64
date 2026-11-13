//! DLL-named object creation, searches and flags (LUA-05 step 2).
//!
//! `BuildObject`, `SetSpecialForcesFlag`, `RecruitSpecialForces`,
//! `GetNearestObjectNearPosition` / `NearMe`, `GetFirstCraft`, the
//! `GetLocation(path, index)` overload, `DestroyCritical`, `ExplodeNow`,
//! `SetCanBeDecommissioned` and `DisableShieldGenerator`. Calls the sim seam
//! cannot serve yet are bound as raising-and-counted (`no_data_fn`), each with
//! the reason; nothing is a silent no-op.
//!
//! # Handles
//!
//! `BuildObject` queues a SCRIPT spawn and returns a pending handle
//! (`TOKEN_BASE + request`, the same range as the `A1` table). [`finish`] wraps
//! every function of the `A2` table so that a token which has a spawn receipt
//! is replaced by the real entity id before the call; an unresolved token is
//! passed on unchanged and so names no live object (`Exists` false, others
//! raise) until the next tick delivers the receipt. `finish` must run after
//! every other module has registered its functions.
//!
//! # Evidence grades
//!
//! Each rule below is `exe` = read from the executable, static (the
//! `ScriptInterfaceImp` method bodies of the shipped Armada II binary), `args`
//! = read from the 36 stock mission scripts' constant arguments, or `not
//! measured`. Findings are kept outside this repository.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table, Value};
use sta_core::{EntityId, Fixed, Vec3};
use sta_sim::command::{Edit, Subsystem};
use sta_sim::view::UnitFlags;

use super::objects::{handle, integer, live};
use super::queries::craft_family;
use crate::api::entity_ref::{EntityRef, SpawnHandle, SpawnStatus};
use crate::api::search::stem;
use crate::api::{Ctx, Shared, edit_feedback, pos_f64, spawn, to_vec3, vector};

/// Handles at or above this are pending spawns (same range as `A1`).
const TOKEN_BASE: u32 = 0x4000_0000;

/// `DestroyCritical` hits with this much damage (exe: the constant it builds).
const CRITICAL_DAMAGE: f64 = 100_000.0;

#[derive(Default)]
struct State {
    pending: BTreeMap<u32, Rc<SpawnHandle>>,
    /// Handles flagged as special forces: a real id or a pending token.
    special: BTreeSet<u32>,
}

type St = Rc<RefCell<State>>;

enum Token {
    Id(EntityId),
    Pending,
    Failed,
    Unknown,
}

impl State {
    fn token(&self, raw: u32) -> Token {
        match self.pending.get(&raw).map(|h| h.state.get()) {
            None => Token::Unknown,
            Some(SpawnStatus::Resolved(id)) => Token::Id(id),
            Some(SpawnStatus::Failed) => Token::Failed,
            Some(SpawnStatus::Queued | SpawnStatus::Submitted(_)) => Token::Pending,
        }
    }

    fn is_special(&self, id: EntityId) -> bool {
        self.special.contains(&id.0)
            || self
                .special
                .iter()
                .any(|t| *t >= TOKEN_BASE && matches!(self.token(*t), Token::Id(i) if i == id))
    }

    /// A value a script passed in: a resolved token becomes its entity id.
    fn translate(&self, v: Value) -> Value {
        let raw = match &v {
            Value::Integer(n) => u32::try_from(*n).ok(),
            Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 && *n <= f64::from(u32::MAX) => Some(*n as u32),
            _ => None,
        };
        match raw.filter(|r| *r >= TOKEN_BASE).map(|r| self.token(r)) {
            Some(Token::Id(id)) => Value::Integer(i64::from(id.0)),
            _ => v,
        }
    }
}

/// Why a handle that is a `BuildObject` token cannot be used yet: still
/// pending, or its spawn failed. `None` for resolved, unknown and real handles.
pub(super) fn token_problem(lua: &Lua, raw: u32) -> Option<&'static str> {
    if raw < TOKEN_BASE {
        return None;
    }
    let st = lua.app_data_ref::<St>()?;
    match st.borrow().token(raw) {
        Token::Pending => Some("handle is still being built; use it after the next tick"),
        Token::Failed => Some("handle names no object: spawn failed"),
        Token::Id(_) | Token::Unknown => None,
    }
}

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

fn signed(name: &str, args: &MultiValue, i: usize) -> mlua::Result<i64> {
    match &args[i] {
        Value::Integer(n) if i32::try_from(*n).is_ok() => Ok(*n),
        Value::Number(n) if n.is_finite() && n.fract() == 0.0 && n.abs() < 2_147_483_648.0 => Ok(*n as i64),
        _ => Err(bad(name, i, "a signed 32-bit integer")),
    }
}

fn number(name: &str, args: &MultiValue, i: usize) -> mlua::Result<f64> {
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

fn vec3_arg(name: &str, args: &MultiValue, i: usize) -> mlua::Result<(f64, f64, f64)> {
    let Value::Table(t) = &args[i] else { return Err(bad(name, i, "a Vector")) };
    let get = |k: &str| t.get::<Value>(k).ok().and_then(|v| match v {
        Value::Integer(n) => Some(n as f64),
        Value::Number(n) if n.is_finite() => Some(n),
        _ => None,
    });
    match (get("x"), get("y"), get("z")) {
        (Some(x), Some(y), Some(z)) => Ok((x, y, z)),
        _ => Err(bad(name, i, "a Vector with finite x, y and z")),
    }
}

/// A bound call whose data or sim seam does not exist: raises, never guesses,
/// and is counted in `unsupported()`.
fn no_data_fn(lua: &Lua, ctx: &Shared, name: &'static str, why: &'static str) -> mlua::Result<Function> {
    let c = ctx.clone();
    lua.create_function(move |_, _: MultiValue| -> mlua::Result<Value> {
        c.borrow_mut().unsupported(&format!("A2.{name}: {why}"));
        Err(mlua::Error::runtime(format!("A2.{name} is not available: {why}")))
    })
}

/// `name(id, number)` queued as the typed edit `make` builds.
fn float_edit(lua: &Lua, ctx: &Shared, name: &'static str, label: &'static str, make: fn(EntityId, Fixed) -> Edit) -> mlua::Result<Function> {
    let c = ctx.clone();
    lua.create_function(move |_, args: MultiValue| {
        arity(name, &args, 2)?;
        let value = number(name, &args, 1)?;
        if !value.is_finite() {
            return Err(bad(name, 1, "a finite number"));
        }
        let id = EntityId(integer(name, &args[0])?);
        live(&c, name, id)?;
        edit_feedback::queue(&mut c.borrow_mut(), label, make(id, Fixed::from_f64(value)))
    })
}

fn path_error(c: &Shared, name: &str, why: String) -> mlua::Error {
    let why = format!("A2.{name}: {why}");
    c.borrow_mut().unsupported(&why);
    mlua::Error::runtime(why)
}

/// The table of `A2.paths[path]`: an exact name first, else the one key that
/// matches ignoring ASCII case. `Ok(Err(why))` when several keys match
/// case-insensitively (no deterministic winner), `Ok(Ok(None))` when none does.
pub(super) fn lookup_path(paths: &Table, path: &str) -> mlua::Result<Result<Option<Table>, String>> {
    if let Value::Table(t) = paths.raw_get::<Value>(path)? {
        return Ok(Ok(Some(t)));
    }
    let mut found: Vec<(String, Table)> = Vec::new();
    for pair in paths.pairs::<Value, Value>() {
        if let (Value::String(k), Value::Table(t)) = pair?
            && let Ok(k) = k.to_str()
            && k.eq_ignore_ascii_case(path)
        {
            found.push((k.to_string(), t));
        }
    }
    match found.len() {
        0 => Ok(Ok(None)),
        1 => Ok(Ok(found.pop().map(|(_, t)| t))),
        _ => {
            let mut names: Vec<String> = found.into_iter().map(|(k, _)| k).collect();
            names.sort();
            Ok(Err(format!("path '{path}' is ambiguous: A2.paths has {}", names.join(", "))))
        }
    }
}

/// Point `index` (0-based) of `A2.paths[path]`, a table the host fills from
/// the map. `Ok(None)` when the path exists but has no such point.
fn path_point(lua: &Lua, ctx: &Shared, name: &str, path: &str, index: i64) -> mlua::Result<Option<(f64, f64, f64)>> {
    let a2: Table = lua.globals().raw_get("A2")?;
    let Value::Table(paths) = a2.raw_get::<Value>("paths")? else {
        return Err(path_error(ctx, name, "A2.paths is missing".into()));
    };
    let points = match lookup_path(&paths, path)? {
        Ok(Some(t)) => Value::Table(t),
        Ok(None) => Value::Nil,
        Err(why) => return Err(path_error(ctx, name, why)),
    };
    let Value::Table(points) = points else {
        return Err(path_error(ctx, name, format!("path '{path}' is not in A2.paths (the host fills it from the map)")));
    };
    let Some(slot) = usize::try_from(index).ok() else { return Ok(None) };
    let Value::Table(p) = points.raw_get::<Value>(slot + 1)? else { return Ok(None) };
    let get = |k: &str| p.raw_get::<f64>(k).ok().filter(|v| v.is_finite());
    match (get("x"), get("y"), get("z")) {
        (Some(x), Some(y), Some(z)) => Ok(Some((x, y, z))),
        _ => Err(path_error(ctx, name, format!("path '{path}' has a malformed point"))),
    }
}

/// Real owner of `u` as the script's team number.
fn team_number(c: &Ctx, owner: sta_core::PlayerId) -> Option<i64> {
    c.player_numbering.encode(owner).map(i64::from)
}

struct Search<'a> {
    center: (f64, f64, f64),
    /// Negative: unlimited.
    radius: f64,
    /// Negative: any team.
    team: i64,
    odf: &'a Option<String>,
    /// Also require the perceived team to match.
    perceived: bool,
    exclude: Option<EntityId>,
}

/// The nearest live unit (exe: GetNearestObjectNearPosition). Squared XZ
/// distance, limit inclusive, and a tie goes to the later unit in the view
/// (the executable replaces on `<=`; its list order is taken as ascending id,
/// not measured). The executable also skips derelict crafts and, for a
/// non-negative mask argument, tests a bit of a class field: the view has no
/// derelict state and the mask is rejected by the caller.
fn nearest(c: &Ctx, q: &Search) -> Option<EntityId> {
    let mut best = if q.radius >= 0.0 { q.radius * q.radius } else { f64::INFINITY };
    let mut found = None;
    for u in &c.view.units {
        if u.flags.contains(UnitFlags::DYING) || q.exclude == Some(u.id) {
            continue;
        }
        if q.team >= 0 {
            if team_number(c, u.owner) != Some(q.team) {
                continue;
            }
            if q.perceived
                && c.view.perceived_owner(u.id).and_then(|p| team_number(c, p)) != Some(q.team)
            {
                continue;
            }
        }
        if let Some(odf) = q.odf
            && (odf.is_empty() || !stem(&c.odf_name(u)).eq_ignore_ascii_case(stem(odf)))
        {
            continue;
        }
        let p = pos_f64(u.pos);
        let d = {
            let (dx, dz) = (q.center.0 - p.0, q.center.2 - p.2);
            dx * dx + dz * dz
        };
        if d <= best {
            best = d;
            found = Some(u.id);
        }
    }
    found
}

fn id_value(id: Option<EntityId>) -> Value {
    Value::Integer(id.map_or(0, |id| i64::from(id.0)))
}

/// A live unit as a script handle argument, or an error naming the call.
fn live_unit(ctx: &Shared, name: &str, args: &MultiValue) -> mlua::Result<EntityId> {
    let id = handle(name, args, 1)?;
    live(ctx, name, id)?;
    Ok(id)
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let st: St = Rc::new(RefCell::new(State::default()));
    let _ = lua.set_app_data(st.clone());

    // BuildObject(odf, team, path, index) and
    // BuildObject(odf, team, object, dx, dy, dz): queue a SCRIPT spawn and
    // return a pending handle. Rules (exe): the path form places the object at
    // point `index` of the path with identity orientation and returns 0 when
    // the point does not exist. A path missing from `A2.paths` raises here and
    // is counted (host data is missing); what the engine does for a missing
    // path is not measured. The object form places it at the
    // object's frame offset by (dx, dy, dz). A failed creation returns 0.
    // Not measured: the object form's rotation convention, so a non-zero
    // offset raises (the stock call passes zeros). Heading is the spawn
    // default (+Z).
    let (c, s) = (ctx.clone(), st.clone());
    table.set(
        "BuildObject",
        lua.create_function(move |lua, args: MultiValue| {
            let name = "BuildObject";
            if args.len() != 4 && args.len() != 6 {
                return Err(mlua::Error::runtime("A2.BuildObject requires exactly 4 or 6 argument(s)"));
            }
            let odf = text(name, &args, 0)?.filter(|s| !s.is_empty()).ok_or_else(|| bad(name, 0, "an ODF name"))?;
            let team = signed(name, &args, 1)?;
            let owner = {
                let c = c.borrow();
                u8::try_from(team)
                    .ok()
                    .and_then(|t| c.player_numbering.decode(t))
                    .filter(|p| p.is_player() || *p == sta_sim::world::NEUTRAL)
                    .ok_or_else(|| mlua::Error::runtime("A2.BuildObject requires a valid team number"))?
            };
            let pos: Vec3 = if args.len() == 4 {
                let path = text(name, &args, 2)?.ok_or_else(|| bad(name, 2, "a path name"))?;
                let index = signed(name, &args, 3)?;
                match path_point(lua, &c, name, &path, index)? {
                    Some((x, y, z)) => to_vec3(x, y, z),
                    None => return Ok(Value::Integer(0)),
                }
            } else {
                let reference = EntityId(integer(name, &args[2])?);
                let offset = [number(name, &args, 3)?, number(name, &args, 4)?, number(name, &args, 5)?];
                if offset != [0.0; 3] {
                    return Err(mlua::Error::runtime(
                        "A2.BuildObject: a non-zero offset from an object needs the object's rotation convention (not measured)",
                    ));
                }
                let c = c.borrow();
                let unit = c.alive(reference).ok_or_else(|| {
                    mlua::Error::runtime(format!("A2.BuildObject: object {} is not alive", reference.0))
                })?;
                unit.pos
            };
            let mut cx = c.borrow_mut();
            let Some(key) = cx.content.as_ref().and_then(|db| db.key(stem(&odf))) else {
                cx.unsupported(&format!("A2.BuildObject: unknown ODF '{odf}'"));
                return Ok(Value::Integer(0));
            };
            let queued = spawn::queue_spawn(&mut cx, key, owner, pos).map_err(mlua::Error::runtime)?;
            let EntityRef::Spawn(handle) = queued else {
                return Err(mlua::Error::runtime("A2.BuildObject: spawn was not queued"));
            };
            let token = u32::try_from(handle.request)
                .ok()
                .and_then(|r| TOKEN_BASE.checked_add(r))
                .ok_or_else(|| mlua::Error::runtime("A2.BuildObject: spawn handles exhausted"))?;
            s.borrow_mut().pending.insert(token, handle);
            Ok(Value::Integer(i64::from(token)))
        })?,
    )?;

    // int SetSpecialForcesFlag(int, bool). Rule (exe): for a live Craft, store
    // the flag on it and return its handle; anything else returns 0 and does
    // nothing. Setting it also detaches the craft from its AI team; the AI
    // seats of a mission do not read the flag yet (AI-02 / SIM-58 own that
    // consumer), so its one consumer today is RecruitSpecialForces. A pending
    // `BuildObject` token is flagged at once and the flag follows its object.
    let (c, s) = (ctx.clone(), st.clone());
    table.set(
        "SetSpecialForcesFlag",
        lua.create_function(move |_, args: MultiValue| {
            let name = "SetSpecialForcesFlag";
            arity(name, &args, 2)?;
            let raw = integer(name, &args[0])?;
            let on = boolean(name, &args, 1)?;
            let valid = if raw >= TOKEN_BASE {
                matches!(s.borrow().token(raw), Token::Pending | Token::Id(_))
            } else {
                let c = c.borrow();
                c.alive(EntityId(raw)).is_some_and(|u| {
                    u.progress.is_none() && c.def(u).is_none_or(|d| craft_family(&d.class.0) != Some(false))
                })
            };
            if !valid {
                return Ok(0i64);
            }
            let mut s = s.borrow_mut();
            if on {
                s.special.insert(raw);
            } else {
                s.special.remove(&raw);
                // A flag stored under the token of this id goes too.
                let stale: Vec<u32> = s
                    .special
                    .iter()
                    .copied()
                    .filter(|t| *t >= TOKEN_BASE && matches!(s.token(*t), Token::Id(i) if i.0 == raw))
                    .collect();
                for t in stale {
                    s.special.remove(&t);
                }
            }
            Ok(i64::from(raw))
        })?,
    )?;

    // bool RecruitSpecialForces(char *odf, int team, int &out). Rule (exe): the
    // first craft of that team and ODF that is not already special forces is
    // flagged and its handle stored in `out`; false (and out 0) when the ODF
    // is null/unknown or none qualifies. Lua has no out parameter: the call
    // returns `ok, handle`; a third argument is accepted and ignored. The
    // executable also requires the craft to have no AI team: nothing here
    // has one.
    let (c, s) = (ctx.clone(), st.clone());
    table.set(
        "RecruitSpecialForces",
        lua.create_function(move |_, args: MultiValue| {
            let name = "RecruitSpecialForces";
            if args.len() != 2 && args.len() != 3 {
                return Err(mlua::Error::runtime("A2.RecruitSpecialForces requires exactly 2 or 3 argument(s)"));
            }
            let odf = text(name, &args, 0)?;
            let team = signed(name, &args, 1)?;
            let c = c.borrow();
            let mut s = s.borrow_mut();
            let found = odf.filter(|o| !o.is_empty()).and_then(|odf| {
                c.view
                    .units
                    .iter()
                    .filter(|u| !u.flags.contains(UnitFlags::DYING) && u.progress.is_none())
                    .filter(|u| team_number(&c, u.owner) == Some(team))
                    .filter(|u| stem(&c.odf_name(u)).eq_ignore_ascii_case(stem(&odf)))
                    .filter(|u| c.def(u).is_none_or(|d| craft_family(&d.class.0) != Some(false)))
                    .find(|u| !s.is_special(u.id))
                    .map(|u| u.id)
            });
            if let Some(id) = found {
                s.special.insert(id.0);
            }
            Ok((found.is_some(), found.map_or(0, |id| i64::from(id.0))))
        })?,
    )?;

    // int GetNearestObjectNearPosition(Vector, radius, team, odf, mask,
    // perceived, exclude). Rules (exe): see `nearest`; radius < 0 is
    // unlimited, team < 0 any team, a null odf any ODF, `perceived` also
    // requires the perceived team to match, `exclude` is a handle to skip
    // (0 none). All stock calls pass mask -1; any other mask is a class-field
    // bit test that is not measured and raises.
    let c = ctx.clone();
    table.set(
        "GetNearestObjectNearPosition",
        lua.create_function(move |_, args: MultiValue| {
            let name = "GetNearestObjectNearPosition";
            arity(name, &args, 7)?;
            let center = vec3_arg(name, &args, 0)?;
            let radius = number(name, &args, 1)?;
            let team = signed(name, &args, 2)?;
            let odf = text(name, &args, 3)?;
            let mask = signed(name, &args, 4)?;
            let perceived = boolean(name, &args, 5)?;
            let exclude = integer(name, &args[6])?;
            if mask != -1 {
                return Err(path_error(&c, name, "a class-bit mask other than -1 is not measured".into()));
            }
            let q = Search { center, radius, team, odf: &odf, perceived, exclude: (exclude != 0).then_some(EntityId(exclude)) };
            Ok(id_value(nearest(&c.borrow(), &q)))
        })?,
    )?;

    // int GetNearestObjectNearMe(object, radius, team, odf, mask, perceived):
    // GetNearestObjectNearPosition around a live object's position excluding
    // the object itself (exe); 0 when the object is not live.
    let c = ctx.clone();
    table.set(
        "GetNearestObjectNearMe",
        lua.create_function(move |_, args: MultiValue| {
            let name = "GetNearestObjectNearMe";
            arity(name, &args, 6)?;
            let me = EntityId(integer(name, &args[0])?);
            let radius = number(name, &args, 1)?;
            let team = signed(name, &args, 2)?;
            let odf = text(name, &args, 3)?;
            let mask = signed(name, &args, 4)?;
            let perceived = boolean(name, &args, 5)?;
            if mask != -1 {
                return Err(path_error(&c, name, "a class-bit mask other than -1 is not measured".into()));
            }
            let c = c.borrow();
            let Some(unit) = c.alive(me) else { return Ok(id_value(None)) };
            let q = Search { center: pos_f64(unit.pos), radius, team, odf: &odf, perceived, exclude: Some(me) };
            Ok(id_value(nearest(&c, &q)))
        })?,
    )?;

    // int GetFirstCraft(void): the first Craft of the world (exe: the first
    // element of the craft list; its order is taken as creation order, not
    // measured), 0 when there is none.
    let c = ctx.clone();
    table.set(
        "GetFirstCraft",
        lua.create_function(move |_, args: MultiValue| {
            arity("GetFirstCraft", &args, 0)?;
            let c = c.borrow();
            let first = c
                .view
                .units
                .iter()
                .filter(|u| !u.flags.contains(UnitFlags::DYING) && u.progress.is_none())
                .find(|u| c.def(u).is_none_or(|d| craft_family(&d.class.0) != Some(false)))
                .map(|u| u.id);
            Ok(id_value(first))
        })?,
    )?;

    // void DestroyCritical(int): exe: the live Craft takes one hit of 100000
    // damage (type 6). Queued as a lethal SCRIPT hull `Damage`; an immortal
    // object keeps one hull (whether the native hit honours that is not
    // measured).
    let c = ctx.clone();
    table.set(
        "DestroyCritical",
        lua.create_function(move |_, args: MultiValue| {
            let id = live_unit(&c, "DestroyCritical", &args)?;
            edit_feedback::queue(&mut c.borrow_mut(), "A2.DestroyCritical", Edit::Damage(id, Fixed::from_f64(CRITICAL_DAMAGE)))
        })?,
    )?;

    // void ExplodeNow(int): exe: the Craft's own destroy method runs at once.
    // Queued as a SCRIPT Kill (the sim's only destroy edit; the Destroyed event
    // carries no explosion/removal difference, so this equals RemoveObject).
    let c = ctx.clone();
    table.set(
        "ExplodeNow",
        lua.create_function(move |_, args: MultiValue| {
            let id = live_unit(&c, "ExplodeNow", &args)?;
            edit_feedback::queue(&mut c.borrow_mut(), "A2.ExplodeNow", Edit::Kill(id))
        })?,
    )?;

    // void SetCanBeDecommissioned(int, bool): exe: stores the bool on the
    // Craft. Queued as the typed `Decommissionable` edit; the sim rejects it
    // until decommissioning exists, which the edit feedback reports and counts.
    let c = ctx.clone();
    table.set(
        "SetCanBeDecommissioned",
        lua.create_function(move |_, args: MultiValue| {
            let name = "SetCanBeDecommissioned";
            arity(name, &args, 2)?;
            let on = boolean(name, &args, 1)?;
            let id = EntityId(integer(name, &args[0])?);
            live(&c, name, id)?;
            edit_feedback::queue(&mut c.borrow_mut(), "A2.SetCanBeDecommissioned", Edit::Decommissionable(id, on))
        })?,
    )?;

    // void DisableShieldGenerator(int, bool): the bool means disabled (the
    // sim edit takes `on`). The sim rejects the shield subsystem edit until
    // shields are switchable; reported and counted by the edit feedback.
    let c = ctx.clone();
    table.set(
        "DisableShieldGenerator",
        lua.create_function(move |_, args: MultiValue| {
            let name = "DisableShieldGenerator";
            arity(name, &args, 2)?;
            let disabled = boolean(name, &args, 1)?;
            let id = EntityId(integer(name, &args[0])?);
            live(&c, name, id)?;
            edit_feedback::queue(&mut c.borrow_mut(), "A2.DisableShieldGenerator", Edit::Subsystem(id, Subsystem::Shields, !disabled))
        })?,
    )?;

    // GetLocation(path, index): point `index` of the path (exe: the native
    // call reads the path's point array). The one-argument form stays the
    // existing handle query.
    let single: Function = table.get("GetLocation")?;
    let c = ctx.clone();
    table.set(
        "GetLocation",
        lua.create_function(move |lua, args: MultiValue| {
            if args.len() != 2 {
                return single.call::<Value>(args);
            }
            let name = "GetLocation";
            let path = text(name, &args, 0)?.ok_or_else(|| bad(name, 0, "a path name"))?;
            let index = signed(name, &args, 1)?;
            match path_point(lua, &c, name, &path, index)? {
                Some(p) => Ok(Value::Table(vector(lua, p)?)),
                None => Err(path_error(&c, name, format!("path '{path}' has no point {index}"))),
            }
        })?,
    )?;

    seams(lua, ctx, table)
}

/// Calls the current sim seam cannot serve, each with the reason.
fn seams(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    // Shield and subsystem damage edits (SIM-55): SetShieldPercent takes a
    // fraction of the maximum (as the Roots missions write `shields = 1.0`),
    // SetMaxShields the new maximum, Damage<System> points of damage.
    table.set("SetShieldPercent", float_edit(lua, ctx, "SetShieldPercent", "A2.SetShieldPercent", Edit::ShieldPercent)?)?;
    table.set("SetMaxShields", float_edit(lua, ctx, "SetMaxShields", "A2.SetMaxShields", Edit::MaxShields)?)?;
    table.set("AllowAsTarget", no_data_fn(lua, ctx, "AllowAsTarget", "the native flag is a per-team mask; Edit::Targetable has no team and the sim rejects it")?)?;
    table.set("DamageEngines", float_edit(lua, ctx, "DamageEngines", "A2.DamageEngines", |id, v| Edit::SubsystemDamage(id, Subsystem::Engines, v))?)?;
    table.set("DamageSensors", float_edit(lua, ctx, "DamageSensors", "A2.DamageSensors", |id, v| Edit::SubsystemDamage(id, Subsystem::Sensors, v))?)?;
    table.set("DamageLifeSupport", float_edit(lua, ctx, "DamageLifeSupport", "A2.DamageLifeSupport", |id, v| Edit::SubsystemDamage(id, Subsystem::LifeSupport, v))?)?;
    table.set("DamageShieldGenerator", float_edit(lua, ctx, "DamageShieldGenerator", "A2.DamageShieldGenerator", |id, v| Edit::SubsystemDamage(id, Subsystem::Shields, v))?)?;
    table.set("DamageWeapons", float_edit(lua, ctx, "DamageWeapons", "A2.DamageWeapons", |id, v| Edit::SubsystemDamage(id, Subsystem::Weapons, v))?)?;
    table.set("HideObject", no_data_fn(lua, ctx, "HideObject", "the world view and sim have no hidden-object state")?)?;
    table.set("ButItCanBeDerelicted", no_data_fn(lua, ctx, "ButItCanBeDerelicted", "the sim has no derelict state")?)?;
    table.set("CraftCanRecrew", no_data_fn(lua, ctx, "CraftCanRecrew", "the sim has no per-craft recrew switch")?)?;
    table.set("SetName", no_data_fn(lua, ctx, "SetName", "the sim and view carry no per-object display name")?)?;
    table.set("SetPlanetCivPop", no_data_fn(lua, ctx, "SetPlanetCivPop", "the sim has no planet population edit")?)?;
    table.set("StuffIntoWormhole", no_data_fn(lua, ctx, "StuffIntoWormhole", "the sim has no wormhole transport edit")?)?;
    table.set("LinkWormholes", no_data_fn(lua, ctx, "LinkWormholes", "the sim has no wormhole link edit")?)?;
    table.set("SetWormholeDeleteOnTransport", no_data_fn(lua, ctx, "SetWormholeDeleteOnTransport", "the sim has no wormhole transport rules")?)?;
    table.set("LoadCargoShip", no_data_fn(lua, ctx, "LoadCargoShip", "the world view carries no cargo state")?)?;
    table.set("GetNearestShipyard", no_data_fn(lua, ctx, "GetNearestShipyard", "the search's team-bit test and shipyard list are not measured")?)?;
    table.set("CreateNewSiegeFleet", no_data_fn(lua, ctx, "CreateNewSiegeFleet", "no fleet objects exist in the mission API")?)?;
    Ok(())
}

/// Wrap every function of the `A2` table so a resolved `BuildObject` token
/// reaches it as the real entity id. Call once, after all modules installed.
pub(super) fn finish(lua: &Lua, table: &Table) -> mlua::Result<()> {
    let Some(st) = lua.app_data_ref::<St>().map(|s| s.clone()) else { return Ok(()) };
    let entries: Vec<(String, Function)> = table
        .pairs::<String, Value>()
        .filter_map(Result::ok)
        .filter_map(|(k, v)| match v {
            Value::Function(f) => Some((k, f)),
            _ => None,
        })
        .collect();
    for (name, inner) in entries {
        let st = st.clone();
        table.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                let args: MultiValue = args.into_iter().map(|v| st.borrow().translate(v)).collect();
                inner.call::<MultiValue>(args)
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "dll_objects/tests.rs"]
mod tests;
