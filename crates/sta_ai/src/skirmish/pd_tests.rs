//! AI-04: the skirmish AI switches point defence on when it is shot at and
//! off when it is quiet. Under the `pd_energy` rules, on the fake content.

use std::sync::Arc;

use sta_content::{ContentDb, DefRef, Hardpoint};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::Registry;

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// The specials fixture (`fx_cloaker` has a 600-point energy pool) with its
/// beam swapped for a PD weapon that costs 20 energy per second.
fn content() -> ContentDb {
    let base = sta_sim::specials::fixture::db();
    let mut features = base.features().clone();
    features.pd_energy = true;
    let mut b = ContentDb::builder().profile(base.profile()).features(features);
    for mut w in base.weapons().cloned() {
        if w.odf == "fx_pd" {
            w.energy_cost = Fixed::from_int(20);
        }
        b = b.weapon(w);
    }
    for mut u in base.units().cloned() {
        if u.odf == "fx_cloaker" {
            u.hardpoints[0] = Hardpoint { weapon: DefRef::new("fx_pd"), node: "hp01".into(), arc: None };
        }
        b = b.unit(u);
    }
    b.build().unwrap()
}

#[test]
fn pd_goes_on_under_attack_catches_the_torpedo_and_goes_off_when_quiet() {
    let seat = |id: u8, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
    let setup = GameSetup {
        seed: 9,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::new(content()),
        players: vec![seat(0, Control::Ai(crate::skirmish::SKIRMISH.into())), seat(1, Control::Idle)],
        options: GameOptions::default(),
    };
    let db = Arc::clone(&setup.content);
    let mut sim = Sim::from_setup(&setup);
    let ahead = (Fixed::ZERO, Fixed::ONE);
    let ship = sim.spawn_def(db.key("fx_cloaker").unwrap(), PlayerId(0), v(2000, 2800), ahead).unwrap();
    let boat = sim.spawn_def(db.key("fx_cruiser").unwrap(), PlayerId(1), v(2000, 2000), ahead).unwrap();
    let mut ctl = Registry::builtin().create_for_seat(crate::skirmish::SKIRMISH, &setup, PlayerId(0)).unwrap();

    let (mut events, mut on_at, mut off_at, mut pd_shots, mut hits) = (Vec::<SimEvent>::new(), None, None, 0, 0);
    for t in 0..900u64 {
        let view = sim.view();
        let mut out = Vec::new();
        ctl.think(&view, &events, &mut out);
        if t == 0 {
            // Pin the ship: a torpedo fired from inside PD range cannot be caught.
            out.push(Command { player: PlayerId::BENCH, seq: 9, kind: CommandKind::Edit(Edit::Engines(ship, false)) });
            out.push(Command {
                player: PlayerId(1),
                seq: 0,
                kind: CommandKind::Order(Order::Attack { units: vec![boat], target: ship }),
            });
        }
        if t == 300 {
            // Automatic fire would go on without the order, so silence the guns.
            out.push(Command { player: PlayerId::BENCH, seq: 10, kind: CommandKind::Edit(Edit::Weapons(boat, false)) });
        }
        events = sim.step(&out).events;
        for e in &events {
            match e {
                SimEvent::PointDefenceChanged { id, on: true } if *id == ship => on_at = on_at.or(Some(t)),
                SimEvent::PointDefenceChanged { id, on: false } if *id == ship => off_at = off_at.or(Some(t)),
                SimEvent::Fired { shooter, hardpoint: 0, .. } if *shooter == ship => pd_shots += 1,
                SimEvent::Hit { target, .. } if *target == ship => hits += 1,
                _ => {}
            }
        }
    }
    let on = on_at.expect("the AI never switched PD on");
    assert!(on < 40, "on before the first torpedo is in range: tick {on}");
    assert!(pd_shots >= 3, "PD shot at the torpedoes: {pd_shots}");
    assert_eq!(hits, 0, "every torpedo was intercepted");
    let off = off_at.expect("the AI never switched PD off");
    assert!(off > 300, "off only after the attack stopped: tick {off}");
    assert!(off < 300 + 90 + 40, "off within a few seconds of quiet: tick {off}");
    assert!(!sim.view().units.is_empty());
}

/// A mod that sets `pd_energy` without `point_defense_cycles` gets no PD
/// slot, so the AI sends nothing and nothing is refused over and over.
#[test]
fn without_the_cycles_bit_the_ai_sends_no_pd_orders() {
    let base = content();
    let mut features = base.features().clone();
    features.point_defense_cycles = false;
    let mut b = ContentDb::builder().profile(base.profile()).features(features);
    for w in base.weapons().cloned() {
        b = b.weapon(w);
    }
    for u in base.units().cloned() {
        b = b.unit(u);
    }
    let seat = |id: u8, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
    let setup = GameSetup {
        seed: 9,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::new(b.build().unwrap()),
        players: vec![seat(0, Control::Ai(crate::skirmish::SKIRMISH.into())), seat(1, Control::Idle)],
        options: GameOptions::default(),
    };
    let db = Arc::clone(&setup.content);
    let mut sim = Sim::from_setup(&setup);
    let ahead = (Fixed::ZERO, Fixed::ONE);
    let ship = sim.spawn_def(db.key("fx_cloaker").unwrap(), PlayerId(0), v(2000, 2800), ahead).unwrap();
    let boat = sim.spawn_def(db.key("fx_cruiser").unwrap(), PlayerId(1), v(2000, 2000), ahead).unwrap();
    let mut ctl = Registry::builtin().create_for_seat(crate::skirmish::SKIRMISH, &setup, PlayerId(0)).unwrap();
    let mut events = Vec::<SimEvent>::new();
    for t in 0..300u64 {
        let view = sim.view();
        let mut out = Vec::new();
        ctl.think(&view, &events, &mut out);
        assert!(!out.iter().any(|c| matches!(c.kind, CommandKind::Order(Order::Special { .. }))), "tick {t}");
        if t == 0 {
            out.push(Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![boat], target: ship }) });
        }
        events = sim.step(&out).events;
        assert!(!events.iter().any(|e| matches!(e, SimEvent::Rejected { player, .. } if *player == PlayerId(0))));
    }
}
