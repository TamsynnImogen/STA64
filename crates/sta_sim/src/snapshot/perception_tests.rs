use super::*;
use crate::WorldSource;

fn live() -> (Sim, EntityId, EntityId) {
    let mut sim = Sim::new(43);
    let heading = (Fixed::ZERO, Fixed::ONE);
    let first = sim.spawn(0, Vec3::ZERO, heading, MoveStats::STATIC);
    let second = sim.spawn(0, Vec3::ZERO, heading, MoveStats::STATIC);
    (sim, first, second)
}

fn split(bytes: &[u8]) -> (Body, Option<(Vec3, Vec3)>, BoundaryRec, PerceptionRec) {
    let (body, rest) = postcard::take_from_bytes(&bytes[HEADER..]).unwrap();
    let (bounds, rest) = postcard::take_from_bytes(rest).unwrap();
    let (boundary, rest) = postcard::take_from_bytes(rest).unwrap();
    let (perception, rest) = postcard::take_from_bytes(rest).unwrap();
    let (personnel, rest): (PersonnelRec, _) = postcard::take_from_bytes(rest).unwrap();
    assert!(personnel.rows.is_empty());
    assert!(rest.is_empty());
    (body, bounds, boundary, perception)
}

fn join(
    bytes: &[u8],
    body: &Body,
    bounds: Option<(Vec3, Vec3)>,
    boundary: &BoundaryRec,
    perception: &PerceptionRec,
) -> Vec<u8> {
    let mut out = bytes[..HEADER].to_vec();
    out.extend(postcard::to_allocvec(body).unwrap());
    out.extend(postcard::to_allocvec(&bounds).unwrap());
    out.extend(postcard::to_allocvec(boundary).unwrap());
    out.extend(postcard::to_allocvec(perception).unwrap());
    if u16::from_le_bytes(out[4..6].try_into().unwrap()) == FORMAT_VERSION {
        out.extend(postcard::to_allocvec(&PersonnelRec { version: 1, rows: Vec::new() }).unwrap());
    }
    out
}

#[test]
fn v11_roundtrip_and_continuation_keep_perceived_owner_separate() {
    let (mut sim, first, second) = live();
    sim.perception.insert(first, PlayerId(1));
    let saved = sim.snapshot();
    assert_eq!(u16::from_le_bytes(saved[4..6].try_into().unwrap()), FORMAT_VERSION);
    assert_eq!(u32::from_le_bytes(saved[6..10].try_into().unwrap()), SEAM_REV);
    let mut loaded = Sim::from_snapshot(&saved, None).unwrap();
    assert_eq!(loaded.snapshot(), saved);
    assert_eq!(loaded.digest(), sim.digest());
    assert_eq!(loaded.perceived_owner(first), Some(PlayerId(1)));
    assert_eq!(loaded.perceived_owner(second), Some(PlayerId(0)));
    assert_eq!(loaded.unit(first).unwrap().owner, PlayerId(0));
    assert_eq!(loaded.view().perceived_owner(first), Some(PlayerId(1)));
    for tick in 0..30 {
        assert_eq!(loaded.step(&[]), sim.step(&[]), "tick {tick}");
    }
    assert_eq!(loaded.snapshot(), sim.snapshot());
}

#[test]
fn v11_rejects_noncanonical_rows_unknown_version_missing_suffix_and_trailing_bytes() {
    let (mut sim, first, second) = live();
    sim.perception.insert(first, PlayerId(1));
    let bytes = sim.snapshot();
    let (body, bounds, boundary, mut perception) = split(&bytes);
    assert_eq!(perception.version, 1);
    assert_eq!(perception.rows, [(first, PlayerId(1))]);
    for rows in [
        vec![(first, PlayerId(1)), (first, PlayerId(2))],
        vec![(second, PlayerId(1)), (first, PlayerId(2))],
        vec![(EntityId(999), PlayerId(1))],
        vec![(first, PlayerId(0))],
        vec![(first, PlayerId(9))],
        vec![(first, PlayerId::SCRIPT)],
    ] {
        perception.rows = rows;
        assert!(Sim::from_snapshot(&join(&bytes, &body, bounds, &boundary, &perception), None).is_err());
    }
    perception.rows = vec![(first, PlayerId(1))];
    perception.version = 2;
    assert!(matches!(
        Sim::from_snapshot(&join(&bytes, &body, bounds, &boundary, &perception), None),
        Err(SnapshotError::Corrupt(why)) if why.contains("perceived-team record version")
    ));
    let mut missing = bytes[..HEADER].to_vec();
    missing.extend(postcard::to_allocvec(&body).unwrap());
    missing.extend(postcard::to_allocvec(&bounds).unwrap());
    missing.extend(postcard::to_allocvec(&boundary).unwrap());
    assert!(Sim::from_snapshot(&missing, None).is_err());
    assert!(Sim::from_snapshot(&bytes[..bytes.len() - 1], None).is_err());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(Sim::from_snapshot(&trailing, None).is_err());
}

#[test]
fn historical_v10_seam1_layout_restores_empty_perception_and_mismatched_seams_reject() {
    let (mut sim, first, _) = live();
    sim.perception.insert(first, PlayerId(1));
    let bytes = sim.snapshot();
    let (body, bounds, boundary, _) = split(&bytes);
    let mut old = bytes[..HEADER].to_vec();
    old[4..6].copy_from_slice(&PREVIOUS_FORMAT.to_le_bytes());
    old[6..10].copy_from_slice(&1_u32.to_le_bytes());
    old.extend(postcard::to_allocvec(&body).unwrap());
    old.extend(postcard::to_allocvec(&bounds).unwrap());
    old.extend(postcard::to_allocvec(&boundary).unwrap());
    let restored = Sim::from_snapshot(&old, None).unwrap();
    assert_eq!(restored.perceived_owner(first), Some(PlayerId(0)));
    assert!(restored.perception.iter().next().is_none());
    let mut wrong_new = bytes.clone();
    wrong_new[6..10].copy_from_slice(&1_u32.to_le_bytes());
    assert!(matches!(
        Sim::from_snapshot(&wrong_new, None),
        Err(SnapshotError::SeamRev { found: 1 })
    ));
    let mut wrong_old = old;
    wrong_old[6..10].copy_from_slice(&SEAM_REV.to_le_bytes());
    assert!(matches!(Sim::from_snapshot(&wrong_old, None), Err(SnapshotError::SeamRev { found }) if found == SEAM_REV));
}
