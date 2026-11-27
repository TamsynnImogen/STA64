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
fn keys_depend_on_every_part_the_version_and_the_kind_and_parts_cannot_run_together() {
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
    let f = Entry::Failed { message: "DecodeError: invented \u{e9} text\nsecond line".to_owned() };
    let k2 = key(&[b"f"]);
    c.put(&k2, &f).unwrap();
    assert_eq!(c.get(&k2), Some(f));
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

// ------------------------------------------------------------------ through the command line

fn no_listing(_: &Path) -> sta_mission_convert::Result<Vec<(u32, String)>> {
    Err(sta_mission_convert::DecodeError::raw("no listing in an authored test"))
}

fn run(args: &[&str]) -> (i32, Vec<String>) {
    let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    let (mut so, mut se) = (Vec::new(), Vec::new());
    let mut listing = no_listing;
    let code = cli::run(&args, &Env::default(), &mut Io { out: &mut so, err: &mut se }, &mut listing);
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

/// A failed mission is recorded; the next run replays the record (proved by changing the record);
/// a change of an input is another entry; a damaged entry is rebuilt.
#[test]
fn the_command_line_replays_invalidates_and_rebuilds() {
    let t = TempDir::new("cache-cli");
    let root = t.path().join("Armada");
    let (out, cdir) = (t.path().join("out"), t.path().join("cache"));
    a1_install(&root, b"junk one");
    let args = |extra: &[&'static str]| -> Vec<String> {
        let mut a = vec!["--install".to_owned(), s(&root), "--out".to_owned(), s(&out), "--cache-dir".to_owned(), s(&cdir)];
        a.extend(extra.iter().map(|x| (*x).to_owned()));
        a
    };
    let go = |extra: &[&'static str]| {
        let a = args(extra);
        run(&a.iter().map(String::as_str).collect::<Vec<_>>())
    };
    let (code, first) = go(&[]);
    assert_eq!(code, 1);
    assert!(first[1].starts_with("  FAIL fed1: "), "{first:?}");
    let entries = entry_files(&cdir);
    assert_eq!(entries.len(), 1, "the failure is recorded");
    // the same run again: a hit (the console is the same)
    let (_, again) = go(&[]);
    assert_eq!(again, first);
    assert_eq!(entry_files(&cdir), entries);
    // prove it was a hit: the record answers, not the converter
    let k = entries[0].file_stem().unwrap().to_string_lossy().into_owned();
    Cache::new(&cdir).put(&k, &Entry::Failed { message: "replayed from the cache".to_owned() }).unwrap();
    let (_, replay) = go(&[]);
    assert_eq!(replay[1], "  FAIL fed1: replayed from the cache");
    // an input changes: another key, the converter runs again
    a1_install(&root, b"junk two");
    let (_, changed) = go(&[]);
    assert_eq!(changed, first, "the converter answered, not the old record");
    assert_eq!(entry_files(&cdir).len(), 2);
    // a damaged entry is rebuilt
    let k2 = entry_files(&cdir).into_iter().find(|p| *p != entries[0]).unwrap();
    fs::write(&k2, b"damaged").unwrap();
    let (_, rebuilt) = go(&[]);
    assert_eq!(rebuilt, first);
    assert!(Cache::new(&cdir).get(&k2.file_stem().unwrap().to_string_lossy()).is_some(), "rewritten");
    // without the option nothing is read or written there
    let before = entry_files(&cdir);
    fs::remove_file(&before[0]).unwrap();
    let a: Vec<String> = vec!["--install".into(), s(&root), "--out".into(), s(&out)];
    let (_, plain) = run(&a.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(plain, first);
    assert_eq!(entry_files(&cdir).len(), before.len() - 1);
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
    cli::run(&args, &env, &mut Io { out: &mut so, err: &mut se }, &mut listing);
    assert_eq!(entry_files(&data.join("convert-cache").join("star-trek-armada-test")).len(), 1);
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
