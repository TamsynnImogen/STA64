//! LUA-17: a craft ordered onto a gate comes out of its partner (Armada I pairing).

use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{EntityId, Fixed, PlayerId, Sim, TeamId, Vec3, WorldSource};
use sta_content::{ContentDb, UnitDef};
use std::sync::Arc;

fn db(a1: bool) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.a1_compat = a1;
    let mut b = ContentDb::builder().profile(fake.profile()).features(features);
    for w in fake.weapons().cloned() {
        b = b.weapon(w);
    }
    for u in fake.units().cloned() {
        b = b.unit(u);
    }
    b = b.unit(UnitDef::new("fx_gate", "wormhole_obj"));
    Arc::new(b.build().unwrap())
}

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// Gates at (1000, 0) and (5000, 5000), a third unpaired one at (-3000, 0), a cruiser of seat 1
/// at the origin and one passing by.
fn world(db: &Arc<ContentDb>) -> (Sim, EntityId, EntityId) {
    let seat = PlayerSetup { id: PlayerId(1), team: TeamId(1), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::clone(db),
        players: vec![seat],
        options: GameOptions::default(),
    });
    let gate = db.key("fx_gate").unwrap();
    let cruiser = db.key("fx_cruiser").unwrap();
    let h = (Fixed::ONE, Fixed::ZERO);
    sim.spawn_def(gate, PlayerId(255), v(1000, 0), h).unwrap();
    sim.spawn_def(gate, PlayerId(255), v(5000, 5000), h).unwrap();
    sim.spawn_def(gate, PlayerId(255), v(-3000, 0), h).unwrap();
    let a = sim.spawn_def(cruiser, PlayerId(1), v(0, 0), h).unwrap();
    let b = sim.spawn_def(cruiser, PlayerId(1), v(0, 300), h).unwrap();
    (sim, a, b)
}

fn run(sim: &mut Sim, a: EntityId, b: EntityId) {
    let order = |seq, units, to| Command { player: PlayerId(1), seq, kind: CommandKind::Order(Order::Move { units, to, queue: false }) };
    // `a` is sent onto the first gate; `b` flies past it to a point beyond.
    sim.step(&[order(1, vec![a], v(1000, 0)), order(2, vec![b], v(2000, 300))]);
    for _ in 0..30 * 30 {
        sim.step(&[]);
    }
}

fn pos(sim: &Sim, id: EntityId) -> (f32, f32) {
    let u = sim.units.get(id).unwrap();
    (u.pos.x.to_f32(), u.pos.z.to_f32())
}

#[test]
fn a_craft_ordered_onto_a_gate_comes_out_of_its_partner_at_rest() {
    let d = db(true);
    let (mut sim, a, b) = world(&d);
    run(&mut sim, a, b);
    let (x, z) = pos(&sim, a);
    assert!((x - 5000.0).abs() < 200.0 && (z - 5000.0).abs() < 200.0, "a at ({x}, {z})");
    let u = sim.units.get(a).unwrap();
    assert!(u.target.is_none() && u.speed == Fixed::ZERO, "idle after the exit");
    // A craft that only passes the gate is not taken.
    let (x, _) = pos(&sim, b);
    assert!(x > 1500.0, "b went on to x {x}");
}

#[test]
fn gates_move_nothing_outside_the_armada_i_pairing() {
    let d = db(false);
    let (mut sim, a, b) = world(&d);
    run(&mut sim, a, b);
    let (x, z) = pos(&sim, a);
    assert!((x - 1000.0).abs() < 200.0 && z.abs() < 200.0, "a at ({x}, {z})");
}

#[test]
fn the_same_run_gives_the_same_digest() {
    let d = db(true);
    let (mut one, a, b) = world(&d);
    run(&mut one, a, b);
    let (mut two, a, b) = world(&d);
    run(&mut two, a, b);
    assert_eq!(one.digest(), two.digest());
}

/// LUA-24: gates placed by a map are entities that missions find by label.
fn map_with_gates() -> sta_content::MapDef {
    let mut map = sta_content::MapDef::empty();
    let h = (Fixed::ONE, Fixed::ZERO);
    for (i, (x, z)) in [(1000, 0), (5000, 5000)].into_iter().enumerate() {
        map.objects.push(sta_content::MapObject { odf: "fx_gate".into(), owner: None, pos: v(x, z), heading: h, label: Some(format!("Gate{i}")) });
    }
    map
}

#[test]
fn map_gates_are_neutral_props_found_by_label_and_never_damaged() {
    let d = db(true);
    let seat = PlayerSetup { id: PlayerId(1), team: TeamId(1), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 3, map: Arc::new(map_with_gates()), content: Arc::clone(&d), players: vec![seat], options: GameOptions::default() });
    let ids: Vec<EntityId> = (0..2).map(|i| sim.labels.iter().find(|(_, l)| l.as_str() == format!("Gate{i}")).map(|(id, _)| id).expect("label resolves")).collect();
    for &id in &ids {
        let u = sim.units.get(id).unwrap();
        assert_eq!((u.kind, u.owner), (crate::world::EntityKind::Prop, crate::world::NEUTRAL));
    }
    let before: Vec<_> = ids.iter().map(|&id| sim.units.get(id).unwrap().hull).collect();
    // A cruiser of seat 1 beside the gate, left to fight whatever is near: nothing hurts a gate.
    let cruiser = d.key("fx_cruiser").unwrap();
    sim.spawn_def(cruiser, PlayerId(1), v(1100, 0), (Fixed::ONE, Fixed::ZERO)).unwrap();
    for _ in 0..30 * 20 {
        sim.step(&[]);
    }
    let after: Vec<_> = ids.iter().map(|&id| sim.units.get(id).map(|u| u.hull)).collect();
    assert_eq!(after, before.into_iter().map(Some).collect::<Vec<_>>());
}

#[test]
fn map_gates_pair_in_map_order_like_any_gates() {
    let d = db(true);
    let seat = PlayerSetup { id: PlayerId(1), team: TeamId(1), race: "fx".into(), control: Control::Idle };
    let sim = Sim::from_setup(&GameSetup { seed: 3, map: Arc::new(map_with_gates()), content: Arc::clone(&d), players: vec![seat], options: GameOptions::default() });
    let pairs = sim.gate_pairs();
    assert_eq!(pairs.len(), 2);
    assert_eq!((pairs[0].0, pairs[0].1), (pairs[1].1, pairs[1].0));
}
