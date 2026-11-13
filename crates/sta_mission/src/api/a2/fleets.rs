//! Script fleets: `CreateNewSiegeFleet` / `CreateNewMoveFleet` / `CreateNewAttackFleet`
//! and their methods `AddShip`, `SetFleetDestination`, `TryToWarp`,
//! `SetStayInFormationWhileMoving`, `SetFormation` (LUA-05).
//!
//! What the stock scripts do with a fleet (read from the 36 converted scripts):
//! make it, add ships (handles, often fresh `BuildObject` tokens), set a destination,
//! and for a move fleet a warp flag, a formation and a stay-in-formation flag. They
//! never read a fleet back.
//!
//! Rules (read from `Armada2.exe`, static; not measured):
//!
//! - A fleet is a host-side group of ship handles and a destination; the engine's
//!   own fleets are `AIFleet` objects with a stage machine driven every tick (read
//!   from `Armada2.exe`, static). STA64 has no fleet in the sim, so a fleet gives
//!   each member the scripted order its stage ends in, through the same sim orders
//!   `Goto` / `AI_Attack` use (`Order::Move`, `Order::MoveWarp`, `Order::AttackPoint`):
//!   a move fleet moves to the destination (GO_WARP, as `TryToWarp` defaults to true;
//!   `TryToWarp(false)` gives a plain GO), a siege or attack fleet flies to weapon range
//!   of it (the end state of the siege stages: the engine rallies 1400 short, breaches
//!   enemy starbases within 1800, then attacks and never retreats). A ship added after the destination
//!   was set gets the order at once.
//! - Pending `BuildObject` handles ride through `A2.FleetOrder`, which waits for
//!   the spawn like every other order.
//! - Not modelled: formation keeping and speed matching (`SetFormation`,
//!   `SetStayInFormationWhileMoving` are stored, nothing more), the siege stages
//!   (rally, breach of the defences, retreat), a fleet re-evaluating each tick.

use mlua::{Lua, MultiValue, Table, Value};
use sta_sim::command::Order;

use super::orders::{Env, def, nothing};
use crate::api::to_vec3;

/// `A2.FleetOrder` modes.
const MOVE: i64 = 0;
const MOVE_WARP: i64 = 1;
const APPROACH: i64 = 2;

#[derive(Clone, Copy)]
enum Kind {
    Move,
    Siege,
    Attack,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Move => "MoveFleet",
            Kind::Siege => "SiegeFleet",
            Kind::Attack => "AttackFleet",
        }
    }

    fn mode(self, warp: bool) -> i64 {
        match (self, warp) {
            (Kind::Move, true) => MOVE_WARP,
            (Kind::Move, false) => MOVE,
            _ => APPROACH,
        }
    }
}

fn fleet_error(kind: Kind, method: &str, why: &str) -> mlua::Error {
    mlua::Error::runtime(format!("{}.{method}: {why}", kind.label()))
}

/// Order one member to the fleet's destination.
fn send(lua: &Lua, kind: Kind, fleet: &Table, ship: Value) -> mlua::Result<()> {
    let dest: Value = fleet.raw_get("dest")?;
    if matches!(dest, Value::Nil) {
        return Ok(());
    }
    let warp: bool = fleet.raw_get("warp").unwrap_or(false);
    let a2: Table = lua.globals().raw_get("A2")?;
    let order: mlua::Function = a2.raw_get("FleetOrder")?;
    order.call::<Value>((ship, dest, kind.mode(warp)))?;
    Ok(())
}

fn self_table(kind: Kind, method: &str, args: &MultiValue) -> mlua::Result<Table> {
    match args.front() {
        Some(Value::Table(t)) => Ok(t.clone()),
        _ => Err(fleet_error(kind, method, "called without its fleet")),
    }
}

fn new_fleet(lua: &Lua, kind: Kind) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set("ships", lua.create_table()?)?;
    t.raw_set("warp", true)?;
    t.raw_set("stay_in_formation", true)?;
    let add = lua.create_function(move |lua, args: MultiValue| {
        let me = self_table(kind, "AddShip", &args)?;
        if args.len() != 2 {
            return Err(fleet_error(kind, "AddShip", "requires exactly 1 argument"));
        }
        let ship = args[1].clone();
        if !matches!(ship, Value::Integer(_) | Value::Number(_)) {
            return Err(fleet_error(kind, "AddShip", "requires a handle"));
        }
        let ships: Table = me.raw_get("ships")?;
        // The same handle added twice is one member (the engine's list is a set).
        for v in ships.sequence_values::<Value>() {
            if v? == ship {
                return Ok(());
            }
        }
        ships.raw_push(ship.clone())?;
        send(lua, kind, &me, ship)
    })?;
    t.raw_set("AddShip", add)?;
    let dest = lua.create_function(move |lua, args: MultiValue| {
        let me = self_table(kind, "SetFleetDestination", &args)?;
        if args.len() != 2 {
            return Err(fleet_error(kind, "SetFleetDestination", "requires exactly 1 argument"));
        }
        let Value::Table(v) = &args[1] else {
            return Err(fleet_error(kind, "SetFleetDestination", "requires a Vector"));
        };
        me.raw_set("dest", v.clone())?;
        let ships: Table = me.raw_get("ships")?;
        for ship in ships.sequence_values::<Value>() {
            send(lua, kind, &me, ship?)?;
        }
        Ok(())
    })?;
    t.raw_set("SetFleetDestination", dest)?;
    let flag = |name: &'static str, field: &'static str| {
        lua.create_function(move |_, args: MultiValue| {
            let me = self_table(kind, name, &args)?;
            match args.get(1) {
                Some(Value::Boolean(b)) if args.len() == 2 => me.raw_set(field, *b),
                _ => Err(fleet_error(kind, name, "requires exactly 1 boolean")),
            }
        })
    };
    t.raw_set("TryToWarp", flag("TryToWarp", "warp")?)?;
    t.raw_set("SetStayInFormationWhileMoving", flag("SetStayInFormationWhileMoving", "stay_in_formation")?)?;
    let formation = lua.create_function(move |_, args: MultiValue| {
        let me = self_table(kind, "SetFormation", &args)?;
        match args.get(1) {
            Some(Value::Integer(n)) if args.len() == 2 => me.raw_set("formation", *n),
            _ => Err(fleet_error(kind, "SetFormation", "requires exactly 1 integer")),
        }
    })?;
    t.raw_set("SetFormation", formation)?;
    Ok(t)
}

pub(super) fn install(lua: &Lua, env: &Env, table: &Table) -> mlua::Result<()> {
    for (name, kind) in [
        ("CreateNewSiegeFleet", Kind::Siege),
        ("CreateNewMoveFleet", Kind::Move),
        ("CreateNewAttackFleet", Kind::Attack),
    ] {
        table.set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                if !args.is_empty() {
                    return Err(mlua::Error::runtime(format!("A2.{name} takes no argument")));
                }
                new_fleet(lua, kind)
            })?,
        )?;
    }
    // The order a fleet gives one member; a pending `BuildObject` handle waits like any order.
    table.set(
        "FleetOrder",
        def(lua, env, "FleetOrder", &["hvi"], |_, e, _, a| {
            let Some(unit) = e.alive(a[0].id()) else { return nothing() };
            e.owned("FleetOrder", &unit)?;
            let (x, y, z) = a[1].vec();
            let to = to_vec3(x, y, z);
            match a[2].int() {
                MOVE => e.order("FleetOrder", unit.id, |units| Order::Move { units, to, queue: false })?,
                MOVE_WARP => e.order("FleetOrder", unit.id, |units| Order::MoveWarp { units, to, queue: false })?,
                _ => e.order("FleetOrder", unit.id, |units| Order::AttackPoint { units, at: to })?,
            }
            nothing()
        })?,
    )?;
    Ok(())
}
