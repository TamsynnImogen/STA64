//! The content-level warp switch (#1102): with `Rules::warp_enabled` off, no craft enters warp
//! by any path, and a script cannot turn warp back on. On the approach scene's ships
//! (`approach_tests::db`: impulse 140, warp 400) and one authored craft with the per-unit warp
//! distance key; the switch is set in code, as an authored header would set it.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, Profile};
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, TeamId, Vec3};

use crate::command::{Command, CommandKind, Edit, MissionSwitch, Order};
use crate::movement::approach::IN_WARP;
use crate::movement::approach_tests;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

const HZ: u32 = TICK_HZ;
const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}
fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: Fixed::ZERO, z: n(z) }
}

/// The approach scene's content under `profile` with the warp switch as given (`None`: a header
/// without the key), plus `st_keyed`: the same ship with the per-unit warp distance key, whose
/// plain moves warp by themselves.
fn db(profile: Profile, warp: Option<bool>) -> Arc<ContentDb> {
    let base = approach_tests::db(profile, Some(1100));
    let mut b = ContentDb::builder().profile(base.profile()).features(base.features().clone());
    for w in base.weapons() {
        b = b.weapon(w.clone());
    }
    for u in base.units() {
        b = b.unit(u.clone());
    }
    let mut keyed = base.units().find(|u| u.odf == "st_seer").unwrap().clone();
    keyed.odf = "st_keyed".into();
    keyed.movement.too_close_for_warp = n(600);
    Arc::new(b.unit(keyed).build().unwrap().with_rules(base.rules().clone().with_warp(warp)))
}

fn game(db: &Arc<ContentDb>) -> Sim {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "st".into(), control: Control::Idle };
    Sim::from_setup(&GameSetup { seed: 5, map: Arc::new(MapDef::empty()), content: Arc::clone(db), players: vec![seat(1), seat(2)], options: GameOptions::default() })
}

fn spawn(sim: &mut Sim, db: &ContentDb, odf: &str, player: u8, at: Vec3) -> EntityId {
    sim.spawn_def(db.key(odf).unwrap(), PlayerId(player), at, AHEAD).unwrap()
}

fn by(player: PlayerId, o: Order) -> Command {
    Command { player, seq: 0, kind: CommandKind::Order(o) }
}

/// What a craft did over `ticks` after `first`: whether it was ever in the warp phase, its top
/// speed, and how far it got along z.
struct Flight {
    warped: bool,
    top: Fixed,
    z: Fixed,
}

fn fly(sim: &mut Sim, first: Vec<Command>, id: EntityId, ticks: u32) -> Flight {
    let mut f = Flight { warped: false, top: Fixed::ZERO, z: Fixed::ZERO };
    for i in 0..ticks {
        sim.step(if i == 0 { &first } else { &[] });
        let u = sim.unit(id).unwrap();
        f.warped |= sim.approach_warp.phase(id) == Some(IN_WARP);
        f.top = f.top.max(u.speed);
        f.z = u.pos.z;
    }
    f
}

/// One craft of `odf` at the origin given the order `order` builds for it, flown for eight
/// seconds; `prepare` may add units and return commands to send first.
fn flight(db: &Arc<ContentDb>, odf: &str, order: impl Fn(&mut Sim, EntityId) -> Vec<Command>) -> (Flight, Fixed) {
    let mut sim = game(db);
    let s = spawn(&mut sim, db, odf, 1, v(0, 0));
    let impulse = sim.unit(s).unwrap().stats.max_speed;
    let cmds = order(&mut sim, s);
    (fly(&mut sim, cmds, s, 8 * HZ), impulse)
}

/// With the switch on (or the key absent) the path warps; with it off the craft flies the same
/// order at impulse speed: it still moves, it never enters the warp phase.
fn warps_only_with_the_switch(profile: Profile, odf: &str, what: &str, order: impl Fn(&mut Sim, EntityId) -> Vec<Command> + Copy) {
    for on in [None, Some(true)] {
        let (f, impulse) = flight(&db(profile, on), odf, order);
        assert!(f.warped && f.top > impulse, "{what}, switch {on:?}: the path warps ({:?} top)", f.top);
    }
    let (f, impulse) = flight(&db(profile, Some(false)), odf, order);
    assert!(!f.warped, "{what}: in the warp phase with warp switched off");
    assert!(f.top <= impulse, "{what}: {:?} is above impulse {impulse:?}", f.top);
    assert!(f.z > n(500), "{what}: the craft still carries its order out at impulse (got to {:?})", f.z);
}

#[test]
fn the_default_is_on_and_the_switch_is_part_of_the_content_digest() {
    let (absent, on, off) = (db(Profile::Sta2Classic, None), db(Profile::Sta2Classic, Some(true)), db(Profile::Sta2Classic, Some(false)));
    assert!(absent.rules().warp_enabled() && on.rules().warp_enabled() && !off.rules().warp_enabled());
    assert_eq!(absent.hash(), on.hash(), "restating the default is no other content");
    assert_ne!(absent.hash(), off.hash());
}

#[test]
fn a_warp_move_flies_at_impulse_when_the_content_has_no_warp() {
    warps_only_with_the_switch(Profile::Sta2Classic, "st_seer", "a player's warp move", |_, s| vec![by(PlayerId(1), Order::MoveWarp { units: vec![s], to: v(0, 6000), queue: false })]);
}

#[test]
fn a_scripted_warp_move_flies_as_a_plain_move() {
    warps_only_with_the_switch(Profile::Sta2Classic, "st_seer", "a script's warp move", |_, s| vec![by(PlayerId::SCRIPT, Order::MoveWarp { units: vec![s], to: v(0, 6000), queue: false })]);
}

#[test]
fn an_attack_approach_does_not_warp() {
    warps_only_with_the_switch(Profile::Sta2Classic, "st_seer", "an attack approach", |sim, s| {
        let db = Arc::clone(sim.content.as_ref().unwrap());
        let t = spawn(sim, &db, "st_target", 2, v(0, 6000));
        vec![by(PlayerId(1), Order::Attack { units: vec![s], target: t })]
    });
}

/// A craft with the per-unit warp distance key warps by itself on a long plain move (the native
/// profile's own rule). Off means no warp by any path: that one too.
#[test]
fn the_keyed_automatic_warp_of_a_plain_move_is_off_too() {
    warps_only_with_the_switch(Profile::Native, "st_keyed", "a keyed craft's plain move", |_, s| vec![by(PlayerId(1), Order::Move { units: vec![s], to: v(0, 6000), queue: false, mode: None })]);
}

/// The mission switch and the content switch must both allow warp: a script that sets
/// `AllowWarp` cannot turn warp back on for content that has none, and can still turn it off
/// for content that has it.
#[test]
fn a_script_cannot_turn_warp_back_on() {
    let allow = |on: bool| Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(Edit::MissionSwitch(MissionSwitch::AllowWarp, on)) };
    let run = |db: &Arc<ContentDb>, switch: bool| {
        let mut sim = game(db);
        let s = spawn(&mut sim, db, "st_seer", 1, v(0, 0));
        sim.step(&[allow(switch)]);
        fly(&mut sim, vec![by(PlayerId(1), Order::MoveWarp { units: vec![s], to: v(0, 6000), queue: false })], s, 8 * HZ).warped
    };
    let (with, without) = (db(Profile::Sta2Classic, None), db(Profile::Sta2Classic, Some(false)));
    assert!(run(&with, true), "content with warp, mission switch on");
    assert!(!run(&with, false), "content with warp, mission switch off");
    assert!(!run(&without, true), "content without warp: the script's switch does not bring it back");
    assert!(!run(&without, false));
}

/// Armada I's own cruise model is another mechanism: the switch does not slow it.
#[test]
fn armada_one_cruise_is_not_touched_by_the_switch() {
    let far = |warp: Option<bool>| {
        let db = db(Profile::Sta1Classic, warp);
        let mut sim = game(&db);
        let s = spawn(&mut sim, &db, "st_seer", 1, v(0, 0));
        let f = fly(&mut sim, vec![by(PlayerId(1), Order::Move { units: vec![s], to: v(0, 7000), queue: false, mode: None })], s, 8 * HZ);
        (f.z, f.top)
    };
    assert_eq!(far(Some(false)), far(None), "the same flight with and without the switch");
}
