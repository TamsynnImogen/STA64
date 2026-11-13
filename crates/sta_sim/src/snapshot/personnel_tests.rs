use super::*;
use crate::WorldSource;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_content::{Races, Rules, StartingResources};

fn setup(mode: StartingResources, choice: u32) -> (GameSetup, Races) {
    let header = "int SINGLE_PLAYER_OFFICER_LIMIT=600; int MAX_OFFICERS=999; int MP_OFFICER_LIST_SIZE=4; MP_Officer_Limit_Class MP_OFFICER_LIMIT[4]={\"ol_normal\", SINGLE_PLAYER_OFFICER_LIMIT,\"ol_restricted\",SINGLE_PLAYER_OFFICER_LIMIT/2,\"ol_more\",SINGLE_PLAYER_OFFICER_LIMIT+200,\"ol_max\",MAX_OFFICERS};";
    let (rules, warnings) = Rules::from_header(header);
    assert!(warnings.is_empty(), "{warnings:?}");
    let db = Arc::new(sta_content::fake::db().with_rules(rules));
    let (races, warnings) = Races::load_with(&mut |name, _| {
        match name {
        "races.odf" => Some("race0=fx".into()),
        "fx.odf" => Some("name=fx\ninstantActionSlot=0\nstandardUnits1=fx_cruiser\nstandardUnits2=fx_cruiser\nnormalDilithium=3500\nnormalMetal=2000\nnormalLatinum=500\nnormalCrew=1200\nlotsDilithium=30000\nlotsMetal=30000\nlotsLatinum=30000\nlotsCrew=30000".into()),
        _ => None,
    }
    });
    assert!(warnings.is_empty(), "{warnings:?}");
    let options = GameOptions { starting_resources: mode, officer_limit: choice };
    (
        GameSetup {
            seed: 55,
            content: db,
            map: Arc::new(sta_content::fake::two_bases()),
            players: vec![PlayerSetup {
                id: PlayerId(0),
                team: TeamId(0),
                race: "fx".into(),
                control: Control::Human,
            }],
            options,
        },
        races,
    )
}

#[test]
fn normal_lots_and_officer_choices_reach_shared_view_and_snapshot() {
    for (mode, crew, stock) in [
        (StartingResources::Normal, 1200, [3500, 500, 2000, 0]),
        (StartingResources::Lots, 30000, [30000, 30000, 30000, 0]),
    ] {
        for (choice, limit) in [(0, 600), (1, 300), (2, 800), (3, 999)] {
            let (setup, races) = setup(mode, choice);
            let (mut sim, warnings) = Sim::from_setup_with_starts(&setup, &races);
            assert!(warnings.is_empty(), "{warnings:?}");
            let view = sim.view();
            let player = view.player(PlayerId(0)).unwrap();
            assert_eq!(player.crew, Some(Fixed::from_int(crew)));
            assert_eq!(player.resources, stock.map(Fixed::from_int));
            assert_eq!(player.officers, [Fixed::from_int(8), Fixed::from_int(limit)]);
            let bytes = sim.snapshot();
            let mut restored = Sim::from_snapshot(&bytes, Some(setup.content.clone())).unwrap();
            assert_eq!(restored.view(), sim.view());
            for _ in 0..10 {
                assert_eq!(restored.step(&[]), sim.step(&[]));
            }
        }
    }
}

#[test]
fn crew_zero_absence_and_mutation_have_distinct_saved_hashes() {
    let (setup, races) = setup(StartingResources::Normal, 0);
    let (mut sim, _) = Sim::from_setup_with_starts(&setup, &races);
    let initial = sim.digest();
    sim.economy
        .set_personnel(PlayerId(0), Some(Fixed::ZERO), Some(Fixed::from_int(600)));
    assert_ne!(sim.digest(), initial);
    let zero = sim.snapshot();
    let restored = Sim::from_snapshot(&zero, Some(setup.content.clone())).unwrap();
    assert_eq!(restored.view().player(PlayerId(0)).unwrap().crew, Some(Fixed::ZERO));
    sim.economy.set_personnel(PlayerId(0), None, Some(Fixed::from_int(600)));
    assert_ne!(sim.snapshot(), zero);
    assert_eq!(sim.view().player(PlayerId(0)).unwrap().crew, None);
}

fn before_personnel(bytes: &[u8]) -> usize {
    let (_, rest): (Provenance, _) = postcard::take_from_bytes(&bytes[HEADER..]).unwrap();
    let (_, rest): (Body, _) = postcard::take_from_bytes(rest).unwrap();
    let (_, rest): (Option<(Vec3, Vec3)>, _) = postcard::take_from_bytes(rest).unwrap();
    let (_, rest): (BoundaryRec, _) = postcard::take_from_bytes(rest).unwrap();
    let (_, rest): (PerceptionRec, _) = postcard::take_from_bytes(rest).unwrap();
    bytes.len() - rest.len()
}

#[test]
fn current_suffix_is_mandatory_and_rejects_noncanonical_rows() {
    let (setup, races) = setup(StartingResources::Normal, 0);
    let (sim, _) = Sim::from_setup_with_starts(&setup, &races);
    let bytes = sim.snapshot();
    let end = before_personnel(&bytes);
    assert!(Sim::from_snapshot(&bytes[..end], Some(setup.content.clone())).is_err());
    let valid = (PlayerId(0), Some(Fixed::ONE.to_bits()), Some(Fixed::ONE.to_bits()));
    for rows in [
        vec![valid, valid],
        vec![(PlayerId(9), valid.1, valid.2)],
        vec![(PlayerId(0), None, None)],
        vec![(PlayerId(0), Some(-1), None)],
    ] {
        let mut bad = bytes[..end].to_vec();
        bad.extend(postcard::to_allocvec(&PersonnelRec { version: 1, rows }).unwrap());
        bad.extend(postcard::to_allocvec(&UnitCrewRec { version: 1, rows: Vec::new() }).unwrap());
        bad.extend(postcard::to_allocvec(&StartEffectsRec { version: 1, rows: Vec::new() }).unwrap());
        bad.extend(postcard::to_allocvec(&crate::autonomy::AutonomyWire { version: 1, rows: Vec::new() }).unwrap());
        bad.extend(postcard::to_allocvec(&AttackRec { version: 1, warp: Vec::new() }).unwrap());
        assert!(Sim::from_snapshot(&bad, Some(setup.content.clone())).is_err());
    }
    let mut bad = bytes[..end].to_vec();
    bad.extend(
        postcard::to_allocvec(&PersonnelRec {
            version: 2,
            rows: Vec::new(),
        })
        .unwrap(),
    );
    assert!(Sim::from_snapshot(&bad, Some(setup.content.clone())).is_err());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(Sim::from_snapshot(&trailing, Some(setup.content)).is_err());
}

#[test]
fn the_v11_layout_is_refused_by_version() {
    let (setup, races) = setup(StartingResources::Normal, 0);
    let (sim, _) = Sim::from_setup_with_starts(&setup, &races);
    let bytes = sim.snapshot();
    let mut old = bytes[..before_personnel(&bytes)].to_vec();
    old[4..6].copy_from_slice(&11u16.to_le_bytes());
    old[6..10].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(Sim::from_snapshot(&old, Some(setup.content)).err(), Some(SnapshotError::FormatVersion { found: 11 }));
}

#[test]
fn replay_roundtrips_start_choices_and_rejects_previous_format() {
    let (setup, _) = setup(StartingResources::Lots, 2);
    let recorder = crate::replay::Recorder::new(&setup);
    let replay = recorder.finish();
    let mut bytes = replay.encode();
    let loaded = crate::replay::Replay::decode(&bytes).unwrap();
    assert_eq!(loaded.setup(setup.content.clone()).unwrap().options, setup.options);
    bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
    assert!(crate::replay::Replay::decode(&bytes).is_err());
}

#[test]
fn starting_rules_are_part_of_the_saved_identity() {
    // The pre-CON-35 migration path (a format-9 save loading under content
    // that only added starting rules) is gone with the old formats; the
    // rules now simply are part of the one content hash.
    let bytes = include_bytes!("../../tests/fixtures/native-control-v9.stas");
    let legacy = Arc::new(sta_content::fake::db());
    let rules = Rules::from_header("int SINGLE_PLAYER_OFFICER_LIMIT=600;").0;
    let current = Arc::new(sta_content::fake::db().with_rules(rules));
    assert_ne!(current.hash(), legacy.hash());
    let diff = legacy.digest().first_difference(current.digest()).unwrap();
    assert_eq!((diff.key.as_str(), diff.theirs.as_deref()), ("RTS_CFG.h:MP_OFFICER_LIMIT", Some("[600]")));
    assert_eq!(Sim::from_snapshot(bytes, Some(current.clone())).err(), Some(SnapshotError::FormatVersion { found: 9 }));
    let mut sim = Sim::new(9);
    sim.content = Some(legacy);
    assert!(matches!(
        Sim::from_snapshot(&sim.snapshot(), Some(current)),
        Err(SnapshotError::ContentMismatch { .. })
    ));
    let bytes = sim.snapshot();
    let changed = sta_content::fake::db();
    let mut builder = ContentDb::builder();
    for u in changed.units() {
        let mut u = u.clone();
        u.hull = u.hull + Fixed::ONE;
        builder = builder.unit(u);
    }
    for w in changed.weapons() {
        builder = builder.weapon(w.clone());
    }
    assert!(matches!(
        Sim::from_snapshot(&bytes, Some(Arc::new(builder.build().unwrap()))),
        Err(SnapshotError::ContentMismatch { .. })
    ));
}

#[test]
fn installed_personnel_enters_the_actual_shared_start_view() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        return;
    };
    let active = std::env::var("STA64_TEST_MOD").ok();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), active.as_deref()).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let (races, _) = Races::load(&vfs, &index);
    let race = races.playable().into_iter().next().unwrap();
    let mut map = sta_content::MapDef::empty();
    map.starts = sta_content::fake::two_bases().starts;
    for (mode, crew) in [
        (StartingResources::Normal, race.normal_crew),
        (StartingResources::Lots, race.lots_crew),
    ] {
        let options = GameOptions { starting_resources: mode, ..GameOptions::default() };
        let setup = GameSetup {
            seed: 71,
            content: Arc::new(db.clone()),
            map: Arc::new(map.clone()),
            players: vec![PlayerSetup {
                id: PlayerId(0),
                team: TeamId(0),
                race: race.name.clone(),
                control: Control::Human,
            }],
            options,
        };
        let (sim, warnings) = Sim::from_setup_with_starts(&setup, &races);
        let view = sim.view();
        let player = view.player(PlayerId(0)).unwrap();
        assert_eq!(
            player.crew,
            crew.map(|n| Fixed::from_int(n.clamp(0, 1_000_000_000) as i32))
        );
        assert_eq!(
            player.officers[1],
            Fixed::from_int(db.rules().officer_limit(0).unwrap())
        );
        eprintln!(
            "{} {mode:?}: crew={:?}, officers={:?}, resources={:?}, warnings={}",
            race.name,
            player.crew,
            player.officers,
            player.resources,
            warnings.len()
        );
        let restored = Sim::from_snapshot(&sim.snapshot(), Some(setup.content)).unwrap();
        assert_eq!(restored.view(), view);
    }
}
