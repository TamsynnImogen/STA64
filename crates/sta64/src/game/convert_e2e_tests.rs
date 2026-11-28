//! The in-game mission conversion against a real install (LUA-15l), install-gated.
//!
//! `STA64_TEST_A1_INSTALL` and `STA64_TEST_A2_STOCK_INSTALL` name base installs, read at run time;
//! each one that is set is run, and the tests skip with a message where none is. The installs are
//! read-only: they are fingerprinted before and after and must not change. Output and cache go to a
//! temporary user-data folder (never the player's, never in the repository: the test asserts its output
//! root) and are deleted with it. Converted scripts are game content: nothing here names, prints or
//! counts one, and no mission, map or file of the games is named; what to expect is derived from the
//! install (every mission the loader lists as legacy-only must end converted, or failed with a reason).
//!
//! **No Python, no `objdump`.** The conversion runs in a child process (this test binary, started again
//! on one test) whose `PATH` is an empty folder, so neither tool can be reached whatever the machine
//! has; the child first proves that: it tries to start both and must fail. The converter never starts a
//! process anyway (`sta_mission_convert`'s `tests/e2e_install.rs` scans its sources for it). Only the
//! child runs the conversion.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;
use sta_mission_convert::install::{self, Env, Event, Install, MissionResult, Options};
use sta_mission_convert::output::OutDir;
use sta_mission_convert::cache::Cache;
use sta_mission_convert::{DecodeError, Pe, disasm};

use super::mission_convert::{ConversionState, ConvertContext, ConverterFn, MissionConvertPlugin, Outcome, StartConversion, decide_with};
use super::mission_host::load_campaigns_with;
use super::mission_tests::{first_card_of, play_install};
use crate::frontend::campaign::tests::temp_dir;
use crate::frontend::campaign::{Campaigns, ProgressStore};

const A1_VAR: &str = "STA64_TEST_A1_INSTALL";
const A2_VAR: &str = "STA64_TEST_A2_STOCK_INSTALL";
const CHILD_VAR: &str = "STA64_E2E_CHILD";

/// The base installs this machine has, by game.
fn installs() -> Vec<(&'static str, PathBuf)> {
    [("Armada I", A1_VAR), ("Armada II", A2_VAR)]
        .into_iter()
        .filter_map(|(game, var)| match std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_dir()) {
            Some(p) => Some((game, p)),
            None => {
                eprintln!("e2e: {game}: {var} is not set to a folder: skipped");
                None
            }
        })
        .collect()
}

type Prints = BTreeMap<PathBuf, (u64, Option<std::time::SystemTime>)>;

/// Every file of the install with its size and modification time.
fn fingerprint(root: &Path) -> Prints {
    fn walk(dir: &Path, out: &mut Prints) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            out.insert(e.path(), (m.len(), m.modified().ok()));
            if m.is_dir() {
                walk(&e.path(), out);
            }
        }
    }
    let mut out = Prints::new();
    walk(root, &mut out);
    out
}

fn real(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// The output root of a test: asserted to be in the temporary folder, outside the install and outside the
/// repository's sources.
fn assert_output_root(data: &Path, install: &Path) {
    let (data, install) = (real(data), real(install));
    let repo = real(&Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."));
    assert!(data.starts_with(real(&std::env::temp_dir())), "the output root is in the temporary folder");
    assert!(!data.starts_with(&install) && !install.starts_with(&data), "the output root is outside the install");
    assert!(!data.starts_with(&repo), "the output root is outside the repository");
}

/// A closure as the converter (spelled out so that its argument types are inferred).
fn fake<F>(f: F) -> Arc<ConverterFn>
where
    F: Fn(&Install, &Options, &mut OutDir, Option<&Cache>, &dyn Fn() -> bool, &mut dyn FnMut(&Event)) -> sta_mission_convert::install::Summary + Send + Sync + 'static,
{
    Arc::new(f)
}

/// The hook in a Bevy app, as the game runs it, writing below `data`.
pub(super) struct Hook {
    app: App,
    root: PathBuf,
    data: PathBuf,
}

impl Hook {
    pub(super) fn new(root: &Path, data: &Path, converter: Option<Arc<ConverterFn>>) -> Hook {
        let mut ctx = ConvertContext::default();
        ctx.data_dir = Some(data.to_path_buf());
        if let Some(c) = converter {
            ctx.converter = c;
        }
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, MissionConvertPlugin)).insert_resource(ctx);
        Hook { app, root: root.to_path_buf(), data: data.to_path_buf() }
    }

    /// One run (a normal one, or `forced`), up to its end; the state it ended in.
    pub(super) fn run(&mut self, forced: bool) -> ConversionState {
        let before = self.app.world().resource::<ConversionState>().runs;
        self.app.world_mut().write_message(StartConversion { install_root: self.root.clone(), active_mod: None, forced, replace_edited: false });
        for _ in 0..100_000_000u64 {
            self.app.update();
            let s = self.app.world().resource::<ConversionState>();
            if !s.running && (s.runs > before || matches!(s.outcome, Some(Outcome::Refused(_)))) {
                return s.clone();
            }
            std::thread::yield_now();
        }
        panic!("the conversion did not end");
    }

    /// `<data>/converted/<key>`: the folder the game mounts.
    pub(super) fn converted(&self) -> PathBuf {
        sta_mission::converted::dir_in(&self.data, &self.root, None)
    }

    fn stamp(&self) -> PathBuf {
        decide_with(&self.root, None, Some(&self.data), &Env::from_process()).expect("a convertible install").stamp_path
    }
}

/// The conversion of `root` into `data` by the hook, as the game would (for the tests that start a
/// mission afterwards): the folder the game mounts, and how the run ended.
pub(super) fn hook_convert(root: &Path, data: &Path) -> (PathBuf, ConversionState) {
    let mut hook = Hook::new(root, data, None);
    let state = hook.run(false);
    assert!(matches!(state.outcome, Some(Outcome::Completed { .. })), "the conversion completed");
    (hook.converted(), state)
}

/// One mission's outcome is data: converted with its files written, or failed with a reason.
fn assert_outcome(m: &MissionResult, conv: &Path, i: usize) {
    if m.failed() {
        assert!(m.failures.iter().all(|f| !f.trim().is_empty()), "mission #{i}: a failure without a reason");
    } else {
        assert!(m.converted, "mission #{i}: neither converted nor failed");
        assert!(!m.written.is_empty(), "mission #{i}: converted but no file written");
        for f in &m.written {
            let bytes = std::fs::read(conv.join("missions").join(f)).unwrap_or_else(|_| panic!("mission #{i}: a written file is missing"));
            assert!(!bytes.is_empty(), "mission #{i}: an empty script");
        }
    }
}

/// The missions the loader lists as waiting for a conversion, with a loader that mounts `conv` when given.
fn load(root: &Path, conv: Option<&Path>, store: &Path) -> (sta_vfs::Vfs, Campaigns) {
    let vfs = sta_vfs::Vfs::mount_install(root, None).expect("the install mounts");
    let (cs, _) = load_campaigns_with(&vfs, Some(root), None, conv, |b| ProgressStore::in_dir(store, b)).expect("the install has campaigns");
    (vfs, cs)
}

fn legacy(cs: &Campaigns) -> Vec<(String, usize)> {
    cs.book.blocked().into_iter().filter(|b| b.reason.starts_with("legacy DLL only")).map(|b| (b.campaign, b.index)).collect()
}

/// After a run: each mission that waited is converted (and marked) or has a failure with a reason.
fn assert_every_waiting_mission_has_an_outcome(game: &str, waiting: &[(String, usize)], after: &Campaigns, state: &ConversionState) {
    let still = legacy(after);
    for (n, (campaign, index)) in waiting.iter().enumerate() {
        let (_, m) = after.mission(campaign, *index).expect("the mission is still in the book");
        if still.contains(&(campaign.clone(), *index)) {
            assert!(after.conversion_failure(m, &state.failures).is_some_and(|f| !f.trim().is_empty()), "{game}: waiting mission #{n} neither converted nor failed with a reason");
        } else {
            assert!(after.is_experimental(m), "{game}: waiting mission #{n} is converted but not marked");
        }
    }
}

fn scenario(game: &str, root: &Path) {
    let before = fingerprint(root);
    let tmp = temp_dir("e2e");
    let data = tmp.join("userdata");
    assert_output_root(&data, root);
    let store = tmp.join("progress");

    // what is expected is derived from the install: the missions the loader lists as legacy-only
    let (_, cs0) = load(root, None, &store);
    let waiting = legacy(&cs0);
    if waiting.is_empty() {
        eprintln!("e2e: {game}: no mission waits for a conversion in this install: skipped");
        let _ = std::fs::remove_dir_all(&tmp);
        return;
    }
    assert!(cs0.convertible, "{game}: a base install with waiting missions is convertible");

    // a folder made under the old key (the slug alone) is there, with a file in it: it is left alone
    let old_key = sta_core::install_key::slug(&real(root).file_name().expect("the install folder has a name").to_string_lossy());
    let old = data.join("converted").join(&old_key);
    std::fs::create_dir_all(old.join("missions")).unwrap();
    std::fs::write(old.join("missions/left_by_an_earlier_version.lua"), b"-- not ours\n").unwrap();
    assert_ne!(old, Hook::new(root, &data, None).converted(), "{game}: the new key differs from the old one");

    // run 1: the hook converts the install; every mission of the run has an outcome as data
    let mut hook = Hook::new(root, &data, None);
    let conv = hook.converted();
    let s1 = hook.run(false);
    assert!(matches!(s1.outcome, Some(Outcome::Completed { .. })), "{game}: the run completed");
    let sum1 = s1.summary.clone().expect("a summary");
    assert!(sum1.aborted.is_none() && sum1.finish_error.is_none() && !sum1.cancelled, "{game}: the run ended early");
    assert_eq!(sum1.total, sum1.missions.len(), "{game}: every planned mission is reported");
    assert!(!sum1.missions.is_empty(), "{game}: the run had missions");
    for (i, m) in sum1.missions.iter().enumerate() {
        assert_outcome(m, &conv, i);
    }
    assert_eq!(s1.failures.len(), sum1.missions.iter().map(|m| m.failures.len()).sum::<usize>(), "{game}: every failure is on the state");

    // the campaigns after the run: every waiting mission is converted or failed with a reason
    let (vfs, cs1) = load(root, Some(&conv), &store);
    assert_every_waiting_mission_has_an_outcome(game, &waiting, &cs1, &s1);
    assert!(cs1.convertible && !cs1.experimental.is_empty() || sum1.converted_missions() == 0, "{game}: converted missions are marked");

    // the old-key folder is untouched
    assert_eq!(std::fs::read(old.join("missions/left_by_an_earlier_version.lua")).unwrap(), b"-- not ours\n", "{game}: the old-key folder is left alone");

    // run 2 (nothing changed): the stamp answers; run 3 without it: the cache answers for every converted mission
    let s2 = hook.run(false);
    assert_eq!(s2.outcome, Some(Outcome::Skipped), "{game}: the stamp skips an unchanged install");
    std::fs::remove_file(hook.stamp()).expect("the run left a stamp");
    let s3 = hook.run(false);
    let sum3 = s3.summary.clone().expect("a summary");
    assert_eq!(sum3.missions.len(), sum1.missions.len(), "{game}: the same missions");
    for (i, (a, b)) in sum1.missions.iter().zip(&sum3.missions).enumerate() {
        assert_eq!((a.converted, a.failed()), (b.converted, b.failed()), "{game}: mission #{i} changed between runs");
        if !a.failed() {
            assert!(b.cached, "{game}: mission #{i} was converted again, not taken from the cache");
        }
    }

    // a forced run goes without the cache: converted again, the same outcomes
    let s4 = hook.run(true);
    let sum4 = s4.summary.clone().expect("a summary");
    assert!(sum4.missions.iter().all(|m| !m.cached), "{game}: a forced run used the cache");
    assert_eq!(sum4.converted_missions(), sum1.converted_missions(), "{game}: a forced run converts what the first did");

    // one mission starts through the mission host after the run (its first campaign mission that can start)
    let card = first_card_of(&cs1);
    let (_view, _errors, _unsupported, title) = play_install(vfs, cs1, card, 300);
    assert!(!title.is_empty(), "{game}: a mission started");

    assert_eq!(fingerprint(root), before, "{game}: the install was written to");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A refused DLL fails its mission with a reason; the rest are converted and stay startable.
fn failing_scenario(game: &str, root: &Path) {
    let before = fingerprint(root);
    let tmp = temp_dir("e2e-fail");
    let data = tmp.join("userdata");
    assert_output_root(&data, root);
    let store = tmp.join("progress");
    let (_, cs0) = load(root, None, &store);
    let waiting = legacy(&cs0);
    if waiting.is_empty() {
        let _ = std::fs::remove_dir_all(&tmp);
        return;
    }
    // the decoder refuses the first DLL it is asked for, as a fault of that file (a value)
    let asked = Arc::new(AtomicBool::new(false));
    let converter = fake(move |inst, opts, out, cache, cancel, sink| {
        let flag = asked.clone();
        let mut decoder = move |_: &Path, pe: &Pe| if flag.swap(true, Ordering::SeqCst) { disasm::disassemble(pe) } else { Err(DecodeError::new("refused by the test")) };
        install::convert_install(inst, opts, out, cache, &mut decoder, cancel, sink)
    });
    let mut hook = Hook::new(root, &data, Some(converter));
    let conv = hook.converted();
    let s = hook.run(false);
    assert!(matches!(s.outcome, Some(Outcome::Completed { failed, .. }) if failed >= 1), "{game}: the refusal is one failed mission, not an aborted run");
    let sum = s.summary.clone().expect("a summary");
    assert!(sum.aborted.is_none(), "{game}: a refused DLL ended the run");
    assert!(s.failures.iter().any(|(_, why)| why.contains("refused by the test")), "{game}: the reason is reported");
    for (i, m) in sum.missions.iter().enumerate() {
        assert_outcome(m, &conv, i);
    }
    let (_, cs1) = load(root, Some(&conv), &store);
    assert_every_waiting_mission_has_an_outcome(game, &waiting, &cs1, &s);
    // the rest stay startable: a mission that was converted is not blocked any more
    let still = legacy(&cs1);
    let converted_waiting = waiting.iter().filter(|w| !still.contains(w)).count();
    assert!(converted_waiting >= 1 || sum.converted_missions() == 0, "{game}: the missions that did not fail are converted");
    assert!(still.len() < waiting.len() || waiting.len() == 1, "{game}: more than the refused mission is converted");
    assert_eq!(fingerprint(root), before, "{game}: the install was written to");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The body of the gated test, run in the child process (`PATH` empty): nothing converts here unless
/// the parent started it.
#[test]
fn child_body() {
    if std::env::var_os(CHILD_VAR).is_none() {
        return;
    }
    for tool in ["python3", "python", "objdump"] {
        assert!(std::process::Command::new(tool).arg("--version").output().is_err(), "e2e: {tool} is reachable in the child: the proof of an empty PATH failed");
    }
    eprintln!("e2e: python and objdump are unreachable");
    for (game, root) in installs() {
        scenario(game, &root);
        eprintln!("e2e: {game}: converted, cached, started");
        failing_scenario(game, &root);
        eprintln!("e2e: {game}: a refused DLL fails its mission only");
    }
    eprintln!("e2e: done");
}

#[test]
fn install_gated_the_hook_converts_each_base_install_without_python_and_a_mission_starts() {
    if std::env::var_os(CHILD_VAR).is_some() {
        return;
    }
    if installs().is_empty() {
        eprintln!("SKIP: set {A1_VAR} and/or {A2_VAR} to a base install for the end-to-end conversion test");
        return;
    }
    let empty = temp_dir("e2e-path");
    let out = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args(["--exact", "game::convert_e2e_tests::child_body", "--nocapture", "--test-threads=1"])
        .env("PATH", &empty)
        .env(CHILD_VAR, "1")
        .output()
        .expect("the child starts");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
    let lines: Vec<&str> = text.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        if l.contains("e2e:") {
            eprintln!("{l}");
        }
        if l.contains("panicked at") {
            eprintln!("{l}");
            if let Some(next) = lines.get(i + 1) {
                eprintln!("{next}");
            }
        }
    }
    assert!(out.status.success(), "the child failed (see the lines above)");
    assert!(lines.iter().any(|l| l.contains("e2e: done")), "the child ran its body");
    let _ = std::fs::remove_dir_all(&empty);
}
