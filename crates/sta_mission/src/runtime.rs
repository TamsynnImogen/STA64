//! The live mission runtime: a mission script as a [`Controller`].
//!
//! Where [`crate::script::dry_run`] stubs every global to see what a script
//! touches, [`MissionRuntime`] runs the script for real. Per decision D3 it
//! runs on the host like a player: each controller tick it calls the
//! callbacks the script registered and returns commands through `think`
//! (replays record those commands and never rerun the script).
//!
//! # Lua environment
//!
//! - deterministic `math.*` and `math.random` seeded with the game seed
//!   ([`crate::math::install`]);
//! - luabind's `class` and `include(name)` (resolved through the caller's
//!   reader, `missions/<name>` first);
//! - `MMM.register(callback) -> id`: the callback is called once per tick as
//!   `callback(time)` with `time` the game time in seconds (`tick / TICK_HZ`,
//!   a float). `MMM.unregister(id)` removes it. `docs/MMM_API.md` lists
//!   names only, so this signature is an assumption to revisit when a
//!   script shows otherwise;
//! - the MMM API of [`crate::api`];
//! - every other global or `MMM.*` name is a permissive stub (any index or
//!   call returns another stub), so real scripts load before their API
//!   exists. LUA-03 replaces stubs name by name.
//!
//! `MMM.register(object)` also accepts the class instances used by the
//! clean-room A2 ports. Its successful `setup(self)` runs once at the first
//! controller tick, followed by `update(self, time)` each tick. Function
//! registrations keep their original ordering and signature. These are
//! bridge lifecycle rules: optional `cineractiveUpdate(self)` runs while active,
//! followed by `cineractiveFinish(self, false)` once an observed active level ends.
//! Their method handles are selected before each normal update; forced skips
//! and native immediate timing are not implemented.
//!
//! # Error isolation
//!
//! Nothing a script does panics the sim. A load error is kept in
//! [`MissionRuntime::errors`] and leaves a runtime with whatever callbacks
//! registered before the error. A callback error is recorded (callback id
//! and tick), the other callbacks still run that tick, and the callback is
//! called again next tick. After [`MAX_CALLBACK_ERRORS`] errors in total it
//! is disabled for good so a broken script cannot flood the log.
//!
//! # Commands (LUA-03)
//!
//! The API bindings (`crate::api`) queue commands; `think` drains them into
//! `out`, and [`MissionRuntime::take_commands`] drains them directly.
//!
//! # Seam gaps
//!
//! `Controller::think` receives no `PlayerId`, so a mission cannot know
//! which seat it owns, and it sees only a [`WorldView`]: the ODF and content
//! tables scripts query are unreachable from it. [`MissionConfig`] passes
//! the seat, `Arc<ContentDb>` and map labels in at construction instead.

use mlua::{Function, Lua, MultiValue, Table, Value};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use sta_sim::command::Command;
use sta_sim::setup::Controller;
use sta_sim::view::WorldView;
use sta_sim::{SimEvent, TICK_HZ};
use sta_vfs::Vfs;

use crate::api::{self, Ctx, MissionConfig, Objective, Shared as ApiShared};

/// A callback that has failed this many times is disabled.
pub const MAX_CALLBACK_ERRORS: u32 = 5;
/// Errors kept for inspection; later ones are dropped.
const MAX_LOGGED: usize = 100;

/// A permissive stand-in for API that does not exist yet.
const STUBS: &str = r#"
local record = ...
local kindOf, stringify = type, tostring
local function label(key)
  local kind = kindOf(key)
  if kind == "string" then return key end
  if kind == "number" then return stringify(key) end
  return "<" .. kind .. ">"
end
local function field(path, key)
  local name = path .. "." .. label(key)
  record("stub field " .. name)
  return name
end
local function mk(path)
  local t = {}
  setmetatable(t, {
    __index = function(_, key) return mk(path and field(path, key)) end,
    __call = function() return mk(path and (path .. "()")) end,
    __add = function() return 0 end, __sub = function() return 0 end,
    __mul = function() return 0 end, __div = function() return 0 end,
    __mod = function() return 0 end, __unm = function() return 0 end,
    __idiv = function() return 0 end, __pow = function() return 0 end,
    __concat = function() return "" end, __len = function() return 0 end,
    __lt = function() return false end, __le = function() return false end,
    __tostring = function() return "<stub>" end,
  })
  return t
end
rawset(_G, "__stub", mk)
setmetatable(_G, { __index = function(_, key)
  local name = label(key)
  record("stub global " .. name)
  return mk(name)
end })
setmetatable(MMM, { __index = function(_, key) return mk(field("MMM", key)) end })
"#;

struct Callback {
    id: u64,
    func: Function,
    errors: u32,
    disabled: bool,
}

#[derive(Default)]
struct Shared {
    next_id: u64,
    /// Registered since the last tick; moved into the runtime's list.
    added: Vec<(u64, Function)>,
    removed: Vec<u64>,
}

struct Inner {
    // Callbacks hold `Function`s of `lua`: declared first, dropped first.
    callbacks: Vec<Callback>,
    shared: Rc<RefCell<Shared>>,
    ctx: ApiShared,
    #[allow(dead_code)] // owns the state the callbacks belong to
    lua: Lua,
}

/// Where a script's `SucceedMission` / `FailMission` stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionEndState {
    /// Not asked for.
    None,
    /// Asked for; mission time has not passed its end time.
    Pending,
    /// The mission has ended by its own rules.
    Ended { won: bool },
    /// Asked for, but on the strength of a query about an object that never
    /// existed: neither a win nor a loss. The reason names what is missing.
    Blocked(String),
}

/// A mission script running as a [`Controller`].
///
/// The host owns one runtime for its SCRIPT command stream and supplies the
/// complete events from a submitted batch with the next view. Independent
/// runtimes must not share SCRIPT sequences or spawn request tokens.
pub struct MissionRuntime {
    inner: Inner,
    errors: Vec<String>,
}

// SAFETY: `Lua` is not `Send` because its handles share non-atomic
// reference counts. Everything that touches them (the state, the callback
// `Function`s, the closures installed into it) lives inside `Inner`, which
// is moved as a whole and never cloned out, so one thread at a time uses
// them. Public inspection accepts only sealed standard owned conversions;
// neither Lua handles nor custom converters can escape via eval/eval_with.
// The reader closure is required to be Send, and error-send plus a static
// assertion keep errors from exporting thread-affine values either.
unsafe impl Send for MissionRuntime {}

type Reader = api::MissionReader;

mod inspection;
pub use inspection::{EvalArgs, EvalResult};

/// Adapt the class instances used by the converted missions without exposing
/// another callback kind outside the Lua state. Successful setup runs once,
/// against the first controller view; update then runs once per tick. Setup
/// failures use the existing callback retry/error budget, as function callbacks
/// do. Cinematic hooks use an explicit polling policy: normal update first,
/// cinematic update while active, then a single normal-finish notification.
/// Phase errors abort the remaining phases of that poll and use the existing
/// per-object budget; later phases require successful preceding phases.
fn object_callback(lua: &Lua, object: Table) -> mlua::Result<Function> {
    let mut any = false;
    for name in ["setup", "update"] {
        match object.get::<Value>(name)? {
            Value::Nil => {},
            Value::Function(_) => any = true,
            _ => return Err(mlua::Error::runtime(format!("mission object {name} must be a function or nil"))),
        }
    }
    for name in ["cineractiveUpdate", "cineractiveFinish"] {
        match object.get::<Value>(name)? {
            Value::Nil | Value::Function(_) => {},
            _ => return Err(mlua::Error::runtime(format!("mission object {name} must be a function or nil"))),
        }
    }
    if !any {
        return Err(mlua::Error::runtime("mission object must define setup or update"));
    }
    let mut initialized = false;
    let mut was_active = false;
    lua.create_function_mut(move |lua, time: f64| {
        if !initialized {
            if let Some(setup) = object.get::<Option<Function>>("setup")? {
                setup.call::<MultiValue>(object.clone())?;
            }
            initialized = true;
        }
        // Select this poll's methods before update, as with monitor hook snapshots.
        let cinematic_update = object.get::<Option<Function>>("cineractiveUpdate")?;
        let cinematic_finish = object.get::<Option<Function>>("cineractiveFinish")?;
        let cinematic_hooks = cinematic_update.is_some() || cinematic_finish.is_some();
        if cinematic_hooks {
            was_active |= api::cineractive_active(lua)?;
        }
        if let Some(update) = object.get::<Option<Function>>("update")? {
            update.call::<MultiValue>((object.clone(), time))?;
        }
        if cinematic_hooks {
            if api::cineractive_active(lua)? {
                was_active = true;
                if let Some(update) = cinematic_update {
                    update.call::<MultiValue>(object.clone())?;
                }
            }
            if was_active && !api::cineractive_active(lua)? {
                // Consume the edge before calling Lua: a failed finish is not replayed.
                was_active = false;
                if let Some(finish) = cinematic_finish {
                    finish.call::<MultiValue>((object.clone(), false))?;
                }
            }
            // A finish may start a new cinematic; it is updated on the next poll.
            was_active = api::cineractive_active(lua)?;
        } else {
            was_active = false;
        }
        Ok(())
    })
}

impl MissionRuntime {
    /// Load `path` (a VFS path such as `missions/Eviction.lua`) from `vfs`.
    pub fn from_vfs(vfs: Arc<Vfs>, path: &str, seed: u64) -> MissionRuntime {
        MissionRuntime::from_vfs_with(vfs, path, seed, MissionConfig::default())
    }

    /// [`MissionRuntime::from_vfs`] with the game's seat, content and labels.
    pub fn from_vfs_with(vfs: Arc<Vfs>, path: &str, seed: u64, cfg: MissionConfig) -> MissionRuntime {
        MissionRuntime::load_with(path, seed, cfg, move |name| vfs.read(name).and_then(Result::ok))
    }

    /// Load the script `path`, reading it and its `include()`s with `read`
    /// (called with VFS-style paths, `missions/<name>` first).
    pub fn load(path: &str, seed: u64, read: impl Fn(&str) -> Option<Vec<u8>> + Send + 'static) -> MissionRuntime {
        MissionRuntime::load_with(path, seed, MissionConfig::default(), read)
    }

    /// [`MissionRuntime::load`] with the game's seat, content and labels.
    pub fn load_with(
        path: &str,
        seed: u64,
        cfg: MissionConfig,
        read: impl Fn(&str) -> Option<Vec<u8>> + Send + 'static,
    ) -> MissionRuntime {
        let shared = Rc::new(RefCell::new(Shared::default()));
        let ctx: ApiShared = Rc::new(RefCell::new(Ctx::new(cfg, seed)));
        let lua = crate::sandbox::new_lua();
        let mut errors = Vec::new();
        if let Err(e) = Self::setup(&lua, &shared, &ctx, seed, Arc::new(Mutex::new(read)), path) {
            errors.push(format!("{path}: {e}"));
        }
        MissionRuntime { inner: Inner { callbacks: Vec::new(), shared, ctx, lua }, errors }
    }

    /// Load a script given as text (tests, tools).
    pub fn from_source(name: &str, source: &str, seed: u64) -> MissionRuntime {
        MissionRuntime::from_source_with(name, source, seed, MissionConfig::default())
    }

    /// [`MissionRuntime::from_source`] with the game's seat, content and labels.
    pub fn from_source_with(name: &str, source: &str, seed: u64, cfg: MissionConfig) -> MissionRuntime {
        let (n, s) = (name.to_owned(), source.as_bytes().to_vec());
        MissionRuntime::load_with(name, seed, cfg, move |p| (p == n).then(|| s.clone()))
    }

    fn setup(lua: &Lua, shared: &Rc<RefCell<Shared>>, ctx: &ApiShared, seed: u64, read: Reader, path: &str) -> mlua::Result<()> {
        ctx.borrow_mut().reader = Some(read.clone());
        crate::math::install(lua, seed)?;
        lua.load(crate::script::LUABIND_CLASS).set_name("=luabind class").exec()?;
        let globals = lua.globals();

        let mmm = lua.create_table()?;
        let sh = shared.clone();
        mmm.set(
            "register",
            lua.create_function(move |lua, value: Value| {
                let f = match value {
                    Value::Function(f) => f,
                    Value::Table(t) => object_callback(lua, t)?,
                    _ => return Err(mlua::Error::runtime("MMM.register expects a function or mission object")),
                };
                let mut s = sh.borrow_mut();
                s.next_id = s.next_id.checked_add(1).ok_or_else(|| mlua::Error::runtime("mission callback ids exhausted"))?;
                let id = s.next_id;
                s.added.push((id, f));
                Ok(id)
            })?,
        )?;
        let sh = shared.clone();
        mmm.set(
            "unregister",
            lua.create_function(move |_, id: u64| {
                sh.borrow_mut().removed.push(id);
                Ok(())
            })?,
        )?;
        globals.raw_set("MMM", mmm)?;

        let r2 = read.clone();
        globals.raw_set(
            "include",
            lua.create_function(move |lua, name: String| {
                let name = name.replace('\\', "/");
                let found = {
                    let r = r2.lock().unwrap_or_else(|p| p.into_inner());
                    [format!("missions/{name}"), name.clone()].into_iter().find_map(|p| r(&p).map(|b| (p, b)))
                };
                let Some((p, src)) = found else {
                    return Err(mlua::Error::runtime(format!("include {name}: not found")));
                };
                lua.load(&src[..]).set_mode(mlua::chunk::ChunkMode::Text).set_name(format!("@{p}")).exec()
            })?,
        )?;

        api::install(lua, ctx)?;
        let diagnostics = ctx.clone();
        let record = lua.create_function(move |_, name: String| {
            diagnostics.borrow_mut().unsupported(&name);
            Ok(())
        })?;
        lua.load(STUBS).set_name("=stubs").call::<()>(record)?;
        let src = {
            let r = read.lock().unwrap_or_else(|p| p.into_inner());
            r(path).ok_or_else(|| mlua::Error::runtime("not found"))?
        };
        lua.load(&src[..]).set_mode(mlua::chunk::ChunkMode::Text).set_name(format!("@{path}")).exec()
    }

    /// Evaluate a chunk, returning only sealed owned data. Lua handles and
    /// custom conversions cannot escape this runtime's manual Send boundary.
    ///
    /// ```compile_fail
    /// use sta_mission::runtime::MissionRuntime;
    /// let rt = MissionRuntime::from_source("test.lua", "", 1);
    /// let _: mlua::Table = rt.eval("return {}").unwrap();
    /// ```
    ///
    /// A Send result alone would not prevent a custom converter retaining
    /// a Lua clone as a side effect; custom conversions are also sealed out.
    ///
    /// ```compile_fail
    /// use sta_mission::runtime::MissionRuntime;
    /// struct Custom;
    /// impl mlua::FromLuaMulti for Custom {
    ///     fn from_lua_multi(_: mlua::MultiValue, _: &mlua::Lua) -> mlua::Result<Self> { Ok(Custom) }
    /// }
    /// let rt = MissionRuntime::from_source("test.lua", "", 1);
    /// let _: Custom = rt.eval("return 1").unwrap();
    /// ```
    pub fn eval<T: EvalResult>(&self, chunk: &str) -> mlua::Result<T> {
        crate::sandbox::refill(&self.inner.lua);
        self.inner.lua.load(chunk).eval()
    }

    /// Call a chunk with sealed arguments, avoiding both string interpolation
    /// and exported Lua Function handles. Lua receives arguments through `...`.
    ///
    /// ```compile_fail
    /// use sta_mission::runtime::MissionRuntime;
    /// struct Custom;
    /// impl mlua::IntoLuaMulti for Custom {
    ///     fn into_lua_multi(self, _: &mlua::Lua) -> mlua::Result<mlua::MultiValue> {
    ///         Ok(mlua::MultiValue::new())
    ///     }
    /// }
    /// let rt = MissionRuntime::from_source("test.lua", "", 1);
    /// rt.eval_with::<(), _>("", Custom).unwrap();
    /// ```
    pub fn eval_with<T: EvalResult, A: EvalArgs>(&self, chunk: &str, args: A) -> mlua::Result<T> {
        crate::sandbox::refill(&self.inner.lua);
        self.inner.lua.load(chunk).call(args)
    }

    /// The most VM instructions any one host entry (load, callback, monitor
    /// pass) has used, against `sandbox::INSTRUCTION_BUDGET`.
    pub fn instruction_peak(&self) -> u64 {
        crate::sandbox::refill(&self.inner.lua);
        crate::sandbox::peak_used(&self.inner.lua)
    }

    /// Errors so far: load errors and callback errors, oldest first.
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// Callbacks registered and not disabled.
    pub fn active_callbacks(&self) -> usize {
        self.inner.callbacks.iter().filter(|c| !c.disabled).count()
    }

    /// Commands the script has queued since the last call.
    pub fn take_commands(&mut self) -> Vec<Command> {
        api::drain_commands(&self.inner.ctx)
    }

    /// The objectives the script added, in order.
    pub fn objectives(&self) -> Vec<Objective> {
        self.inner.ctx.borrow().objectives.clone()
    }

    /// File-backed heading, briefing and notes for inspection. Like the goal
    /// list, this is host mission state, not yet a replicated UI presentation.
    pub fn objective_briefing(&self) -> Option<api::ObjectiveBriefing> {
        self.inner.ctx.borrow().objective_briefing.clone()
    }

    /// Whether the script called `Mission.finish`.
    pub fn finished(&self) -> bool {
        self.inner.ctx.borrow().finished
    }

    /// What the script `print`ed.
    pub fn log(&self) -> Vec<String> {
        self.inner.ctx.borrow().log.clone()
    }

    /// The state of the mission end the script asked for with `SucceedMission`
    /// / `FailMission` (A2), as of `tick` (LUA-13).
    pub fn mission_end(&self, tick: u64) -> MissionEndState {
        let c = self.inner.ctx.borrow();
        match &c.mission_end {
            None => MissionEndState::None,
            Some(e) if e.blocked.is_some() => MissionEndState::Blocked(e.blocked.clone().unwrap_or_default()),
            Some(e) if tick as f64 / sta_core::TICK_HZ as f64 > e.time => MissionEndState::Ended { won: !e.failed },
            Some(_) => MissionEndState::Pending,
        }
    }

    /// AIP loads (`AI_Team_Load_AIP`, `Player:loadAIP`) asked for since the
    /// last call, in call order, with the seat they name (AI-06).
    pub fn take_aip_requests(&self) -> Vec<(sta_core::PlayerId, String)> {
        let mut c = self.inner.ctx.borrow_mut();
        let out = c.loaded_aips[c.aips_taken..].to_vec();
        c.aips_taken = c.loaded_aips.len();
        out
    }

    /// Unsupported API encounters, with counts ascending by name. Includes
    /// accepted operations with no effect and permissive fallback lookups
    /// (`stub global <name>` / `stub field <path>`). Lookup counts are not
    /// function-call counts. Lookup keys may be dynamically computed strings;
    /// function arguments are not inspected.
    ///
    /// AIP loads no host has taken ([`MissionRuntime::take_aip_requests`]) are
    /// listed as `Player.loadAIP`: nothing has applied them.
    pub fn unsupported(&self) -> Vec<(String, u32)> {
        let c = self.inner.ctx.borrow();
        let mut out: Vec<(String, u32)> = c.unsupported.iter().map(|(k, v)| (k.clone(), *v)).collect();
        let pending = c.loaded_aips.len() - c.aips_taken;
        if pending > 0 {
            let at = out.partition_point(|(k, _)| k.as_str() < "Player.loadAIP");
            out.insert(at, ("Player.loadAIP".to_owned(), u32::try_from(pending).unwrap_or(u32::MAX)));
        }
        out
    }

    /// How `from` stands toward `to`, with the script's overrides applied
    /// (`player:setRelation`).
    pub fn relation(&self, from: sta_core::PlayerId, to: sta_core::PlayerId) -> sta_sim::view::Relation {
        self.inner.ctx.borrow().relation(from, to)
    }

    /// Run every live callback once for `tick`.
    pub fn step(&mut self, tick: u64) {
        // `step` is public as well as called by `think`: clock-based API must
        // see the tick supplied here even without a new world snapshot.
        self.inner.ctx.borrow_mut().view.tick = tick;
        {
            let mut s = self.inner.shared.borrow_mut();
            let added = std::mem::take(&mut s.added);
            let removed = std::mem::take(&mut s.removed);
            drop(s);
            for (id, func) in added {
                self.inner.callbacks.push(Callback { id, func, errors: 0, disabled: false });
            }
            self.inner.callbacks.retain(|c| !removed.contains(&c.id));
        }
        let time = tick as f64 / TICK_HZ as f64;
        let mut failures = Vec::new();
        crate::sandbox::refill(&self.inner.lua);
        api::fire_monitors(&self.inner.lua, &self.inner.ctx, &mut failures);
        for cb in self.inner.callbacks.iter_mut().filter(|c| !c.disabled) {
            // A Lua error is a Result; catch_unwind also guards a panic in
            // a Rust function installed into the state.
            crate::sandbox::refill(&self.inner.lua);
            let call = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cb.func.call::<MultiValue>(time)));
            let err = match call {
                Ok(Ok(_)) => continue,
                Ok(Err(e)) => e.to_string(),
                Err(_) => "panic in callback".to_owned(),
            };
            cb.errors += 1;
            let tail = if cb.errors >= MAX_CALLBACK_ERRORS {
                cb.disabled = true;
                " (disabled)"
            } else {
                ""
            };
            failures.push(format!("callback {} at tick {tick}: {err}{tail}", cb.id));
        }
        let room = MAX_LOGGED.saturating_sub(self.errors.len());
        self.errors.extend(failures.into_iter().take(room));
    }
}

#[cfg(test)]
mod object_tests;

impl Controller for MissionRuntime {
    fn think(&mut self, view: &WorldView, events: &[SimEvent], out: &mut Vec<Command>) {
        let mut failures;
        {
            let mut c = self.inner.ctx.borrow_mut();
            c.view = view.clone();
            c.events = events.to_vec();
            let tick = view.tick;
            let ids: Vec<_> = view.units.iter().map(|u| u.id).collect();
            c.seen.extend(ids);
            for e in events {
                if let SimEvent::Hit { target, .. } = e {
                    c.last_hit.insert(*target, tick);
                }
            }
            failures = api::process_edit_feedback(&mut c);
            api::process_spawns(&mut c);
        }
        crate::sandbox::refill(&self.inner.lua);
        api::fire_text_input(&self.inner.lua, &self.inner.ctx, &mut failures);
        let room = MAX_LOGGED.saturating_sub(self.errors.len());
        self.errors.extend(failures.into_iter().take(room));
        self.step(view.tick);
        out.extend(api::drain_commands(&self.inner.ctx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Table;
    use std::path::Path;

    fn view(tick: u64) -> WorldView {
        WorldView { tick, digest: 0, units: Vec::new(), players: Vec::new(), visibility: Vec::new(), perceived_owners: Vec::new() }
    }

    fn run(rt: &mut MissionRuntime, ticks: u64) {
        let mut out = Vec::new();
        for t in 0..ticks {
            rt.think(&view(t), &[], &mut out);
        }
    }

    fn global<T: mlua::FromLua>(rt: &MissionRuntime, name: &str) -> T {
        rt.inner.lua.globals().get(name).unwrap()
    }

    #[test]
    fn callback_runs_once_per_tick() {
        let mut rt = MissionRuntime::from_source("missions/T.lua", "N = 0\nMMM.register(function(t) N = N + 1; LAST = t end)", 1);
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        run(&mut rt, 60);
        assert_eq!(global::<i64>(&rt, "N"), 60);
        assert_eq!(global::<f64>(&rt, "LAST"), 59.0 / TICK_HZ as f64);
    }

    #[test]
    fn erroring_callback_is_isolated() {
        let mut rt = MissionRuntime::from_source(
            "missions/T.lua",
            "GOOD = 0\nMMM.register(function(t) if math.floor(t * 30 + 0.5) == 5 then error('boom') end end)\n\
             MMM.register(function(t) GOOD = GOOD + 1 end)",
            1,
        );
        run(&mut rt, 20);
        assert_eq!(global::<i64>(&rt, "GOOD"), 20);
        assert_eq!(rt.errors().len(), 1, "{:?}", rt.errors());
        assert!(rt.errors()[0].contains("tick 5") && rt.errors()[0].contains("boom"));
        assert_eq!(rt.active_callbacks(), 2);
    }

    #[test]
    fn repeat_offender_is_disabled() {
        let mut rt = MissionRuntime::from_source("missions/T.lua", "MMM.register(function() error('x') end)", 1);
        run(&mut rt, 50);
        assert_eq!(rt.errors().len(), MAX_CALLBACK_ERRORS as usize);
        assert_eq!(rt.active_callbacks(), 0);
    }

    #[test]
    fn load_errors_are_kept() {
        let mut rt = MissionRuntime::from_source("missions/T.lua", "if then end", 1);
        assert_eq!(rt.errors().len(), 1);
        run(&mut rt, 3);
        let mut missing = MissionRuntime::load("missions/Nope.lua", 1, |_| None);
        assert_eq!(missing.errors().len(), 1);
        run(&mut missing, 3);
    }

    fn randoms(seed: u64) -> Vec<f64> {
        let mut rt = MissionRuntime::from_source(
            "missions/T.lua",
            "R = {}\nMMM.register(function() R[#R+1] = math.random() + math.random(1, 1000) end)",
            seed,
        );
        run(&mut rt, 10);
        let r: Table = global(&rt, "R");
        r.sequence_values::<f64>().map(Result::unwrap).collect()
    }

    #[test]
    fn same_seed_same_random_sequence() {
        assert_eq!(randoms(42), randoms(42));
        assert_ne!(randoms(42), randoms(43));
    }

    #[test]
    fn unknown_api_is_stubbed() {
        let mut rt = MissionRuntime::from_source(
            "missions/T.lua",
            "local e = EntityFinder():find('x')\nlocal m = Monitor(); m:hook(State.Started)\nMMM.other.thing()\n\
             MMM.register(function() local _ = Game.time + 1 end)",
            1,
        );
        run(&mut rt, 3);
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    }

    /// With STA64_TEST_INSTALL / STA64_TEST_MOD: every installed mission
    /// loads and survives 300 ticks.
    #[test]
    fn real_missions_if_available() {
        let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else { return };
        let vfs = Arc::new(Vfs::mount_install(Path::new(&root), Some(&m)).unwrap());
        let scripts: Vec<String> = vfs.list("missions").into_iter().filter(|p| p.ends_with(".lua")).collect();
        let (mut load_errors, mut cb_errors, mut callbacks) = (0, 0, 0);
        for p in &scripts {
            let mut rt = MissionRuntime::from_vfs(vfs.clone(), p, 1);
            let before = rt.errors().len();
            run(&mut rt, 300);
            load_errors += before;
            cb_errors += rt.errors().len() - before;
            callbacks += rt.inner.callbacks.len();
            if let Some(e) = rt.errors().first() {
                eprintln!("  {p}: {e}");
            }
        }
        eprintln!("{} scripts, {load_errors} load errors, {callbacks} callbacks, {cb_errors} callback errors", scripts.len());
        assert!(!scripts.is_empty());
    }
}
