//! Spawn receipts correlate commands with actual allocation, including failure
//! and same-tick destruction. These fixtures contain no game content.

use std::sync::Arc;

use sta_content::{MapDef, MapObject};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, Vec3};

use crate::command::{Command, CommandKind, Edit};
use crate::setup::{GameOptions, GameSetup};
use crate::{Sim, SimEvent, WorldSource};

fn scene() -> Sim {
    let mut map = MapDef::empty();
    map.objects.push(MapObject {
        odf: "fx_moon".into(),
        owner: None,
        pos: Vec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        label: None,
    });
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 13,
        map: Arc::new(map),
        content: Arc::new(sta_content::fake::db()),
        players: Vec::new(),
        options: GameOptions::default(),
    });
    // Map setup queues ordinary Spawned notifications for its first tick.
    // Drain those before checking the events of our command batches.
    sim.step(&[]);
    sim
}

fn position(x: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::ZERO,
    }
}

fn request(sim: &Sim, token: u64, seq: u32, x: i32) -> Command {
    Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::SpawnRequest {
            request: token,
            odf: sim.content().unwrap().key("fx_cruiser").unwrap(),
            player: PlayerId(0),
            pos: position(x),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    }
}

fn ordinary(c: &Command) -> Command {
    let CommandKind::Edit(Edit::SpawnRequest {
        odf,
        player,
        pos,
        heading,
        ..
    }) = c.kind
    else {
        unreachable!()
    };
    Command {
        player: c.player,
        seq: c.seq,
        kind: CommandKind::Edit(Edit::Spawn {
            odf,
            player,
            pos,
            heading,
        }),
    }
}

#[test]
fn interleaved_spawns_report_actual_ids_and_replay_from_serialized_commands() {
    let mut sim = scene();
    let mut replay = sim.clone();
    let first = request(&sim, 91, 1, 100);
    let unrelated = ordinary(&request(&sim, 0, 2, 200));
    let second = request(&sim, u64::MAX, 3, 300);
    // Input order differs from execution order; an ordinary spawn is between
    // the two requests and the map already owns an entity ID.
    let commands = vec![second, unrelated, first];
    let bytes = postcard::to_allocvec(&commands).unwrap();
    let decoded: Vec<Command> = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, commands);
    let output = sim.step(&commands);
    assert_eq!(replay.step(&decoded), output);
    let receipts: Vec<_> = output
        .events
        .iter()
        .filter_map(|e| match e {
            SimEvent::ScriptSpawned { request, id } => Some((*request, *id)),
            _ => None,
        })
        .collect();
    assert_eq!(receipts.len(), 2);
    assert_eq!((receipts[0].0, receipts[1].0), (91, u64::MAX));
    assert_ne!(receipts[0].1, receipts[1].1);
    for ((token, id), x) in receipts.iter().zip([100, 300]) {
        let unit = output.view.unit(*id).unwrap();
        assert_eq!(unit.pos, position(x));
        assert_eq!(unit.owner, PlayerId(0));
        assert!(output.events.contains(&SimEvent::Spawned { id: *id }));
        assert!(output.events.windows(2).any(|pair| {
            pair[0] == SimEvent::Spawned { id: *id }
                && pair[1]
                    == SimEvent::ScriptSpawned {
                        request: *token,
                        id: *id,
                    }
        }));
    }
    assert_eq!(
        output
            .events
            .iter()
            .filter(|e| matches!(e, SimEvent::Spawned { .. }))
            .count(),
        3
    );
    assert!(!output.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    for _ in 0..30 {
        assert_eq!(sim.step(&[]), replay.step(&[]));
    }
    assert_eq!(sim.digest(), replay.digest());
}

#[test]
fn failed_definitions_and_unauthorized_sources_only_emit_correlated_rejections() {
    let mut sim = scene();
    let before = sim.view().units.len();
    let next = sim.units.next_id();
    let mut invalid = request(&sim, 12, 3, 100);
    if let CommandKind::Edit(Edit::SpawnRequest { odf, .. }) = &mut invalid.kind {
        *odf = ContentKey::NONE;
    }
    let mut player = request(&sim, 13, 4, 200);
    player.player = PlayerId(0);
    let mut bench = request(&sim, 14, 5, 300);
    bench.player = PlayerId::BENCH;
    let output = sim.step(&[bench, invalid, player]);
    assert_eq!(output.view.units.len(), before);
    assert!(
        !output
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Spawned { .. } | SimEvent::ScriptSpawned { .. }))
    );
    let rejected: Vec<_> = output
        .events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Rejected { player, seq } => Some((*player, *seq)),
            _ => None,
        })
        .collect();
    assert_eq!(
        rejected,
        [(PlayerId(0), 4), (PlayerId::SCRIPT, 3), (PlayerId::BENCH, 5)]
    );
    let output = sim.step(&[request(&sim, 20, 6, 400)]);
    assert!(
        output
            .events
            .contains(&SimEvent::ScriptSpawned { request: 20, id: next })
    );

    let mut no_content = Sim::new(13);
    let output = no_content.step(&[request(&sim, 15, 6, 100)]);
    assert_eq!(
        output.events,
        [SimEvent::Rejected {
            player: PlayerId::SCRIPT,
            seq: 6
        }]
    );
    assert_eq!(no_content.units.next_id(), EntityId(1));
}

#[test]
fn receipt_is_not_a_liveness_guarantee_and_tokens_do_not_change_physical_state() {
    let mut tagged = scene();
    let mut plain = tagged.clone();
    let create = request(&tagged, 42, 1, 100);
    // Fixture allocation is known: the map moon is 1. A consumer must use
    // the receipt, and must never use this fixture's ID prediction technique.
    let kill = Command {
        player: PlayerId::SCRIPT,
        seq: 2,
        kind: CommandKind::Edit(Edit::Kill(EntityId(2))),
    };
    let with_receipt = tagged.step(&[create.clone(), kill.clone()]);
    let without_receipt = plain.step(&[ordinary(&create), kill]);
    assert!(with_receipt.events.contains(&SimEvent::ScriptSpawned {
        request: 42,
        id: EntityId(2)
    }));
    assert!(with_receipt.events.contains(&SimEvent::Destroyed {
        id: EntityId(2),
        by: None
    }));
    assert!(with_receipt.view.unit(EntityId(2)).is_none());
    assert_eq!(with_receipt.view, without_receipt.view);
    let ordinary_events: Vec<_> = with_receipt
        .events
        .into_iter()
        .filter(|e| !matches!(e, SimEvent::ScriptSpawned { .. }))
        .collect();
    assert_eq!(ordinary_events, without_receipt.events);
    assert_eq!(tagged.digest(), plain.digest());
}

#[test]
fn new_variants_append_wire_tags_and_round_trip_without_moving_old_tags() {
    let old = Edit::Decommissionable(EntityId(1), true);
    assert_eq!(postcard::to_allocvec(&old).unwrap(), [17, 1, 1]);
    let old_event = SimEvent::Rejected {
        player: PlayerId::SCRIPT,
        seq: 1,
    };
    assert_eq!(postcard::to_allocvec(&old_event).unwrap(), [9, 254, 1]);
    let c = request(&scene(), u64::MAX, 1, 100);
    let CommandKind::Edit(edit) = c.kind else {
        unreachable!()
    };
    let bytes = postcard::to_allocvec(&edit).unwrap();
    assert_eq!(bytes[0], 18);
    assert_eq!(postcard::from_bytes::<Edit>(&bytes).unwrap(), edit);
    let receipt = SimEvent::ScriptSpawned {
        request: 42,
        id: EntityId(7),
    };
    let bytes = postcard::to_allocvec(&receipt).unwrap();
    assert_eq!(bytes, [12, 42, 7]);
    assert_eq!(postcard::from_bytes::<SimEvent>(&bytes).unwrap(), receipt);
}
