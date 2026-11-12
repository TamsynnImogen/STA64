//! Done-when for SIM-13: the last unit of a seat is killed, exactly one
//! `PlayerEliminated` comes out, on the tick of the `Destroyed`.

use crate::command::{Command, CommandKind, Order};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

fn p(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

/// Seats `(player, team)`; P1's cruiser at (2000,2000), P2's frigate astern.
fn fixture(seats: &[(u8, u8)]) -> (Sim, EntityId, EntityId) {
    let db = sta_content::fake::db();
    let (cruiser, frigate) = (db.key("fx_cruiser").unwrap(), db.key("fx_frigate").unwrap());
    let setup = GameSetup {
        seed: 7,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(db),
        players: seats
            .iter()
            .map(|&(id, team)| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(team),
                race: "fx".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let ahead = (Fixed::ZERO, Fixed::ONE);
    let a = sim
        .spawn_def(cruiser, PlayerId(1), p(2000, 2000), ahead)
        .unwrap();
    let t = sim
        .spawn_def(frigate, PlayerId(2), p(2000, 2350), ahead)
        .unwrap();
    for &(id, _) in seats.iter().filter(|s| s.0 > 2) {
        sim.spawn_def(frigate, PlayerId(id), p(6000, 6000), ahead)
            .unwrap();
    }
    (sim, a, t)
}

fn attack(a: EntityId, t: EntityId) -> Vec<Command> {
    vec![Command {
        player: PlayerId(1),
        seq: 0,
        kind: CommandKind::Order(Order::Attack {
            units: vec![a],
            target: t,
        }),
    }]
}

/// Run well past the kill; returns `(destroyed tick, eliminations)`.
fn run(sim: &mut Sim, a: EntityId, t: EntityId, ticks: u64) -> (Option<u64>, Vec<(u64, PlayerId)>) {
    let (mut dead, mut elim) = (None, Vec::new());
    for tick in 0..ticks {
        let out = sim.step(&if tick == 0 { attack(a, t) } else { vec![] });
        for e in &out.events {
            match e {
                SimEvent::Destroyed { id, .. } if *id == t => dead = Some(tick),
                SimEvent::PlayerEliminated { player } => elim.push((tick, *player)),
                _ => {}
            }
        }
    }
    (dead, elim)
}

#[test]
fn last_unit_killed_eliminates_once() {
    let (mut sim, a, t) = fixture(&[(1, 1), (2, 2)]);
    assert_eq!(sim.winners(), None);
    let (dead, elim) = run(&mut sim, a, t, 1500);
    let dead = dead.expect("the frigate dies");
    assert_eq!(elim, vec![(dead, PlayerId(2))]);
    assert!(sim.is_eliminated(PlayerId(2)) && !sim.is_eliminated(PlayerId(1)));
    assert_eq!(sim.winners(), Some(vec![PlayerId(1)]));
    // Commands of the eliminated seat are rejected.
    let cmd = Command {
        player: PlayerId(2),
        seq: 1,
        kind: CommandKind::Order(Order::Stop { units: vec![] }),
    };
    let out = sim.step(&[cmd]);
    assert!(out.events.contains(&SimEvent::Rejected {
        player: PlayerId(2),
        seq: 1
    }));
    assert_eq!(crate::unit_status_tests::digest_before_crew(&sim), 0x8ed0_6fd5_65be_9495);
    assert_eq!(
        sim.digest(),
        0xa4f1_4288_154b_3eef,
        "digest after elimination"
    );
}

#[test]
fn ally_alive_means_game_continues_for_that_team_only() {
    // P1 and P3 allied against P2.
    let (mut sim, a, t) = fixture(&[(1, 1), (2, 2), (3, 1)]);
    let (_, elim) = run(&mut sim, a, t, 1500);
    assert_eq!(elim.len(), 1);
    assert_eq!(sim.winners(), Some(vec![PlayerId(1), PlayerId(3)]));
}

#[test]
fn two_teams_left_is_not_over() {
    // P3 is a third party: killing P2 leaves teams 1 and 3.
    let (mut sim, a, t) = fixture(&[(1, 1), (2, 2), (3, 3)]);
    let (_, elim) = run(&mut sim, a, t, 1500);
    assert_eq!(elim.len(), 1);
    assert_eq!(sim.winners(), None);
}

#[test]
fn seat_with_a_surviving_unit_is_not_eliminated() {
    let (mut sim, a, t) = fixture(&[(1, 1), (2, 2)]);
    let frigate = sim.unit(t).unwrap().def;
    sim.spawn_def(
        frigate,
        PlayerId(2),
        p(6000, 6000),
        (Fixed::ZERO, Fixed::ONE),
    )
    .unwrap();
    let (dead, elim) = run(&mut sim, a, t, 1500);
    assert!(dead.is_some());
    assert!(elim.is_empty());
    assert_eq!(sim.winners(), None);
}

#[test]
fn rec_round_trips() {
    let (mut sim, a, t) = fixture(&[(1, 1), (2, 2)]);
    run(&mut sim, a, t, 1500);
    let r = sim.victory.to_rec();
    assert_eq!(r.eliminated, vec![2]);
    assert_eq!(
        super::Victory::from_rec(&r).eliminated(),
        sim.victory.eliminated()
    );
}

#[test]
fn the_view_shows_who_is_eliminated() {
    let (mut sim, a, t) = fixture(&[(1, 1), (2, 2)]);
    run(&mut sim, a, t, 1500);
    let view = sim.step(&[]).view;
    let flags: Vec<_> = view.players.iter().map(|p| (p.id, p.eliminated)).collect();
    assert_eq!(flags, vec![(PlayerId(1), false), (PlayerId(2), true)]);
}
