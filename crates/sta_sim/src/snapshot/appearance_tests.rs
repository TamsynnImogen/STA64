use super::*;
use crate::setup::{Control, GameSetup, PlayerSetup};

fn fixture() -> (Vec<u8>, Arc<ContentDb>) {
    let content = Arc::new(sta_content::fake::db());
    let setup = GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: content.clone(),
        options: Default::default(),
        players: vec![
            PlayerSetup {
                id: PlayerId(0),
                team: TeamId(0),
                race: "federation".into(),
                control: Control::Human,
            },
            PlayerSetup {
                id: PlayerId(1),
                team: TeamId(1),
                race: "klingon".into(),
                control: Control::Idle,
            },
        ],
    };
    (Sim::from_setup(&setup).snapshot(), content)
}

#[test]
fn appearance_record_refuses_unknown_versions_missing_data_and_noncanonical_rows() {
    let (bytes, content) = fixture();
    let record = chain::APPEARANCE;
    for factions in [
        vec![(PlayerId(0), "".into())],
        vec![(PlayerId(0), "Federation".into())],
        vec![(PlayerId(0), " federation".into())],
        vec![(PlayerId(2), "other".into())],
        vec![
            (PlayerId(0), "federation".into()),
            (PlayerId(0), "klingon".into()),
        ],
        vec![
            (PlayerId(1), "klingon".into()),
            (PlayerId(0), "federation".into()),
        ],
    ] {
        let bad = chain::with(
            &bytes,
            record,
            chain::enc(&AppearanceRec {
                version: 1,
                factions,
            }),
        );
        assert!(chain::corrupt(&bad, Some(content.clone())).contains("appearance faction"));
    }
    let bad = chain::with(
        &bytes,
        record,
        chain::enc(&AppearanceRec {
            version: 2,
            factions: Vec::new(),
        }),
    );
    assert!(chain::corrupt(&bad, Some(content.clone())).contains("appearance record version"));
    let truncated = chain::before(&bytes, record);
    assert!(Sim::from_snapshot(&truncated, Some(content.clone())).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(chain::corrupt(&trailing, Some(content)).contains("trailing"));
}

#[test]
fn mission_attributes_and_setup_factions_survive_the_same_snapshot() {
    let (bytes, content) = fixture();
    let mut sim = Sim::from_snapshot(&bytes, Some(content.clone())).unwrap();
    sim.attrs.set_player_race(PlayerId(0), "klingon");
    let bytes = sim.snapshot();
    let restored = Sim::from_snapshot(&bytes, Some(content)).unwrap();
    assert_eq!(sim.view().attrs, restored.view().attrs);
    assert_eq!(sim.view().appearance, restored.view().appearance);
    assert_eq!(sim.digest(), restored.digest());
    assert_eq!(bytes, restored.snapshot());
}

#[test]
fn previous_snapshot_and_view_seams_are_refused_before_new_data_is_read() {
    let (mut bytes, content) = fixture();
    for previous in [20_u16, 21, 22] {
        bytes[4..6].copy_from_slice(&previous.to_le_bytes());
        assert!(matches!(
            Sim::from_snapshot(&bytes, Some(content.clone())),
            Err(SnapshotError::FormatVersion { found }) if found == previous
        ));
    }
    bytes[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes[6..10].copy_from_slice(&4_u32.to_le_bytes());
    assert!(matches!(
        Sim::from_snapshot(&bytes, Some(content)),
        Err(SnapshotError::SeamRev { found: 4 })
    ));
    assert!(
        crate::fake::ViewLog::from_bytes(&[4])
            .unwrap_err()
            .contains("seam revision 4")
    );
}
