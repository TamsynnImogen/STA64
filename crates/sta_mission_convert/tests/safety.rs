//! Where the command line writes and deletes (LUA-15f): path resolution and containment, the
//! output folder with its manifest and atomic writes, and the refusals through the command line.
//! Authored folders only; a test that needs symbolic links is Unix-only, one that needs a case
//! insensitive file system is for Windows and macOS.

#[path = "parity/harness.rs"]
mod harness;

use std::fs;
use std::path::{Path, PathBuf};

use harness::TempDir;
use sta_mission_convert::cache::{Cache, Entry};
use sta_mission_convert::cli::{self, Env, Io};
use sta_mission_convert::output::{OutDir, MANIFEST};
use sta_mission_convert::paths::{contains, game_root_above, resolve, simplify_verbatim};

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn touch(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

fn no_listing(_: &Path) -> sta_mission_convert::Result<Vec<(u32, String)>> {
    Err(sta_mission_convert::DecodeError::raw("no listing in an authored test"))
}

struct Run {
    code: i32,
    out: Vec<String>,
    err: String,
}

fn run_with<S: AsRef<std::ffi::OsStr>>(args: &[S], env: &Env) -> Run {
    let (mut so, mut se) = (Vec::new(), Vec::new());
    let mut listing = no_listing;
    let code = cli::run(args, env, &mut Io { out: &mut so, err: &mut se }, &mut listing);
    Run {
        code,
        out: String::from_utf8_lossy(&so).replace('\r', "").lines().map(str::to_owned).collect(),
        err: String::from_utf8_lossy(&se).replace('\r', "").trim_end().to_owned(),
    }
}

fn run(args: &[&str]) -> Run {
    run_with(args, &Env::default())
}

fn a1_install(root: &Path) {
    touch(&root.join("Armada.exe"), b"");
    touch(&root.join("missions/x1S.dsl"), b"junk");
    touch(&root.join("missions/x1R.drl"), b"junk");
    touch(&root.join("bzn/fed1.bzn"), b"xx x1R.drl yy");
}

fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let rel = s(p.strip_prefix(base).unwrap());
            if p.is_dir() {
                out.push((format!("{rel}/"), Vec::new()));
                walk(&p, base, out);
            } else {
                out.push((rel, fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

// ------------------------------------------------------------------ resolve and contains

#[test]
fn resolve_removes_dot_and_dotdot_after_the_existing_prefix() {
    let t = TempDir::new("safe-resolve");
    let w = resolve(t.path()).unwrap();
    fs::create_dir_all(w.join("a/b")).unwrap();
    assert_eq!(resolve(&w.join("a/b")).unwrap(), w.join("a/b"));
    assert_eq!(resolve(&w.join("a/./b/../b")).unwrap(), w.join("a/b"));
    // a missing component followed by `..` leads back to what exists
    assert_eq!(resolve(&w.join("missing/../a/b/x")).unwrap(), w.join("a/b/x"));
    assert_eq!(resolve(&w.join("m1/m2/../../a")).unwrap(), w.join("a"));
    assert_eq!(resolve(&w.join("a/missing/../../a/b")).unwrap(), w.join("a/b"));
    // a non-existent tail is kept, `..` in it removed
    assert_eq!(resolve(&w.join("a/new/deeper/../x")).unwrap(), w.join("a/new/x"));
    // trailing separators and `.`
    let with_sep = format!("{}{}", w.join("a").display(), std::path::MAIN_SEPARATOR);
    assert_eq!(resolve(Path::new(&with_sep)).unwrap(), w.join("a"));
    assert_eq!(resolve(&w.join("a").join(".")).unwrap(), w.join("a"));
    // `..` above the root stays at the root
    let up: PathBuf = std::iter::repeat_n("..", 100).collect();
    assert_eq!(resolve(&w.join(up).join("x")).unwrap().parent().unwrap().parent(), None);
}

#[test]
fn resolve_makes_relative_paths_absolute() {
    let cwd = resolve(&std::env::current_dir().unwrap()).unwrap();
    assert_eq!(resolve(Path::new("Cargo.toml")).unwrap(), cwd.join("Cargo.toml"));
    assert_eq!(resolve(Path::new("./nope/../Cargo.toml")).unwrap(), cwd.join("Cargo.toml"));
    assert_eq!(resolve(Path::new(".")).unwrap(), cwd);
    assert!(resolve(Path::new("a/new/tail")).unwrap().starts_with(&cwd));
}

#[cfg(unix)]
#[test]
fn resolve_follows_links_and_applies_dotdot_to_the_real_path() {
    let t = TempDir::new("safe-links");
    let w = resolve(t.path()).unwrap();
    fs::create_dir_all(w.join("real/deep")).unwrap();
    std::os::unix::fs::symlink(w.join("real/deep"), w.join("link")).unwrap();
    // the parent of the link's target, not of the link
    assert_eq!(resolve(&w.join("link/..")).unwrap(), w.join("real"));
    assert_eq!(resolve(&w.join("link/new")).unwrap(), w.join("real/deep/new"));
    assert_eq!(resolve(&w.join("link/new/../..")).unwrap(), w.join("real"));
    // a chain, and a relative link
    std::os::unix::fs::symlink("link", w.join("chain")).unwrap();
    assert_eq!(resolve(&w.join("chain/x")).unwrap(), w.join("real/deep/x"));
    // a link that leads nowhere and a loop are errors, not guesses
    std::os::unix::fs::symlink(w.join("nowhere"), w.join("dangling")).unwrap();
    assert!(resolve(&w.join("dangling")).is_err());
    assert!(resolve(&w.join("dangling/x")).is_err());
    std::os::unix::fs::symlink("loop2", w.join("loop1")).unwrap();
    std::os::unix::fs::symlink("loop1", w.join("loop2")).unwrap();
    assert!(resolve(&w.join("loop1")).is_err());
}

#[test]
fn containment_is_component_wise() {
    let p = |x: &str| PathBuf::from(x);
    assert!(contains(&p("/g/Armada"), &p("/g/Armada")));
    assert!(contains(&p("/g/Armada"), &p("/g/Armada/x/y")));
    assert!(!contains(&p("/g/Armada"), &p("/g/Armada2")));
    assert!(!contains(&p("/g/Armada"), &p("/g")));
    assert!(!contains(&p("/g/Armada"), &p("/h/Armada")));
    // case: only where the platform's file systems usually ignore it
    let ci = cfg!(any(windows, target_os = "macos"));
    assert_eq!(contains(&p("/g/Armada"), &p("/g/ARMADA/x")), ci);
}

#[test]
fn verbatim_prefixes_are_dropped_where_the_plain_form_means_the_same() {
    assert_eq!(simplify_verbatim(r"\\?\C:\Games\Armada"), r"C:\Games\Armada");
    assert_eq!(simplify_verbatim(r"\\?\c:"), "c:");
    assert_eq!(simplify_verbatim(r"\\?\UNC\srv\share\x"), r"\\srv\share\x");
    assert_eq!(simplify_verbatim(r"C:\Games"), r"C:\Games");
    assert_eq!(simplify_verbatim(r"\\?\Volume{1234}\x"), r"\\?\Volume{1234}\x");
    assert_eq!(simplify_verbatim("/usr/lib"), "/usr/lib");
}

#[test]
fn a_game_folder_is_recognised_at_its_top_and_in_data() {
    let t = TempDir::new("safe-game");
    let w = resolve(t.path()).unwrap();
    touch(&w.join("A/Armada.exe"), b"");
    touch(&w.join("B/Data/Armada2.exe"), b"");
    touch(&w.join("C/readme.txt"), b"");
    assert_eq!(game_root_above(&w.join("A")), Some(w.join("A")));
    assert_eq!(game_root_above(&w.join("A/missions/new/deep")), Some(w.join("A")));
    // the engine folder itself counts too (the nearest folder that holds an executable)
    assert!(game_root_above(&w.join("B/Data/Mods")).is_some_and(|g| g.starts_with(w.join("B"))));
    assert_eq!(game_root_above(&w.join("C/out")), None);
}

// ------------------------------------------------------------------ the output folder

fn ours_dir(t: &TempDir) -> (PathBuf, PathBuf) {
    let w = resolve(t.path()).unwrap();
    (w.clone(), w.join("out"))
}

#[test]
fn files_are_written_atomically_listed_and_deleted_only_when_ours() {
    let t = TempDir::new("safe-out");
    let (_, out) = ours_dir(&t);
    let mut o = OutDir::inspect(&out, out.clone(), false).unwrap();
    o.wipe().unwrap();
    o.write("a.lua", b"one").unwrap();
    o.write("b.lua", b"two").unwrap();
    o.finish().unwrap();
    // no temporary file stays; the manifest names both files by hash
    let names: Vec<String> = fs::read_dir(out.join("missions")).unwrap().flatten().map(|e| s(&PathBuf::from(e.file_name()))).collect();
    assert_eq!(names.len(), 2, "{names:?}");
    let m = fs::read_to_string(out.join(MANIFEST)).unwrap();
    assert!(m.starts_with("sta_mission_convert-manifest 1\n"));
    assert_eq!(m.lines().count(), 3);
    // a player's file and a note beside ours
    touch(&out.join("missions/mine.lua"), b"mine");
    touch(&out.join("missions/notes.txt"), b"notes");
    assert!(OutDir::inspect(&out, out.clone(), false).unwrap_err().contains("did not write"));
    let mut o = OutDir::inspect(&out, out.clone(), true).unwrap();
    o.wipe().unwrap();
    assert!(!out.join("missions/a.lua").exists() && !out.join("missions/b.lua").exists());
    assert_eq!(fs::read(out.join("missions/mine.lua")).unwrap(), b"mine");
    assert_eq!(fs::read(out.join("missions/notes.txt")).unwrap(), b"notes");
    // a name that collides with the player's file is an error, the file is untouched
    let e = o.write("mine.lua", b"x").unwrap_err().to_string();
    assert!(e.contains("refusing to overwrite"), "{e}");
    assert_eq!(fs::read(out.join("missions/mine.lua")).unwrap(), b"mine");
    // names that are not one plain file name are refused
    for bad in ["", ".", "..", "a/b.lua", r"a\b.lua", "../x.lua", "C:x.lua", "a\0b"] {
        assert!(o.write(bad, b"x").is_err(), "{bad:?}");
    }
    assert!(!out.join("x.lua").exists());
}

#[test]
fn a_file_a_killed_run_wrote_is_still_ours_and_its_leftover_temporary_is_removed() {
    let t = TempDir::new("safe-kill");
    let (_, out) = ours_dir(&t);
    let mut o = OutDir::inspect(&out, out.clone(), false).unwrap();
    o.write("a.lua", b"one").unwrap();
    drop(o); // no finish: the run was killed
    // the same name is rewritten with other content by a second run that is killed after the
    // manifest listed it and before the file was renamed
    let mut o = OutDir::inspect(&out, out.clone(), false).unwrap();
    o.write("a.lua", b"two").unwrap();
    drop(o);
    touch(&out.join("missions/.a.lua.tmp-9-9"), b"half");
    let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    fs::File::options().write(true).open(out.join("missions/.a.lua.tmp-9-9")).unwrap().set_modified(long_ago).unwrap();
    let mut o = OutDir::inspect(&out, out.clone(), false).expect("recognised as ours");
    o.wipe().unwrap();
    assert!(!out.join("missions/a.lua").exists());
    assert!(!out.join("missions/.a.lua.tmp-9-9").exists(), "the leftover temporary file goes");
    // both hashes were listed while the run was in flight: a file with the older content is ours too
    let mut o = OutDir::inspect(&out, out.clone(), false).unwrap();
    o.write("c.lua", b"first").unwrap();
    let m = fs::read_to_string(out.join(MANIFEST)).unwrap();
    let h: Vec<&str> = m.lines().skip(1).collect();
    assert_eq!(h.len(), 1);
    drop(o);
}

#[test]
fn the_manifest_is_read_strictly() {
    let t = TempDir::new("safe-manifest");
    let (_, out) = ours_dir(&t);
    let hash = "a".repeat(64);
    for bad in [
        "garbage\n".to_owned(),
        "sta_mission_convert-manifest 2\n".to_owned(),
        format!("sta_mission_convert-manifest 1\n{hash} a.lua\n"),
        format!("sta_mission_convert-manifest 1\n{hash}  ../x.lua\n"),
        format!("sta_mission_convert-manifest 1\n{hash}  a/b.lua\n"),
        "sta_mission_convert-manifest 1\nzz  a.lua\n".to_owned(),
    ] {
        touch(&out.join(MANIFEST), bad.as_bytes());
        let e = OutDir::inspect(&out, out.clone(), true).unwrap_err();
        assert!(e.contains("is damaged"), "{e}");
    }
    // a good one, with Windows line ends
    touch(&out.join(MANIFEST), format!("sta_mission_convert-manifest 1\r\n{hash}  a.lua\r\n").as_bytes());
    assert!(OutDir::inspect(&out, out.clone(), false).is_ok());
}

#[cfg(unix)]
#[test]
fn links_are_refused_at_the_folder_at_missions_at_the_manifest_and_at_files() {
    let t = TempDir::new("safe-out-links");
    let (w, out) = ours_dir(&t);
    // the folder itself is a link: refused, nothing created in the target
    fs::create_dir_all(w.join("real")).unwrap();
    std::os::unix::fs::symlink(w.join("real"), w.join("olink")).unwrap();
    let given = w.join("olink");
    let e = OutDir::inspect(&given, resolve(&given).unwrap(), false).unwrap_err();
    assert!(e.contains("symbolic link"), "{e}");
    // missions/ is a link to a folder that holds a file of somebody else's
    fs::create_dir_all(&out).unwrap();
    touch(&w.join("elsewhere/precious.lua"), b"precious");
    std::os::unix::fs::symlink(w.join("elsewhere"), out.join("missions")).unwrap();
    let e = OutDir::inspect(&out, out.clone(), true).unwrap_err();
    assert!(e.contains("symbolic link"), "{e}");
    fs::remove_file(out.join("missions")).unwrap();
    // the manifest is a link
    touch(&w.join("m.txt"), b"x");
    std::os::unix::fs::symlink(w.join("m.txt"), out.join(MANIFEST)).unwrap();
    assert!(OutDir::inspect(&out, out.clone(), true).unwrap_err().contains("symbolic link"));
    fs::remove_file(out.join(MANIFEST)).unwrap();
    // a file symbolic link where a file would go: not ours, never followed, never replaced
    touch(&w.join("target.txt"), b"target");
    fs::create_dir_all(out.join("missions")).unwrap();
    std::os::unix::fs::symlink(w.join("target.txt"), out.join("missions/x.lua")).unwrap();
    assert!(OutDir::inspect(&out, out.clone(), false).unwrap_err().contains("did not write"));
    let mut o = OutDir::inspect(&out, out.clone(), true).unwrap();
    assert!(o.write("x.lua", b"new").unwrap_err().to_string().contains("refusing to overwrite"));
    assert_eq!(fs::read(w.join("target.txt")).unwrap(), b"target");
    // a wipe never follows it either
    o.wipe().unwrap();
    assert_eq!(fs::read(w.join("target.txt")).unwrap(), b"target");
    assert!(fs::symlink_metadata(out.join("missions/x.lua")).is_ok());
    // missions/ turned into a link after the inspection: the write refuses
    fs::remove_file(out.join("missions/x.lua")).unwrap();
    fs::remove_dir(out.join("missions")).unwrap();
    std::os::unix::fs::symlink(w.join("elsewhere"), out.join("missions")).unwrap();
    let e = o.write("y.lua", b"y").unwrap_err().to_string();
    assert!(e.contains("symbolic link"), "{e}");
    assert!(!w.join("elsewhere/y.lua").exists());
    assert_eq!(fs::read(w.join("elsewhere/precious.lua")).unwrap(), b"precious");
}

// ------------------------------------------------------------------ through the command line

#[test]
fn dotdot_in_a_path_that_does_not_exist_yet_cannot_reach_an_install() {
    let t = TempDir::new("safe-dotdot");
    let w = resolve(t.path()).unwrap();
    let root = w.join("Armada");
    a1_install(&root);
    let before = snapshot(&w);
    let name = "Armada";
    for out in [
        format!("{}/nonexist/../{name}/x", w.display()),
        format!("{}/a/b/../../{name}", w.display()),
        format!("{}/{name}/missions/../../{name}/new", w.display()),
        format!("{}/nonexist/../{name}/missions", w.display()),
    ] {
        let r = run(&["--install", &s(&root), "--out", &out]);
        assert_eq!(r.code, 1, "{out}: {:?} {}", r.out, r.err);
        assert!(r.err.contains("is inside the install"), "{out}: {}", r.err);
        assert!(r.out.is_empty());
        assert_eq!(snapshot(&w), before, "{out}: something was created");
        // the same for the cache folder
        let r = run(&["--install", &s(&root), "--out", &s(&w.join("out")), "--cache-dir", &out]);
        assert_eq!(r.code, 1, "{out}");
        assert!(r.err.starts_with("cache folder ") && r.err.contains("is inside the install"), "{}", r.err);
        assert_eq!(snapshot(&w), before);
    }
    // an output folder that contains the install is refused as well
    let r = run(&["--install", &s(&root), "--out", &s(&w.join("nonexist/.."))]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("contains the install") || r.err.contains("is at or inside the game folder"), "{}", r.err);
    assert_eq!(snapshot(&w), before);
    // and the repository check is made on the same form
    let repo = w.join("repo");
    fs::create_dir_all(repo.join("target")).unwrap();
    let env = Env { repo_root: Some(repo.clone()), target_dirs: vec![repo.join("target")], ..Env::default() };
    let before = snapshot(&w);
    let out = format!("{}/nonexist/../repo/out", w.display());
    let r = run_with(&["--install", &s(&root), "--out", &out], &env);
    assert_eq!(r.code, 1, "{:?}", r.out);
    assert!(r.err.contains("is inside the repository"), "{}", r.err);
    assert_eq!(snapshot(&w), before);
    let r = run_with(&["--install", &s(&root), "--out", &s(&w.join("out")), "--cache-dir", &out], &env);
    assert!(r.err.contains("is inside the repository"), "{}", r.err);
    // the build folder inside it stays allowed
    let r = run_with(&["--install", &s(&root), "--out", &format!("{}/x/../target/o", repo.display())], &env);
    assert!(r.err.is_empty() && r.out[0].starts_with("converting "), "{} {:?}", r.err, r.out);
}

#[cfg(unix)]
#[test]
fn links_in_the_parents_of_the_output_are_resolved_before_the_check() {
    let t = TempDir::new("safe-parents");
    let w = resolve(t.path()).unwrap();
    let root = w.join("Armada");
    a1_install(&root);
    let before = snapshot(&root);
    // a link to the install, to its parent, and a chain through a link and `..`
    std::os::unix::fs::symlink(&root, w.join("to-install")).unwrap();
    std::os::unix::fs::symlink(&w, w.join("to-parent")).unwrap();
    std::os::unix::fs::symlink(root.join("missions"), w.join("to-missions")).unwrap();
    for out in [
        w.join("to-install/new"),
        w.join("to-parent/Armada/new"),
        w.join("to-missions/new/deep"),
        w.join("to-missions/../new"),
        w.join("to-install/missing/../x"),
    ] {
        let r = run(&["--install", &s(&root), "--out", &s(&out)]);
        assert_eq!(r.code, 1, "{out:?}: {}", r.err);
        assert!(r.err.contains("is inside the install"), "{out:?}: {}", r.err);
        assert_eq!(snapshot(&root), before, "{out:?}");
    }
    // the install given through a link is the same install
    let r = run(&["--install", &s(&w.join("to-install")), "--out", &s(&w.join("to-install/new"))]);
    assert_eq!(r.code, 1);
    assert_eq!(snapshot(&root), before);
    // a link as the output folder is refused whatever it points at; a dangling one is no guess
    fs::create_dir_all(w.join("plain")).unwrap();
    std::os::unix::fs::symlink(w.join("plain"), w.join("olink")).unwrap();
    let r = run(&["--install", &s(&root), "--out", &s(&w.join("olink"))]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("symbolic link"), "{}", r.err);
    assert!(fs::read_dir(w.join("plain")).unwrap().next().is_none());
    std::os::unix::fs::symlink(w.join("nowhere"), w.join("dangling")).unwrap();
    let r = run(&["--install", &s(&root), "--out", &s(&w.join("dangling"))]);
    assert_eq!(r.code, 1);
    assert!(!w.join("nowhere").exists());
}

#[test]
fn an_output_folder_that_looks_like_a_game_is_refused_and_its_lua_files_survive() {
    let t = TempDir::new("safe-other-game");
    let w = resolve(t.path()).unwrap();
    let root = w.join("Armada");
    a1_install(&root);
    // a Fleet Operations style folder ships Lua missions
    touch(&w.join("OtherGame/Data/Armada2.exe"), b"");
    touch(&w.join("OtherGame/Data/missions/shipped.lua"), b"shipped");
    // another Armada I install
    touch(&w.join("Other1/Armada.exe"), b"");
    touch(&w.join("Other1/missions/own.lua"), b"own");
    let before = snapshot(&w);
    for out in [
        w.join("OtherGame/Data"),
        w.join("OtherGame"),
        w.join("OtherGame/Data/new"),
        w.join("Other1"),
        w.join("Other1/missions"),
        w.join("Other1/x/y"),
    ] {
        let r = run(&["--install", &s(&root), "--out", &s(&out)]);
        assert_eq!(r.code, 1, "{out:?}");
        assert!(r.err.contains("game folder"), "{out:?}: {}", r.err);
        assert!(r.out.is_empty());
        assert_eq!(snapshot(&w), before, "{out:?}");
    }
}

#[cfg(unix)]
#[test]
fn a_full_run_does_not_delete_through_a_linked_missions_folder() {
    let t = TempDir::new("safe-wipe-link");
    let w = resolve(t.path()).unwrap();
    let root = w.join("Armada");
    a1_install(&root);
    touch(&root.join("missions/install_own.lua"), b"install own");
    let before = snapshot(&root);
    fs::create_dir_all(w.join("out")).unwrap();
    std::os::unix::fs::symlink(root.join("missions"), w.join("out/missions")).unwrap();
    let r = run(&["--install", &s(&root), "--out", &s(&w.join("out"))]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("symbolic link"), "{}", r.err);
    assert_eq!(snapshot(&root), before, "the install lost or gained a file");
}

#[test]
fn arguments_that_could_write_somewhere_else_are_refused() {
    let t = TempDir::new("safe-args");
    let w = resolve(t.path()).unwrap();
    let root = w.join("Armada");
    a1_install(&root);
    // a value that looks like an option is no value (argparse); nothing is written to ./-x
    let r = run(&["--install", &s(&root), "--out", "-x"]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("expected one argument"), "{}", r.err);
    assert!(!Path::new("-x").exists());
    assert_eq!(run(&["--install", &s(&root), "--only", "-x"]).code, 2);
    // `--out=-x` is a value (argparse accepts it); it is relative to the working directory, resolved
    let r = run(&["--install", &s(&root), "--out=-x-never-created", "--cache-dir"]);
    assert_eq!(r.code, 2);
    // the full help names the options and the difference from the Python tool
    let h = run(&["--help"]);
    assert_eq!(h.code, 0);
    let text = h.out.join("\n");
    for needle in ["--install", "--out", "--only", "--include-unmapped", "--keep-foreign", "--cache", "manifest", "deletes only"] {
        assert!(text.contains(needle), "{needle}");
    }
}

// macOS file systems (APFS, HFS+) refuse file names that are not valid UTF-8, so there is no such
// path to write to there.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn a_path_that_is_not_utf8_is_written_where_it_says_not_somewhere_else() {
    use std::os::unix::ffi::OsStrExt;
    let t = TempDir::new("safe-nonutf8");
    let w = resolve(t.path()).unwrap();
    let root = w.join("Armada");
    a1_install(&root);
    let key = cli::a1_cache_key("fed1", &root.join("missions/x1S.dsl"), &root.join("missions/x1R.drl")).unwrap();
    let cdir = w.join("cache");
    Cache::new(&cdir)
        .put(&key, &Entry::Converted { source: b"-- x\n".to_vec(), console: vec!["  fed1: x".into()] })
        .unwrap();
    let mut name = b"o".to_vec();
    name.push(0xff);
    let out = w.join(std::ffi::OsStr::from_bytes(&name));
    let args: Vec<std::ffi::OsString> = vec![
        "--install".into(),
        root.clone().into(),
        "--out".into(),
        out.clone().into(),
        "--cache-dir".into(),
        cdir.clone().into(),
    ];
    let r = run_with(&args, &Env::default());
    assert_eq!(r.code, 0, "{:?} {}", r.out, r.err);
    assert!(out.join("missions/fed1.lua").is_file(), "written where it was asked");
    // no folder with a replacement character was made
    let made: Vec<_> = fs::read_dir(&w).unwrap().flatten().filter(|e| e.file_name().as_bytes().starts_with(b"o")).collect();
    assert_eq!(made.len(), 1, "only the folder that was asked for");
    // a mission name that is not UTF-8 is refused with a message
    let mut bad: Vec<std::ffi::OsString> = args.clone();
    bad.push("--only".into());
    bad.push(std::ffi::OsStr::from_bytes(b"f\xffd").to_os_string());
    let r = run_with(&bad, &Env::default());
    assert_eq!(r.code, 2);
    assert!(r.err.contains("not valid UTF-8"), "{}", r.err);
}

/// The decoder refuses a DLL with a symbol table (a decode error): through the command line that
/// mission is a FAIL line, the run goes on, and the exit code is the wrapper's.
#[test]
fn a_dll_the_decoder_refuses_is_a_failed_mission_and_the_run_goes_on() {
    use sta_mission_convert::fixture::{PeBuilder, CODE};
    let t = TempDir::new("safe-symtab");
    let w = resolve(t.path()).unwrap();
    let bytes = PeBuilder::new(0x1000_0000).section(".text", 0x1000, &[0x55, 0xc3], CODE).build().bytes;
    let pe_at = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    let mut with_table = bytes.clone();
    with_table[pe_at + 12..pe_at + 16].copy_from_slice(&0x2000u32.to_le_bytes());
    with_table[pe_at + 16..pe_at + 20].copy_from_slice(&3u32.to_le_bytes());
    let root = w.join("Armada");
    touch(&root.join("Armada.exe"), b"");
    for n in ["x1S.dsl", "x1R.drl", "x2S.dsl", "x2R.drl"] {
        touch(&root.join("missions").join(n), &with_table);
    }
    touch(&root.join("bzn/fed1.bzn"), b"x1R.drl");
    touch(&root.join("bzn/fed2.bzn"), b"x2R.drl");
    let (mut so, mut se) = (Vec::new(), Vec::new());
    let mut listing = |p: &Path| sta_mission_convert::disasm::disassemble(&sta_mission_convert::Pe::open(p)?);
    let args = ["--install".to_owned(), s(&root), "--out".to_owned(), s(&w.join("out"))];
    let code = cli::run(&args, &Env::default(), &mut Io { out: &mut so, err: &mut se }, &mut listing);
    let lines: Vec<String> = String::from_utf8_lossy(&so).lines().map(str::to_owned).collect();
    assert_eq!(code, 1, "{lines:?}");
    assert_eq!(lines.len(), 3, "both missions were tried: {lines:?}");
    assert!(lines[1].starts_with("  FAIL fed1: DecodeError: the file has a COFF symbol table"), "{}", lines[1]);
    assert!(lines[2].starts_with("  FAIL fed2: DecodeError: the file has a COFF symbol table"), "{}", lines[2]);
    assert!(String::from_utf8_lossy(&se).is_empty());
    assert!(!w.join("out/missions").exists() || fs::read_dir(w.join("out/missions")).unwrap().next().is_none());
}

// ------------------------------------------------------------------ the last low items

const PY_HEADER: &str = "-- m1: converted by tools/dsl_to_lua.py (LUA-11) from m1S.dsl and m1R.drl.\nbody\n";

#[test]
fn a_python_tools_earlier_conversion_is_moved_aside_never_deleted_and_nothing_else_is() {
    let t = TempDir::new("safe-python-output");
    let (_, out) = ours_dir(&t);
    let edited = format!("{PY_HEADER}the player added this line\n");
    touch(&out.join("missions/old1.lua"), edited.as_bytes());
    touch(&out.join("missions/old2.lua"), "-- m2: converted by tools/a1_to_lua.py (LUA-14) from x.dsl.\nx\n".as_bytes());
    // not the header: the player's
    touch(&out.join("missions/mine.lua"), b"-- converted by tools/dsl_to_lua.py\n");
    touch(&out.join("missions/notes.txt"), PY_HEADER.as_bytes());
    assert!(OutDir::inspect(&out, out.clone(), false).unwrap_err().contains("did not write"));
    let mut o = OutDir::inspect(&out, out.clone(), true).unwrap();
    o.wipe().unwrap();
    // the Python tool's files are renamed, content intact; nothing was deleted
    let m = out.join("missions");
    assert!(!m.join("old1.lua").exists() && !m.join("old2.lua").exists());
    assert_eq!(fs::read(m.join("old1.lua.old")).unwrap(), edited.as_bytes());
    assert!(fs::read(m.join("old2.lua.old")).unwrap().starts_with(b"-- m2: converted by tools/a1_to_lua.py"));
    let mut moved = o.take_moved();
    moved.sort();
    assert_eq!(moved, [("old1.lua".to_owned(), "old1.lua.old".to_owned()), ("old2.lua".to_owned(), "old2.lua.old".to_owned())]);
    assert!(m.join("mine.lua").exists() && m.join("notes.txt").exists());
    // a second full run moves and loses nothing further
    let before: Vec<_> = {
        let mut v: Vec<_> = fs::read_dir(&m).unwrap().flatten().map(|e| (e.file_name(), fs::read(e.path()).unwrap())).collect();
        v.sort();
        v
    };
    let mut o = OutDir::inspect(&out, out.clone(), true).unwrap();
    o.wipe().unwrap();
    assert!(o.take_moved().is_empty());
    let after: Vec<_> = {
        let mut v: Vec<_> = fs::read_dir(&m).unwrap().flatten().map(|e| (e.file_name(), fs::read(e.path()).unwrap())).collect();
        v.sort();
        v
    };
    assert_eq!(before, after);
    // a new file of the Python tool with a name that is taken: the next name, nothing overwritten
    touch(&m.join("old1.lua"), PY_HEADER.as_bytes());
    o.wipe().unwrap();
    assert_eq!(fs::read(m.join("old1.lua.old")).unwrap(), edited.as_bytes());
    assert_eq!(fs::read(m.join("old1.lua.old2")).unwrap(), PY_HEADER.as_bytes());
    // a write that would replace one moves it aside first
    let mut o = OutDir::inspect(&out, out.clone(), true).unwrap();
    touch(&m.join("m9.lua"), PY_HEADER.as_bytes());
    o.write("m9.lua", b"new").unwrap();
    assert_eq!(fs::read(m.join("m9.lua")).unwrap(), b"new");
    assert_eq!(fs::read(m.join("m9.lua.old")).unwrap(), PY_HEADER.as_bytes());
    assert_eq!(o.take_moved(), [("m9.lua".to_owned(), "m9.lua.old".to_owned())]);
    // only regular files: a link with the header is not moved and not followed
    #[cfg(unix)]
    {
        touch(&out.join("target.txt"), PY_HEADER.as_bytes());
        std::os::unix::fs::symlink(out.join("target.txt"), m.join("link.lua")).unwrap();
        let mut o = OutDir::inspect(&out, out.clone(), true).unwrap();
        o.wipe().unwrap();
        assert!(out.join("target.txt").exists());
        assert!(fs::symlink_metadata(m.join("link.lua")).is_ok());
        assert!(!m.join("link.lua.old").exists());
    }
}

#[test]
fn the_wipe_removes_only_the_tools_own_old_temporary_files() {
    let t = TempDir::new("safe-temps");
    let (_, out) = ours_dir(&t);
    let m = out.join("missions");
    let old = |name: &str| {
        touch(&m.join(name), b"half");
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        fs::File::options().write(true).open(m.join(name)).unwrap().set_modified(when).unwrap();
    };
    old(".a.lua.tmp-123-4"); // the tool's own pattern, old: goes
    old(".mine.tmp-notes"); // dot file with another shape: stays
    old(".x.tmp-1-2-3"); // wrong shape: stays
    old("plain.tmp-1-2"); // no leading dot: stays
    touch(&m.join(".b.lua.tmp-5-6"), b"in flight"); // recent: a running process's: stays
    let mut o = OutDir::inspect(&out, out.clone(), false).unwrap();
    o.wipe().unwrap();
    assert!(!m.join(".a.lua.tmp-123-4").exists());
    for stays in [".mine.tmp-notes", ".x.tmp-1-2-3", "plain.tmp-1-2", ".b.lua.tmp-5-6"] {
        assert!(m.join(stays).exists(), "{stays}");
    }
}

#[cfg(unix)]
#[test]
fn the_output_folder_is_checked_before_it_is_created() {
    let t = TempDir::new("safe-check-first");
    let (w, out) = ours_dir(&t);
    let mut o = OutDir::inspect(&out.join("deep/er"), out.join("deep/er"), false).unwrap();
    // an ancestor appears as a link to somewhere else after the inspection: nothing is made there
    fs::create_dir_all(w.join("elsewhere")).unwrap();
    fs::create_dir_all(&out).unwrap();
    std::os::unix::fs::symlink(w.join("elsewhere"), out.join("deep")).unwrap();
    let e = o.write("a.lua", b"x").unwrap_err().to_string();
    assert!(e.contains("changed while the tool ran"), "{e}");
    assert!(fs::read_dir(w.join("elsewhere")).unwrap().next().is_none(), "nothing was created there");
}

#[test]
fn only_the_markers_of_an_install_make_a_folder_a_game_folder() {
    let t = TempDir::new("safe-markers");
    let w = resolve(t.path()).unwrap();
    // a downloads folder with an installer named like a game does not block what is below it
    touch(&w.join("Downloads/Armada2_patch.exe"), b"");
    touch(&w.join("Downloads/armada_setup.exe"), b"");
    assert_eq!(game_root_above(&w.join("Downloads/out")), None);
    // the exact executables do
    touch(&w.join("G1/ARMADA2.EXE"), b"");
    touch(&w.join("G2/Data/ArmadaL.exe"), b"");
    touch(&w.join("G3/Data/other.exe"), b"");
    assert!(game_root_above(&w.join("G1/x")).is_some());
    assert!(game_root_above(&w.join("G2/x")).is_some());
    assert_eq!(game_root_above(&w.join("G3/x")), None);
    // a folder named like the executable is not one
    fs::create_dir_all(w.join("G4/Armada.exe")).unwrap();
    assert_eq!(game_root_above(&w.join("G4/x")), None);
}

/// Through the command line: the Python tool's file is listed as moved aside and counted in the
/// summary, and a second full run says nothing more about it.
#[test]
fn the_command_line_reports_the_files_it_moves_aside() {
    let t = TempDir::new("safe-moved-cli");
    let w = resolve(t.path()).unwrap();
    let root = w.join("Armada");
    a1_install(&root);
    let key = cli::a1_cache_key("fed1", &root.join("missions/x1S.dsl"), &root.join("missions/x1R.drl")).unwrap();
    let cdir = w.join("cache");
    Cache::new(&cdir)
        .put(&key, &Entry::Converted { source: b"-- new\n".to_vec(), console: vec!["  fed1: x".into()] })
        .unwrap();
    let out = w.join("out");
    touch(&out.join("missions/fed1.lua"), PY_HEADER.as_bytes());
    let args = ["--install", &s(&root), "--out", &s(&out), "--cache-dir", &s(&cdir)];
    let r = run(&args);
    assert_eq!(r.code, 0, "{:?} {}", r.out, r.err);
    assert!(r.out.iter().any(|l| l.contains("moved aside") && l.contains("missions/fed1.lua -> missions/fed1.lua.old")), "{:?}", r.out);
    assert!(r.out.iter().any(|l| l.starts_with("1 file(s) of the Python tool moved aside")), "{:?}", r.out);
    assert!(r.out.iter().any(|l| l.starts_with("1 mission scripts in ")), "{:?}", r.out);
    assert_eq!(fs::read(out.join("missions/fed1.lua")).unwrap(), b"-- new\n");
    assert_eq!(fs::read(out.join("missions/fed1.lua.old")).unwrap(), PY_HEADER.as_bytes());
    let r = run(&args);
    assert_eq!(r.code, 0);
    assert!(r.out.iter().all(|l| !l.contains("moved aside")), "{:?}", r.out);
    assert_eq!(fs::read(out.join("missions/fed1.lua.old")).unwrap(), PY_HEADER.as_bytes());
}

// ------------------------------------------------------------------ the aside names, the exit code

#[cfg(unix)]
#[test]
fn a_read_only_folder_with_a_python_file_is_an_error_not_a_loop() {
    use std::os::unix::fs::PermissionsExt;
    let t = TempDir::new("safe-readonly");
    let (_, out) = ours_dir(&t);
    let m = out.join("missions");
    touch(&m.join("old1.lua"), PY_HEADER.as_bytes());
    fs::set_permissions(&m, fs::Permissions::from_mode(0o555)).unwrap();
    // a user that can write anyway (root) cannot show this
    let probe = m.join(".probe");
    if fs::write(&probe, b"x").is_ok() {
        let _ = fs::remove_file(&probe);
        fs::set_permissions(&m, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let mut o = OutDir::inspect(&out, out.clone(), true).unwrap();
    let e = o.wipe().unwrap_err();
    assert!(e.contains("cannot move") && e.contains("aside"), "{e}");
    // and a write that would replace it fails that file, with a collision (the mission fails)
    let e = o.write("old1.lua", b"new").unwrap_err();
    assert!(matches!(e, sta_mission_convert::output::WriteError::Collision(_) | sta_mission_convert::output::WriteError::Fatal(_)), "{e}");
    fs::set_permissions(&m, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(fs::read(m.join("old1.lua")).unwrap(), PY_HEADER.as_bytes(), "nothing was lost");
    assert!(fs::read_dir(&m).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().contains(".old")), "no reservation stays");
}

#[test]
fn when_every_aside_name_is_taken_the_file_stays_and_the_error_says_so() {
    let t = TempDir::new("safe-aside-cap");
    let (_, out) = ours_dir(&t);
    let m = out.join("missions");
    touch(&m.join("old1.lua"), PY_HEADER.as_bytes());
    touch(&m.join("old1.lua.old"), b"taken");
    for k in 2..=sta_mission_convert::output::MAX_ASIDE {
        touch(&m.join(format!("old1.lua.old{k}")), b"taken");
    }
    let mut o = OutDir::inspect(&out, out.clone(), true).unwrap();
    let e = o.wipe().unwrap_err();
    assert!(e.contains("are all taken"), "{e}");
    assert_eq!(fs::read(m.join("old1.lua")).unwrap(), PY_HEADER.as_bytes());
    assert_eq!(fs::read(m.join("old1.lua.old")).unwrap(), b"taken");
    // one name free in the middle is found
    fs::remove_file(m.join("old1.lua.old57")).unwrap();
    let mut o = OutDir::inspect(&out, out.clone(), true).unwrap();
    o.wipe().unwrap();
    assert_eq!(fs::read(m.join("old1.lua.old57")).unwrap(), PY_HEADER.as_bytes());
    assert!(!m.join("old1.lua").exists());
}

/// A run in which any mission failed exits 1 even when other scripts were written, and still
/// prints the warning and the summary (the Python wrapper exits 0 there).
#[test]
fn a_run_with_a_failed_mission_exits_one_even_when_other_scripts_exist() {
    let t = TempDir::new("safe-exit");
    let w = resolve(t.path()).unwrap();
    let root = w.join("Armada");
    a1_install(&root);
    touch(&root.join("missions/x2S.dsl"), b"junk");
    touch(&root.join("missions/x2R.drl"), b"junk");
    touch(&root.join("bzn/fed2.bzn"), b"x2R.drl");
    let key = cli::a1_cache_key("fed1", &root.join("missions/x1S.dsl"), &root.join("missions/x1R.drl")).unwrap();
    let cdir = w.join("cache");
    Cache::new(&cdir)
        .put(&key, &Entry::Converted { source: b"-- ok\n".to_vec(), console: vec!["  fed1: ok".into()] })
        .unwrap();
    let r = run(&["--install", &s(&root), "--out", &s(&w.join("out")), "--cache-dir", &s(&cdir)]);
    assert_eq!(r.code, 1, "{:?} {}", r.out, r.err);
    assert!(r.out.iter().any(|l| l.starts_with("  FAIL fed2: ")));
    assert!(r.out.iter().any(|l| l.starts_with("warning: some missions failed")));
    assert!(r.out.iter().any(|l| l.starts_with("1 mission scripts in ")));
    assert_eq!(fs::read(w.join("out/missions/fed1.lua")).unwrap(), b"-- ok\n");
    // no failure: exit 0
    let r = run(&["--install", &s(&root), "--out", &s(&w.join("out")), "--cache-dir", &s(&cdir), "--only", "fed1"]);
    assert_eq!(r.code, 0, "{:?} {}", r.out, r.err);
}

/// The folder check wants the resolved folder and says so when it gets another form of it: the
/// mistake is refused when the folder is opened, on every system, not at the first write and only
/// where a temp folder happens to lie behind a link.
#[cfg(unix)]
#[test]
fn a_folder_that_is_not_in_its_resolved_form_is_refused_when_it_is_opened() {
    let t = TempDir::new("safe-unresolved");
    let (w, _) = ours_dir(&t);
    fs::create_dir_all(w.join("real")).unwrap();
    std::os::unix::fs::symlink(w.join("real"), w.join("via")).unwrap();
    // the same folder twice: through the link, and resolved
    let (through, resolved) = (w.join("via/out"), w.join("real/out"));
    assert_eq!(resolve(&through).unwrap(), resolved);
    let e = OutDir::inspect(&through, through.clone(), false).unwrap_err();
    assert!(e.contains("is not its resolved form"), "{e}");
    assert!(!resolved.exists(), "nothing was created");
    // handed the resolved form, the same folder is usable, and a write goes through
    let mut o = OutDir::inspect(&through, resolved.clone(), false).unwrap();
    o.write("a.lua", b"one").unwrap();
    o.finish().unwrap();
    assert!(resolved.join("missions/a.lua").is_file());
    // the read-only helper resolves for itself: an unresolved path is fine there
    assert!(sta_mission_convert::output::edited_scripts(&through).is_empty());
    fs::write(resolved.join("missions/a.lua"), b"changed").unwrap();
    assert_eq!(sta_mission_convert::output::edited_scripts(&through), vec!["a.lua".to_owned()]);
}
