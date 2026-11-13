//! Test helper: a snapshot as its chain of records, so a test replaces ONE
//! record and re-joins the rest unchanged. Adding a record to the format is one
//! edit here ([`split`] and the index constants), not one per test file.

use super::*;

pub(super) const PERSONNEL: usize = 6;
pub(super) const CREW: usize = 7;
pub(super) const SUBSYSTEMS: usize = 10;
pub(super) const ATTACK: usize = 11;
pub(super) const WATCH: usize = 12;
/// Number of segments [`split`] returns (header plus twelve records).
pub(super) const LEN: usize = 13;

fn cut<T: serde::de::DeserializeOwned>(rest: &mut &[u8], out: &mut Vec<Vec<u8>>) {
    let (_, tail): (T, _) = postcard::take_from_bytes(rest).expect("record parses");
    out.push(rest[..rest.len() - tail.len()].to_vec());
    *rest = tail;
}

/// The header, then each record's bytes in file order.
pub(super) fn split(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = vec![bytes[..HEADER].to_vec()];
    let mut rest = &bytes[HEADER..];
    cut::<Provenance>(&mut rest, &mut out);
    cut::<Body>(&mut rest, &mut out);
    cut::<Option<(Vec3, Vec3)>>(&mut rest, &mut out);
    cut::<BoundaryRec>(&mut rest, &mut out);
    cut::<PerceptionRec>(&mut rest, &mut out);
    cut::<PersonnelRec>(&mut rest, &mut out);
    cut::<UnitCrewRec>(&mut rest, &mut out);
    cut::<StartEffectsRec>(&mut rest, &mut out);
    cut::<crate::autonomy::AutonomyWire>(&mut rest, &mut out);
    cut::<SubsystemsRec>(&mut rest, &mut out);
    cut::<AttackRec>(&mut rest, &mut out);
    cut::<WatchRec>(&mut rest, &mut out);
    assert!(rest.is_empty(), "the chain covers the whole snapshot");
    assert_eq!(out.len(), LEN);
    out
}

pub(super) fn join(segments: &[Vec<u8>]) -> Vec<u8> {
    segments.concat()
}

pub(super) fn enc<T: Serialize>(record: &T) -> Vec<u8> {
    postcard::to_allocvec(record).unwrap()
}

/// `bytes` with record `index` replaced.
pub(super) fn with(bytes: &[u8], index: usize, record: Vec<u8>) -> Vec<u8> {
    let mut segments = split(bytes);
    segments[index] = record;
    join(&segments)
}

/// `bytes` cut before record `index`.
pub(super) fn before(bytes: &[u8], index: usize) -> Vec<u8> {
    join(&split(bytes)[..index])
}

/// The error text of a rejected load.
pub(super) fn corrupt(bytes: &[u8], content: Option<Arc<ContentDb>>) -> String {
    match Sim::from_snapshot(bytes, content) {
        Err(SnapshotError::Corrupt(why)) => why,
        other => panic!("expected Corrupt, got {:?}", other.map(|_| "a sim")),
    }
}
