//! The conversion cache (LUA-15f, S9a): entries, keys, damage, and the cache used by the command
//! line on authored folders (no game file); an install-gated run shows a second run replays the
//! first byte for byte.

#[path = "parity/harness.rs"]
mod harness;

use std::fs;
use std::path::{Path, PathBuf};

use harness::TempDir;
use sta_mission_convert::cache::{Cache, Entry, KeyBuilder};
use sta_mission_convert::cli::{self, Env, Io};

fn key(parts: &[&[u8]]) -> String {
    parts.iter().fold(KeyBuilder::new("t"), |k, p| k.part(p)).finish()
}

fn converted() -> Entry {
    Entry::Converted {
        source: b"-- invented\nline\n".to_vec(),
        console: vec!["  invented: 1/1".to_owned(), "    registration: x".to_owned()],
    }
}

fn entry_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "entry")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn keys_depend_on_every_part_the_version_the_build_and_the_kind_and_parts_cannot_run_together() {
    let k = key(&[b"dll bytes", b"drl bytes"]);
    assert_eq!(k.len(), 64);
    assert_eq!(k, key(&[b"dll bytes", b"drl bytes"]));
    assert_ne!(k, key(&[b"dll bytez", b"drl bytes"]));
    assert_ne!(k, key(&[b"dll bytes", b"drl bytez"]));
    assert_ne!(k, key(&[b"dll bytes"]));
    assert_ne!(key(&[b"ab", b"c"]), key(&[b"a", b"bc"]));
    assert_ne!(key(&[b"", b"x"]), key(&[b"x", b""]));
    assert_ne!(k, [b"dll bytes".as_slice(), b"drl bytes"].iter().fold(KeyBuilder::new("u"), |k, p| k.part(p)).finish());
    // a raised converter version is another key (the old entries are never asked for again)
    let v = |n| KeyBuilder::with_version("t", n).part(b"x").finish();
    assert_ne!(v(1), v(2));
    assert_eq!(v(7), v(7));
    assert_eq!(KeyBuilder::new("t").part(b"x").finish(), v(sta_mission_convert::CONVERTER_VERSION));
    // so is another build of the converter: a changed source file changes the identity
    let b = |id: &str| KeyBuilder::with_identity("t", 1, id).part(b"x").finish();
    assert_ne!(b("0.1.0+aaaa"), b("0.1.0+aaab"));
    assert_ne!(b("0.1.0+aaaa"), b("0.2.0+aaaa"));
    assert_eq!(
        KeyBuilder::new("t").part(b"x").finish(),
        KeyBuilder::with_identity("t", sta_mission_convert::CONVERTER_VERSION, sta_mission_convert::cache::BUILD_ID)
            .part(b"x")
            .finish()
    );
    assert!(sta_mission_convert::cache::BUILD_ID.contains('+') && sta_mission_convert::cache::BUILD_ID.len() > 32);
}

#[test]
fn entries_round_trip_and_a_hit_is_identical() {
    let t = TempDir::new("cache-rt");
    let c = Cache::new(t.path().join("c"));
    let k = key(&[b"a"]);
    assert_eq!(c.get(&k), None);
    c.put(&k, &converted()).unwrap();
    assert_eq!(c.get(&k), Some(converted()));
    assert_eq!(c.get(&key(&[b"b"])), None);
    let odd = Entry::Converted {
        source: vec![0, 255, 10, 13, 0xe9],
        console: vec!["invented \u{e9} text".to_owned(), String::new()],
    };
    let k2 = key(&[b"f"]);
    c.put(&k2, &odd).unwrap();
    assert_eq!(c.get(&k2), Some(odd));
    // put replaces
    let again = Entry::Converted { source: Vec::new(), console: Vec::new() };
    c.put(&k, &again).unwrap();
    assert_eq!(c.get(&k), Some(again));
    // no temporary files stay
    assert_eq!(fs::read_dir(c.dir()).unwrap().count(), 2);
    // a key that is not a key is refused and never touches the file system outside the folder
    assert_eq!(c.get("../x"), None);
    assert!(c.put("../x", &converted()).is_err());
    assert!(c.put("abc", &converted()).is_err());
}

#[test]
fn damaged_entries_are_ignored_and_rebuilt() {
    let t = TempDir::new("cache-bad");
    let c = Cache::new(t.path().join("c"));
    let k = key(&[b"a"]);
    c.put(&k, &converted()).unwrap();
    let path = c.dir().join(format!("{k}.entry"));
    let good = fs::read(&path).unwrap();
    let mut damaged: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"not a cache entry".to_vec(),
        good[..good.len() - 1].to_vec(),
        good[..good.len() / 2].to_vec(),
    ];
    let mut extended = good.clone();
    extended.push(0);
    damaged.push(extended);
    for at in [0, 8, good.len() / 2, good.len() - 33, good.len() - 1] {
        let mut b = good.clone();
        b[at] ^= 0x40;
        damaged.push(b);
    }
    for (i, bad) in damaged.iter().enumerate() {
        fs::write(&path, bad).unwrap();
        assert_eq!(c.get(&k), None, "damage {i} was accepted");
        // rebuilt: the next put replaces the damaged file
        c.put(&k, &converted()).unwrap();
        assert_eq!(c.get(&k), Some(converted()), "damage {i}");
    }
    // an entry file copied under another key is not that key's entry
    let other = key(&[b"other"]);
    fs::write(c.dir().join(format!("{other}.entry")), &good).unwrap();
    assert_eq!(c.get(&other), None);
    // a directory where the entry should be
    let k3 = key(&[b"dir"]);
    fs::create_dir_all(c.dir().join(format!("{k3}.entry"))).unwrap();
    assert_eq!(c.get(&k3), None);
}

#[test]
fn an_oversized_entry_is_a_miss_and_prune_goes_by_age_only() {
    let t = TempDir::new("cache-upkeep");
    let c = Cache::new(t.path().join("c"));
    let (k1, k2, k3) = (key(&[b"1"]), key(&[b"2"]), key(&[b"3"]));
    for k in [&k1, &k2, &k3] {
        c.put(k, &converted()).unwrap();
    }
    let age = |p: &Path, secs: u64| {
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(secs);
        fs::File::options().write(true).open(p).unwrap().set_modified(when).unwrap();
    };
    let entry = |k: &str| c.dir().join(format!("{k}.entry"));
    // a file that is not ours, a folder, a recent temporary file and an old one
    fs::write(c.dir().join("notes.txt"), b"x").unwrap();
    fs::create_dir_all(c.dir().join("sub.entry")).unwrap();
    let recent = c.dir().join(format!("{k1}.tmp-1-1"));
    fs::write(&recent, b"x").unwrap();
    let old = c.dir().join(format!("{k2}.tmp-2-2"));
    fs::write(&old, b"x").unwrap();
    age(&old, 7200);
    // k2 was last used long ago, k3 a while ago, k1 now
    age(&entry(&k2), 40 * 24 * 3600);
    age(&entry(&k3), 10 * 24 * 3600);
    let month = std::time::Duration::from_secs(30 * 24 * 3600);
    c.prune(month);
    assert!(c.get(&k1).is_some());
    assert_eq!(c.get(&k2), None, "not used for longer than the limit");
    assert!(c.get(&k3).is_some(), "another install's entry, used recently enough, stays");
    assert!(c.dir().join("notes.txt").exists(), "foreign files stay");
    assert!(c.dir().join("sub.entry").is_dir(), "folders stay");
    assert!(recent.exists(), "a recent temporary file may belong to a running process");
    assert!(!old.exists(), "an old temporary file is a killed run's");
    // a hit marks the entry as used
    age(&entry(&k3), 40 * 24 * 3600);
    c.touch(&k3);
    c.prune(month);
    assert!(c.get(&k3).is_some());
    // an entry file larger than the cap is not read
    let big = key(&[b"big"]);
    let f = fs::File::create(c.dir().join(format!("{big}.entry"))).unwrap();
    f.set_len(65 * 1024 * 1024).unwrap();
    assert_eq!(c.get(&big), None);
}

#[cfg(unix)]
#[test]
fn a_cache_folder_that_is_a_link_is_not_written() {
    let t = TempDir::new("cache-link");
    let real = t.path().join("real");
    fs::create_dir_all(&real).unwrap();
    let link = t.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let c = Cache::new(&link);
    assert!(c.put(&key(&[b"a"]), &converted()).is_err());
    assert_eq!(fs::read_dir(&real).unwrap().count(), 0);
}

// ------------------------------------------------------------------ through the command line

fn no_listing(_: &Path) -> sta_mission_convert::Result<Vec<(u32, String)>> {
    Err(sta_mission_convert::DecodeError::raw("no listing in an authored test"))
}

fn run(args: &[&str]) -> (i32, Vec<String>) {
    let (mut so, mut se) = (Vec::new(), Vec::new());
    let mut listing = no_listing;
    let code = cli::run(args, &Env::default(), &mut Io { out: &mut so, err: &mut se }, &mut listing);
    (code, String::from_utf8_lossy(&so).replace('\r', "").lines().map(str::to_owned).collect())
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn a1_install(root: &Path, x1s: &[u8]) {
    for (rel, b) in [
        ("Armada.exe", b"".as_slice()),
        ("missions/x1S.dsl", x1s),
        ("missions/x1R.drl", b"junk"),
        ("bzn/fed1.bzn", b"xx x1R.drl yy"),
    ] {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b).unwrap();
    }
}

/// The key the command line uses for the authored Armada I mission.
fn mission_key(root: &Path) -> String {
    cli::a1_cache_key("fed1", &root.join("missions/x1S.dsl"), &root.join("missions/x1R.drl")).unwrap()
}

/// Nothing converts here (junk DLLs), so a hit is an entry the test put there under the key the
/// command line asks for: the run replays it (console lines and file); another input is another
/// key and the converter answers (a failure, which is never cached); a damaged entry is ignored.
#[test]
fn the_command_line_replays_invalidates_and_ignores_damage() {
    let t = TempDir::new("cache-cli");
    let root = t.path().join("Armada");
    let (out, cdir) = (t.path().join("out"), t.path().join("cache"));
    a1_install(&root, b"junk one");
    let go = || {
        run(&["--install", &s(&root), "--out", &s(&out), "--cache-dir", &s(&cdir)])
    };
    // nothing cached: the converter fails, and a failure is not recorded
    let (code, first) = go();
    assert_eq!(code, 1);
    assert!(first[1].starts_with("  FAIL fed1: "), "{first:?}");
    assert!(entry_files(&cdir).is_empty(), "a failure is not cached");
    // a hit: the entry answers
    let cache = Cache::new(&cdir);
    let k1 = mission_key(&root);
    let entry = Entry::Converted {
        source: b"-- invented lua\nreturn 1\n".to_vec(),
        console: vec!["  fed1 (x1): invented line".to_owned(), "    registration: invented".to_owned()],
    };
    cache.put(&k1, &entry).unwrap();
    let (code, replay) = go();
    assert_eq!(code, 0, "{replay:?}");
    assert_eq!(replay[1], "  fed1 (x1): invented line");
    assert_eq!(replay[2], "    registration: invented");
    assert_eq!(fs::read(out.join("missions/fed1.lua")).unwrap(), b"-- invented lua\nreturn 1\n");
    // an input changes: another key, the converter runs and fails again, the file of the old
    // conversion is deleted by the full run (it was ours)
    a1_install(&root, b"junk two");
    assert_ne!(mission_key(&root), k1);
    let (code, changed) = go();
    assert_eq!(code, 1);
    assert_eq!(changed, first, "the converter answered, not the old entry");
    assert!(!out.join("missions/fed1.lua").exists());
    // a damaged entry under the right key is ignored
    let k2 = mission_key(&root);
    cache.put(&k2, &entry).unwrap();
    fs::write(cdir.join(format!("{k2}.entry")), b"damaged").unwrap();
    let (code, damaged) = go();
    assert_eq!(code, 1);
    assert_eq!(damaged, first);
    // a hit marks the entry as used; a full run removes entries not used for a month, whoever's
    cache.put(&k1, &entry).unwrap();
    cache.put(&k2, &entry).unwrap();
    let stale = cdir.join(format!("{k1}.entry"));
    let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(40 * 24 * 3600);
    fs::File::options().write(true).open(&stale).unwrap().set_modified(long_ago).unwrap();
    let (_, _) = run(&["--install", &s(&root), "--out", &s(&out), "--cache-dir", &s(&cdir), "--only", "fed1"]);
    assert_eq!(entry_files(&cdir).len(), 2, "--only prunes nothing");
    let (code, _) = go();
    assert_eq!(code, 0, "the entry for the current input answers");
    assert_eq!(entry_files(&cdir).len(), 1, "a full run removes what was not used for a month");
    assert!(cache.get(&k2).is_some());
}

#[test]
fn the_cache_option_uses_the_data_directory() {
    let t = TempDir::new("cache-data");
    let root = t.path().join("Star Trek Armada (Test)");
    a1_install(&root, b"junk");
    let data = t.path().join("data");
    let args: Vec<String> = vec!["--install".into(), s(&root), "--out".into(), s(&t.path().join("out")), "--cache".into()];
    let (mut so, mut se) = (Vec::new(), Vec::new());
    let mut listing = no_listing;
    let env = Env { data_dir: Some(data.clone()), ..Env::default() };
    // an entry in the default folder is used
    let dir = data.join("convert-cache").join("star-trek-armada-test");
    Cache::new(&dir)
        .put(&mission_key(&root), &Entry::Converted { source: b"-- x\n".to_vec(), console: vec!["  replayed".to_owned()] })
        .unwrap();
    let code = cli::run(&args, &env, &mut Io { out: &mut so, err: &mut se }, &mut listing);
    assert_eq!(code, 0);
    assert!(String::from_utf8_lossy(&so).contains("  replayed"));
    // no data directory known: refused, with the way out named
    let (mut so, mut se) = (Vec::new(), Vec::new());
    let code = cli::run(&args, &Env::default(), &mut Io { out: &mut so, err: &mut se }, &mut listing);
    assert_eq!(code, 1);
    assert!(String::from_utf8_lossy(&se).contains("--cache-dir"));
}

// ------------------------------------------------------------------ gated

/// With an install: the second run with a cache replays the first byte for byte (files and
/// console) and writes no new entry; the files equal those of a run without a cache.
#[test]
fn gated_a1_second_run_replays_the_first() {
    replays(harness::A1_VAR);
}

#[test]
fn gated_a2_second_run_replays_the_first() {
    replays(harness::A2_VAR);
}

fn replays(var: &str) {
    let Some(root) = std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_dir()) else {
        eprintln!("{var} is not set: skipped");
        return;
    };
    let t = TempDir::new("cache-gated");
    let cdir = t.path().join("cache");
    type Files = Vec<(String, Vec<u8>)>;
    let go = |out: &Path, cache: bool| -> (i32, Vec<String>, Files) {
        let mut args = vec!["--install".to_owned(), s(&root), "--out".to_owned(), s(out)];
        if cache {
            args.push("--cache-dir".into());
            args.push(s(&cdir));
        }
        let (mut so, mut se) = (Vec::new(), Vec::new());
        let mut listing =
            |p: &Path| -> sta_mission_convert::Result<Vec<(u32, String)>> { sta_mission_convert::disasm::disassemble(&sta_mission_convert::Pe::open(p)?) };
        let code = cli::run(&args, &Env::from_process(), &mut Io { out: &mut so, err: &mut se }, &mut listing);
        let lines: Vec<String> = String::from_utf8_lossy(&so).lines().map(str::to_owned).collect();
        let mut files: Vec<(String, Vec<u8>)> = fs::read_dir(out.join("missions"))
            .unwrap()
            .flatten()
            .map(|e| (e.file_name().to_string_lossy().into_owned(), fs::read(e.path()).unwrap()))
            .collect();
        files.sort();
        (code, lines, files)
    };
    let (c0, plain_lines, plain) = go(&t.path().join("plain"), false);
    let (c1, l1, f1) = go(&t.path().join("one"), true);
    let n = entry_files(&cdir).len();
    let (c2, l2, f2) = go(&t.path().join("two"), true);
    assert_eq!((c0, c1, c2), (0, 0, 0));
    assert!(n > 0);
    assert_eq!(entry_files(&cdir).len(), n, "a hit writes no entry");
    // the console differs between the runs only in the folder's name
    let norm = |v: &[String], d: &str| -> Vec<String> { v.iter().map(|l| l.replace(&s(&t.path().join(d)), "<out>")).collect() };
    assert_eq!(norm(&l1, "one"), norm(&plain_lines, "plain"));
    assert_eq!(norm(&l2, "two"), norm(&l1, "one"));
    assert_eq!(f1, plain, "a run with an empty cache equals a run without one");
    assert_eq!(f2, f1, "the replay equals the first run");
}
