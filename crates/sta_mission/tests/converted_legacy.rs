//! LUA-11: stock Armada II missions converted to Lua, install-gated on
//! `STA64_TEST_A2_STOCK_INSTALL` (skipped when the variable is not set). The tests convert at run
//! time with the Rust converter (`sta_mission_convert`, the library the game calls; no Python, no
//! `objdump`) into a temporary folder that is removed with the test, mount the output as a top VFS
//! layer (`missions/<map>.lua` then shadows nothing and resolves before the legacy DLLs), and play
//! the first mission of the first campaign in the campaign runner on its own map.
//!
//! Nothing here names a mission or pins a count taken from the game: which mission is played, how
//! many rules it has and how many scripts there are come from the install and from what the
//! conversion reports, at run time. Diagnostics are printed as counts only.

#[path = "support/convert.rs"]
mod support;

use std::path::Path;
use std::sync::Arc;

use sta_mission::campaign::*;
use sta_vfs::Vfs;

const TICKS: u64 = 9000;

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

/// The install with its whole conversion mounted on top, and the conversion.
fn converted(install: &str, tag: &str) -> (Vfs, support::Conversion) {
    let conv = support::convert(Path::new(install), tag, &[], false);
    let mut vfs = Vfs::mount_install(Path::new(install), None).expect("mount install");
    vfs.push_dir("converted".into(), conv.dir()).expect("mount the converted missions");
    (vfs, conv)
}

/// The first campaign of the book whose first mission runs a converted script: its id and that
/// mission's map stem (lower case).
fn first_converted_mission(book: &CampaignBook, conv: &support::Conversion) -> (String, String) {
    let written = conv.run.written();
    book.campaigns
        .iter()
        .find_map(|c| {
            let first = c.missions.first()?;
            let stem = first.stem().to_ascii_lowercase();
            written.contains(&format!("{stem}.lua")).then(|| (c.id.clone(), stem))
        })
        .expect("a campaign whose first mission was converted")
}

/// The number of rules the converter counted for the mission that wrote `script`, from the
/// counts its result carries (#1378): a mission without them fails the test.
fn reported_rules(conv: &support::Conversion, script: &str) -> usize {
    let mission = conv.run.summary.missions.iter().find(|m| m.written.iter().any(|f| f.eq_ignore_ascii_case(script))).expect("the mission that wrote the script");
    let counts = mission.counts.as_ref().expect("the converted mission's result carries its counts");
    assert_eq!(counts.rule_conditions_decoded, Some(counts.rules), "every rule of the mission decodes");
    counts.rules
}

#[test]
fn the_first_converted_mission_runs_in_the_campaign_runner() {
    let Ok(install) = std::env::var("STA64_TEST_A2_STOCK_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL to a stock Armada II root");
        return;
    };
    let (vfs, conv) = converted(&install, "legacy-runner");
    let book = CampaignBook::load_vfs(&vfs);
    let (campaign, stem) = first_converted_mission(&book, &conv);
    let script = format!("{stem}.lua");
    assert!(conv.dir().join("missions").join(&script).is_file());
    let first = &book.campaign(&campaign).unwrap().missions[0];
    assert!(
        matches!(&first.script, MissionScript::Lua { path } if path.eq_ignore_ascii_case(&format!("missions/{script}"))),
        "the first mission resolves to its converted script"
    );

    let host = VfsHost::load(Arc::new(vfs)).expect("installed content loads");
    let mut runner = CampaignRunner::new(book);
    let a = runner.start(&host, &campaign, 0, 1).expect("converted mission starts");
    let ended = a.run(TICKS);
    let rt = a.runtime();
    // `A2.<call>: simulation rejected ...` entries are counted edits the sim refused, not script errors.
    let script_errors: Vec<_> = rt.errors().iter().filter(|e| !e.contains(": simulation rejected")).collect();
    assert!(script_errors.is_empty(), "{script_errors:?}");

    let rules: u32 = rt.eval("return #Mission_.rules").unwrap();
    let fired: Vec<String> = rt.eval("return Mission_.fired").unwrap();
    let steps: u32 = rt.eval("return Mission_.stepCount").unwrap();
    let step_errors: Vec<String> = rt.eval("return Mission_.stepErrors").unwrap();
    eprintln!("first mission: {TICKS} ticks, ended {ended:?}; rules {rules}, rule firings {}, steps run {steps}", fired.len());
    eprintln!(
        "engine calls made {}, unbound {}, raised {}, undecoded functions that ran {}, runtime unsupported {}",
        table(rt, "calls").len(),
        table(rt, "missing").len(),
        table(rt, "failed").len(),
        table(rt, "notDecoded").len(),
        rt.unsupported().len()
    );

    assert_eq!(rules as usize, reported_rules(&conv, &script), "every rule the converter reports for the mission is registered");
    assert!(rules > 0 && !fired.is_empty(), "the converted script fires its rules");
    assert!(steps > 0, "the converted script runs its action steps");
    assert!(step_errors.is_empty(), "converted code raised: {step_errors:?}");
}

/// LUA-11 progress: with the stand-in host answering the client's completion reports, the first
/// converted mission plays its intro, passes the audio message the next rule waits on and issues
/// its first player objective (its main phase), within the ticks given.
#[test]
fn the_first_converted_mission_reaches_its_first_objective_headless() {
    let Ok(install) = std::env::var("STA64_TEST_A2_STOCK_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL to a stock Armada II root");
        return;
    };
    let (vfs, conv) = converted(&install, "legacy-objective");
    let book = CampaignBook::load_vfs(&vfs);
    let (_, stem) = first_converted_mission(&book, &conv);
    let host = VfsHost::load(Arc::new(vfs)).expect("installed content loads");
    // exactly that map: its stem and the dot before the extension
    let probes = sta_mission::progress::probe_book(&host, &book, TICKS, true, Some(&format!("{stem}.")));
    assert_eq!(probes.len(), 1);
    let p = &probes[0];
    eprintln!("first mission: objective at tick {:?}, rules held {}, problems {}", p.first_objective_tick, p.rules_held.len(), p.problems.len());
    assert!(p.started, "{:?}", p.start_error);
    assert!(p.step_errors.is_empty(), "{:?}", p.step_errors);
    assert!(p.first_objective_tick.is_some(), "no objective issued");
    assert!(p.reports > 0, "the stand-in host reported nothing");
    // Labels of objects the mission builds itself name nothing until then (the invalid handle, as
    // in the original); the ones its map places resolve.
    assert!(p.handles_resolved > 0, "no label of the map resolved ({} unresolved)", p.unresolved.len());
}

/// Every mission the conversion wrote loads in the runtime and runs 300 ticks on an empty view
/// without an error in the converted code.
#[test]
fn every_converted_a2_mission_loads_and_runs() {
    let Ok(install) = std::env::var("STA64_TEST_A2_STOCK_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL to a stock Armada II root");
        return;
    };
    let conv = support::convert(Path::new(&install), "legacy-all", &[], false);
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(conv.dir().join("missions")).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "lua")).collect();
    files.sort();
    // what is on disk is what the run says it wrote, and it wrote something
    let on_disk: std::collections::BTreeSet<String> = files.iter().map(|f| f.file_name().unwrap().to_string_lossy().to_ascii_lowercase()).collect();
    assert_eq!(on_disk, conv.run.written());
    assert!(!files.is_empty(), "the conversion wrote no script");
    assert_eq!(conv.run.summary.failed_missions(), 0, "a mission of the install did not convert");
    let mut all: std::collections::BTreeMap<String, bool> = Default::default();
    for (i, f) in files.iter().enumerate() {
        let name = format!("missions/{}", f.file_name().unwrap().to_string_lossy());
        let source = std::fs::read_to_string(f).unwrap();
        // no address is handed to Lua as a table, and every function decodes (the std::vector members are modelled)
        assert!(!source.contains("Ref("), "script #{i}: a by-address argument was left as a table");
        assert_eq!(source.matches("(not decoded:").count(), 0, "script #{i}: an undecoded function");
        let mut rt = sta_mission::runtime::MissionRuntime::from_source(&name, &source, 7);
        assert!(rt.errors().is_empty(), "script #{i}: {:?}", rt.errors());
        for at in 0..300 {
            let view = sta_sim::WorldView { tick: at, ..sta_sim::WorldView::default() };
            let mut commands = Vec::new();
            sta_sim::setup::Controller::think(&mut rt, &view, &[], &mut commands);
        }
        assert!(rt.errors().is_empty(), "script #{i}: {:?}", rt.errors());
        let step_errors: Vec<String> = rt.eval("return Mission_.stepErrors").unwrap();
        assert!(step_errors.is_empty(), "script #{i}: {step_errors:?}");
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
    let bound = all.values().filter(|b| **b).count();
    eprintln!("{} scripts; engine calls in them: {} distinct, {bound} bound on A2, {} not", files.len(), all.len(), all.len() - bound);
}
