//! The compiled missions of a mod inside an install (#927, ADR 0004 D6): `Install::open_mod` and
//! the command line's `--mod` and `--engine`, on authored folders. The mission DLLs are junk and
//! the engine files are the authored ones of `fixture::write_checked_engine`, so nothing converts
//! here: a converted mission is an entry the test put in the cache under the key the run asks
//! for (as `tests/install_api.rs` does). What is tested is which missions are found, under which
//! names they are written, where the output goes and what is refused.

#[path = "parity/harness.rs"]
mod harness;

use std::fs;
use std::path::{Path, PathBuf};

use harness::TempDir;
use sta_mission_convert::cache::{Cache, Entry};
use sta_mission_convert::cli::{self, Io};
use sta_mission_convert::disasm::Ins;
use sta_mission_convert::fixture::write_checked_engine;
use sta_mission_convert::install::{self, Env, Event, Game, Install, InstallError, ModError, Options, Summary};
use sta_mission_convert::output::OutDir;
use sta_mission_convert::{Pe, Result};

fn write(p: PathBuf, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

/// An install of the kind that hosts mods (its data and its engine in `Data/`), with two mods.
/// `Base` holds two mission DLL pairs and two maps: `alpha` runs `one`, `beta` runs `two`.
/// `Child` names `Base` as its parent and holds no DLL: it replaces `beta` (which then runs
/// `one`) and adds `gamma`, which runs `two`.
fn hosting_install(root: &Path) {
    let files: [(&str, &[u8]); 10] = [
        ("Data/Armada2.exe", b""),
        ("Data/Mods/Base/info.ini", b"[Mod]\n"),
        ("Data/Mods/Base/missions/oneS.dsl", b"junk mission one"),
        ("Data/Mods/Base/missions/oneR.drl", b"junk rules one"),
        ("Data/Mods/Base/missions/twoS.dsl", b"junk mission two"),
        ("Data/Mods/Base/missions/twoR.drl", b"junk rules two"),
        ("Data/Mods/Base/bzn/alpha.bzn", b"xx oneS.dsl yy"),
        ("Data/Mods/Base/bzn/beta.bzn", b"xx twoS.dsl yy"),
        ("Data/Mods/Child/info.ini", b"[Mod]\nParentMod=Base\n"),
        ("Data/Mods/Child/bzn/beta.bzn", b"xx oneS.dsl yy"),
    ];
    for (rel, bytes) in files {
        write(root.join(rel), bytes);
    }
    write(root.join("Data/Mods/Child/bzn/gamma.bzn"), b"xx twoS.dsl yy");
}

/// The folders of one test: the hosting install, a folder with authored engine files, and a
/// place for output, cache and user data.
struct Scene {
    tmp: TempDir,
    root: PathBuf,
    engine: PathBuf,
}

fn scene(tag: &str) -> Scene {
    let tmp = TempDir::new(tag);
    let (root, engine) = (tmp.path().join("Hosting Install"), tmp.path().join("engine"));
    hosting_install(&root);
    write_checked_engine(&engine).unwrap();
    Scene { tmp, root, engine }
}

fn open(s: &Scene, name: &str) -> Install {
    Install::open_mod(&s.root, name).expect("the mount takes the mod").with_engine(&s.engine).expect("the engine folder")
}

/// The missions a discovery finds: name and the stems it is written under.
fn found(inst: &Install, only: &[&str]) -> Vec<(String, Vec<String>)> {
    let opts = Options { only: only.iter().map(|s| (*s).to_owned()).collect(), ..Options::default() };
    let d = inst.discover(&opts).expect("the engine files load");
    d.missions.iter().map(|m| (m.name.clone(), m.targets.clone())).collect()
}

fn names(v: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    v.iter().map(|(n, t)| ((*n).to_owned(), t.iter().map(|s| (*s).to_owned()).collect())).collect()
}

/// A decoder that must not be reached: a cached mission is not parsed, and junk is refused by
/// the parser before the decoder is asked.
fn no_decoder(_: &Path, _: &Pe) -> Result<Vec<Ins>> {
    panic!("the decoder was asked for a listing");
}

/// Puts an invented script for the mission `name` of the mod folder `owner` in the cache, under
/// the key a run with this scene's engine asks for.
fn plant(cache: &Cache, s: &Scene, owner: &str, name: &str) {
    let dir = s.root.join("Data/Mods").join(owner).join("missions");
    let (dsl, drl) = (dir.join(format!("{name}S.dsl")), dir.join(format!("{name}R.drl")));
    let key = install::a2_cache_key(&dsl, Some(&drl), &s.engine).expect("the files and the engine exist");
    let entry = Entry::Converted {
        source: format!("-- invented script {name}\nreturn 1\n").into_bytes(),
        console: vec![format!("  {name}: invented")],
    };
    cache.put(&key, &entry).unwrap();
}

fn convert(inst: &Install, out: &Path, cache: &Cache) -> (Summary, Vec<Event>) {
    let res = install::resolve_output(out, inst, &Env::default()).expect("a fresh folder is allowed");
    let mut dir = OutDir::inspect(out, res, false).expect("the folder is usable");
    let mut events = Vec::new();
    let mut dec = no_decoder;
    let summary =
        install::convert_install(inst, &Options::default(), &mut dir, Some(cache), &mut dec, &|| false, &mut |e: &Event| events.push(e.clone()));
    (summary, events)
}

fn lua_files(out: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(out.join("missions"))
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Every file under `root` with its bytes (an install must not change).
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.push((p.clone(), Vec::new()));
                walk(&p, out);
            } else {
                out.push((p.clone(), fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

#[test]
fn a_mods_missions_are_the_ones_its_mount_shows_named_by_the_maps_it_shows() {
    let s = scene("mod-found");
    // the mod that holds the DLLs: each mission under the map that runs it
    let base = open(&s, "Base");
    assert_eq!((base.game, base.active_mod()), (Game::Armada2, Some("Base")));
    assert_eq!(found(&base, &[]), names(&[("one", &["alpha"]), ("two", &["beta"])]));
    // its child has no DLL of its own: the parent's missions, under the names the child's maps
    // give them (the replaced map now runs the other mission, the added map is a new name)
    let child = open(&s, "Child");
    assert_eq!(child.active_mod(), Some("Child"));
    assert_eq!(found(&child, &[]), names(&[("one", &["alpha", "beta"]), ("two", &["gamma"])]));
    assert_eq!(found(&child, &["TWO"]), names(&[("two", &["gamma"])]), "--only selects by mission name, any case");
    // a file one mod of the chain overrides is the overriding one
    write(s.root.join("Data/Mods/Child/missions/oneS.dsl"), b"the child's own one");
    let d = open(&s, "Child").discover(&Options::default()).unwrap();
    assert_eq!(d.missions.len(), 2);
    let (events, cache) = (s.tmp.path().join("out"), Cache::new(s.tmp.path().join("cache")));
    plant(&cache, &s, "Base", "one"); // the parent's bytes: not what the child's mount shows
    let (summary, _) = convert(&open(&s, "Child"), &events, &cache);
    let one = summary.missions.iter().find(|m| m.mission == "one").unwrap();
    assert!(!one.cached && !one.converted, "the overriding DLL has other bytes, so another key");
}

#[test]
fn a_mod_that_hides_its_parents_maps_names_the_missions_by_its_own_maps_only() {
    let s = scene("mod-hidden-maps");
    write(s.root.join("Data/Mods/Child/info.ini"), b"[Mod]\nParentMod=Base\nignoreParentMaps=1\n");
    // `alpha` of the parent is hidden; `one` keeps the child's `beta`, `two` the child's `gamma`
    assert_eq!(found(&open(&s, "Child"), &[]), names(&[("one", &["beta"]), ("two", &["gamma"])]));
    // a mission no shown map names is written under its own name
    fs::remove_file(s.root.join("Data/Mods/Child/bzn/gamma.bzn")).unwrap();
    assert_eq!(found(&open(&s, "Child"), &[]), names(&[("one", &["beta"]), ("two", &["two"])]));
}

#[test]
fn a_standalone_mod_does_not_convert_what_lies_below_it() {
    let s = scene("mod-standalone");
    write(s.root.join("Data/Mods/Child/info.ini"), b"[Mod]\nParentMod=Base\nstandalone=1\n");
    assert_eq!(found(&open(&s, "Child"), &[]), names(&[]), "the parent's DLLs are not in a standalone mod's mount");
}

#[test]
fn the_conversion_is_written_for_the_install_with_that_mod_only() {
    let s = scene("mod-key");
    let data = s.tmp.path().join("data");
    let cdir = s.tmp.path().join("cache");
    let cache = Cache::new(&cdir);
    plant(&cache, &s, "Base", "one");
    plant(&cache, &s, "Base", "two");
    let before = (snapshot(&s.root), snapshot(&s.engine));

    let env = Env { data_dir: Some(data.clone()), ..Env::default() };
    let run = |args: &[&str]| {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut listing = |_: &Path| -> Result<Vec<Ins>> { panic!("the decoder was asked for a listing") };
        let code = cli::run(args, &env, &mut Io { out: &mut out, err: &mut err }, &mut listing);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    };
    let (root, engine, cached) = (s.root.to_str().unwrap(), s.engine.to_str().unwrap(), cdir.to_str().unwrap());
    let (code, out, err) = run(&["--install", root, "--mod", "Child", "--engine", engine, "--cache-dir", cached]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");

    // under the key of the install WITH the mod, the same rule the game mounts by
    let folder = |m: Option<&str>| data.join("converted").join(sta_core::install_key::key(&s.root, m));
    assert_eq!(lua_files(&folder(Some("Child"))), ["alpha.lua", "beta.lua", "gamma.lua"]);
    assert!(out.contains(&format!("(mod Child) -> {}", folder(Some("Child")).display())), "{out}");
    assert_eq!(fs::read(folder(Some("Child")).join("missions/gamma.lua")).unwrap(), b"-- invented script two\nreturn 1\n");
    // nothing for the install without a mod, nor for the parent mod: each is another folder
    assert_ne!(folder(Some("Child")), folder(None));
    assert_ne!(folder(Some("Child")), folder(Some("Base")));
    assert!(!folder(None).exists() && !folder(Some("Base")).exists());
    let kept: Vec<_> = fs::read_dir(data.join("converted")).unwrap().flatten().map(|e| e.path()).collect();
    assert_eq!(kept, [folder(Some("Child"))]);

    // the parent's own conversion is a second folder with the parent's names
    let (code, _, err) = run(&["--install", root, "--mod", "Base", "--engine", engine, "--cache-dir", cached]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(lua_files(&folder(Some("Base"))), ["alpha.lua", "beta.lua"]);
    assert_eq!(lua_files(&folder(Some("Child"))), ["alpha.lua", "beta.lua", "gamma.lua"], "untouched by the other run");
    // the install and the engine folder were only read
    assert!(before == (snapshot(&s.root), snapshot(&s.engine)));
}

#[test]
fn an_install_and_a_mod_the_mount_refuses_are_refused_with_its_reason() {
    let s = scene("mod-refused");
    // without a mod, an install of this kind keeps its refusal
    assert!(matches!(Install::open(&s.root).unwrap_err(), InstallError::FleetOperations { .. }));
    // a mod the install does not have
    let ModError::Mount { name, reason, root, .. } = Install::open_mod(&s.root, "Nowhere").unwrap_err() else { panic!("the mount's refusal") };
    assert_eq!((name.as_str(), root), ("Nowhere", fs::canonicalize(&s.root).unwrap()));
    let mount_says = sta_vfs::Vfs::mount_install(&s.root, Some("Nowhere")).unwrap_err().to_string();
    assert_eq!(reason, mount_says, "the mount's own text, no second rule");
    // an empty name would key the output like the install without a mod
    assert!(matches!(Install::open_mod(&s.root, " ").unwrap_err(), ModError::Mount { .. }));
    // a root without a mods folder: here a stock shaped Armada II root
    let stock = s.tmp.path().join("stock");
    for rel in ["Armada2.exe", "missions/oneS.dsl", "odf/x.odf", "bzn/alpha.bzn"] {
        write(stock.join(rel), b"junk");
    }
    assert!(Install::open(&stock).is_ok(), "the base game's own conversion is as before");
    assert!(matches!(Install::open_mod(&stock, "Child").unwrap_err(), ModError::Mount { .. }));
    assert_eq!(Install::open_mod(&s.tmp.path().join("missing"), "Child").unwrap_err(), ModError::NotAFolder { path: s.tmp.path().join("missing") });

    // the command line words them
    let run = |args: &[&str]| {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut listing = |_: &Path| -> Result<Vec<Ins>> { panic!("the decoder was asked for a listing") };
        let code = cli::run(args, &Env::default(), &mut Io { out: &mut out, err: &mut err }, &mut listing);
        (code, String::from_utf8(err).unwrap())
    };
    let (root, out) = (s.root.to_str().unwrap(), s.tmp.path().join("o"));
    let out = out.to_str().unwrap();
    let (code, err) = run(&["--install", root, "--mod", "Nowhere", "--out", out]);
    assert_eq!((code, err.trim_end()), (1, format!("--install {root} --mod Nowhere: {mount_says}").as_str()));
    let (code, err) = run(&["--install", root, "--out", out]);
    assert!(code == 1 && err.contains("there are no mission DLLs to convert"), "{err}");
    let (code, err) = run(&["--install", root, "--mod", "Child", "--engine", s.tmp.path().join("missing").to_str().unwrap(), "--out", out]);
    assert!(code == 1 && err.contains("--engine") && err.contains("not a folder"), "{err}");
    assert!(!Path::new(out).exists(), "a refused run writes nothing");
}

#[test]
fn a_mod_needs_engine_files_and_the_folder_they_are_read_from_is_read_only() {
    let s = scene("mod-engine");
    // the hosting install has no engine files of the kind the conversion reads at its root
    let alone = Install::open_mod(&s.root, "Child").unwrap();
    assert_eq!(alone.engine_root(), fs::canonicalize(&s.root).unwrap());
    let e = alone.discover(&Options::default()).unwrap_err();
    assert!(e.contains("--engine"), "the refusal asks for an Armada II root: {e}");
    // the same through the command line: nothing is written, the run fails
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut listing = |_: &Path| -> Result<Vec<Ins>> { panic!("the decoder was asked for a listing") };
    let dest = s.tmp.path().join("o");
    let args = ["--install", s.root.to_str().unwrap(), "--mod", "Child", "--out", dest.to_str().unwrap()];
    let code = cli::run(&args, &Env::default(), &mut Io { out: &mut out, err: &mut err }, &mut listing);
    assert!(code == 1 && String::from_utf8(err).unwrap().contains("--engine"));
    assert_eq!(lua_files(&dest), Vec::<String>::new());

    // with the folder named, it is read; no output or cache may go into it
    let inst = open(&s, "Child");
    assert_eq!(inst.engine_root(), fs::canonicalize(&s.engine).unwrap());
    assert!(inst.discover(&Options::default()).is_ok());
    let inside = s.engine.join("out");
    assert!(install::resolve_output(&inside, &inst, &Env::default()).unwrap_err().contains("read-only"));
    let out_res = install::resolve_output(&s.tmp.path().join("fine"), &inst, &Env::default()).unwrap();
    assert!(install::open_cache(&s.engine.join("cache"), &out_res, &inst, &Env::default()).is_err());
    // nor around it: an engine folder elsewhere, and an output folder that would contain it
    let far = TempDir::new("mod-engine-far");
    write_checked_engine(&far.path().join("engine")).unwrap();
    let elsewhere = Install::open_mod(&s.root, "Child").unwrap().with_engine(&far.path().join("engine")).unwrap();
    let why = install::resolve_output(far.path(), &elsewhere, &Env::default()).unwrap_err();
    assert!(why.contains("contains the install"), "{why}");
    assert!(install::resolve_output(far.path(), &Install::open_mod(&s.root, "Child").unwrap(), &Env::default()).is_ok(), "only a folder the run reads is guarded");
    assert!(matches!(Install::open_mod(&s.root, "Child").unwrap().with_engine(&s.engine.join("missing")).unwrap_err(), InstallError::NotAFolder { .. }));
    // engine files that are not the ones the converter was read from are refused by its slot check
    let other = s.tmp.path().join("other-engine");
    write(other.join("Armada2.exe"), b"not an executable");
    write(other.join("armada2.map"), b"");
    assert!(Install::open_mod(&s.root, "Child").unwrap().with_engine(&other).unwrap().discover(&Options::default()).is_err());
}

#[test]
fn a_mod_is_named_by_a_folder_name_and_every_folder_its_mount_reads_is_read_only() {
    let s = scene("mod-outside");
    // a name that is a path is refused before anything is mounted
    let outside = s.tmp.path().join("outside");
    write(outside.join("info.ini"), b"[Mod]\n");
    write(outside.join("missions/farS.dsl"), b"junk");
    let absolute = outside.to_str().unwrap().to_owned();
    for name in ["..", ".", "../../outside", "Base/../Child", "Base\\..\\Child", absolute.as_str()] {
        let ModError::Mount { reason, .. } = Install::open_mod(&s.root, name).unwrap_err() else { panic!("{name}") };
        assert!(reason.contains("not by a path"), "{name}: {reason}");
    }
    // a parent entry that leads out of the install: the mount follows it, so that folder is read
    // and nothing may be written into it or around it
    write(s.root.join("Data/Mods/Child/info.ini"), b"[Mod]\nParentMod=../../../outside\n");
    let inst = open(&s, "Child");
    assert_eq!(found(&inst, &[]).iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["far"], "the mount reads the outside folder");
    let why = install::resolve_output(&outside.join("out"), &inst, &Env::default()).unwrap_err();
    assert!(why.contains("read-only"), "{why}");
    let out_res = install::resolve_output(&s.tmp.path().join("fine"), &inst, &Env::default()).unwrap();
    assert!(install::open_cache(&outside.join("cache"), &out_res, &inst, &Env::default()).is_err());
    // with the plain parent again that folder is no part of the run
    write(s.root.join("Data/Mods/Child/info.ini"), b"[Mod]\nParentMod=Base\n");
    assert!(install::resolve_output(&outside.join("out"), &open(&s, "Child"), &Env::default()).is_ok());
}

/// A mod folder that is a link to a folder elsewhere: the folder it leads to is guarded.
#[cfg(unix)]
#[test]
fn a_linked_mod_folder_is_guarded_where_it_really_is() {
    let s = scene("mod-linked");
    let real = s.tmp.path().join("elsewhere");
    write(real.join("info.ini"), b"[Mod]\nParentMod=Base\n");
    std::os::unix::fs::symlink(&real, s.root.join("Data/Mods/Linked")).unwrap();
    let inst = open(&s, "Linked");
    let why = install::resolve_output(&real.join("out"), &inst, &Env::default()).unwrap_err();
    assert!(why.contains("read-only"), "{why}");
    // and inside its parent mod's folder, which is in the install
    assert!(install::resolve_output(&s.root.join("Data/Mods/Base/out"), &inst, &Env::default()).is_err());
}

#[test]
fn a_mission_dll_inside_an_archive_is_reported_and_the_others_convert() {
    use sta_vfs::sta_write::{Compression, Content, pack};
    let s = scene("mod-packed");
    // the child ships a third mission inside a package instead of as a file
    let files = vec![
        ("missions/threeS.dsl".to_owned(), Content::Data(b"junk mission three".to_vec())),
        ("missions/threeR.drl".to_owned(), Content::Data(b"junk rules three".to_vec())),
    ];
    write(s.root.join("Data/Mods/Child/child.sta"), &pack(files, Compression::None).unwrap());
    let cache = Cache::new(s.tmp.path().join("cache"));
    plant(&cache, &s, "Base", "one");
    plant(&cache, &s, "Base", "two");
    let out = s.tmp.path().join("out");
    let (summary, events) = convert(&open(&s, "Child"), &out, &cache);
    assert!(summary.aborted.is_none() && !summary.cancelled);
    let by_name = |n: &str| summary.missions.iter().find(|m| m.mission == n).unwrap_or_else(|| panic!("{n} is reported"));
    assert!(by_name("one").converted && by_name("two").converted);
    let three = by_name("three");
    assert!(!three.converted && three.written.is_empty());
    assert!(three.failures.len() == 1 && three.failures[0].contains("inside an archive"), "{:?}", three.failures);
    assert!(events.iter().any(|e| matches!(e, Event::Failed { mission, .. } if mission == "three")));
    assert_eq!(lua_files(&out), ["alpha.lua", "beta.lua", "gamma.lua"]);
}

#[test]
fn the_engine_folder_argument_serves_a_stock_shaped_install_too_and_armada_one_refuses_it() {
    let s = scene("mod-engine-stock");
    // an Armada II root whose own engine files are not usable, with the engine folder named
    let stock = s.tmp.path().join("stock");
    for (rel, bytes) in [("Armada2.exe", &b""[..]), ("missions/oneS.dsl", b"junk"), ("missions/oneR.drl", b"junk"), ("bzn/alpha.bzn", b"xx oneS.dsl yy")] {
        write(stock.join(rel), bytes);
    }
    let plain = Install::open(&stock).unwrap();
    assert_eq!(plain.active_mod(), None);
    assert!(plain.clone().discover(&Options::default()).is_err(), "no usable engine files at its root");
    assert_eq!(found(&plain.with_engine(&s.engine).unwrap(), &[]), names(&[("one", &["alpha"])]));
    // Armada I reads no engine files: the command line says so instead of ignoring the argument
    let a1 = s.tmp.path().join("a1");
    for rel in ["Armada.exe", "missions/x1S.dsl", "missions/x1R.drl"] {
        write(a1.join(rel), b"junk");
    }
    write(a1.join("bzn/fed1.bzn"), b"xx x1R.drl yy");
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut listing = |_: &Path| -> Result<Vec<Ins>> { panic!("the decoder was asked for a listing") };
    let dest = s.tmp.path().join("o");
    let args = ["--install", a1.to_str().unwrap(), "--engine", s.engine.to_str().unwrap(), "--out", dest.to_str().unwrap()];
    let code = cli::run(&args, &Env::default(), &mut Io { out: &mut out, err: &mut err }, &mut listing);
    assert!(code == 1 && String::from_utf8(err).unwrap().contains("--engine is for Armada II missions"));
}
