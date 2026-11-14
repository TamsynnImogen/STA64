//! LUA-28: the skirmish AI leaves special-forces craft alone. Fake content, authored scene.

use std::collections::BTreeSet;
use std::sync::Arc;

use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::Registry;

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn units_of(order: &Order) -> &[EntityId] {
    match order {
        Order::Move { units, .. }
        | Order::Attack { units, .. }
        | Order::AttackAny { units }
        | Order::Mine { units, .. }
        | Order::Stop { units }
        | Order::Patrol { units, .. }
        | Order::Special { units, .. } => units,
        _ => &[],
    }
}

/// Run the AI for `minutes`; returns every unit of seat 0 it ordered, with `flag` of the six
/// cruisers flagged first.
fn ordered(flag: bool, minutes: u64) -> (BTreeSet<EntityId>, Vec<EntityId>) {
    let seat = |id: u8, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
    let setup = GameSetup {
        seed: 5,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::new(sta_content::fake::db()),
        players: vec![seat(0, Control::Ai(super::SKIRMISH.into())), seat(1, Control::Idle)],
        options: GameOptions::default(),
    };
    let db = Arc::clone(&setup.content);
    let mut sim = Sim::from_setup(&setup);
    let ahead = (Fixed::ZERO, Fixed::ONE);
    let mut seq = 0;
    let mut bench = |e: Edit| {
        seq += 1;
        Command { player: PlayerId::BENCH, seq, kind: CommandKind::Edit(e) }
    };
    let mut batch = Vec::new();
    for (i, s) in setup.map.starts.iter().enumerate() {
        let p = PlayerId(i as u8);
        for (odf, dz) in [("fx_station", 0), ("fx_yard", 300)] {
            let pos = Vec3 { x: s.pos.x, y: s.pos.y, z: s.pos.z + Fixed::from_int(dz) };
            batch.push(bench(Edit::Spawn { odf: db.key(odf).unwrap(), player: p, pos, heading: s.heading }));
        }
        batch.push(bench(Edit::Cargo(p, 0, Fixed::from_int(1000))));
    }
    sim.step(&batch);
    let cruisers: Vec<EntityId> = (0..6)
        .map(|n| sim.spawn_def(db.key("fx_cruiser").unwrap(), PlayerId(0), v(1500 + 40 * n, 1500), ahead).unwrap())
        .collect();
    if flag {
        let edits: Vec<Command> = cruisers
            .iter()
            .enumerate()
            .map(|(n, id)| Command { player: PlayerId::SCRIPT, seq: n as u32 + 1, kind: CommandKind::Edit(Edit::SpecialForces(*id, true)) })
            .collect();
        sim.step(&edits);
        assert!(cruisers.iter().all(|id| sim.view().is_special_forces(*id)), "the view lists the flagged craft");
    }
    let mut ctl = Registry::builtin().create_for_seat(super::SKIRMISH, &setup, PlayerId(0)).unwrap();
    let (mut events, mut seen) = (Vec::<SimEvent>::new(), BTreeSet::new());
    for _ in 0..minutes * 60 * u64::from(TICK_HZ) {
        let view = sim.view();
        let mut out = Vec::new();
        ctl.think(&view, &events, &mut out);
        for c in &out {
            if let CommandKind::Order(o) = &c.kind {
                seen.extend(units_of(o).iter().copied());
            }
        }
        events = sim.step(&out).events;
    }
    (seen, cruisers)
}

#[test]
fn flagged_craft_get_no_orders_from_the_strategic_ai_and_unflagged_ones_do() {
    let (seen, cruisers) = ordered(false, 6);
    assert!(cruisers.iter().any(|c| seen.contains(c)), "control: the AI uses unflagged warships");
    let (seen, cruisers) = ordered(true, 6);
    assert!(cruisers.iter().all(|c| !seen.contains(c)), "no order named a flagged craft: {seen:?}");
}
