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

/// Armada II-family scene: the map file places `gates` (labelled, in order) and a script adds
/// `scripted` (unlabelled); a cruiser `a` of seat 1 at the origin and `b` 300 north of it.
fn world_of(db: &Arc<ContentDb>, gates: &[(i32, i32)], scripted: &[(i32, i32)]) -> (Sim, EntityId, EntityId) {
    let mut map = sta_content::MapDef::empty();
    let h = (Fixed::ONE, Fixed::ZERO);
    for (i, &(x, z)) in gates.iter().enumerate() {
        map.objects.push(sta_content::MapObject { odf: "fx_gate".into(), owner: None, pos: v(x, z), heading: h, label: Some(format!("G{i}")) });
    }
    let seat = PlayerSetup { id: PlayerId(1), team: TeamId(1), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 3, map: Arc::new(map), content: Arc::clone(db), players: vec![seat], options: GameOptions::default() });
    let gate = db.key("fx_gate").unwrap();
    let cruiser = db.key("fx_cruiser").unwrap();
    for &(x, z) in scripted {
        sim.spawn_def(gate, PlayerId(255), v(x, z), h).unwrap();
    }
    let a = sim.spawn_def(cruiser, PlayerId(1), v(0, 0), h).unwrap();
    let b = sim.spawn_def(cruiser, PlayerId(1), v(0, 300), h).unwrap();
    (sim, a, b)
}

/// Two map gates at (1000, 0) and (5000, 5000).
fn world2(db: &Arc<ContentDb>) -> (Sim, EntityId, EntityId) {
    world_of(db, &[(1000, 0), (5000, 5000)], &[])
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
fn a_craft_ordered_onto_a_gate_in_the_armada_ii_family_emerges_at_the_partner_centre() {
    let d = db(false);
    let (mut sim, a, b) = world2(&d);
    run(&mut sim, a, b);
    let (x, z) = pos(&sim, a);
    assert!((x - 5000.0).abs() < 1.0 && (z - 5000.0).abs() < 1.0, "a at ({x}, {z})");
    let u = sim.units.get(a).unwrap();
    assert!(u.target.is_none() && u.speed == Fixed::ZERO, "order consumed");
    assert!(pos(&sim, b).0 > 1500.0, "a craft only passing the gate does not travel");
}

#[test]
fn an_emerged_craft_stays_until_ordered_onto_the_gate_again() {
    let d = db(false);
    let (mut sim, a, b) = world2(&d);
    run(&mut sim, a, b);
    for _ in 0..30 * 20 {
        sim.step(&[]);
    }
    let (x, z) = pos(&sim, a);
    assert!((x - 5000.0).abs() < 1.0 && (z - 5000.0).abs() < 1.0, "no travel back by itself: ({x}, {z})");
    let back = Command { player: PlayerId(1), seq: 9, kind: CommandKind::Order(Order::Move { units: vec![a], to: v(5000, 5000), queue: false }) };
    sim.step(&[back]);
    for _ in 0..30 * 5 {
        sim.step(&[]);
    }
    let (x, z) = pos(&sim, a);
    assert!((x - 1000.0).abs() < 1.0 && z.abs() < 1.0, "an explicit order sends it back: ({x}, {z})");
}

#[test]
fn a_group_ordered_onto_one_gate_all_emerge_and_the_separation_spreads_them() {
    let d = db(false);
    let (mut sim, _, _) = world2(&d);
    let cruiser = d.key("fx_cruiser").unwrap();
    let h = (Fixed::ONE, Fixed::ZERO);
    let ids: Vec<EntityId> = (0..3).map(|i| sim.spawn_def(cruiser, PlayerId(1), v(0, 100 * i), h).unwrap()).collect();
    let order = Command { player: PlayerId(1), seq: 1, kind: CommandKind::Order(Order::Move { units: ids.clone(), to: v(1000, 0), queue: false }) };
    sim.step(&[order]);
    for _ in 0..30 * 40 {
        sim.step(&[]);
    }
    let at: Vec<(f32, f32)> = ids.iter().map(|&i| pos(&sim, i)).collect();
    for &(x, z) in &at {
        assert!((x - 5000.0).abs() < 200.0 && (z - 5000.0).abs() < 200.0, "emerged near the partner: ({x}, {z})");
    }
    let apart = |p: (f32, f32), q: (f32, f32)| (p.0 - q.0).abs() + (p.1 - q.1).abs();
    eprintln!("group at {at:?}");
    assert!(apart(at[0], at[1]) > 1.0 && apart(at[1], at[2]) > 1.0 && apart(at[0], at[2]) > 1.0, "{at:?}");
}

/// Order `a` onto (x, z) and let it run; where it ends.
fn send_a_onto(sim: &mut Sim, a: EntityId, x: i32, z: i32) -> (f32, f32) {
    let to = Command { player: PlayerId(1), seq: 1, kind: CommandKind::Order(Order::Move { units: vec![a], to: v(x, z), queue: false }) };
    sim.step(&[to]);
    for _ in 0..30 * 60 {
        sim.step(&[]);
    }
    pos(sim, a)
}

#[test]
fn a_map_with_three_gates_pairs_nothing_in_the_armada_ii_family() {
    let d = db(false);
    let (mut sim, a, _) = world_of(&d, &[(1000, 0), (5000, 5000), (-3000, 0)], &[]);
    let (x, z) = send_a_onto(&mut sim, a, 1000, 0);
    assert!((x - 1000.0).abs() < 200.0 && z.abs() < 200.0, "no travel: ({x}, {z})");
    let (mut sim, a, _) = world_of(&d, &[(1000, 0)], &[]);
    let (x, z) = send_a_onto(&mut sim, a, 1000, 0);
    assert!((x - 1000.0).abs() < 200.0 && z.abs() < 200.0, "a lone gate does nothing: ({x}, {z})");
}

#[test]
fn a_script_added_third_gate_neither_joins_nor_breaks_the_pair() {
    let d = db(false);
    // The map's pair still works...
    let (mut sim, a, _) = world_of(&d, &[(1000, 0), (5000, 5000)], &[(-3000, 0)]);
    let (x, z) = send_a_onto(&mut sim, a, 1000, 0);
    assert!((x - 5000.0).abs() < 1.0 && (z - 5000.0).abs() < 1.0, "the pair works: ({x}, {z})");
    // ...and the script's gate goes nowhere.
    let (mut sim, a, _) = world_of(&d, &[(1000, 0), (5000, 5000)], &[(-3000, 0)]);
    let (x, z) = send_a_onto(&mut sim, a, -3000, 0);
    assert!((x + 3000.0).abs() < 200.0 && z.abs() < 200.0, "no travel into a script gate: ({x}, {z})");
}

#[test]
fn a_right_click_move_onto_a_gate_is_still_given_on_a_map_that_pairs_nothing() {
    let d = db(false);
    let (mut sim, _) = gate_world(&d);
    let cruiser = d.key("fx_cruiser").unwrap();
    let c = sim.spawn_def(cruiser, PlayerId(1), v(900, 0), (Fixed::ONE, Fixed::ZERO)).unwrap();
    let g0 = sim.labels.iter().find(|(_, l)| l.as_str() == "Gate0").map(|(id, _)| id).unwrap();
    let order = crate::context::context_order(Some(&d), &sim.view(), PlayerId(1), &[c], g0);
    assert_eq!(order, Some(Order::Move { units: vec![c], to: v(1000, 0), queue: false }));
}

/// Whole-state digest after a craft of an Armada II-family scene went through a gate.
#[test]
fn a_travelling_craft_is_pinned_in_the_whole_state_digest() {
    let d = db(false);
    let (mut sim, a, b) = world2(&d);
    run(&mut sim, a, b);
    assert_eq!(sim.digest(), PINNED_TRAVEL_DIGEST, "{:#x}", sim.digest());
}
const PINNED_TRAVEL_DIGEST: u64 = 0x8422_6030_5bce_e23c;

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
    for (i, (x, z)) in [(1000, 0), (5000, 5000), (-3000, 0), (-3000, 4000)].into_iter().enumerate() {
        map.objects.push(sta_content::MapObject { odf: "fx_gate".into(), owner: None, pos: v(x, z), heading: h, label: Some(format!("Gate{i}")) });
    }
    map
}

fn gate_world(d: &Arc<ContentDb>) -> (Sim, Vec<EntityId>) {
    let seat = PlayerSetup { id: PlayerId(1), team: TeamId(1), race: "fx".into(), control: Control::Idle };
    let sim = Sim::from_setup(&GameSetup { seed: 3, map: Arc::new(map_with_gates()), content: Arc::clone(d), players: vec![seat], options: GameOptions::default() });
    let ids = (0..4).map(|i| sim.labels.iter().find(|(_, l)| l.as_str() == format!("Gate{i}")).map(|(id, _)| id).expect("label resolves")).collect();
    (sim, ids)
}

#[test]
fn map_gates_are_neutral_props_found_by_label() {
    let d = db(true);
    let (sim, ids) = gate_world(&d);
    for &id in &ids {
        let u = sim.units.get(id).unwrap();
        assert_eq!((u.kind, u.owner), (crate::world::EntityKind::Prop, crate::world::NEUTRAL));
    }
}

#[test]
fn an_explicit_attack_does_not_hurt_a_stock_shaped_gate() {
    // Stock wormhole ODFs carry no hull key (read from data), so the gate has hull 0. A gate ODF
    // with a hull would be destroyable by an explicit attack (tried; not in stock data).
    let d = db(true);
    let (mut sim, ids) = gate_world(&d);
    let cruiser = d.key("fx_cruiser").unwrap();
    let c = sim.spawn_def(cruiser, PlayerId(1), v(1100, 0), (Fixed::ONE, Fixed::ZERO)).unwrap();
    let hull = sim.units.get(ids[0]).unwrap().hull;
    assert_eq!(hull[1], Fixed::ZERO);
    let attack = Command { player: PlayerId(1), seq: 1, kind: CommandKind::Order(Order::Attack { units: vec![c], target: ids[0] }) };
    sim.step(&[attack]);
    for _ in 0..30 * 20 {
        sim.step(&[]);
    }
    assert_eq!(sim.units.get(ids[0]).map(|u| u.hull), Some(hull));
}

#[test]
fn map_gates_pair_in_map_order() {
    let d = db(true);
    let (sim, g) = gate_world(&d);
    assert_eq!(sim.gate_pairs(), [(g[0], g[1]), (g[1], g[0]), (g[2], g[3]), (g[3], g[2])]);
}

#[test]
fn a_right_click_on_a_gate_moves_the_mobile_units_onto_its_centre() {
    let d = db(false);
    let (mut sim, g) = gate_world(&d);
    let cruiser = d.key("fx_cruiser").unwrap();
    let c = sim.spawn_def(cruiser, PlayerId(1), v(900, 0), (Fixed::ONE, Fixed::ZERO)).unwrap();
    let view = sim.view();
    let order = crate::context::context_order(Some(&d), &view, PlayerId(1), &[c], g[0]);
    assert_eq!(order, Some(Order::Move { units: vec![c], to: v(1000, 0), queue: false }));
    // Not a gate, not this rule.
    assert_eq!(crate::context::context_order(Some(&d), &view, PlayerId(1), &[c], c), None);
}
