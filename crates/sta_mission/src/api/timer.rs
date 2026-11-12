//! Tick-based mission timers. The named constructor and hook shape come from
//! the user-owned Recovery.lua conversion; other signatures are bridge choices.
//! A timer is one-shot by default. Seconds round half-up to at least one tick.

use mlua::{
    Function, Lua, MetaMethod, MultiValue, UserData, UserDataFields, UserDataMethods, Value,
};
use sta_core::TICK_HZ;

use super::{stub, Shared};

const MAX_ERRORS: u32 = 5;

pub(crate) struct TimerHook {
    serial: u64,
    id: String,
    func: Function,
    value: Value,
    errors: u32,
    active: bool,
}

pub(crate) struct TimerState {
    id: String,
    interval: u64,
    repeats: bool,
    started: bool,
    due: Option<u64>,
    version: u64,
    hooks: Vec<TimerHook>,
}

#[derive(Default)]
pub(crate) struct TimerStore {
    timers: Vec<TimerState>,
    next_hook_serial: u64,
}

#[derive(Clone)]
struct Timer(usize, Shared);

fn bad(message: &str) -> mlua::Error {
    mlua::Error::runtime(format!("Timer: {message}"))
}

fn interval(value: &Value) -> mlua::Result<u64> {
    let seconds = match value {
        Value::Number(n) => *n,
        Value::Integer(n) => *n as f64,
        _ => return Err(bad("interval must be a finite nonnegative number")),
    };
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(bad("interval must be a finite nonnegative number"));
    }
    let ticks = (seconds * TICK_HZ as f64 + 0.5).floor().max(1.0);
    if !ticks.is_finite() || ticks >= u64::MAX as f64 {
        return Err(bad("interval exceeds tick range"));
    }
    Ok(ticks as u64)
}

fn due(now: u64, interval: u64) -> mlua::Result<u64> {
    now.checked_add(interval)
        .ok_or_else(|| bad("due tick overflow"))
}

fn name(value: &Value, what: &str) -> mlua::Result<String> {
    match value {
        Value::String(s) => Ok(s.to_str()?.to_owned()),
        _ => Err(bad(what)),
    }
}

fn timer_state(store: &TimerStore, index: usize) -> mlua::Result<&TimerState> {
    store.timers.get(index).ok_or_else(|| bad("invalid handle"))
}

fn timer_state_mut(store: &mut TimerStore, index: usize) -> mlua::Result<&mut TimerState> {
    store
        .timers
        .get_mut(index)
        .ok_or_else(|| bad("invalid handle"))
}

impl UserData for Timer {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, t| {
            Ok(timer_state(&t.1.borrow().timers, t.0)?.id.clone())
        });
        fields.add_field_method_get("interval", |_, t| {
            Ok(timer_state(&t.1.borrow().timers, t.0)?.interval as f64 / TICK_HZ as f64)
        });
        fields.add_field_method_set("interval", |_, t, v: Value| {
            let ticks = interval(&v)?;
            let mut c = t.1.borrow_mut();
            let now = c.view.tick;
            let state = timer_state_mut(&mut c.timers, t.0)?;
            let next = if state.started {
                Some(due(now, ticks)?)
            } else {
                None
            };
            state.interval = ticks;
            state.due = next;
            state.version = state.version.wrapping_add(1);
            Ok(())
        });
        fields.add_field_method_get("repeats", |_, t| {
            Ok(timer_state(&t.1.borrow().timers, t.0)?.repeats)
        });
        fields.add_field_method_set("repeats", |_, t, v: Value| {
            let Value::Boolean(repeats) = v else {
                return Err(bad("repeats must be boolean"));
            };
            timer_state_mut(&mut t.1.borrow_mut().timers, t.0)?.repeats = repeats;
            Ok(())
        });
        fields.add_field_method_get("state", |_, t| {
            Ok(if timer_state(&t.1.borrow().timers, t.0)?.started {
                "Started"
            } else {
                "Stopped"
            })
        });
        fields.add_field_method_get("hooks", |_, t| {
            Ok(timer_state(&t.1.borrow().timers, t.0)?.hooks.len())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("start", |_, t, args: MultiValue| {
            if !args.is_empty() {
                return Err(bad("start takes no arguments"));
            }
            let mut c = t.1.borrow_mut();
            let now = c.view.tick;
            let state = timer_state_mut(&mut c.timers, t.0)?;
            let next = due(now, state.interval)?;
            state.started = true;
            state.due = Some(next);
            state.version = state.version.wrapping_add(1);
            Ok(())
        });
        methods.add_method("stop", |_, t, args: MultiValue| {
            if !args.is_empty() {
                return Err(bad("stop takes no arguments"));
            }
            let mut c = t.1.borrow_mut();
            let state = timer_state_mut(&mut c.timers, t.0)?;
            state.started = false;
            state.due = None;
            state.version = state.version.wrapping_add(1);
            Ok(())
        });
        methods.add_method("hook", |_, t, args: MultiValue| {
            if args.len() != 4 {
                return Err(bad("hook expects (id, condition, callback, value)"));
            }
            let id = name(&args[0], "hook id must be string")?;
            if !matches!(args[1], Value::Nil) {
                return Err(bad("non-nil hook condition is unsupported"));
            }
            let Value::Function(func) = &args[2] else {
                return Err(bad("hook callback must be function"));
            };
            let value = if matches!(args[3], Value::Nil) {
                args[0].clone()
            } else {
                args[3].clone()
            };
            let mut c = t.1.borrow_mut();
            if timer_state(&c.timers, t.0)?
                .hooks
                .iter()
                .any(|h| h.id == id)
            {
                return Err(bad("duplicate hook id"));
            }
            let serial = c
                .timers
                .next_hook_serial
                .checked_add(1)
                .ok_or_else(|| bad("hook serial overflow"))?;
            c.timers.next_hook_serial = serial;
            let state = timer_state_mut(&mut c.timers, t.0)?;
            state.hooks.push(TimerHook {
                serial,
                id,
                func: func.clone(),
                value,
                errors: 0,
                active: true,
            });
            Ok(())
        });
        methods.add_method("unhook", |_, t, args: MultiValue| {
            if args.len() > 1 {
                return Err(bad("unhook takes zero or one id"));
            }
            let id = args
                .front()
                .map(|v| name(v, "hook id must be string"))
                .transpose()?;
            let mut c = t.1.borrow_mut();
            let state = timer_state_mut(&mut c.timers, t.0)?;
            if let Some(id) = id {
                state.hooks.retain(|h| h.id != id);
            } else {
                state.hooks.clear();
            }
            state.version = state.version.wrapping_add(1);
            Ok(())
        });
        methods.add_meta_method(MetaMethod::Index, |lua, t, key: Value| {
            if let Value::String(s) = &key {
                t.1.borrow_mut()
                    .unsupported(&format!("Timer.{}", s.to_string_lossy()));
            }
            stub(lua)
        });
    }
}

pub(crate) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    let class = lua.create_table()?;
    let c = ctx.clone();
    let make = lua.create_function(move |lua, args: MultiValue| {
        // Lua passes the class table as the first argument of __call.
        let args: Vec<Value> = args.into_iter().skip(1).collect();
        if !(2..=3).contains(&args.len()) {
            return Err(bad("constructor expects (id, seconds[, state])"));
        }
        let id = name(&args[0], "id must be string")?;
        let ticks = interval(&args[1])?;
        let started = match args.get(2) {
            None | Some(Value::Nil) => false,
            Some(Value::String(s)) if s.to_str()? == "Started" => true,
            Some(Value::String(s)) if s.to_str()? == "Stopped" => false,
            _ => return Err(bad("state must be Timer.State.Started or Stopped")),
        };
        let mut b = c.borrow_mut();
        if b.timers.timers.iter().any(|t| t.id == id) {
            return Err(bad("duplicate id"));
        }
        let next = if started {
            Some(due(b.view.tick, ticks)?)
        } else {
            None
        };
        let index = b.timers.timers.len();
        b.timers.timers.push(TimerState {
            id,
            interval: ticks,
            repeats: false,
            started,
            due: next,
            version: 0,
            hooks: Vec::new(),
        });
        drop(b);
        lua.create_userdata(Timer(index, c.clone()))
    })?;
    let state = lua.create_table()?;
    state.set("Started", "Started")?;
    state.set("Stopped", "Stopped")?;
    class.set("State", state)?;
    let c = ctx.clone();
    class.set(
        "find",
        lua.create_function(move |lua, id: Value| {
            let id = name(&id, "find id must be string")?;
            let found = c.borrow().timers.timers.iter().position(|t| t.id == id);
            found
                .map(|i| lua.create_userdata(Timer(i, c.clone())))
                .transpose()
        })?,
    )?;
    let c = ctx.clone();
    class.set(
        "getAll",
        lua.create_function(move |lua, args: MultiValue| {
            if !args.is_empty() {
                return Err(bad("getAll takes no arguments"));
            }
            let all = lua.create_table()?;
            let count = c.borrow().timers.timers.len();
            for i in 0..count {
                all.set(i + 1, lua.create_userdata(Timer(i, c.clone()))?)?;
            }
            Ok(all)
        })?,
    )?;
    let c = ctx.clone();
    class.set(
        "stopAll",
        lua.create_function(move |_, args: MultiValue| {
            if !args.is_empty() {
                return Err(bad("stopAll takes no arguments"));
            }
            for t in &mut c.borrow_mut().timers.timers {
                t.started = false;
                t.due = None;
                t.version = t.version.wrapping_add(1);
            }
            Ok(())
        })?,
    )?;
    let mt = lua.create_table()?;
    mt.set("__call", make)?;
    let c = ctx.clone();
    mt.set(
        "__index",
        lua.create_function(move |lua, (_table, key): (Value, Value)| {
            if let Value::String(s) = &key {
                c.borrow_mut()
                    .unsupported(&format!("Timer.{}", s.to_string_lossy()));
            }
            stub(lua)
        })?,
    )?;
    class.set_metatable(Some(mt))?;
    lua.globals().raw_set("Timer", class)
}

/// Update due state before callbacks and keep no Ctx borrow across Lua calls.
pub(crate) fn fire_timers(lua: &Lua, ctx: &Shared, errors: &mut Vec<String>) {
    let mut firings = Vec::new();
    {
        let mut c = ctx.borrow_mut();
        let tick = c.view.tick;
        for (index, timer) in c.timers.timers.iter_mut().enumerate() {
            if !timer.started || !timer.due.is_some_and(|d| d <= tick) {
                continue;
            }
            if timer.repeats {
                timer.due = timer.due.and_then(|d| d.checked_add(timer.interval));
                if timer.due.is_none() {
                    timer.started = false;
                }
            } else {
                timer.started = false;
                timer.due = None;
            }
            for (hook, h) in timer.hooks.iter().enumerate().filter(|(_, h)| h.active) {
                firings.push((
                    index,
                    hook,
                    h.serial,
                    timer.version,
                    h.func.clone(),
                    h.value.clone(),
                    h.id.clone(),
                ));
            }
        }
    }
    for (index, hook, serial, version, func, value, hook_id) in firings {
        let valid = {
            let c = ctx.borrow();
            c.timers.timers.get(index).is_some_and(|t| {
                t.version == version
                    && t.hooks
                        .get(hook)
                        .is_some_and(|h| h.serial == serial && h.active)
            })
        };
        if !valid {
            continue;
        }
        crate::sandbox::refill(lua);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let handle = lua.create_userdata(Timer(index, ctx.clone()))?;
            func.call::<MultiValue>((handle, super::hook_object(lua, &hook_id, value)?))
        }));
        let err = match result {
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => e.to_string(),
            Err(_) => "panic in timer callback".to_owned(),
        };
        let mut c = ctx.borrow_mut();
        let tick = c.view.tick;
        let mut tail = "";
        if let Some(h) = c
            .timers
            .timers
            .get_mut(index)
            .and_then(|t| t.hooks.iter_mut().find(|h| h.serial == serial))
        {
            h.errors += 1;
            if h.errors >= MAX_ERRORS {
                h.active = false;
                tail = " (disabled)";
            }
        }
        errors.push(format!(
            "timer {} hook {} at tick {tick}: {err}{tail}",
            index, hook
        ));
    }
}

#[cfg(test)]
mod tests;
