//! Install-gated (#927): converts the compiled missions of a real mod inside the player's own
//! install, with the engine files of their stock Armada II install, into a temporary folder that
//! is removed afterwards. Nothing of the install is pinned here: the test asserts outcomes only.
//!
//! It runs when `STA64_TEST_ROOTS_INSTALL` (the install that hosts the mod) and
//! `STA64_TEST_A2_STOCK_INSTALL` (the engine files) are both set, for the mod named in
//! `STA64_TEST_ROOTS_MOD`. No mod name is written in this file: without that variable the test
//! is skipped with a message. A variable that is set and does not lead anywhere fails the test.

#[path = "parity/harness.rs"]
mod harness;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use harness::TempDir;
use sta_mission_convert::install::{self, Env, Event, Install, Options};
use sta_mission_convert::output::OutDir;
use sta_mission_convert::{Pe, cli, disasm};

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Every file and folder under `root` with its size and time of change: what a run that only
/// reads leaves as it was.
fn state(root: &Path) -> Vec<(PathBuf, u64, Option<SystemTime>)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64, Option<SystemTime>)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            // a folder's own time changes when the system touches it; its entries are what counts
            let (len, time) = if meta.is_dir() { (0, None) } else { (meta.len(), meta.modified().ok()) };
            out.push((e.path(), len, time));
            if meta.is_dir() {
                walk(&e.path(), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

#[test]
fn the_missions_of_a_real_mod_convert_and_each_reports_its_outcome() {
    let (Some(root), Some(engine)) = (var("STA64_TEST_ROOTS_INSTALL"), var("STA64_TEST_A2_STOCK_INSTALL")) else {
        eprintln!("skipped: set STA64_TEST_ROOTS_INSTALL, STA64_TEST_A2_STOCK_INSTALL and STA64_TEST_ROOTS_MOD");
        return;
    };
    let Some(name) = var("STA64_TEST_ROOTS_MOD") else {
        eprintln!("skipped: set STA64_TEST_ROOTS_MOD to the folder name of the mod whose missions to convert");
        return;
    };
    let (root, engine) = (PathBuf::from(root), PathBuf::from(engine));
    let inst = Install::open_mod(&root, &name)
        .unwrap_or_else(|e| panic!("STA64_TEST_ROOTS_INSTALL with the mod of STA64_TEST_ROOTS_MOD is refused: {e:?}"))
        .with_engine(&engine)
        .unwrap_or_else(|e| panic!("STA64_TEST_A2_STOCK_INSTALL is refused: {e:?}"));
    let before = (state(&inst.root), state(inst.engine_root()));

    // the folder the command line would use below a data folder: the install's key with the mod
    let data = TempDir::new("mod-install");
    let key = cli::key(&inst.root, inst.active_mod());
    assert_ne!(key, cli::key(&inst.root, None), "a mod's conversion has a folder of its own");
    let out = data.path().join("converted").join(&key);
    let resolved = install::resolve_output(&out, &inst, &Env::from_process()).expect("a temporary folder is allowed");
    let mut dir = OutDir::inspect(&out, resolved, false).expect("a fresh folder is usable");
    let mut decoder = |_: &Path, pe: &Pe| disasm::disassemble(pe);
    let mut reported = Vec::new();
    let summary = install::convert_install(&inst, &Options::default(), &mut dir, None, &mut decoder, &|| false, &mut |e: &Event| match e {
        Event::Done { mission, .. } => reported.push(mission.clone()),
        Event::Failed { message, .. } => assert!(!message.trim().is_empty(), "a mission failed without a reason"),
        _ => {}
    });
    assert!(summary.aborted.is_none() && summary.finish_error.is_none() && !summary.cancelled, "the run ended early: {:?}", summary.aborted);

    // every mission reports its outcome: converted and written, or failed with a reason
    assert!(!summary.missions.is_empty(), "the mod's mount shows no compiled mission");
    assert_eq!(reported.len(), summary.missions.len(), "one report per mission");
    let mut converted = 0;
    for m in &summary.missions {
        if m.converted && !m.written.is_empty() {
            converted += 1;
            for f in &m.written {
                assert!(out.join("missions").join(f).is_file(), "a written script is missing in the folder of the install with the mod");
            }
        } else {
            assert!(!m.failures.is_empty() || !m.kept.is_empty(), "a mission neither converted nor says why");
            assert!(m.failures.iter().all(|f| !f.trim().is_empty()));
            // the converter's reason, without the mission's name
            eprintln!("a mission failed: {}", m.failures.join("; "));
        }
    }
    eprintln!("{converted} of {} missions converted", summary.missions.len());
    assert!(converted >= 1, "no mission of the mod converted");

    // only the one folder was written, and neither the install nor the engine folder changed
    let folders: Vec<PathBuf> = std::fs::read_dir(data.path().join("converted")).unwrap().flatten().map(|e| e.path()).collect();
    assert_eq!(folders, [out]);
    assert!(before == (state(&inst.root), state(inst.engine_root())), "the run changed a folder it may only read");
}
