//! LUA-14: the Armada I campaign missions converted to Lua by
//! `tools/a1_to_lua.py`, install-gated. The test converts at run time from the
//! install named by `STA64_TEST_A1_INSTALL` into the target directory (never
//! into the repository) and checks that the decode coverage does not regress
//! (counts only: no game text reaches the test or its output), that every
//! converted mission loads and runs on an empty view without an error in the
//! converted code, and how far mission 1 gets in the campaign runner with the
//! stand-in host. Needs `python3` and `objdump`; skipped when the variable is
//! not set.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_mission::campaign::*;
use sta_vfs::Vfs;

const TICKS: u64 = 9000;

/// Per mission: (rules, then for each role "complete+partial/total").
type Coverage = BTreeMap<String, (u32, BTreeMap<String, (u32, u32, u32)>)>;

fn convert(install: &str, only: Option<&str>) -> (PathBuf, Coverage) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("converted_a1_{}", only.unwrap_or("all")));
    let _ = std::fs::remove_dir_all(&out);
    let mut cmd = std::process::Command::new("python3");
    cmd.arg("-I").arg(repo.join("tools/a1_to_lua.py")).args(["--install", install]);
    if let Some(m) = only {
        cmd.args(["--only", m]);
    }
    let run = cmd.arg("--out").arg(&out).output().expect("run python3 tools/a1_to_lua.py");
    let report = String::from_utf8_lossy(&run.stdout).into_owned();
    eprintln!("{report}");
    assert!(run.status.success(), "{report}\n{}", String::from_utf8_lossy(&run.stderr));
    (out, coverage(&report))
}

/// `  fed1 (Federation1): rules 34, action 33+0p/33, condition 34+0p/34, ...`
fn coverage(report: &str) -> Coverage {
    let mut out = Coverage::new();
    for line in report.lines() {
        let Some((head, rest)) = line.trim().split_once(": rules ") else { continue };
        let map = head.split_whitespace().next().unwrap_or("").to_owned();
        let mut parts = rest.split(", ");
        let rules: u32 = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        let mut roles = BTreeMap::new();
        for p in parts {
            let Some((role, counts)) = p.split_once(' ') else { continue };
            let Some((done, total)) = counts.split_once('/') else { continue };
            let Some((complete, partial)) = done.trim_end_matches('p').split_once('+') else { continue };
            roles.insert(role.to_owned(), (complete.parse().unwrap_or(0), partial.parse().unwrap_or(0), total.parse().unwrap_or(0)));
        }
        out.insert(map, (rules, roles));
    }
    out
}

#[test]
fn a1_campaign_decode_coverage_does_not_regress() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let (out, cov) = convert(&install, None);
    assert_eq!(cov.len(), 20, "the 20 campaign maps convert: {:?}", cov.keys().collect::<Vec<_>>());
    let mut sum: BTreeMap<String, (u32, u32, u32)> = BTreeMap::new();
    let mut rules = 0;
    for (map, (r, roles)) in &cov {
        assert!(out.join(format!("missions/{map}.lua")).is_file(), "{map}");
        assert!(*r > 0, "{map}: no rules found");
        rules += r;
        for (role, (c, p, t)) in roles {
            let s = sum.entry(role.clone()).or_default();
            s.0 += c;
            s.1 += p;
            s.2 += t;
        }
    }
    eprintln!("rules {rules}; by role (complete, partial, total): {sum:?}");
    // Measured on the GOG Armada I install (LUA-14): every rule, condition,
    // action, step and init function decodes completely.
    assert_eq!(rules, 1050);
    for (role, total) in [("condition", 1050), ("action", 1005), ("step", 1005), ("init", 20)] {
        let (c, _, t) = sum.get(role).copied().unwrap_or_default();
        assert_eq!(t, total, "{role} count");
        assert_eq!(c, total, "{role}: {} not decoded completely", t - c);
    }
}

#[test]
fn every_converted_a1_mission_loads_and_runs() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let (out, _) = convert(&install, None);
    let mut files: Vec<PathBuf> = std::fs::read_dir(out.join("missions")).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    for f in files {
        let name = format!("missions/{}", f.file_name().unwrap().to_string_lossy());
        let source = std::fs::read_to_string(&f).unwrap();
        let mut rt = sta_mission::runtime::MissionRuntime::from_source(&name, &source, 7);
        assert!(rt.errors().is_empty(), "{name}: {:?}", rt.errors());
        for at in 0..300 {
            let view = sta_sim::WorldView { tick: at, ..sta_sim::WorldView::default() };
            let mut commands = Vec::new();
            sta_sim::setup::Controller::think(&mut rt, &view, &[], &mut commands);
        }
        assert!(rt.errors().is_empty(), "{name}: {:?}", rt.errors());
        let step_errors: Vec<String> = rt.eval("return Mission_.stepErrors").unwrap();
        let undecoded: u32 = rt.eval("local n = 0 for _, c in pairs(Mission_.undecoded) do n = n + c end return n").unwrap();
        let fired: Vec<String> = rt.eval("return Mission_.fired").unwrap();
        eprintln!("{name}: {} rule firings, {undecoded} undecoded calls ran", fired.len());
        assert!(step_errors.is_empty(), "{name}: {step_errors:?}");
        assert_eq!(undecoded, 0, "{name}");
    }
}

/// Mission 1 (`fed1`) on its own map in the campaign runner, headless, with
/// the stand-in host reporting sounds and movies after their file lengths.
#[test]
fn converted_a1_fed1_runs_in_the_campaign_runner() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let (out, _) = convert(&install, Some("fed1"));
    let mut vfs = Vfs::mount_install(Path::new(&install), None).expect("mount install");
    vfs.push_dir("converted".into(), out).expect("mount the converted missions");
    let book = CampaignBook::load_vfs(&vfs);
    let first = &book.campaigns.iter().find(|c| !c.missions.is_empty()).expect("a campaign").missions[0];
    assert_eq!(first.bzn.to_ascii_lowercase(), "fed1.bzn");
    assert!(
        matches!(&first.script, MissionScript::Lua { path } if path.eq_ignore_ascii_case("missions/fed1.lua")),
        "{:?}",
        first.script
    );
    let host = VfsHost::load(Arc::new(vfs)).expect("installed content loads");
    let probes = sta_mission::progress::probe_book(&host, &book, TICKS, true, Some("fed1"));
    assert_eq!(probes.len(), 1);
    let p = &probes[0];
    eprintln!(
        "[fed1] ticks {}, rules held {:?}, last rule {:?}, last step {:?}, objective at {:?}, problems {:?}",
        p.ticks,
        p.rules_held,
        p.last_rule,
        p.last_step,
        p.first_objective_tick,
        p.problems.iter().map(|q| (&q.0, &q.1)).collect::<Vec<_>>()
    );
    assert!(p.started, "{:?}", p.start_error);
    assert!(p.errors.is_empty(), "{:?}", p.errors);
    assert!(p.step_errors.is_empty(), "{:?}", p.step_errors);
    assert_eq!(p.rules, 34);
    assert!(p.rules_held.len() >= 5, "{:?}", p.rules_held);
    assert!(p.handles_resolved >= 10, "resolved {}, unresolved {:?}", p.handles_resolved, p.unresolved);
}
