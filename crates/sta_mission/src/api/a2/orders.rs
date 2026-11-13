//! DLL-named order, AI and team calls of the Armada II `ScriptInterface`
//! (LUA-05): `Goto Stop Attack AI_Attack Mine SetImpulseSpeed ...` (`BuildObject` lives in `dll_objects`).
//!
//! Rules, each from the executable's own `ScriptInterfaceImp` bodies (read from
//! the executable, static) unless marked otherwise:
//!
//! - A call whose unit handle is absent or dying does nothing (the engine
//!   looks the handle up and returns). Here too: no command, no error. The
//!   exception is a `BuildObject` handle that is still pending or whose spawn
//!   failed (the engine would have a real object, or 0): that raises and is
//!   counted, so a script that orders a unit in the tick it built it is seen. An
//!   argument of the wrong shape is a Lua error before anything is queued.
//! - `Goto` is the engine's GO command (GO_WARP when the bool is true) with a
//!   path or an object; the trailing `int` is never read. `Stop`, `Attack`,
//!   `AI_Attack(unit, object)` are STOP / ATTACK orders; the extra `int` of
//!   `Stop` and `Attack` is never read. `Mine` is MINE with an object, or
//!   without a target when the object is absent (nearest node, see `spawn.rs`).
//!   They are sent as the unit's owner like a player's input.
//! - `SetImpulseSpeed` / `ResetImpulseSpeed` set and lift the unit's impulse
//!   speed, `SetRedAlert` / `SetGreenAlert` set its alert status (red 0, yellow
//!   1, green 2): typed SCRIPT edits (`Edit::MaxSpeed`, `Edit::Alert`) whose
//!   rejection by the sim is reported and counted (`edit_feedback`).
//! - `GetNearestEnemy(unit, r)`: nearest live unit whose owner is an enemy of
//!   the unit's owner, inside the square of half side `r` around the unit,
//!   measured in the XZ plane, and whose owner can see the unit; 0 when none.
//!   Not measured: the engine's grid rounding of the square, tie order, and
//!   whether its `isEnemy` is exactly the view's `Relation::Enemy`.
//! - Paths come from `A2.paths[name]`, the table the host fills from the map
//!   (points 1-based, `{x=,y=,z=}`), as for `GetDistance`.
//!
//! Calls with no sim seam or no data raise a Lua error and add
//! `A2.<Name>: <why>` to `unsupported()`; they never return a guessed value.
//! That list is this package's missing-seam report (README.md).
//!
//! Evidence grades are `read from the executable, static` unless a line says
//! `not measured`. Argument constants of the stock scripts: measured with the
//! inventory's call decoder; findings are kept outside the repository.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table, Value};
use sta_core::{EntityId, Fixed, PlayerId, Vec3};
use sta_sim::command::{AlertState, Edit, Order};
use sta_sim::view::{Relation, UnitView};

use super::dll_objects;
use crate::api::entity_ref::EntityRef;
use crate::api::search::stem;
use crate::api::{Shared, edit_feedback, pos_f64, spawn, to_vec3, vector};

/// `AiCommand` values (read from the executable) the `SetCommand*VO` calls use.
const CMD_STOP: i64 = 3;
const CMD_GO: i64 = 4;
const CMD_ATTACK: i64 = 6;
const CMD_MINE: i64 = 21;
const CMD_BUILD: i64 = 25;
/// Highest `AiCommand` value.
const CMD_LAST: i64 = 56;

#[derive(Default)]
struct State {
    /// The impulse speed this runtime last set per unit.
    speed: BTreeMap<EntityId, Fixed>,
}

#[derive(Clone)]
struct Env {
    ctx: Shared,
    st: Rc<RefCell<State>>,
}

/// One validated argument.
enum P {
    Handle(EntityId),
    Int(i64),
    Num(f64),
    Bool(bool),
    Str(String),
    Vec((f64, f64, f64)),
}

impl P {
    fn id(&self) -> EntityId {
        match self {
            P::Handle(id) => *id,
            _ => EntityId::NONE,
        }
    }
    fn int(&self) -> i64 {
        match self {
            P::Int(n) => *n,
            _ => 0,
        }
    }
    fn num(&self) -> f64 {
        match self {
            P::Num(n) => *n,
            P::Int(n) => *n as f64,
            _ => 0.0,
        }
    }
    fn boolean(&self) -> bool {
        matches!(self, P::Bool(true))
    }
    fn text(&self) -> &str {
        match self {
            P::Str(s) => s,
            _ => "",
        }
    }
    fn vec(&self) -> (f64, f64, f64) {
        match self {
            P::Vec(v) => *v,
            _ => (0.0, 0.0, 0.0),
        }
    }
}

fn whole(v: &Value) -> Option<i64> {
    match v {
        Value::Integer(n) => Some(*n),
        Value::Number(n) if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.0e15 => Some(*n as i64),
        _ => None,
    }
}

fn finite(v: &Value) -> Option<f64> {
    match v {
        Value::Integer(n) => Some(*n as f64),
        Value::Number(n) if n.is_finite() => Some(*n),
        _ => None,
    }
}

fn bad(name: &str, index: usize, want: &str) -> mlua::Error {
    mlua::Error::runtime(format!("A2.{name} argument {} must be {want}", index + 1))
}

fn parse_one(name: &str, index: usize, kind: char, v: &Value) -> mlua::Result<P> {
    match kind {
        'h' => whole(v)
            .and_then(|n| u32::try_from(n).ok())
            .map(|n| P::Handle(EntityId(n)))
            .ok_or_else(|| bad(name, index, "an unsigned integer handle")),
        'i' => whole(v)
            .filter(|n| i32::try_from(*n).is_ok())
            .map(P::Int)
            .ok_or_else(|| bad(name, index, "a signed 32-bit integer")),
        'f' => finite(v).map(P::Num).ok_or_else(|| bad(name, index, "a finite number")),
        'b' => match v {
            Value::Boolean(b) => Ok(P::Bool(*b)),
            _ => Err(bad(name, index, "a boolean")),
        },
        's' => match v {
            Value::String(s) => {
                let s = s.to_str().map_err(|_| bad(name, index, "a UTF-8 string"))?;
                if s.contains('\0') {
                    return Err(bad(name, index, "a string without NUL"));
                }
                Ok(P::Str(s.to_owned()))
            }
            _ => Err(bad(name, index, "a string")),
        },
        'v' => match v {
            Value::Table(t) => {
                let get = |k: &str| t.raw_get::<Value>(k).ok().and_then(|v| finite(&v));
                match (get("x"), get("y"), get("z")) {
                    (Some(x), Some(y), Some(z)) => Ok(P::Vec((x, y, z))),
                    _ => Err(bad(name, index, "a Vector with finite x, y and z")),
                }
            }
            _ => Err(bad(name, index, "a Vector")),
        },
        other => unreachable!("argument kind {other}"),
    }
}

/// Parse `args` against the first overload (a string of kind letters) that accepts them.
fn parse(name: &str, specs: &[&str], args: &MultiValue) -> mlua::Result<(usize, Vec<P>)> {
    let mut first_error = None;
    for (n, spec) in specs.iter().enumerate() {
        if spec.len() != args.len() {
            continue;
        }
        let parsed: mlua::Result<Vec<P>> =
            spec.chars().zip(args.iter()).enumerate().map(|(i, (k, v))| parse_one(name, i, k, v)).collect();
        match parsed {
            Ok(values) => return Ok((n, values)),
            Err(e) => first_error = first_error.or(Some(e)),
        }
    }
    Err(first_error.unwrap_or_else(|| {
        let shapes: Vec<String> = specs.iter().map(|s| s.len().to_string()).collect();
        mlua::Error::runtime(format!("A2.{name} requires exactly {} argument(s)", shapes.join(" or ")))
    }))
}

impl Env {
    /// A call with no sim seam or data: counted, and an error.
    fn gap(&self, name: &str, why: &str) -> mlua::Error {
        self.ctx.borrow_mut().unsupported(&format!("A2.{name}: {why}"));
        mlua::Error::runtime(format!("A2.{name} is not available: {why}"))
    }

    /// The live unit, or `None` when the handle is absent or dying.
    fn alive(&self, id: EntityId) -> Option<UnitView> {
        self.ctx.borrow().alive(id).cloned()
    }

    /// Send an order as the unit's owner.
    fn order(&self, name: &str, id: EntityId, make: impl FnOnce(Vec<EntityId>) -> Order) -> mlua::Result<()> {
        if self.ctx.borrow_mut().order(id, make) {
            Ok(())
        } else {
            Err(mlua::Error::runtime(format!(
                "A2.{name}: object {} has no player owner or the order could not be queued",
                id.0
            )))
        }
    }

    fn owned(&self, name: &str, unit: &UnitView) -> mlua::Result<()> {
        if unit.owner.is_player() {
            Ok(())
        } else {
            Err(mlua::Error::runtime(format!("A2.{name}: object {} has no player owner", unit.id.0)))
        }
    }

    fn team(&self, name: &str, number: i64) -> mlua::Result<PlayerId> {
        let c = self.ctx.borrow();
        u8::try_from(number)
            .ok()
            .and_then(|n| c.player_numbering.decode(n))
            .filter(|p| p.is_player() || *p == sta_sim::world::NEUTRAL)
            .ok_or_else(|| mlua::Error::runtime(format!("A2.{name} requires a valid team number")))
    }

    /// The points of `A2.paths[path]`; a missing path is a host gap.
    fn path(&self, lua: &Lua, name: &str, path: &str) -> mlua::Result<Vec<Vec3>> {
        let missing = |why: String| self.gap(name, &why);
        let a2: Table = lua.globals().raw_get("A2")?;
        let Value::Table(paths) = a2.raw_get::<Value>("paths")? else {
            return Err(missing("A2.paths is missing".into()));
        };
        let points = match dll_objects::lookup_path(&paths, path)? {
            Ok(Some(t)) => t,
            Ok(None) => return Err(missing(format!("path '{path}' is not in A2.paths (the host fills it from the map)"))),
            Err(why) => return Err(missing(why)),
        };
        let mut out = Vec::new();
        for entry in points.sequence_values::<Table>() {
            let p = entry?;
            let get = |k: &str| p.raw_get::<f64>(k).ok().filter(|v| v.is_finite());
            match (get("x"), get("y"), get("z")) {
                (Some(x), Some(y), Some(z)) => out.push(to_vec3(x, y, z)),
                _ => return Err(missing(format!("path '{path}' has a malformed point"))),
            }
        }
        if out.is_empty() {
            return Err(missing(format!("path '{path}' has no points")));
        }
        Ok(out)
    }
}

/// Bind one DLL-named function.
fn def(
    lua: &Lua,
    env: &Env,
    name: &'static str,
    specs: &'static [&'static str],
    f: impl Fn(&Lua, &Env, usize, &[P]) -> mlua::Result<Value> + 'static,
) -> mlua::Result<Function> {
    let env = env.clone();
    lua.create_function(move |lua, args: MultiValue| {
        let (overload, values) = parse(name, specs, &args)?;
        // A `BuildObject` token that is still pending or whose spawn failed
        // names no live object yet, unlike an absent real handle (a silent
        // no-op): the script has to wait a tick, so raise and count.
        for v in &values {
            if let P::Handle(id) = v
                && let Some(why) = dll_objects::token_problem(lua, id.0)
            {
                env.ctx.borrow_mut().unsupported(&format!("A2.{name}: {why}"));
                return Err(mlua::Error::runtime(format!("A2.{name}: {why} (handle {})", id.0)));
            }
        }
        f(lua, &env, overload, &values)
    })
}

/// A bound call with no seam: arguments are validated, then it raises and is counted.
fn raising(
    lua: &Lua,
    env: &Env,
    name: &'static str,
    specs: &'static [&'static str],
    why: &'static str,
) -> mlua::Result<Function> {
    def(lua, env, name, specs, move |_, e, _, _| Err(e.gap(name, why)))
}

fn nothing() -> mlua::Result<Value> {
    Ok(Value::Nil)
}

/// `Goto` and `Order::Move`: along the path's points in order, or to the object.
fn goto(lua: &Lua, env: &Env, overload: usize, a: &[P]) -> mlua::Result<Value> {
    let Some(unit) = env.alive(a[0].id()) else { return nothing() };
    let dest = if overload == 0 {
        None
    } else {
        // The engine ignores an absent or dying destination object too.
        let Some(target) = env.alive(a[1].id()) else { return nothing() };
        Some(target.pos)
    };
    let points = match dest {
        Some(p) => vec![p],
        None => env.path(lua, "Goto", a[1].text())?,
    };
    if a[2].boolean() {
        return Err(env.gap("Goto", "no warp order in the sim seam (GO_WARP)"));
    }
    env.owned("Goto", &unit)?;
    for (n, to) in points.into_iter().enumerate() {
        env.order("Goto", unit.id, |units| Order::Move { units, to, queue: n > 0 })?;
    }
    nothing()
}

/// `Attack(unit, object)` and `AI_Attack(unit, object)`: ATTACK with a target.
fn attack(env: &Env, name: &str, unit: EntityId, target: EntityId) -> mlua::Result<Value> {
    let (Some(unit), Some(target)) = (env.alive(unit), env.alive(target)) else { return nothing() };
    env.owned(name, &unit)?;
    env.order(name, unit.id, |units| Order::Attack { units, target: target.id })?;
    nothing()
}

fn mine(env: &Env, unit: EntityId, node: EntityId) -> mlua::Result<Value> {
    let Some(unit) = env.alive(unit) else { return nothing() };
    env.owned("Mine", &unit)?;
    // An absent object means MINE without a target: the nearest node.
    let node = match env.alive(node) {
        Some(n) => Some(n.id),
        None => spawn::default_mine_node(&env.ctx.borrow(), &EntityRef::Existing(unit.id)).and_then(|n| n.id()),
    };
    let node = node.ok_or_else(|| env.gap("Mine", "no mineable node in the view for a Mine without a target"))?;
    env.order("Mine", unit.id, |units| Order::Mine { units, node })?;
    nothing()
}

fn alert(lua: &Lua, env: &Env, name: &'static str, state: AlertState) -> mlua::Result<Function> {
    def(lua, env, name, &["h"], move |_, e, _, a| {
        if e.alive(a[0].id()).is_some() {
            edit_feedback::queue(&mut e.ctx.borrow_mut(), name_label(name), Edit::Alert(a[0].id(), state))?;
        }
        nothing()
    })
}

/// The label an edit's rejection is counted under.
fn name_label(name: &str) -> &'static str {
    match name {
        "SetRedAlert" => "A2.SetRedAlert",
        "SetGreenAlert" => "A2.SetGreenAlert",
        "SetImpulseSpeed" => "A2.SetImpulseSpeed",
        _ => "A2.ResetImpulseSpeed",
    }
}

/// The point on the first unit's bounding sphere facing the second, or one
/// radius along +x when there is no second unit.
fn attack_location(env: &Env, lua: &Lua, a: &[P]) -> mlua::Result<Value> {
    let name = "GetAttackLocation";
    let Some(from) = env.alive(a[0].id()) else {
        return Err(env.gap(name, "the first object is absent (the engine returns a global constant vector)"));
    };
    let radius = {
        let c = env.ctx.borrow();
        c.def(&from).map(|d| d.radius.to_f32() as f64).filter(|r| *r > 0.0)
    };
    let Some(radius) = radius else {
        return Err(env.gap(name, "the unit's bounding radius is not in the content (SIM-21 `radius` is zero)"));
    };
    let p = pos_f64(from.pos);
    let out = match env.alive(a[1].id()) {
        None => (p.0 + radius, p.1, p.2),
        Some(to) => {
            let q = pos_f64(to.pos);
            let d = (q.0 - p.0, q.1 - p.1, q.2 - p.2);
            let len = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt();
            if len == 0.0 {
                return Err(env.gap(name, "the two objects coincide (the engine divides by zero)"));
            }
            (p.0 + d.0 / len * radius, p.1 + d.1 / len * radius, p.2 + d.2 / len * radius)
        }
    };
    Ok(Value::Table(vector(lua, out)?))
}

fn nearest_enemy(env: &Env, a: &[P]) -> mlua::Result<Value> {
    let Some(me) = env.alive(a[0].id()) else { return Ok(Value::Integer(0)) };
    let r = a[1].num();
    let c = env.ctx.borrow();
    let at = pos_f64(me.pos);
    let mut best: Option<(f64, EntityId)> = None;
    // Owners that can see the unit, computed once per owner.
    let mut sees: BTreeMap<PlayerId, bool> = BTreeMap::new();
    for u in &c.view.units {
        if u.id == me.id || c.alive(u.id).is_none() {
            continue;
        }
        let p = pos_f64(u.pos);
        if (p.0 - at.0).abs() > r || (p.2 - at.2).abs() > r {
            continue;
        }
        if c.relation(me.owner, u.owner) != Relation::Enemy {
            continue;
        }
        let seen = *sees
            .entry(u.owner)
            .or_insert_with(|| c.view.visible_to(u.owner).iter().any(|v| v.id == me.id));
        if !seen {
            continue;
        }
        let d = (p.0 - at.0) * (p.0 - at.0) + (p.2 - at.2) * (p.2 - at.2);
        if best.is_none_or(|(b, id)| d < b || (d == b && u.id < id)) {
            best = Some((d, u.id));
        }
    }
    Ok(Value::Integer(best.map_or(0, |(_, id)| i64::from(id.0))))
}

/// The commands the `SetCommandWhat...VO` calls can map onto the sim's orders.
fn command_name(what: i64) -> String {
    match what {
        CMD_STOP => "STOP".into(),
        CMD_GO => "GO".into(),
        CMD_ATTACK => "ATTACK".into(),
        CMD_MINE => "MINE".into(),
        CMD_BUILD => "BUILD".into(),
        n => format!("command {n}"),
    }
}

fn command_arg(name: &str, what: &P) -> mlua::Result<i64> {
    let n = what.int();
    if (0..=CMD_LAST).contains(&n) {
        Ok(n)
    } else {
        Err(mlua::Error::runtime(format!("A2.{name} argument 2 must be an AiCommand value 0..={CMD_LAST}")))
    }
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    let env = Env { ctx: ctx.clone(), st: Rc::new(RefCell::new(State::default())) };

    // Movement and attack orders.
    table.set("Goto", def(lua, &env, "Goto", &["hsbi", "hhbi"], goto)?)?;
    table.set("Stop", def(lua, &env, "Stop", &["hi"], |_, e, _, a| {
        let Some(unit) = e.alive(a[0].id()) else { return nothing() };
        e.owned("Stop", &unit)?;
        e.order("Stop", unit.id, |units| Order::Stop { units })?;
        nothing()
    })?)?;
    table.set("Attack", def(lua, &env, "Attack", &["hhi"], |_, e, _, a| attack(e, "Attack", a[0].id(), a[1].id()))?)?;
    table.set("AI_Attack", def(lua, &env, "AI_Attack", &["hh", "h", "hv"], |_, e, ov, a| match ov {
        0 => attack(e, "AI_Attack", a[0].id(), a[1].id()),
        // ATTACK without a target, and attack-move to a point: no order in the seam.
        1 => Err(e.gap("AI_Attack", "no target-less attack (seek) order in the sim seam")),
        _ => Err(e.gap("AI_Attack", "no attack-move order in the sim seam")),
    })?)?;
    table.set("Mine", def(lua, &env, "Mine", &["hh"], |_, e, _, a| mine(e, a[0].id(), a[1].id()))?)?;
    table.set("Patrol", raising(lua, &env, "Patrol", &["hs"], "no patrol (looping path) order in the sim seam")?)?;
    table.set("SearchAndDestroy", raising(lua, &env, "SearchAndDestroy", &["h"], "no search-and-destroy order in the sim seam")?)?;
    table.set("Scout", raising(lua, &env, "Scout", &["h"], "no scout order in the sim seam")?)?;
    table.set("Colonize", raising(lua, &env, "Colonize", &["hh"], "no colonize order in the sim seam")?)?;
    table.set("Special_Attack", raising(lua, &env, "Special_Attack", &["hs", "hhs"], "the sim orders only cloak and point-defence specials, and its special order carries no target")?)?;
    table.set("SetCommandMoveAutonomyLow", raising(lua, &env, "SetCommandMoveAutonomyLow", &["h"], "no per-unit movement autonomy order in the sim seam")?)?;
    table.set("PointAt", raising(lua, &env, "PointAt", &["hs", "hh"], "no facing (orientation) edit in the sim seam")?)?;

    // Speed and alert status: typed edits.
    table.set("SetImpulseSpeed", def(lua, &env, "SetImpulseSpeed", &["hf"], |_, e, _, a| {
        if a[1].num() < 0.0 {
            return Err(mlua::Error::runtime("A2.SetImpulseSpeed requires a non-negative speed"));
        }
        let id = a[0].id();
        if e.alive(id).is_some() {
            let speed = Fixed::from_f64(a[1].num());
            edit_feedback::queue(&mut e.ctx.borrow_mut(), name_label("SetImpulseSpeed"), Edit::MaxSpeed(id, Some(speed)))?;
            e.st.borrow_mut().speed.insert(id, speed);
        }
        nothing()
    })?)?;
    table.set("ResetImpulseSpeed", def(lua, &env, "ResetImpulseSpeed", &["h"], |_, e, _, a| {
        let id = a[0].id();
        if e.alive(id).is_some() {
            edit_feedback::queue(&mut e.ctx.borrow_mut(), name_label("ResetImpulseSpeed"), Edit::MaxSpeed(id, None))?;
            e.st.borrow_mut().speed.remove(&id);
        }
        nothing()
    })?)?;
    // The value this runtime last set, else the class impulse speed; 0 for an
    // absent unit, as the engine does. Not measured: the engine's class value
    // is looked up by an `int` (read as the unit's upgrade level).
    table.set("GetImpulseSpeed", def(lua, &env, "GetImpulseSpeed", &["h"], |_, e, _, a| {
        let Some(unit) = e.alive(a[0].id()) else { return Ok(Value::Number(0.0)) };
        if let Some(speed) = e.st.borrow().speed.get(&unit.id) {
            return Ok(Value::Number(f64::from(speed.to_f32())));
        }
        let c = e.ctx.borrow();
        match c.def(&unit) {
            Some(d) => Ok(Value::Number(f64::from(d.movement.max_speed.to_f32()))),
            None => Err(e.gap("GetImpulseSpeed", "the unit has no content definition")),
        }
    })?)?;
    table.set("SetRedAlert", alert(lua, &env, "SetRedAlert", AlertState::Red)?)?;
    table.set("SetGreenAlert", alert(lua, &env, "SetGreenAlert", AlertState::Green)?)?;

    // Queries.
    table.set("GetNearestEnemy", def(lua, &env, "GetNearestEnemy", &["hf"], |_, e, _, a| nearest_enemy(e, a))?)?;
    table.set("GetAttackLocation", def(lua, &env, "GetAttackLocation", &["hh"], |lua, e, _, a| attack_location(e, lua, a))?)?;
    table.set("GetMainEnemyBaseLocation", raising(lua, &env, "GetMainEnemyBaseLocation", &["i"], "the AI team's strategic map (its main enemy base) is not in the world view")?)?;
    table.set("AttackMainEnemyBase", raising(lua, &env, "AttackMainEnemyBase", &["hb"], "the AI team's strategic map (its main enemy base) is not in the world view")?)?;
    table.set("GetAllyStationAttacks", raising(lua, &env, "GetAllyStationAttacks", &["ii"], "the AI team's attack statistics are not in the world view")?)?;

    // AI scripts: the request is kept for the host, nothing runs it.
    table.set("AI_Team_Load_AIP", def(lua, &env, "AI_Team_Load_AIP", &["is"], |_, e, _, a| {
        let team = e.team("AI_Team_Load_AIP", a[0].int())?;
        e.ctx.borrow_mut().loaded_aips.push((team, a[1].text().to_owned()));
        Err(e.gap("AI_Team_Load_AIP", "no AI script (AIP) runs in the simulation"))
    })?)?;

    // Unit state with no edit in the seam.
    // SIM-54: the unit's crew, clamped by the sim to 0..its maximum crew.
    table.set("SetCrew", def(lua, &env, "SetCrew", &["hf"], |_, e, _, a| {
        let to = a[1].num();
        if !to.is_finite() {
            return Err(mlua::Error::runtime("A2.SetCrew requires a finite number"));
        }
        if e.alive(a[0].id()).is_some() {
            edit_feedback::queue(&mut e.ctx.borrow_mut(), "A2.SetCrew", Edit::UnitCrew(a[0].id(), Fixed::from_f64(to)))?;
        }
        nothing()
    })?)?;
    table.set("SetRace", raising(lua, &env, "SetRace", &["hs"], "no mutable-race edit in the sim seam")?)?;
    table.set("CraftCanRepair", raising(lua, &env, "CraftCanRepair", &["hb"], "no can-repair flag in the sim seam")?)?;

    // `SetCommandWhat...VO(who, command, ...)`: SET_COMMAND with a target
    // object (both objects must be live craft, else nothing happens), a
    // parameter, or a position. The trailing bool is the voice acknowledgement:
    // not measured, not applied.
    table.set("SetCommandWhatWhoVO", def(lua, &env, "SetCommandWhatWhoVO", &["hihb"], |_, e, _, a| {
        let what = command_arg("SetCommandWhatWhoVO", &a[1])?;
        let (Some(unit), Some(target)) = (e.alive(a[0].id()), e.alive(a[2].id())) else { return nothing() };
        match what {
            CMD_GO => {
                e.owned("SetCommandWhatWhoVO", &unit)?;
                e.order("SetCommandWhatWhoVO", unit.id, |units| Order::Move { units, to: target.pos, queue: false })?;
            }
            CMD_ATTACK => return attack(e, "SetCommandWhatWhoVO", unit.id, target.id),
            CMD_MINE => {
                e.owned("SetCommandWhatWhoVO", &unit)?;
                e.order("SetCommandWhatWhoVO", unit.id, |units| Order::Mine { units, node: target.id })?;
            }
            other => {
                let why = format!("{} toward an object has no order in the sim seam", command_name(other));
                return Err(e.gap("SetCommandWhatWhoVO", &why));
            }
        }
        nothing()
    })?)?;
    table.set("SetCommandWhatWhoParamVO", def(lua, &env, "SetCommandWhatWhoParamVO", &["hihib"], |_, e, _, a| {
        let what = command_arg("SetCommandWhatWhoParamVO", &a[1])?;
        if e.alive(a[0].id()).is_none() || e.alive(a[2].id()).is_none() {
            return nothing();
        }
        let why = format!("{} with a parameter has no order in the sim seam", command_name(what));
        Err(e.gap("SetCommandWhatWhoParamVO", &why))
    })?)?;
    table.set("SetCommandWhatWhoWhereVO", def(lua, &env, "SetCommandWhatWhoWhereVO", &["hisvb"], |_, e, _, a| {
        let what = command_arg("SetCommandWhatWhoWhereVO", &a[1])?;
        let Some(unit) = e.alive(a[0].id()) else { return nothing() };
        if what != CMD_BUILD {
            let why = format!("{} at a position has no order in the sim seam", command_name(what));
            return Err(e.gap("SetCommandWhatWhoWhereVO", &why));
        }
        e.owned("SetCommandWhatWhoWhereVO", &unit)?;
        let key = e
            .ctx
            .borrow()
            .content
            .as_ref()
            .and_then(|db| db.key(stem(a[2].text())))
            .ok_or_else(|| mlua::Error::runtime(format!("A2.SetCommandWhatWhoWhereVO: unknown ODF '{}'", a[2].text())))?;
        let (x, y, z) = a[3].vec();
        e.order("SetCommandWhatWhoWhereVO", unit.id, |builder| Order::Build {
            builder: builder[0],
            what: key,
            at: Some(to_vec3(x, y, z)),
        })?;
        nothing()
    })?)?;

    Ok(())
}

#[cfg(test)]
#[path = "orders/tests.rs"]
mod tests;
