//! Bounded `TextInput:hook` bridge for mission text events.
//!
//! This accepts the six-argument tutorial call shape (including the colon
//! receiver): id, receiver table, callback, caller argument, Lua pattern.
//! Matching uses the Lua `string.find` captured at install, and dispatch is
//! limited to the mission seat's accepted sim events. `unhook(id)` is an
//! explicit bridge extension; native MMM timing is not inferred here.

use std::collections::BTreeSet;

use mlua::{Function, Lua, MultiValue, Table, UserData, UserDataMethods, Value};
use sta_core::PlayerId;
use sta_sim::SimEvent;

use super::Shared;

const MAX_PATTERN_BYTES: usize = 4096;
const MAX_ERRORS: u32 = 5;

struct Hook {
    serial: u64,
    id: String,
    receiver: Table,
    callback: Function,
    argument: Value,
    pattern: String,
    errors: u32,
    active: bool,
}

#[derive(Default)]
pub(crate) struct Store {
    hooks: Vec<Hook>,
    next_serial: u64,
    find: Option<Function>,
    seen_tick: Option<u64>,
    seen: BTreeSet<(PlayerId, u32)>,
}

struct TextInput(Shared);

fn bad(message: &str) -> mlua::Error {
    mlua::Error::runtime(format!("TextInput: {message}"))
}

fn string(value: &Value, name: &str) -> mlua::Result<String> {
    match value {
        Value::String(s) => Ok(s.to_str()?.to_owned()),
        _ => Err(bad(&format!("{name} must be a UTF-8 string"))),
    }
}

impl UserData for TextInput {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("hook", |_, input, args: MultiValue| {
            if args.len() != 5 {
                return Err(bad("hook expects (id, receiverTable, callback, argument, pattern)"));
            }
            let id = string(&args[0], "id")?;
            let Value::Table(receiver) = &args[1] else {
                return Err(bad("receiver must be a table"));
            };
            let Value::Function(callback) = &args[2] else {
                return Err(bad("callback must be a function"));
            };
            let pattern = string(&args[4], "pattern")?;
            if pattern.len() > MAX_PATTERN_BYTES {
                return Err(bad("pattern exceeds 4096 bytes"));
            }
            let find = {
                let c = input.0.borrow();
                if c.text_input.hooks.iter().any(|hook| hook.id == id && hook.active) {
                    return Err(bad("duplicate hook id"));
                }
                c.text_input.find.clone().expect("string.find captured at install")
            };
            // Preflight catches patterns the builtin evaluates on an empty
            // subject; suffix errors may surface only on a matching event.
            find.call::<Value>(("", pattern.as_str()))?;
            let mut c = input.0.borrow_mut();
            let store = &mut c.text_input;
            let serial = store
                .next_serial
                .checked_add(1)
                .ok_or_else(|| bad("hook serials exhausted"))?;
            store.hooks.push(Hook {
                serial,
                id,
                receiver: receiver.clone(),
                callback: callback.clone(),
                argument: args[3].clone(),
                pattern,
                errors: 0,
                active: true,
            });
            store.next_serial = serial;
            Ok(())
        });
        methods.add_method("unhook", |_, input, args: MultiValue| {
            if args.len() != 1 {
                return Err(bad("unhook expects one id"));
            }
            let id = string(&args[0], "id")?;
            input.0.borrow_mut().text_input.hooks.retain(|hook| hook.id != id);
            Ok(())
        });
    }
}

pub(crate) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    let find: Function = lua.globals().get::<Table>("string")?.get("find")?;
    let input = lua.create_userdata(TextInput(ctx.clone()))?;
    ctx.borrow_mut().text_input.find = Some(find);
    lua.globals().raw_set("TextInput", input)
}

fn error(result: Result<mlua::Result<Value>, Box<dyn std::any::Any + Send>>) -> Result<Value, String> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("panic in TextInput hook".to_owned()),
    }
}

/// Snapshot hook serials for the entire event batch, then check cancellation
/// before each match/callback. A consumed `(player,seq)` cannot fire again on
/// another `think` at this view tick, including after a Lua error.
pub(crate) fn fire(lua: &Lua, ctx: &Shared, errors: &mut Vec<String>) {
    let (events, serials, find, tick) = {
        let mut c = ctx.borrow_mut();
        let tick = c.view.tick;
        let seat = c.seat;
        let events = c.events.clone();
        let store = &mut c.text_input;
        if store.seen_tick != Some(tick) {
            store.seen_tick = Some(tick);
            store.seen.clear();
        }
        let serials = store
            .hooks
            .iter()
            .filter(|h| h.active)
            .map(|h| h.serial)
            .collect::<Vec<_>>();
        let eligible = events
            .into_iter()
            .filter_map(|event| match event {
                SimEvent::MissionText { player, seq, text }
                    if player.is_player()
                        && player == seat
                        && text.len() <= sta_sim::command::MAX_MISSION_TEXT_BYTES
                        && store.seen.insert((player, seq)) =>
                {
                    Some(text)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        (eligible, serials, store.find.clone(), tick)
    };
    let Some(find) = find else { return };
    for text in events {
        for &serial in &serials {
            let hook = {
                let c = ctx.borrow();
                c.text_input
                    .hooks
                    .iter()
                    .find(|h| h.serial == serial && h.active)
                    .map(|h| {
                        (
                            h.id.clone(),
                            h.receiver.clone(),
                            h.callback.clone(),
                            h.argument.clone(),
                            h.pattern.clone(),
                        )
                    })
            };
            let Some((id, receiver, callback, argument, pattern)) = hook else {
                continue;
            };
            crate::sandbox::refill(lua);
            let matched = error(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                find.call::<Value>((text.as_str(), pattern.as_str()))
            })));
            let result = match matched {
                Ok(Value::Nil) => continue,
                Ok(_) => error(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    callback.call::<Value>((receiver, text.as_str(), argument))
                }))),
                Err(err) => Err(err),
            };
            if let Err(err) = result {
                let disabled = {
                    let mut c = ctx.borrow_mut();
                    if let Some(h) = c.text_input.hooks.iter_mut().find(|h| h.serial == serial) {
                        h.errors += 1;
                        h.active = h.errors < MAX_ERRORS;
                        !h.active
                    } else {
                        false
                    }
                };
                errors.push(format!(
                    "TextInput hook {id} serial {serial} at tick {tick}: {err}{}",
                    if disabled { " (disabled)" } else { "" }
                ));
            }
        }
    }
    let _ = lua; // The captured function and callbacks own their Lua VM handles.
}
