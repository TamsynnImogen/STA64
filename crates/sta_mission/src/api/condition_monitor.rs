//! Observed named condition monitors. Timing and failure handling below are
//! explicit STA64 bridge policies, not a claim about native MMM defaults.

use mlua::{Function, Lua, MetaMethod, MultiValue, UserData, UserDataFields, UserDataMethods, UserDataRef, Value};

use super::{Shared, stub};

const MAX_ERRORS: u32 = 5;

struct Hook {
    serial: u64,
    id: String,
    function: Function,
    argument: Value,
    errors: u32,
    active: bool,
}

struct State {
    id: String,
    value: bool,
    active: bool,
    predicate: Function,
    argument: Value,
    errors: u32,
    hooks: Vec<Hook>,
}

#[derive(Default)]
pub(crate) struct Store {
    monitors: Vec<State>,
    next_hook: u64,
}

struct Monitor(usize, Shared);

fn bad(message: &str) -> mlua::Error {
    mlua::Error::runtime(format!("Monitor: {message}"))
}

fn string(value: &Value) -> mlua::Result<String> {
    match value {
        Value::String(s) => Ok(s.to_str()?.to_owned()),
        _ => Err(bad("id must be a string")),
    }
}

/// The caller strips the class table before dispatching the constructor.
pub(crate) fn make(lua: &Lua, ctx: &Shared, args: &[Value]) -> mlua::Result<mlua::AnyUserData> {
    if args.len() != 5 {
        return Err(bad("constructor expects (id, initialState, nil, predicate, argument)"));
    }
    let id = string(&args[0])?;
    let Value::Boolean(value) = args[1] else {
        return Err(bad("initialState must be boolean"));
    };
    if !args[2].is_nil() {
        return Err(bad("non-nil predicate table is unsupported"));
    }
    let Value::Function(predicate) = &args[3] else {
        return Err(bad("predicate must be a function"));
    };
    let index = {
        let c = ctx.borrow();
        if c.conditions.monitors.iter().any(|m| m.id == id) {
            return Err(bad("duplicate monitor id"));
        }
        c.conditions.monitors.len()
    };
    // Allocate the userdata before committing any registration.
    let handle = lua.create_userdata(Monitor(index, ctx.clone()))?;
    let mut c = ctx.borrow_mut();
    c.conditions.monitors.push(State {
        id,
        value,
        active: true,
        predicate: predicate.clone(),
        argument: args[4].clone(),
        errors: 0,
        hooks: Vec::new(),
    });
    Ok(handle)
}

impl UserData for Monitor {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("id", |_, m| Ok(m.1.borrow().conditions.monitors[m.0].id.clone()));
        f.add_field_method_get("state", |_, m| Ok(m.1.borrow().conditions.monitors[m.0].value));
        f.add_field_method_get("hooks", |_, m| Ok(m.1.borrow().conditions.monitors[m.0].hooks.len()));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Eq, |_, m, other: UserDataRef<Monitor>| {
            Ok(m.0 == other.0 && std::rc::Rc::ptr_eq(&m.1, &other.1))
        });
        m.add_method("stop", |_, m, args: MultiValue| {
            if !args.is_empty() {
                return Err(bad("stop takes no arguments"));
            }
            m.1.borrow_mut().conditions.monitors[m.0].active = false;
            Ok(())
        });
        m.add_method("hook", |_, m, args: MultiValue| {
            if args.len() != 5 {
                return Err(bad("hook expects (id, OnTrue, nil, callback, argument)"));
            }
            let id = string(&args[0])?;
            if !matches!(&args[1], Value::String(s) if s.to_str()?.as_ref() == "OnTrue") {
                return Err(bad("only MonitorHook.Type.OnTrue is supported"));
            }
            if !args[2].is_nil() {
                return Err(bad("non-nil hook table is unsupported"));
            }
            let Value::Function(function) = &args[3] else {
                return Err(bad("hook callback must be a function"));
            };
            let mut c = m.1.borrow_mut();
            if c.conditions.monitors[m.0].hooks.iter().any(|h| h.id == id) {
                return Err(bad("duplicate hook id"));
            }
            let serial = c
                .conditions
                .next_hook
                .checked_add(1)
                .ok_or_else(|| bad("hook ids exhausted"))?;
            c.conditions.monitors[m.0].hooks.push(Hook {
                serial,
                id,
                function: function.clone(),
                argument: args[4].clone(),
                errors: 0,
                active: true,
            });
            c.conditions.next_hook = serial;
            Ok(())
        });
        m.add_method("unhook", |_, m, args: MultiValue| {
            if args.len() > 1 {
                return Err(bad("unhook takes zero or one id"));
            }
            let id = args.front().map(string).transpose()?;
            let mut c = m.1.borrow_mut();
            let hooks = &mut c.conditions.monitors[m.0].hooks;
            if let Some(id) = id {
                hooks.retain(|h| h.id != id);
            } else {
                hooks.clear();
            }
            Ok(())
        });
        m.add_meta_method(MetaMethod::Index, |lua, m, key: Value| {
            if let Value::String(s) = key {
                m.1.borrow_mut()
                    .unsupported(&format!("Monitor.{}", s.to_string_lossy()));
            }
            stub(lua)
        });
    }
}

pub(crate) fn stop_all(store: &mut Store) {
    for m in &mut store.monitors {
        m.active = false;
    }
}

pub(crate) fn install(lua: &Lua) -> mlua::Result<()> {
    let class = lua.create_table()?;
    let kinds = lua.create_table()?;
    // Expose the observed name only. Other registered native kinds are not implemented.
    kinds.set("OnTrue", "OnTrue")?;
    class.set("Type", kinds)?;
    lua.globals().raw_set("MonitorHook", class)
}

fn error(result: Result<mlua::Result<Value>, Box<dyn std::any::Any + Send>>) -> Result<Value, String> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("panic in condition monitor".to_owned()),
    }
}

/// Snapshot identities at tick start, but recheck cancellation before each Lua
/// call. Lua predicates and hooks may queue commands or modify registrations.
pub(crate) fn fire(lua: &Lua, ctx: &Shared, errors: &mut Vec<String>) {
    let work: Vec<_> = {
        let c = ctx.borrow();
        c.conditions
            .monitors
            .iter()
            .enumerate()
            .filter(|(_, m)| m.active)
            .map(|(index, m)| {
                (
                    index,
                    m.predicate.clone(),
                    m.argument.clone(),
                    m.hooks
                        .iter()
                        .filter(|h| h.active)
                        .map(|h| h.serial)
                        .collect::<Vec<_>>(),
                )
            })
            .collect()
    };
    for (index, predicate, argument, hooks) in work {
        if !ctx.borrow().conditions.monitors[index].active {
            continue;
        }
        crate::sandbox::refill(lua);
        let result = error(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            predicate.call::<Value>((lua.create_userdata(Monitor(index, ctx.clone()))?, argument))
        })))
        .and_then(|v| match v {
            Value::Boolean(b) => Ok(b),
            _ => Err("predicate must return a boolean".to_owned()),
        });
        let rising = {
            let mut c = ctx.borrow_mut();
            let tick = c.view.tick;
            let state = &mut c.conditions.monitors[index];
            match result {
                Ok(value) => {
                    if !state.active {
                        continue;
                    }
                    let rising = value && !state.value;
                    state.value = value;
                    rising
                }
                Err(err) => {
                    state.errors += 1;
                    state.active = state.active && state.errors < MAX_ERRORS;
                    errors.push(format!(
                        "condition monitor {} at tick {tick}: {err}{}",
                        state.id,
                        if state.active { "" } else { " (disabled)" }
                    ));
                    false
                }
            }
        };
        if !rising {
            continue;
        }
        for serial in hooks {
            let callback = {
                let c = ctx.borrow();
                let state = &c.conditions.monitors[index];
                if !state.active {
                    break;
                }
                state
                    .hooks
                    .iter()
                    .find(|h| h.serial == serial && h.active)
                    .map(|h| (h.function.clone(), h.argument.clone(), h.id.clone()))
            };
            let Some((function, argument, hook_id)) = callback else {
                continue;
            };
            crate::sandbox::refill(lua);
            let result = error(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                function.call::<Value>((
                    lua.create_userdata(Monitor(index, ctx.clone()))?,
                    super::hook_object(lua, &hook_id, argument)?,
                ))
            })));
            if let Err(err) = result {
                let mut c = ctx.borrow_mut();
                let tick = c.view.tick;
                let state = &mut c.conditions.monitors[index];
                let mut disabled = false;
                if let Some(h) = state.hooks.iter_mut().find(|h| h.serial == serial) {
                    h.errors += 1;
                    h.active = h.errors < MAX_ERRORS;
                    disabled = !h.active;
                }
                errors.push(format!(
                    "condition monitor {} hook {serial} at tick {tick}: {err}{}",
                    state.id,
                    if disabled { " (disabled)" } else { "" }
                ));
            }
        }
    }
}
