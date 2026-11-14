//! LUA-27: the scripted impulse-speed override (`Edit::MaxSpeed`).

use crate::command::{Command, CommandKind, Edit, Order};
use crate::present::Presentation;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{EntityId, Fixed, PlayerId, Sim, SimEvent, TeamId, Vec3, WorldSource};
use sta_content::MapDef;
use std::sync::Arc;

fn at(x: i32) -> Vec3 {
    Vec3::from_f32([x as f32, 0.0, 0.0])
}

fn game() -> Sim {
    Sim::from_setup(&GameSetup {
        seed: 27,
        map: Arc::new(MapDef::empty()),
        content: Arc::new(sta_content::fake::db()),
        players: [0u8, 1]
            .iter()
            .map(|&id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Human })
            .collect(),
        options: GameOptions::default(),
    })
}

fn spawn(sim: &mut Sim, odf: &str, owner: u8, x: i32) -> EntityId {
    let key = sim.content().unwrap().key(odf).unwrap();
    sim.spawn_def(key, PlayerId(owner), at(x), (Fixed::ONE, Fixed::ZERO)).unwrap()
}

fn class(sim: &Sim, id: EntityId) -> Fixed {
    sim.content().unwrap().unit(sim.unit(id).unwrap().def).unwrap().movement.max_speed
}

fn speed(sim: &Sim, id: EntityId) -> Fixed {
    sim.unit(id).unwrap().stats.max_speed
}

fn edit(sim: &mut Sim, e: Edit) -> bool {
    let c = Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(e) };
    !sim.step(&[c]).events.iter().any(|e| matches!(e, SimEvent::Rejected { .. }))
}

fn order(sim: &mut Sim, player: u8, o: Order) {
    sim.step(&[Command { player: PlayerId(player), seq: 0, kind: CommandKind::Order(o) }]);
}

fn peak(sim: &mut Sim, id: EntityId, ticks: u32) -> Fixed {
    let mut top = Fixed::ZERO;
    for _ in 0..ticks {
        sim.step(&[]);
        top = top.max(sim.unit(id).unwrap().speed);
    }
    top
}

#[test]
fn set_caps_speed_on_a_move_and_above_the_definition_raises_it() {
    let mut sim = game();
    let a = spawn(&mut sim, "fx_cruiser", 0, 0);
    let def = class(&sim, a);
    assert!(edit(&mut sim, Edit::MaxSpeed(a, Some(Fixed::from_int(30)))));
    order(&mut sim, 0, Order::Move { units: vec![a], to: at(20000), queue: false });
    let top = peak(&mut sim, a, 300);
    assert!(top <= Fixed::from_int(30) && top > Fixed::from_int(25), "{top:?}");

    let mut fast = game();
    let b = spawn(&mut fast, "fx_cruiser", 0, 0);
    assert!(edit(&mut fast, Edit::MaxSpeed(b, Some(def * Fixed::from_int(2)))));
    order(&mut fast, 0, Order::Move { units: vec![b], to: at(40000), queue: false });
    assert!(peak(&mut fast, b, 600) > def);
}

#[test]
fn set_caps_an_attack_approach() {
    let mut sim = game();
    let a = spawn(&mut sim, "fx_cruiser", 0, 0);
    let target = spawn(&mut sim, "fx_cruiser", 1, 30000);
    assert!(edit(&mut sim, Edit::MaxSpeed(a, Some(Fixed::from_int(40)))));
    order(&mut sim, 0, Order::Attack { units: vec![a], target });
    let top = peak(&mut sim, a, 300);
    assert!(top <= Fixed::from_int(40) && top > Fixed::from_int(30), "{top:?}");
}

#[test]
fn reset_restores_the_definition_and_the_override_persists_across_orders() {
    let mut sim = game();
    let a = spawn(&mut sim, "fx_cruiser", 0, 0);
    let def = class(&sim, a);
    assert!(edit(&mut sim, Edit::MaxSpeed(a, Some(Fixed::from_int(30)))));
    order(&mut sim, 0, Order::Move { units: vec![a], to: at(5000), queue: false });
    peak(&mut sim, a, 30);
    order(&mut sim, 0, Order::Stop { units: vec![a] });
    order(&mut sim, 0, Order::Move { units: vec![a], to: at(-5000), queue: false });
    assert_eq!(speed(&sim, a), Fixed::from_int(30));
    assert!(edit(&mut sim, Edit::MaxSpeed(a, None)));
    assert_eq!(speed(&sim, a), def);
    assert!(sim.view().attrs.impulse.is_empty());
}

#[test]
fn view_lists_only_craft_that_differ_from_their_definition() {
    let mut sim = game();
    let a = spawn(&mut sim, "fx_cruiser", 0, 0);
    let _b = spawn(&mut sim, "fx_cruiser", 0, 100);
    assert!(edit(&mut sim, Edit::MaxSpeed(a, Some(Fixed::from_int(31)))));
    assert_eq!(sim.view().attrs.impulse, vec![(a, Fixed::from_int(31))]);
}

#[test]
fn a_team_change_resets_the_override() {
    let mut sim = game();
    let a = spawn(&mut sim, "fx_cruiser", 0, 0);
    let def = class(&sim, a);
    assert!(edit(&mut sim, Edit::MaxSpeed(a, Some(Fixed::from_int(30)))));
    assert!(edit(&mut sim, Edit::Team(a, PlayerId(1))));
    assert_eq!(speed(&sim, a), def);
}

#[test]
fn the_end_of_a_cineractive_resets_every_craft() {
    let mut sim = game();
    let a = spawn(&mut sim, "fx_cruiser", 0, 0);
    let b = spawn(&mut sim, "fx_frigate", 1, 500);
    let (da, db) = (class(&sim, a), class(&sim, b));
    for id in [a, b] {
        assert!(edit(&mut sim, Edit::MaxSpeed(id, Some(Fixed::from_int(30)))));
    }
    assert!(edit(&mut sim, Edit::Present(Presentation::Cineractive { on: true, skippable: false })));
    assert_eq!(speed(&sim, a), Fixed::from_int(30));
    assert!(edit(&mut sim, Edit::Present(Presentation::Cineractive { on: false, skippable: false })));
    assert_eq!((speed(&sim, a), speed(&sim, b)), (da, db));
}

#[test]
fn only_live_craft_are_accepted() {
    let mut sim = game();
    let a = spawn(&mut sim, "fx_cruiser", 0, 0);
    let station = spawn(&mut sim, "fx_station", 0, 900);
    let before = speed(&sim, station);
    assert!(!edit(&mut sim, Edit::MaxSpeed(station, Some(Fixed::from_int(30)))));
    assert!(!edit(&mut sim, Edit::MaxSpeed(EntityId(9999), None)));
    assert_eq!(speed(&sim, station), before);
    sim.units.get_mut(a).unwrap().hull[0] = Fixed::ZERO;
    assert!(!edit(&mut sim, Edit::MaxSpeed(a, Some(Fixed::from_int(30)))));
    sim.units.get_mut(a).unwrap().hull[0] = Fixed::ONE;
    sim.units.get_mut(a).unwrap().flags = crate::view::UnitFlags::DYING;
    assert!(!edit(&mut sim, Edit::MaxSpeed(a, None)));
    // Only the script (or bench) may send it.
    let c = Command { player: PlayerId(0), seq: 0, kind: CommandKind::Edit(Edit::MaxSpeed(a, None)) };
    assert!(sim.step(&[c]).events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
}

/// The pinned whole-state digest of a run holding an overridden unit; it moves
/// only when the override's effect on the sim or the digest changes.
#[test]
fn snapshot_round_trip_keeps_the_override_and_the_pinned_digest() {
    let mut sim = game();
    let a = spawn(&mut sim, "fx_cruiser", 0, 0);
    let b = spawn(&mut sim, "fx_frigate", 1, 800);
    assert!(edit(&mut sim, Edit::MaxSpeed(a, Some(Fixed::from_int(30)))));
    order(&mut sim, 0, Order::Move { units: vec![a], to: at(3000), queue: false });
    order(&mut sim, 1, Order::Move { units: vec![b], to: at(-3000), queue: false });
    for _ in 0..120 {
        sim.step(&[]);
    }
    let digest = sim.digest();
    let mut loaded = Sim::from_snapshot(&sim.snapshot(), sim.content().cloned()).unwrap();
    assert_eq!(loaded.digest(), digest);
    assert_eq!(speed(&loaded, a), Fixed::from_int(30));
    for _ in 0..60 {
        sim.step(&[]);
        loaded.step(&[]);
    }
    assert_eq!(loaded.digest(), sim.digest());
    assert_eq!(digest, PINNED_DIGEST, "{digest:#x}");
}

const PINNED_DIGEST: u64 = 0x84a8_3715_ffcd_bf31;
