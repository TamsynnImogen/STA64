//! The match option "warp allowed" (#1102): with `GameOptions::allow_warp` off no craft warps in
//! a stock Armada II match, and a script cannot turn warp back on. On the approach scene's ships
//! (`approach_tests::db`); nothing here is game data.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, Profile};
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, TeamId, Vec3};

use crate::command::{Command, CommandKind, Edit, MissionSwitch, Order};
use crate::movement::approach_tests;
use crate::replay::{Recorder, Replay, ReplayError};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::snapshot::{PRE_WARP_OPTION_FORMAT, SnapshotError};
use crate::{Sim, WorldSource};

const HZ: u32 = TICK_HZ;
const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}
fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: Fixed::ZERO, z: n(z) }
}

/// The approach scene's content under `profile`; `content_warp` false is content that switches
/// warp off itself.
fn db(profile: Profile, content_warp: bool) -> Arc<ContentDb> {
    let base = approach_tests::db(profile, Some(1100));
    let rules = base.rules().clone().with_warp((!content_warp).then_some(false));
    Arc::new(base.with_rules(rules))
}

fn options(allow_warp: bool) -> GameOptions {
    GameOptions { allow_warp, ..GameOptions::default() }
}

fn setup(db: &Arc<ContentDb>, options: GameOptions) -> GameSetup {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "st".into(), control: Control::Idle };
    GameSetup { seed: 5, map: Arc::new(MapDef::empty()), content: Arc::clone(db), players: vec![seat(1), seat(2)], options }
}

fn game(db: &Arc<ContentDb>, allow_warp: bool) -> Sim {
    Sim::from_setup(&setup(db, options(allow_warp)))
}

fn spawn(sim: &mut Sim, odf: &str, player: u8, at: Vec3) -> EntityId {
    let db = Arc::clone(sim.content.as_ref().unwrap());
    sim.spawn_def(db.key(odf).unwrap(), PlayerId(player), at, AHEAD).unwrap()
}

fn by(player: PlayerId, o: Order) -> Command {
    Command { player, seq: 0, kind: CommandKind::Order(o) }
}
fn warp_move(s: EntityId) -> Command {
    by(PlayerId(1), Order::MoveWarp { units: vec![s], to: v(0, 6000), queue: false })
}

/// Whether the craft was ever at warp (or entering it) in eight seconds after `first`, its top
/// speed, and how far it got.
fn fly(sim: &mut Sim, first: Vec<Command>, id: EntityId) -> (bool, Fixed, Fixed) {
    let (mut warped, mut top) = (false, Fixed::ZERO);
    for i in 0..8 * HZ {
        sim.step(if i == 0 { &first } else { &[] });
        warped |= sim.approach_warp.contains(id);
        top = top.max(sim.unit(id).unwrap().speed);
    }
    (warped, top, sim.unit(id).unwrap().pos.z)
}

/// A craft given the order `order` builds: with the option on it warps, with it off it carries
/// the same order out at impulse.
fn warps_only_with_the_option(what: &str, order: impl Fn(&mut Sim, EntityId) -> Vec<Command>) {
    let db = db(Profile::Sta2Classic, true);
    for allow in [true, false] {
        let mut sim = game(&db, allow);
        assert_eq!(sim.match_warp_off(), !allow);
        let s = spawn(&mut sim, "st_seer", 1, v(0, 0));
        let impulse = sim.unit(s).unwrap().stats.max_speed;
        let cmds = order(&mut sim, s);
        let (warped, top, z) = fly(&mut sim, cmds, s);
        if allow {
            assert!(warped && top > impulse, "{what}: with the option on the path warps");
        } else {
            assert!(!warped && top <= impulse, "{what}: warp with the option off ({top:?})");
            assert!(z > n(500), "{what}: the order is still carried out at impulse (got to {z:?})");
        }
    }
}

#[test]
fn the_default_is_on_and_only_stock_armada_ii_content_with_warp_is_offered_the_option() {
    assert!(GameOptions::default().allow_warp, "warp is allowed unless the players switch it off");
    assert!(GameOptions::warp_offered(&db(Profile::Sta2Classic, true)));
    assert!(!GameOptions::warp_offered(&db(Profile::Sta2Classic, false)), "content without warp: nothing to choose");
    for profile in [Profile::Native, Profile::Fo4, Profile::Fo3, Profile::Sta1Classic] {
        assert!(!GameOptions::warp_offered(&db(profile, true)), "{profile:?}");
    }
    // a form of the options written before the field existed reads as on
    let old: GameOptions = json5::from_str(r#"{ starting_resources: "Normal", officer_limit: 0, directors_cut: false }"#).expect("the options without the field");
    assert!(old.allow_warp);
    let off: GameOptions = json5::from_str(r#"{ starting_resources: "Normal", officer_limit: 0, allow_warp: false }"#).unwrap();
    assert!(!off.allow_warp && !off.directors_cut);
}

/// The view carries the option, derived each tick, so a client can keep its offer honest: off only in a stock
/// Armada II match that has it off, and never outside the profile the option counts for.
#[test]
fn the_view_shows_the_match_option_where_it_counts() {
    for (profile, allow, shown_off) in [(Profile::Sta2Classic, true, false), (Profile::Sta2Classic, false, true), (Profile::Fo4, false, false), (Profile::Native, false, false)] {
        let db = db(profile, true);
        let sim = game(&db, allow);
        assert_eq!(sim.view().attrs.match_warp_off, shown_off, "{profile:?} allow_warp {allow}");
        assert_eq!(sim.view().attrs.match_warp_off, sim.match_warp_off());
    }
}

#[test]
fn with_the_option_off_a_warp_move_flies_at_impulse() {
    warps_only_with_the_option("a player's warp move", |_, s| vec![warp_move(s)]);
    warps_only_with_the_option("a script's warp move", |_, s| vec![by(PlayerId::SCRIPT, Order::MoveWarp { units: vec![s], to: v(0, 6000), queue: false })]);
}

#[test]
fn with_the_option_off_an_attack_approach_does_not_warp() {
    warps_only_with_the_option("an attack approach", |sim, s| {
        let t = spawn(sim, "st_target", 2, v(0, 6000));
        vec![by(PlayerId(1), Order::Attack { units: vec![s], target: t })]
    });
}

/// The option, the mission switch and the content rule must all allow warp: a script that sets
/// the mission switch cannot bring warp back in a match that has it off, and can still switch
/// it off in a match that has it on.
#[test]
fn a_script_cannot_turn_warp_back_on_in_a_match_without_it() {
    let allow = |on: bool| Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(Edit::MissionSwitch(MissionSwitch::AllowWarp, on)) };
    let db = db(Profile::Sta2Classic, true);
    let run = |option: bool, switch: bool| {
        let mut sim = game(&db, option);
        let s = spawn(&mut sim, "st_seer", 1, v(0, 0));
        sim.step(&[allow(switch)]);
        fly(&mut sim, vec![warp_move(s)], s).0
    };
    assert!(run(true, true));
    assert!(!run(true, false), "the mission switch still switches warp off");
    assert!(!run(false, true), "the script's switch does not bring warp back");
    assert!(!run(false, false));
}

#[test]
fn content_without_warp_has_none_whatever_the_option_says() {
    let db = db(Profile::Sta2Classic, false);
    for allow in [true, false] {
        let mut sim = game(&db, allow);
        let s = spawn(&mut sim, "st_seer", 1, v(0, 0));
        assert!(!fly(&mut sim, vec![warp_move(s)], s).0, "option {allow}");
    }
}

/// With the option on nothing is hashed, so a match keeps every digest it had before the
/// option existed; off, a marker is, from the first tick on.
#[test]
fn the_digest_is_unchanged_with_the_option_on_and_differs_with_it_off() {
    let db = db(Profile::Sta2Classic, true);
    let run = |sim: &mut Sim| {
        let s = spawn(sim, "st_seer", 1, v(0, 0));
        let mut digests = vec![sim.digest()];
        for i in 0..2 * HZ {
            // a plain move: the same flight with and without warp allowed
            let first = [by(PlayerId(1), Order::Move { units: vec![s], to: v(0, 300), queue: false, mode: None })];
            sim.step(if i == 0 { &first } else { &[] });
            digests.push(sim.digest());
        }
        (digests, sim.unit(s).unwrap().pos)
    };
    let (on, at_on) = run(&mut game(&db, true));
    let (by_default, _) = run(&mut Sim::from_setup(&setup(&db, GameOptions::default())));
    assert_eq!(on, by_default);
    let (off, at_off) = run(&mut game(&db, false));
    assert_eq!(at_on, at_off, "the same flight");
    assert!(on.iter().zip(&off).all(|(a, b)| a != b), "the option is part of the state on every tick");
    // and the marker is in the section the option belongs to
    assert_eq!(game(&db, false).section_digests().differing(&game(&db, true).section_digests()), ["movement"]);
}

/// The option counts under stock Armada II only: every other profile ignores it, as the other
/// per-game option is ignored outside its game.
#[test]
fn other_profiles_ignore_the_option() {
    for profile in [Profile::Native, Profile::Fo4, Profile::Fo3, Profile::Sta1Classic] {
        let db = db(profile, true);
        let run = |allow: bool| {
            let mut sim = game(&db, allow);
            assert!(!sim.match_warp_off(), "{profile:?}");
            let s = spawn(&mut sim, "st_seer", 1, v(0, 0));
            let flight = fly(&mut sim, vec![warp_move(s)], s);
            (flight, sim.digest())
        };
        assert_eq!(run(false), run(true), "{profile:?}: the same match with the field off");
    }
}

#[test]
fn a_snapshot_keeps_the_option_and_one_from_before_it_loads_with_warp_allowed() {
    let db = db(Profile::Sta2Classic, true);
    let mut sim = game(&db, false);
    let s = spawn(&mut sim, "st_seer", 1, v(0, 0));
    sim.step(&[warp_move(s)]);
    let bytes = sim.snapshot();
    assert_eq!(bytes[bytes.len() - 3], 0, "the option's byte, before the empty flights record: off");
    let mut back = Sim::from_snapshot(&bytes, Some(Arc::clone(&db))).unwrap();
    assert!(back.match_warp_off());
    assert_eq!((back.digest(), back.snapshot()), (sim.digest(), bytes.clone()));
    assert!(!fly(&mut back, Vec::new(), s).0, "still no warp after the load");

    // the format before the option is the same bytes without it: warp is allowed there
    let on = game(&db, true);
    let current = on.snapshot();
    assert_eq!(current[current.len() - 3], 1);
    let mut old = current[..current.len() - 3].to_vec();
    old[4..6].copy_from_slice(&PRE_WARP_OPTION_FORMAT.to_le_bytes());
    let loaded = Sim::from_snapshot(&old, Some(Arc::clone(&db))).expect("the previous format loads");
    assert!(!loaded.match_warp_off());
    assert_eq!((loaded.digest(), loaded.snapshot()), (on.digest(), current.clone()), "and is written back in the current format");

    // the byte is mandatory in its format, the reserved number between the two is no format,
    // and the option cannot be off for content it does not count for
    assert!(Sim::from_snapshot(&current[..current.len() - 3], Some(Arc::clone(&db))).is_err());
    let mut reserved = current.clone();
    reserved[4..6].copy_from_slice(&57u16.to_le_bytes());
    assert_eq!(Sim::from_snapshot(&reserved, Some(Arc::clone(&db))).err(), Some(SnapshotError::FormatVersion { found: 57 }));
    let other = self::db(Profile::Fo4, true);
    let mut forged = game(&other, true).snapshot();
    let at = forged.len() - 3;
    forged[at] = 0;
    match Sim::from_snapshot(&forged, Some(other)) {
        Err(SnapshotError::Corrupt(why)) => assert!(why.contains("warp option"), "{why}"),
        other => panic!("{:?}", other.map(|_| "loaded")),
    }
}

#[test]
fn a_replay_records_the_option_and_plays_back_and_the_format_before_it_is_refused() {
    let db = db(Profile::Sta2Classic, true);
    let record = |allow: bool| {
        let mut rec = Recorder::with_interval(&setup(&db, options(allow)), 5);
        let mut s = None;
        for i in 0..4 * HZ {
            let cmds = match i {
                0 => vec![Command { player: PlayerId::BENCH, seq: 0, kind: CommandKind::Edit(Edit::Spawn { odf: db.key("st_seer").unwrap(), player: PlayerId(1), pos: v(0, 0), heading: AHEAD }) }],
                1 => {
                    s = rec.sim().view().units.first().map(|u| u.id);
                    vec![warp_move(s.expect("the craft was spawned"))]
                }
                _ => Vec::new(),
            };
            rec.step(&cmds);
        }
        let warped = rec.sim().approach_warp.contains(s.unwrap());
        (rec.finish(), warped)
    };
    let (off, warped_off) = record(false);
    let (on, warped_on) = record(true);
    assert!(warped_on && !warped_off);
    assert!(!off.options.allow_warp && on.options.allow_warp);

    let bytes = off.encode();
    let back = Replay::decode(&bytes).expect("the replay decodes");
    assert!(!back.options.allow_warp, "the option is in the file");
    let played = back.play(Arc::clone(&db)).expect("it plays back to the recorded digests");
    assert!(played.match_warp_off());
    // played with the option the other way, the digests no longer agree
    let mut wrong = back.clone();
    wrong.options.allow_warp = true;
    assert!(wrong.play(Arc::clone(&db)).is_err());

    // a replay of the format before the option is refused by its number, not read as damaged
    let mut old = bytes.clone();
    let previous = crate::replay::FORMAT_VERSION - 1;
    old[4..6].copy_from_slice(&previous.to_le_bytes());
    let e = Replay::decode(&old).unwrap_err();
    assert_eq!(e, ReplayError::FormatVersion { found: previous });
    assert!(e.to_string().contains("older"), "{e}");
}
