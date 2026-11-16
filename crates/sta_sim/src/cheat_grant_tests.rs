//! Synthetic command-application coverage for repeatable resource grants.

use crate::command::{Command, CommandKind, Edit, TeamOp};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Fixed, PlayerId, Sim, SimEvent, TeamId, WorldSource};
use sta_content::MapDef;
use std::sync::Arc;

fn game() -> Sim {
    Sim::from_setup(&GameSetup {
        seed: 71,
        map: Arc::new(MapDef::empty()),
        content: Arc::new(sta_content::fake::db()),
        players: [0u8, 1]
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "fx".into(),
                control: Control::Human,
            })
            .to_vec(),
        options: GameOptions::default(),
    })
}

fn grant(player: PlayerId, seq: u32, target: PlayerId, slot: u8, amount: i32) -> Command {
    Command {
        player,
        seq,
        kind: CommandKind::Edit(Edit::TeamPool(
            target,
            TeamOp::ResourceAdd(slot),
            Fixed::from_int(amount),
        )),
    }
}

#[test]
fn grants_add_at_application_time_after_same_tick_cargo_write() {
    let mut sim = game();
    sim.economy.set_stock(
        PlayerId(0),
        [
            Fixed::from_int(11),
            Fixed::from_int(22),
            Fixed::ZERO,
            Fixed::from_int(44),
        ],
    );
    let commands = [
        Command {
            player: PlayerId::SCRIPT,
            seq: 0,
            kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 2, Fixed::from_int(7))),
        },
        grant(PlayerId::SCRIPT, 1, PlayerId(0), 2, 3),
        grant(PlayerId::SCRIPT, 2, PlayerId(0), 2, 4),
    ];
    let out = sim.step(&commands);
    assert!(
        !out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    assert_eq!(
        sim.economy.stock_of(PlayerId(0)),
        [11, 22, 14, 44].map(Fixed::from_int)
    );
}

#[test]
fn grants_require_authorized_issuer_configured_player_valid_slot_and_nonnegative_amount() {
    let mut sim = game();
    sim.economy
        .set_stock(PlayerId(0), [Fixed::ONE; crate::economy::RESOURCES]);
    let commands = [
        grant(PlayerId(0), 0, PlayerId(0), 0, 5),
        grant(PlayerId::SCRIPT, 0, PlayerId(9), 0, 5),
        grant(PlayerId::SCRIPT, 1, PlayerId(0), 4, 5),
        grant(PlayerId::SCRIPT, 2, PlayerId(0), 0, -1),
    ];
    let out = sim.step(&commands);
    let rejected: Vec<_> = out
        .events
        .iter()
        .filter(|e| matches!(e, SimEvent::Rejected { .. }))
        .collect();
    assert_eq!(rejected.len(), 4);
    assert_eq!(
        sim.economy.stock_of(PlayerId(0)),
        [Fixed::ONE; crate::economy::RESOURCES]
    );
    assert!(sim.economy.stock_of(PlayerId(1)).is_empty());
}

#[test]
fn script_and_bench_can_grant_configured_player_resources() {
    let mut sim = game();
    let out = sim.step(&[
        grant(PlayerId::SCRIPT, 0, PlayerId(1), 0, 8),
        grant(PlayerId::BENCH, 0, PlayerId(1), 3, 6),
    ]);
    assert!(
        !out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    assert_eq!(
        sim.economy.stock_of(PlayerId(1)),
        [8, 0, 0, 6].map(Fixed::from_int)
    );
}

#[test]
fn resource_sum_saturates_at_the_fixed_maximum() {
    let mut sim = game();
    sim.economy.set_stock(
        PlayerId(0),
        [
            Fixed::from_bits(i64::MAX - 1),
            Fixed::ZERO,
            Fixed::ZERO,
            Fixed::ZERO,
        ],
    );
    let command = Command {
        player: PlayerId::SCRIPT,
        seq: 0,
        kind: CommandKind::Edit(Edit::TeamPool(
            PlayerId(0),
            TeamOp::ResourceAdd(0),
            Fixed::from_bits(i64::MAX),
        )),
    };
    let out = sim.step(&[command]);
    assert!(
        !out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    assert_eq!(
        sim.economy.stock_of(PlayerId(0))[0],
        Fixed::from_bits(i64::MAX)
    );
}
