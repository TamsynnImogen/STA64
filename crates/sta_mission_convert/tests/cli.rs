//! The command line (LUA-15f): `sta_mission_convert::cli::run` on authored folders (no game file).
//!
//! The installs here are made of invented files: empty executables and junk "DLLs" that no reader
//! accepts, so the run reaches the Python's failure paths (`FAIL` lines, the wrapper's exit
//! codes, the refusals) without a converting mission; a mission that converts is covered by the
//! install-gated tests of `tests/parity.rs`. Where `python3` runs, the same authored folders go
//! through `tools/convert_install_missions.py` and the console output and exit code are compared
//! (a test without `python3` skips that comparison).

#[path = "parity/harness.rs"]
mod harness;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use harness::TempDir;
use sta_mission_convert::cli::{self, Env, Io};

/// A listing source that fails: no authored DLL reaches the point where a listing is read.
fn no_listing(_: &Path) -> sta_mission_convert::Result<Vec<(u32, String)>> {
    Err(sta_mission_convert::DecodeError::raw("no listing in an authored test"))
}

struct Run {
    code: i32,
    out: Vec<String>,
    err: String,
}

fn run_with(args: &[&str], env: &Env) -> Run {
    let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    let (mut so, mut se) = (Vec::new(), Vec::new());
    let mut listing = no_listing;
    let code = cli::run(&args, env, &mut Io { out: &mut so, err: &mut se }, &mut listing);
    Run {
        code,
        out: String::from_utf8_lossy(&so).replace('\r', "").lines().map(str::to_owned).collect(),
        err: String::from_utf8_lossy(&se).replace('\r', "").trim_end().to_owned(),
    }
}

fn run(args: &[&str]) -> Run {
    run_with(args, &Env::default())
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn touch(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

/// Armada I root: `Armada.exe`, `missions/<dlls>`, `bzn/<maps>`.
fn a1_install(root: &Path, dlls: &[(&str, &[u8])], maps: &[(&str, &str)]) {
    touch(&root.join("Armada.exe"), b"");
    fs::create_dir_all(root.join("missions")).unwrap();
    fs::create_dir_all(root.join("bzn")).unwrap();
    for (n, b) in dlls {
        touch(&root.join("missions").join(n), b);
    }
    for (n, text) in maps {
        touch(&root.join("bzn").join(n), text.as_bytes());
    }
}

/// Every file under `root` with its bytes (the install must not change).
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let rel = s(p.strip_prefix(base).unwrap());
            if p.is_dir() {
                out.insert(format!("{rel}/"), Vec::new());
                walk(&p, base, out);
            } else {
                out.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn key_and_data_dir_have_the_vectors_of_the_python_tests() {
    let t = TempDir::new("cli-key");
    for (name, m, want) in [
        ("Star Trek Armada II (Stock)", None, "star-trek-armada-ii-stock"),
        ("Star Trek Armada II", Some("Fleet Ops 4.0"), "star-trek-armada-ii+fleet-ops-4-0"),
        ("@@@", None, "install"),
    ] {
        let d = t.path().join(name);
        fs::create_dir_all(&d).unwrap();
        assert_eq!(cli::key(&d, m), want);
    }
    let get = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_owned())
    };
    let p = |x: &str| PathBuf::from(x);
    let j = |a: &str, b: &[&str]| b.iter().fold(p(a), |acc, x| acc.join(x));
    assert_eq!(cli::data_dir(&get(&[("STA64_DATA_DIR", "/d"), ("HOME", "/h")]), "linux"), Some(p("/d")));
    assert_eq!(cli::data_dir(&get(&[("XDG_DATA_HOME", "/x"), ("HOME", "/h")]), "linux"), Some(j("/x", &["sta64"])));
    assert_eq!(cli::data_dir(&get(&[("HOME", "/h")]), "linux"), Some(j("/h", &[".local", "share", "sta64"])));
    assert_eq!(cli::data_dir(&get(&[("LOCALAPPDATA", "/l")]), "win32"), Some(j("/l", &["sta64"])));
    assert_eq!(
        cli::data_dir(&get(&[("HOME", "/h")]), "darwin"),
        Some(j("/h", &["Library", "Application Support", "sta64"]))
    );
    assert_eq!(cli::data_dir(&get(&[]), "linux"), None);
    // empty counts as unset
    assert_eq!(cli::data_dir(&get(&[("STA64_DATA_DIR", ""), ("HOME", "/h")]), "linux"), Some(j("/h", &[".local", "share", "sta64"])));
}

#[test]
fn armada1_is_told_from_armada2_by_the_exe() {
    let t = TempDir::new("cli-kind");
    touch(&t.path().join("Armada.exe"), b"");
    assert!(cli::is_armada1(t.path()));
    touch(&t.path().join("Armada2.exe"), b"");
    assert!(!cli::is_armada1(t.path()));
}

#[test]
fn arguments_follow_argparse() {
    let r = run(&[]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("--install"), "{}", r.err);
    assert_eq!(run(&["-h"]).code, 0);
    assert_eq!(run(&["--help"]).code, 0);
    assert_eq!(run(&["--install"]).code, 2);
    assert_eq!(run(&["--bogus", "x"]).code, 2);
    assert_eq!(run(&["--in", "/x", "--o"]).code, 2); // --o: ambiguous (--only, --out)
    // an unambiguous prefix and `--name=value`
    let t = TempDir::new("cli-args");
    let r = run(&[&format!("--inst={}", s(&t.path().join("nope")))]);
    assert_eq!(r.code, 1);
    assert!(r.err.ends_with(": not a folder"), "{}", r.err);
}

#[test]
fn refuses_what_the_python_refuses_with_its_messages() {
    let t = TempDir::new("cli-refuse");
    let nope = s(&t.path().join("nope"));
    let r = run(&["--install", &nope]);
    assert_eq!((r.code, r.err.as_str()), (1, format!("--install {nope}: not a folder").as_str()));
    let empty = t.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let r = run(&["--install", &s(&empty)]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("neither an Armada I root (Armada.exe) nor an Armada II root (Armada2.exe)"));
    // a Fleet Operations style install: the engine in Data/
    let fo = t.path().join("fo");
    touch(&fo.join("Data").join("Armada2.exe"), b"");
    let r = run(&["--install", &s(&fo)]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("a Fleet Operations style install (engine in Data/)"), "{}", r.err);
    // a root without a missions folder
    let bare = t.path().join("bare");
    touch(&bare.join("Armada.exe"), b"");
    let r = run(&["--install", &s(&bare), "--out", &s(&t.path().join("o"))]);
    assert_eq!(r.code, 1);
    assert!(r.err.ends_with(": no missions folder"), "{}", r.err);
    // no output folder and no data folder
    let a1 = t.path().join("a1");
    a1_install(&a1, &[], &[]);
    let r = run(&["--install", &s(&a1)]);
    assert_eq!(r.code, 1);
    assert_eq!(r.err, "no user data folder known: set STA64_DATA_DIR or pass --out");
}

/// The package's safety claim: nothing is written into an install, and an output folder inside
/// one is refused before anything is created.
#[test]
fn never_writes_into_an_install() {
    let t = TempDir::new("cli-safe");
    let root = t.path().join("Armada");
    a1_install(&root, &[("x1S.dsl", b"junk"), ("x1R.drl", b"junk")], &[("fed1.bzn", "xx x1R.drl yy")]);
    let before = snapshot(&root);
    // inside the install: itself, a new folder below it, a folder that exists below it
    for out in [root.clone(), root.join("converted"), root.join("missions"), root.join("missions").join("sub")] {
        let r = run(&["--install", &s(&root), "--out", &s(&out)]);
        assert_eq!(r.code, 1, "{out:?}");
        assert!(r.err.contains("is inside the install"), "{}", r.err);
        assert!(r.err.starts_with("output folder "), "{}", r.err);
        assert!(r.out.is_empty(), "{:?}", r.out);
        assert_eq!(snapshot(&root), before, "the install changed");
    }
    // through a symbolic link into the install
    #[cfg(unix)]
    {
        let link = t.path().join("link");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        let r = run(&["--install", &s(&root), "--out", &s(&link.join("deep"))]);
        assert_eq!(r.code, 1);
        assert!(r.err.contains("is inside the install"));
        assert_eq!(snapshot(&root), before);
    }
    // an output folder elsewhere: the run writes there only
    let out = t.path().join("out");
    let r = run(&["--install", &s(&root), "--out", &s(&out)]);
    assert_eq!(snapshot(&root), before, "the install changed");
    assert!(r.out[0].starts_with("converting "));
    // a cache folder inside the install is refused too
    let r = run(&["--install", &s(&root), "--out", &s(&out), "--cache-dir", &s(&root.join("cache"))]);
    assert_eq!(r.code, 1);
    assert!(r.err.starts_with("cache folder "), "{}", r.err);
    assert_eq!(snapshot(&root), before);
}

#[test]
fn refuses_the_repository_but_not_its_build_folder() {
    let t = TempDir::new("cli-repo");
    let repo = t.path().join("repo");
    fs::create_dir_all(repo.join("target")).unwrap();
    let root = t.path().join("Armada");
    a1_install(&root, &[], &[]);
    let env = Env { repo_root: Some(repo.clone()), target_dirs: vec![repo.join("target")], ..Env::default() };
    let r = run_with(&["--install", &s(&root), "--out", &s(&repo.join("out"))], &env);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("is inside the repository: output is derived from game files"), "{}", r.err);
    assert!(!repo.join("out").exists());
    let r = run_with(&["--install", &s(&root), "--out", &s(&repo.join("target").join("o"))], &env);
    assert_eq!(r.code, 0, "{}", r.err);
}

#[test]
fn the_default_output_is_below_the_data_directory_by_the_install_key() {
    let t = TempDir::new("cli-default");
    let root = t.path().join("Star Trek Armada (Test)");
    a1_install(&root, &[], &[]);
    let data = t.path().join("data");
    let env = Env { data_dir: Some(data.clone()), ..Env::default() };
    let r = run_with(&["--install", &s(&root)], &env);
    assert_eq!(r.code, 0, "{}", r.err);
    let want = data.join("converted").join("star-trek-armada-test");
    assert_eq!(r.out[0], format!("converting {} -> {}", s(&cli::realpath(&root)), s(&want)));
    assert_eq!(r.out[1], format!("0 mission scripts in {}", s(&want)));
}

#[test]
fn a_failing_mission_is_reported_and_exit_one_without_other_scripts() {
    let t = TempDir::new("cli-fail");
    let root = t.path().join("Armada");
    a1_install(
        &root,
        &[("x1S.dsl", b"junk"), ("x1R.drl", b"junk"), ("x2S.dsl", b"junk"), ("x2R.drl", b"junk")],
        &[("fed1.bzn", "xx x1R.drl yy"), ("fed2.bzn", "x2r.drl")],
    );
    let out = t.path().join("out");
    let r = run(&["--install", &s(&root), "--out", &s(&out)]);
    // the wrapper: the child exits 1 and no script exists, so the run exits 1 with no summary
    assert_eq!(r.code, 1, "{:?}", r.out);
    assert_eq!(r.out.len(), 3, "{:?}", r.out);
    assert!(r.out[0].starts_with("converting "));
    assert!(r.out[1].starts_with("  FAIL fed1: "), "{:?}", r.out);
    assert!(r.out[2].starts_with("  FAIL fed2: "), "{:?}", r.out);
    // --only selects maps
    let r = run(&["--install", &s(&root), "--out", &s(&out), "--only", "FED2"]);
    assert_eq!(r.out.len(), 2);
    assert!(r.out[1].starts_with("  FAIL fed2: "));
    // unmapped pairs only with the option; here every pair is mapped
    let r = run(&["--install", &s(&root), "--out", &s(&out), "--include-unmapped"]);
    assert_eq!(r.out.len(), 3);
}

/// The full run deletes what the tool wrote (the manifest lists it) and nothing else; a folder with
/// a .lua file the tool did not write is refused unless --keep-foreign says to leave it alone.
/// Nothing converts here (junk DLLs), so the files come from cache entries the test puts there.
#[test]
fn a_full_run_deletes_only_what_the_tool_wrote_and_refuses_foreign_files() {
    let t = TempDir::new("cli-wipe");
    let root = t.path().join("Armada");
    a1_install(&root, &[("x1S.dsl", b"junk"), ("x1R.drl", b"junk")], &[("fed1.bzn", "xx x1R.drl yy")]);
    let (out, cdir) = (t.path().join("out"), t.path().join("cache"));
    let key = cli::a1_cache_key("fed1", &root.join("missions/x1S.dsl"), &root.join("missions/x1R.drl")).unwrap();
    sta_mission_convert::cache::Cache::new(&cdir)
        .put(&key, &sta_mission_convert::cache::Entry::Converted { source: b"-- one\n".to_vec(), console: vec!["  fed1: x".into()] })
        .unwrap();
    let go = |extra: &[&str]| {
        let mut a = vec!["--install", &*Box::leak(s(&root).into_boxed_str()), "--out", &*Box::leak(s(&out).into_boxed_str()), "--cache-dir", &*Box::leak(s(&cdir).into_boxed_str())];
        a.extend_from_slice(extra);
        run(&a)
    };
    let r = go(&[]);
    assert_eq!(r.code, 0, "{:?} {}", r.out, r.err);
    assert_eq!(fs::read(out.join("missions/fed1.lua")).unwrap(), b"-- one\n");
    assert!(out.join(".sta_mission_convert.manifest").is_file());
    // a file of the player's: the next full run refuses the folder and deletes nothing
    touch(&out.join("missions").join("mine.lua"), b"mine");
    touch(&out.join("missions").join("notes.txt"), b"notes");
    let r = go(&[]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("did not write") && r.err.contains("--keep-foreign"), "{}", r.err);
    assert!(r.out.is_empty());
    assert!(out.join("missions/fed1.lua").exists() && out.join("missions/mine.lua").exists());
    // --keep-foreign: the tool's own files are replaced, the player's stay
    let r = go(&["--keep-foreign"]);
    assert_eq!(r.code, 0, "{:?} {}", r.out, r.err);
    assert_eq!(fs::read(out.join("missions/mine.lua")).unwrap(), b"mine");
    assert_eq!(fs::read(out.join("missions/notes.txt")).unwrap(), b"notes");
    assert_eq!(fs::read(out.join("missions/fed1.lua")).unwrap(), b"-- one\n");
    // --only does not delete anything
    touch(&out.join("missions").join("other.lua"), b"other");
    let r = go(&["--keep-foreign", "--only", "fed1"]);
    assert_eq!(r.code, 0);
    assert!(out.join("missions/other.lua").exists());
    // a file the tool wrote that the player changed is the player's now
    fs::write(out.join("missions/fed1.lua"), b"edited by the player").unwrap();
    let r = go(&[]);
    assert_eq!(r.code, 1);
    assert_eq!(fs::read(out.join("missions/fed1.lua")).unwrap(), b"edited by the player");
    // with --keep-foreign it is not overwritten either: that mission is an error
    // (the run aborts; other scripts exist, so the wrapper's rule applies: a warning and exit 0)
    let r = go(&["--keep-foreign"]);
    assert_eq!(r.code, 0, "{:?}", r.out);
    assert!(r.err.contains("refusing to overwrite"), "{}", r.err);
    assert!(r.out.iter().any(|l| l.starts_with("warning: some missions failed")), "{:?}", r.out);
    assert_eq!(fs::read(out.join("missions/fed1.lua")).unwrap(), b"edited by the player");
    assert_eq!(fs::read(out.join("missions/mine.lua")).unwrap(), b"mine");
    // the manifest damaged: refused, nothing deleted
    fs::write(out.join(".sta_mission_convert.manifest"), b"garbage").unwrap();
    let r = go(&["--keep-foreign"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("is damaged"), "{}", r.err);
    assert_eq!(fs::read(out.join("missions/mine.lua")).unwrap(), b"mine");
}

#[test]
fn include_unmapped_is_for_armada_one() {
    let t = TempDir::new("cli-unmapped");
    let root = t.path().join("Armada2");
    touch(&root.join("Armada2.exe"), b"");
    fs::create_dir_all(root.join("missions")).unwrap();
    let r = run(&["--install", &s(&root), "--out", &s(&t.path().join("o")), "--include-unmapped"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("--include-unmapped is for Armada I installs"));
}

// ------------------------------------------------------------------ against the Python tool

fn python_runs() -> bool {
    Command::new("python3").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// stdout, stderr and exit code of `tools/convert_install_missions.py` on `args`.
fn python(args: &[&str]) -> Run {
    let tool = harness::repo_tools_dir().join("convert_install_missions.py");
    let o = Command::new("python3")
        .arg("-I")
        .arg(tool)
        .args(args)
        .env("PYTHONHASHSEED", "0")
        .env("PYTHONUTF8", "1")
        .stdin(Stdio::null())
        .output()
        .expect("python3 runs");
    Run {
        code: o.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&o.stdout).replace('\r', "").lines().map(str::to_owned).collect(),
        err: String::from_utf8_lossy(&o.stderr).replace('\r', "").trim_end().to_owned(),
    }
}

/// The console output of both tools on the same authored folder agrees, the output folder's path
/// (which differs between the two runs) replaced.
fn same_as_python(root: &Path, extra: &[&str]) {
    if !python_runs() {
        eprintln!("python3 is not available: the comparison with the Python tool is skipped");
        return;
    }
    let t = TempDir::new("cli-py");
    let (po, ro) = (t.path().join("py-out"), t.path().join("rs-out"));
    let mut pa = vec!["--install", &*Box::leak(s(root).into_boxed_str()), "--out", &*Box::leak(s(&po).into_boxed_str())];
    pa.extend_from_slice(extra);
    let mut ra = vec!["--install", &*Box::leak(s(root).into_boxed_str()), "--out", &*Box::leak(s(&ro).into_boxed_str())];
    ra.extend_from_slice(extra);
    let (p, r) = (python(&pa), run(&ra));
    let norm = |lines: &[String], out: &Path| -> Vec<String> {
        lines.iter().map(|l| l.replace(&s(out), "<out>")).collect()
    };
    assert_eq!(r.code, p.code, "exit codes");
    assert_eq!(norm(&r.out, &ro), norm(&p.out, &po), "console output");
    let files = |o: &Path| -> Vec<String> {
        let mut v: Vec<String> =
            fs::read_dir(o.join("missions")).map(|rd| rd.flatten().map(|e| s(&PathBuf::from(e.file_name()))).collect()).unwrap_or_default();
        v.sort();
        v
    };
    assert_eq!(files(&ro), files(&po), "files written");
}

#[test]
fn console_output_and_exit_codes_equal_the_python_tools_on_authored_folders() {
    let t = TempDir::new("cli-same");
    // Armada I: two maps, junk DLLs (both fail), an unmapped pair
    let a1 = t.path().join("Armada");
    a1_install(
        &a1,
        &[("x1S.dsl", b"junk"), ("x1R.drl", b"junk"), ("x2S.dsl", b"junk"), ("x2R.drl", b"junk"), ("zS.dsl", b"junk"), ("zR.drl", b"junk")],
        &[("fed1.bzn", "xx x1R.drl yy"), ("fed2.bzn", "x2r.drl")],
    );
    same_as_python(&a1, &[]);
    same_as_python(&a1, &["--only", "fed2"]);
    // an install with nothing to convert: the summary lines
    let empty = t.path().join("Armada-empty");
    a1_install(&empty, &[], &[]);
    same_as_python(&empty, &[]);
    // Armada II with an executable no reader accepts: no engine names, exit 1, nothing else
    let a2 = t.path().join("Armada2");
    touch(&a2.join("Armada2.exe"), b"");
    fs::create_dir_all(a2.join("missions")).unwrap();
    same_as_python(&a2, &[]);
    // the refusals
    for (root, out) in [(&a1, a1.join("o")), (&a2, a2.join("missions"))] {
        if !python_runs() {
            return;
        }
        let (r, p) = (
            run(&["--install", &s(root), "--out", &s(&out)]),
            python(&["--install", &s(root), "--out", &s(&out)]),
        );
        assert_eq!((r.code, &r.out, &r.err), (p.code, &p.out, &p.err));
    }
}
