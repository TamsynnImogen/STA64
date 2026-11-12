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
use mlua::{Function, HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, Value, VmState};

/// The standard libraries a mission state opens (the base library is always
/// present). Everything else (`io`, `os`, `package`, `debug`) is not opened.
pub fn std_libs() -> StdLib {
    StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH
}

/// Globals removed from the base library: run-time compilation and loading,
/// and `collectgarbage` (the host owns GC timing; it decides when `__gc` and
/// frees run, which must not differ between peers).
pub const REMOVED_GLOBALS: [&str; 6] = ["load", "loadstring", "loadfile", "dofile", "require", "collectgarbage"];

/// Memory limit of one mission state, in bytes (256 MiB).
pub const MEMORY_LIMIT: usize = 256 * 1024 * 1024;

/// VM instructions one host entry (load, callback, monitor pass) may run
/// before the script is stopped.
pub const INSTRUCTION_BUDGET: u64 = 10_000_000;

/// The hook fires every this many instructions and charges them to the budget
/// (so the budget is granular to this many instructions).
pub const HOOK_GRANULARITY: u32 = 10_000;

/// Longest string `string.rep` may build, in bytes. Work done inside one C
/// call is invisible to the instruction hook, so such calls are capped.
pub const STRING_REP_MAX: usize = 16 * 1024 * 1024;

/// Pattern-matching functions (`find`, `match`, `gmatch`, `gsub`) take a
/// subject of at most `PATTERN_SUBJECT_MAX` bytes and a pattern of at most
/// `PATTERN_MAX` bytes with at most `PATTERN_QUANTIFIERS_MAX` of `* + - ?`:
/// Lua's matcher backtracks in time polynomial in the subject length with the
/// quantifier count as exponent, all inside one C call. `string.find` with
/// the `plain` flag is linear and exempt.
pub const PATTERN_SUBJECT_MAX: usize = 1024;
pub const PATTERN_MAX: usize = 256;
pub const PATTERN_QUANTIFIERS_MAX: usize = 3;

/// The state's remaining budget, kept as Lua app data.
struct Budget {
    remaining: Arc<AtomicU64>,
    /// Most instructions any one entry has used so far.
    peak: Arc<AtomicU64>,
}

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
    lua.set_app_data(Budget { remaining, peak: Arc::new(AtomicU64::new(0)) });
    let globals = lua.globals();
    for name in REMOVED_GLOBALS {
        globals.raw_set(name, Value::Nil).expect("clear global");
    }
    guard_setmetatable(&lua, &globals).expect("guard setmetatable");
    if let Ok(string) = globals.raw_get::<Table>("string") {
        string.raw_set("dump", Value::Nil).expect("clear string.dump");
        guard_string(&lua, &string).expect("guard string functions");
    }
    drop(globals);
    lua
}

/// Give `lua` a full instruction budget; call at every entry from the host.
pub fn refill(lua: &Lua) {
    if let Some(b) = lua.app_data_ref::<Budget>() {
        let used = INSTRUCTION_BUDGET - b.remaining.load(Ordering::Relaxed).min(INSTRUCTION_BUDGET);
        b.peak.fetch_max(used, Ordering::Relaxed);
        b.remaining.store(INSTRUCTION_BUDGET, Ordering::Relaxed);
    }
}

/// The most instructions one host entry has used so far (measured at the
/// refills; the entry in progress is not counted). For reports and tests.
pub fn peak_used(lua: &Lua) -> u64 {
    lua.app_data_ref::<Budget>().map_or(0, |b| b.peak.load(Ordering::Relaxed))
}

fn restricted(name: &str, why: String) -> mlua::Error {
    mlua::Error::runtime(format!("{name}: {why}"))
}

/// `setmetatable` that refuses a `__gc` finalizer: Lua runs finalizers with
/// hooks disabled, so a looping finalizer could never be stopped.
fn guard_setmetatable(lua: &Lua, globals: &Table) -> mlua::Result<()> {
    let original: Function = globals.raw_get("setmetatable")?;
    globals.raw_set(
        "setmetatable",
        lua.create_function(move |_, args: MultiValue| {
            if let Some(Value::Table(meta)) = args.get(1)
                && !meta.raw_get::<Value>("__gc")?.is_nil()
            {
                return Err(restricted("setmetatable", "__gc finalizers are not available to mission scripts".into()));
            }
            original.call::<MultiValue>(args)
        })?,
    )
}

/// Bytes of a string or number argument, `None` for anything else (the
/// original function then reports the type error itself).
fn byte_len(value: Option<&Value>) -> Option<usize> {
    match value? {
        Value::String(s) => Some(s.as_bytes().len()),
        Value::Integer(_) | Value::Number(_) => Some(32),
        _ => None,
    }
}

fn guard_string(lua: &Lua, string: &Table) -> mlua::Result<()> {
    let rep: Function = string.raw_get("rep")?;
    string.raw_set(
        "rep",
        lua.create_function(move |_, args: MultiValue| {
            let count = match args.get(1) {
                Some(Value::Integer(n)) => Some(*n),
                Some(Value::Number(n)) if n.is_finite() => Some(*n as i64),
                _ => None,
            };
            let (Some(len), Some(n)) = (byte_len(args.front()), count) else {
                return rep.call::<MultiValue>(args);
            };
            let sep = byte_len(args.get(2)).unwrap_or(0);
            let total = (len as u128 + sep as u128) * n.max(0) as u128;
            if total > STRING_REP_MAX as u128 {
                return Err(restricted("string.rep", format!("result over {STRING_REP_MAX} bytes")));
            }
            rep.call::<MultiValue>(args)
        })?,
    )?;
    for name in ["find", "match", "gmatch", "gsub"] {
        let original: Function = string.raw_get(name)?;
        string.raw_set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                let plain = name == "find" && args.get(3).is_some_and(|v| !matches!(v, Value::Nil | Value::Boolean(false)));
                if !plain {
                    if let Some(len) = byte_len(args.front())
                        && len > PATTERN_SUBJECT_MAX
                    {
                        return Err(restricted(
                            &format!("string.{name}"),
                            format!("subject over {PATTERN_SUBJECT_MAX} bytes"),
                        ));
                    }
                    if let Some(Value::String(pattern)) = args.get(1) {
                        let bytes = pattern.as_bytes();
                        let quantifiers = bytes.iter().filter(|b| matches!(b, b'*' | b'+' | b'-' | b'?')).count();
                        if bytes.len() > PATTERN_MAX || quantifiers > PATTERN_QUANTIFIERS_MAX {
                            return Err(restricted(
                                &format!("string.{name}"),
                                format!("pattern over {PATTERN_MAX} bytes or {PATTERN_QUANTIFIERS_MAX} quantifiers"),
                            ));
                        }
                    }
                }
                original.call::<MultiValue>(args)
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_libraries_and_loaders_are_absent() {
        let lua = new_lua();
        for g in ["io", "os", "package", "debug", "require", "load", "loadstring", "loadfile", "dofile", "collectgarbage"] {
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

    #[test]
    fn gc_finalizers_and_c_side_string_work_are_refused() {
        let lua = new_lua();
        let err = lua.load("setmetatable({}, {__gc = function() while true do end end})").exec().unwrap_err().to_string();
        assert!(err.contains("__gc"), "{err}");
        // Ordinary metatables are untouched.
        assert!(lua.load("return getmetatable(setmetatable({}, {__index = {a = 1}})).__index.a == 1").eval::<bool>().unwrap());
        let err = lua.load("return string.rep('x', 1024 * 1024 * 1024)").eval::<Value>().unwrap_err().to_string();
        assert!(err.contains("string.rep"), "{err}");
        assert_eq!(lua.load("return #string.rep('ab', 3, ',')").eval::<i64>().unwrap(), 8);
        let slow = "return string.find(string.rep('a', 1000), string.rep('a-', 25) .. 'b')";
        let err = lua.load(slow).eval::<Value>().unwrap_err().to_string();
        assert!(err.contains("string.find"), "{err}");
        let err = lua.load("return string.find(string.rep('a', 2000), 'a')").eval::<Value>().unwrap_err().to_string();
        assert!(err.contains("subject"), "{err}");
        // Everyday use still works, plain find is exempt.
        assert_eq!(lua.load("return string.find('hello world', 'o w')").eval::<i64>().unwrap(), 5);
        assert_eq!(lua.load("return (string.gsub('  hi  ', '^%s*(.-)%s*$', '%1'))").eval::<String>().unwrap(), "hi");
        assert_eq!(lua.load("return string.find(string.rep('a', 5000), 'aab', 1, true)").eval::<Option<i64>>().unwrap(), None);
    }

    fn step_all(rt: &mut crate::runtime::MissionRuntime, ticks: u64) {
        for t in 0..ticks {
            rt.step(t);
        }
    }

    #[test]
    fn a_runaway_callback_is_stopped_and_disabled_while_the_others_keep_running() {
        let mut rt = crate::runtime::MissionRuntime::from_source(
            "missions/T.lua",
            "N = 0\nMMM.register(function() while true do end end)\nMMM.register(function() N = N + 1 end)",
            1,
        );
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        step_all(&mut rt, 6);
        assert!(rt.errors().iter().any(|e| e.contains("instruction budget")), "{:?}", rt.errors());
        assert_eq!(rt.eval::<i64>("return N").unwrap(), 6);
        assert!(rt.instruction_peak() >= INSTRUCTION_BUDGET - u64::from(HOOK_GRANULARITY));
    }

    #[test]
    fn a_runaway_load_is_reported_and_bytecode_is_refused() {
        let rt = crate::runtime::MissionRuntime::from_source("missions/T.lua", "while true do end", 1);
        assert!(rt.errors().iter().any(|e| e.contains("instruction budget")), "{:?}", rt.errors());
        let rt = crate::runtime::MissionRuntime::from_source("missions/T.lua", "\u{1b}Lua binary", 1);
        assert!(rt.errors().iter().any(|e| e.contains("binary")), "{:?}", rt.errors());
        // io/os resolve to recorded stubs, with no effect on the host.
        let rt = crate::runtime::MissionRuntime::from_source(
            "missions/T.lua",
            "local f = io and io.open and io.open('/tmp/sta64-should-not-exist', 'w'); X = f == nil or type(f) ~= 'userdata'",
            1,
        );
        assert!(!std::path::Path::new("/tmp/sta64-should-not-exist").exists());
        assert!(rt.eval::<bool>("return X").unwrap());
    }
}
