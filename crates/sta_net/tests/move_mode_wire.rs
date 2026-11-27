//! The move mode of `Order::Move` (SIM-76 seam, ADR 0011 G4) on the wire: a
//! turn record carrying a move order round-trips with each mode and without
//! one, and a peer on the protocol before the mode is refused by version.

use sta_core::{EntityId, PlayerId, Vec3};
use sta_net::proto::{decode, encode, Message, TurnCommands, PROTOCOL_VERSION};
use sta_sim::command::{Command, CommandKind, MoveMode, Order};

fn turn(mode: Option<MoveMode>) -> Message {
    Message::Turn(TurnCommands {
        turn: 3,
        slot: PlayerId(0),
        commands: vec![Command {
            player: PlayerId(0),
            seq: 1,
            kind: CommandKind::Order(Order::Move { units: vec![EntityId(4), EntityId(5)], to: Vec3::from_f32([1.5, 0.0, -2.0]), queue: true, mode }),
        }],
        schedule: Vec::new(),
        digest: None,
    })
}

#[test]
fn a_move_order_round_trips_with_each_mode_and_without() {
    for mode in [None, Some(MoveMode::Formation), Some(MoveMode::Group)] {
        let m = turn(mode);
        let bytes = encode(&m).expect("encodes");
        assert_eq!(decode(&bytes), Ok(m), "{mode:?}");
    }
    assert_ne!(encode(&turn(Some(MoveMode::Formation))).unwrap(), encode(&turn(Some(MoveMode::Group))).unwrap());
    assert_ne!(encode(&turn(None)).unwrap(), encode(&turn(Some(MoveMode::Group))).unwrap());
}

#[test]
fn a_frame_of_the_previous_protocol_is_refused() {
    let mut bytes = encode(&turn(None)).unwrap();
    bytes[4..6].copy_from_slice(&(PROTOCOL_VERSION - 1).to_le_bytes());
    assert!(decode(&bytes).is_err());
}
