//! The one place a mission Lua state is built (LUA-09, MODDING_DECISIONS D13).
//!
//! Mission scripts get an explicit standard-library set, a memory limit and an
//! instruction budget. There is no `io`, `os`, `package`, `debug`, `require`,
//! `load`, `loadstring`, `loadfile`, `dofile` or `string.dump`: a script can
//! neither touch the host nor compile code at run time, and a runaway loop or
//! allocation is stopped with a Lua error instead of hanging or exhausting the
//! machine. The limits are generous; see `docs/MISSIONS.md` ("Lua sandbox").
//!
//! The instruction budget counts VM instructions, so it is deterministic and
//! identical on every peer. It is refilled on every entry from the host
//! ([`refill`]): the script load, each callback, each monitor pass, each
//! `eval`. A script that exhausts it gets an error from that entry only.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use mlua::debug::DebugEvent;
use mlua::{HookTriggers, Lua, LuaOptions, StdLib, Value, VmState};

/// The standard libraries a mission state opens (the base library is always
/// present). Everything else (`io`, `os`, `package`, `debug`) is not opened.
pub fn std_libs() -> StdLib {
    StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH
}

/// Globals removed from the base library: run-time compilation and loading.
pub const REMOVED_GLOBALS: [&str; 5] = ["load", "loadstring", "loadfile", "dofile", "require"];

/// Memory limit of one mission state, in bytes (256 MiB).
pub const MEMORY_LIMIT: usize = 256 * 1024 * 1024;

/// VM instructions one host entry (load, callback, monitor pass) may run
/// before the script is stopped.
pub const INSTRUCTION_BUDGET: u64 = 100_000_000;

/// The hook fires every this many instructions and charges them to the budget.
pub const HOOK_GRANULARITY: u32 = 10_000;

/// The state's remaining budget, kept as Lua app data.
struct Budget(Arc<AtomicU64>);

/// A fresh mission Lua state with the restrictions above.
pub fn new_lua() -> Lua {
    let lua = Lua::new_with(std_libs(), LuaOptions::default()).expect("open the mission standard libraries");
    // Both calls only fail on a state not built by `Lua::new_with`.
    lua.set_memory_limit(MEMORY_LIMIT).expect("memory limit available (vendored Lua)");
    let remaining = Arc::new(AtomicU64::new(INSTRUCTION_BUDGET));
    let charge = remaining.clone();
    // Count events charge the budget. Call events (every function call, `pcall`
    // included) only check it: once the budget is spent a script cannot keep
    // going by catching the error in a `pcall` loop.
    let triggers = HookTriggers::new().every_nth_instruction(HOOK_GRANULARITY).on_calls();
    lua.set_global_hook(triggers, move |_, debug| {
        let left = charge.load(Ordering::Relaxed);
        let left = if matches!(debug.event(), DebugEvent::Count) {
            left.saturating_sub(u64::from(HOOK_GRANULARITY))
        } else {
            left
        };
        charge.store(left, Ordering::Relaxed);
        if left == 0 {
            Err(mlua::Error::runtime("mission script exceeded its instruction budget"))
        } else {
            Ok(VmState::Continue)
        }
    })
    .expect("instruction hook");
    lua.set_app_data(Budget(remaining));
    let globals = lua.globals();
    for name in REMOVED_GLOBALS {
        globals.raw_set(name, Value::Nil).expect("clear global");
    }
    if let Ok(string) = globals.raw_get::<mlua::Table>("string") {
        string.raw_set("dump", Value::Nil).expect("clear string.dump");
    }
    drop(globals);
    lua
}

/// Give `lua` a full instruction budget; call at every entry from the host.
pub fn refill(lua: &Lua) {
    if let Some(b) = lua.app_data_ref::<Budget>() {
        b.0.store(INSTRUCTION_BUDGET, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_libraries_and_loaders_are_absent() {
        let lua = new_lua();
        for g in ["io", "os", "package", "debug", "require", "load", "loadstring", "loadfile", "dofile"] {
            assert!(matches!(lua.globals().get::<Value>(g), Ok(Value::Nil)), "{g} must be unavailable");
        }
        assert!(lua.load("return string.dump").eval::<Value>().unwrap().is_nil());
        // What scripts need stays.
        let ok: bool = lua
            .load("return type(string.format) == 'function' and type(table.insert) == 'function' and type(math.floor) == 'function' and type(coroutine.wrap) == 'function' and type(pcall) == 'function'")
            .eval()
            .unwrap();
        assert!(ok);
    }

    #[test]
    fn a_runaway_loop_is_stopped_and_the_next_entry_runs() {
        let lua = new_lua();
        let err = lua.load("while true do end").exec().unwrap_err().to_string();
        assert!(err.contains("instruction budget"), "{err}");
        // pcall cannot swallow it for long: the budget stays at zero.
        let err = lua.load("while true do pcall(function() while true do end end) end").exec().unwrap_err().to_string();
        assert!(err.contains("instruction budget"), "{err}");
        refill(&lua);
        assert_eq!(lua.load("return 1 + 1").eval::<i64>().unwrap(), 2);
    }

    #[test]
    fn a_coroutine_is_limited_too() {
        let lua = new_lua();
        let err = lua.load("coroutine.wrap(function() while true do end end)()").exec().unwrap_err().to_string();
        assert!(err.contains("instruction budget"), "{err}");
    }

    #[test]
    fn runaway_allocation_hits_the_memory_limit() {
        let lua = new_lua();
        let err = lua
            .load("local s = string.rep('x', 1024 * 1024); local t = {} for i = 1, 100000 do t[i] = s .. i end")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(err.to_lowercase().contains("memory"), "{err}");
        lua.gc_collect().ok();
        refill(&lua);
        assert_eq!(lua.load("return 2").eval::<i64>().unwrap(), 2);
    }
}
