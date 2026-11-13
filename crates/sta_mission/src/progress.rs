//! Converted-mission progress probe (LUA-11): each mission of a campaign book
//! whose script is a converted Lua file (`tools/dsl_to_lua.py`) runs N ticks
//! headless in the campaign runner, optionally with the [`StandInHost`]
//! answering the client's completion reports, and the probe records how far it
//! got: the last rule that became satisfied, the last step that ran, the first
//! calls that were unbound or raised (in order), the rules whose condition
//! could not be evaluated, scripts still held by a wait, and the tick of the
//! first player objective. `tools/campaign_readiness.py --converted` prints the
//! table. Nothing here reads a game file except through [`VfsHost`] and the
//! stand-in's file headers.
//!
//! Evidence grade: measured (a run of N ticks, no player input). "Progress" is
//! how far the converted script's rules and steps run; it is not a win.

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::campaign::*;
use crate::readiness::json_string;
use crate::standin::StandInHost;

#[derive(Debug, Clone, Default)]
pub struct ProgressProbe {
    pub campaign: String,
    pub index: usize,
    pub bzn: String,
    pub started: bool,
    pub start_error: Option<String>,
    pub ticks: u64,
    pub ended: Option<bool>,
    /// The script asked for an end that depends on an object that never
    /// existed (LUA-13): neither a win nor a loss, and why.
    pub end_blocked: Option<String>,
    pub errors: Vec<String>,
    pub rules: u32,
    /// Distinct rules that became satisfied, in order of first time.
    pub rules_held: Vec<String>,
    pub last_rule: Option<String>,
    pub last_step: Option<String>,
    pub steps_run: u32,
    pub distinct_steps: u32,
    /// (call, `unbound`|`raised`, mission seconds, error text), in order of first occurrence.
    pub problems: Vec<(String, String, f64, String)>,
    pub missing: Vec<(String, u32)>,
    pub failed: Vec<(String, u32)>,
    /// Calls answered 0 because a handle was negative (no object).
    pub invalid: Vec<(String, u32)>,
    /// `GetHandle` labels that named no object (answered 0), and how many `GetHandle` calls resolved.
    pub unresolved: Vec<String>,
    pub handles_resolved: u32,
    /// Rules whose condition could not be evaluated: (rule, first call that failed in it).
    pub blocked: Vec<(String, String)>,
    /// Scripts still queued at the end: (name, steps left, waits held).
    pub scripts: Vec<(String, u32, i64)>,
    pub step_errors: Vec<String>,
    pub objectives: usize,
    pub first_objective_tick: Option<u64>,
    pub reports: usize,
    pub unsupported: Vec<(String, u32)>,
    /// Reads of variables the engine writes (team strength, moons, warp tries): the script's
    /// counted `engineReads`, for a host to bind (`A2.EngineVariable`) later.
    pub engine_reads: Vec<(String, u32)>,
}

fn counted(v: &[(String, u32)]) -> String {
    format!("[{}]", v.iter().map(|(k, n)| format!("[{},{}]", json_string(k), n)).collect::<Vec<_>>().join(","))
}

fn list(v: &[String]) -> String {
    format!("[{}]", v.iter().map(|x| json_string(x)).collect::<Vec<_>>().join(","))
}

impl ProgressProbe {
    pub fn to_json(&self) -> String {
        let opt = |o: &Option<String>| o.as_deref().map(json_string).unwrap_or_else(|| "null".into());
        let problems: Vec<String> = self
            .problems
            .iter()
            .map(|(n, k, t, w)| format!("[{},{},{:.2},{}]", json_string(n), json_string(k), t, json_string(w)))
            .collect();
        let blocked: Vec<String> = self.blocked.iter().map(|(r, c)| format!("[{},{}]", json_string(r), json_string(c))).collect();
        let scripts: Vec<String> =
            self.scripts.iter().map(|(n, left, w)| format!("[{},{},{}]", json_string(n), left, w)).collect();
        format!(
            "{{\"campaign\":{},\"index\":{},\"bzn\":{},\"started\":{},\"start_error\":{},\"ticks\":{},\"ended\":{},\"end_blocked\":{},\"errors\":{},\"rules\":{},\"rules_held\":{},\"last_rule\":{},\"last_step\":{},\"steps_run\":{},\"distinct_steps\":{},\"problems\":[{}],\"missing\":{},\"failed\":{},\"invalid\":{},\"unresolved\":{},\"handles_resolved\":{},\"blocked\":[{}],\"scripts\":[{}],\"step_errors\":{},\"objectives\":{},\"first_objective_tick\":{},\"reports\":{},\"unsupported\":{},\"engine_reads\":{}}}",
            json_string(&self.campaign),
            self.index,
            json_string(&self.bzn),
            self.started,
            opt(&self.start_error),
            self.ticks,
            match self.ended {
                Some(true) => "true",
                Some(false) => "false",
                None => "null",
            },
            opt(&self.end_blocked),
            list(&self.errors),
            self.rules,
            list(&self.rules_held),
            opt(&self.last_rule),
            opt(&self.last_step),
            self.steps_run,
            self.distinct_steps,
            problems.join(","),
            counted(&self.missing),
            counted(&self.failed),
            counted(&self.invalid),
            list(&self.unresolved),
            self.handles_resolved,
            blocked.join(","),
            scripts.join(","),
            list(&self.step_errors),
            self.objectives,
            self.first_objective_tick.map_or("null".into(), |t| t.to_string()),
            self.reports,
            counted(&self.unsupported),
            counted(&self.engine_reads),
        )
    }
}

fn counts(rt: &crate::runtime::MissionRuntime, field: &str) -> Vec<(String, u32)> {
    let mut v: Vec<(String, u32)> = rt
        .eval(&format!(
            "local t = {{}} for k, n in pairs(Mission_.{field}) do t[#t + 1] = k .. '\\t' .. tostring(n) end table.sort(t) return t"
        ))
        .map(|l: Vec<String>| {
            l.into_iter().filter_map(|s| s.rsplit_once('\t').map(|(k, n)| (k.to_owned(), n.parse().unwrap_or(0)))).collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn read(rt: &crate::runtime::MissionRuntime, p: &mut ProgressProbe) {
    p.rules = rt.eval("return #Mission_.rules").unwrap_or(0);
    let fired: Vec<String> = rt.eval("return Mission_.fired").unwrap_or_default();
    for r in fired {
        if !p.rules_held.contains(&r) {
            p.rules_held.push(r);
        }
    }
    p.last_rule = rt.eval("return Mission_.lastRule").unwrap_or(None);
    p.last_step = rt.eval("return Mission_.lastStep").unwrap_or(None);
    p.steps_run = rt.eval("return Mission_.stepCount").unwrap_or(0);
    p.distinct_steps = rt
        .eval("local seen, n = {}, 0 for _, s in ipairs(Mission_.ran) do if not seen[s] then seen[s] = true n = n + 1 end end return n")
        .unwrap_or(0);
    p.problems = rt
        .eval::<Vec<String>>(
            "local o = {} for i, q in ipairs(Mission_.problemOrder or {}) do \
             o[i] = q[1] .. '\\t' .. q[2] .. '\\t' .. tostring(q[3]) .. '\\t' .. tostring(q[4] or ''):gsub('[\\r\\n\\t]+', ' ') end return o",
        )
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| {
            let mut f = s.splitn(4, '\t');
            Some((f.next()?.into(), f.next()?.into(), f.next()?.parse().ok()?, f.next().unwrap_or("").chars().take(160).collect()))
        })
        .collect();
    p.missing = counts(rt, "missing");
    p.failed = counts(rt, "failed");
    p.invalid = counts(rt, "invalid");
    p.unresolved = rt
        .eval::<Vec<String>>("local o = {} for k in pairs(Mission_.unresolved or {}) do o[#o + 1] = k end table.sort(o) return o")
        .unwrap_or_default();
    p.handles_resolved = rt.eval("return Mission_.resolved or 0").unwrap_or(0);
    p.blocked = rt
        .eval::<Vec<String>>("local o = {} for r, c in pairs(Mission_.blocked or {}) do o[#o + 1] = r .. '\\t' .. tostring(c) end table.sort(o) return o")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| s.split_once('\t').map(|(a, b)| (a.to_owned(), b.to_owned())))
        .collect();
    p.scripts = rt
        .eval::<Vec<String>>(
            "local o = {} for _, s in ipairs(Mission_.scripts) do \
             local w = 0 if A2 and A2.ScriptWaits then local ok, n = pcall(A2.ScriptWaits, s) if ok then w = n end end \
             o[#o + 1] = s.name .. '\\t' .. (#s.steps - s.at + 1) .. '\\t' .. w end return o",
        )
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| {
            let mut f = s.splitn(3, '\t');
            Some((f.next()?.to_owned(), f.next()?.parse().ok()?, f.next()?.parse().ok()?))
        })
        .collect();
    p.step_errors = rt.eval("return Mission_.stepErrors").unwrap_or_default();
    p.unsupported = rt.unsupported();
    p.engine_reads = counts(rt, "engineReads");
}

/// Run one mission up to `ticks` ticks and describe how far it got. `standin`
/// runs the [`StandInHost`] after every tick.
pub fn probe_mission(host: &VfsHost, book: &CampaignBook, campaign: &str, index: usize, ticks: u64, standin: bool) -> ProgressProbe {
    let m = &book.campaigns.iter().find(|c| c.id == campaign).expect("campaign in book").missions[index];
    let mut p = ProgressProbe { campaign: campaign.to_owned(), index, bzn: m.bzn.clone(), ..Default::default() };
    let mut open = book.clone();
    open.give_all_missions = true;
    let mut progress = CampaignProgress::new(&open);
    progress.apply_give_all(&open);
    let mut runner = CampaignRunner::with_progress(open, progress);
    let started = std::cell::Cell::new(false);
    let run = catch_unwind(AssertUnwindSafe(|| match runner.start(host, campaign, index, 1) {
        Err(e) => Err(e.to_string()),
        Ok(a) => {
            started.set(true);
            let mut hosted = standin.then(|| StandInHost::new(host.vfs.clone()));
            let mut ended = None;
            let mut first_objective = None;
            for _ in 0..ticks {
                ended = a.tick();
                if let Some(h) = hosted.as_mut() {
                    h.after_tick(a);
                }
                if first_objective.is_none() && !a.runtime().objectives().is_empty() {
                    first_objective = Some(a.sim().tick);
                }
                if ended.is_some() {
                    break;
                }
            }
            read(a.runtime(), &mut p);
            p.errors = a.runtime().errors().to_vec();
            p.objectives = a.runtime().objectives().len();
            p.first_objective_tick = first_objective;
            p.reports = hosted.map_or(0, |h| h.reports.len());
            if let crate::runtime::MissionEndState::Blocked(why) = a.runtime().mission_end(a.sim().tick) {
                p.end_blocked = Some(why);
            }
            Ok((a.sim().tick, ended.map(|o| o.won)))
        }
    }));
    match run {
        Err(_) => {
            p.started = started.get();
            if p.started {
                p.errors.push("panic while running".into());
            } else {
                p.start_error = Some("panic while starting".into());
            }
        }
        Ok(Err(e)) => p.start_error = Some(e),
        Ok(Ok((tick, ended))) => {
            p.started = true;
            p.ticks = tick;
            p.ended = ended;
        }
    }
    p
}

/// Probe every mission of `book` that resolves to a Lua script.
pub fn probe_book(host: &VfsHost, book: &CampaignBook, ticks: u64, standin: bool, only: Option<&str>) -> Vec<ProgressProbe> {
    let mut out = Vec::new();
    for c in &book.campaigns {
        for m in &c.missions {
            let wanted = only.is_none_or(|o| m.bzn.to_ascii_lowercase().starts_with(&o.to_ascii_lowercase()));
            if wanted && matches!(m.script, MissionScript::Lua { .. }) {
                out.push(probe_mission(host, book, &c.id, m.index, ticks, standin));
            }
        }
    }
    out
}
