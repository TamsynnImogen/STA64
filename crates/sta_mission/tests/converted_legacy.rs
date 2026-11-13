//! LUA-11: stock Armada II missions converted to Lua by `tools/dsl_to_lua.py`,
//! install-gated. The test converts at run time from the install named by
//! `STA64_TEST_A2_STOCK_INSTALL` into the target directory (never into the
//! repository), mounts the output as a top VFS layer (`missions/<map>.lua`
//! then shadows nothing and resolves before the legacy DLLs), and plays the
//! first Federation mission in the campaign runner on its own map. It prints
//! how far the converted script gets: rules fired, the last rule and step,
//! the calls that are unbound (`Mission_.missing`), that raised
//! (`Mission_.failed`) and the runtime's `unsupported()` counts. Needs
//! `python3` and `objdump`; skipped when the variable is not set.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_mission::campaign::*;
use sta_vfs::Vfs;

const TICKS: u64 = 9000;

fn convert(install: &str, only: Option<&str>) -> PathBuf {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("converted_legacy_{}", only.unwrap_or("all")));
    let _ = std::fs::remove_dir_all(&out);
    let mut cmd = std::process::Command::new("python3");
    cmd.arg("-I").arg(repo.join("tools/dsl_to_lua.py")).args(["--install", install]);
    if let Some(m) = only {
        cmd.args(["--only", m]);
    }
    let run = cmd
        .arg("--out")
        .arg(&out)
        .output()
        .expect("run python3 tools/dsl_to_lua.py");
    let report = String::from_utf8_lossy(&run.stdout);
    eprintln!("{report}");
    assert!(run.status.success(), "{report}\n{}", String::from_utf8_lossy(&run.stderr));
    out
}

fn table(rt: &sta_mission::runtime::MissionRuntime, field: &str) -> Vec<(String, u32)> {
    let mut v: Vec<(String, u32)> = rt
        .eval(&format!(
            "local t = {{}} for k, n in pairs(Mission_.{field}) do t[#t + 1] = {{k, n}} end \
             table.sort(t, function(a, b) return a[1] < b[1] end) \
             local out = {{}} for i, p in ipairs(t) do out[i] = p[1] .. '=' .. tostring(p[2]) end return out"
        ))
        .map(|l: Vec<String>| {
            l.into_iter()
                .filter_map(|s| s.rsplit_once('=').map(|(k, n)| (k.to_owned(), n.parse().unwrap_or(0))))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn converted_a2_fed01_runs_in_the_campaign_runner() {
    let Ok(install) = std::env::var("STA64_TEST_A2_STOCK_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL to a stock Armada II root");
        return;
    };
    let out = convert(&install, Some("a2_fed01"));
    assert!(out.join("missions/a2_fed01.lua").is_file());

    let mut vfs = Vfs::mount_install(Path::new(&install), None).expect("mount install");
    vfs.push_dir("converted".into(), out.clone()).expect("mount the converted missions");
    let book = CampaignBook::load_vfs(&vfs);
    let fed = book.campaign("federation").expect("federation campaign");
    let first = &fed.missions[0];
    assert_eq!(first.bzn.to_ascii_lowercase(), "a2_fed01.bzn");
    assert!(
        matches!(&first.script, MissionScript::Lua { path } if path.eq_ignore_ascii_case("missions/a2_fed01.lua")),
        "{:?}",
        first.script
    );

    let host = VfsHost::load(Arc::new(vfs)).expect("installed content loads");
    let mut runner = CampaignRunner::new(book);
    let a = runner.start(&host, "federation", 0, 1).expect("converted mission starts");
    let ended = a.run(TICKS);
    let rt = a.runtime();
    // `A2.<call>: simulation rejected ...` entries are counted edits the sim refused, not script errors.
    let script_errors: Vec<_> = rt.errors().iter().filter(|e| !e.contains(": simulation rejected")).collect();
    assert!(script_errors.is_empty(), "{script_errors:?}");

    let rules: u32 = rt.eval("return #Mission_.rules").unwrap();
    let fired: Vec<String> = rt.eval("return Mission_.fired").unwrap();
    let last_rule: Option<String> = rt.eval("return Mission_.lastRule").unwrap();
    let last_step: Option<String> = rt.eval("return Mission_.lastStep").unwrap();
    let steps: u32 = rt.eval("return Mission_.stepCount").unwrap();
    let step_errors: Vec<String> = rt.eval("return Mission_.stepErrors").unwrap();
    let not_decoded = table(rt, "notDecoded");
    let missing = table(rt, "missing");
    let failed = table(rt, "failed");
    let calls = table(rt, "calls");
    eprintln!("[a2_fed01] {TICKS} ticks, ended {ended:?}");
    eprintln!("[a2_fed01] rules {rules}, rule firings (false-to-true) {}, steps run {steps}", fired.len());
    let mut distinct: Vec<&String> = Vec::new();
    for r in &fired {
        if !distinct.contains(&r) {
            distinct.push(r);
        }
    }
    eprintln!("[a2_fed01] rules that held, in order of first firing: {distinct:?}");
    let ran: Vec<String> = rt.eval("return Mission_.ran").unwrap();
    let mut steps_seen: Vec<&String> = Vec::new();
    for r in &ran {
        if !steps_seen.contains(&r) {
            steps_seen.push(r);
        }
    }
    eprintln!("[a2_fed01] steps run, in order of first run: {steps_seen:?}");
    eprintln!("[a2_fed01] last rule {last_rule:?}, last step {last_step:?}");
    eprintln!("[a2_fed01] engine calls made: {calls:?}");
    eprintln!("[a2_fed01] unbound calls: {missing:?}");
    eprintln!("[a2_fed01] calls that raised: {failed:?}");
    eprintln!("[a2_fed01] functions not decoded that ran: {not_decoded:?}");
    eprintln!("[a2_fed01] runtime unsupported(): {:?}", rt.unsupported());
    eprintln!("[a2_fed01] step errors: {step_errors:?}");

    assert_eq!(rules, 50, "every rule of the rule DLL is registered");
    assert!(!fired.is_empty(), "the converted script fires its rules");
    assert!(steps > 0, "the converted script runs its action steps");
    assert!(step_errors.is_empty(), "converted code raised: {step_errors:?}");
}

/// LUA-11 progress: with the stand-in host answering the client's completion
/// reports, converted mission 1 plays its intro, passes the audio message the
/// next rule waits on and issues its first player objective (its main phase).
/// Measured: the objective appears about 2000 ticks in (stand-in lengths come
/// from the install's WAV headers); the assertion only needs it within 9000.
#[test]
fn converted_a2_fed01_reaches_its_first_objective_headless() {
    let Ok(install) = std::env::var("STA64_TEST_A2_STOCK_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL to a stock Armada II root");
        return;
    };
    let out = convert(&install, Some("a2_fed01"));
    let mut vfs = Vfs::mount_install(Path::new(&install), None).expect("mount install");
    vfs.push_dir("converted".into(), out).expect("mount the converted missions");
    let book = CampaignBook::load_vfs(&vfs);
    let host = VfsHost::load(Arc::new(vfs)).expect("installed content loads");
    let probes = sta_mission::progress::probe_book(&host, &book, TICKS, true, Some("a2_fed01"));
    assert_eq!(probes.len(), 1);
    let p = &probes[0];
    eprintln!("[a2_fed01] objective at tick {:?}, rules held {}, problems {:?}", p.first_objective_tick, p.rules_held.len(), p.problems.iter().map(|q| &q.0).collect::<Vec<_>>());
    assert!(p.started, "{:?}", p.start_error);
    assert!(p.step_errors.is_empty(), "{:?}", p.step_errors);
    assert!(p.first_objective_tick.is_some(), "no objective issued; last rule {:?}, blocked {:?}", p.last_rule, p.blocked);
    assert!(p.reports > 0, "the stand-in host reported nothing");
    // Labels of objects the mission builds itself (turrets, mines, planets it
    // places) name nothing until then: the invalid handle, as in the original.
    assert!(p.handles_resolved > 20, "resolved {}, unresolved labels {:?}", p.handles_resolved, p.unresolved);
}

/// Every stock mission converts, loads in the runtime and runs 300 ticks on
/// an empty view without an error in the converted code.
#[test]
fn every_converted_a2_mission_loads_and_runs() {
    let Ok(install) = std::env::var("STA64_TEST_A2_STOCK_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL to a stock Armada II root");
        return;
    };
    let out = convert(&install, None);
    let mut files: Vec<PathBuf> = std::fs::read_dir(out.join("missions")).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    assert_eq!(files.len(), 36, "{files:?}");
    // the UI tutorial's map runs a2_Tutorial1: its script is named by the map's stem
    assert!(files.iter().any(|f| f.ends_with("a2_tutorial_ui.lua")), "{files:?}");
    let mut all: std::collections::BTreeMap<String, bool> = Default::default();
    for f in files {
        let name = format!("missions/{}", f.file_name().unwrap().to_string_lossy());
        let source = std::fs::read_to_string(&f).unwrap();
        // no address is handed to Lua as a table, and every function decodes (the std::vector members are modelled)
        assert!(!source.contains("Ref("), "{name}: a by-address argument was left as a table");
        assert_eq!(source.matches("(not decoded:").count(), 0, "{name}: an undecoded function");
        let mut rt = sta_mission::runtime::MissionRuntime::from_source(&name, &source, 7);
        assert!(rt.errors().is_empty(), "{name}: {:?}", rt.errors());
        for at in 0..300 {
            let view = sta_sim::WorldView { tick: at, ..sta_sim::WorldView::default() };
            let mut commands = Vec::new();
            sta_sim::setup::Controller::think(&mut rt, &view, &[], &mut commands);
        }
        assert!(rt.errors().is_empty(), "{name}: {:?}", rt.errors());
        let step_errors: Vec<String> = rt.eval("return Mission_.stepErrors").unwrap();
        let fired: Vec<String> = rt.eval("return Mission_.fired").unwrap();
        let not_decoded: u32 = rt.eval("local n = 0 for _ in pairs(Mission_.notDecoded) do n = n + 1 end return n").unwrap();
        eprintln!("{name}: {} rule firings, {not_decoded} undecoded functions ran", fired.len());
        assert!(step_errors.is_empty(), "{name}: {step_errors:?}");
        let names: Vec<(String, bool)> = rt
            .eval("local t = {} for i, n in ipairs(Mission_.engineNames) do \
                   local f = n:find('.', 1, true) and nil or (A2 and A2[n]) \
                   t[i] = n .. '=' .. tostring(type(f) == 'function') end return t")
            .map(|l: Vec<String>| l.into_iter().filter_map(|s| s.rsplit_once('=').map(|(n, b)| (n.to_owned(), b == "true"))).collect())
            .unwrap();
        for (n, bound) in names {
            all.insert(n, bound);
        }
    }
    let bound: Vec<&String> = all.iter().filter(|(_, b)| **b).map(|(n, _)| n).collect();
    let unbound: Vec<&String> = all.iter().filter(|(_, b)| !**b).map(|(n, _)| n).collect();
    eprintln!("engine calls in the converted missions: {} distinct, {} bound on A2, {} not", all.len(), bound.len(), unbound.len());
    eprintln!("not bound: {unbound:?}");
}
