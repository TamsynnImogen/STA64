//! Playtest captures (TOOL-14): a capture made from a headless skirmish is
//! written, read back and replays to the same digest at the capture tick.
//! The install-gated variant does the same on real content (`STA64_TEST_INSTALL`,
//! `STA64_TEST_MOD`), mounting the install again from the capture's own record.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_bench::capture::{self, BuildInfo, Capture, CaptureError, CaptureIndex, MountInfo, StartMode, UnitInfo};
use sta_content::{ContentDb, Races};
use sta_core::{PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::replay::{DEFAULT_DIGEST_EVERY, Replay, ReplayError};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("capture-tests").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn setup(content: Arc<ContentDb>, map: sta_content::MapDef, races: [&str; 2]) -> GameSetup {
    let seat = |i: u8, race: &str, control| PlayerSetup { id: PlayerId(i), team: TeamId(i), race: race.into(), control };
    GameSetup {
        seed: 11,
        map: Arc::new(map),
        content,
        players: vec![seat(0, races[0], Control::Human), seat(1, races[1], Control::Idle)],
        options: GameOptions::default(),
    }
}

/// What the game's sim link does, without Bevy: step a sim, hand it commands
/// and log every step into a [`Replay`] header made from the setup.
struct Played {
    sim: Sim,
    replay: Replay,
}

impl Played {
    fn start(setup: &GameSetup, races: Option<&Races>) -> Played {
        let sim = match races {
            Some(r) => Sim::from_setup_with_starts(setup, r).0,
            None => Sim::from_setup(setup),
        };
        let mut replay = Replay::new(setup);
        replay.digests.push((0, sim.digest()));
        Played { sim, replay }
    }

    fn step(&mut self, commands: &[Command]) {
        let before = self.sim.view().tick;
        let out = self.sim.step(commands);
        self.replay.log_step(before, commands, out.view.tick, out.view.digest, DEFAULT_DIGEST_EVERY);
    }

    fn capture(&self, content: &ContentDb, start: StartMode, mount: MountInfo) -> Capture {
        let view = self.sim.view();
        let mut index = CaptureIndex::new(&self.replay, content, view.tick, view.digest, start, 1_794_000_000, BuildInfo::default());
        index.mount = mount;
        index.units_total = view.units.len();
        index.has_snapshot = true;
        // The first unit of seat 0 stands in for the player's selection.
        index.selection = view
            .units
            .iter()
            .filter(|u| u.owner == PlayerId(0))
            .take(1)
            .map(|u| UnitInfo {
                id: u.id.0,
                def: content.name(u.def).map(str::to_owned),
                owner: u.owner.0,
                race: None,
                pos: u.pos.to_f32(),
                heading: [u.heading.0.to_f32(), u.heading.1.to_f32()],
                hull: [u.hull[0].to_f32(), u.hull[1].to_f32()],
                flags: u.flags.0,
                flag_names: Vec::new(),
                progress: None,
            })
            .collect();
        Capture { index, replay: self.replay.clone(), snapshot: Some(self.sim.snapshot()), note: "the beam outlived its shooter".into() }
    }
}

fn move_order(sim: &Sim, player: u8, seq: u32) -> Command {
    let units: Vec<_> = sim.view().units.iter().filter(|u| u.owner == PlayerId(player)).map(|u| u.id).collect();
    assert!(!units.is_empty(), "seat {player} starts with units");
    let to = Vec3::from_f32([0.0, 0.0, 1500.0]);
    Command { player: PlayerId(player), seq, kind: CommandKind::Order(Order::Move { units, to, queue: false }) }
}

#[test]
fn a_skirmish_capture_reloads_and_replays_to_the_same_digest() {
    let content = Arc::new(sta_content::fake::db());
    let races = Races::default();
    let s = setup(content.clone(), sta_content::fake::two_bases(), ["fx", "fx"]);
    let mut game = Played::start(&s, Some(&races));
    for tick in 0..200u32 {
        // Orders at ticks 3 and 90, so the log is not empty and not only at the start.
        let cmds = if tick == 3 || tick == 90 { vec![move_order(&game.sim, 0, tick)] } else { Vec::new() };
        game.step(&cmds);
    }
    let cap = game.capture(&content, StartMode::Starts, MountInfo::default());
    assert_eq!(cap.index.tick, 200);
    assert_eq!(cap.replay.commands.len(), 2);

    let dir = cap.write(&scratch("skirmish")).unwrap();
    for f in [capture::INDEX_FILE, capture::REPORT_FILE, capture::REPLAY_FILE, capture::SNAPSHOT_FILE, capture::NOTE_FILE] {
        assert!(dir.join(f).is_file(), "{f} written");
    }
    let md = std::fs::read_to_string(dir.join(capture::REPORT_FILE)).unwrap();
    assert!(md.contains("tick 200") && md.contains("fx_two_bases") && md.contains("fx_yard"), "{md}");

    // A fresh read of the folder, as a fix agent would do it.
    let loaded = Capture::load(&dir).unwrap();
    assert_eq!(loaded.index, cap.index);
    assert_eq!(loaded.note, "the beam outlived its shooter");
    assert_eq!(loaded.digest().unwrap(), game.sim.digest());

    let verified = loaded.verify(content.clone(), Some(&races)).unwrap();
    assert_eq!((verified.tick, verified.digest, verified.snapshot_checked), (200, game.sim.digest(), true));

    // The replayed sim and the snapshot's are the same sim, and go on alike.
    let mut replayed = loaded.replay_to_tick(content.clone(), Some(&races)).unwrap();
    let mut restored = loaded.sim_from_snapshot(content).unwrap().unwrap();
    for _ in 0..30 {
        replayed.step(&[]);
        restored.step(&[]);
        game.sim.step(&[]);
    }
    assert_eq!((replayed.digest(), restored.digest()), (game.sim.digest(), game.sim.digest()));
}

#[test]
fn a_plain_capture_needs_no_races_and_a_skirmish_one_does() {
    let content = Arc::new(sta_content::fake::db());
    let s = setup(content.clone(), sta_content::fake::two_bases(), ["fx", "fx"]);
    let mut game = Played::start(&s, None);
    game.step(&[]);
    let plain = game.capture(&content, StartMode::Plain, MountInfo::default());
    plain.verify(content.clone(), None).unwrap();
    // The same log, claimed to be a skirmish: refused for want of races, and with
    // them, a desync at tick 0 (the start state differs), named as such.
    let mut wrong = plain;
    wrong.index.start = StartMode::Starts;
    assert!(matches!(wrong.verify(content.clone(), None), Err(CaptureError::NeedsRaces)));
    match wrong.verify(content, Some(&Races::default())) {
        Err(CaptureError::Replay(ReplayError::Desync { tick: 0, .. })) => {}
        other => panic!("expected a desync at tick 0, got {other:?}"),
    }
}

#[test]
fn a_capture_that_no_longer_reproduces_names_the_first_bad_tick() {
    let content = Arc::new(sta_content::fake::db());
    let races = Races::default();
    let s = setup(content.clone(), sta_content::fake::two_bases(), ["fx", "fx"]);
    let mut game = Played::start(&s, Some(&races));
    for tick in 0..100u32 {
        let cmds = if tick == 5 { vec![move_order(&game.sim, 0, 0)] } else { Vec::new() };
        game.step(&cmds);
    }
    let mut cap = game.capture(&content, StartMode::Starts, MountInfo::default());
    // Drop the order: the replay now plays another game, and says where it parted.
    cap.replay.commands.clear();
    match cap.verify(content, Some(&races)) {
        Err(CaptureError::Replay(ReplayError::Desync { tick, .. })) => assert!(tick > 5 && tick <= 30, "first digest after the order: {tick}"),
        other => panic!("expected a desync, got {other:?}"),
    }
}

#[test]
fn captures_in_the_same_second_get_their_own_folders_and_list_newest_first() {
    let content = Arc::new(sta_content::fake::db());
    let s = setup(content.clone(), sta_content::fake::two_bases(), ["fx", "fx"]);
    let mut game = Played::start(&s, None);
    game.step(&[]);
    let root = scratch("listing");
    let cap = game.capture(&content, StartMode::Plain, MountInfo::default());
    let a = cap.write(&root).unwrap();
    let b = cap.write(&root).unwrap();
    assert_ne!(a, b);
    let mut later = cap.clone();
    later.index.created_unix += 60;
    let c = later.write(&root).unwrap();
    assert_eq!(capture::list(&root), [c, b, a]);
    assert_eq!(capture::stamp(1_794_000_000), "20261106-212000");
}

#[test]
fn the_mount_is_read_back_from_the_layers() {
    // Paths are stored absolute, so a capture can be checked from any directory: on
    // Windows `/games/Armada2` gains a drive letter and backslashes.
    let abs = |p: &str| std::path::absolute(p).unwrap().display().to_string();
    let root = abs("/games/Armada2");
    let data = Path::new("/games/Armada2/Data");
    let modroot = Path::new("/games/Armada2/Data/Mods/FO");
    let m = MountInfo::from_layers([("FO", modroot), ("Data", data)]);
    assert_eq!((m.install_root.as_deref(), m.active_mod.as_deref()), (Some(root.as_str()), Some("FO")));
    assert_eq!(m.layers.len(), 2);
    assert_eq!(m.layers[0].root, abs("/games/Armada2/Data/Mods/FO"));
    let stock = MountInfo::from_layers([("Data", data)]);
    assert_eq!((stock.install_root.as_deref(), stock.active_mod.as_deref()), (Some(root.as_str()), None));
    let standalone = MountInfo::from_layers([("Roots", Path::new("/games/Armada2/Data/Mods/Roots"))]);
    assert_eq!((standalone.install_root.as_deref(), standalone.active_mod.as_deref()), (Some(root.as_str()), Some("Roots")));
}

/// A capture of a real skirmish on the installed game: two playable races on
/// the first map with two start positions, 300 ticks, an order at tick 10.
/// The capture is checked against content mounted again from its own record.
#[test]
fn an_installed_skirmish_capture_replays_to_the_same_digest() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let modname = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(Path::new(&root), modname.as_deref()).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let (races, _) = Races::load(&vfs, &index);
    let db = Arc::new(db);
    let playable: Vec<String> = races.playable().iter().map(|r| r.name.clone()).collect();
    if playable.len() < 2 {
        eprintln!("SKIP: fewer than two playable races");
        return;
    }
    let mut paths = vfs.list("bzn");
    paths.sort();
    let map = paths
        .iter()
        .filter(|p| p.to_ascii_lowercase().ends_with(".bzn"))
        .filter_map(|p| vfs.read(p)?.ok())
        .filter_map(|bytes| sta_bzn::parse(&bytes).ok())
        .map(|bzn| sta_content::MapDef::convert_bzn_in(&bzn, &db, &vfs, &index).map)
        .find(|m| m.start_for(0).is_some() && m.start_for(1).is_some());
    let Some(map) = map else {
        eprintln!("SKIP: no map with two start positions");
        return;
    };
    let s = setup(db.clone(), map, [playable[0].as_str(), playable[1].as_str()]);
    let mut game = Played::start(&s, Some(&races));
    for tick in 0..300u32 {
        let cmds = if tick == 10 && game.sim.view().units.iter().any(|u| u.owner == PlayerId(0)) { vec![move_order(&game.sim, 0, 0)] } else { Vec::new() };
        game.step(&cmds);
    }
    let mount = MountInfo::from_layers(vfs.layers().iter().map(|l| (l.name(), l.root.as_path())));
    assert_eq!(mount.active_mod, modname);
    let dir = game.capture(&db, StartMode::Starts, mount).write(&scratch("installed")).unwrap();

    let loaded = Capture::load(&dir).unwrap();
    // Content and races mounted from the capture's record, not from this test's state.
    let (db2, races2) = capture::load_install(Path::new(loaded.index.mount.install_root.as_deref().unwrap()), loaded.index.mount.active_mod.as_deref()).unwrap();
    let verified = loaded.verify(db2, Some(&races2)).unwrap();
    assert_eq!((verified.tick, verified.digest), (300, game.sim.digest()));
}

#[test]
fn the_check_command_passes_a_capture_and_fails_a_tampered_one() {
    let content = Arc::new(sta_bench::run::fake_content());
    let races = Races::default();
    let s = setup(content.clone(), sta_content::fake::two_bases(), ["fx", "fx"]);
    let mut game = Played::start(&s, Some(&races));
    for tick in 0..60u32 {
        let cmds = if tick == 5 { vec![move_order(&game.sim, 0, 0)] } else { Vec::new() };
        game.step(&cmds);
    }
    let mut cap = game.capture(&content, StartMode::Starts, MountInfo::default());
    let root = scratch("cli");
    let good = cap.write(&root).unwrap();
    let args = |dir: &Path| vec!["capture".to_owned(), "check".into(), dir.display().to_string(), "--fake".into()];
    assert_eq!(sta_bench::cli::main(args(&good)), 0);
    cap.index.created_unix += 1;
    cap.replay.commands.clear();
    let bad = cap.write(&root).unwrap();
    assert_eq!(sta_bench::cli::main(args(&bad)), 1);
    // Without --fake and without a recorded install, the command says what is missing.
    assert_eq!(sta_bench::cli::main(vec!["capture".into(), "check".into(), good.display().to_string()]), 1);
}

#[test]
fn the_index_is_strict_json_and_the_older_json5_form_still_loads() {
    let content = Arc::new(sta_content::fake::db());
    let s = setup(content.clone(), sta_content::fake::two_bases(), ["fx", "fx"]);
    let mut game = Played::start(&s, Some(&Races::default()));
    for _ in 0..5 {
        game.step(&[]);
    }
    let cap = game.capture(&content, StartMode::Starts, MountInfo::default());
    let dir = cap.write(&scratch("strict_json")).unwrap();
    let text = std::fs::read_to_string(dir.join(capture::INDEX_FILE)).unwrap();
    // `jq` and every JSON parser read it (the writer used to emit JSON5: bare keys).
    let v: serde_json::Value = serde_json::from_str(&text).expect("capture.json is strict JSON");
    assert_eq!(v["tick"], 5);
    // Captures taken before the fix have bare keys; they still load.
    let old = json5::to_string(&Capture::load(&dir).unwrap().index).unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&old).is_err(), "the old form is not JSON");
    std::fs::write(dir.join(capture::INDEX_FILE), old).unwrap();
    assert_eq!(Capture::load(&dir).unwrap().index, cap.index);
}
