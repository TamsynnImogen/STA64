//! Same-tick use of a built object (LUA-17).
//!
//! **Held calls.** Armada I's `BuildObject` returns the new object's handle at
//! once, and stock missions order it in the same breath (`BuildObject` then
//! `SetSpecialForcesFlag`, `PointAt`, `Goto`, `Attack`, `CineractiveShot` on the
//! new handle; read from data, the converted Armada I missions). STA64 builds
//! through the sim's command queue, so the handle is a pending token until the
//! spawn receipt one tick later. As for `A2` (see `a2/dll_objects.rs`), a void
//! order, setter or camera call named in [`HELD`] that is given a still-pending
//! token is held and replayed, in call order, by the first `A1` call after the
//! receipt (at most one tick late); a failed spawn drops it. Queries still raise
//! until the receipt is in.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table, Value};

use super::Env;
use crate::api::entity_ref::SpawnStatus;

/// Void calls whose first argument is an object, held while it is a pending spawn.
const HELD: &[&str] = &[
    "AI_Attack",
    "AddTransporterEffect",
    "AllowMultipleSpecialWeaponFire",
    "Attack",
    "CineractiveShot",
    "Cloak",
    "CraftCannotDie",
    "Decloak",
    "DestroyCritical",
    "DestroyEngines",
    "DestroyLifeSupport",
    "DestroySensors",
    "DestroyShieldGenerator",
    "DestroyWeapons",
    "DisableEngines",
    "DisableLifeSupport",
    "DisableSensors",
    "DisableShieldGenerator",
    "DisableWeapons",
    "EnableLight",
    "EnableWormholeTravel",
    "Goto",
    "Mine",
    "Patrol",
    "PointAt",
    "RemoveObject",
    "ResetWarpSpeed",
    "SearchAndDestroy",
    "SetCrew",
    "SetImpulseSpeed",
    "SetMaxShields",
    "SetPosition",
    "SetRace",
    "SetShieldPercent",
    "SetSpecialForcesFlag",
    "SetWarpSpeed",
    "SetWormholeTeam",
    "Special_Attack",
    "Stop",
];

/// What a raw argument is: a pending token, a failed one, or anything else.
#[derive(PartialEq)]
enum Tok {
    Pending,
    Failed,
    Other,
}

fn tok(env: &Env, v: &Value) -> Tok {
    let Value::Integer(n) = v else { return Tok::Other };
    let Ok(raw) = u32::try_from(*n) else { return Tok::Other };
    if raw < super::TOKEN_BASE {
        return Tok::Other;
    }
    match env.st.borrow().pending.get(&raw).map(|h| h.state.get()) {
        Some(SpawnStatus::Queued | SpawnStatus::Submitted(_)) => Tok::Pending,
        Some(SpawnStatus::Failed) => Tok::Failed,
        _ => Tok::Other,
    }
}

struct Held {
    name: String,
    inner: Function,
    args: Vec<Value>,
}

fn replay(env: &Env, held: &Rc<RefCell<Vec<Held>>>) {
    if held.borrow().is_empty() {
        return;
    }
    let all = std::mem::take(&mut *held.borrow_mut());
    let mut waiting = Vec::new();
    for h in all {
        if h.args.iter().any(|v| tok(env, v) == Tok::Pending) {
            waiting.push(h);
        } else if !h.args.iter().any(|v| tok(env, v) == Tok::Failed)
            && let Err(e) = h.inner.call::<MultiValue>(MultiValue::from_vec(h.args.clone()))
        {
            env.ctx.borrow_mut().unsupported(&format!("A1.{}: held until its object was built, then failed: {e}", h.name));
        }
    }
    let mut now = held.borrow_mut();
    waiting.append(&mut now);
    *now = waiting;
}

/// Wrap every function of the `A1` table (after all modules installed it).
pub(super) fn install(lua: &Lua, env: &Env, table: &Table) -> mlua::Result<()> {
    let held: Rc<RefCell<Vec<Held>>> = Rc::default();
    let entries: Vec<(String, Function)> = table
        .pairs::<String, Value>()
        .filter_map(Result::ok)
        .filter_map(|(k, v)| match v {
            Value::Function(f) => Some((k, f)),
            _ => None,
        })
        .collect();
    for (name, inner) in entries {
        let (env, held) = (env.clone(), held.clone());
        let holds = HELD.contains(&name.as_str());
        let label = name.clone();
        table.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                replay(&env, &held);
                if holds && args.iter().any(|v| tok(&env, v) == Tok::Pending) {
                    held.borrow_mut().push(Held { name: label.clone(), inner: inner.clone(), args: args.into_iter().collect() });
                    return Ok(MultiValue::new());
                }
                inner.call::<MultiValue>(args)
            })?,
        )?;
    }
    Ok(())
}
