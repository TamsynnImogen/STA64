//! A script the tool wrote that the player edited since (LUA-15k): the `Edited` policy of the output
//! folder. The command line's rule (`Foreign`) refuses such a file like a foreign one; the game's
//! normal run (`Keep`) leaves it as it is, keeps it in the manifest and says so with an event; the
//! game's "convert again" (`Replace`) takes it as the tool's again. A file the manifest never listed
//! is foreign under every policy. Authored folders and cache entries only (no game file).

#[path = "parity/harness.rs"]
mod harness;

use std::fs;
use std::path::Path;

use harness::TempDir;
use sta_mission_convert::cache::{Cache, Entry};
use sta_mission_convert::disasm::Ins;
use sta_mission_convert::install::{self, Event, Install, Options, Summary};
use sta_mission_convert::output::{self, Edited, MANIFEST, OutDir, WriteError};
use sta_mission_convert::{Pe, Result};

fn open(root: &Path, edited: Edited) -> std::result::Result<OutDir, String> {
    OutDir::inspect_with(root, root.to_path_buf(), false, edited)
}

fn manifest_names(root: &Path) -> Vec<String> {
    let text = fs::read_to_string(root.join(MANIFEST)).unwrap_or_default();
    let mut v: Vec<String> = text.lines().skip(1).filter_map(|l| l.split_once("  ").map(|(_, n)| n.to_owned())).collect();
    v.sort();
    v
}

/// A folder the tool wrote `a.lua` and `b.lua` into.
fn written(t: &TempDir) -> std::path::PathBuf {
    let root = t.path().join("out");
    let mut o = open(&root, Edited::Foreign).unwrap();
    o.write("a.lua", b"-- one\n").unwrap();
    o.write("b.lua", b"-- two\n").unwrap();
    o.finish().unwrap();
    root
}

#[test]
fn an_edited_file_is_foreign_to_the_command_line_and_kept_by_the_game() {
    let t = TempDir::new("edited-policy");
    let root = written(&t);
    assert!(output::edited_scripts(&root).is_empty(), "nothing is edited yet");
    fs::write(root.join("missions/a.lua"), b"-- the player's own\n").unwrap();
    assert_eq!(output::edited_scripts(&root), vec!["a.lua".to_owned()]);
    // the command line's rule: refused like a foreign file
    let e = open(&root, Edited::Foreign).unwrap_err();
    assert!(e.contains("did not write"), "{e}");
    // the game's: usable; the edited file is reported, a plain write leaves it alone
    let mut o = open(&root, Edited::Keep).unwrap();
    assert_eq!(o.edited_names(), vec!["a.lua".to_owned()]);
    assert_eq!(o.write("a.lua", b"-- new\n"), Err(WriteError::Kept));
    assert_eq!(fs::read(root.join("missions/a.lua")).unwrap(), b"-- the player's own\n");
    // a file the manifest never listed is foreign whatever the policy says
    fs::write(root.join("missions/c.lua"), b"-- not ours\n").unwrap();
    let e = open(&root, Edited::Keep).unwrap_err();
    assert!(e.contains("did not write"), "{e}");
    for edited in [Edited::Foreign, Edited::Keep, Edited::Replace] {
        assert!(open(&root, edited).is_err(), "{edited:?}");
    }
}

#[test]
fn a_full_clear_keeps_the_edited_file_and_remembers_it() {
    let t = TempDir::new("edited-wipe");
    let root = written(&t);
    fs::write(root.join("missions/a.lua"), b"-- the player's own\n").unwrap();
    let mut o = open(&root, Edited::Keep).unwrap();
    o.wipe().unwrap();
    assert!(!root.join("missions/b.lua").exists(), "the tool's own file went");
    assert_eq!(fs::read(root.join("missions/a.lua")).unwrap(), b"-- the player's own\n");
    assert_eq!(manifest_names(&root), vec!["a.lua".to_owned()], "the manifest still remembers whose it was");
    o.write("b.lua", b"-- two again\n").unwrap();
    o.finish().unwrap();
    // the next run sees the same state: still edited, not foreign
    let o = open(&root, Edited::Keep).unwrap();
    assert_eq!(o.edited_names(), vec!["a.lua".to_owned()]);
    assert_eq!(manifest_names(&root), vec!["a.lua".to_owned(), "b.lua".to_owned()]);
}

#[test]
fn replacing_moves_the_edit_aside_and_never_deletes_it() {
    let t = TempDir::new("edited-replace");
    let root = written(&t);
    fs::write(root.join("missions/a.lua"), b"-- the player's own\n").unwrap();
    let mut o = open(&root, Edited::Replace).unwrap();
    o.wipe().unwrap();
    // the tool's own file is gone, the edit is beside it under another name, content intact
    assert!(!root.join("missions/a.lua").exists() && !root.join("missions/b.lua").exists());
    assert_eq!(fs::read(root.join("missions/a.lua.old")).unwrap(), b"-- the player's own\n");
    assert_eq!(o.take_moved(), vec![("a.lua".to_owned(), "a.lua.old".to_owned())]);
    o.write("a.lua", b"-- new\n").unwrap();
    o.finish().unwrap();
    assert_eq!(fs::read(root.join("missions/a.lua")).unwrap(), b"-- new\n");
    assert!(output::edited_scripts(&root).is_empty(), "nothing is edited after the replacement");
    // a write over an edited file moves it aside too
    fs::write(root.join("missions/a.lua"), b"-- edited again\n").unwrap();
    let mut o = open(&root, Edited::Replace).unwrap();
    o.write("a.lua", b"-- third\n").unwrap();
    assert_eq!(fs::read(root.join("missions/a.lua")).unwrap(), b"-- third\n");
    assert_eq!(fs::read(root.join("missions/a.lua.old2")).unwrap(), b"-- edited again\n", "the first aside name is taken: the next one");
    assert_eq!(fs::read(root.join("missions/a.lua.old")).unwrap(), b"-- the player's own\n");
}

// ------------------------------------------------------------------ through convert_install

fn a1_install(root: &Path) {
    for i in 1..=2 {
        for (rel, b) in [
            (format!("missions/x{i}S.dsl"), format!("junk mission {i}")),
            (format!("missions/x{i}R.drl"), format!("junk rules {i}")),
            (format!("bzn/fed{i}.bzn"), format!("xx x{i}R.drl yy")),
        ] {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b).unwrap();
        }
    }
    fs::write(root.join("Armada.exe"), b"").unwrap();
}

fn no_decoder(_: &Path, _: &Pe) -> Result<Vec<Ins>> {
    panic!("the decoder was asked for a listing");
}

fn run(root: &Path, out: &Path, cache: &Cache, edited: Edited) -> (Summary, Vec<Event>) {
    let inst = Install::open(root).unwrap();
    let res = install::resolve_output(out, &inst, &install::Env::default()).unwrap();
    let mut od = OutDir::inspect_with(out, res, false, edited).unwrap();
    let mut events = Vec::new();
    let mut dec = no_decoder;
    let s = install::convert_install(
        &inst,
        &Options { prepare_first: true, ..Options::default() },
        &mut od,
        Some(cache),
        &mut dec,
        &|| false,
        &mut |e: &Event| events.push(e.clone()),
    );
    (s, events)
}

#[test]
fn a_run_keeps_an_edited_script_and_reports_it_as_an_event() {
    let t = TempDir::new("edited-run");
    let (root, out, cdir) = (t.path().join("Armada"), t.path().join("out"), t.path().join("cache"));
    a1_install(&root);
    let cache = Cache::new(&cdir);
    for i in 1..=2 {
        let key = install::a1_cache_key(&format!("fed{i}"), &root.join(format!("missions/x{i}S.dsl")), &root.join(format!("missions/x{i}R.drl"))).unwrap();
        cache.put(&key, &Entry::Converted { source: format!("-- invented {i}\n").into_bytes(), console: vec![] }).unwrap();
    }
    let (s, _) = run(&root, &out, &cache, Edited::Keep);
    assert!(s.completed(), "{s:?}");
    fs::write(out.join("missions/fed1.lua"), b"-- the player's own\n").unwrap();
    // the normal run: fed1 is kept and reported, fed2 is written again, the run completes
    let (s, events) = run(&root, &out, &cache, Edited::Keep);
    assert!(s.completed() && !s.any_failed(), "{s:?}");
    assert!(events.contains(&Event::KeptEdited { mission: "fed1".into(), name: "fed1".into() }), "{events:?}");
    assert!(!events.iter().any(|e| matches!(e, Event::Failed { .. })), "a kept file is no failure: {events:?}");
    assert_eq!(s.missions[0].kept, vec!["fed1.lua".to_owned()]);
    assert!(s.missions[0].written.is_empty() && s.missions[1].kept.is_empty());
    assert_eq!(fs::read(out.join("missions/fed1.lua")).unwrap(), b"-- the player's own\n");
    assert_eq!(fs::read(out.join("missions/fed2.lua")).unwrap(), b"-- invented 2\n");
    // replacing: the edit is moved aside (reported as an event), the tool's script is in its place
    let (s, events) = run(&root, &out, &cache, Edited::Replace);
    assert!(s.completed(), "{s:?}");
    assert!(!events.iter().any(|e| matches!(e, Event::KeptEdited { .. })), "{events:?}");
    assert!(events.contains(&Event::MovedAside { old: "fed1.lua".into(), new: "fed1.lua.old".into() }), "{events:?}");
    assert_eq!(fs::read(out.join("missions/fed1.lua")).unwrap(), b"-- invented 1\n");
    assert_eq!(fs::read(out.join("missions/fed1.lua.old")).unwrap(), b"-- the player's own\n");
}

/// A folder with `fed1.lua` the player edited (and `fed2.lua` as the tool wrote it), the cache holding
/// both missions.
fn edited_site(t: &TempDir) -> (std::path::PathBuf, std::path::PathBuf, Cache) {
    let (root, out, cdir) = (t.path().join("Armada"), t.path().join("out"), t.path().join("cache"));
    a1_install(&root);
    let cache = Cache::new(&cdir);
    for i in 1..=2 {
        let key = install::a1_cache_key(&format!("fed{i}"), &root.join(format!("missions/x{i}S.dsl")), &root.join(format!("missions/x{i}R.drl"))).unwrap();
        cache.put(&key, &Entry::Converted { source: format!("-- invented {i}\n").into_bytes(), console: vec![] }).unwrap();
    }
    let (s, _) = run(&root, &out, &cache, Edited::Keep);
    assert!(s.completed(), "{s:?}");
    fs::write(out.join("missions/fed1.lua"), b"-- the player's own\n").unwrap();
    (root, out, cache)
}

fn run_stopping(root: &Path, out: &Path, cache: &Cache, cancel: &dyn Fn() -> bool) -> Summary {
    let inst = Install::open(root).unwrap();
    let res = install::resolve_output(out, &inst, &install::Env::default()).unwrap();
    let mut od = OutDir::inspect_with(out, res, false, Edited::Replace).unwrap();
    let mut dec = no_decoder;
    install::convert_install(&inst, &Options { prepare_first: true, ..Options::default() }, &mut od, Some(cache), &mut dec, cancel, &mut |_: &Event| {})
}

#[test]
fn a_replacement_that_is_cancelled_or_fails_loses_no_edit() {
    let own = b"-- the player's own\n";
    // cancelled after the clearing, before the first mission: the edit is aside, not lost
    let t = TempDir::new("edited-cancel");
    let (root, out, cache) = edited_site(&t);
    let calls = std::cell::Cell::new(0);
    let s = run_stopping(&root, &out, &cache, &|| {
        calls.set(calls.get() + 1);
        calls.get() > 1 // the first question is prepare_first's, the second comes after the clearing
    });
    assert!(s.cancelled && s.cleared, "{s:?}");
    assert_eq!(fs::read(out.join("missions/fed1.lua.old")).unwrap(), own);
    assert!(!out.join("missions/fed1.lua").exists());
    // a mission that fails (its cache entry is gone and the DLL is junk): the edit is aside
    let t = TempDir::new("edited-fail");
    let (root, out, cache) = edited_site(&t);
    let key = install::a1_cache_key("fed1", &root.join("missions/x1S.dsl"), &root.join("missions/x1R.drl")).unwrap();
    fs::remove_file(cache.dir().join(format!("{key}.entry"))).unwrap();
    let s = run_stopping(&root, &out, &cache, &never);
    assert_eq!(s.failed_missions(), 1, "{s:?}");
    assert_eq!(fs::read(out.join("missions/fed1.lua.old")).unwrap(), own);
    // a cancel at once, or a fault before anything is cleared: the edit is untouched where it was
    let t = TempDir::new("edited-first");
    let (root, out, cache) = edited_site(&t);
    let s = run_stopping(&root, &out, &cache, &|| true);
    assert!(s.cancelled && !s.cleared, "{s:?}");
    assert_eq!(fs::read(out.join("missions/fed1.lua")).unwrap(), own);
    // an Armada II install whose engine files are missing aborts the discovery before the clearing
    let t = TempDir::new("edited-abort");
    let (root, out) = (t.path().join("Armada2"), t.path().join("out"));
    for (rel, b) in [("Armada2.exe", &b""[..]), ("missions/m1S.dsl", b"junk"), ("missions/m1R.drl", b"junk")] {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b).unwrap();
    }
    let mut o = open(&out, Edited::Foreign).unwrap();
    o.write("m1.lua", b"-- tool\n").unwrap();
    o.finish().unwrap();
    fs::write(out.join("missions/m1.lua"), own).unwrap();
    let cache = Cache::new(t.path().join("cache"));
    let s = run_stopping(&root, &out, &cache, &never);
    assert!(s.aborted.is_some() && !s.cleared, "{s:?}");
    assert_eq!(fs::read(out.join("missions/m1.lua")).unwrap(), own);
}

fn never() -> bool {
    false
}
