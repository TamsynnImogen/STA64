use super::*;

#[test]
fn the_repair_queue_record_is_mandatory_and_schema_checked() {
    let bytes = Sim::new(9).snapshot();
    let unknown = crate::repair::RepairsRec { version: 3, queues: Vec::new(), beams: Vec::new() };
    let bad = chain::with(&bytes, chain::REPAIRS, chain::enc(&unknown));
    assert!(chain::corrupt(&bad, None).contains("repair queues"), "{}", chain::corrupt(&bad, None));
    let missing = chain::before(&bytes, chain::REPAIRS);
    assert!(matches!(Sim::from_snapshot(&missing, None), Err(SnapshotError::Corrupt(_))));
    let restored = Sim::from_snapshot(&bytes, None).unwrap();
    assert_eq!(restored.digest(), Sim::new(9).digest());
}

#[test]
fn the_decommission_record_is_mandatory_and_schema_checked() {
    let bytes = Sim::new(9).snapshot();
    let unknown = crate::economy::recycle::RecyclesRec { version: 2, jobs: Vec::new() };
    let bad = chain::with(&bytes, chain::RECYCLES, chain::enc(&unknown));
    assert!(chain::corrupt(&bad, None).contains("decommissions"), "{}", chain::corrupt(&bad, None));
    let missing = chain::before(&bytes, chain::RECYCLES);
    assert!(matches!(Sim::from_snapshot(&missing, None), Err(SnapshotError::Corrupt(_))));
}
