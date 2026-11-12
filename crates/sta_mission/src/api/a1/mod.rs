//! DLL-named Armada I mission calls (LUA-06): the `A1` table.
//!
//! Armada I's missions are compiled DLLs that import engine functions
//! by name from `Armada.exe` (docs/MISSIONS.md, docs/MISSION_DLL_API.md). This
//! module binds those names, with their imported argument shapes, so a
//! converted mission can call `A1.Name(...)`. It mirrors [`super::a2`].
//!
//! Where the operation is the same as the A2 one (object queries, teams, the
//! single-handle switches) the `A2` binding is reused through [`alias`]; the
//! rest is bound here. Each call does one of four things:
//!
//! - a **sim-backed** call reads the latest mission view or queues a command
//!   (an order from the unit's owner, a SCRIPT edit) like the A2 table;
//! - a **presentation** call queues `Edit::Present`: typed where the seam has
//!   a variant, otherwise `Presentation::Call` named `A1.<Name>` with the
//!   arguments in call order (bridge scenes, cineractive shots, titles...);
//! - a **seam gap** write has no simulation seam yet: the request is retained
//!   as a `Presentation::Call`, counted in `unsupported()` and does nothing
//!   else; see `seams.rs` for the list;
//! - a **seam gap** query raises a Lua error naming what is missing.
//!
//! `A1.paths` is the path seam: a table `name -> { {x=,y=,z=}, ... }` the
//! converting host fills from the map (the mission API holds no map data).
//! `A1.GetHandle` reads the existing label map. `BuildObject` returns a
//! pending handle (`TOKEN_BASE + request`) that every `A1` call (and each
//! aliased `A2` call) resolves to the real entity once the spawn receipt is in.
//!
//! Arguments are validated strictly: exact arity per overload, integral
//! handles, finite numbers, strict booleans; a malformed call is a Lua error
//! and queues nothing. What each argument means is taken from the DLL import
//! signature and the constants the stock DLLs pass (evidence grades in README.md), and anything not measured
//! is said so there.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table, Value};
use sta_core::{EntityId, Fixed, PlayerId, Vec3};
use sta_sim::command::{CommandKind, Edit};
use sta_sim::present::{Arg, Presentation};

use super::entity_ref::{SpawnHandle, SpawnStatus};
use super::{Shared, to_vec3};

mod objects;
mod orders;
mod presentation;
mod seams;
mod teams;
#[cfg(test)]
mod tests;

/// Handles at or above this are pending spawns (`BuildObject`): the request
/// number is `handle - TOKEN_BASE`. Real entity ids stay far below it.
pub(super) const TOKEN_BASE: u32 = 0x4000_0000;

/// A special-forces flag (`CreateSpecialFlag`): a named hero tied to a unit.
pub(super) struct Flag {
    pub unit: EntityId,
    pub restrictions: i64,
}

#[derive(Default)]
pub(super) struct State {
    pub pending: BTreeMap<u32, Rc<SpawnHandle>>,
    pub next_title: u32,
    pub next_sound: u32,
    /// Sounds started and not yet reported done or stopped.
    pub sounds_active: BTreeSet<u32>,
    pub movie_playing: bool,
    /// Dilithium a team was last set to this tick, so several writes in one
    /// tick build on each other instead of on the stale view.
    pub dilithium: BTreeMap<PlayerId, (u64, Fixed)>,
    pub groups: BTreeMap<u32, (String, BTreeSet<i64>)>,
    pub next_group: u32,
    pub flags: BTreeMap<u32, Flag>,
    pub next_flag: u32,
}

#[derive(Clone)]
pub(super) struct Env {
    pub ctx: Shared,
    pub st: Rc<RefCell<State>>,
}

/// One validated argument.
pub(super) enum A {
    Handle(EntityId),
    Int(i64),
    Num(f64),
    Bool(bool),
    Str(String),
    Nil,
    Vec(Vec3),
}

impl A {
    pub fn int(&self) -> i64 {
        match self {
            A::Int(n) => *n,
            _ => 0,
        }
    }
    pub fn num(&self) -> f64 {
        match self {
            A::Num(n) => *n,
            A::Int(n) => *n as f64,
            _ => 0.0,
        }
    }
    pub fn boolean(&self) -> bool {
        matches!(self, A::Bool(true))
    }
    pub fn id(&self) -> EntityId {
        match self {
            A::Handle(id) => *id,
            _ => EntityId::NONE,
        }
    }
    pub fn text(&self) -> &str {
        match self {
            A::Str(s) => s,
            _ => "",
        }
    }
    /// The argument as a presentation argument; `None` for an omitted string.
    pub fn arg(&self) -> Option<Arg> {
        Some(match self {
            A::Handle(id) => Arg::Entity(*id),
            A::Int(n) => Arg::Int(*n),
            A::Num(n) => Arg::Num(Fixed::from_f64(*n)),
            A::Bool(b) => Arg::Bool(*b),
            A::Str(s) => Arg::Text(s.clone()),
            A::Vec(v) => Arg::Position(*v),
            A::Nil => return None,
        })
    }
}

pub(super) fn args_of(values: &[A]) -> Vec<Arg> {
    values.iter().filter_map(A::arg).collect()
}

fn whole(v: &Value) -> Option<i64> {
    match v {
        Value::Integer(n) => Some(*n),
        Value::Number(n) if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.0e15 => Some(*n as i64),
        _ => None,
    }
}

fn bad(name: &str, index: usize, want: &str) -> mlua::Error {
    mlua::Error::runtime(format!("A1.{name} argument {} must be {want}", index + 1))
}

impl Env {
    /// The entity a pending-spawn token stands for.
    pub fn token(&self, raw: u32) -> Option<Result<EntityId, &'static str>> {
        if raw < TOKEN_BASE {
            return None;
        }
        let mut st = self.st.borrow_mut();
        let Some(handle) = st.pending.get(&raw).cloned() else {
            return Some(Err("is not a known pending object"));
        };
        Some(match handle.state.get() {
            SpawnStatus::Resolved(id) => Ok(id),
            SpawnStatus::Failed => {
                st.pending.remove(&raw);
                Err("could not be built (the spawn failed)")
            }
            SpawnStatus::Queued | SpawnStatus::Submitted(_) => Err("is still being built; use it after the next tick"),
        })
    }

    fn parse_one(&self, name: &str, index: usize, kind: char, v: &Value) -> mlua::Result<A> {
        let finite = |v: &Value| match v {
            Value::Integer(n) => Some(*n as f64),
            Value::Number(n) if n.is_finite() && n.abs() < 2_147_483_648.0 => Some(*n),
            _ => None,
        };
        match kind {
            'h' => {
                let raw = whole(v)
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| bad(name, index, "an unsigned integer handle"))?;
                match self.token(raw) {
                    None => Ok(A::Handle(EntityId(raw))),
                    Some(Ok(id)) => Ok(A::Handle(id)),
                    Some(Err(why)) => Err(mlua::Error::runtime(format!("A1.{name} argument {}: object {why}", index + 1))),
                }
            }
            // A handle that may name nothing: a pending or failed spawn is "no object".
            'H' => {
                let raw = whole(v)
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| bad(name, index, "an unsigned integer handle"))?;
                Ok(A::Handle(match self.token(raw) {
                    None => EntityId(raw),
                    Some(Ok(id)) => id,
                    Some(Err(_)) => EntityId::NONE,
                }))
            }
            'i' => whole(v)
                .filter(|n| i32::try_from(*n).is_ok())
                .map(A::Int)
                .ok_or_else(|| bad(name, index, "a signed 32-bit integer")),
            'f' => finite(v).map(A::Num).ok_or_else(|| bad(name, index, "a finite number")),
            // A number that keeps its Lua subtype (integer or float).
            'n' => match v {
                Value::Integer(n) if i32::try_from(*n).is_ok() => Ok(A::Int(*n)),
                _ => finite(v).map(A::Num).ok_or_else(|| bad(name, index, "a finite number")),
            },
            'b' => match v {
                Value::Boolean(b) => Ok(A::Bool(*b)),
                _ => Err(bad(name, index, "a boolean")),
            },
            's' | 'S' => match v {
                Value::String(s) => {
                    let s = s.to_str().map_err(|_| bad(name, index, "a UTF-8 string"))?;
                    if s.contains('\0') {
                        return Err(bad(name, index, "a string without NUL"));
                    }
                    Ok(A::Str(s.to_owned()))
                }
                Value::Nil if kind == 'S' => Ok(A::Nil),
                _ => Err(bad(name, index, if kind == 'S' { "a string or nil" } else { "a string" })),
            },
            'v' => match v {
                Value::Table(t) => {
                    let get = |k: &str| t.raw_get::<Value>(k).ok().and_then(|v| finite(&v));
                    match (get("x"), get("y"), get("z")) {
                        (Some(x), Some(y), Some(z)) => Ok(A::Vec(to_vec3(x, y, z))),
                        _ => Err(bad(name, index, "a Vector with finite x, y and z")),
                    }
                }
                _ => Err(bad(name, index, "a Vector")),
            },
            other => unreachable!("argument kind {other}"),
        }
    }

    /// Parse `args` against the first overload that accepts them.
    pub fn parse(&self, name: &str, specs: &[&str], args: &MultiValue) -> mlua::Result<(usize, Vec<A>)> {
        let mut first_error = None;
        for (n, spec) in specs.iter().enumerate() {
            if spec.len() != args.len() {
                continue;
            }
            let parsed: mlua::Result<Vec<A>> = spec
                .chars()
                .zip(args.iter())
                .enumerate()
                .map(|(i, (k, v))| self.parse_one(name, i, k, v))
                .collect();
            match parsed {
                Ok(values) => return Ok((n, values)),
                Err(e) => first_error = first_error.or(Some(e)),
            }
        }
        Err(first_error.unwrap_or_else(|| {
            let shapes: Vec<String> = specs.iter().map(|s| s.len().to_string()).collect();
            mlua::Error::runtime(format!("A1.{name} requires exactly {} argument(s)", shapes.join(" or ")))
        }))
    }

    /// A live (present, not dying) unit, cloned from the view.
    pub fn unit(&self, name: &str, id: EntityId) -> mlua::Result<sta_sim::view::UnitView> {
        self.ctx
            .borrow()
            .alive(id)
            .cloned()
            .ok_or_else(|| mlua::Error::runtime(format!("A1.{name}: object {} is not alive", id.0)))
    }

    pub fn team(&self, name: &str, number: i64) -> mlua::Result<PlayerId> {
        let c = self.ctx.borrow();
        u8::try_from(number)
            .ok()
            .and_then(|n| c.player_numbering.decode(n))
            .filter(|p| p.is_player() || *p == sta_sim::world::NEUTRAL)
            .ok_or_else(|| mlua::Error::runtime(format!("A1.{name} requires a valid team number")))
    }

    /// A team filter where -1 means any team.
    pub fn team_filter(&self, name: &str, number: i64) -> mlua::Result<Option<PlayerId>> {
        if number == -1 { Ok(None) } else { self.team(name, number).map(Some) }
    }

    /// Queue `Presentation::Call` named `A1.<name>`.
    pub fn call(&self, name: &str, args: Vec<Arg>) -> mlua::Result<()> {
        self.present(name, Presentation::Call { name: format!("A1.{name}"), args })
    }

    pub fn present(&self, name: &str, p: Presentation) -> mlua::Result<()> {
        self.ctx
            .borrow_mut()
            .try_push(PlayerId::SCRIPT, CommandKind::Edit(Edit::Present(p)))
            .map(|_| ())
            .map_err(|reason| mlua::Error::runtime(format!("A1.{name}: {reason}")))
    }

    pub fn edit(&self, name: &str, e: Edit) -> mlua::Result<()> {
        self.ctx
            .borrow_mut()
            .try_push(PlayerId::SCRIPT, CommandKind::Edit(e))
            .map(|_| ())
            .map_err(|reason| mlua::Error::runtime(format!("A1.{name}: {reason}")))
    }

    /// A write with no simulation seam: retained as a call, counted, not applied.
    pub fn gap(&self, name: &str, why: &str, args: Vec<Arg>) -> mlua::Result<()> {
        self.call(name, args)?;
        self.ctx.borrow_mut().unsupported(&format!("A1.{name}: {why}"));
        Ok(())
    }

    /// A query with no data behind it.
    pub fn no_data(&self, name: &str, why: &str) -> mlua::Error {
        self.ctx.borrow_mut().unsupported(&format!("A1.{name}: {why}"));
        mlua::Error::runtime(format!("A1.{name} is not available: {why}"))
    }

    /// The points of `A1.paths[path]`.
    pub fn path(&self, lua: &Lua, name: &str, path: &str) -> mlua::Result<Vec<Vec3>> {
        let missing = || {
            self.no_data(name, &format!("path '{path}' is not in A1.paths (the host fills it from the map)"))
        };
        let a1: Table = lua.globals().raw_get("A1")?;
        let Value::Table(paths) = a1.raw_get::<Value>("paths")? else { return Err(missing()) };
        let Value::Table(points) = paths.raw_get::<Value>(path)? else { return Err(missing()) };
        let mut out = Vec::new();
        for entry in points.sequence_values::<Table>() {
            let p = entry?;
            let get = |k: &str| p.raw_get::<f64>(k).ok().filter(|v| v.is_finite());
            match (get("x"), get("y"), get("z")) {
                (Some(x), Some(y), Some(z)) => out.push(to_vec3(x, y, z)),
                _ => return Err(mlua::Error::runtime(format!("A1.{name}: path '{path}' has a malformed point"))),
            }
        }
        if out.is_empty() {
            return Err(missing());
        }
        Ok(out)
    }

    pub fn path_point(&self, lua: &Lua, name: &str, path: &str, index: i64) -> mlua::Result<Vec3> {
        let points = self.path(lua, name, path)?;
        usize::try_from(index)
            .ok()
            .and_then(|i| points.get(i).copied())
            .ok_or_else(|| mlua::Error::runtime(format!("A1.{name}: path '{path}' has no point {index}")))
    }
}

/// Bind one DLL-named function.
pub(super) fn def(
    lua: &Lua,
    env: &Env,
    name: &'static str,
    specs: &'static [&'static str],
    f: impl Fn(&Lua, &Env, usize, &[A]) -> mlua::Result<Value> + 'static,
) -> mlua::Result<Function> {
    let env = env.clone();
    lua.create_function(move |lua, args: MultiValue| {
        let (overload, values) = env.parse(name, specs, &args)?;
        f(lua, &env, overload, &values)
    })
}

/// The `A2` function of the same operation, under the `A1` name: pending
/// handles are resolved first and errors name `A1`.
fn alias(lua: &Lua, env: &Env, a2: &Table, name: &'static str) -> mlua::Result<Function> {
    alias_as(lua, env, a2, name, name)
}

/// [`alias`] under another A1 name (errors are reported under it).
fn alias_as(lua: &Lua, env: &Env, a2: &Table, name: &'static str, a2_name: &'static str) -> mlua::Result<Function> {
    // Existence queries treat a pending or failed spawn as "no such object".
    let lenient = matches!(name, "Exists" | "DoesNotExist" | "IsAlive");
    let f: Function = a2.get(a2_name)?;
    let env = env.clone();
    lua.create_function(move |_, args: MultiValue| {
        let mut mapped = Vec::with_capacity(args.len());
        // Every aliased call takes its object handle first; no other argument is one.
        for (position, v) in args.into_iter().enumerate() {
            let resolved = match &v {
                Value::Integer(n) if position == 0 => u32::try_from(*n).ok().and_then(|raw| env.token(raw)),
                _ => None,
            };
            mapped.push(match resolved {
                Some(Ok(id)) => Value::Integer(i64::from(id.0)),
                Some(Err(_)) if lenient => Value::Integer(0),
                Some(Err(why)) => {
                    return Err(mlua::Error::runtime(format!("A1.{name}: object {why}")));
                }
                None => v,
            });
        }
        f.call::<MultiValue>(MultiValue::from_vec(mapped)).map_err(|e| {
            let e = rename_error(e);
            mlua::Error::runtime(e.to_string().replace(&format!("A1.{a2_name}"), &format!("A1.{name}")))
        })
    })
}

/// An `A2` binding's error, reported under the `A1` name.
fn rename_error(e: mlua::Error) -> mlua::Error {
    let mut e = &e;
    while let mlua::Error::CallbackError { cause, .. } = e {
        e = cause;
    }
    let message = match e {
        mlua::Error::RuntimeError(m) => m.clone(),
        other => other.to_string(),
    };
    mlua::Error::runtime(message.replace("A2.", "A1."))
}

pub(super) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    let env = Env { ctx: ctx.clone(), st: Rc::new(RefCell::new(State::default())) };
    let a2: Table = lua.globals().raw_get("A2")?;
    let table = lua.create_table()?;
    table.set("paths", lua.create_table()?)?;
    objects::install(lua, &env, &a2, &table)?;
    orders::install(lua, &env, &table)?;
    teams::install(lua, &env, &a2, &table)?;
    presentation::install(lua, &env, &a2, &table)?;
    seams::install(lua, &env, &table)?;
    lua.globals().raw_set("A1", table)?;
    Ok(())
}
