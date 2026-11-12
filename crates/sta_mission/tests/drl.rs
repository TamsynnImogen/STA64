//! LUA-07: Lua scaffolds generated from Armada II rule DLLs by
//! `tools/drl_to_lua.py`.
//!
//! - The asset-free test runs `SYNTHETIC_SCAFFOLD`, the tool's output for a
//!   synthetic three-rule mission (`tools/test_drl_to_lua.py` checks that the
//!   two stay identical; it prints a new one with `--print-fixture`).
//! - The install-gated test runs the tool on `STA64_A2_INSTALL` (an Armada II
//!   root with `missions/a2_fed01R.drl`), loads the generated `a2_fed01`
//!   scaffold and checks that every decoded rule is registered. It is skipped
//!   when the variable is not set. Needs `python3` and `objdump`; the scaffold
//!   is written under the target directory and never committed.

use sta_mission::runtime::MissionRuntime;
use sta_sim::WorldView;
use sta_sim::command::Command;
use sta_sim::setup::Controller;

/// Capture the registered mission object as `__drl`, as the port tests do.
fn load(name: &str, source: &str) -> MissionRuntime {
    let observed = format!(
        "local register = MMM.register\n\
         MMM.register = function(object) __drl = object; return register(object) end\n\
         {source}"
    );
    let rt = MissionRuntime::from_source(name, &observed, 7);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

fn tick(rt: &mut MissionRuntime, at: u64) {
    let view = WorldView {
        tick: at,
        ..WorldView::default()
    };
    let mut commands = Vec::<Command>::new();
    rt.think(&view, &[], &mut commands);
}

fn ran(rt: &MissionRuntime) -> Vec<String> {
    rt.eval("return __drl.ran").unwrap()
}

#[test]
fn synthetic_scaffold_registers_rules_and_fires_actions() {
    let mut rt = load("missions/synthetic01.lua", SYNTHETIC_SCAFFOLD);
    tick(&mut rt, 0);
    assert_eq!(rt.eval::<u32>("return #__drl.ruleMonitors").unwrap(), 3);
    let names: Vec<String> = rt
        .eval("local t = {} for i, m in ipairs(__drl.ruleMonitors) do t[i] = m.id end return t")
        .unwrap();
    assert_eq!(names, ["AlphaRule", "TimerRule", "MissingRule"]);
    assert_eq!(
        rt.eval::<i64>("return __drl.state.p_ALWAYS_EVALUATE")
            .unwrap(),
        -1
    );
    assert_eq!(rt.eval::<i64>("return __drl.state.p_ALPHA").unwrap(), 0);

    // Monitors registered during setup start polling on the next tick.
    tick(&mut rt, 1);
    assert!(ran(&rt).is_empty());
    assert!(rt.eval::<u32>("return __drl.missing.NotBoundYet").unwrap() >= 1);

    // AlphaRule: the variable turns true, but IsAlive(nil) fails inside the
    // guard, so the rule does not hold (no answer is not `false`).
    rt.eval::<()>("__drl.state.p_ALPHA = 1").unwrap();
    tick(&mut rt, 2);
    assert!(ran(&rt).is_empty());
    assert!(rt.eval::<u32>("return __drl.failed.IsAlive").unwrap() >= 1);
    // With a handle the view does not hold, IsAlive answers false: the rule
    // holds and its action runs once (edge, not level).
    rt.eval::<()>("__drl.objects.ship = 4242").unwrap();
    tick(&mut rt, 3);
    tick(&mut rt, 4);
    assert_eq!(ran(&rt), ["Do_Alpha"]);

    // TimerRule: a 0.5 s timer reads 1 after it runs out, then fires once.
    rt.eval::<()>("__drl:startTimer('p_tWait', 0.5)").unwrap();
    let ticks = sta_sim::TICK_HZ as u64;
    for at in 5..5 + ticks {
        tick(&mut rt, at);
    }
    assert_eq!(ran(&rt), ["Do_Alpha", "Do_Timer"]);
    assert_eq!(rt.eval::<i64>("return __drl.state.p_tWait").unwrap(), 1);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn installed_a2_fed01_scaffold_loads_and_registers_its_rules() {
    let Ok(install) = std::env::var("STA64_A2_INSTALL") else {
        eprintln!("skipped: set STA64_A2_INSTALL to an Armada II root");
        return;
    };
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("drl_scaffolds");
    let _ = std::fs::remove_dir_all(&out);
    let run = std::process::Command::new("python3")
        .arg(repo.join("tools/drl_to_lua.py"))
        .args(["--install", &install, "--out"])
        .arg(&out)
        .output()
        .expect("run python3 tools/drl_to_lua.py");
    let report = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{report}\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    // "  missions/a2_fed01R.drl: 50 rules, 82 dependencies, 50/50 conditions, ..."
    let line = report
        .lines()
        .find(|l| l.contains("a2_fed01R.drl:"))
        .expect("a2_fed01R.drl decoded");
    // the number in front of `what`; for "50/50 conditions" the decoded (left) one
    let count = |what: &str| -> u32 {
        let at = line.find(what).unwrap();
        let word = line[..at].trim_end().rsplit(' ').next().unwrap();
        word.split('/').next().unwrap().parse().unwrap()
    };
    let rules = count(" rules");
    let decoded = count(" conditions");
    assert!(rules > 0);
    assert_eq!(decoded, rules, "{line}");

    let path = std::fs::read_dir(&out)
        .unwrap()
        .map(|d| d.unwrap().path().join("a2_fed01.lua"))
        .find(|p| p.exists())
        .expect("a2_fed01.lua written");
    let source = std::fs::read_to_string(&path).unwrap();
    let mut rt = load("missions/a2_fed01.lua", &source);
    tick(&mut rt, 0);
    assert_eq!(rt.eval::<u32>("return #__drl.rules").unwrap(), rules);
    assert_eq!(rt.eval::<u32>("return #__drl.ruleMonitors").unwrap(), rules);
    let ids: Vec<String> = rt
        .eval("local t = {} for i, m in ipairs(__drl.ruleMonitors) do t[i] = m.id end return t")
        .unwrap();
    let names: Vec<String> = rt
        .eval("local t = {} for i, r in ipairs(__drl.rules) do t[i] = r.name end return t")
        .unwrap();
    assert_eq!(ids, names);
    // Every decoded condition runs against the empty start state without a
    // Lua error: unbound or failing engine calls are counted by the guard.
    for at in 1..=30 {
        tick(&mut rt, at);
    }
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    eprintln!(
        "a2_fed01: {rules} rules registered; actions run in 30 ticks: {:?}",
        ran(&rt)
    );

    // Every other generated scaffold loads, registers all its rules and runs.
    let dir = path.parent().unwrap();
    let mut loaded = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "lua") {
            continue;
        }
        let name = format!("missions/{}", path.file_name().unwrap().to_string_lossy());
        let mut rt = load(&name, &std::fs::read_to_string(&path).unwrap());
        tick(&mut rt, 0);
        let (rules, monitors): (u32, u32) =
            rt.eval("return #__drl.rules, #__drl.ruleMonitors").unwrap();
        assert!(
            rules > 0 && rules == monitors,
            "{name}: {rules} rules, {monitors} monitors"
        );
        for at in 1..=30 {
            tick(&mut rt, at);
        }
        assert!(rt.errors().is_empty(), "{name}: {:?}", rt.errors());
        loaded += 1;
    }
    assert!(
        report.contains(&format!("{loaded} decoded, 0 failed")),
        "{loaded} scaffolds\n{report}"
    );
}

/// The fixture below must stay the tool's output: run the Python tests
/// (synthetic inputs only) when `python3` is available, as on CI's Ubuntu.
#[test]
fn python_decoder_tests_pass_and_fixture_matches() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let run = match std::process::Command::new("python3")
        .arg("-I")
        .arg(repo.join("tools/test_drl_to_lua.py"))
        .output()
    {
        Ok(run) => run,
        Err(e) => {
            eprintln!("skipped: python3 not available ({e})");
            return;
        }
    };
    assert!(
        run.status.success(),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

/// `python3 tools/test_drl_to_lua.py --print-fixture`
const SYNTHETIC_SCAFFOLD: &str = r#"-- synthetic01: rule scaffold generated by tools/drl_to_lua.py (LUA-07).
-- Rules decoded from synthetic01R.drl; actions, variable values and object handles
-- live in synthetic01S.dsl and are left for the hand port. Regenerate, do not edit
-- the generated tables. Not in the .drl: the engine's rule scheduling. Here each
-- rule is a named condition monitor checked every tick that fires on a
-- false-to-true edge; the stock engine re-evaluates a rule when one of its
-- dependencies is marked dirty (that policy is not reproduced).

local function T(v) return v ~= nil and v ~= false and v ~= 0 end
local function N(v) if v == true then return 1 elseif type(v) ~= 'number' then return 0 end return v end

local Mission = {
    name = 'synthetic01',
    state = {},        -- the .dsl's rule variables, by name
    objects = {},      -- the .dsl's object handles, set by the ported setup
    timers = {},       -- running timer_int deadlines, by name
    rules = {},        -- the decoded rule table, in creation order
    ruleMonitors = {},
    missing = {},      -- engine calls the runtime does not bind yet: name -> count
    failed = {},       -- engine calls that raised an error: name -> count
    callFailed = false, -- set while a condition's engine call is unbound or failed
    time = 0,
}

-- Engine calls of the conditions go to the A2 table by the ScriptInterface
-- method's own name. Unbound names and errors are counted, not raised, so an
-- unported mission still runs; a rule whose engine call gave no answer does
-- not hold that tick.
local function A(name, ...)
    local f = A2[name]
    if type(f) ~= 'function' then
        Mission.missing[name] = (Mission.missing[name] or 0) + 1
        Mission.callFailed = true
        return nil
    end
    local ok, v = pcall(f, ...)
    if not ok then
        Mission.failed[name] = (Mission.failed[name] or 0) + 1
        Mission.callFailed = true
        return nil
    end
    return v
end

-- Variables (3): kind, and whether the engine supplies it.
Mission.variables = {
    { name = 'p_ALPHA', kind = 'hot_int' },
    { name = 'p_ALWAYS_EVALUATE', kind = 'dirty_int', engine = true },
    { name = 'p_tWait', kind = 'timer_int' },
}
Mission.objectNames = { 'ship' }

-- Start values: the .dsl's setup assigns them; the hand port puts them here
-- (unlisted variables start at 0). The engine's ALWAYS_EVALUATE reads -1
-- (measured).
Mission.initial = {
    p_ALWAYS_EVALUATE = -1,
}

-- Rules (3), in the order the .drl creates them.
Mission.rules[1] = {
    name = 'AlphaRule',
    action = 'Do_Alpha',
    deps = { 'p_ALPHA', 'p_ALWAYS_EVALUATE' },
    when = function(S, O) return T(S.p_ALWAYS_EVALUATE) and T(S.p_ALPHA) and not T(A('IsAlive', O.ship)) end,
}
Mission.rules[2] = {
    name = 'TimerRule',
    action = 'Do_Timer',
    deps = { 'p_tWait' },
    when = function(S, O) return T(S.p_tWait) end,
}
Mission.rules[3] = {
    name = 'MissingRule',
    action = 'Do_Missing',
    deps = { 'p_ALWAYS_EVALUATE' },
    when = function(S, O) return N(A('NotBoundYet', 1)) > 0 end,
}

-- Actions (3): the .dsl functions the rules call. Port each body
-- from synthetic01S.dsl; until then it only records that it ran.
Mission.ran = {}
function Mission:Do_Alpha() self.ran[#self.ran + 1] = 'Do_Alpha' end
function Mission:Do_Timer() self.ran[#self.ran + 1] = 'Do_Timer' end
function Mission:Do_Missing() self.ran[#self.ran + 1] = 'Do_Missing' end

-- timer_int: reads 1 once its time has run out (measured); here starting or
-- stopping a timer clears it.
function Mission:startTimer(name, seconds)
    self.timers[name] = self.time + seconds
    self.state[name] = 0
end

function Mission:stopTimer(name)
    self.timers[name] = nil
    self.state[name] = 0
end

function Mission:setup()
    for _, v in ipairs(self.variables) do
        if self.state[v.name] == nil then self.state[v.name] = self.initial[v.name] or 0 end
    end
    for i, rule in ipairs(self.rules) do
        local monitor = Monitor(rule.name, false, nil, function()
            self.callFailed = false
            local holds = rule.when(self.state, self.objects) == true
            return holds and not self.callFailed  -- no answer from the engine: does not hold
        end, nil)
        monitor:hook(rule.name, MonitorHook.Type.OnTrue, nil, function()
            self[rule.action](self)
        end, nil)
        self.ruleMonitors[i] = monitor
    end
end

function Mission:update(time)
    self.time = time
    for _, v in ipairs(self.variables) do
        local at = self.timers[v.name]
        if at and time > at then
            self.timers[v.name] = nil
            self.state[v.name] = 1
        end
    end
end

MMM.register(Mission)
"#;
