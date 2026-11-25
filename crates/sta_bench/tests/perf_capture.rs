//! `sta-bench perf --capture` (#1238): a capture the game wrote is read into the perf
//! bench's scene (map, mount, camera, window size, snapshot) and loads with the content
//! hash check on. Every capture here is authored by the test; the install-gated test
//! reads the install at run time.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_bench::capture::{self, BuildInfo, Capture, CaptureError, CaptureIndex, CameraPose, MountInfo, StartMode};
use sta_bench::parse::SceneFile;
use sta_bench::run::{Bench, RunConfig};
use sta_content::{ContentDb, MapDef};
use sta_core::{PlayerId, TeamId};
use sta_sim::replay::Replay;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("perf-capture-tests").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A capture of `map` on `content`, `ticks` steps in, with a snapshot and the camera and
/// window a player would have had.
fn capture_of(content: &Arc<ContentDb>, map: MapDef, ticks: u32, mount: MountInfo) -> Capture {
    let map_name = map.name.clone();
    let setup = GameSetup {
        seed: 3,
        map: Arc::new(map),
        content: content.clone(),
        players: vec![PlayerSetup { id: PlayerId(0), team: TeamId(0), race: "fx".into(), control: Control::Human }],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    for _ in 0..ticks {
        sim.step(&[]);
    }
    let replay = Replay::new(&setup);
    let view = sim.view();
    let mut index = CaptureIndex::new(&replay, content, view.tick, view.digest, StartMode::Plain, 1_794_000_000, BuildInfo::default());
    assert_eq!(index.map, map_name);
    index.mount = mount;
    index.units_total = view.units.len();
    index.has_snapshot = true;
    index.window = Some([1376.0, 576.4]);
    index.camera = Some(CameraPose {
        translation: [10.0, 100.0, -300.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        focus: Some([10.0, 0.0, -500.0]),
        yaw: Some(0.0),
        pitch: Some(0.5),
        distance: Some(200.0),
    });
    Capture { index, replay, snapshot: Some(sim.snapshot()), note: String::new() }
}

fn fake_capture(name: &str) -> (PathBuf, Capture, Arc<RunConfig>) {
    let cfg = Arc::new(RunConfig::fake());
    let cap = capture_of(&cfg.content, sta_content::fake::two_bases(), 40, MountInfo::default());
    let dir = cap.write(&scratch(name)).unwrap();
    (dir, cap, cfg)
}

fn numbers(line: &str) -> Vec<f32> {
    line.split_whitespace().skip(1).map(|w| w.parse().unwrap()).collect()
}

#[test]
fn the_camera_is_the_players_in_the_scenes_space() {
    // The renderer mirrors Z: eye and focus come out with their Z negated, as the scene's `camera` wants them.
    let pose = CameraPose { translation: [10.0, 100.0, -300.0], rotation: [0.0, 0.0, 0.0, 1.0], focus: Some([10.0, 0.0, -500.0]), yaw: None, pitch: None, distance: None };
    assert_eq!(capture::camera_command(&pose), "camera 10 100 300 10 0 500");
    // Without a focus the line of sight to the ground plane stands in: straight down from 100 high.
    let q = std::f32::consts::FRAC_1_SQRT_2;
    let down = CameraPose { rotation: [-q, 0.0, 0.0, q], focus: None, ..pose };
    let n = numbers(&capture::camera_command(&down));
    for (got, want) in n.iter().zip([10.0, 100.0, 300.0, 10.0, 0.0, 300.0]) {
        assert!((got - want).abs() < 1e-3, "{n:?}");
    }
}

#[test]
fn a_capture_becomes_a_scene_with_its_map_snapshot_camera_and_size() {
    let (dir, cap, _) = fake_capture("scene");
    let index = CaptureIndex::load(&dir).unwrap();
    assert_eq!(index, cap.index, "the index alone reads back whole");
    let launch = capture::perf_launch(&dir, &index, Some(&dir), None).unwrap();
    assert_eq!(launch.size, Some((1376, 576)));
    assert_eq!(launch.snapshot, std::path::absolute(dir.join(capture::SNAPSHOT_FILE)).unwrap());
    let scene = SceneFile::parse(&launch.scene).unwrap_or_else(|e| panic!("{e}\n{}", launch.scene));
    assert_eq!(scene.scene.map.as_deref(), Some("fx_two_bases"));
    assert!(matches!(&scene.scene.start, Some(sta_bench::parse::scene::Start::Snapshot(f)) if Path::new(f) == launch.snapshot), "{:?}", scene.scene.start);
    assert!(scene.scene.camera.is_some(), "the capture's camera is in the scene");
    assert!(launch.scene.contains("Fog=1") && launch.scene.contains("Hud=0"));
}

#[test]
fn a_capture_without_a_snapshot_or_an_install_is_refused_with_the_reason() {
    let (dir, cap, _) = fake_capture("refused");
    // No recorded install and none given.
    let e = capture::perf_launch(&dir, &cap.index, None, None).unwrap_err();
    assert!(e.to_string().contains("--install"), "{e}");
    // A recorded install that is not here.
    let mut gone = cap.index.clone();
    gone.mount.install_root = Some(dir.join("no-such-install").display().to_string());
    assert!(capture::perf_launch(&dir, &gone, None, None).unwrap_err().to_string().contains("not on this machine"));
    // No snapshot.
    let mut bare = cap.index.clone();
    bare.has_snapshot = false;
    assert!(capture::perf_launch(&dir, &bare, Some(&dir), None).unwrap_err().to_string().contains("no snapshot"));
}

#[test]
fn the_bench_loads_the_scene_of_a_capture_with_the_hash_check_on_and_refuses_other_content() {
    let (dir, cap, cfg) = fake_capture("bench");
    let launch = capture::perf_launch(&dir, &cap.index, Some(&dir), None).unwrap();
    let scene = SceneFile::parse(&launch.scene).unwrap();

    let bench = Bench::build(cfg.clone(), &scene).expect("the capture's own content loads its snapshot");
    assert_eq!(bench.view().units.len(), cap.index.units_total);
    assert_eq!(bench.view().tick, cap.index.tick);

    // Other content: the hash check fails the build, with a message that says so and that it is not skipped.
    let other = {
        let rules = sta_content::Rules::from_header("int MP_OFFICER_LIST_SIZE = 2;\nOfficer MP_OFFICER_LIMIT[2] = { \"ol_normal\", 600, \"ol_small\", 300 };\n").0;
        Arc::new(RunConfig { content: Arc::new((*cfg.content).clone().with_rules(rules)), ..RunConfig::fake() })
    };
    assert_ne!(other.content.hash(), cfg.content.hash());
    let e = Bench::build(other, &scene).err().expect("other content is refused");
    assert!(e.contains("content hash mismatch") && e.contains("not skipped"), "{e}");
}

/// A synthetic install (authored files only): a default tree, a smaller one, a map that
/// names the smaller one, and a mod folder to mount.
fn install_with_map_tree(name: &str) -> PathBuf {
    let root = scratch(name);
    let w = |rel: &str, bytes: &[u8]| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    };
    w("Data/techtree/tech1.tt", b"yard.odf 0\nship.odf 1 yard.odf\nbig.odf 1 yard.odf\n");
    w("Data/techtree/small.tt", b"#include tech1.tt\nbig.odf -1\n");
    w("Data/Mods/M/info.ini", b"[mod]\n");
    let map = sta_bzn::Map { version: 3013, binary: false, tech_tree_file: "small.tt".into(), ..Default::default() };
    w("Data/bzn/treemap.bzn", &sta_bzn::write::write(&map).unwrap());
    root
}

#[test]
fn the_bench_content_of_a_map_is_the_content_the_game_plays_it_with() {
    let root = install_with_map_tree("tree_install");
    let (game, _) = capture::load_install_for_map(&root, Some("M"), "treemap").unwrap();
    let (default, _) = capture::load_install(&root, Some("M")).unwrap();
    assert_ne!(game.hash(), default.hash(), "the map's tree is part of the content hash");

    // `install_config` (the bench's own set-up) applies the tree as the game does...
    let cfg = sta_bench::pair::install_config(&root, Some("M"), &["treemap".to_owned()]).unwrap();
    assert_eq!(cfg.content.hash(), game.hash());
    // ...and with no map (or several) the default tree stays.
    assert_eq!(sta_bench::pair::install_config(&root, Some("M"), &[]).unwrap().content.hash(), default.hash());
}

#[test]
fn the_snapshot_check_names_what_differs_and_skips_nothing() {
    let root = install_with_map_tree("check_install");
    let (game, _) = capture::load_install_for_map(&root, Some("M"), "treemap").unwrap();
    let mut map = MapDef::empty();
    map.name = "treemap".into();
    let cap = capture_of(&game, map, 25, MountInfo::default());
    let bytes = cap.snapshot.clone().unwrap();

    let ok = capture::check_snapshot(&cap.index, &root, Some("M"), &bytes).unwrap();
    assert_eq!((ok.tick, ok.digest), (25, capture::parse_hex(&cap.index.digest).unwrap()));

    // A capture of a map whose tree the bench would not apply (a different map name) is other content.
    let mut other_map = cap.index.clone();
    other_map.map = "no-such-map".into();
    let e = capture::check_snapshot(&other_map, &root, Some("M"), &bytes).unwrap_err();
    assert!(matches!(e, CaptureError::Content(_)) && e.to_string().contains("content hash mismatch") && e.to_string().contains("not skipped"), "{e}");

    // The profile the capture was made with is checked first.
    let mut profile = cap.index.clone();
    profile.profile = "Fo4".into();
    let e = capture::check_snapshot(&profile, &root, Some("M"), &bytes).unwrap_err();
    assert!(e.to_string().contains("profile"), "{e}");

    // A snapshot whose sim is not the one the index describes is refused too.
    let mut digest = cap.index.clone();
    digest.digest = capture::hex(1);
    assert!(matches!(capture::check_snapshot(&digest, &root, Some("M"), &bytes), Err(CaptureError::SnapshotDigest { .. })));
}

/// A capture of an installed map, made the way the bench makes its sim, goes through the
/// whole path: the bench's `install_config` content, the scene a capture makes, the hash check.
/// Needs `STA64_TEST_INSTALL` (and `STA64_TEST_MOD`); skips without it.
#[test]
fn an_installed_capture_loads_into_the_bench_with_matching_hash() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let modname = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(Path::new(&root), modname.as_deref()).unwrap();
    let mut paths = vfs.list("bzn");
    paths.sort();
    let Some(stem) = paths.iter().find_map(|p| p.strip_suffix(".bzn").or_else(|| p.strip_suffix(".BZN")).map(|s| s.rsplit('/').next().unwrap_or(s).to_ascii_lowercase())) else {
        eprintln!("SKIP: no map in the install");
        return;
    };
    let cfg = sta_bench::pair::install_config(Path::new(&root), modname.as_deref(), std::slice::from_ref(&stem)).unwrap();
    let map = cfg.maps.get(&stem).cloned().expect("the map converts");
    let mount = MountInfo::from_layers(vfs.layers().iter().map(|l| (l.name(), l.root.as_path())));
    let cap = capture_of(&cfg.content, map, 30, mount);
    let dir = cap.write(&scratch("installed")).unwrap();

    let index = CaptureIndex::load(&dir).unwrap();
    let launch = capture::perf_launch(&dir, &index, None, None).unwrap();
    let found = capture::check_snapshot(&index, &launch.install_root, launch.active_mod.as_deref(), &std::fs::read(&launch.snapshot).unwrap()).unwrap();
    assert_eq!(found.units, cap.index.units_total);

    // What `sta64 --perf` does with the scene: the install's content for the scene's map, then the snapshot.
    let scene = SceneFile::parse(&launch.scene).unwrap();
    let maps = [index.map.clone()];
    let cfg = sta_bench::pair::install_config(&launch.install_root, launch.active_mod.as_deref(), &maps).unwrap();
    let bench = Bench::build(Arc::new(cfg), &scene).expect("the bench's content hashes as the capture's");
    assert_eq!(bench.view().units.len(), found.units);
}
