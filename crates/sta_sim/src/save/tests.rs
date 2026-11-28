//! Unit tests of the container that need its private parts: files sealed by
//! hand (forged headers and sections with valid checksums), golden encodings,
//! and damage that keeps the checksums valid so the parsers behind them are
//! exercised.

use super::container::{self, PREFIX, Parts};
use super::*;
use crate::{SimEvent, WorldSource};
use crate::command::{Command, CommandKind, Edit, Order};
use crate::replay::StartRec;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_core::{EntityId, Fixed, TeamId, Vec3};

struct Stateless;
impl Controller for Stateless {
    fn think(&mut self, _: &crate::view::WorldView, _: &[SimEvent], _: &mut Vec<Command>) {}
    fn save_state(&self) -> Option<ControllerState> {
        Some(ControllerState { version: 1, data: vec![] })
    }
}

fn setup_with(enemy: Control) -> GameSetup {
    GameSetup {
        seed: 3,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::new(sta_content::fake::db()),
        players: vec![
            PlayerSetup { id: PlayerId(0), team: TeamId(0), race: "fx".into(), control: Control::Human },
            PlayerSetup { id: PlayerId(1), team: TeamId(1), race: "fx".into(), control: enemy },
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
        local_seat: Some(PlayerId(0)),
        ..SaveInfo::default()
    }
}

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// A game with units, orders and (when `ai`) a computer seat, run `ticks`.
fn a_game(ai: bool, ticks: u64) -> (Sim, Replay, InFlight) {
    let setup = setup_with(if ai { Control::Ai("stateless".into()) } else { Control::Idle });
    let mut sim = Sim::from_setup(&setup);
    let mut replay = Replay::new(&setup);
    replay.digests.push((0, sim.digest()));
    let db = &setup.content;
    for t in 0..ticks {
        let mut cmds = Vec::new();
        if t == 0 {
            for (i, (odf, owner, x, z)) in
                [("fx_cruiser", 0u8, -300, 0), ("fx_cruiser", 0, -200, 0), ("fx_frigate", 1, 300, 0), ("fx_frigate", 1, 200, 0)].into_iter().enumerate()
            {
                let e = Edit::Spawn { odf: db.key(odf).unwrap(), player: PlayerId(owner), pos: v(x, z), heading: (Fixed::ZERO, Fixed::ONE) };
                cmds.push(Command { player: PlayerId::BENCH, seq: i as u32, kind: CommandKind::Edit(e) });
            }
        }
        if t == 1 {
            let ids: Vec<EntityId> = sim.view().units.iter().filter(|u| u.owner == PlayerId(0)).map(|u| u.id).collect();
            let target = sim.view().units.iter().find(|u| u.owner == PlayerId(1)).unwrap().id;
            cmds.push(Command { player: PlayerId(0), seq: 50, kind: CommandKind::Order(Order::Attack { units: ids, target }) });
        }
        let out = sim.step(&cmds);
        replay.log_step(t, &cmds, out.view.tick, out.view.digest, 5);
        let _ = out.events;
    }
    // Waiting for the next tick: the local player's order and a sandbox edit.
    let waiting = vec![
        Command { player: PlayerId(0), seq: 76, kind: CommandKind::Order(Order::Stop { units: vec![EntityId(3)] }) },
        Command { player: PlayerId::BENCH, seq: 77, kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(5))) },
    ];
    (sim, replay, InFlight { next_seq: 78, commands: waiting })
}

fn save_of(sim: &Sim, replay: &Replay, events: &InFlight, ai: bool) -> Vec<u8> {
    let ctl: Vec<(PlayerId, &dyn Controller)> = if ai { vec![(PlayerId(1), &Stateless)] } else { vec![] };
    save(sim, replay, &info(), &ctl, events, &ClientState { version: 2, data: vec![1, 2, 3] }).expect("saves")
}

fn a_save() -> Vec<u8> {
    let (sim, replay, events) = a_game(false, 40);
    save_of(&sim, &replay, &events, false)
}

fn a_save_with_ai() -> Vec<u8> {
    let (sim, replay, events) = a_game(true, 40);
    save_of(&sim, &replay, &events, true)
}

fn map() -> Option<MapSource> {
    Some(MapSource { hash: 7, map: Arc::new(sta_content::fake::two_bases()) })
}

fn db() -> Arc<ContentDb> {
    Arc::new(sta_content::fake::db())
}

fn loads(bytes: &[u8]) -> Result<Loaded, SaveError> {
    load(bytes, db(), map())
}

/// Recomputes the header checksum and the file checksum after an edit.
fn reseal(mut b: Vec<u8>) -> Vec<u8> {
    if b.len() >= PREFIX + 8 {
        let hl = u32::from_le_bytes([b[14], b[15], b[16], b[17]]) as usize;
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

/// A file's header and sections edited, sealed with valid checksums.
fn forge(bytes: &[u8], edit: impl FnOnce(&mut Header, &mut Parts)) -> Vec<u8> {
    let (mut header, body_at) = container::parse_header(bytes).unwrap();
    let s = container::parse_sections(&bytes[body_at..bytes.len() - 8]).unwrap();
    let mut parts = Parts {
        snapshot: s.snapshot.to_vec(),
        journal: s.journal.to_vec(),
        controllers: s.controllers.to_vec(),
        in_flight: s.in_flight.to_vec(),
        client_version: s.client_version,
        client: s.client.to_vec(),
    };
    edit(&mut header, &mut parts);
    container::seal(&header, &parts).unwrap()
}

fn forged(edit: impl FnOnce(&mut Header, &mut Parts)) -> Vec<u8> {
    forge(&a_save(), edit)
}

fn forged_ai(edit: impl FnOnce(&mut Header, &mut Parts)) -> Vec<u8> {
    forge(&a_save_with_ai(), edit)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ------------------------------------------------------------- H1: numbers

#[test]
fn a_forged_file_with_valid_checksums_loads_as_the_original_does() {
    let bytes = forged(|_, _| {});
    assert!(bytes == a_save());
    assert!(loads(&bytes).is_ok());
}

#[test]
fn the_numbers_are_read_at_their_offsets_and_refused_by_exact_match() {
    let bytes = a_save();
    let (c, s, r, m) = (CONTAINER_VERSION, snapshot::FORMAT_VERSION, replay::FORMAT_VERSION, SEAM_REV);
    assert_eq!(&bytes[..4], b"STAG");
    assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), c);
    assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), s);
    assert_eq!(u16::from_le_bytes([bytes[8], bytes[9]]), r);
    assert_eq!(u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]), m);
    // Each number changed in the file, with no checksum repaired: its own
    // refusal, with found and expected, never `Damaged`.
    let put16 = |at: usize, n: u16| {
        let mut b = bytes.clone();
        b[at..at + 2].copy_from_slice(&n.to_le_bytes());
        b
    };
    let mut seam = bytes.clone();
    seam[10..14].copy_from_slice(&(m + 1).to_le_bytes());
    for (file, want) in [
        (put16(4, c + 1), SaveError::ContainerVersion { found: c + 1, expected: c }),
        (put16(4, 0), SaveError::ContainerVersion { found: 0, expected: c }),
        (put16(6, s - 1), SaveError::SnapshotFormat { found: s - 1, expected: s }),
        (put16(6, s + 1), SaveError::SnapshotFormat { found: s + 1, expected: s }),
        (put16(8, r - 1), SaveError::ReplayFormat { found: r - 1, expected: r }),
        (put16(8, r + 1), SaveError::ReplayFormat { found: r + 1, expected: r }),
        (seam, SaveError::SeamRev { found: m + 1, expected: m }),
    ] {
        assert_eq!(read_header(&file).err(), Some(want.clone()));
        assert_eq!(loads(&file).err(), Some(want.clone()));
        let msg = want.to_string();
        assert!(msg.contains("not migrated"), "{msg}");
    }
}

#[test]
fn a_header_that_no_longer_decodes_is_still_refused_by_its_version_number() {
    // What a change to a type the header embeds does to an older file: the
    // postcard header cannot be read. The number in the prefix is read first.
    let mut b = a_save();
    for byte in &mut b[PREFIX..PREFIX + 40] {
        *byte = 0xff;
    }
    b[6..8].copy_from_slice(&(snapshot::FORMAT_VERSION - 1).to_le_bytes());
    assert_eq!(read_header(&b).err(), Some(SaveError::SnapshotFormat { found: snapshot::FORMAT_VERSION - 1, expected: snapshot::FORMAT_VERSION }));
    // With the right numbers it is damage, as it should be.
    b[6..8].copy_from_slice(&snapshot::FORMAT_VERSION.to_le_bytes());
    assert!(matches!(read_header(&b), Err(SaveError::Damaged(_))));
}

#[test]
fn the_previous_formats_are_not_accepted() {
    let prev = forged(|h, _| h.snapshot_format = snapshot::FORMAT_VERSION - 1);
    assert!(matches!(loads(&prev), Err(SaveError::SnapshotFormat { .. })));
    let prev = forged(|h, _| h.replay_format = replay::FORMAT_VERSION - 1);
    assert!(matches!(loads(&prev), Err(SaveError::ReplayFormat { .. })));
    // Older and newer say so.
    let newer = forged(|h, _| h.seam_rev = SEAM_REV + 1);
    assert!(loads(&newer).err().unwrap().to_string().contains("newer"));
    let older = forged(|h, _| h.snapshot_format = 1);
    assert!(loads(&older).err().unwrap().to_string().contains("older"));
    // The header alone says the same, for the load list.
    assert!(matches!(read_header(&older), Err(SaveError::SnapshotFormat { found: 1, .. })));
}

/// The types in the header and the section records, encoded: when one of
/// these changes, `CONTAINER_VERSION` must rise (see `container`'s docs).
/// The golden strings are authored here, not taken from a real match.
fn sample_header() -> Header {
    Header {
        build_id: "build".into(),
        snapshot_format: 0,
        replay_format: 0,
        seam_rev: 0,
        content_hash: 0x0123_4567_89ab_cdef,
        lineage: "native".into(),
        packs: vec!["pack one".into(), "pack two".into()],
        profile: "native".into(),
        install_key: "install".into(),
        mod_key: "mods".into(),
        map: MapId { path: "maps/x".into(), hash: 42 },
        kind: GameKind::Skirmish,
        mission: MissionIdentity { campaign_digest: 1, mission: "m".into(), script_path: "s".into(), script_hash: 2, converter_version: 3 },
        seats: vec![
            SeatRec { id: PlayerId(0), team: TeamId(0), race: "a".into(), control: ControlRec::Human },
            SeatRec { id: PlayerId(1), team: TeamId(1), race: "b".into(), control: ControlRec::Ai("ai".into()) },
            SeatRec { id: PlayerId(2), team: TeamId(1), race: "c".into(), control: ControlRec::Idle },
            SeatRec { id: PlayerId(3), team: TeamId(2), race: "d".into(), control: ControlRec::Remote },
            SeatRec { id: PlayerId(4), team: TeamId(2), race: "e".into(), control: ControlRec::RemoteAi("r".into()) },
        ],
        options: GameOptions { officer_limit: 5, directors_cut: true, allow_warp: false, ..GameOptions::default() },
        starts: StartRec::Races { loadouts: 9 },
        seed: 77,
        local_seat: Some(PlayerId(0)),
        tick: 1234,
        play_time_secs: 5678,
        made_at: 1_700_000_000,
        description: "desc".into(),
    }
}

const GOLDEN_HEADER: &str = "056275696c64ef9bafcdf8acd19101066e617469766502087061636b206f6e65087061636b2074776f066e617469766507696e7374616c6c046d6f6473066d6170732f782a0001016d01730203050000016100010101620202616902010163030302016401040201650401720005010001094d0100d209ae2c80e2cfaa060464657363";
const GOLDEN_SEAT_CONTROLLER: &str = "010102616903020908";
const GOLDEN_JOURNAL: &str = "ac020200011e0200";
const GOLDEN_IN_FLIGHT: &str = "0501000400010107";

#[test]
fn golden_header_encoding_has_not_changed() {
    let h = postcard::to_allocvec(&sample_header()).unwrap();
    assert_eq!(hex(&h), GOLDEN_HEADER, "Header layout changed: bump CONTAINER_VERSION and regenerate the golden");
    // And it reads back.
    let back: Header = postcard::from_bytes(&h).unwrap();
    assert_eq!(back, sample_header());
}

#[test]
fn golden_section_records_have_not_changed() {
    let seat = postcard::to_allocvec(&vec![SeatController { seat: PlayerId(1), name: "ai".into(), version: 3, data: vec![9, 8] }]).unwrap();
    assert_eq!(hex(&seat), GOLDEN_SEAT_CONTROLLER, "SeatController layout changed: bump CONTAINER_VERSION and regenerate the golden");
    let journal = postcard::to_allocvec(&JournalRec { end_tick: 300, digests: &[(0, 1), (30, 2)], commands: &[] }).unwrap();
    assert_eq!(hex(&journal), GOLDEN_JOURNAL, "journal layout changed: bump CONTAINER_VERSION and regenerate the golden");
    let waiting = InFlight {
        next_seq: 5,
        commands: vec![Command { player: PlayerId(0), seq: 4, kind: CommandKind::Order(Order::Stop { units: vec![EntityId(7)] }) }],
    };
    let in_flight = postcard::to_allocvec(&waiting).unwrap();
    assert_eq!(hex(&in_flight), GOLDEN_IN_FLIGHT, "commands-in-flight layout changed: bump CONTAINER_VERSION and regenerate the golden");
}

// ------------------------------------------------------ other refusals

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
fn a_mission_file_is_refused_on_load_and_at_save() {
    let sealed = forged(|h, _| h.kind = GameKind::Mission);
    assert_eq!(read_header(&sealed).unwrap().refusal(None), Some(SaveError::Mission));
    assert!(matches!(loads(&sealed), Err(SaveError::Mission)));
    let (sim, replay, events) = a_game(false, 3);
    let mut i = info();
    i.kind = GameKind::Mission;
    assert_eq!(save(&sim, &replay, &i, &[], &events, &ClientState::default()), Err(SaveError::Mission));
}

#[test]
fn an_observers_save_is_refused_at_save_and_at_load() {
    let (sim, replay, events) = a_game(false, 3);
    let mut i = info();
    i.local_seat = None;
    assert_eq!(save(&sim, &replay, &i, &[], &events, &ClientState::default()), Err(SaveError::Observer));
    let file = forged(|h, _| h.local_seat = None);
    assert_eq!(loads(&file).err(), Some(SaveError::Observer));
    assert!(SaveError::Observer.to_string().contains("observer"));
    i.local_seat = Some(PlayerId(5));
    assert!(matches!(save(&sim, &replay, &i, &[], &events, &ClientState::default()), Err(SaveError::Invalid(_))));
}

// ------------------------------------------------ commands in flight

#[test]
fn the_commands_in_flight_and_the_sequence_number_come_back_as_saved() {
    let (sim, replay, in_flight) = a_game(false, 40);
    let loaded = loads(&save_of(&sim, &replay, &in_flight, false)).unwrap();
    assert_eq!(loaded.in_flight, in_flight);
    assert_eq!(loaded.in_flight.next_seq, 78, "the counter is stored, not derived from the commands");
    // The journal is not touched by them: they enter it when they are stepped.
    assert!(loaded.replay.commands == replay.commands);
}

#[test]
fn a_command_in_flight_from_a_stranger_is_refused_at_save_and_at_load() {
    let (sim, replay, mut in_flight) = a_game(false, 40);
    in_flight.commands.push(Command { player: PlayerId(5), seq: 80, kind: CommandKind::Order(Order::Stop { units: vec![] }) });
    let e = save(&sim, &replay, &info(), &[], &in_flight, &ClientState::default()).unwrap_err();
    assert!(matches!(&e, SaveError::Invalid(why) if why.contains("player 5")), "{e:?}");
    let forged = forged(|_, p| p.in_flight = postcard::to_allocvec(&in_flight).unwrap());
    assert!(matches!(loads(&forged), Err(SaveError::Invalid(why)) if why.contains("player 5")));
    // The sandbox player and every seat are fine; the script player is not a seat.
    in_flight.commands.pop();
    in_flight.commands.push(Command { player: PlayerId::SCRIPT, seq: 81, kind: CommandKind::Order(Order::Stop { units: vec![] }) });
    assert!(matches!(save(&sim, &replay, &info(), &[], &in_flight, &ClientState::default()), Err(SaveError::Invalid(_))));
}

// -------------------------------------- M2: the header against the snapshot

#[test]
fn a_header_that_hides_a_computer_seat_is_refused() {
    let hidden = forged_ai(|h, p| {
        h.seats[1].control = ControlRec::Idle;
        p.controllers = postcard::to_allocvec(&Vec::<SeatController>::new()).unwrap();
    });
    let e = loads(&hidden).err().expect("refused");
    assert!(matches!(&e, SaveError::Invalid(why) if why.contains("computer seats")), "{e:?}");
    // And the other way: a computer seat the snapshot does not have.
    let invented = forged(|h, _| h.seats[1].control = ControlRec::Ai("x".into()));
    assert!(matches!(loads(&invented), Err(SaveError::Invalid(_))));
}

#[test]
fn a_header_with_other_seats_teams_or_options_is_refused() {
    let team = forged(|h, _| h.seats[1].team = TeamId(0));
    assert!(matches!(loads(&team), Err(SaveError::Invalid(why)) if why.contains("teams")));
    let dc = forged(|h, _| h.options.directors_cut = true);
    assert!(matches!(loads(&dc), Err(SaveError::Invalid(why)) if why.contains("Director")));
    let id = forged(|h, _| h.seats[1].id = PlayerId(4));
    assert!(matches!(loads(&id), Err(SaveError::Invalid(_))));
}

/// The match option "warp allowed" (#1102) under the profile it counts for: a save keeps it,
/// and a header that says the other thing than the snapshot is refused.
#[test]
fn a_save_keeps_the_warp_option_and_a_header_that_disagrees_is_refused() {
    let content = Arc::new(ContentDb::builder().profile(sta_content::Profile::Sta2Classic).build().unwrap());
    let map = Arc::new(sta_content::MapDef::empty());
    let game = |allow_warp: bool| {
        let setup = GameSetup {
            seed: 3,
            map: Arc::clone(&map),
            content: Arc::clone(&content),
            players: vec![
                PlayerSetup { id: PlayerId(0), team: TeamId(0), race: "fx".into(), control: Control::Human },
                PlayerSetup { id: PlayerId(1), team: TeamId(1), race: "fx".into(), control: Control::Idle },
            ],
            options: GameOptions { allow_warp, ..GameOptions::default() },
        };
        let mut sim = Sim::from_setup(&setup);
        let mut replay = Replay::new(&setup);
        replay.digests.push((0, sim.digest()));
        for t in 0..10 {
            let out = sim.step(&[]);
            replay.log_step(t, &[], out.view.tick, out.view.digest, 5);
        }
        save_of(&sim, &replay, &InFlight { next_seq: 0, commands: Vec::new() }, false)
    };
    let load = |bytes: &[u8]| load(bytes, Arc::clone(&content), Some(MapSource { hash: 7, map: Arc::clone(&map) }));
    for allow in [false, true] {
        let bytes = game(allow);
        let back = load(&bytes).expect("the save loads");
        assert_eq!((back.header.options.allow_warp, back.sim.match_warp_off(), back.replay.options.allow_warp), (allow, !allow, allow));
        // the header saying the other thing than the snapshot holds
        let lied = forge(&bytes, |h, _| h.options.allow_warp = !allow);
        assert!(matches!(load(&lied), Err(SaveError::Invalid(why)) if why.contains("warp option")), "option {allow}");
    }
    // under a profile the option does not count for, the field is ignored and does not refuse
    let ignored = forged(|h, _| h.options.allow_warp = false);
    assert!(loads(&ignored).is_ok_and(|l| !l.sim.match_warp_off()));
}

#[test]
fn implausible_seat_lists_are_refused_before_anything_else_is_decoded() {
    let none = forged(|h, _| h.seats.clear());
    assert!(matches!(loads(&none), Err(SaveError::Invalid(_))));
    let nine = forged(|h, _| {
        h.seats = (0..9).map(|i| SeatRec { id: PlayerId(i), team: TeamId(i), race: "fx".into(), control: ControlRec::Idle }).collect();
    });
    assert!(matches!(loads(&nine), Err(SaveError::Invalid(_))));
    let dup = forged(|h, _| h.seats[1].id = PlayerId(0));
    assert!(matches!(loads(&dup), Err(SaveError::Invalid(_))));
    let unsorted = forged(|h, _| h.seats.reverse());
    assert!(matches!(loads(&unsorted), Err(SaveError::Invalid(_))));
    let local = forged(|h, _| h.local_seat = Some(PlayerId(6)));
    assert!(matches!(loads(&local), Err(SaveError::Invalid(_))));
    // A header packed with seats still fits its cap and is refused, not decoded further.
    let many = forged(|h, _| {
        h.seats = (0..2000u32).map(|i| SeatRec { id: PlayerId((i % 200) as u8), team: TeamId(0), race: "f".into(), control: ControlRec::Idle }).collect();
    });
    assert!(matches!(loads(&many), Err(SaveError::Invalid(_))));
}

#[test]
fn journal_and_controller_sections_that_disagree_with_the_header_are_refused() {
    let ends_early = forged(|h, _| h.tick = 0);
    assert!(matches!(loads(&ends_early), Err(SaveError::Damaged(_))));
    // A computer seat with no record.
    let missing = forged_ai(|_, p| p.controllers = postcard::to_allocvec(&Vec::<SeatController>::new()).unwrap());
    assert!(matches!(loads(&missing), Err(SaveError::Controller { seat, .. }) if seat == PlayerId(1)));
    // A record for a seat that is not a computer seat, and the wrong name.
    let extra = forged(|_, p| {
        p.controllers = postcard::to_allocvec(&vec![SeatController { seat: PlayerId(1), name: "x".into(), version: 1, data: vec![] }]).unwrap();
    });
    assert!(matches!(loads(&extra), Err(SaveError::Damaged(_))));
    let renamed = forged_ai(|_, p| {
        p.controllers = postcard::to_allocvec(&vec![SeatController { seat: PlayerId(1), name: "other".into(), version: 1, data: vec![] }]).unwrap();
    });
    assert!(matches!(loads(&renamed), Err(SaveError::Damaged(_))));
    let twice = forged_ai(|_, p| {
        let r = SeatController { seat: PlayerId(1), name: "stateless".into(), version: 1, data: vec![] };
        p.controllers = postcard::to_allocvec(&vec![r.clone(), r]).unwrap();
    });
    assert!(matches!(loads(&twice), Err(SaveError::Damaged(_))));
    let journal = forged(|_, p| p.journal.push(0));
    assert!(matches!(loads(&journal), Err(SaveError::Damaged(_))));
    let in_flight = forged(|_, p| p.in_flight.push(0));
    assert!(matches!(loads(&in_flight), Err(SaveError::Damaged(_))));
}

// ----------------------------------------------------- M1: bounds on input

/// A count that would ask for far more than the file holds is refused from
/// its own bytes, before the list is decoded: by construction the hostile
/// files below are a few dozen bytes and name 2^40 elements.
#[test]
fn counts_are_refused_before_anything_is_allocated_from_them() {
    let huge = postcard::to_allocvec(&(1u64 << 40)).unwrap();
    let controllers = forged(|_, p| p.controllers = huge.clone());
    let e = loads(&controllers).err().unwrap();
    assert!(e.to_string().contains("too many controller records"), "{e}");
    let mut hostile = postcard::to_allocvec(&1u32).unwrap();
    hostile.extend_from_slice(&huge);
    let in_flight = forged(|_, p| p.in_flight = hostile.clone());
    assert!(loads(&in_flight).err().unwrap().to_string().contains("too many commands in flight"));
    let tick = 40u64;
    let mut digests = postcard::to_allocvec(&tick).unwrap();
    digests.extend_from_slice(&huge);
    let digests = forged(|_, p| p.journal = digests.clone());
    assert!(loads(&digests).err().unwrap().to_string().contains("too many journal digests"));
    let mut commands = postcard::to_allocvec(&tick).unwrap();
    commands.push(0); // no digests
    commands.extend_from_slice(&huge);
    let commands = forged(|_, p| p.journal = commands.clone());
    assert!(loads(&commands).err().unwrap().to_string().contains("too many journal commands"));
    // One more than the seat count is too many, too.
    let nine = forged(|_, p| {
        let r = |i| SeatController { seat: PlayerId(i), name: "x".into(), version: 1, data: vec![] };
        p.controllers = postcard::to_allocvec(&(0..9).map(r).collect::<Vec<_>>()).unwrap();
    });
    assert!(loads(&nine).err().unwrap().to_string().contains("too many controller records"));
    // The same bytes, undamaged and just within the bound, are not refused for their count.
    assert!(!matches!(loads(&forged(|_, p| p.in_flight = postcard::to_allocvec(&InFlight::default()).unwrap())), Err(SaveError::Damaged(_))));
}

#[test]
fn a_file_over_the_size_cap_is_refused_whole() {
    let mut big = a_save();
    big.resize(MAX_FILE + 1, 0);
    let e = loads(&big).err().unwrap();
    assert!(matches!(&e, SaveError::Damaged(why) if why.contains("larger")), "{e:?}");
}

#[test]
fn a_huge_declared_length_is_refused_without_allocating_it() {
    let mut b = a_save();
    b[14..18].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(read_header(&b), Err(SaveError::Damaged(_))));
    assert!(matches!(loads(&b), Err(SaveError::Damaged(_))));
}

// ----------------------------------------------- damage that keeps checksums

/// Damage that keeps both checksums valid reaches the parsers behind them:
/// the container never panics, whatever it decides.
#[test]
fn resealed_damage_never_panics() {
    let bytes = a_save_with_ai();
    let mut refused = 0;
    let mut total = 0;
    for i in 0..bytes.len() - 8 {
        // Every header byte, then a spread of the rest.
        if i > 700 && i % 7 != 0 {
            continue;
        }
        for mask in [0x01u8, 0x80, 0xff] {
            let mut b = bytes.clone();
            b[i] ^= mask;
            let b = reseal(b);
            total += 1;
            if loads(&b).is_err() {
                refused += 1;
            }
        }
    }
    assert!(refused * 2 > total, "only {refused} of {total} damaged files were refused");
}

#[test]
fn resealed_cuts_never_panic() {
    let bytes = a_save();
    for n in 0..bytes.len() {
        let b = reseal(bytes[..n].to_vec());
        assert!(loads(&b).is_err(), "a file cut to {n} bytes loads");
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Seeded random mutation of a valid save (with units, a computer seat and
/// events) with the checksums repaired, aimed at the section boundaries and
/// at the insides of the sections, so the snapshot, journal, events and
/// controller decoders see damaged input; a file that still loads is stepped
/// twice. Fixed seed, bounded count: any panic here is a finding.
#[test]
fn mutated_valid_saves_do_not_panic_in_the_decoders() {
    let (sim, replay, in_flight) = a_game(true, 60);
    let bytes = save_of(&sim, &replay, &in_flight, true);
    let (_, body_at) = container::parse_header(&bytes).unwrap();
    // Section boundaries (offsets of each section's head).
    let mut bounds = vec![body_at];
    let mut at = body_at;
    for _ in 0..5 {
        let len = u32::from_le_bytes([bytes[at + 5], bytes[at + 6], bytes[at + 7], bytes[at + 8]]) as usize;
        at += 9 + len;
        bounds.push(at);
    }
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let (mut loaded, mut panics) = (0, Vec::new());
    for round in 0..2500 {
        let mut b = bytes.clone();
        let pos = match round % 3 {
            // Near a boundary.
            0 => (bounds[rng.below(bounds.len())] + rng.below(24)).saturating_sub(12).min(b.len() - 9),
            // Anywhere in the body.
            _ => body_at + rng.below(b.len() - body_at - 8),
        };
        match rng.below(5) {
            0 => b[pos] ^= 1 << rng.below(8),
            1 => b[pos] = rng.next() as u8,
            2 => {
                for k in 0..=rng.below(8) {
                    if pos + k < b.len() - 8 {
                        b[pos + k] = rng.next() as u8;
                    }
                }
            }
            3 => {
                let n = (rng.below(12) + 1).min(b.len() - pos - 8);
                b.drain(pos..pos + n);
            }
            _ => {
                let junk: Vec<u8> = (0..=rng.below(12)).map(|_| rng.next() as u8).collect();
                b.splice(pos..pos, junk);
            }
        }
        let b = reseal(b);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Ok(mut l) = loads(&b) {
                l.sim.step(&[]);
                l.sim.step(&[]);
                true
            } else {
                false
            }
        }));
        match outcome {
            Ok(true) => loaded += 1,
            Ok(false) => {}
            Err(_) => panics.push(round),
        }
    }
    assert!(panics.is_empty(), "panics at rounds {panics:?} ({loaded} mutated files still loaded)");
}
