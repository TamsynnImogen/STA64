//! The move mode of `Order::Move` (SIM-76 seam, ADR 0011 G4): the order carries
//! it, the sim keeps what the last move order of each unit named, and the mode
//! survives the queue, a snapshot, a replay and the wire encoding. Nothing
//! steers by it yet, so a run with a mode named moves exactly like one without.
//!
//! Authored data only: the fake content with its authored formations
//! (`sta_content::fake::db_with_formations`) and synthetic maps. No game files
//! are read. Evidence for the two modes themselves: read from the executable,
//! static, not measured (TOOL-26).

use std::sync::Arc;

use sta_content::{ContentDb, GroupRules, MapDef, Profile};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, MoveMode, Order};
use sta_sim::replay::{Recorder, Replay};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::snapshot::SnapshotError;
use sta_sim::{Sim, WorldSource};

fn content() -> Arc<ContentDb> {
    Arc::new(sta_content::fake::db_with_formations())
}

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn seat(id: u8) -> PlayerSetup {
    PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Human }
}

fn setup(map: MapDef) -> GameSetup {
    GameSetup { seed: 5, map: Arc::new(map), content: content(), players: vec![seat(0), seat(1)], options: GameOptions::default() }
}

/// An empty map with three cruisers of the first seat.
fn fleet() -> (Sim, [EntityId; 3]) {
    let mut sim = Sim::from_setup(&setup(MapDef::empty()));
    let key = sim.content().expect("content").key("fx_cruiser").expect("authored unit");
    let at = |x| (v(x, 0), (Fixed::ZERO, Fixed::ONE));
    let ids = [0, 200, 400].map(|x| {
        let (p, h) = at(x);
        sim.spawn_def(key, PlayerId(0), p, h).expect("spawns")
    });
    (sim, ids)
}

fn go(seq: u32, units: &[EntityId], to: Vec3, queue: bool, mode: Option<MoveMode>) -> Command {
    Command { player: PlayerId(0), seq, kind: CommandKind::Order(Order::Move { units: units.to_vec(), to, queue, mode }) }
}

fn modes(sim: &Sim) -> Vec<(EntityId, MoveMode)> {
    sim.view().attrs.move_modes.clone()
}

fn run(sim: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        sim.step(&[]);
    }
}

#[test]
fn an_order_issued_with_each_mode_keeps_it_through_the_queue_and_a_stop() {
    let (mut sim, [a, b, c]) = fleet();
    sim.step(&[
        go(0, &[a], v(3000, 0), false, Some(MoveMode::Formation)),
        go(1, &[b], v(3000, 400), false, Some(MoveMode::Group)),
        go(2, &[c], v(3000, 800), false, None),
    ]);
    assert_eq!(modes(&sim), vec![(a, MoveMode::Formation), (b, MoveMode::Group)], "only named modes are rows");
    // The unit that named none runs under the content's default for its profile.
    let rules = sim.content().unwrap().group_rules();
    assert_eq!(sim.move_mode(c), Some(MoveMode::resolve(None, &rules)));
    assert_eq!(sim.move_mode(a), Some(MoveMode::Formation));
    assert_eq!(sim.move_mode(b), Some(MoveMode::Group));

    // A queued move names its own mode: the mode of the last move order.
    sim.step(&[go(3, &[a], v(3000, 2000), true, Some(MoveMode::Group))]);
    assert_eq!(sim.move_mode(a), Some(MoveMode::Group));
    // A move that names none takes the unit's row away: the content's default again.
    sim.step(&[go(4, &[b], v(-3000, 0), false, None)]);
    assert!(!modes(&sim).iter().any(|(id, _)| *id == b));
    assert_eq!(sim.move_mode(b), Some(MoveMode::resolve(None, &rules)));
    // Stop clears the row and the unit has nothing to fly to.
    sim.step(&[Command { player: PlayerId(0), seq: 5, kind: CommandKind::Order(Order::Stop { units: vec![a, b, c] }) }]);
    assert!(modes(&sim).is_empty());
    assert_eq!(sim.move_mode(a), None);
}

#[test]
fn a_unit_that_is_gone_leaves_no_row() {
    let (mut sim, [a, b, _]) = fleet();
    sim.step(&[go(0, &[a, b], v(3000, 0), false, Some(MoveMode::Formation))]);
    assert_eq!(modes(&sim).len(), 2);
    assert!(sim.despawn(a, None));
    sim.step(&[]);
    assert_eq!(modes(&sim), vec![(b, MoveMode::Formation)]);
}

#[test]
fn a_named_mode_changes_no_movement_only_the_digest() {
    let moves = |mode| {
        let (mut sim, [a, b, c]) = fleet();
        sim.step(&[go(0, &[a, b, c], v(2500, 600), false, mode)]);
        run(&mut sim, 240);
        let at: Vec<_> = sim.view().units.iter().map(|u| (u.id, u.pos, u.speed)).collect();
        (at, sim.digest())
    };
    let (plain, plain_digest) = moves(None);
    let (formation, formation_digest) = moves(Some(MoveMode::Formation));
    let (group, group_digest) = moves(Some(MoveMode::Group));
    assert_eq!(plain, formation, "no behaviour change: positions and speeds are the same");
    assert_eq!(plain, group);
    assert_ne!(plain_digest, formation_digest, "the mode is state, so it is in the digest");
    assert_ne!(formation_digest, group_digest);
}

#[test]
fn the_modes_round_trip_through_a_snapshot_and_the_run_continues_on_the_same_digests() {
    let (mut sim, [a, b, c]) = fleet();
    sim.step(&[
        go(0, &[a], v(3000, 0), false, Some(MoveMode::Formation)),
        go(1, &[b], v(3000, 400), false, Some(MoveMode::Group)),
        go(2, &[c], v(3000, 800), false, None),
        go(3, &[a], v(3000, 1200), true, Some(MoveMode::Formation)),
    ]);
    run(&mut sim, 30);
    let bytes = sim.snapshot();
    let mut back = Sim::from_snapshot(&bytes, sim.content().cloned()).expect("loads");
    assert_eq!(back.digest(), sim.digest());
    assert_eq!(modes(&back), modes(&sim));
    assert_eq!(modes(&back), vec![(a, MoveMode::Formation), (b, MoveMode::Group)]);
    assert_eq!(back.snapshot(), bytes, "save, load, save");
    for tick in 0..120 {
        let (x, y) = (sim.step(&[]), back.step(&[]));
        assert_eq!(x.view.digest, y.view.digest, "tick {tick}");
    }
    assert_eq!(back.snapshot(), sim.snapshot());
}

#[test]
fn older_snapshots_load_with_every_order_on_the_content_default() {
    let (mut sim, [a, ..]) = fleet();
    sim.step(&[go(0, &[a], v(3000, 0), false, None)]);
    let bytes = sim.snapshot();
    // Format 51 is the same chain without the final move-mode record (two bytes when it has no rows).
    let mut old = bytes[..bytes.len() - 2].to_vec();
    old[4..6].copy_from_slice(&sta_sim::snapshot::PRE_MOVE_MODE_FORMAT.to_le_bytes());
    let back = Sim::from_snapshot(&old, sim.content().cloned()).expect("the previous format loads");
    assert!(modes(&back).is_empty());
    assert_eq!(back.digest(), sim.digest());
    assert_eq!(back.snapshot(), bytes);
}

#[test]
fn a_mode_row_for_a_unit_that_does_not_exist_is_a_corrupt_snapshot() {
    let (sim, _) = fleet();
    let bytes = sim.snapshot();
    // Replace the empty record by one row: version 1, one row, unit 999 (varint), mode `Group` (tag 1).
    let mut forged = bytes[..bytes.len() - 2].to_vec();
    forged.extend_from_slice(&[1, 1, 0xE7, 0x07, 1]);
    assert!(matches!(Sim::from_snapshot(&forged, sim.content().cloned()), Err(SnapshotError::Corrupt(_))));
}

#[test]
fn the_default_mode_follows_the_profile_s_rule() {
    // Read from data per profile (`cfgDEFAULT_TO_FORMATION_MOVE`, `DEFAULT_TO_FORMATION_MOVE` in Armada I):
    // the profile defaults are CON-59's; the order's own mode always wins.
    for profile in [Profile::Sta2Classic, Profile::Fo3, Profile::Sta1Classic] {
        let rules = GroupRules::defaults(profile);
        let default = MoveMode::resolve(None, &rules);
        assert_eq!(default == MoveMode::Formation, rules.default_to_formation_move, "{profile:?}");
        assert_eq!(MoveMode::resolve(Some(MoveMode::Group), &rules), MoveMode::Group);
        assert_eq!(MoveMode::resolve(Some(MoveMode::Formation), &rules), MoveMode::Formation);
    }
    assert_eq!(MoveMode::resolve(None, &GroupRules::defaults(Profile::Sta2Classic)), MoveMode::Formation);
    assert_eq!(MoveMode::resolve(None, &GroupRules::defaults(Profile::Fo3)), MoveMode::Group);
    assert_eq!(MoveMode::resolve(None, &GroupRules::defaults(Profile::Sta1Classic)), MoveMode::Group);
}

#[test]
fn the_wire_encoding_carries_the_mode_and_none_is_one_zero_byte() {
    let wire = |mode| {
        let c = go(7, &[EntityId(4)], v(1, 2), true, mode);
        let bytes = postcard::to_allocvec(&c).unwrap();
        assert_eq!(postcard::from_bytes::<Command>(&bytes).unwrap(), c);
        bytes
    };
    let none = wire(None);
    let formation = wire(Some(MoveMode::Formation));
    let group = wire(Some(MoveMode::Group));
    // The mode is the last field of the order: `None` is the byte 0, `Some(mode)` is 1 and the variant tag.
    assert_eq!(none.last(), Some(&0));
    assert_eq!(formation[formation.len() - 2..], [1, 0]);
    assert_eq!(group[group.len() - 2..], [1, 1]);
    assert_eq!(formation.len(), none.len() + 1);
    // The older tags keep their values: Move is still variant 0.
    assert_eq!(postcard::to_allocvec(&Order::move_to(vec![EntityId(1)], v(0, 0), false)).unwrap()[0], 0);
}

/// The offset of `needle` in `hay`, which must contain it once.
fn find(hay: &[u8], needle: &[u8]) -> usize {
    let at: Vec<usize> = hay.windows(needle.len()).enumerate().filter(|(_, w)| *w == needle).map(|(i, _)| i).collect();
    assert_eq!(at.len(), 1, "the command's bytes appear once");
    at[0]
}

#[test]
fn an_unknown_mode_value_is_refused_by_the_decoders() {
    // A command: the mode is the last two bytes, `Some` tag then the variant index.
    let c = go(7, &[EntityId(4)], v(1, 2), true, Some(MoveMode::Group));
    let bytes = postcard::to_allocvec(&c).unwrap();
    for (from_end, bad) in [(1, 2u8), (2, 2u8)] {
        let mut forged = bytes.clone();
        let at = forged.len() - from_end;
        forged[at] = bad;
        assert!(postcard::from_bytes::<Command>(&forged).is_err(), "byte {from_end} from the end set to {bad}");
    }
    // A snapshot record naming a mode that does not exist.
    let (sim, _) = fleet();
    let snap = sim.snapshot();
    let mut forged = snap[..snap.len() - 2].to_vec();
    forged.extend_from_slice(&[1, 1, 1, 2]);
    assert!(matches!(Sim::from_snapshot(&forged, sim.content().cloned()), Err(SnapshotError::Corrupt(_))));
    // A replay whose logged move order carries one.
    let mut rec = Recorder::with_interval(&setup(MapDef::empty()), 10);
    rec.step(&[go(0, &[EntityId(1)], v(5, 5), false, Some(MoveMode::Group))]);
    let replay = rec.finish();
    let command = postcard::to_allocvec(&replay.commands[0].command).unwrap();
    let mut file = replay.encode();
    assert!(Replay::decode(&file).is_ok());
    let end = find(&file, &command) + command.len() - 1;
    file[end] = 2;
    assert!(Replay::decode(&file).is_err());
}

#[test]
fn a_replay_with_named_modes_plays_back_to_the_same_digests() {
    let setup = self::setup(MapDef::empty());
    let mut rec = Recorder::with_interval(&setup, 10);
    // Two cruisers arrive by a mission's spawn edit, which is in the command log like any command.
    let cruiser = setup.content.key("fx_cruiser").expect("authored unit");
    let spawn = |seq, x| Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::SpawnRequest { request: u64::from(seq), odf: cruiser, player: PlayerId(0), pos: v(x, 0), heading: (Fixed::ZERO, Fixed::ONE) }),
    };
    rec.step(&[spawn(0, 0), spawn(1, 300)]);
    rec.step(&[]);
    let mine: Vec<EntityId> = rec.sim().view().units.iter().filter(|u| u.owner == PlayerId(0)).map(|u| u.id).collect();
    assert_eq!(mine.len(), 2);
    rec.step(&[go(0, &mine, v(900, 400), false, Some(MoveMode::Group))]);
    rec.step(&[go(1, &mine, v(-300, 100), true, Some(MoveMode::Formation))]);
    for _ in 0..90 {
        rec.step(&[]);
    }
    let expected = modes(rec.sim());
    assert!(!expected.is_empty(), "some unit took the move");
    let replay = Replay::decode(&rec.finish().encode()).expect("decodes");
    let played = replay.play(setup.content.clone()).expect("every recorded digest matches");
    assert_eq!(modes(&played), expected);
}

#[test]
fn replay_formats_before_the_mode_are_refused() {
    let setup = setup(MapDef::empty());
    let replay = Recorder::with_interval(&setup, 10).finish();
    for old in [6u16, 7] {
        let mut bytes = replay.encode();
        bytes[4..6].copy_from_slice(&old.to_le_bytes());
        assert!(Replay::decode(&bytes).is_err(), "format {old}");
    }
}
