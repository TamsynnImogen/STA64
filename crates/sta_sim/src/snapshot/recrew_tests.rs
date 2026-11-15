use super::*;

#[test]
fn station_recrew_record_is_mandatory_and_schema_checked() {
    let bytes = Sim::new(9).snapshot();
    let unknown = crate::recrew::RecrewRec { version: 2, rows: Vec::new() };
    let bad = chain::with(&bytes, chain::RECREW, chain::enc(&unknown));
    assert_eq!(chain::corrupt(&bad, None), "invalid station recrew record");
    let missing = chain::before(&bytes, chain::RECREW);
    assert!(matches!(Sim::from_snapshot(&missing, None), Err(SnapshotError::Corrupt(_))));
    let restored = Sim::from_snapshot(&bytes, None).unwrap();
    assert_eq!(restored.digest(), Sim::new(9).digest());
}
