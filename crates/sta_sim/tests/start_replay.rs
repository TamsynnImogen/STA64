//! SIM-27: a match started through `Sim::from_setup_with_starts` is recorded
//! and replayed to equal digests, loadouts coming from the race ODFs.
//!
//! Install-gated like the CON tests: `STA64_TEST_INSTALL` plus optional
//! `STA64_TEST_MOD` (skipped when unset). The map is `fake::two_bases`' start
//! positions; the content and races are the install's.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, OdfIndex, Races};
use sta_core::{PlayerId, TeamId};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::replay::{Recorder, Replay, ReplayError, StartRec};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};

#[test]
fn installed_race_starts_record_and_replay_to_equal_digests() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        return;
    };
    let active = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(Path::new(&root), active.as_deref()).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let (races, _) = Races::load(&vfs, &index);
    let playable = races.playable();
    assert!(playable.len() >= 2, "an install with two playable races");
    // The install's first map with two start positions (e.g. Fleet Ops 4.0's
    // own Instant Action maps), else the synthetic `two_bases` starts.
    let mut map = vfs
        .list("bzn")
        .into_iter()
        .filter(|p| p.to_ascii_lowercase().ends_with(".bzn"))
        .filter_map(|p| sta_bzn::parse(&vfs.read(&p)?.ok()?).ok())
        .filter(|b| !b.objects.is_empty())
        .map(|b| MapDef::convert_bzn_in(&b, &db, &vfs, &index).map)
        .find(|m| m.start_for(0).is_some() && m.start_for(1).is_some())
        .unwrap_or_else(|| {
            let mut m = MapDef::empty();
            m.starts = sta_content::fake::two_bases().starts;
            m
        });
    map.name = "installed map".into();
    eprintln!("map: {} objects, {} starts", map.objects.len(), map.starts.len());
    let db = Arc::new(db);
    let setup = GameSetup {
        seed: 91,
        map: Arc::new(map),
        content: db.clone(),
        players: (0..2)
            .map(|i| PlayerSetup {
                id: PlayerId(i as u8),
                team: TeamId(i as u8),
                race: playable[i].name.clone(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    };
    let mut recorder = Recorder::with_starts(&setup, &races, 15);
    let at_start = recorder.sim().view().units.len();
    assert!(at_start > 0, "the races' start forces were placed");
    for t in 0..90u32 {
        // Every 30 ticks seat 0 orders its craft to move, so the log has commands.
        let mine: Vec<_> = recorder.sim().view().units.iter()
            .filter(|u| u.owner == PlayerId(0) && u.flags.contains(sta_sim::view::UnitFlags::SELECTABLE) && u.speed >= sta_core::Fixed::ZERO)
            .map(|u| (u.id, u.pos)).collect();
        let cmds: Vec<Command> = if t % 30 == 0 && !mine.is_empty() {
            let to = mine[0].1;
            vec![Command { player: PlayerId(0), seq: t, kind: CommandKind::Order(Order::Move {
                units: mine.iter().map(|m| m.0).collect(),
                to: sta_core::Vec3 { x: to.x + sta_core::Fixed::from_int(400), y: to.y, z: to.z },
                queue: false,
            }) }]
        } else {
            vec![]
        };
        recorder.step(&cmds);
    }
    let digest = recorder.sim().digest();
    let replay = Replay::decode(&recorder.finish().encode()).unwrap();
    assert!(matches!(replay.starts, StartRec::Races { .. }));
    assert!(!replay.commands.is_empty());
    assert_ne!(replay.digests[0].1, Sim::from_setup(&setup).digest());
    assert_eq!(replay.play(db.clone()).unwrap_err(), ReplayError::StartsNeedRaces);
    let sim = replay.play_with_starts(db, &races).unwrap();
    assert_eq!(sim.digest(), digest);
    assert!(sim.view().units.len() >= at_start);
}
