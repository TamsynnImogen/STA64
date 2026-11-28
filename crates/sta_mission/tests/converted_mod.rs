//! A conversion made for an install with a mod is found only while that mod is active (#927,
//! ADR 0004 D6): the converter writes it into the folder [`sta_mission::converted::dir_in`] names
//! for that install and that mod, and the game mounts the folder of the mod that is active.
//!
//! On authored folders: the mission DLLs are junk and the engine files are the converter's
//! authored fixture, so a converted mission is an entry this test put in the converter's cache.

#[path = "support/convert.rs"]
mod support;

use std::path::{Path, PathBuf};

use sta_mission::converted;
use sta_mission_convert::cache::{Cache, Entry};
use sta_mission_convert::fixture::write_checked_engine;
use sta_mission_convert::install::{self, Env, Install, Options};
use sta_mission_convert::output::OutDir;
use sta_vfs::Vfs;
use support::TempDir;

fn write(p: PathBuf, bytes: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

/// Converts the missions of `name` inside `root` into the folder the game mounts for that pair.
/// Returns that folder.
fn convert(root: &Path, engine: &Path, data: &Path, cache: &Cache, name: &str) -> PathBuf {
    let inst = Install::open_mod(root, name).expect("the mount takes the mod").with_engine(engine).expect("the engine folder");
    let out = converted::dir_in(data, root, inst.active_mod());
    let resolved = install::resolve_output(&out, &inst, &Env::default()).expect("the output folder is allowed");
    let mut dir = OutDir::inspect(&out, resolved, false).expect("a fresh output folder is usable");
    let mut decoder = |_: &Path, _: &sta_mission_convert::Pe| panic!("a cached mission is not decoded");
    let summary = install::convert_install(&inst, &Options::default(), &mut dir, Some(cache), &mut decoder, &|| false, &mut |_| {});
    assert!(summary.aborted.is_none() && summary.finish_error.is_none());
    assert!(summary.missions.iter().all(|m| m.converted), "every mission came from the cache");
    out
}

#[test]
fn a_mods_conversion_is_mounted_only_while_that_mod_is_active() {
    let tmp = TempDir::new("converted-mod");
    let (root, engine, data) = (tmp.path().join("Hosting Install"), tmp.path().join("engine"), tmp.path().join("data"));
    // an install that hosts mods: `Base` holds a mission DLL pair and the map that runs it;
    // `Child` has `Base` as its parent and adds a second map running the same mission
    for (rel, bytes) in [
        ("Data/Mods/Base/info.ini", &b"[Mod]\n"[..]),
        ("Data/Mods/Base/missions/oneS.dsl", b"junk mission one"),
        ("Data/Mods/Base/missions/oneR.drl", b"junk rules one"),
        ("Data/Mods/Base/bzn/alpha.bzn", b"xx oneS.dsl yy"),
        ("Data/Mods/Child/info.ini", b"[Mod]\nParentMod=Base\n"),
        ("Data/Mods/Child/bzn/gamma.bzn", b"xx oneS.dsl yy"),
        ("Data/Mods/Other/info.ini", b"[Mod]\n"),
    ] {
        write(root.join(rel), bytes);
    }
    write_checked_engine(&engine).unwrap();
    let cache = Cache::new(tmp.path().join("cache"));
    let missions = root.join("Data/Mods/Base/missions");
    let key = install::a2_cache_key(&missions.join("oneS.dsl"), Some(&missions.join("oneR.drl")), &engine).unwrap();
    cache.put(&key, &Entry::Converted { source: b"-- invented script\nreturn 1\n".to_vec(), console: Vec::new() }).unwrap();

    let out = convert(&root, &engine, &data, &cache, "Child");
    assert_eq!(out, converted::dir_in(&data, &root, Some("Child")));

    // what the game does at start: mount the install with its active mod, then the converted
    // folder of that install and that mod below it
    let script_with = |active: Option<&str>| {
        let mut vfs = Vfs::mount_install(&root, active).expect("the install mounts");
        let mounted = converted::mount(&mut vfs, &converted::dir_in(&data, &root, active));
        (mounted, vfs.read("missions/gamma.lua").map(|r| r.unwrap()))
    };
    assert_eq!(script_with(Some("Child")), (true, Some(b"-- invented script\nreturn 1\n".to_vec())));
    for other in [None, Some("Base"), Some("Other")] {
        assert_eq!(script_with(other), (false, None), "active mod {other:?}: the child's conversion is not its own");
    }

    // the parent's own conversion is a second folder, with the names the parent's maps give
    let base_out = convert(&root, &engine, &data, &cache, "Base");
    assert_ne!(base_out, out);
    let mut vfs = Vfs::mount_install(&root, Some("Base")).unwrap();
    assert!(converted::mount(&mut vfs, &converted::dir_in(&data, &root, Some("Base"))));
    assert!(vfs.read("missions/alpha.lua").is_some() && vfs.read("missions/gamma.lua").is_none());
}
