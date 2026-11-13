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

fn crew_offset(bytes: &[u8]) -> usize {
    let (_, rest): (Body, _) = postcard::take_from_bytes(&bytes[HEADER..]).unwrap();
    let (_, rest): (Option<(Vec3, Vec3)>, _) = postcard::take_from_bytes(rest).unwrap();
    let (_, rest): (BoundaryRec, _) = postcard::take_from_bytes(rest).unwrap();
    let (_, rest): (PerceptionRec, _) = postcard::take_from_bytes(rest).unwrap();
    let (_, rest): (PersonnelRec, _) = postcard::take_from_bytes(rest).unwrap();
    bytes.len() - rest.len()
}

fn replace_crew(bytes: &[u8], version: u8, rows: Vec<(EntityId, [i64; 2])>) -> Vec<u8> {
    let mut out = bytes[..crew_offset(bytes)].to_vec();
    out.extend(postcard::to_allocvec(&UnitCrewRec { version, rows }).unwrap());
    out.extend(postcard::to_allocvec(&StartEffectsRec { version: 1, rows: Vec::new() }).unwrap());
    out.extend(postcard::to_allocvec(&crate::autonomy::AutonomyWire { version: 1, rows: Vec::new() }).unwrap());
    out
}

#[test]
fn current_crew_roundtrips_known_zero_and_live_values_without_capacity_refill() {
    let (mut sim, first, second) = live();
    let bytes = sim.snapshot();
    assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), crate::snapshot::FORMAT_VERSION);
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
    assert!(Sim::from_snapshot(&bytes[..crew_offset(&bytes)], None).is_err());
    assert!(Sim::from_snapshot(&bytes[..bytes.len() - 1], None).is_err());
    for rows in [
        vec![valid, valid],
        vec![(second, [0, 0]), valid],
        vec![(EntityId(99), [0, f])],
        vec![(first, [-1, f])],
        vec![(first, [0, -1])],
    ] {
        assert!(matches!(Sim::from_snapshot(&replace_crew(&bytes, 1, rows), None), Err(SnapshotError::Corrupt(_))));
    }
    assert!(matches!(Sim::from_snapshot(&replace_crew(&bytes, 2, vec![valid]), None), Err(SnapshotError::Corrupt(why)) if why.contains("unit crew record version")));
    let mut trailing = bytes;
    trailing.push(0);
    assert!(Sim::from_snapshot(&trailing, None).is_err());
}

#[test]
fn removed_or_noncraft_entities_cannot_receive_saved_crew_rows() {
    let (mut sim, first, _) = live();
    let row = (first, [0, Fixed::ONE.to_bits()]);
    sim.despawn(first, None);
    assert!(Sim::from_snapshot(&replace_crew(&sim.snapshot(), 1, vec![row]), None).is_err());
    let (mut sim, first, _) = live();
    sim.units.get_mut(first).unwrap().kind = EntityKind::Prop;
    assert!(Sim::from_snapshot(&sim.snapshot(), None).is_err());
}

#[test]
fn previous_v12_preserves_player_personnel_but_unit_crew_stays_unknown() {
    let (sim, first, second) = live();
    let bytes = sim.snapshot();
    let mut old = bytes[..crew_offset(&bytes)].to_vec();
    old[4..6].copy_from_slice(&12u16.to_le_bytes());
    old[6..10].copy_from_slice(&3u32.to_le_bytes());
    let restored = Sim::from_snapshot(&old, None).unwrap();
    for id in [first, second] {
        assert_eq!(restored.view().unit(id).unwrap().crew, None);
    }
    assert_eq!(restored.view().player(PlayerId(0)).unwrap().crew, Some(Fixed::from_int(1200)));
    assert_eq!(restored.economy.officer_limit(PlayerId(0)), Some(Fixed::from_int(600)));
    assert_ne!(restored.digest(), sim.digest());
    assert_eq!(Sim::from_snapshot(&restored.snapshot(), None).unwrap().view(), restored.view());
    old[6..10].copy_from_slice(&SEAM_REV.to_le_bytes());
    assert!(matches!(Sim::from_snapshot(&old, None), Err(SnapshotError::SeamRev { .. })));
}

#[test]
fn genuine_v12_producer_save_keeps_unknown_crew_and_original_continuation_hashes() {
    // Generated with the preserved v12/seam3 compiled producer, not by
    // stripping/re-labelling this build's snapshot. Synthetic content only.
    let bytes = include_bytes!("../../tests/fixtures/unit-status-v12.stas");
    assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 12);
    assert_eq!(u32::from_le_bytes(bytes[6..10].try_into().unwrap()), 3);
    let db = Arc::new(sta_content::fake::db());
    let mut sim = Sim::from_snapshot(bytes, Some(db.clone())).unwrap();
    assert_eq!(sim.digest(), 0x4e40_8817_1a0a_6089);
    assert_eq!(sim.tick, 5);
    assert_eq!(sim.view().player(PlayerId(0)).unwrap().crew, Some(Fixed::from_int(1200)));
    let unit = sim.units.iter().next().unwrap();
    assert_eq!(db.unit(unit.def).unwrap().crew, Fixed::from_int(300));
    assert_eq!(unit.crew, None); // Current crew is never filled from that capacity.
    for _ in 0..30 {
        sim.step(&[]);
    }
    assert_eq!(sim.tick, 35);
    assert_eq!(sim.digest(), 0x010a_ea28_0667_9d07);
    assert_eq!(Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap().view(), sim.view());
}

#[test]
fn v12_does_not_gain_the_historical_pre_personnel_content_identity_exception() {
    let db = Arc::new(sta_content::fake::db().with_rules(sta_content::Rules::from_header("int SINGLE_PLAYER_OFFICER_LIMIT=600;").0));
    assert_ne!(db.hash(), db.hash_without_starting_rules());
    let setup = crate::setup::GameSetup {
        seed: 61,
        content: db.clone(),
        map: Arc::new(sta_content::MapDef::empty()),
        players: Vec::new(),
        options: Default::default(),
    };
    let bytes = Sim::from_setup(&setup).snapshot();
    let mut old = bytes[..crew_offset(&bytes)].to_vec();
    old[4..6].copy_from_slice(&12u16.to_le_bytes());
    old[6..10].copy_from_slice(&3u32.to_le_bytes());
    assert!(Sim::from_snapshot(&old, Some(db.clone())).is_ok());
    old[10..18].copy_from_slice(&db.hash_without_starting_rules().to_le_bytes());
    assert!(matches!(Sim::from_snapshot(&old, Some(db)), Err(SnapshotError::ContentMismatch { .. })));
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
