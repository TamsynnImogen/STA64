//! AI-05: the skirmish AI does not attack an enemy derelict. Fake content, authored scene.

use std::sync::Arc;

use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::Registry;

/// Attack orders the AI aims at an enemy cruiser parked by its base over `minutes`; the
/// cruiser is a derelict (no crew) when `derelict`.
fn attacks_on_intruder(derelict: bool, minutes: u64) -> usize {
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
        batch.push(bench(Edit::Cargo(p, 0, Fixed::from_int(5000))));
    }
    sim.step(&batch);
    let home = setup.map.starts[0].pos;
    let at = Vec3 { x: home.x + Fixed::from_int(600), y: home.y, z: home.z };
    let intruder: EntityId = sim.spawn_def(db.key("fx_cruiser").unwrap(), PlayerId(1), at, (Fixed::ZERO, Fixed::ONE)).unwrap();
    if derelict {
        let u = sim.units.get_mut(intruder).unwrap();
        let max = u.crew.map_or(Fixed::from_int(100), |c| c[1]).max(Fixed::ONE);
        u.crew = Some([Fixed::ZERO, max]);
    }
    let mut ctl = Registry::builtin().create_for_seat(super::SKIRMISH, &setup, PlayerId(0)).unwrap();
    let (mut events, mut hits) = (Vec::<SimEvent>::new(), 0);
    for _ in 0..minutes * 60 * u64::from(TICK_HZ) {
        let view = sim.view();
        let mut out = Vec::new();
        ctl.think(&view, &events, &mut out);
        hits += out.iter().filter(|c| matches!(&c.kind, CommandKind::Order(Order::Attack { target, .. }) if *target == intruder)).count();
        events = sim.step(&out).events;
    }
    hits
}

#[test]
fn an_enemy_derelict_by_the_base_is_not_attacked_and_a_crewed_one_is() {
    assert!(attacks_on_intruder(false, 6) > 0, "control: the AI attacks a crewed intruder");
    assert_eq!(attacks_on_intruder(true, 6), 0, "no attack order on a derelict");
}
