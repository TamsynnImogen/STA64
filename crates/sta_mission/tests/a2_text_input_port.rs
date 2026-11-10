//! Private converted tutorial objective-flow probe, using command text and an
//! empty fake-content world. This does not test the installed map, physical
//! tutorial actions, frontend input, audio/camera playback or native parity.
use sta_core::{PlayerId, TeamId};
use sta_mission::api::{MissionConfig, PlayerNumbering};
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{Command, CommandKind};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};
use std::path::PathBuf;
use std::sync::Arc;

#[test]
#[ignore = "requires external STA64_A2_PORTS and STA64_A2_OBJECTIVES folders"]
fn converted_ui_tutorial_next_text_completes_its_authored_objective_flow() {
    let ports = PathBuf::from(std::env::var("STA64_A2_PORTS").expect("set STA64_A2_PORTS"));
    let objectives =
        PathBuf::from(std::env::var("STA64_A2_OBJECTIVES").expect("set STA64_A2_OBJECTIVES"));
    let source = std::fs::read_to_string(ports.join("UI Tutorial.lua")).unwrap();
    // Retain the instance solely for observation, leaving its methods unchanged.
    let observed = format!("local register = MMM.register\nMMM.register = function(o) __port_instance = o; return register(o) end\n{source}").into_bytes();
    let setup = GameSetup {
        seed: 17,
        content: Arc::new(sta_content::fake::db()),
        map: Arc::new(sta_content::MapDef::empty()),
        players: (0..2)
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "fx".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let cfg = MissionConfig {
        player_numbering: PlayerNumbering::ArmadaTeams,
        view: Some((*sim.view()).clone()),
        ..MissionConfig::default()
    };
    let mut runtime = MissionRuntime::load_with("missions/UI Tutorial.lua", 17, cfg, move |path| {
        if path == "missions/UI Tutorial.lua" {
            return Some(observed.clone());
        }
        let name = path.strip_prefix("objectives/")?;
        if name.contains('/') || name.contains('\\') {
            return None;
        }
        std::fs::read(objectives.join(name)).ok()
    });
    let mut commands = Vec::new();
    runtime.think(&sim.view(), &[], &mut commands);
    for _ in 0..530 {
        let tick = sim.step(&commands);
        assert!(
            !tick
                .events
                .iter()
                .any(|e| matches!(e, SimEvent::Rejected { .. })),
            "{:?}",
            tick.events
        );
        commands.clear();
        runtime.think(&tick.view, &tick.events, &mut commands);
    }
    assert_eq!(
        runtime
            .eval::<u32>("return __port_instance.tutorialStep")
            .unwrap(),
        1
    );
    assert!(
        runtime
            .eval::<bool>("return __port_instance.awaitingAdvance")
            .unwrap()
    );
    let mut seen = vec![runtime.objective_briefing().unwrap().file.clone()];
    for expected in 2..=15 {
        commands.push(Command {
            player: PlayerId(0),
            seq: expected,
            kind: CommandKind::MissionText {
                text: if expected % 2 == 0 { "nExT" } else { "NEXT" }.into(),
            },
        });
        let tick = sim.step(&commands);
        assert!(
            !tick
                .events
                .iter()
                .any(|e| matches!(e, SimEvent::Rejected { .. })),
            "{:?}",
            tick.events
        );
        assert_eq!(
            tick.events
                .iter()
                .filter(|e| matches!(e, SimEvent::MissionText { .. }))
                .count(),
            1
        );
        commands.clear();
        runtime.think(&tick.view, &tick.events, &mut commands);
        assert_eq!(
            runtime
                .eval::<u32>("return __port_instance.tutorialStep")
                .unwrap(),
            expected
        );
        if expected < 15 {
            let file = runtime.objective_briefing().unwrap().file.clone();
            assert_ne!(seen.last(), Some(&file));
            seen.push(file);
            assert!(
                runtime
                    .eval::<bool>("return __port_instance.awaitingAdvance")
                    .unwrap()
            );
            assert!(!runtime.finished());
        }
    }
    assert_eq!(seen.len(), 14);
    assert!(
        runtime.finished(),
        "authored end-rule monitor must finish the mission"
    );
    assert!(
        !runtime
            .eval::<bool>("return __port_instance.awaitingAdvance")
            .unwrap()
    );
    assert!(runtime.errors().is_empty(), "{:?}", runtime.errors());
    assert!(
        !runtime
            .unsupported()
            .iter()
            .any(|(name, _)| name.contains("TextInput"))
    );
    eprintln!(
        "converted UI Tutorial: 14 objective files, 14 accepted text commands, authored step15/Mission.finish; remaining unsupported={:?}",
        runtime.unsupported()
    );
}
