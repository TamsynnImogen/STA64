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
    let mut map = MapDef::empty();
    map.starts = sta_content::fake::two_bases().starts;
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
    for _ in 0..90 {
        recorder.step(&[]);
    }
    let digest = recorder.sim().digest();
    let replay = Replay::decode(&recorder.finish().encode()).unwrap();
    assert!(matches!(replay.starts, StartRec::Races { .. }));
    assert_ne!(replay.digests[0].1, Sim::from_setup(&setup).digest());
    assert_eq!(replay.play(db.clone()).unwrap_err(), ReplayError::StartsNeedRaces);
    let sim = replay.play_with_starts(db, &races).unwrap();
    assert_eq!(sim.digest(), digest);
    assert!(sim.view().units.len() >= at_start);
}
