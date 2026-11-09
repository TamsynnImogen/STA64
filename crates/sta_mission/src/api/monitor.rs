//! `Monitor`: condition callbacks, evaluated once per controller tick before
//! the script's own per-tick callbacks.
//!
//! ```lua
//! local m = Monitor()
//! m:hook(Monitor.Destroyed, unit, function(u) ... end)          -- once
//! m:hook(Monitor.Enters, center, radius, function(u) ... end [, player])
//! m:hook(Monitor.Timer, seconds, function() ... end [, repeating])
//! m:stop(); m:unhook(); Monitor.stopAll()
//! ```
//!
//! - `Destroyed` fires once, when the sim reports the unit destroyed (or it
//!   was seen alive and has left the view), with the handle of the dead unit.
//! - `Enters` fires for each unit (optionally of `player`) that comes inside
//!   the sphere; units already inside when hooked do not fire. It repeats
//!   until stopped.
//! - `Timer` fires after `seconds` of game time, once or repeating.
//!
//! The name inventory lists `hook`, `stop`, `stopAll`, `unhook`, `state`,
//! `id`, `hooks` but no arguments; the kinds and argument order are this
//! runtime's choice. Other kinds (`State.Started` hooks) are recorded as
//! unsupported and never fire.

use std::collections::BTreeSet;

use mlua::{Function, Lua, MetaMethod, MultiValue, UserData, UserDataFields, UserDataMethods, Value};
use sta_core::{EntityId, PlayerId, TICK_HZ};
use sta_sim::SimEvent;

use super::entity::handle;
use super::player::player_of;
use super::{dist, point_of, pos_f64, stub, Shared};

/// A failing monitor callback is disabled after this many errors.
const MAX_ERRORS: u32 = 5;

/// What a hook waits for.
#[derive(Debug, Clone, PartialEq)]
pub enum MonitorKind {
    Destroyed { target: EntityId, seen_alive: bool },
    Enters { centre: (f64, f64, f64), radius: f64, owner: Option<PlayerId>, inside: BTreeSet<EntityId> },
    Timer { due: u64, every: Option<u64> },
}

/// One hook of a monitor.
pub(crate) struct MonitorState {
    pub monitor: u64,
    pub hook: u64,
    pub kind: MonitorKind,
    pub func: Function,
    pub active: bool,
    pub errors: u32,
}

struct Monitor(u64, Shared);

fn add_hook(lua: &Lua, m: &Monitor, args: &MultiValue) -> mlua::Result<()> {
    let _ = lua;
    let mut c = m.1.borrow_mut();
    let Some(Value::String(kind)) = args.front() else {
        c.unsupported("Monitor:hook: non-string kind");
        return Ok(());
    };
    let kind = kind.to_string_lossy();
    let rest: Vec<Value> = args.iter().skip(1).cloned().collect();
    let func = |i: usize| match rest.get(i) {
        Some(Value::Function(f)) => Some(f.clone()),
        _ => None,
    };
    let parsed = match kind.as_str() {
        "Destroyed" => match (rest.first(), func(1)) {
            (Some(Value::UserData(u)), Some(f)) => u
                .borrow::<super::entity::Entity>()
                .ok()
                .map(|e| (MonitorKind::Destroyed { target: e.id, seen_alive: c.alive(e.id).is_some() }, f)),
            _ => None,
        },
        "Enters" => match (rest.first().and_then(|v| point_of(&c, v)), rest.get(1), func(2)) {
            (Some(centre), Some(r), Some(f)) => {
                let radius = match r {
                    Value::Number(n) => *n,
                    Value::Integer(n) => *n as f64,
                    _ => 0.0,
                };
                let owner = rest.get(3).and_then(player_of);
                let inside = c
                    .view
                    .units
                    .iter()
                    .filter(|u| owner.is_none_or(|o| u.owner == o) && dist(pos_f64(u.pos), centre) <= radius)
                    .map(|u| u.id)
                    .collect();
                Some((MonitorKind::Enters { centre, radius, owner, inside }, f))
            }
            _ => None,
        },
        "Timer" => match (rest.first(), func(1)) {
            (Some(Value::Number(_) | Value::Integer(_)), Some(f)) => {
                let secs = match rest[0] {
                    Value::Number(n) => n,
                    Value::Integer(n) => n as f64,
                    _ => 0.0,
                };
                let ticks = (secs * TICK_HZ as f64).round().max(1.0) as u64;
                let every = matches!(rest.get(2), Some(Value::Boolean(true))).then_some(ticks);
                Some((MonitorKind::Timer { due: c.view.tick + ticks, every }, f))
            }
            _ => None,
        },
        _ => None,
    };
    match parsed {
        Some((kind, func)) => {
            let hook = c.monitors.iter().map(|h| h.hook).max().unwrap_or(0) + 1;
            c.monitors.push(MonitorState { monitor: m.0, hook, kind, func, active: true, errors: 0 });
        }
        None => c.unsupported(&format!("Monitor:hook {kind}")),
    }
    Ok(())
}

impl UserData for Monitor {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("id", |_, m| Ok(m.0));
        f.add_field_method_get("hooks", |_, m| Ok(m.1.borrow().monitors.iter().filter(|h| h.monitor == m.0).count()));
        f.add_field_method_get("state", |_, m| {
            let on = m.1.borrow().monitors.iter().any(|h| h.monitor == m.0 && h.active);
            Ok(if on { "Active" } else { "Stopped" })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("hook", |lua, m, args: MultiValue| add_hook(lua, m, &args));
        m.add_method("stop", |_, m, _: MultiValue| {
            m.1.borrow_mut().monitors.iter_mut().filter(|h| h.monitor == m.0).for_each(|h| h.active = false);
            Ok(())
        });
        m.add_method("unhook", |_, m, _: MultiValue| {
            m.1.borrow_mut().monitors.retain(|h| h.monitor != m.0);
            Ok(())
        });
        m.add_meta_method(MetaMethod::Index, |lua, _, _: Value| stub(lua));
    }
}

pub(crate) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    let class = lua.create_table()?;
    for k in ["Destroyed", "Enters", "Timer"] {
        class.set(k, k)?;
    }
    let c = ctx.clone();
    let make = lua.create_function(move |lua, _: MultiValue| {
        let mut b = c.borrow_mut();
        b.next_monitor += 1;
        let id = b.next_monitor;
        drop(b);
        lua.create_userdata(Monitor(id, c.clone()))
    })?;
    let c = ctx.clone();
    class.set(
        "stopAll",
        lua.create_function(move |_, _: MultiValue| {
            c.borrow_mut().monitors.iter_mut().for_each(|h| h.active = false);
            Ok(())
        })?,
    )?;
    // Shorthands: `Monitor.destroyed(unit, fn)`, `.enters(...)`, `.timer(...)`
    // make a monitor, hook it and return it.
    for (short, kind) in [("destroyed", "Destroyed"), ("enters", "Enters"), ("timer", "Timer")] {
        let (c, make) = (ctx.clone(), make.clone());
        class.set(
            short,
            lua.create_function(move |lua, args: MultiValue| {
                let m: mlua::AnyUserData = make.call(())?;
                let mut full = MultiValue::new();
                full.push_back(Value::String(lua.create_string(kind)?));
                full.extend(args);
                add_hook(lua, &*m.borrow::<Monitor>()?, &full)?;
                let _ = &c;
                Ok(m)
            })?,
        )?;
    }
    let mt = lua.create_table()?;
    mt.set("__call", make)?;
    mt.set("__index", lua.create_function(|lua, (_t, _k): (Value, Value)| stub(lua))?)?;
    class.set_metatable(Some(mt))?;
    lua.globals().raw_set("Monitor", class)?;
    Ok(())
}

/// Evaluate every active hook against the current view and events, then
/// call the callbacks of those that fired. Callback errors are appended to
/// `errors` (and disable a hook after [`MAX_ERRORS`]).
pub(crate) fn fire_monitors(lua: &Lua, ctx: &Shared, errors: &mut Vec<String>) {
    let mut firings: Vec<(u64, Function, Option<EntityId>)> = Vec::new();
    {
        let mut c = ctx.borrow_mut();
        let c = &mut *c;
        let tick = c.view.tick;
        for h in c.monitors.iter_mut().filter(|h| h.active) {
            match &mut h.kind {
                MonitorKind::Destroyed { target, seen_alive } => {
                    let alive = c.view.unit(*target).is_some_and(|u| !u.flags.contains(sta_sim::view::UnitFlags::DYING));
                    let event = c.events.iter().any(|e| matches!(e, SimEvent::Destroyed { id, .. } if id == target));
                    if event || (*seen_alive && !alive) {
                        h.active = false;
                        firings.push((h.hook, h.func.clone(), Some(*target)));
                    } else if alive {
                        *seen_alive = true;
                    }
                }
                MonitorKind::Enters { centre, radius, owner, inside } => {
                    let now: BTreeSet<EntityId> = c
                        .view
                        .units
                        .iter()
                        .filter(|u| owner.is_none_or(|o| u.owner == o) && dist(pos_f64(u.pos), *centre) <= *radius)
                        .map(|u| u.id)
                        .collect();
                    for id in now.difference(inside) {
                        firings.push((h.hook, h.func.clone(), Some(*id)));
                    }
                    *inside = now;
                }
                MonitorKind::Timer { due, every } => {
                    if tick >= *due {
                        firings.push((h.hook, h.func.clone(), None));
                        match every {
                            Some(e) => *due += *e,
                            None => h.active = false,
                        }
                    }
                }
            }
        }
    }
    for (hook, func, id) in firings {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match id {
            Some(id) => handle(lua, ctx, id).and_then(|e| func.call::<MultiValue>(e)),
            None => func.call::<MultiValue>(()),
        }));
        let err = match result {
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => e.to_string(),
            Err(_) => "panic in monitor callback".to_owned(),
        };
        let mut c = ctx.borrow_mut();
        let tick = c.view.tick;
        let mut tail = "";
        if let Some(h) = c.monitors.iter_mut().find(|h| h.hook == hook) {
            h.errors += 1;
            if h.errors >= MAX_ERRORS {
                h.active = false;
                tail = " (disabled)";
            }
        }
        errors.push(format!("monitor hook {hook} at tick {tick}: {err}{tail}"));
    }
}
