//! Unit tests of the container that need its private parts: files sealed by
//! hand (a mission header), and damage that keeps the checksums valid so
//! the parsers behind them are exercised.

use super::container::{self, PREFIX, Parts};
use super::*;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::WorldSource;
use sta_core::TeamId;

fn setup() -> GameSetup {
    GameSetup {
        seed: 3,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::new(sta_content::fake::db()),
        players: vec![
            PlayerSetup { id: PlayerId(0), team: TeamId(0), race: "fx".into(), control: Control::Human },
            PlayerSetup { id: PlayerId(1), team: TeamId(1), race: "fx".into(), control: Control::Idle },
        ],
        options: GameOptions::default(),
    }
}

fn info() -> SaveInfo {
    SaveInfo {
        build_id: "test".into(),
        map: Some(MapId { path: "maps/two_bases".into(), hash: 7 }),
        description: "unit".into(),
        made_at: 1000,
        ..SaveInfo::default()
    }
}

fn a_save() -> Vec<u8> {
    let setup = setup();
    let mut sim = Sim::from_setup(&setup);
    let mut replay = Replay::new(&setup);
    replay.digests.push((0, sim.digest()));
    for _ in 0..5 {
        let before = sim.tick;
        let out = sim.step(&[]);
        replay.log_step(before, &[], out.view.tick, out.view.digest, 1);
    }
    save(&sim, &replay, &info(), &[], &ClientState { version: 2, data: vec![1, 2, 3] }).expect("saves")
}

fn map() -> Option<MapSource> {
    Some(MapSource { hash: 7, map: Arc::new(sta_content::fake::two_bases()) })
}

/// Recomputes the header checksum and the file checksum after an edit.
fn reseal(mut b: Vec<u8>) -> Vec<u8> {
    if b.len() >= PREFIX + 8 {
        let hl = u32::from_le_bytes([b[6], b[7], b[8], b[9]]) as usize;
        if hl <= MAX_HEADER && PREFIX + hl + 8 <= b.len() {
            let sum = container::checksum(&b[..PREFIX + hl]);
            b[PREFIX + hl..PREFIX + hl + 8].copy_from_slice(&sum.to_le_bytes());
        }
    }
    if b.len() >= 8 {
        let n = b.len() - 8;
        let sum = container::checksum(&b[..n]);
        b[n..].copy_from_slice(&sum.to_le_bytes());
    }
    b
}

/// The save with its header and sections edited, sealed with valid checksums.
fn forged(edit: impl FnOnce(&mut Header, &mut Parts)) -> Vec<u8> {
    let bytes = a_save();
    let (mut header, body_at) = container::parse_header(&bytes).unwrap();
    let s = container::parse_sections(&bytes[body_at..bytes.len() - 8]).unwrap();
    let mut parts = Parts {
        snapshot: s.snapshot.to_vec(),
        journal: s.journal.to_vec(),
        controllers: s.controllers.to_vec(),
        client_version: s.client_version,
        client: s.client.to_vec(),
    };
    edit(&mut header, &mut parts);
    container::seal(&header, &parts).unwrap()
}

fn loads(bytes: &[u8]) -> Result<Loaded, SaveError> {
    load(bytes, Arc::new(sta_content::fake::db()), map())
}

#[test]
fn a_forged_file_with_valid_checksums_loads_as_the_original_does() {
    let bytes = forged(|_, _| {});
    assert!(bytes == a_save());
    assert!(loads(&bytes).is_ok());
}

#[test]
fn formats_this_build_does_not_read_are_refused_with_their_own_variant() {
    let snap = forged(|h, _| h.snapshot_format = snapshot::FORMAT_VERSION + 1000);
    let e = loads(&snap).err().unwrap();
    assert_eq!(e, SaveError::SnapshotFormat { found: snapshot::FORMAT_VERSION + 1000 });
    assert!(e.to_string().contains("snapshot format"));
    let old = forged(|h, _| h.snapshot_format = 3);
    assert!(matches!(loads(&old), Err(SaveError::SnapshotFormat { found: 3 })));
    let rep = forged(|h, _| h.replay_format = 1);
    let e = loads(&rep).err().unwrap();
    assert_eq!(e, SaveError::ReplayFormat { found: 1 });
    assert!(e.to_string().contains("replay format"));
    let seam = forged(|h, _| h.seam_rev = SEAM_REV + 1);
    let e = loads(&seam).err().unwrap();
    assert_eq!(e, SaveError::SeamRev { found: SEAM_REV + 1 });
    assert!(e.to_string().contains("seam revision"));
    // The header alone says the same, for the load list.
    assert_eq!(read_header(&seam).unwrap().refusal(None), Some(e));
}

#[test]
fn a_snapshot_the_sim_rejects_is_refused_with_its_reason() {
    let cut = forged(|_, p| p.snapshot.truncate(p.snapshot.len() / 2));
    let e = loads(&cut).err().unwrap();
    assert!(matches!(e, SaveError::Snapshot(SnapshotError::Corrupt(_))), "{e:?}");
    assert!(e.to_string().contains("cannot be loaded"));
    let tick = forged(|h, _| h.tick += 1);
    assert!(matches!(loads(&tick), Err(SaveError::Damaged(_))));
}

#[test]
fn a_networked_file_is_refused_on_load() {
    let remote = forged(|h, _| h.seats[1].control = ControlRec::Remote);
    assert_eq!(loads(&remote).err(), Some(SaveError::Networked { seat: PlayerId(1) }));
    let remote_ai = forged(|h, _| h.seats[1].control = ControlRec::RemoteAi("x".into()));
    assert_eq!(read_header(&remote_ai).unwrap().refusal(None), Some(SaveError::Networked { seat: PlayerId(1) }));
}

#[test]
fn journal_and_controller_sections_that_disagree_with_the_header_are_damage() {
    let ends_early = forged(|h, _| h.tick = 0);
    assert!(matches!(loads(&ends_early), Err(SaveError::Damaged(_))));
    // A computer seat with no record.
    let ai = forged(|h, _| h.seats[1].control = ControlRec::Ai("counter".into()));
    assert!(matches!(loads(&ai), Err(SaveError::Controller { seat, .. }) if seat == PlayerId(1)));
    // A record for a seat that is not a computer seat.
    let extra = forged(|_, p| {
        p.controllers = postcard::to_allocvec(&vec![SeatController { seat: PlayerId(1), name: "x".into(), version: 1, data: vec![] }]).unwrap();
    });
    assert!(matches!(loads(&extra), Err(SaveError::Damaged(_))));
    let journal = forged(|_, p| p.journal.push(0));
    assert!(matches!(loads(&journal), Err(SaveError::Damaged(_))));
}

#[test]
fn a_mission_file_is_refused_on_load_and_at_save() {
    let bytes = a_save();
    let (mut header, body_at) = container::parse_header(&bytes).unwrap();
    let s = container::parse_sections(&bytes[body_at..bytes.len() - 8]).unwrap();
    header.kind = GameKind::Mission;
    let sealed = container::seal(
        &header,
        &Parts {
            snapshot: s.snapshot.to_vec(),
            journal: s.journal.to_vec(),
            controllers: s.controllers.to_vec(),
            client_version: s.client_version,
            client: s.client.to_vec(),
        },
    )
    .unwrap();
    assert_eq!(read_header(&sealed).unwrap().refusal(None), Some(SaveError::Mission));
    assert!(matches!(load(&sealed, Arc::new(sta_content::fake::db()), map()), Err(SaveError::Mission)));
    let mut i = info();
    i.kind = GameKind::Mission;
    let setup = setup();
    let err = save(&Sim::from_setup(&setup), &Replay::new(&setup), &i, &[], &ClientState::default());
    assert_eq!(err, Err(SaveError::Mission));
}

/// Damage that keeps both checksums valid reaches the parsers behind them:
/// none may panic, whatever they decide.
#[test]
fn resealed_damage_never_panics() {
    let bytes = a_save();
    let content = Arc::new(sta_content::fake::db());
    let mut refused = 0;
    let mut total = 0;
    for i in 0..bytes.len() - 8 {
        // Every header byte, then a spread of the rest.
        if i > 600 && i % 7 != 0 {
            continue;
        }
        for mask in [0x01u8, 0x80, 0xff] {
            let mut b = bytes.clone();
            b[i] ^= mask;
            let b = reseal(b);
            total += 1;
            if load(&b, Arc::clone(&content), map()).is_err() {
                refused += 1;
            }
        }
    }
    assert!(refused * 2 > total, "only {refused} of {total} damaged files were refused");
}

#[test]
fn resealed_cuts_never_panic() {
    let bytes = a_save();
    let content = Arc::new(sta_content::fake::db());
    for n in 0..bytes.len() {
        let b = reseal(bytes[..n].to_vec());
        assert!(load(&b, Arc::clone(&content), map()).is_err(), "a file cut to {n} bytes loads");
    }
}

#[test]
fn a_huge_declared_length_is_refused_without_allocating_it() {
    let mut b = a_save();
    b[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(read_header(&b), Err(SaveError::Damaged(_))));
    assert!(matches!(load(&b, Arc::new(sta_content::fake::db()), map()), Err(SaveError::Damaged(_))));
}
