//! Ending check (LUA-29): do the endings of the converted missions work, cutscenes included?
//! `tools/campaign_readiness.py --converted a1|a2 --ending` prints the table.
//!
//! Three checks per mission, each from a mid-mission state (the first player objective has been
//! issued and at least [`EndingOptions::min_tick`] ticks have run), headless, with the stand-in
//! host answering the client's completion reports:
//!
//! 1. **Forced result** ([`crate::cheat`]): force a win, then a loss, through the mission end
//!    record; pass when the end resolves to the right result, no call is unbound or raises (nor
//!    does anything count as a new gap) while it resolves, and the campaign transition applies
//!    and the next mission becomes available (a win: [`Transition::Next`] with `reached` moved
//!    to the next mission, or [`Transition::CampaignComplete`] for a last mission; a loss:
//!    [`Transition::Lost`] with the progress unchanged). A forced win does not prove the mission's
//!    own win condition is reachable; the forced end skips the script's win and lose steps.
//! 2. **Win sequence** and 3. **lose sequence**: run the script's own steps that lead to
//!    `SucceedMission` / `FailMission` from that mid-mission state and let the scripted end
//!    complete; pass when it ends with the right result within [`EndingOptions::cap`] ticks,
//!    nothing new is unbound, raises, is unsupported or stops a rule or step, and the end is not
//!    one the mission reaches by itself: one **uninjected baseline** run per mission from the same
//!    test point (deterministic) gives `(result, ticks)`, and an entry is not credited when the
//!    baseline ends the same way at or before the entry's own end tick. That is a baseline
//!    differential: an entry that ends the mission earlier or differently than nothing would is
//!    credited, whether or not the entry's steps are what reached `SucceedMission` / `FailMission`
//!    (a rule satisfied by something else in the same ticks cannot be told apart). It does not
//!    prove the state the steps expect exists in a real game and can reach the end by a path no
//!    player can. A mission whose baseline ends by itself within the cap scores FAIL (not n/a) in
//!    columns 2 and 3 when the entry is not earlier: the check cannot tell its entry from the
//!    mission's own end.
//!
//! **Selection of the end steps** (read from the converted script's own text; [`plan`]): the
//! converter output knows which rule calls which action (`action = ` of the rule table), which
//! steps an action queues (`AddStep(.., M.step, ..)`), which step or helper calls which
//! (`M.name`), the calls to `SucceedMission` / `FailMission`, and the variables a body sets
//! (`VarOp(name, 'set'|'start', ..)`) and a rule waits on (`deps`). A function *reaches* an end
//! call when its body, or a function it names with `M.`, contains it. A **terminal rule** is a
//! rule whose action reaches an end call. The victory and defeat sequences (cineractive, voice
//! lines, fades) are usually not in the terminal action: an earlier action plays them and then
//! sets the variable the terminal rule waits on. So per terminal rule: the **flag variables** are
//! its `deps` minus `ALWAYS_EVALUATE` and minus variables the init code sets; if some rule's
//! action sets such a variable (`set` to anything but a literal 0, or `start`), those actions are
//! the **lead-in** entries (the terminal action itself, which sets the flag too, and every other
//! terminal action are left out) and the mission's own terminal rule completes the end; with no
//! flag variable or no setter the **terminal** action is the entry. Each entry runs alone, from
//! its own fresh mission, by firing that action once as a rule would. One level of lead-in only
//! (a chain of several levels runs on through the mission's own rules). A rule whose action reaches both end calls (a shared helper) is terminal for neither side, and a lead-in is dropped when its reach contains the opposite end. `M.` references are found in the text, comments and strings included. Over-approximations: a
//! `VarOp` counts as a setter whatever the branch it sits in, and an action that reaches an end
//! call only on one branch counts as reaching it.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use crate::campaign::*;
use crate::cheat::{self, CheatGame};
use crate::readiness::json_string;
use crate::standin::StandInHost;

// ---------------------------------------------------------------------------
// Reading the converted script
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Func {
    kind: String,
    refs: BTreeSet<String>,
    succeed: bool,
    fail: bool,
    /// Variables the body sets (to anything but a literal zero) or starts.
    sets: BTreeSet<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RuleRow {
    name: String,
    action: String,
    deps: Vec<String>,
}

/// Where an entry comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// An action that sets a variable the terminal rule waits on.
    LeadIn,
    /// The action of a terminal rule itself.
    Terminal,
}

/// One end sequence to run: the action to fire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub action: String,
    pub kind: EntryKind,
    /// The terminal rules it leads to.
    pub rules: Vec<String>,
}

/// The entries found in one script, by the selection rule in the module docs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub win: Vec<Entry>,
    pub lose: Vec<Entry>,
    /// A body in the script calls `SucceedMission` / `FailMission` at all.
    pub calls_succeed: bool,
    pub calls_fail: bool,
    /// Entries beyond [`MAX_ENTRIES`] that were left out, per side.
    pub truncated: (usize, usize),
}

/// Entries run per side; more are counted in [`Plan::truncated`] and not run.
pub const MAX_ENTRIES: usize = 6;

fn ident_end(s: &str) -> usize {
    s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(s.len())
}

fn quoted_list(s: &str) -> Vec<String> {
    s.split('\'').skip(1).step_by(2).map(str::to_owned).collect()
}

fn parse_funcs(src: &str) -> BTreeMap<String, Func> {
    let mut out = BTreeMap::new();
    let mut kind = String::new();
    let mut cur: Option<(String, Func)> = None;
    for line in src.lines() {
        if let Some((name, f)) = cur.as_mut() {
            if line == "end" {
                let (n, f) = cur.take().expect("current");
                out.insert(n, f);
                continue;
            }
            body_line(name, f, line);
            continue;
        }
        if let Some(rest) = line.strip_prefix("-- ")
            && let Some((k, _)) = rest.split_once(": ")
            && !k.contains(' ')
        {
            kind = k.to_owned();
        } else if let Some(rest) = line.strip_prefix("function M.") {
            let name = &rest[..ident_end(rest)];
            cur = Some((name.to_owned(), Func { kind: std::mem::take(&mut kind), ..Default::default() }));
        }
    }
    out
}

fn body_line(own: &str, f: &mut Func, line: &str) {
    if line.contains("'SucceedMission'") {
        f.succeed = true;
    }
    if line.contains("'FailMission'") {
        f.fail = true;
    }
    let mut at = 0;
    while let Some(i) = line[at..].find("M.") {
        let start = at + i;
        let before = line[..start].chars().next_back();
        let rest = &line[start + 2..];
        let id = &rest[..ident_end(rest)];
        if !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') && !id.is_empty() && id != own {
            f.refs.insert(id.to_owned());
        }
        at = start + 2;
    }
    let mut at = 0;
    while let Some(i) = line[at..].find("VarOp('") {
        let rest = &line[at + i + 7..];
        at += i + 7;
        let Some((var, tail)) = rest.split_once('\'') else { break };
        let Some(tail) = tail.strip_prefix(", '") else { continue };
        let Some((op, tail)) = tail.split_once('\'') else { continue };
        let arg = tail.strip_prefix(", ").unwrap_or("");
        let arg = arg.split([')', ',']).next().unwrap_or("").trim();
        let zero = arg.parse::<f64>().is_ok_and(|v| v == 0.0) || arg == "false" || arg == "nil";
        if (op == "set" && !zero) || op == "start" {
            f.sets.insert(var.to_owned());
        }
    }
}

fn parse_rules(src: &str) -> Vec<RuleRow> {
    let mut out = Vec::new();
    let mut cur: Option<RuleRow> = None;
    for line in src.lines() {
        if line == "Mission.rules[#Mission.rules + 1] = {" {
            cur = Some(RuleRow::default());
        } else if let Some(r) = cur.as_mut() {
            let t = line.trim();
            if line == "}" {
                out.extend(cur.take());
            } else if let Some(v) = t.strip_prefix("name = '") {
                r.name = v.split('\'').next().unwrap_or("").to_owned();
            } else if let Some(v) = t.strip_prefix("action = '") {
                r.action = v.split('\'').next().unwrap_or("").to_owned();
            } else if let Some(v) = t.strip_prefix("deps = {") {
                r.deps = quoted_list(v.split('}').next().unwrap_or(""));
            }
        }
    }
    out
}

fn reach(funcs: &BTreeMap<String, Func>, from: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![from.to_owned()];
    while let Some(n) = stack.pop() {
        if !funcs.contains_key(&n) || !seen.insert(n.clone()) {
            continue;
        }
        stack.extend(funcs[&n].refs.iter().cloned());
    }
    seen
}

/// The entries of `src`, a converted mission script (selection rule in the module docs).
pub fn plan(src: &str) -> Plan {
    let funcs = parse_funcs(src);
    let rules = parse_rules(src);
    let mut init_set: BTreeSet<String> = BTreeSet::new();
    for (n, f) in &funcs {
        if n == "ScriptInit" || f.kind == "init" {
            for g in reach(&funcs, n) {
                init_set.extend(funcs[&g].sets.iter().cloned());
            }
        }
    }
    let reaches = |name: &str, fail: bool| reach(&funcs, name).iter().any(|g| if fail { funcs[g].fail } else { funcs[g].succeed });
    // A rule that reaches both ends (a shared helper) is terminal for neither side: it cannot
    // be told apart by the text, so it is left out of both and counted nowhere.
    let terminal_win: Vec<&RuleRow> = rules.iter().filter(|r| reaches(&r.action, false) && !reaches(&r.action, true)).collect();
    let terminal_lose: Vec<&RuleRow> = rules.iter().filter(|r| reaches(&r.action, true) && !reaches(&r.action, false)).collect();
    let all_terminal: BTreeSet<&str> = rules.iter().filter(|r| reaches(&r.action, true) || reaches(&r.action, false)).map(|r| r.action.as_str()).collect();
    let side = |terminals: &[&RuleRow]| -> (Vec<Entry>, usize) {
        let mut entries: Vec<Entry> = Vec::new();
        let mut add = |action: &str, kind: EntryKind, rule: &str| {
            if let Some(e) = entries.iter_mut().find(|e| e.action == action) {
                if !e.rules.iter().any(|r| r == rule) {
                    e.rules.push(rule.to_owned());
                }
            } else {
                entries.push(Entry { action: action.to_owned(), kind, rules: vec![rule.to_owned()] });
            }
        };
        for t in terminals {
            let flags: BTreeSet<&str> = t
                .deps
                .iter()
                .map(String::as_str)
                .filter(|d| !d.to_ascii_uppercase().contains("ALWAYS_EVALUATE") && !init_set.contains(*d))
                .collect();
            let mut lead: Vec<&str> = Vec::new();
            if !flags.is_empty() {
                for r in &rules {
                    let a = r.action.as_str();
                    if all_terminal.contains(a) || lead.contains(&a) || !funcs.contains_key(a) {
                        continue;
                    }
                    if reach(&funcs, a).iter().any(|g| funcs[g].sets.iter().any(|v| flags.contains(v.as_str()))) {
                        lead.push(a);
                    }
                }
            }
            if lead.is_empty() {
                add(&t.action, EntryKind::Terminal, &t.name);
            } else {
                for a in lead {
                    add(a, EntryKind::LeadIn, &t.name);
                }
            }
        }
        let extra = entries.len().saturating_sub(MAX_ENTRIES);
        entries.truncate(MAX_ENTRIES);
        (entries, extra)
    };
    let (win, tw) = side(&terminal_win);
    let (lose, tl) = side(&terminal_lose);
    Plan {
        win,
        lose,
        calls_succeed: funcs.values().any(|f| f.succeed),
        calls_fail: funcs.values().any(|f| f.fail),
        truncated: (tw, tl),
    }
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

/// How the probe runs.
#[derive(Debug, Clone)]
pub struct EndingOptions {
    pub game: CheatGame,
    /// The test point is the first tick at or after the first objective and this one.
    pub min_tick: u64,
    /// Give up on reaching an objective after this many ticks and test from there anyway.
    pub warm_max: u64,
    /// Ticks a sequence may take to end before it counts as not ending.
    pub cap: u64,
    /// Ticks the forced end may take to resolve.
    pub forced_cap: u64,
    /// Run only the forced results (the two sequences are skipped).
    pub forced_only: bool,
}

impl EndingOptions {
    pub fn new(game: CheatGame) -> EndingOptions {
        EndingOptions { game, min_tick: 300, warm_max: 3000, cap: 6000, forced_cap: 5, forced_only: false }
    }
}

/// One run from the test point.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndTrial {
    /// The action fired (sequences); empty for a forced end.
    pub action: String,
    /// `lead-in` / `terminal` for a sequence.
    pub kind: String,
    pub ok: bool,
    /// `Some(won)` when it ended.
    pub result: Option<bool>,
    /// Ticks from the start of the trial's end to its resolution.
    pub ticks: u64,
    /// New bound-but-failed, unbound or unsupported calls, new runtime or step errors.
    pub gaps: Vec<String>,
    /// What the campaign did with the result (forced end only).
    pub transition: Option<String>,
    pub note: String,
}

/// What one mission did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndingProbe {
    pub campaign: String,
    pub index: usize,
    pub bzn: String,
    pub started: bool,
    pub start_error: Option<String>,
    /// The tick the trials start from, and the first objective's tick.
    pub test_tick: Option<u64>,
    pub first_objective_tick: Option<u64>,
    pub plan: Plan,
    pub forced_win: Option<EndTrial>,
    /// `None` when the mission did not start.
    pub forced_lose: Option<EndTrial>,
    /// How the mission ends with nothing injected, run from the same test point: `(won, ticks)`.
    pub baseline: Option<(bool, u64)>,
    pub win_seq: Vec<EndTrial>,
    pub lose_seq: Vec<EndTrial>,
    pub notes: Vec<String>,
}

impl EndingProbe {
    /// Column 1: the forced win and (where the mission has a loss) the forced loss pass.
    pub fn forced_ok(&self) -> bool {
        self.forced_win.as_ref().is_some_and(|t| t.ok) && self.forced_lose.as_ref().is_none_or(|t| t.ok)
    }
    /// Column 2: at least one win entry was found, none was left out for the cap, and all pass.
    pub fn win_seq_ok(&self) -> bool {
        !self.win_seq.is_empty() && self.plan.truncated.0 == 0 && self.win_seq.iter().all(|t| t.ok)
    }
    /// Column 3: at least one lose entry was found and all pass (`None`: the mission has no loss).
    pub fn lose_seq_ok(&self) -> Option<bool> {
        if !self.plan.calls_fail && self.plan.lose.is_empty() {
            return None;
        }
        Some(!self.lose_seq.is_empty() && self.plan.truncated.1 == 0 && self.lose_seq.iter().all(|t| t.ok))
    }

    pub fn to_json(&self) -> String {
        fn trial(t: &EndTrial) -> String {
            let gaps = t.gaps.iter().map(|g| json_string(g)).collect::<Vec<_>>().join(",");
            format!(
                "{{\"action\":{},\"kind\":{},\"ok\":{},\"result\":{},\"ticks\":{},\"gaps\":[{}],\"transition\":{},\"note\":{}}}",
                json_string(&t.action),
                json_string(&t.kind),
                t.ok,
                t.result.map_or("null", |w| if w { "true" } else { "false" }),
                t.ticks,
                gaps,
                t.transition.as_deref().map_or("null".into(), json_string),
                json_string(&t.note),
            )
        }
        fn opt(t: &Option<EndTrial>) -> String {
            t.as_ref().map_or("null".into(), trial)
        }
        fn many(v: &[EndTrial]) -> String {
            format!("[{}]", v.iter().map(trial).collect::<Vec<_>>().join(","))
        }
        fn entries(v: &[Entry]) -> String {
            let one = |e: &Entry| {
                format!(
                    "{{\"action\":{},\"kind\":{},\"rules\":[{}]}}",
                    json_string(&e.action),
                    json_string(if e.kind == EntryKind::LeadIn { "lead-in" } else { "terminal" }),
                    e.rules.iter().map(|r| json_string(r)).collect::<Vec<_>>().join(",")
                )
            };
            format!("[{}]", v.iter().map(one).collect::<Vec<_>>().join(","))
        }
        let num = |o: Option<u64>| o.map_or("null".into(), |v| v.to_string());
        format!(
            "{{\"campaign\":{},\"index\":{},\"bzn\":{},\"started\":{},\"start_error\":{},\"test_tick\":{},\"first_objective_tick\":{},\"win_entries\":{},\"lose_entries\":{},\"truncated\":[{},{}],\"calls_fail\":{},\"forced_win\":{},\"forced_lose\":{},\"baseline\":{},\"win_seq\":{},\"lose_seq\":{},\"notes\":[{}]}}",
            json_string(&self.campaign),
            self.index,
            json_string(&self.bzn),
            self.started,
            self.start_error.as_deref().map_or("null".into(), json_string),
            num(self.test_tick),
            num(self.first_objective_tick),
            entries(&self.plan.win),
            entries(&self.plan.lose),
            self.plan.truncated.0,
            self.plan.truncated.1,
            self.plan.calls_fail,
            opt(&self.forced_win),
            opt(&self.forced_lose),
            self.baseline.map_or("null".into(), |(w, k)| format!("[{w},{k}]")),
            many(&self.win_seq),
            many(&self.lose_seq),
            self.notes.iter().map(|n| json_string(n)).collect::<Vec<_>>().join(","),
        )
    }
}

/// Counts that grow when something goes wrong: per-name calls that were unbound, raised or are
/// bound without effect, plus the runtime's and the steps' error lists.
#[derive(Debug, Clone, Default)]
struct Gaps {
    calls: BTreeMap<String, u32>,
    errors: usize,
    step_errors: usize,
}

fn gaps_now(rt: &crate::runtime::MissionRuntime) -> Gaps {
    let mut g = Gaps::default();
    for field in ["missing", "failed"] {
        let list: Vec<String> = rt
            .eval(&format!("local t = {{}} for k, n in pairs(Mission_.{field}) do t[#t + 1] = k .. '\\t' .. tostring(n) end return t"))
            .unwrap_or_default();
        for s in list {
            if let Some((k, n)) = s.rsplit_once('\t') {
                g.calls.insert(format!("{k} ({})", if field == "missing" { "unbound" } else { "raised" }), n.parse().unwrap_or(0));
            }
        }
    }
    for (field, what) in [("conditionErrors", "condition error"), ("notDecoded", "not decoded")] {
        let keys: Vec<String> = rt.eval(&format!("local t = {{}} for k in pairs(Mission_.{field} or {{}}) do t[#t + 1] = tostring(k) end return t")).unwrap_or_default();
        for k in keys {
            g.calls.insert(format!("{k} ({what})"), 1);
        }
    }
    for (k, n) in rt.unsupported() {
        g.calls.insert(format!("{} (unsupported)", k.split(':').next().unwrap_or(&k)), n);
    }
    g.errors = rt.errors().len();
    g.step_errors = rt.eval::<Vec<String>>("return Mission_.stepErrors").map_or(0, |v| v.len());
    g
}

fn new_gaps(before: &Gaps, after: &Gaps) -> Vec<String> {
    let mut out: Vec<String> = after
        .calls
        .iter()
        .filter(|(k, n)| **n > before.calls.get(*k).copied().unwrap_or(0))
        .map(|(k, _)| k.clone())
        .collect();
    if after.errors > before.errors {
        out.push(format!("{} runtime error(s)", after.errors - before.errors));
    }
    if after.step_errors > before.step_errors {
        out.push(format!("{} step error(s)", after.step_errors - before.step_errors));
    }
    out
}

/// A fresh progress with every campaign open and `campaign` standing at `index`.
fn runner_at(book: &CampaignBook, campaign: &str, index: usize) -> CampaignRunner {
    let mut open = book.clone();
    open.give_all_missions = true;
    let mut progress = CampaignProgress::new(&open);
    progress.apply_give_all(&open);
    progress.campaigns.insert(campaign.to_owned(), CampaignState { reached: index, complete: false });
    CampaignRunner::with_progress(open, progress)
}

/// The test point: a started mission, run until an objective has been issued and `min_tick`
/// ticks have passed.
struct Mid {
    standin: Option<StandInHost>,
    first_objective_tick: Option<u64>,
}

fn step(a: &mut ActiveMission, standin: &mut Option<StandInHost>) -> Option<MissionOutcome> {
    let o = a.tick();
    if let Some(h) = standin.as_mut() {
        h.after_tick(a);
    }
    o
}

fn warm_up(a: &mut ActiveMission, vfs: Option<&Arc<sta_vfs::Vfs>>, opts: &EndingOptions) -> Result<Mid, String> {
    let mut m = Mid { standin: vfs.map(|v| StandInHost::new(v.clone())), first_objective_tick: None };
    loop {
        let tick = a.sim().tick;
        if m.first_objective_tick.is_some() && tick >= opts.min_tick {
            return Ok(m);
        }
        if tick >= opts.warm_max {
            return Ok(m);
        }
        if let Some(o) = step(a, &mut m.standin) {
            return Err(format!("the mission ended by itself ({}) at tick {} before the test point", if o.won { "won" } else { "lost" }, o.tick));
        }
        if m.first_objective_tick.is_none() && !a.runtime().objectives().is_empty() {
            m.first_objective_tick = Some(a.sim().tick);
        }
    }
}

/// Fire `action` once, as a satisfied rule would on the next update. The rule is trashed by the
/// first read of its `action` (the engine's fire loop reads `trashed` first, then the action), so a
/// read of `trashed` by anything else (`TrashRule`) cannot consume the one firing.
fn inject(a: &ActiveMission, action: &str) -> Result<(), String> {
    let chunk = format!(
        "local m = rawget(_G, 'Mission_') \
         local r = setmetatable({{ name = '__ending_check', last = 1 }}, \
           {{ __index = function(t, k) if k == 'action' then rawset(t, 'trashed', true) return '{action}' end end }}) \
         table.insert(m.satisfied, 1, r)"
    );
    a.runtime().eval::<()>(&chunk).map_err(|e| format!("cannot fire {action}: {e}"))
}

/// Whether an entry's end is no evidence of the entry: the uninjected mission ends the same way
/// by the same tick.
fn natural_end(baseline: Option<(bool, u64)>, won: bool, ticks: u64) -> bool {
    baseline.is_some_and(|(w, k)| w == won && k <= ticks)
}

/// The mission run from the test point with nothing injected, for `cap` ticks (the sim and the
/// stand-in host are deterministic, so this is what every entry's trial would do without it).
fn baseline_trial(book: &CampaignBook, host: &dyn MissionHost, vfs: Option<&Arc<sta_vfs::Vfs>>, campaign: &str, index: usize, opts: &EndingOptions) -> Option<(bool, u64)> {
    at_test_point(book, host, vfs, campaign, index, opts, |runner, mut mid| {
        let a = runner.active_mut().expect("started");
        let at = a.sim().tick;
        (0..opts.cap).find_map(|_| step(a, &mut mid.standin)).map(|o| (o.won, o.tick.saturating_sub(at)))
    })
    .ok()
    .flatten()
}

type Probed<R> = Result<R, String>;

/// Start `campaign`'s mission `index` at the test point and hand it to `f`.
fn at_test_point<R>(
    book: &CampaignBook,
    host: &dyn MissionHost,
    vfs: Option<&Arc<sta_vfs::Vfs>>,
    campaign: &str,
    index: usize,
    opts: &EndingOptions,
    f: impl FnOnce(&mut CampaignRunner, Mid) -> R,
) -> Probed<R> {
    let mut runner = runner_at(book, campaign, index);
    let run = catch_unwind(AssertUnwindSafe(|| -> Probed<R> {
        let mid = {
            let a = runner.start(host, campaign, index, 1).map_err(|e| e.to_string())?;
            warm_up(a, vfs, opts)?
        };
        Ok(f(&mut runner, mid))
    }));
    run.unwrap_or_else(|_| Err("panic while running".into()))
}

fn forced_trial(book: &CampaignBook, host: &dyn MissionHost, vfs: Option<&Arc<sta_vfs::Vfs>>, campaign: &str, index: usize, opts: &EndingOptions, won: bool) -> EndTrial {
    let mut t = EndTrial::default();
    let res = at_test_point(book, host, vfs, campaign, index, opts, |runner, mut mid| {
        let a = runner.active_mut().expect("started");
        let at = a.sim().tick;
        let before = gaps_now(a.runtime());
        if !cheat::force_end(a.runtime(), at, won) {
            return Err("force_end refused (already ended)".to_owned());
        }
        let mut outcome = None;
        for _ in 0..opts.forced_cap {
            outcome = step(a, &mut mid.standin);
            if outcome.is_some() {
                break;
            }
        }
        let gaps = new_gaps(&before, &gaps_now(a.runtime()));
        let Some(o) = outcome else {
            return Ok((None, 0, gaps, None, format!("not resolved within {} ticks", opts.forced_cap)));
        };
        let ticks = o.tick.saturating_sub(at);
        let transition = runner.finish(o);
        let count = book.campaign(campaign).map_or(0, |c| c.missions.len());
        let st = runner.progress.state(campaign);
        let (text, good) = match (&transition, won) {
            (Some(Transition::Next { campaign: c, mission }), true) => (
                format!("next: {c} mission {mission}"),
                c == campaign && *mission == index + 1 && st.reached == index + 1 && runner.progress.available(&runner.book, campaign).contains(mission),
            ),
            (Some(Transition::CampaignComplete { .. }), true) => ("campaign complete".to_owned(), index + 1 >= count && st.complete),
            (Some(Transition::Lost { campaign: c, mission }), false) => {
                (format!("lost: {c} mission {mission}"), c == campaign && *mission == index && st.reached == index && !st.complete)
            }
            (other, _) => (format!("{other:?}"), false),
        };
        Ok((Some(o.won), ticks, gaps, Some(text.clone()), if good { String::new() } else { format!("wrong transition: {text}") }))
    });
    match res {
        Err(e) | Ok(Err(e)) => t.note = e,
        Ok(Ok((result, ticks, gaps, transition, note))) => {
            t.result = result;
            t.ticks = ticks;
            t.transition = transition;
            t.ok = result == Some(won) && gaps.is_empty() && note.is_empty();
            t.note = if result.is_some() && result != Some(won) { format!("resolved to the wrong result ({result:?})") } else { note };
            t.gaps = gaps;
        }
    }
    t
}

#[allow(clippy::too_many_arguments)]
fn sequence_trial(book: &CampaignBook, host: &dyn MissionHost, vfs: Option<&Arc<sta_vfs::Vfs>>, campaign: &str, index: usize, opts: &EndingOptions, entry: &Entry, won: bool, baseline: Option<(bool, u64)>) -> EndTrial {
    let mut t = EndTrial {
        action: entry.action.clone(),
        kind: if entry.kind == EntryKind::LeadIn { "lead-in" } else { "terminal" }.into(),
        ..Default::default()
    };
    let res = at_test_point(book, host, vfs, campaign, index, opts, |runner, mut mid| {
        let a = runner.active_mut().expect("started");
        let at = a.sim().tick;
        let before = gaps_now(a.runtime());
        inject(a, &entry.action)?;
        let mut outcome = None;
        for _ in 0..opts.cap {
            outcome = step(a, &mut mid.standin);
            if outcome.is_some() {
                break;
            }
        }
        let gaps = new_gaps(&before, &gaps_now(a.runtime()));
        let blocked = match a.runtime().mission_end(a.sim().tick) {
            crate::runtime::MissionEndState::Blocked(why) => Some(why),
            _ => None,
        };
        let last = a.runtime().eval::<Option<String>>("return Mission_.lastStep").ok().flatten();
        Ok((outcome.map(|o| (o.won, o.tick.saturating_sub(at))), gaps, blocked, last))
    });
    match res {
        Err(e) | Ok(Err(e)) => t.note = e,
        Ok(Ok((out, gaps, blocked, last))) => {
            let natural = out.is_some_and(|(w, k)| natural_end(baseline, w, k));
            t.result = out.map(|(w, _)| w);
            t.ticks = out.map_or(opts.cap, |(_, k)| k);
            t.ok = out.is_some_and(|(w, _)| w == won) && gaps.is_empty() && !natural;
            t.note = match (out, blocked) {
                (Some((w, _)), _) if w != won && baseline.is_some_and(|(bw, bk)| bw == w && bk <= out.map_or(0, |(_, k)| k)) => {
                    format!("preempted by the mission's own end at tick {} ({})", baseline.map_or(0, |(_, bk)| bk), if w { "won" } else { "lost" })
                }
                (Some((w, _)), _) if w != won => format!("ended {} instead", if w { "won" } else { "lost" }),
                (Some(_), _) if natural => "the mission ends the same way by itself, as soon, without the entry (a natural end, not credited)".to_owned(),
                (None, Some(why)) => format!("end blocked by a gap: {why}"),
                (None, None) => format!("did not end within {} ticks (last step {})", opts.cap, last.as_deref().unwrap_or("none")),
                _ => String::new(),
            };
            t.gaps = gaps;
        }
    }
    t
}

/// Probe one mission. `vfs` feeds the stand-in host (none: nothing answers a wait).
pub fn probe_mission(host: &dyn MissionHost, vfs: Option<&Arc<sta_vfs::Vfs>>, book: &CampaignBook, campaign: &str, index: usize, opts: &EndingOptions) -> EndingProbe {
    let c = book.campaign(campaign).expect("campaign in book");
    let m = &c.missions[index];
    let mut p = EndingProbe { campaign: campaign.to_owned(), index, bzn: m.bzn.clone(), ..Default::default() };
    let src = match host.script(c, m) {
        Ok((path, reader)) => match reader(&path) {
            Some(b) => String::from_utf8_lossy(&b).into_owned(),
            None => {
                p.start_error = Some(format!("{path} unreadable"));
                return p;
            }
        },
        Err(e) => {
            p.start_error = Some(e);
            return p;
        }
    };
    p.plan = plan(&src);
    // The test point itself, from the first trial.
    let point = at_test_point(book, host, vfs, campaign, index, opts, |runner, mid| (runner.active().map_or(0, |a| a.sim().tick), mid.first_objective_tick));
    match point {
        Ok((tick, obj)) => {
            p.started = true;
            p.test_tick = Some(tick);
            p.first_objective_tick = obj;
            if obj.is_none() {
                p.notes.push(format!("no objective issued by tick {}", opts.warm_max));
            }
        }
        Err(e) => {
            p.start_error = Some(e);
            return p;
        }
    }
    p.forced_win = Some(forced_trial(book, host, vfs, campaign, index, opts, true));
    // Armada II's loss cheat works in every mission; Armada I's loss is the test-only extension.
    p.forced_lose = Some(forced_trial(book, host, vfs, campaign, index, opts, false));
    if !opts.forced_only {
        p.baseline = baseline_trial(book, host, vfs, campaign, index, opts);
        let (win, lose) = (p.plan.win.clone(), p.plan.lose.clone());
        p.win_seq = win.iter().map(|e| sequence_trial(book, host, vfs, campaign, index, opts, e, true, p.baseline)).collect();
        p.lose_seq = lose.iter().map(|e| sequence_trial(book, host, vfs, campaign, index, opts, e, false, p.baseline)).collect();
    }
    if p.plan.truncated != (0, 0) {
        p.notes.push(format!("entries beyond {MAX_ENTRIES} not run: {} win, {} lose", p.plan.truncated.0, p.plan.truncated.1));
    }
    p
}

/// Probe every Lua mission of `book` (`only`: those whose BZN name starts with it).
pub fn probe_book(host: &dyn MissionHost, vfs: Option<&Arc<sta_vfs::Vfs>>, book: &CampaignBook, opts: &EndingOptions, only: Option<&str>) -> Vec<EndingProbe> {
    let mut out = Vec::new();
    for c in &book.campaigns {
        for m in &c.missions {
            let wanted = only.is_none_or(|o| m.bzn.to_ascii_lowercase().starts_with(&o.to_ascii_lowercase()));
            if wanted && matches!(m.script, MissionScript::Lua { .. }) {
                out.push(probe_mission(host, vfs, book, &c.id, m.index, opts));
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "ending_tests.rs"]
mod tests;
