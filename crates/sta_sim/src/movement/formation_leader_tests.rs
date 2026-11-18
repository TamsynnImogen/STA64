//! SIM-50: a formation keeps going when its leader leaves.
//!
//! STA64 gives every ship of a formation move its own slot when the order is given (the slots
//! are laid out about the destination, in ascending id; see [`crate::movement`]), so no ship
//! follows another: the followers reach their slots whatever happens to the lowest-id ship
//! afterwards. This is the Roots rule (engine changelog: followers continue to the formation's
//! destination when the leader is destroyed, replaced, ranked up or sent away; not measured).
//! Whether stock Armada II's followers stop with their leader is not measured either.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, Profile};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn db() -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(Profile::Fo4).features(Profile::Fo4.features());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    Arc::new(b.build().unwrap())
}

/// Three cruisers of player 1 in line formation, ordered 3000 along x.
fn formation() -> (Sim, Vec<EntityId>) {
    let d = db();
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 7, map: Arc::new(MapDef::empty()), content: Arc::clone(&d), players: vec![seat(1), seat(2)], options: GameOptions::default() });
    let ids: Vec<EntityId> = (0..3).map(|i| sim.spawn_def(d.key("fx_cruiser").unwrap(), PlayerId(1), v(0, i * 200), (Fixed::ONE, Fixed::ZERO)).unwrap()).collect();
    let order = |seq, o| Command { player: PlayerId(1), seq, kind: CommandKind::Order(o) };
    sim.step(&[order(0, Order::Formation { units: ids.clone(), kind: 1 })]);
    sim.step(&[order(1, Order::Move { units: ids.clone(), to: v(3000, 200), queue: false })]);
    (sim, ids)
}

fn run(sim: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        sim.step(&[]);
    }
}

fn gap(a: Vec3, b: Vec3) -> f32 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt().to_f32()
}

/// Where the two followers end when nothing happens to the leader.
fn control() -> Vec<Vec3> {
    let (mut sim, ids) = formation();
    run(&mut sim, 1800);
    ids[1..].iter().map(|id| sim.unit(*id).unwrap().pos).collect()
}

#[test]
fn the_followers_reach_their_slots_when_the_leader_is_destroyed_on_the_way() {
    let want = control();
    let (mut sim, ids) = formation();
    run(&mut sim, 150);
    assert!(sim.despawn(ids[0], None), "the leader is lost");
    run(&mut sim, 1650);
    for (id, at) in ids[1..].iter().zip(&want) {
        let now = sim.unit(*id).unwrap().pos;
        assert!(gap(now, *at) < 40.0, "{id:?} ended {} from its slot", gap(now, *at));
        assert!(now.x.to_f32() > 2500.0, "{id:?} went on to the destination");
    }
}

#[test]
fn the_followers_reach_their_slots_when_the_leader_is_sent_elsewhere() {
    let want = control();
    let (mut sim, ids) = formation();
    run(&mut sim, 150);
    let away = Command { player: PlayerId(1), seq: 2, kind: CommandKind::Order(Order::Move { units: vec![ids[0]], to: v(0, -2000), queue: false }) };
    sim.step(&[away]);
    run(&mut sim, 1650);
    for (id, at) in ids[1..].iter().zip(&want) {
        let now = sim.unit(*id).unwrap().pos;
        assert!(gap(now, *at) < 40.0, "{id:?} ended {} from its slot", gap(now, *at));
    }
    assert!(sim.unit(ids[0]).unwrap().pos.z.to_f32() < -1000.0, "the leader went its own way");
}
