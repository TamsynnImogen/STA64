//! The install API (LUA-15i): `sta_mission_convert::install`, the library seam the command line and
//! the game share, on authored folders (no game file, no decoder). Nothing converts here (the DLLs
//! are junk), so a converted mission is an entry the test put in the cache under the key the run
//! asks for, as `tests/cache.rs` does: what is tested is the orchestration: the events, the
//! cancellation, the output folder after it, the cache, the faults as values.

#[path = "parity/harness.rs"]
mod harness;

use std::cell::Cell;
use std::fs;
use std::path::Path;

use harness::TempDir;
use sta_mission_convert::cache::{Cache, Entry};
use sta_mission_convert::disasm::Ins;
use sta_mission_convert::install::{
    self, Abort, Event, Game, Install, InstallError, Options, Stage, Summary,
};
use sta_mission_convert::output::{OutDir, MANIFEST};
use sta_mission_convert::{Pe, Result};

/// An Armada I root with `n` missions: maps `fed1` .. `fedN`, junk DLL pairs (distinct bytes).
fn a1_install(root: &Path, n: usize) {
    let mut files: Vec<(String, Vec<u8>)> = vec![("Armada.exe".into(), Vec::new())];
    for i in 1..=n {
        files.push((format!("missions/x{i}S.dsl"), format!("junk mission {i}").into_bytes()));
        files.push((format!("missions/x{i}R.drl"), format!("junk rules {i}").into_bytes()));
        files.push((format!("bzn/fed{i}.bzn"), format!("xx x{i}R.drl yy").into_bytes()));
    }
    for (rel, b) in files {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b).unwrap();
    }
}

fn key(root: &Path, i: usize) -> String {
    install::a1_cache_key(
        &format!("fed{i}"),
        &root.join(format!("missions/x{i}S.dsl")),
        &root.join(format!("missions/x{i}R.drl")),
    )
    .expect("the DLLs exist")
}

/// What the cache holds for mission `i`: invented Lua and two invented console lines.
fn entry(i: usize) -> Entry {
    Entry::Converted {
        source: format!("-- invented script {i}\nreturn {i}\n").into_bytes(),
        console: vec![format!("  fed{i} (x{i}): invented"), format!("    registration: invented {i}")],
    }
}

fn plant(cache: &Cache, root: &Path, which: &[usize]) {
    for &i in which {
        cache.put(&key(root, i), &entry(i)).unwrap();
    }
}

/// A decoder that must not be reached: a cached mission is not parsed, and junk is refused by the
/// parser before the decoder is asked.
fn no_decoder(_: &Path, _: &Pe) -> Result<Vec<Ins>> {
    panic!("the decoder was asked for a listing");
}

fn out_dir(inst: &Install, out: &Path) -> OutDir {
    let res = install::resolve_output(out, inst, &install::Env::default()).expect("a fresh folder is allowed");
    OutDir::inspect(out, res, false).expect("the folder is usable")
}

struct Run {
    summary: Summary,
    events: Vec<Event>,
}

fn go(root: &Path, out: &Path, cache: Option<&Cache>, only: &[&str], cancel: &dyn Fn() -> bool, stop_after: Option<usize>) -> Run {
    go_with(root, out, cache, only, false, cancel, stop_after)
}

fn go_with(
    root: &Path,
    out: &Path,
    cache: Option<&Cache>,
    only: &[&str],
    prepare_first: bool,
    cancel: &dyn Fn() -> bool,
    stop_after: Option<usize>,
) -> Run {
    let inst = Install::open(root).expect("an Armada I root");
    assert_eq!(inst.game, Game::Armada1);
    let mut od = out_dir(&inst, out);
    let opts = Options { only: only.iter().map(|s| (*s).to_owned()).collect(), prepare_first, ..Options::default() };
    let stop = Cell::new(false);
    let mut events = Vec::new();
    let mut sink = |e: &Event| {
        events.push(e.clone());
        if let (Some(n), Event::Done { index, .. }) = (stop_after, e)
            && *index + 1 == n
        {
            stop.set(true);
        }
    };
    let mut dec = no_decoder;
    let summary =
        install::convert_install(&inst, &opts, &mut od, cache, &mut dec, &|| stop.get() || cancel(), &mut sink);
    Run { summary, events }
}

fn never() -> bool {
    false
}

fn manifest_names(out: &Path) -> Vec<String> {
    let text = fs::read_to_string(out.join(MANIFEST)).unwrap_or_default();
    let mut v: Vec<String> = text.lines().skip(1).filter_map(|l| l.split_once("  ").map(|(_, n)| n.to_owned())).collect();
    v.sort();
    v
}

fn lua_files(out: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(out.join("missions"))
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn progress_events_come_in_order_and_every_mission_is_reported() {
    let t = TempDir::new("api-events");
    let (root, out, cdir) = (t.path().join("Armada"), t.path().join("out"), t.path().join("cache"));
    a1_install(&root, 3);
    let cache = Cache::new(&cdir);
    plant(&cache, &root, &[1, 3]);
    // mission 2 is not cached and the DLL is junk: it fails, the run goes on
    let r = go(&root, &out, Some(&cache), &[], &never, None);
    let e = &r.events;
    let name = |i: usize| format!("fed{i}");
    let started = |i: usize| Event::Started { index: i - 1, total: 3, mission: name(i) };
    let done = |i: usize| Event::Done { index: i - 1, total: 3, mission: name(i) };
    let console = |i: usize| -> Vec<Event> { match entry(i) { Entry::Converted { console, .. } => console.into_iter().map(Event::Console).collect() } };
    assert_eq!(e[0], Event::Planned { total: 3 });
    assert_eq!(e[1], started(1));
    let mut at = 2;
    for ev in console(1) {
        assert_eq!(e[at], ev);
        at += 1;
    }
    assert_eq!(e[at], done(1));
    assert_eq!(e[at + 1], started(2));
    let Event::Failed { mission, name: n, message } = &e[at + 2] else { panic!("a failure was expected: {:?}", e[at + 2]) };
    assert_eq!((mission.as_str(), n.as_str()), ("fed2", "fed2"));
    assert_eq!(message, "DecodeError: not a PE file");
    assert_eq!(e[at + 3], done(2));
    assert_eq!(e[at + 4], started(3));
    assert_eq!(e.len(), at + 5 + 2 + 1, "{e:?}");
    assert_eq!(e[e.len() - 1], done(3));
    // the summary says the same
    let s = &r.summary;
    assert_eq!(s.total, 3);
    assert!(s.aborted.is_none() && s.finish_error.is_none() && !s.cancelled);
    assert!(s.completed() && s.any_failed());
    assert_eq!((s.failed_missions(), s.converted_missions()), (1, 2));
    let by: Vec<(&str, bool, bool)> = s.missions.iter().map(|m| (m.mission.as_str(), m.converted, m.cached)).collect();
    assert_eq!(by, [("fed1", true, true), ("fed2", false, false), ("fed3", true, true)]);
    assert_eq!(s.missions[0].written, ["fed1.lua"]);
    assert!(s.missions[1].written.is_empty() && s.missions[1].failed());
    // the files: written through the verified writer, listed in the manifest
    assert_eq!(lua_files(&out), ["fed1.lua", "fed3.lua"]);
    assert_eq!(manifest_names(&out), ["fed1.lua", "fed3.lua"]);
    assert_eq!(fs::read(out.join("missions/fed3.lua")).unwrap(), b"-- invented script 3\nreturn 3\n");
}

#[test]
fn the_cache_answers_without_parsing_and_a_miss_is_converted_and_a_hit_marks_the_entry() {
    let t = TempDir::new("api-cache");
    let (root, out, cdir) = (t.path().join("Armada"), t.path().join("out"), t.path().join("cache"));
    a1_install(&root, 2);
    let cache = Cache::new(&cdir);
    plant(&cache, &root, &[1, 2]);
    // hits: the panicking decoder is never asked, and no entry is added or removed
    let r = go(&root, &out, Some(&cache), &[], &never, None);
    assert!(r.summary.missions.iter().all(|m| m.cached && m.converted && !m.failed()), "{:?}", r.summary);
    assert_eq!(fs::read_dir(&cdir).unwrap().count(), 2);
    // a changed DLL is another key: not cached any more, so it is converted (and fails: junk)
    fs::write(root.join("missions/x2S.dsl"), b"junk mission two, changed").unwrap();
    let r = go(&root, &out, Some(&cache), &[], &never, None);
    let by: Vec<(bool, bool)> = r.summary.missions.iter().map(|m| (m.cached, m.failed())).collect();
    assert_eq!(by, [(true, false), (false, true)]);
    // the file of the mission that no longer converts is gone with the full run's clearing
    assert_eq!(lua_files(&out), ["fed1.lua"]);
    // no cache at all: nothing is answered from one
    let r = go(&root, &out, None, &[], &never, None);
    assert!(r.summary.missions.iter().all(|m| !m.cached));
}

#[test]
fn a_cancelled_run_leaves_a_consistent_folder_and_the_next_run_completes() {
    let t = TempDir::new("api-cancel");
    let (root, out, cdir) = (t.path().join("Armada"), t.path().join("out"), t.path().join("cache"));
    a1_install(&root, 3);
    let cache = Cache::new(&cdir);
    plant(&cache, &root, &[1, 2, 3]);
    // stopped after the first mission: the check is made between missions
    let r = go(&root, &out, Some(&cache), &[], &never, Some(1));
    let s = &r.summary;
    assert!(s.cancelled && !s.completed() && s.aborted.is_none() && s.finish_error.is_none());
    assert_eq!(s.total, 3);
    assert_eq!(s.missions.len(), 1, "the second mission was never begun");
    assert!(r.events.iter().all(|e| !matches!(e, Event::Started { index: 1.., .. })), "{:?}", r.events);
    assert_eq!(r.events.last(), Some(&Event::Done { index: 0, total: 3, mission: "fed1".to_owned() }));
    // consistent: exactly what was written is in the folder and in the manifest, no temp files
    assert_eq!(lua_files(&out), ["fed1.lua"]);
    assert_eq!(manifest_names(&out), ["fed1.lua"]);
    assert_eq!(fs::read_dir(&out).unwrap().count(), 2, "the manifest and missions/ only");
    // cancelled before the first mission: nothing is written, and the folder is still usable
    let r0 = go(&root, &out, Some(&cache), &[], &|| true, None);
    assert!(r0.summary.cancelled && r0.summary.missions.is_empty());
    assert_eq!(lua_files(&out), Vec::<String>::new(), "the full run's clearing took the earlier file");
    // a second run after the cancellation completes: all three, listed in the manifest
    let r2 = go(&root, &out, Some(&cache), &[], &never, None);
    assert!(r2.summary.completed() && !r2.summary.any_failed());
    assert_eq!(r2.summary.converted_missions(), 3);
    assert_eq!(lua_files(&out), ["fed1.lua", "fed2.lua", "fed3.lua"]);
    assert_eq!(manifest_names(&out), ["fed1.lua", "fed2.lua", "fed3.lua"]);
}

#[test]
fn only_selects_missions_and_a_partial_run_keeps_the_files_of_earlier_runs() {
    let t = TempDir::new("api-only");
    let (root, out, cdir) = (t.path().join("Armada"), t.path().join("out"), t.path().join("cache"));
    a1_install(&root, 3);
    let cache = Cache::new(&cdir);
    plant(&cache, &root, &[1, 2, 3]);
    let r = go(&root, &out, Some(&cache), &["FED1"], &never, None);
    assert_eq!(r.events[0], Event::Planned { total: 1 });
    assert_eq!(lua_files(&out), ["fed1.lua"]);
    let r = go(&root, &out, Some(&cache), &["fed3"], &never, None);
    assert_eq!(r.summary.total, 1);
    assert_eq!(lua_files(&out), ["fed1.lua", "fed3.lua"], "no clearing without a full run");
    assert_eq!(manifest_names(&out), ["fed1.lua", "fed3.lua"]);
}

#[test]
fn faults_are_values_not_prints_or_exits() {
    let t = TempDir::new("api-faults");
    // not an install
    // the error values carry the paths, so a caller words them itself
    let missing = t.path().join("missing");
    let InstallError::NotAFolder { path } = Install::open(&missing).unwrap_err() else { panic!("not a folder") };
    assert_eq!(path, missing);
    let empty = t.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let InstallError::NotArmada { path, root } = Install::open(&empty).unwrap_err() else { panic!("not armada") };
    assert_eq!(path, empty);
    assert!(root.ends_with("empty"));
    let no_missions = t.path().join("Armada");
    fs::create_dir_all(&no_missions).unwrap();
    fs::write(no_missions.join("Armada.exe"), b"").unwrap();
    assert!(matches!(Install::open(&no_missions).unwrap_err(), InstallError::NoMissionsFolder { .. }));
    let fleet = t.path().join("Fleet");
    fs::create_dir_all(fleet.join("Data")).unwrap();
    fs::write(fleet.join("Data/Armada2.exe"), b"").unwrap();
    assert!(matches!(Install::open(&fleet).unwrap_err(), InstallError::FleetOperations { .. }));
    // an Armada II root without its engine files: the run aborts after clearing, as a value
    let a2 = t.path().join("A2");
    fs::create_dir_all(a2.join("missions")).unwrap();
    fs::write(a2.join("Armada2.exe"), b"junk").unwrap();
    fs::write(a2.join("missions/a2_fed01S.dsl"), b"junk").unwrap();
    let inst = Install::open(&a2).unwrap();
    assert_eq!(inst.game, Game::Armada2);
    let out = t.path().join("out");
    let mut od = out_dir(&inst, &out);
    let mut events = Vec::new();
    let mut dec = no_decoder;
    let s = install::convert_install(&inst, &Options::default(), &mut od, None, &mut dec, &never, &mut |e| events.push(e.clone()));
    let Some(Abort { stage, message }) = &s.aborted else { panic!("an abort was expected") };
    assert_eq!(*stage, Stage::Convert);
    assert!(message.starts_with("no Armada2.exe + armada2.map found"), "{message}");
    assert!(events.is_empty(), "{events:?}");
    assert!(s.any_failed() && !s.completed() && s.missions.is_empty());
    // a fault while clearing: the manifest is damaged, the folder is refused up front
    let bad = t.path().join("bad");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join(MANIFEST), b"not a manifest").unwrap();
    let res = install::resolve_output(&bad, &inst, &install::Env::default()).unwrap();
    assert!(OutDir::inspect(&bad, res, false).is_err());
    // an output folder inside the install is refused as text
    let inside = a2.join("out");
    assert!(install::resolve_output(&inside, &inst, &install::Env::default()).unwrap_err().contains("is inside the install"));
}

/// An earlier conversion in `out` (written through the writer, as a run would have), for the
/// tests of what a stop leaves behind.
fn earlier_conversion(inst: &Install, out: &Path) {
    let mut od = out_dir(inst, out);
    od.write("old.lua", b"-- earlier\n").unwrap();
    od.finish().unwrap();
}

#[test]
fn prepare_first_keeps_the_earlier_conversion_on_a_cancel_at_the_start_or_an_engine_fault() {
    let t = TempDir::new("api-prepare");
    let (root, out, cdir) = (t.path().join("Armada"), t.path().join("out"), t.path().join("cache"));
    a1_install(&root, 2);
    let cache = Cache::new(&cdir);
    plant(&cache, &root, &[1, 2]);
    earlier_conversion(&Install::open(&root).unwrap(), &out);
    // a cancel before the first mission: nothing cleared, nothing written
    let r = go_with(&root, &out, Some(&cache), &[], true, &|| true, None);
    assert!(r.summary.cancelled && !r.summary.cleared && r.summary.missions.is_empty());
    assert_eq!(r.summary.total, 2);
    assert!(r.events.is_empty());
    assert_eq!(lua_files(&out), ["old.lua"]);
    assert_eq!(manifest_names(&out), ["old.lua"]);
    // the default order (the command line's) clears first
    let r = go_with(&root, &out, Some(&cache), &[], false, &|| true, None);
    assert!(r.summary.cancelled && r.summary.cleared);
    assert_eq!(lua_files(&out), Vec::<String>::new());
    // an Armada II root whose engine data cannot be loaded: prepare first keeps, default clears
    let a2 = t.path().join("A2");
    fs::create_dir_all(a2.join("missions")).unwrap();
    fs::write(a2.join("Armada2.exe"), b"junk").unwrap();
    fs::write(a2.join("missions/a2_fed01S.dsl"), b"junk").unwrap();
    let inst = Install::open(&a2).unwrap();
    let out2 = t.path().join("out2");
    earlier_conversion(&inst, &out2);
    for (prepare_first, kept) in [(true, true), (false, false)] {
        let mut od = out_dir(&inst, &out2);
        let opts = Options { prepare_first, ..Options::default() };
        let mut dec = no_decoder;
        let s = install::convert_install(&inst, &opts, &mut od, None, &mut dec, &never, &mut |_| {});
        let ab = s.aborted.as_ref().expect("the engine data is missing");
        assert!(ab.message.starts_with("no Armada2.exe + armada2.map found"));
        assert_eq!(s.cleared, !kept);
        assert_eq!(out2.join("missions/old.lua").exists(), kept, "prepare_first = {prepare_first}");
        if !kept {
            break;
        }
    }
}

#[test]
fn the_manifests_own_temporary_files_are_cleaned_when_old() {
    let t = TempDir::new("api-temps");
    let (root, out, cdir) = (t.path().join("Armada"), t.path().join("out"), t.path().join("cache"));
    a1_install(&root, 1);
    let cache = Cache::new(&cdir);
    plant(&cache, &root, &[1]);
    fs::create_dir_all(&out).unwrap();
    let old = out.join(format!(".{MANIFEST}.tmp-4242-1"));
    let young = out.join(format!(".{MANIFEST}.tmp-4242-2"));
    let foreign = out.join("notes.tmp-1-1");
    // another program's file that matches the tool's temporary-name shape, in the folder's root
    let dotted = out.join(".notes.tmp-12-3");
    for p in [&old, &young, &foreign, &dotted] {
        fs::write(p, b"x").unwrap();
    }
    let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    for p in [&old, &foreign, &dotted] {
        fs::File::options().write(true).open(p).unwrap().set_modified(long_ago).unwrap();
    }
    let r = go(&root, &out, Some(&cache), &[], &never, None);
    assert!(r.summary.completed());
    assert!(!old.exists(), "an old temporary file of a killed save is removed");
    assert!(young.exists(), "a young one may belong to a running process");
    assert!(foreign.exists(), "a file that is not the tool's naming stays");
    assert!(dotted.exists(), "a foreign file of the temporary-name shape stays: only the manifest's own names go");
}
