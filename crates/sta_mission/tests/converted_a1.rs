//! LUA-14: the Armada I campaign missions converted to Lua, install-gated on
//! `STA64_TEST_A1_INSTALL` (skipped when the variable is not set). The tests convert at run time
//! with the Rust converter (`sta_mission_convert`, the library the game calls; no Python, no
//! `objdump`) into a temporary folder that is removed with the test, and check that every function
//! of every mission decodes completely (from the converter's own report: no game text reaches the
//! test or its output), that every converted mission loads and runs on an empty view without an
//! error in the converted code, and how far the first mission gets in the campaign runner with the
//! stand-in host.
//!
//! Nothing here names a mission or pins a count taken from the game: which mission is the first,
//! how many there are and how many rules each has come from the install and from what the
//! conversion reports, at run time.

#[path = "support/convert.rs"]
mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_mission::campaign::*;
use sta_vfs::Vfs;

const TICKS: u64 = 9000;

/// Per mission (the map's stem, lower case): (rules, then for each role "complete+partial/total").
type Coverage = BTreeMap<String, (u32, BTreeMap<String, (u32, u32, u32)>)>;

/// Converts `only` (everything when `None`) into a temporary folder of the test's own.
fn convert(install: &str, only: Option<&str>, tag: &str, unmapped: bool) -> (support::Conversion, Coverage) {
    let only: Vec<&str> = only.into_iter().collect();
    let conv = support::convert(Path::new(install), tag, &only, unmapped);
    let report = conv.run.console.join("\n");
    assert!(!report.contains("registration:"), "rule set-up code of a mission was not converted");
    let cov = coverage(&report);
    (conv, cov)
}

/// `  fed1 (Federation1): rules 34, action 33+0p/33, condition 34+0p/34, ...`
fn coverage(report: &str) -> Coverage {
    let mut out = Coverage::new();
    for line in report.lines() {
        let Some((head, rest)) = line.trim().split_once(": rules ") else { continue };
        let map = head.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
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
    let (conv, cov) = convert(&install, None, "a1-coverage", false);
    assert!(!cov.is_empty(), "the converter reported no mission");
    assert_eq!(conv.run.summary.failed_missions(), 0, "a mission of the install did not convert");
    assert_eq!(cov.len(), conv.run.converted().len(), "one report line for every converted mission");
    let mut sum: BTreeMap<String, (u32, u32, u32)> = BTreeMap::new();
    let mut rules = 0;
    for (i, (map, (r, roles))) in cov.iter().enumerate() {
        assert!(conv.dir().join(format!("missions/{map}.lua")).is_file(), "mission #{i}: its script was written");
        assert!(*r > 0, "mission #{i}: no rules found");
        rules += r;
        for (role, (c, p, t)) in roles {
            // every rule, condition, action, step and init function decodes completely
            assert!(c == t && *p == 0, "mission #{i} {role}: {c}+{p}p of {t} decoded");
            let s = sum.entry(role.clone()).or_default();
            s.0 += c;
            s.1 += p;
            s.2 += t;
        }
    }
    eprintln!("{} missions, rules {rules}; by role (complete, partial, total): {sum:?}", cov.len());
    // What the totals must be follows from the conversion itself: one condition per rule, one
    // init function per mission.
    assert_eq!(sum.get("condition").map(|s| s.2), Some(rules), "one condition for every rule");
    assert_eq!(sum.get("init").map(|s| s.2), Some(cov.len() as u32), "one init function for every mission");
}

/// The DLL pairs no `.bzn` names are cut campaign content: neither the maps, nor `mshell.set`
/// (the campaign list), nor the objective texts reach them (measured, see the LUA-14 follow-up).
/// `--include-unmapped` converts them anyway under their DLL's name; the default does not.
#[test]
fn unmapped_a1_dll_pairs_convert_only_on_request() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let (_, default) = convert(&install, None, "a1-mapped", false);
    let (conv, all) = convert(&install, None, "a1-unmapped", true);
    assert!(default.keys().all(|k| all.contains_key(k)), "the request only adds missions");
    let extra: Vec<&String> = all.keys().filter(|k| !default.contains_key(*k)).collect();
    if extra.is_empty() {
        eprintln!("this install has no DLL pair without a map: nothing more to check");
        return;
    }
    for (i, map) in extra.iter().enumerate() {
        assert!(conv.dir().join(format!("missions/{map}.lua")).is_file(), "unmapped pair #{i}: its script was written");
        let (rules, roles) = &all[*map];
        assert!(*rules > 0, "unmapped pair #{i}: no rules found");
        // all but at most one function per role decode completely
        for (role, (c, p, t)) in roles {
            assert!(t - c <= 1 && *p <= 1, "unmapped pair #{i} {role}: {c}+{p}/{t}");
        }
    }
    eprintln!("{} mapped missions, {} more on request", default.len(), extra.len());
}

#[test]
fn every_converted_a1_mission_loads_and_runs() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let (conv, _) = convert(&install, None, "a1-load", false);
    let mut files: Vec<PathBuf> = std::fs::read_dir(conv.dir().join("missions")).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "lua")).collect();
    files.sort();
    assert!(!files.is_empty(), "the conversion wrote no script");
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

/// The first mission of the first campaign, launched through the campaign host path with Armada I content:
/// its human seat has a crew pool and an officer limit equal to what the map's team table gives team 1
/// (read from the install at run time; no value is in this source).
#[test]
fn a1_first_mission_seat_starts_with_the_maps_crew_and_officer_limit() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let (conv, _) = convert(&install, None, "a1-first-start", false);
    let mut vfs = Vfs::mount_install(Path::new(&install), None).expect("mount install");
    vfs.push_dir("converted".into(), conv.dir()).expect("mount the converted missions");
    let book = CampaignBook::load_vfs(&vfs);
    let campaign = book.campaigns.iter().find(|c| !c.missions.is_empty()).expect("a campaign");
    let (id, bzn) = (campaign.id.clone(), campaign.missions[0].bzn.clone());
    let bytes = vfs.read(&format!("bzn/{bzn}")).expect("the first mission's map").expect("readable");
    let table = sta_bzn::a1::team_starts(&bytes);
    assert_eq!(table.len(), 8, "the map's team table is read");
    let host = VfsHost::load(Arc::new(vfs)).expect("installed content loads");
    let mut runner = CampaignRunner::new(book);
    let mission = runner.start(&host, &id, 0, 7).expect("the first mission starts");
    for _ in 0..3 {
        mission.tick();
    }
    let view = sta_sim::WorldSource::view(mission.sim());
    use sta_core::{Fixed, PlayerId};
    let first = &table[0];
    assert!(first.crew > 0.0 && first.officers > 0, "else the test cannot prove anything for the human seat");
    // Every seat that owns a unit at the start, with a non-zero table value, holds exactly that value
    // (the record is the cap shown at the start; the written base is the record less the starting stations' gain).
    let mut occupied: Vec<u8> = view.units.iter().map(|u| u.owner.0).filter(|&o| usize::from(o) < table.len()).collect();
    occupied.sort_unstable();
    occupied.dedup();
    assert!(occupied.contains(&0), "the human seat owns units");
    for s in occupied {
        let (t, seat) = (&table[usize::from(s)], PlayerId(s));
        let p = view.player(seat).unwrap();
        let written = view.attrs.seats.iter().find(|a| a.player == seat).and_then(|a| a.officer_base);
        if t.crew > 0.0 {
            assert_eq!(p.crew, Some(Fixed::from_int(t.crew as i32)), "seat {s} crew");
        }
        if t.officers > 0 {
            assert!(written.is_some(), "seat {s} officer base is written");
            assert_eq!(p.officers[1], Fixed::from_int(t.officers), "seat {s} shown cap equals the record");
        }
    }
}

/// For every Armada I map of the install, launched with the mission start path and an empty script: the cap
/// shown at the start of every seat that owns a unit equals the map's record (read at run time).
#[test]
fn a1_every_maps_start_cap_is_its_record() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    use sta_core::{Fixed, PlayerId};
    use sta_sim::WorldSource;
    let vfs = Vfs::mount_install(Path::new(&install), None).expect("mount install");
    let host = VfsHost::load(Arc::new(vfs)).expect("installed content loads");
    let mut checked = 0;
    for name in host.vfs.list("bzn") {
        let Some(bzn) = name.strip_prefix("bzn/").filter(|n| n.to_ascii_lowercase().ends_with(".bzn")) else { continue };
        let Some(Ok(bytes)) = host.vfs.read(&name) else { continue };
        let table = sta_bzn::a1::team_starts(&bytes);
        if table.len() != 8 {
            continue;
        }
        let map = sta_bzn::parse(&bytes).expect("map parses");
        let (map, _) = sta_content::MapDef::from_bzn_in(&map, &host.content, &host.vfs, &host.index);
        let setup = map_setup(map, host.content.clone(), 1);
        let mut sim = sta_sim::Sim::from_setup(&setup);
        let reader: ScriptReader = Arc::new(|p| (p == "m.lua").then(Vec::new));
        let start = sta_mission::start_stock::read_start_stock(&host.vfs, bzn);
        let mut rt = launch_runtime_with_start("m.lua", reader, 1, (PlayerId(0), sta_mission::api::PlayerNumbering::ArmadaTeams), &setup, &sim, Vec::new(), Vec::new(), start).expect("launch");
        let mut view = (*sim.view()).clone();
        for _ in 0..2 {
            let mut out = Vec::new();
            sta_sim::setup::Controller::think(&mut rt, &view, &[], &mut out);
            view = (*sim.step(&out).view).clone();
        }
        let mut owners: Vec<u8> = view.units.iter().map(|u| u.owner.0).filter(|&o| usize::from(o) < table.len()).collect();
        owners.sort_unstable();
        owners.dedup();
        for s in owners {
            let record = table[usize::from(s)].officers;
            if record > 0 {
                assert_eq!(view.player(PlayerId(s)).unwrap().officers[1], Fixed::from_int(record), "{bzn} seat {s}");
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "at least one seat was compared");
}

/// The first mission of the first campaign on its own map in the campaign runner, headless, with
/// the stand-in host reporting sounds and movies after their file lengths.
#[test]
fn the_first_converted_a1_mission_runs_in_the_campaign_runner() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    // which map that is comes from the install's own campaign list
    let stem = {
        let vfs = Vfs::mount_install(Path::new(&install), None).expect("mount install");
        let book = CampaignBook::load_vfs(&vfs);
        book.campaigns.iter().find(|c| !c.missions.is_empty()).expect("a campaign").missions[0].stem().to_ascii_lowercase()
    };
    let (conv, cov) = convert(&install, Some(&stem), "a1-first", false);
    let mut vfs = Vfs::mount_install(Path::new(&install), None).expect("mount install");
    vfs.push_dir("converted".into(), conv.dir()).expect("mount the converted missions");
    let book = CampaignBook::load_vfs(&vfs);
    let first = &book.campaigns.iter().find(|c| !c.missions.is_empty()).expect("a campaign").missions[0];
    assert!(
        matches!(&first.script, MissionScript::Lua { path } if path.eq_ignore_ascii_case(&format!("missions/{stem}.lua"))),
        "the first mission resolves to its converted script"
    );
    let host = VfsHost::load(Arc::new(vfs)).expect("installed content loads");
    // exactly that map: its stem and the dot before the extension
    let probes = sta_mission::progress::probe_book(&host, &book, TICKS, true, Some(&format!("{stem}.")));
    assert_eq!(probes.len(), 1);
    let p = &probes[0];
    eprintln!(
        "first mission: ticks {}, rules held {}, objective at {:?}, problems {}",
        p.ticks,
        p.rules_held.len(),
        p.first_objective_tick,
        p.problems.len()
    );
    assert!(p.started, "{:?}", p.start_error);
    assert!(p.errors.is_empty(), "{:?}", p.errors);
    assert!(p.step_errors.is_empty(), "{:?}", p.step_errors);
    assert_eq!(Some(p.rules), cov.get(&stem).map(|c| c.0), "every rule the converter reports for the mission is registered");
    assert!(!p.rules_held.is_empty(), "no rule of the mission ever held");
    assert!(p.handles_resolved > 0, "no label of the map resolved ({} unresolved)", p.unresolved.len());
    // UI-36: the opening cineractive finishes (its path ends and `isCineractiveDone` reads 1), so
    // the script gets past its opening.
    assert!(p.engine_reads.iter().any(|(n, c)| n == "isCineractiveDone" && *c > 0), "the script never asked whether the cineractive is done");
}
