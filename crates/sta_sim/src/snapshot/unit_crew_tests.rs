use super::*;
use crate::WorldSource;

fn live() -> (Sim, EntityId, EntityId) {
    let mut sim = Sim::new(61);
    sim.players = vec![(PlayerId(0), TeamId(0))];
    let first = sim.spawn(0, Vec3::ZERO, (Fixed::ZERO, Fixed::ONE), MoveStats::STATIC);
    let second = sim.spawn(0, Vec3::ZERO, (Fixed::ZERO, Fixed::ONE), MoveStats::STATIC);
    sim.units.get_mut(first).unwrap().crew = Some([Fixed::from_int(73), Fixed::from_int(150)]);
    sim.units.get_mut(second).unwrap().crew = Some([Fixed::ZERO; 2]);
    sim.economy.set_personnel(PlayerId(0), Some(Fixed::from_int(1200)), Some(Fixed::from_int(600)));
    (sim, first, second)
}

fn replace_crew(bytes: &[u8], version: u8, rows: Vec<(EntityId, [i64; 2])>) -> Vec<u8> {
    chain::with(bytes, chain::CREW, chain::enc(&UnitCrewRec { version, rows }))
}

#[test]
fn current_crew_roundtrips_known_zero_and_live_values_without_capacity_refill() {
    let (mut sim, first, second) = live();
    let bytes = sim.snapshot();
    assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), FORMAT_VERSION);
    let mut restored = Sim::from_snapshot(&bytes, None).unwrap();
    assert_eq!(restored.snapshot(), bytes);
    assert_eq!(restored.view(), sim.view());
    assert_eq!(restored.view().unit(first).unwrap().crew, Some([Fixed::from_int(73), Fixed::from_int(150)]));
    assert_eq!(restored.view().unit(second).unwrap().crew, Some([Fixed::ZERO; 2]));
    for _ in 0..10 {
        assert_eq!(sim.step(&[]), restored.step(&[]));
    }
}

#[test]
fn mandatory_crew_suffix_rejects_bad_version_order_ids_bounds_and_trailing_bytes() {
    let (sim, first, second) = live();
    let bytes = sim.snapshot();
    let f = Fixed::ONE.to_bits();
    let valid = (first, [f, 2 * f]);
    // The unmodified chain loads, so each rejection below is the edited record's.
    assert!(Sim::from_snapshot(&replace_crew(&bytes, 1, vec![valid]), None).is_ok());
    assert!(Sim::from_snapshot(&chain::before(&bytes, chain::CREW), None).is_err());
    assert!(Sim::from_snapshot(&bytes[..bytes.len() - 1], None).is_err());
    for (rows, why) in [
        (vec![valid, valid], "unit crew rows out of order"),
        (vec![(second, [0, 0]), valid], "unit crew rows out of order"),
        (vec![(EntityId(99), [0, f])], "unknown unit crew entity"),
        (vec![(first, [-1, f])], "invalid unit crew pool"),
        (vec![(first, [0, -1])], "invalid unit crew pool"),
    ] {
        assert_eq!(chain::corrupt(&replace_crew(&bytes, 1, rows), None), why);
    }
    assert_eq!(chain::corrupt(&replace_crew(&bytes, 2, vec![valid]), None), "unknown unit crew record version");
    let mut trailing = bytes;
    trailing.push(0);
    assert_eq!(chain::corrupt(&trailing, None), "trailing bytes");
}

#[test]
fn removed_or_noncraft_entities_cannot_receive_saved_crew_rows() {
    let (mut sim, first, _) = live();
    let row = (first, [0, Fixed::ONE.to_bits()]);
    sim.despawn(first, None);
    assert_eq!(chain::corrupt(&replace_crew(&sim.snapshot(), 1, vec![row]), None), "unknown unit crew entity");
    let (mut sim, first, _) = live();
    sim.units.get_mut(first).unwrap().kind = EntityKind::Prop;
    assert_eq!(chain::corrupt(&sim.snapshot(), None), "unit crew entity is not a craft or station");
}

#[test]
fn the_v12_layout_is_refused_by_version() {
    let (sim, _, _) = live();
    let bytes = sim.snapshot();
    let mut old = chain::before(&bytes, chain::CREW);
    old[4..6].copy_from_slice(&12u16.to_le_bytes());
    old[6..10].copy_from_slice(&3u32.to_le_bytes());
    assert_eq!(Sim::from_snapshot(&old, None).err(), Some(SnapshotError::FormatVersion { found: 12 }));
}

#[test]
fn genuine_v12_producer_save_is_refused_and_its_game_keeps_the_original_hashes() {
    // Generated with the preserved v12/seam3 compiled producer
    // (make-unit-status-v12.rs). Refused by version since CON-35 ...
    let bytes = include_bytes!("../../tests/fixtures/unit-status-v12.stas");
    assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 12);
    assert_eq!(u32::from_le_bytes(bytes[6..10].try_into().unwrap()), 3);
    let db = Arc::new(sta_content::fake::db());
    assert_eq!(Sim::from_snapshot(bytes, Some(db.clone())).err(), Some(SnapshotError::FormatVersion { found: 12 }));
    // ... and the producer's game, rebuilt, keeps its recorded digests with
    // unit crew and the post-CON-35 content hash left out.
    let (races, warnings) = sta_content::Races::load_with(&mut |name, _| match name {
        "races.odf" => Some("race0=fx".into()),
        "fx.odf" => Some("name=fx\ninstantActionSlot=0\nstandardUnits1=fx_cruiser\nnormalCrew=1200".into()),
        _ => None,
    });
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut map = sta_content::MapDef::empty();
    map.starts = sta_content::fake::two_bases().starts;
    let setup = crate::setup::GameSetup {
        seed: 83,
        map: Arc::new(map),
        content: db.clone(),
        players: vec![crate::setup::PlayerSetup { id: PlayerId(0), team: TeamId(0), race: "fx".into(), control: crate::setup::Control::Human }],
        options: Default::default(),
    };
    let (mut sim, warnings) = Sim::from_setup_with_starts(&setup, &races);
    assert!(warnings.is_empty(), "{warnings:?}");
    for _ in 0..5 {
        sim.step(&[]);
    }
    assert_eq!(crate::unit_status_tests::digest_before_crew(&sim), 0x4e40_8817_1a0a_6089);
    assert_eq!(sim.view().player(PlayerId(0)).unwrap().crew, Some(Fixed::from_int(1200)));
    for _ in 0..30 {
        sim.step(&[]);
    }
    assert_eq!(crate::unit_status_tests::digest_before_crew(&sim), 0x010a_ea28_0667_9d07);
}

#[test]
fn previous_revision_three_replay_rejects_before_playback() {
    let db = Arc::new(sta_content::fake::db());
    let setup = crate::setup::GameSetup {
        seed: 61,
        content: db,
        map: Arc::new(sta_content::MapDef::empty()),
        players: Vec::new(),
        options: Default::default(),
    };
    let mut replay = crate::replay::Recorder::new(&setup).finish();
    replay.seam_rev = 3;
    assert!(matches!(crate::replay::Replay::decode(&replay.encode()), Err(crate::replay::ReplayError::SeamRev { found: 3 })));
}
