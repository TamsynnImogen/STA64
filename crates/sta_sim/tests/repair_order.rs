//! Public wire, replay, and rejection contract for the pending Repair order.

use std::sync::Arc;

use serde::Serialize;
use sta_content::{MapDef, MapObject};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{AutonomyKind, AutonomyLevel, Command, CommandKind, Order, Target};
use sta_sim::replay::{Recorder, Replay};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

fn point(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

fn setup() -> GameSetup {
    let content = Arc::new(sta_content::fake::db());
    let map = MapDef {
        objects: [0, 500, 1000]
            .into_iter()
            .map(|x| MapObject {
                odf: "fx_cruiser".into(),
                owner: Some(0),
                pos: point(x, 0),
                heading: (Fixed::ZERO, Fixed::ONE),
                label: None,
            })
            .collect(),
        ..MapDef::empty()
    };
    GameSetup {
        seed: 914,
        map: Arc::new(map),
        content,
        players: (0..2)
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "fx".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    }
}

fn order_command(player: PlayerId, seq: u32, order: Order) -> Command {
    Command {
        player,
        seq,
        kind: CommandKind::Order(order),
    }
}

fn prefixed<T: Serialize>(tag: u8, payload: &T) -> Vec<u8> {
    let mut bytes = vec![tag];
    bytes.extend(postcard::to_allocvec(payload).unwrap());
    bytes
}

#[test]
fn existing_order_tags_and_payload_field_order_are_frozen() {
    let id = EntityId(0x1234);
    let what = ContentKey(0x21);
    macro_rules! wire {
        ($tag:literal, $order:expr, $payload:expr) => {{
            let order = $order;
            let expected = prefixed($tag, &$payload);
            assert_eq!(
                postcard::to_allocvec(&order).unwrap(),
                expected,
                "{order:?}"
            );
            assert_eq!(postcard::from_bytes::<Order>(&expected).unwrap(), order);
        }};
    }

    wire!(
        0,
        Order::Move {
            units: vec![id],
            to: point(1, 2),
            queue: true
        },
        (vec![id], point(1, 2), true)
    );
    wire!(1, Order::Stop { units: vec![id] }, (vec![id],));
    wire!(
        2,
        Order::Attack {
            units: vec![id],
            target: EntityId(2)
        },
        (vec![id], EntityId(2))
    );
    wire!(
        3,
        Order::Build {
            builder: id,
            what,
            at: Some(point(3, 4))
        },
        (id, what, Some(point(3, 4)))
    );
    wire!(
        4,
        Order::Formation {
            units: vec![id],
            kind: 2
        },
        (vec![id], 2u8)
    );
    wire!(
        5,
        Order::Special {
            units: vec![id],
            slot: 1,
            target: Target::Entity(EntityId(2))
        },
        (vec![id], 1u8, Target::Entity(EntityId(2)))
    );
    wire!(
        6,
        Order::Cloak {
            units: vec![id],
            on: true
        },
        (vec![id], true)
    );
    wire!(
        7,
        Order::Mine {
            units: vec![id],
            node: EntityId(2)
        },
        (vec![id], EntityId(2))
    );
    wire!(
        8,
        Order::Rally {
            builders: vec![id],
            at: Target::Point(point(5, 6))
        },
        (vec![id], Target::Point(point(5, 6)))
    );
    wire!(
        9,
        Order::Autonomy {
            units: vec![id],
            kind: AutonomyKind::Movement,
            level: AutonomyLevel::High
        },
        (vec![id], AutonomyKind::Movement, AutonomyLevel::High)
    );
    wire!(
        10,
        Order::Patrol {
            units: vec![id],
            points: vec![point(7, 8), point(9, 10)]
        },
        (vec![id], vec![point(7, 8), point(9, 10)])
    );
    wire!(11, Order::SearchAndDestroy { units: vec![id] }, (vec![id],));
    wire!(12, Order::Scout { units: vec![id] }, (vec![id],));
    wire!(13, Order::AttackAny { units: vec![id] }, (vec![id],));
    wire!(
        14,
        Order::AttackPoint {
            units: vec![id],
            at: point(11, 12)
        },
        (vec![id], point(11, 12))
    );
    wire!(
        15,
        Order::MoveWarp {
            units: vec![id],
            to: point(13, 14),
            queue: false
        },
        (vec![id], point(13, 14), false)
    );
}

fn repair_command(yard: Option<EntityId>) -> Command {
    // Deliberately out of ID order: transport must retain caller vector order.
    Command {
        player: PlayerId(1),
        seq: 0x1234_5678,
        kind: CommandKind::Order(Order::Repair {
            units: vec![
                EntityId(0x1234),
                EntityId(7),
                EntityId(0x1234),
                EntityId(0x5678),
            ],
            yard,
        }),
    }
}

#[test]
fn repair_tag16_payload_and_command_round_trip_for_both_yard_options() {
    let cases = [
        (
            None,
            vec![16, 4, 0xb4, 0x24, 7, 0xb4, 0x24, 0xf8, 0xac, 1, 0],
        ),
        (
            Some(EntityId(0x4321)),
            vec![
                16, 4, 0xb4, 0x24, 7, 0xb4, 0x24, 0xf8, 0xac, 1, 1, 0xa1, 0x86, 1,
            ],
        ),
    ];
    for (yard, expected_order_bytes) in cases {
        let command = repair_command(yard);
        let CommandKind::Order(order) = &command.kind else {
            unreachable!()
        };
        assert_eq!(postcard::to_allocvec(order).unwrap(), expected_order_bytes);

        let decoded: Command =
            postcard::from_bytes(&postcard::to_allocvec(&command).unwrap()).unwrap();
        assert_eq!(decoded, command);
        assert_eq!(decoded.player, PlayerId(1));
        assert_eq!(decoded.seq, 0x1234_5678);
    }
}

#[test]
fn repair_none_and_some_yard_survive_replay_round_trip() {
    let setup = setup();
    for yard in [None, Some(EntityId(0x4321))] {
        let command = repair_command(yard);
        let mut recorder = Recorder::with_interval(&setup, 1);
        recorder.step(std::slice::from_ref(&command));
        let replay = Replay::decode(&recorder.finish().encode()).unwrap();
        assert_eq!(replay.commands.len(), 1);
        assert_eq!(replay.commands[0].command, command);
        assert_eq!(replay.play(setup.content.clone()).unwrap().tick, 1);
    }
}

fn cruisers(sim: &Sim) -> Vec<EntityId> {
    let key = sim.content().unwrap().key("fx_cruiser").unwrap();
    sim.view()
        .units
        .iter()
        .filter(|u| u.def == key)
        .map(|u| u.id)
        .collect()
}

#[test]
fn rejected_repair_keeps_existing_queued_course_and_sim_state() {
    let setup = setup();
    let ids = cruisers(&Sim::from_setup(&setup));
    assert!(ids.len() >= 3);
    let prior_orders = [
        order_command(
            PlayerId(0),
            1,
            Order::Move {
                units: vec![ids[0]],
                to: point(4000, 0),
                queue: false,
            },
        ),
        order_command(
            PlayerId(0),
            2,
            Order::Move {
                units: vec![ids[0]],
                to: point(5000, 0),
                queue: true,
            },
        ),
    ];

    for units in [vec![ids[0], ids[1]], Vec::new()] {
        for yard in [None, Some(ids[2])] {
            let mut baseline = Sim::from_setup(&setup);
            let mut actual = Sim::from_setup(&setup);
            baseline.step(&prior_orders);
            actual.step(&prior_orders);

            let repair = order_command(
                PlayerId(0),
                3,
                Order::Repair {
                    units: units.clone(),
                    yard,
                },
            );
            let out = actual.step(std::slice::from_ref(&repair));
            baseline.step(&[]);
            assert!(out.events.contains(&SimEvent::Rejected {
                player: PlayerId(0),
                seq: 3
            }));
            assert_eq!(
                actual.digest(),
                baseline.digest(),
                "Repair changed digest or queued order state"
            );
            assert_eq!(
                actual.snapshot(),
                baseline.snapshot(),
                "Repair changed serialized sim state"
            );
        }
    }
}
