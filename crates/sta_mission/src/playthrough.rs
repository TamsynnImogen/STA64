//! Mission playthrough driver (LUA-17): one campaign mission run headless in the
//! campaign runner, with a scripted player, recording a timeline.
//!
//! The player is a *plan*: a Lua chunk run as a coroutine in its own Lua state
//! (not the mission's sandbox), resumed every [`PLAN_EVERY`] ticks. It sees the
//! world as a read-only snapshot (`VIEW`) and gives orders only through the
//! normal command interface (`move`, `attack`, `build`, ... become
//! [`Order`]s from the human seat); it never reads or writes mission variables.
//! The [`StandInHost`] answers the client's completion reports (audio, movies,
//! Enter), as the converted-mission probe does.
//!
//! The timeline (one [`Event`] per line, with tick and sim seconds) holds:
//! - `call`: each host call the script makes on `A2` / `A1` (name, arguments,
//!   result). A call with the same text as one already logged in the last
//!   [`CALL_REPEAT_TICKS`] is counted, not logged again (conditions poll every tick);
//!   calls made while the script loaded are not traced (their counts are in `load`); the
//!   same rule thins `step` (a satisfied rule runs its action every tick) and untyped
//!   `present` calls;
//! - `present`: every presentation request (sound, music, camera, title, fade,
//!   cineractive on/off, movie, untyped calls by name), and `camera`: the camera
//!   pose the cineractive model computes, sampled every second while it moves,
//!   and `camera-ignored` when a camera request left the model unchanged;
//! - `rule` / `step` (converted missions): rules becoming satisfied and steps run;
//! - `objective`, `briefing`: objectives issued and changing state, the briefing file;
//! - `unbound`, `raised`, `unsupported`, `error`, `step-error`: what did not work;
//! - `order`, `plan`, `plan-error`: what the scripted player did and said;
//! - `spawned`, `destroyed`, `eliminated`: world changes; `end`: the outcome.
//!
//! Evidence grade: measured (a headless run); presentation is what the script
//! *requested*, not what a client showed.

use std::collections::HashMap;

use mlua::{Function, Lua, Table, Thread};
use mlua::thread::ThreadStatus;
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, Vec3};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::present::Presentation;
use sta_sim::{SimEvent, WorldSource};

use crate::campaign::*;
use crate::readiness::json_string;
use crate::standin::StandInHost;

/// Ticks between two resumes of the plan (half a second).
pub const PLAN_EVERY: u64 = 15;
/// A call logged again only after this many ticks with no identical call.
pub const CALL_REPEAT_TICKS: u64 = 900;

/// The human seat the plan plays.
const SEAT: PlayerId = PlayerId(0);

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub tick: u64,
    pub kind: &'static str,
    pub what: String,
}

impl Event {
    pub fn secs(&self) -> f64 {
        self.tick as f64 / f64::from(TICK_HZ)
    }

    pub fn to_json(&self) -> String {
        format!("{{\"t\":{:.2},\"tick\":{},\"kind\":{},\"what\":{}}}", self.secs(), self.tick, json_string(self.kind), json_string(&self.what))
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub ticks: u64,
    /// The plan's Lua source; `None` plays no input.
    pub plan: Option<String>,
    pub seed: u64,
}

#[derive(Debug, Default)]
pub struct Playthrough {
    pub events: Vec<Event>,
    pub outcome: Option<MissionOutcome>,
    pub ticks: u64,
}

impl Playthrough {
    pub fn of(&self, kind: &str) -> impl Iterator<Item = &Event> {
        let kind = kind.to_owned();
        self.events.iter().filter(move |e| e.kind == kind)
    }
}

/// Wraps every function of `A2` and `A1` so each call is appended to `__sta64_trace`.
/// The host's own plumbing (the stand-in's reports, waits, gap notes) is left out.
const TRACE_HOOK: &str = r##"
local log = {}
rawset(_G, "__sta64_trace", log)
local skip = { HostPending = true, ScriptWaits = true, EngineVariable = true, NoteGap = true }
local function fmt(v)
  local t = type(v)
  if t == "string" then return string.format("%q", v) end
  if t == "number" then
    if math.type(v) == "float" then return string.format("%.3f", v) end
    return tostring(v)
  end
  if t == "table" then return "{}" end
  return tostring(v)
end
for _, tn in ipairs({ "A2", "A1" }) do
  local t = rawget(_G, tn)
  if type(t) == "table" then
    local names = {}
    for k, f in pairs(t) do
      if type(k) == "string" and type(f) == "function" and not skip[k] then names[#names + 1] = k end
    end
    for _, k in ipairs(names) do
      local f = rawget(t, k)
      rawset(t, k, function(...)
        local n = select("#", ...)
        local parts = {}
        for i = 1, math.min(n, 10) do parts[i] = fmt((select(i, ...))) end
        local r = table.pack(f(...))
        local rs = r.n > 0 and (" -> " .. fmt(r[1])) or ""
        log[#log + 1] = tn .. "." .. k .. "(" .. table.concat(parts, ", ") .. ")" .. rs
        return table.unpack(r, 1, r.n)
      end)
    end
  end
end
"##;

const DRAIN: &str = r#"
local o = {}
local t = rawget(_G, "__sta64_trace")
if t then for i = 1, #t do o[i] = t[i] t[i] = nil end end
return o
"#;

/// Rules fired, steps run and problems of a converted script since the given counts.
const CONVERTED: &str = r#"
local f, r, p, e = ...
local M = rawget(_G, "Mission_")
local out = {}
if type(M) ~= "table" then return out, f, r, p, e end
local fired, ran, prob, errs = M.fired or {}, M.ran or {}, M.problemOrder or {}, M.stepErrors or {}
for i = f + 1, #fired do out[#out + 1] = "rule\t" .. tostring(fired[i]) end
for i = r + 1, #ran do out[#out + 1] = "step\t" .. tostring(ran[i]) end
for i = p + 1, #prob do local q = prob[i] out[#out + 1] = tostring(q[2]) .. "\t" .. tostring(q[1]) .. (q[4] and (": " .. tostring(q[4])) or "") end
for i = e + 1, #errs do out[#out + 1] = "step-error\t" .. tostring(errs[i]) end
return out, #fired, #ran, #prob, #errs
"#;

const PLAN_PRELUDE: &str = r#"
__orders = {}
local function push(o) __orders[#__orders + 1] = o end
local function ids(u)
  if type(u) == "number" then return { u } end
  if u.id then return { u.id } end
  local o = {}
  for i, x in ipairs(u) do o[i] = type(x) == "table" and x.id or x end
  return o
end
function now() return VIEW.t end
function move(u, x, z) push({ kind = "move", ids = ids(u), x = x, z = z }) end
function attack(u, target) push({ kind = "attack", ids = ids(u), target = type(target) == "table" and target.id or target }) end
function stop(u) push({ kind = "stop", ids = ids(u) }) end
function build(builder, odf, x, z) push({ kind = "build", builder = type(builder) == "table" and builder.id or builder, odf = odf, x = x, z = z }) end
function mine(u, node) push({ kind = "mine", ids = ids(u), node = type(node) == "table" and node.id or node }) end
function note(s) push({ kind = "note", text = tostring(s) }) end
-- units{ owner = n, odf = "exact", like = "substring", mine = true, enemy = true, alive = true (default) }
function units(f)
  f = f or {}
  local o = {}
  for _, u in ipairs(VIEW.units) do
    local ok = (f.owner == nil or u.owner == f.owner)
      and (f.odf == nil or u.odf == f.odf)
      and (f.like == nil or string.find(u.odf, f.like, 1, true) ~= nil)
      and (not f.mine or u.owner == VIEW.seat)
      and (not f.enemy or VIEW.enemy[u.owner] == true)
      and (f.alive == false or not u.dying)
    if ok then o[#o + 1] = u end
  end
  return o
end
function unit(id) for _, u in ipairs(VIEW.units) do if u.id == id then return u end end end
function label(name) local id = VIEW.labels[name] return id and unit(id) end
function dist(a, b) local dx, dz = a.x - b.x, a.z - b.z return math.sqrt(dx * dx + dz * dz) end
function nearest(from, list) local best, bd for _, u in ipairs(list) do local d = dist(from, u) if not bd or d < bd then best, bd = u, d end end return best, bd end
-- the player has control: no cineractive runs and the script has had its first update
function control() return VIEW.tick > 1 and not VIEW.cineractive end
function objectives() return VIEW.objectives end
function wait(s) local t = VIEW.t + s while VIEW.t < t do coroutine.yield() end end
function wait_until(f, timeout)
  local t0 = VIEW.t
  while not f() do
    if timeout and VIEW.t - t0 > timeout then note("wait_until timed out after " .. timeout .. " s") return false end
    coroutine.yield()
  end
  return true
end
"#;

struct Plan {
    lua: Lua,
    thread: Option<Thread>,
    seq: u32,
    /// Orders sent, by sequence number, for the `rejected` entries.
    sent: HashMap<u32, String>,
}

fn pos(v: &Vec3) -> String {
    format!("({:.0}, {:.0})", f(v.x), f(v.z))
}

fn describe(o: &Order, host: &VfsHost) -> String {
    let ids = |u: &[EntityId]| u.iter().map(|i| format!("#{}", i.0)).collect::<Vec<_>>().join(",");
    match o {
        Order::Move { units, to, .. } => format!("move {} to {}", ids(units), pos(to)),
        Order::Stop { units } => format!("stop {}", ids(units)),
        Order::Attack { units, target } => format!("attack {} -> #{}", ids(units), target.0),
        Order::Mine { units, node } => format!("mine {} at #{}", ids(units), node.0),
        Order::Build { builder, what, at } => format!(
            "build {} by #{}{}",
            host.content.name(*what).unwrap_or("?"),
            builder.0,
            at.as_ref().map_or(String::new(), |p| format!(" at {}", pos(p)))
        ),
        other => format!("{other:?}"),
    }
}

fn f(v: Fixed) -> f64 {
    f64::from(v.to_f32())
}

impl Plan {
    fn new(src: &str, paths: &[(String, Vec<[f32; 3]>)]) -> mlua::Result<Plan> {
        let lua = Lua::new();
        lua.load(PLAN_PRELUDE).set_name("plan prelude").exec()?;
        let pt = lua.create_table()?;
        for (name, pts) in paths {
            let list = lua.create_table()?;
            for (i, p) in pts.iter().enumerate() {
                let q = lua.create_table()?;
                q.set("x", p[0])?;
                q.set("z", p[2])?;
                list.set(i + 1, q)?;
            }
            pt.set(name.as_str(), list)?;
        }
        lua.globals().set("paths", pt)?;
        let main: Function = lua.load(src).set_name("plan").into_function()?;
        let thread = lua.create_thread(main)?;
        Ok(Plan { lua, thread: Some(thread), seq: 0, sent: HashMap::new() })
    }

    fn view(&self, a: &ActiveMission, host: &VfsHost, cine: bool) -> mlua::Result<()> {
        let lua = &self.lua;
        let v = a.sim().view();
        let t = lua.create_table()?;
        t.set("t", v.tick as f64 / f64::from(TICK_HZ))?;
        t.set("tick", v.tick)?;
        t.set("seat", SEAT.0)?;
        t.set("cineractive", cine)?;
        let units = lua.create_table()?;
        for (i, u) in v.units.iter().enumerate() {
            let r = lua.create_table()?;
            r.set("id", u.id.0)?;
            r.set("odf", host.content.name(u.def).unwrap_or(""))?;
            r.set("owner", u.owner.0)?;
            r.set("x", f(u.pos.x))?;
            r.set("y", f(u.pos.y))?;
            r.set("z", f(u.pos.z))?;
            r.set("hull", f(u.hull[0]))?;
            r.set("maxhull", f(u.hull[1]))?;
            r.set("dying", u.flags.0 & sta_sim::view::UnitFlags::DYING.0 != 0)?;
            r.set("building", u.progress.is_some_and(|p| p < Fixed::ONE))?;
            units.set(i + 1, r)?;
        }
        t.set("units", units)?;
        let enemy = lua.create_table()?;
        let res = lua.create_table()?;
        if let Some(p) = v.player(SEAT) {
            for (other, rel) in &p.relations {
                enemy.set(other.0, matches!(rel, sta_sim::view::Relation::Enemy))?;
            }
            for (i, r) in p.resources.iter().enumerate() {
                res.set(i, f(*r))?;
            }
        }
        t.set("enemy", enemy)?;
        t.set("resources", res)?;
        let labels = lua.create_table()?;
        for (id, l) in a.sim().labels.iter() {
            labels.set(l.as_str(), id.0)?;
        }
        t.set("labels", labels)?;
        let objs = lua.create_table()?;
        for (i, o) in a.runtime().objectives().iter().enumerate() {
            let r = lua.create_table()?;
            r.set("id", o.id)?;
            r.set("title", o.title.as_str())?;
            r.set("state", format!("{:?}", o.state))?;
            objs.set(i + 1, r)?;
        }
        t.set("objectives", objs)?;
        lua.globals().set("VIEW", t)
    }

    /// Resume the plan once; its orders become commands, its notes events.
    fn resume(&mut self, a: &mut ActiveMission, host: &VfsHost, cine: bool, tick: u64, out: &mut Vec<Event>) {
        let Some(th) = self.thread.clone() else { return };
        if let Err(e) = self.view(a, host, cine) {
            out.push(Event { tick, kind: "plan-error", what: format!("view: {e}") });
            self.thread = None;
            return;
        }
        if let Err(e) = th.resume::<()>(()) {
            out.push(Event { tick, kind: "plan-error", what: e.to_string().lines().next().unwrap_or("").to_owned() });
            self.thread = None;
        } else if th.status() != ThreadStatus::Resumable {
            out.push(Event { tick, kind: "plan", what: "plan finished".into() });
            self.thread = None;
        }
        let orders: Table = match self.lua.globals().get("__orders") {
            Ok(t) => t,
            Err(_) => return,
        };
        let mut cmds = Vec::new();
        for o in orders.sequence_values::<Table>().flatten() {
            let kind: String = o.get("kind").unwrap_or_default();
            let idl = |o: &Table| -> Vec<EntityId> {
                o.get::<Vec<u32>>("ids").unwrap_or_default().into_iter().map(EntityId).collect()
            };
            let at = |o: &Table| -> Option<Vec3> {
                let x: Option<f64> = o.get("x").ok()?;
                let z: Option<f64> = o.get("z").ok()?;
                Some(Vec3 { x: Fixed::from_f64(x?), y: Fixed::ZERO, z: Fixed::from_f64(z?) })
            };
            let order = match kind.as_str() {
                "note" => {
                    out.push(Event { tick, kind: "plan", what: o.get("text").unwrap_or_default() });
                    continue;
                }
                "move" => at(&o).map(|to| Order::Move { units: idl(&o), to, queue: false }),
                "stop" => Some(Order::Stop { units: idl(&o) }),
                "attack" => o.get::<u32>("target").ok().map(|t| Order::Attack { units: idl(&o), target: EntityId(t) }),
                "mine" => o.get::<u32>("node").ok().map(|n| Order::Mine { units: idl(&o), node: EntityId(n) }),
                "build" => {
                    let odf: String = o.get("odf").unwrap_or_default();
                    match (host.content.key(&odf), o.get::<u32>("builder")) {
                        (Some(what), Ok(b)) => Some(Order::Build { builder: EntityId(b), what, at: at(&o) }),
                        _ => {
                            out.push(Event { tick, kind: "plan-error", what: format!("build: no unit {odf:?} or no builder") });
                            None
                        }
                    }
                }
                _ => None,
            };
            if let Some(order) = order {
                self.seq += 1;
                let text = describe(&order, host);
                out.push(Event { tick, kind: "order", what: format!("#{} {text}", self.seq) });
                self.sent.insert(self.seq, text);
                cmds.push(Command { player: SEAT, seq: self.seq, kind: CommandKind::Order(order) });
            }
        }
        let _ = self.lua.globals().set("__orders", self.lua.create_table().ok());
        a.submit(cmds);
    }
}

/// Is `key` new, or last seen at least [`CALL_REPEAT_TICKS`] ago? Records it as seen now.
fn fresh_repeat(seen: &mut HashMap<String, u64>, key: &str, tick: u64) -> bool {
    let fresh = seen.get(key).is_none_or(|&t| tick.saturating_sub(t) >= CALL_REPEAT_TICKS);
    seen.insert(key.to_owned(), tick);
    fresh
}

fn is_camera(p: &Presentation) -> bool {
    match p {
        Presentation::Shot { .. } | Presentation::CameraOffset { .. } => true,
        Presentation::Call { name, .. } => name.contains("Cineractive") || name.contains("Camera") || name.starts_with("Cineractive."),
        _ => false,
    }
}

fn round3(v: [f64; 3]) -> String {
    format!("({:.0}, {:.0}, {:.0})", v[0], v[1], v[2])
}

/// Play `campaign`'s mission `index` with `cfg`, recording the timeline.
pub fn play(host: &VfsHost, book: &CampaignBook, campaign: &str, index: usize, cfg: &Config) -> Result<Playthrough, String> {
    let m = book.campaigns.iter().find(|c| c.id == campaign).ok_or("no such campaign")?.missions.get(index).ok_or("no such mission")?.clone();
    let mut open = book.clone();
    open.give_all_missions = true;
    let mut progress = CampaignProgress::new(&open);
    progress.apply_give_all(&open);
    let mut runner = CampaignRunner::with_progress(open, progress);
    let a = runner.start(host, campaign, index, cfg.seed).map_err(|e| e.to_string())?;
    let paths = read_map_paths(&host.vfs, &m.bzn);
    let mut ev = Vec::new();
    ev.push(Event { tick: a.sim().tick, kind: "start", what: format!("{} ({}) script {:?}", m.bzn, m.title.as_deref().unwrap_or("-"), m.script) });
    let load_calls: Vec<String> = a
        .runtime()
        .eval("local o = {} local M = rawget(_G, 'Mission_') if M then for k, n in pairs(M.calls) do o[#o + 1] = k .. '=' .. n end end table.sort(o) return o")
        .unwrap_or_default();
    if !load_calls.is_empty() {
        ev.push(Event { tick: a.sim().tick, kind: "load", what: load_calls.join(" ") });
    }
    if let Err(e) = a.runtime().eval::<()>(TRACE_HOOK) {
        ev.push(Event { tick: 0, kind: "error", what: format!("trace hook: {e}") });
    }
    let mut plan = match cfg.plan.as_deref().map(|s| Plan::new(s, &paths)) {
        Some(Ok(p)) => Some(p),
        Some(Err(e)) => {
            ev.push(Event { tick: 0, kind: "plan-error", what: e.to_string() });
            None
        }
        None => None,
    };
    let mut standin = StandInHost::new(host.vfs.clone());
    let mut seen: HashMap<String, u64> = HashMap::new();
    let mut counts = (0i64, 0i64, 0i64, 0i64);
    let mut errs = 0;
    let mut unsupported: HashMap<String, u32> = HashMap::new();
    let mut objectives: HashMap<u32, String> = HashMap::new();
    let mut briefing: Option<String> = None;
    let mut owners: HashMap<EntityId, String> = HashMap::new();
    let mut last_pose: Option<([f64; 3], [f64; 3])> = None;
    let mut outcome = None;
    for _ in 0..cfg.ticks {
        let tick = a.sim().tick;
        let cine_before = a.runtime().cineractive();
        if let Some(p) = plan.as_mut()
            && tick % PLAN_EVERY == 0
        {
            p.resume(a, host, cine_before.active, tick, &mut ev);
        }
        outcome = a.tick();
        standin.after_tick(a);
        let tick = a.sim().tick;
        // host calls
        let calls: Vec<String> = a.runtime().eval(DRAIN).unwrap_or_default();
        for c in calls {
            // a float result changes every poll; the call is the same call
            let key = match c.rsplit_once(" -> ") {
                Some((head, r)) if r.contains('.') => head.to_owned(),
                _ => c.clone(),
            };
            if fresh_repeat(&mut seen, &key, tick) {
                ev.push(Event { tick, kind: "call", what: c });
            }
        }
        // converted-script progress
        if let Ok((lines, f, r, p, e)) = a.runtime().eval_with::<(Vec<String>, i64, i64, i64, i64), _>(CONVERTED, counts) {
            counts = (f, r, p, e);
            for l in lines {
                if let Some((k, w)) = l.split_once('\t') {
                    let kind = match k {
                        "rule" => "rule",
                        "step" => "step",
                        "unbound" => "unbound",
                        "raised" => "raised",
                        _ => "step-error",
                    };
                    // a rule that stays satisfied runs its action every tick
                    if kind == "step" && !fresh_repeat(&mut seen, &format!("step {w}"), tick) {
                        continue;
                    }
                    ev.push(Event { tick, kind, what: w.to_owned() });
                }
            }
        }
        // world and presentation
        let cine = a.runtime().cineractive();
        for e in a.last_events() {
            match e {
                SimEvent::Present(p) => {
                    let what = format!("{p:?}");
                    if matches!(p, Presentation::Call { .. }) && !fresh_repeat(&mut seen, &what, tick) {
                        continue;
                    }
                    ev.push(Event { tick, kind: "present", what });
                    if is_camera(p) && cine.active && cine.shot == cine_before.shot && cine.shot_placed == cine_before.shot_placed {
                        ev.push(Event { tick, kind: "camera-ignored", what: format!("{p:?}") });
                    }
                }
                SimEvent::Spawned { id } => {
                    if let Some(u) = a.sim().view().unit(*id) {
                        let name = host.content.name(u.def).unwrap_or("?").to_owned();
                        ev.push(Event { tick, kind: "spawned", what: format!("p{} {name} #{}", u.owner.0, id.0) });
                    }
                }
                SimEvent::Destroyed { id, .. } => {
                    let who = owners.get(id).cloned().unwrap_or_else(|| "?".into());
                    ev.push(Event { tick, kind: "destroyed", what: format!("{who} #{}", id.0) });
                }
                SimEvent::Rejected { player, seq } if *player == SEAT => {
                    let what = plan.as_ref().and_then(|p| p.sent.get(seq)).cloned().unwrap_or_default();
                    ev.push(Event { tick, kind: "rejected", what: format!("#{seq} {what}") });
                }
                SimEvent::PlayerEliminated { player } => ev.push(Event { tick, kind: "eliminated", what: format!("p{}", player.0) }),
                _ => {}
            }
        }
        for u in &a.sim().view().units {
            owners.entry(u.id).or_insert_with(|| format!("p{} {}", u.owner.0, host.content.name(u.def).unwrap_or("?")));
        }
        if cine.active && tick % u64::from(TICK_HZ) == 0 {
            let v = a.sim().view();
            let locate = |id: EntityId| {
                v.unit(id).map(|u| {
                    ([f(u.pos.x), f(u.pos.y), f(u.pos.z)], [f(u.heading.0), 0.0, f(u.heading.1)])
                })
            };
            let pose = cine.pose(tick as f64, &locate);
            let moved = match (pose, last_pose) {
                (Some((e, l)), Some((pe, pl))) => (0..3).any(|i| (e[i] - pe[i]).abs() > 5.0 || (l[i] - pl[i]).abs() > 5.0),
                (a, b) => a.is_some() != b.is_some(),
            };
            if moved {
                let what = pose.map_or("none (holding)".into(), |(e, l)| format!("eye {} look {}", round3(e), round3(l)));
                ev.push(Event { tick, kind: "camera", what });
            }
            last_pose = pose;
        } else if !cine.active {
            last_pose = None;
        }
        // objectives and briefing
        for o in a.runtime().objectives() {
            let s = format!("{:?}", o.state);
            if objectives.get(&o.id) != Some(&s) {
                let new = !objectives.contains_key(&o.id);
                ev.push(Event { tick, kind: "objective", what: format!("{} #{} {s} {:?}", if new { "issued" } else { "now" }, o.id, o.title) });
                objectives.insert(o.id, s);
            }
        }
        let b = a.runtime().objective_briefing().map(|b| b.file);
        if b != briefing {
            ev.push(Event { tick, kind: "briefing", what: b.clone().unwrap_or_else(|| "none".into()) });
            briefing = b;
        }
        // failures
        let e = a.runtime().errors();
        while errs < e.len() {
            ev.push(Event { tick, kind: "error", what: e[errs].lines().next().unwrap_or("").to_owned() });
            errs += 1;
        }
        if tick % u64::from(TICK_HZ) == 0 || outcome.is_some() {
            for (k, n) in a.runtime().unsupported() {
                if unsupported.insert(k.clone(), n).is_none() {
                    ev.push(Event { tick, kind: "unsupported", what: k });
                }
            }
        }
        if let Some(o) = outcome {
            ev.push(Event { tick, kind: "end", what: format!("{} (script delay {:.1} s)", if o.won { "won" } else { "lost" }, o.delay) });
            break;
        }
    }
    if outcome.is_none()
        && let crate::runtime::MissionEndState::Blocked(why) = a.runtime().mission_end(a.sim().tick)
    {
        ev.push(Event { tick: a.sim().tick, kind: "end", what: format!("blocked: {why}") });
    }
    Ok(Playthrough { events: ev, outcome, ticks: a.sim().tick })
}
