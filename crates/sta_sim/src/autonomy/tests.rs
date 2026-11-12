use std::sync::Arc;

use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{AutonomyKind, AutonomyLevel, Command, CommandKind, Order, Target};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

fn at(x: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::ZERO }
}

fn scene() -> (Sim, Arc<sta_content::ContentDb>, EntityId, EntityId) {
    let db = Arc::new(sta_content::fake::db());
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 5,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::clone(&db),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    });
    let ship = sim.spawn_def(db.key("fx_frigate").unwrap(), PlayerId(1), at(0), AHEAD).unwrap();
    let station = sim.spawn_def(db.key("fx_station").unwrap(), PlayerId(1), at(3000), AHEAD).unwrap();
    (sim, db, ship, station)
}

fn set(player: u8, id: EntityId, kind: AutonomyKind, level: AutonomyLevel) -> Command {
    Command {
        player: PlayerId(player),
        seq: 0,
        kind: CommandKind::Order(Order::Autonomy { units: vec![id], kind, level }),
    }
}

fn view_levels(sim: &Sim, id: EntityId) -> Option<[u8; 2]> {
    sim.view().units.iter().find(|u| u.id == id).and_then(|u| u.autonomy)
}

#[test]
fn craft_start_at_the_engine_defaults_and_the_order_changes_one_level() {
    let (mut sim, _, ship, station) = scene();
    sim.step(&[]);
    assert_eq!(view_levels(&sim, ship), Some([1, 0]));
    assert_eq!(view_levels(&sim, station), None, "only craft carry autonomy");
    sim.step(&[set(1, ship, AutonomyKind::Movement, AutonomyLevel::High)]);
    assert_eq!(view_levels(&sim, ship), Some([2, 0]));
    sim.step(&[set(1, ship, AutonomyKind::SpecialWeapon, AutonomyLevel::Medium)]);
    assert_eq!(view_levels(&sim, ship), Some([2, 1]));
}

#[test]
fn only_the_owner_orders_and_only_craft() {
    let (mut sim, _, ship, station) = scene();
    let out = sim.step(&[set(2, ship, AutonomyKind::Movement, AutonomyLevel::Low)]);
    assert!(out.events.iter().any(|e| matches!(e, crate::SimEvent::Rejected { .. })));
    let out = sim.step(&[set(1, station, AutonomyKind::Movement, AutonomyLevel::Low)]);
    assert!(out.events.iter().any(|e| matches!(e, crate::SimEvent::Rejected { .. })));
    assert_eq!(view_levels(&sim, ship), Some([1, 0]));
}

#[test]
fn back_to_the_default_leaves_no_trace_in_the_digest() {
    let (mut a, _, ship, _) = scene();
    let (mut b, _, ship_b, _) = scene();
    a.step(&[set(1, ship, AutonomyKind::Movement, AutonomyLevel::Low)]);
    a.step(&[set(1, ship, AutonomyKind::Movement, AutonomyLevel::Medium)]);
    b.step(&[]);
    b.step(&[]);
    assert_eq!(ship, ship_b);
    assert_eq!(a.digest(), b.digest());
    // A level that differs is in the digest.
    a.step(&[set(1, ship, AutonomyKind::Movement, AutonomyLevel::High)]);
    b.step(&[]);
    assert_ne!(a.digest(), b.digest());
}

#[test]
fn the_order_takes_the_next_free_wire_tag_and_old_tags_stay() {
    let tag = |o: &Order| postcard::to_allocvec(o).unwrap()[0];
    assert_eq!(tag(&Order::Rally { builders: vec![], at: Target::None }), 8);
    let o = Order::Autonomy { units: vec![EntityId(3)], kind: AutonomyKind::SpecialWeapon, level: AutonomyLevel::High };
    assert_eq!(tag(&o), 9);
    assert_eq!(postcard::from_bytes::<Order>(&postcard::to_allocvec(&o).unwrap()).unwrap(), o);
}

#[test]
fn snapshot_keeps_autonomy_and_a_despawn_drops_it() {
    let (mut sim, db, ship, _) = scene();
    sim.step(&[set(1, ship, AutonomyKind::SpecialWeapon, AutonomyLevel::High)]);
    let mut loaded = Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap();
    assert_eq!(loaded.digest(), sim.digest());
    assert_eq!(view_levels(&loaded, ship), Some([1, 2]));
    for _ in 0..5 {
        sim.step(&[]);
        loaded.step(&[]);
        assert_eq!(loaded.digest(), sim.digest());
    }
    assert!(loaded.despawn(ship, None));
    assert!(loaded.autonomy.is_empty());
}
