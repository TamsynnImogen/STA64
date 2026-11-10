//! Public contract for replayable mission text, without a UI input producer.
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, MAX_MISSION_TEXT_BYTES, Order};
use sta_sim::replay::{Recorder, Replay};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};
use std::sync::Arc;

fn setup() -> GameSetup {
    GameSetup {
        seed: 41,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(sta_content::fake::db()),
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
fn text(player: u8, seq: u32, value: &str) -> Command {
    Command {
        player: PlayerId(player),
        seq,
        kind: CommandKind::MissionText { text: value.into() },
    }
}
fn event(player: u8, seq: u32, value: &str) -> SimEvent {
    SimEvent::MissionText {
        player: PlayerId(player),
        seq,
        text: value.into(),
    }
}

#[test]
fn text_follows_command_order_once_and_leaves_physical_state_unchanged() {
    let setup = setup();
    let mut sim = Sim::from_setup(&setup);
    let mut control = Sim::from_setup(&setup);
    let commands = [
        text(1, 0, "third"),
        text(0, 9, "second"),
        text(0, 2, "NEXT"),
    ];
    let bytes = postcard::to_allocvec(&commands).unwrap();
    let decoded: Vec<Command> = postcard::from_bytes(&bytes).unwrap();
    let mut replay = Sim::from_setup(&setup);
    let out = sim.step(&commands);
    assert_eq!(
        replay.step(&decoded),
        out,
        "serialized commands must reproduce full events/view"
    );
    assert_eq!(
        out.events,
        [
            event(0, 2, "NEXT"),
            event(0, 9, "second"),
            event(1, 0, "third")
        ]
    );
    assert_eq!(out.view, control.step(&[]).view);
    assert!(sim.step(&[]).events.is_empty());
    // Repeated sequence values are a sender error, not sim-level deduplication.
    assert_eq!(
        sim.step(&[text(0, 2, "NEXT")]).events,
        [event(0, 2, "NEXT")]
    );
}

#[test]
fn unknown_reserved_and_oversized_senders_reject_with_no_text_event() {
    let mut sim = Sim::from_setup(&setup());
    let too_long = "é".repeat(MAX_MISSION_TEXT_BYTES / 2 + 1);
    let mut commands: Vec<_> = [2, 8, 253, 254, 255].map(|p| text(p, 7, "NEXT")).into();
    commands.push(text(0, 8, &too_long));
    let out = sim.step(&commands);
    assert_eq!(out.events.len(), commands.len());
    for cmd in commands {
        assert!(out.events.contains(&SimEvent::Rejected {
            player: cmd.player,
            seq: cmd.seq
        }));
    }
    assert_eq!(
        Sim::new(41).step(&[text(0, 0, "NEXT")]).events,
        [SimEvent::Rejected {
            player: PlayerId(0),
            seq: 0
        }]
    );
}

#[test]
fn utf8_byte_boundary_empty_and_embedded_nul_are_preserved() {
    let mut sim = Sim::from_setup(&setup());
    let boundary = "é".repeat(MAX_MISSION_TEXT_BYTES / 2);
    assert_eq!(
        sim.step(&[text(0, 0, &boundary), text(0, 1, ""), text(0, 2, "a\0b")])
            .events,
        [event(0, 0, &boundary), event(0, 1, ""), event(0, 2, "a\0b")]
    );
}

#[test]
fn eliminated_player_text_is_rejected() {
    let setup = setup();
    let mut sim = Sim::from_setup(&setup);
    let target = sim
        .spawn_def(
            setup.content.key("fx_frigate").unwrap(),
            PlayerId(0),
            Vec3::from_f32([2000.0, 0.0, 2350.0]),
            (Fixed::ZERO, Fixed::ONE),
        )
        .unwrap();
    let attacker = sim
        .spawn_def(
            setup.content.key("fx_cruiser").unwrap(),
            PlayerId(1),
            Vec3::from_f32([2000.0, 0.0, 2000.0]),
            (Fixed::ZERO, Fixed::ONE),
        )
        .unwrap();
    // Use combat destruction: direct script removal does not currently feed
    // the victory subsystem's lost-seat list.
    sim.step(&[Command {
        player: PlayerId(1),
        seq: 0,
        kind: CommandKind::Order(Order::Attack {
            units: vec![attacker],
            target,
        }),
    }]);
    for _ in 0..1500 {
        if sim.is_eliminated(PlayerId(0)) {
            break;
        }
        sim.step(&[]);
    }
    assert!(sim.is_eliminated(PlayerId(0)));
    assert_eq!(
        sim.step(&[text(0, 1, "NEXT")]).events,
        [SimEvent::Rejected {
            player: PlayerId(0),
            seq: 1
        }]
    );
}

#[test]
fn additive_tags_and_replay_preserve_text_commands() {
    assert_eq!(
        postcard::to_allocvec(&CommandKind::Edit(Edit::Kill(sta_core::EntityId(3)))).unwrap(),
        [1, 1, 3]
    );
    assert_eq!(
        postcard::to_allocvec(&CommandKind::MissionText { text: "N".into() }).unwrap(),
        [2, 1, b'N']
    );
    assert_eq!(
        postcard::to_allocvec(&SimEvent::ScriptSpawned {
            request: 1,
            id: sta_core::EntityId(2)
        })
        .unwrap(),
        [12, 1, 2]
    );
    let expected = event(0, 4, "N");
    let bytes = postcard::to_allocvec(&expected).unwrap();
    assert_eq!(bytes, [13, 0, 4, 1, b'N']);
    assert_eq!(postcard::from_bytes::<SimEvent>(&bytes).unwrap(), expected);
    let setup = setup();
    let mut recorder = Recorder::with_interval(&setup, 1);
    recorder.step(&[text(0, 0, "NEXT")]);
    recorder.step(&[text(1, 0, "next")]);
    let digest = recorder.sim().digest();
    let replay = Replay::decode(&recorder.finish().encode()).unwrap();
    assert_eq!(
        replay
            .commands
            .iter()
            .map(|entry| entry.command.clone())
            .collect::<Vec<_>>(),
        [text(0, 0, "NEXT"), text(1, 0, "next")]
    );
    assert_eq!(replay.play(setup.content).unwrap().digest(), digest);
}
