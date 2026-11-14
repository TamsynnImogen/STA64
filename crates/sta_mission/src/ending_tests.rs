//! Asset-free tests of the ending check (LUA-29): the selection of end steps on a tiny script written in the
//! converter's shape, and the harness on tiny missions in a synthetic install.

use super::*;
use sta_content::MapDef;
use sta_core::{PlayerId, TeamId};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};

const MSHELL: &str = "// drills\r\nsyn_tut1.bzn\r\n-\r\n// first fleet\r\nsyn_fed01.bzn\r\nsyn_fed02.bzn\r\nsyn_fed03.bzn\r\n-\r\nsyn_kl01.bzn\r\n-\r\n-\r\n";
const LABELS: &str = "single_player {\n\t+ federation_incomplete\t{x}\n\t+ klingon_incomplete {}\n\t+ tutorials {Drills}\n\t+ federation_campaign {First Fleet}\n\t+ klingon_campaign {Second Fleet}\n}\nmission_select {\n}\n";

/// The converter's shape in miniature: actions queue steps, steps call the end, rules fire
/// satisfied actions every tick. `{extra}` adds functions and rules per test.
fn script(extra_funcs: &str, extra_rules: &str, tail: &str) -> String {
    format!(
        r#"
Mission_ = {{ rules = {{}}, scripts = {{}}, satisfied = {{}}, fired = {{}}, fireCount = {{}}, ran = {{}}, missing = {{}}, failed = {{}}, stepErrors = {{}}, notDecoded = {{}}, conditionErrors = {{}}, stepCount = 0 }}
local Mission = Mission_
local M = {{}}
local S = {{}}
local function A(name, ...)
    local f = A2[name]
    local ok, v = pcall(f, ...)
    if not ok then Mission.failed[name] = (Mission.failed[name] or 0) + 1 return nil end
    return v
end
local function VarOp(p, op, x) S[p] = x end
local function NewScript(name) local s = {{ name = name, steps = {{}}, at = 1 }} Mission.scripts[#Mission.scripts + 1] = s return s end
local function AddStep(s, fn, name) s.steps[#s.steps + 1] = fn s.names = s.names or {{}} s.names[#s.steps] = name end

-- init: init_1
function M.ScriptInit(...)
    VarOp('p_IN_PROGRESS', 'set', 1)
    do return 0 end
end

-- action: finish_win
function M.finish_win(...)
    local t = NewScript('w')
    AddStep(t, M.finish_win_step, 'finish_win_step')
end
-- step: finish_win_step
function M.finish_win_step(...)
    VarOp('p_WIN', 'set', 0)
    A('SucceedMission', A('GetTime'), 0)
    do return 0 end
end
-- action: finish_lose
function M.finish_lose(...)
    local t = NewScript('l')
    AddStep(t, M.finish_lose_step, 'finish_lose_step')
end
-- step: finish_lose_step
function M.finish_lose_step(...)
    A('FailMission', A('GetTime'), 0)
    do return 0 end
end
{extra_funcs}
Mission.rules[#Mission.rules + 1] = {{
    name = 'WinRule',
    action = 'finish_win',
    deps = {{ 'p_WIN', 'p_ALWAYS_EVALUATE' }},
    when = function() return S.p_WIN == 1 end,
}}
Mission.rules[#Mission.rules + 1] = {{
    name = 'LoseRule',
    action = 'finish_lose',
    deps = {{ 'p_ALWAYS_EVALUATE', 'p_IN_PROGRESS' }},
    when = function() return false end,
}}
{extra_rules}
function Mission:update(time)
    local running = {{}}
    for _, rule in ipairs(self.rules) do
        local holds = rule.when()
        if holds and not rule.last then rule.last = true table.insert(self.satisfied, 1, rule)
        elseif not holds and rule.last then
            rule.last = false
            for i, x in ipairs(self.satisfied) do if x == rule then table.remove(self.satisfied, i) break end end
        end
    end
    local fire = {{}}
    for i, rule in ipairs(self.satisfied) do fire[i] = rule end
    for _, rule in ipairs(fire) do
        if not rule.trashed then
            self.fireCount[rule.name] = (self.fireCount[rule.name] or 0) + 1
            local action = M[rule.action]
            if action then
                local ok, err = pcall(action)
                if not ok then self.stepErrors[#self.stepErrors + 1] = tostring(err) end
            else
                self.notDecoded[rule.action] = (self.notDecoded[rule.action] or 0) + 1
            end
        end
    end
    -- the satisfied tiny rules fire once only: drop the ones the script's own rules made
    for i = #self.satisfied, 1, -1 do if self.satisfied[i].once then table.remove(self.satisfied, i) end end
    local i = 1
    while i <= #self.scripts do
        local s = self.scripts[i]
        local fn = s.steps[s.at]
        if not fn then table.remove(self.scripts, i) else
            s.at = s.at + 1
            local ok, err = pcall(fn, s)
            if not ok then self.stepErrors[#self.stepErrors + 1] = tostring(err) end
            i = i + 1
        end
    end
    self.lastStep = 'x'
end
{tail}
MMM.register(Mission)
"#
    )
}

/// A win sequence of two steps whose last sets the flag the win rule waits on, an
/// action that is its lead-in; `bad` calls something unbound on the way.
fn lead_in(name: &str, bad: bool) -> String {
    format!(
        "-- action: {name}\nfunction M.{name}(...)\n    local t = NewScript('{name}')\n    AddStep(t, M.{name}_step1, '{name}_step1')\n    AddStep(t, M.{name}_step2, '{name}_step2')\nend\n-- step: {name}_step1\nfunction M.{name}_step1(...)\n    {}\n    do return 0 end\nend\n-- step: {name}_step2\nfunction M.{name}_step2(...)\n    VarOp('p_WIN', 'set', 1)\n    do return 0 end\nend\n",
        if bad { "A('NoSuchCall', 1)" } else { "A('GetTime')" }
    )
}

fn rule(name: &str, action: &str) -> String {
    format!("Mission.rules[#Mission.rules + 1] = {{\n    name = '{name}',\n    action = '{action}',\n    deps = {{ 'p_VICTORY_NEAR' }},\n    when = function() return false end,\n}}\n")
}

#[test]
fn plan_picks_lead_ins_and_falls_back_to_the_terminal_action() {
    let src = script(&(lead_in("victory_cin", false) + &lead_in("victory_bad", true)), &(rule("Near", "victory_cin") + &rule("Near2", "victory_bad")), "");
    let p = plan(&src);
    assert!(p.calls_succeed && p.calls_fail);
    let wins: Vec<_> = p.win.iter().map(|e| (e.action.as_str(), e.kind, e.rules.clone())).collect();
    assert_eq!(wins, [("victory_cin", EntryKind::LeadIn, vec!["WinRule".to_owned()]), ("victory_bad", EntryKind::LeadIn, vec!["WinRule".to_owned()])]);
    // The loss rule waits on no flag the script sets (init sets `p_IN_PROGRESS`): its own action runs.
    assert_eq!(p.lose, [Entry { action: "finish_lose".into(), kind: EntryKind::Terminal, rules: vec!["LoseRule".into()] }]);
    assert_eq!(p.truncated, (0, 0));
}

#[test]
fn plan_without_end_calls_is_empty() {
    let p = plan("function M.a(...)\n    A('Goto', 1)\nend\nMission.rules[#Mission.rules + 1] = {\n    name = 'R',\n    action = 'a',\n    deps = { 'x' },\n}\n");
    assert_eq!(p, Plan::default());
}

#[test]
fn plan_follows_helpers_and_steps_transitively() {
    let src = "-- action: a\nfunction M.a(...)\n    AddStep(t, M.a_step, 'a_step')\nend\n-- step: a_step\nfunction M.a_step(...)\n    t[1] = M.helper_1()\nend\n-- helper: helper_1\nfunction M.helper_1(...)\n    A('FailMission', A('GetTime'), 0)\nend\nMission.rules[#Mission.rules + 1] = {\n    name = 'R',\n    action = 'a',\n    deps = { 'p_ALWAYS_EVALUATE' },\n}\n";
    let p = plan(src);
    assert!(p.win.is_empty());
    assert_eq!(p.lose.iter().map(|e| e.action.as_str()).collect::<Vec<_>>(), ["a"]);
}

#[test]
fn varop_zero_does_not_count_as_a_setter() {
    let funcs = parse_funcs("function M.a(...)\n    VarOp('p_X', 'set', 0)\n    VarOp('p_Y', 'set', 1)\n    VarOp('p_T', 'start', 8.0)\n    VarOp('p_Z', 'set', 0.0)\nend\n");
    assert_eq!(funcs["a"].sets.iter().map(String::as_str).collect::<Vec<_>>(), ["p_T", "p_Y"]);
}

// ---- the harness on tiny missions -------------------------------------------------------

struct Host(Arc<crate::campaign::MemSource>);

impl MissionHost for Host {
    fn game_setup(&self, _c: &Campaign, _m: &CampaignMission, seed: u64) -> Result<GameSetup, String> {
        Ok(GameSetup {
            seed,
            map: Arc::new(MapDef::empty()),
            content: Arc::new(sta_content::fake::db()),
            players: (0..2)
                .map(|id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: if id == 0 { Control::Human } else { Control::Idle } })
                .collect(),
            options: GameOptions::default(),
        })
    }
    fn script(&self, _c: &Campaign, m: &CampaignMission) -> Result<(String, ScriptReader), String> {
        let src = self.0.clone();
        resolved_script(m, Arc::new(move |p| src.read(p)))
    }
}

const GOAL: &str = "Objectives.objectives:add('Goal')\n";

fn book_and_host(fed: [String; 3]) -> (CampaignBook, Host) {
    let mut src = MemSource::new().with("mshell.set", MSHELL).with("label.map", LABELS).with("ART_CFG.h", "int GIVE_ALL_MISSIONS = 0;\r\n");
    for (i, text) in fed.iter().enumerate() {
        src.insert(&format!("bzn/syn_fed0{}.bzn", i + 1), "not parsed");
        src.insert(&format!("missions/syn_fed0{}.lua", i + 1), text.as_str());
    }
    for n in ["syn_tut1", "syn_kl01"] {
        src.insert(&format!("bzn/{n}.bzn"), "not parsed");
        src.insert(&format!("missions/{n}.lua"), "-- idle\n");
    }
    let book = CampaignBook::load(&src, ShellLayout::Armada2);
    (book, Host(Arc::new(src)))
}

fn opts() -> EndingOptions {
    EndingOptions { min_tick: 10, warm_max: 60, cap: 200, ..EndingOptions::new(CheatGame::A2) }
}

fn good() -> String {
    script(&lead_in("victory_cin", false), &rule("Near", "victory_cin"), GOAL)
}

#[test]
fn forced_win_and_loss_resolve_and_advance_the_campaign() {
    let (book, host) = book_and_host([good(), good(), good()]);
    let o = EndingOptions { forced_only: true, ..opts() };
    let p = probe_mission(&host, None, &book, "federation", 0, &o);
    assert!(p.started, "{:?}", p.start_error);
    assert!(p.first_objective_tick.is_some(), "{:?}", p.notes);
    let w = p.forced_win.as_ref().unwrap();
    assert!(w.ok, "{w:?}");
    assert_eq!((w.result, w.ticks), (Some(true), 0), "latched, resolved at the next scan");
    assert_eq!(w.transition.as_deref(), Some("next: federation mission 1"));
    let l = p.forced_lose.as_ref().expect("the script calls FailMission");
    assert!(l.ok, "{l:?}");
    assert_eq!(l.transition.as_deref(), Some("lost: federation mission 0"));
    assert!(p.forced_ok());
    // The last mission of the campaign completes it.
    let last = probe_mission(&host, None, &book, "federation", 2, &o);
    assert_eq!(last.forced_win.as_ref().unwrap().transition.as_deref(), Some("campaign complete"));
    assert!(last.forced_ok());
}

#[test]
fn the_forced_loss_runs_in_armada_ii_even_without_fail_mission() {
    let no_fail = good().replace("'FailMission'", "'Goto'");
    let (book, host) = book_and_host([no_fail, good(), good()]);
    let p = probe_mission(&host, None, &book, "federation", 0, &EndingOptions { forced_only: true, ..opts() });
    assert!(p.forced_lose.as_ref().is_some_and(|t| t.ok), "{:?}", p.forced_lose);
    assert!(p.forced_ok());
    assert_eq!(p.lose_seq_ok(), None, "no lose sequence to run");
}

#[test]
fn sequences_run_from_the_test_point_and_end_by_the_scripts_own_rules() {
    let (book, host) = book_and_host([good(), good(), good()]);
    let p = probe_mission(&host, None, &book, "federation", 0, &opts());
    assert_eq!(p.win_seq.len(), 1);
    let w = &p.win_seq[0];
    assert!(w.ok, "{w:?}");
    assert_eq!((w.action.as_str(), w.kind.as_str(), w.result), ("victory_cin", "lead-in", Some(true)));
    // Step 1, step 2 (sets the flag), the win rule fires, its step calls SucceedMission, ends on the next update.
    assert!(w.ticks >= 3 && w.ticks <= 6, "{}", w.ticks);
    assert!(p.win_seq_ok());
    assert_eq!(p.lose_seq.len(), 1);
    assert!(p.lose_seq[0].ok, "{:?}", p.lose_seq[0]);
    assert_eq!(p.lose_seq[0].kind, "terminal");
    assert_eq!(p.lose_seq_ok(), Some(true));
    assert!(p.to_json().contains("\"win_seq\":[{\"action\":\"victory_cin\""));
}

#[test]
fn an_unbound_call_in_the_sequence_is_a_gap() {
    let bad = script(&lead_in("victory_cin", true), &rule("Near", "victory_cin"), GOAL);
    let (book, host) = book_and_host([bad, good(), good()]);
    let p = probe_mission(&host, None, &book, "federation", 0, &opts());
    let w = &p.win_seq[0];
    assert_eq!(w.result, Some(true), "it still ends");
    assert!(!w.ok);
    assert_eq!(w.gaps, ["NoSuchCall (raised)"]);
    assert!(!p.win_seq_ok());
}

#[test]
fn a_sequence_that_never_ends_is_reported_with_its_cap() {
    // The flag is set to a value the win rule does not read: the lead-in runs, nothing ends.
    let dead = good().replace("VarOp('p_WIN', 'set', 1)", "VarOp('p_WIN', 'set', 2)");
    let (book, host) = book_and_host([dead, good(), good()]);
    let p = probe_mission(&host, None, &book, "federation", 0, &opts());
    let w = &p.win_seq[0];
    assert!(!w.ok);
    assert_eq!((w.result, w.ticks), (None, 200));
    assert!(w.note.contains("did not end within 200 ticks"), "{}", w.note);
}

#[test]
fn a_mission_that_ends_by_itself_before_the_test_point_is_reported() {
    let early = good() + "\nMMM.register(function(t) A2.SucceedMission(0.0, nil) end)\n";
    let (book, host) = book_and_host([early, good(), good()]);
    let p = probe_mission(&host, None, &book, "federation", 0, &opts());
    assert!(!p.started);
    assert!(p.start_error.as_deref().is_some_and(|e| e.contains("ended by itself")), "{:?}", p.start_error);
}

#[test]
fn a_natural_end_is_not_credited_to_an_entry() {
    // The lead-in sets a flag the win rule does not read; the mission wins by itself a little later.
    let dead = good().replace("VarOp('p_WIN', 'set', 1)", "VarOp('p_WIN', 'set', 2)") + "\nMMM.register(function(t) if t >= 0.7 then A2.SucceedMission(0.0, nil) end end)\n";
    let (book, host) = book_and_host([dead, good(), good()]);
    let p = probe_mission(&host, None, &book, "federation", 0, &opts());
    let w = &p.win_seq[0];
    assert_eq!(w.result, Some(true));
    assert!(!w.ok);
    assert!(w.note.contains("not credited"), "{}", w.note);
}

#[test]
fn the_fired_rule_survives_reads_of_trashed_before_the_engine_fires_it() {
    let (book, host) = book_and_host([good(), good(), good()]);
    let r = at_test_point(&book, &host, None, "federation", 0, &opts(), |runner, mut mid| {
        let a = runner.active_mut().unwrap();
        inject(a, "victory_cin").unwrap();
        // What the converter's TrashRule does on every entry: read `.trashed` of every satisfied rule.
        a.runtime().eval::<()>("for _, r in ipairs(Mission_.satisfied) do local _ = r.trashed end").unwrap();
        (0..20).find_map(|_| step(a, &mut mid.standin)).map(|o| o.won)
    });
    assert_eq!(r, Ok(Some(true)));
}

#[test]
fn an_action_the_script_does_not_define_is_a_gap() {
    let (book, host) = book_and_host([good(), good(), good()]);
    let e = Entry { action: "nope".into(), kind: EntryKind::Terminal, rules: vec![] };
    let t = sequence_trial(&book, &host, None, "federation", 0, &opts(), &e, true);
    assert!(!t.ok);
    assert_eq!(t.gaps, ["nope (not decoded)"]);
}

#[test]
fn a_helper_reaching_both_ends_is_terminal_for_neither_side() {
    let src = "-- action: a\nfunction M.a(...)\n    M.helper(...)\nend\n-- helper: helper\nfunction M.helper(...)\n    A('SucceedMission', 0, 0)\n    A('FailMission', 0, 0)\nend\nMission.rules[#Mission.rules + 1] = {\n    name = 'R',\n    action = 'a',\n    deps = { 'p_ALWAYS_EVALUATE' },\n}\n";
    let p = plan(src);
    assert!(p.win.is_empty() && p.lose.is_empty());
    assert!(p.calls_succeed && p.calls_fail);
}

#[test]
fn a_column_needs_every_entry_run() {
    let ok = EndTrial { ok: true, result: Some(true), ..Default::default() };
    let mut p = EndingProbe { win_seq: vec![ok.clone()], lose_seq: vec![ok], ..Default::default() };
    p.plan.calls_fail = true;
    assert!(p.win_seq_ok() && p.lose_seq_ok() == Some(true));
    p.plan.truncated = (1, 0);
    assert!(!p.win_seq_ok());
    p.plan.truncated = (0, 2);
    assert_eq!(p.lose_seq_ok(), Some(false));
}
