//! The converter end to end on a real install (LUA-15l), install-gated: `STA64_TEST_A1_INSTALL` and
//! `STA64_TEST_A2_STOCK_INSTALL` name base installs, read at run time; each one that is set is run,
//! and the test skips with a message for one that is not.
//!
//! The output and the cache are in a temporary folder that is deleted with the test; the install is
//! never written to (the test fingerprints it before and after) and the output root is asserted to be
//! outside the repository and the install. Converted scripts are game content: nothing here prints,
//! names or counts one. What is asserted is the outcome of *every* mission the run reports, as data:
//! converted with files written, or failed with a reason; and that the cache answers a second run,
//! that a changed converter version converts again and that a refused DLL fails that mission only.
//!
//! No Python and no `objdump`: the decoder is the crate's own (`disasm::disassemble`), in this
//! process. The library cannot reach either tool because it never starts a process: the last test
//! reads the crate's own sources and asserts that no process-spawning call is in them.

#[path = "parity/harness.rs"]
mod harness;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use harness::TempDir;
use sta_mission_convert::cache::Cache;
use sta_mission_convert::install::{self, Event, Install, MissionResult, Options, Summary};
use sta_mission_convert::output::OutDir;
use sta_mission_convert::{DecodeError, Pe, disasm};

fn installs() -> Vec<(&'static str, PathBuf)> {
    [("Armada I", harness::A1_VAR), ("Armada II", harness::A2_VAR)]
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

/// Every file of the install with its size and modification time: equal before and after a run
/// means the run wrote nothing there.
fn fingerprint(root: &Path) -> BTreeMap<PathBuf, (u64, Option<std::time::SystemTime>)> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, (u64, Option<std::time::SystemTime>)>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            out.insert(e.path(), (m.len(), m.modified().ok()));
            if m.is_dir() {
                walk(&e.path(), out);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut out);
    out
}

fn real_decoder(_: &Path, pe: &Pe) -> sta_mission_convert::Result<Vec<disasm::Ins>> {
    disasm::disassemble(pe)
}

struct Run {
    summary: Summary,
    events: Vec<Event>,
}

fn go(inst: &Install, out: &Path, cache: Option<&Cache>, version: Option<u32>, decoder: &mut install::Decoder<'_>) -> Run {
    let res = install::resolve_output(out, inst, &install::Env::from_process()).expect("the output folder is allowed");
    let mut od = OutDir::inspect(out, res, false).expect("a fresh folder is usable");
    let mut events = Vec::new();
    let opts = Options { cache_version: version, prepare_first: true, ..Options::default() };
    let summary = install::convert_install(inst, &opts, &mut od, cache, decoder, &|| false, &mut |e: &Event| events.push(e.clone()));
    Run { summary, events }
}

/// One mission's outcome is data: converted with its files written, or failed with a reason.
fn assert_outcome(m: &MissionResult, out: &Path, i: usize) {
    if m.failed() {
        assert!(m.failures.iter().all(|f| !f.trim().is_empty()), "mission #{i}: a failure without a reason");
    } else {
        assert!(m.converted, "mission #{i}: neither converted nor failed");
        assert!(!m.written.is_empty(), "mission #{i}: converted but no file written");
        for f in &m.written {
            let bytes = fs::read(out.join("missions").join(f)).unwrap_or_else(|_| panic!("mission #{i}: a written file is missing"));
            assert!(!bytes.is_empty(), "mission #{i}: an empty script");
        }
    }
}

/// The folder the test may write to: a temporary one, outside the repository and the install.
fn temp_root(t: &TempDir, install: &Path) -> PathBuf {
    let root = t.path().join("userdata");
    let real = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let repo = real(&Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."));
    let (temp, install) = (real(t.path()), real(install));
    assert!(temp.starts_with(real(&std::env::temp_dir())), "the output is in the temporary folder");
    assert!(!temp.starts_with(&install) && !install.starts_with(&temp), "the output is not in the install");
    assert!(!temp.starts_with(&repo) || temp.starts_with(repo.join("target")), "the output is not in the repository");
    root
}

#[test]
fn gated_every_mission_has_an_outcome_the_cache_answers_and_a_new_version_converts_again() {
    for (game, root) in installs() {
        let before = fingerprint(&root);
        let t = TempDir::new("e2e-lib");
        let data = temp_root(&t, &root);
        let (out, cdir) = (data.join("converted"), data.join("cache"));
        let inst = Install::open(&root).expect("a base install");
        let cache = Cache::new(&cdir);
        let mut dec = real_decoder;

        // run 1: nothing cached; every mission has an outcome as data
        let a = go(&inst, &out, Some(&cache), None, &mut dec);
        assert!(a.summary.aborted.is_none() && a.summary.finish_error.is_none() && !a.summary.cancelled, "{game}: the run ended early");
        assert_eq!(a.summary.total, a.summary.missions.len(), "{game}: every planned mission is reported");
        assert!(!a.summary.missions.is_empty(), "{game}: the install has missions to convert");
        for (i, m) in a.summary.missions.iter().enumerate() {
            assert_outcome(m, &out, i);
            assert!(!m.cached, "{game}: mission #{i} was cached on a fresh cache");
        }
        // every mission announced was started and done, in order
        let started = a.events.iter().filter(|e| matches!(e, Event::Started { .. })).count();
        let done = a.events.iter().filter(|e| matches!(e, Event::Done { .. })).count();
        assert_eq!((started, done), (a.summary.total, a.summary.total), "{game}: events");
        let files_a = scripts(&out);

        // run 2: the cache answers for every mission that converted
        let b = go(&inst, &out, Some(&cache), None, &mut dec);
        assert_eq!(b.summary.missions.len(), a.summary.missions.len());
        for (i, (x, y)) in a.summary.missions.iter().zip(&b.summary.missions).enumerate() {
            assert_eq!((x.converted, x.failed()), (y.converted, y.failed()), "{game}: mission #{i} changed between runs");
            if !x.failed() {
                assert!(y.cached, "{game}: mission #{i} was converted again, not taken from the cache");
            }
        }
        assert_eq!(scripts(&out), files_a, "{game}: the cached run writes the same scripts");

        // run 3: another converter version is another key: converted again, same scripts
        let c = go(&inst, &out, Some(&cache), Some(sta_mission_convert::CONVERTER_VERSION + 1), &mut dec);
        for (i, m) in c.summary.missions.iter().enumerate() {
            assert!(!m.cached, "{game}: mission #{i} came from the cache under another converter version");
        }
        assert_eq!(scripts(&out), files_a, "{game}: a new version of the same converter writes the same scripts");
        // and a run under the first version still finds its own entries
        let d = go(&inst, &out, Some(&cache), None, &mut dec);
        assert!(d.summary.missions.iter().all(|m| m.failed() || m.cached), "{game}: the first version's entries are still there");

        assert_eq!(fingerprint(&root), before, "{game}: the install was written to");
    }
}

#[test]
fn gated_a_refused_dll_fails_its_mission_with_a_reason_and_the_rest_are_converted() {
    for (game, root) in installs() {
        let before = fingerprint(&root);
        let t = TempDir::new("e2e-fail");
        let data = temp_root(&t, &root);
        let inst = Install::open(&root).expect("a base install");
        let mut dec = real_decoder;
        let full = go(&inst, &data.join("full"), None, None, &mut dec);
        assert!(full.summary.aborted.is_none(), "{game}: the run ended early");
        // the decoder refuses the first DLL it is asked for, as a fault of that file (a value)
        let refused = std::cell::Cell::new(false);
        let mut refusing = |_: &Path, pe: &Pe| {
            if refused.replace(true) { disasm::disassemble(pe) } else { Err(DecodeError::new("refused by the test")) }
        };
        let part = go(&inst, &data.join("partial"), None, None, &mut refusing);
        assert!(part.summary.aborted.is_none(), "{game}: a refused DLL ended the run");
        assert_eq!(part.summary.missions.len(), full.summary.missions.len());
        let failed: Vec<usize> = part.summary.missions.iter().enumerate().filter(|(_, m)| m.failed()).map(|(i, _)| i).collect();
        let newly: Vec<usize> = failed.iter().copied().filter(|i| !full.summary.missions[*i].failed()).collect();
        assert_eq!(newly.len(), 1, "{game}: exactly one more mission fails than without the refusal");
        for (i, m) in part.summary.missions.iter().enumerate() {
            assert_outcome(m, &data.join("partial"), i);
            if !newly.contains(&i) {
                // the rest are what they were: converted ones stay converted with the same files
                assert_eq!((m.converted, m.failed(), &m.written), (full.summary.missions[i].converted, full.summary.missions[i].failed(), &full.summary.missions[i].written), "{game}: mission #{i}");
            }
        }
        assert!(part.events.iter().any(|e| matches!(e, Event::Failed { message, .. } if message.contains("refused by the test"))), "{game}: the reason is reported");
        assert_eq!(fingerprint(&root), before, "{game}: the install was written to");
    }
}

/// The scripts of an output folder: file name and bytes (kept in memory only).
fn scripts(out: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(out.join("missions"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "lua"))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), fs::read(e.path()).unwrap()))
        .collect()
}

/// The library never starts a process, so it cannot reach Python or `objdump` whatever the
/// environment holds: no source of the crate calls the standard library's process spawning.
#[test]
fn the_library_never_spawns_a_process() {
    fn walk(dir: &Path, hits: &mut Vec<String>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, hits);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = fs::read_to_string(&p).unwrap();
                for (n, line) in text.lines().enumerate() {
                    let code = line.split("//").next().unwrap_or("");
                    if code.contains("process::Command") || code.contains("Command::new") {
                        hits.push(format!("{}:{}", p.display(), n + 1));
                    }
                }
            }
        }
    }
    let mut hits = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut hits);
    assert!(hits.is_empty(), "a process is spawned at {hits:?}");
}
