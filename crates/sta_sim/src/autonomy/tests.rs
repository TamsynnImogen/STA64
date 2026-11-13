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

#[test]
fn a_row_equal_to_the_engine_default_is_not_canonical() {
    let (_, _, ship, _) = scene();
    assert!(super::Autonomy::from_rec(vec![(ship, super::ENGINE_DEFAULT)]).is_none());
    assert!(super::Autonomy::from_rec(vec![(ship, [2, 0])]).is_some());
}

#[test]
fn a_dead_craft_takes_no_autonomy_order() {
    let (mut sim, _, ship, _) = scene();
    sim.units.get_mut(ship).unwrap().hull[0] = Fixed::ZERO;
    sim.step(&[set(1, ship, AutonomyKind::Movement, AutonomyLevel::High)]);
    assert!(sim.autonomy.is_empty());
}

#[test]
fn a_restored_sim_carries_the_definitions_warp_speed_like_a_live_one() {
    let base = sta_content::fake::db();
    let mut b = sta_content::ContentDb::builder().profile(base.profile()).features(base.features().clone());
    for w in base.weapons().cloned() {
        b = b.weapon(w);
    }
    for mut u in base.units().cloned() {
        if u.odf == "fx_frigate" {
            u.movement.warp_speed = Fixed::from_int(900);
        }
        b = b.unit(u);
    }
    let db = Arc::new(b.build().unwrap());
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 5,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::clone(&db),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    });
    let ship = sim.spawn_def(db.key("fx_frigate").unwrap(), PlayerId(1), at(0), AHEAD).unwrap();
    let live = sim.units.get(ship).unwrap().stats.warp_speed;
    assert_eq!(live, Fixed::from_int(900));
    let loaded = Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap();
    assert_eq!(loaded.units.get(ship).unwrap().stats, sim.units.get(ship).unwrap().stats);
    assert_eq!(loaded.digest(), sim.digest());
}

/// SIM-36: a new craft takes `RTS_CFG.h`'s default autonomy as an explicit
/// row (Roots and FO 4.0: 1 and 2); the engine default writes none.
#[test]
fn spawned_crafts_take_the_content_default_as_explicit_rows() {
    let rules = sta_content::Rules::default().with_autonomy(sta_content::AutonomyRules { movement: 1, special_weapon: 2 });
    let db = Arc::new(sta_content::fake::db().with_rules(rules));
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
    assert_eq!(sim.autonomy.levels(ship), [1, 2]);
    assert_eq!(sim.autonomy.to_rec(), vec![(ship, [1, 2])], "stations carry no row");
    assert_eq!(view_levels(&sim, station), None);
    // Setting the engine default removes nothing it should not: the row now says [1, 0] by absence.
    WorldSource::step(&mut sim, &[set(1, ship, AutonomyKind::SpecialWeapon, AutonomyLevel::Low)]);
    assert!(sim.autonomy.to_rec().is_empty());
    let back = Sim::from_snapshot(&sim.snapshot(), Some(Arc::clone(&db))).unwrap();
    assert_eq!(back.digest(), sim.digest());

    // The engine's own default (fake content, no header) writes no row.
    let (plain, _, s, _) = scene();
    assert_eq!(plain.autonomy.levels(s), [1, 0]);
    assert!(plain.autonomy.to_rec().is_empty());
}
