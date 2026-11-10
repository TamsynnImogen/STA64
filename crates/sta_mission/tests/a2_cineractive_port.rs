//! Private ignored check of the external UI Tutorial cineractive transition.
//! This covers its first tutorial step only, not mission completion or client playback.

use std::path::PathBuf;

use sta_mission::api::{MissionConfig, PlayerNumbering};
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::present::Presentation;
use sta_sim::setup::Controller;
use sta_sim::view::WorldView;

const OBJECTIVE_FILE: &str = "a2_tutorial1_zoom.txt";

#[test]
#[ignore = "requires external STA64_A2_PORTS and STA64_A2_OBJECTIVES folders"]
fn ui_tutorial_cineractive_finish_loads_its_first_objective_step() {
    let mission_root = PathBuf::from(std::env::var("STA64_A2_PORTS").expect("set STA64_A2_PORTS"));
    let objective_root = PathBuf::from(std::env::var("STA64_A2_OBJECTIVES").expect("set STA64_A2_OBJECTIVES"));
    let source = std::fs::read_to_string(mission_root.join("UI Tutorial.lua")).expect("read external UI Tutorial.lua");
    let observed = format!(
        "local register = MMM.register\n\
         MMM.register = function(object) __port_instance = object; return register(object) end\n\
         {source}"
    )
    .into_bytes();
    let config = MissionConfig {
        player_numbering: PlayerNumbering::ArmadaTeams,
        view: Some(WorldView::default()),
        ..MissionConfig::default()
    };
    let mut runtime = MissionRuntime::load_with("missions/UI Tutorial.lua", 1, config, move |path| {
        if path == "missions/UI Tutorial.lua" {
            return Some(observed.clone());
        }
        let name = path.strip_prefix("objectives/").unwrap_or(path);
        (name == OBJECTIVE_FILE)
            .then(|| std::fs::read(objective_root.join(OBJECTIVE_FILE)).ok())
            .flatten()
    });
    let mut presentations = Vec::new();

    for tick in 0..=530 {
        let view = WorldView {
            tick,
            ..WorldView::default()
        };
        let mut commands = Vec::<Command>::new();
        runtime.think(&view, &[], &mut commands);
        for command in commands {
            if let CommandKind::Edit(Edit::Present(presentation)) = command.kind {
                presentations.push((tick, presentation));
            }
        }

        if tick == 510 {
            assert_eq!(runtime.eval::<u32>("return __port_instance.tutorialStep").unwrap(), 0);
            assert!(!runtime.eval::<bool>("return __port_instance.tutorialStarted").unwrap());
        }
        if tick == 511 {
            assert_eq!(runtime.eval::<u32>("return __port_instance.tutorialStep").unwrap(), 1);
            assert!(runtime.eval::<bool>("return __port_instance.tutorialStarted").unwrap());
            assert!(!runtime.eval::<bool>("return __port_instance.introRunning").unwrap());
            assert!(runtime.eval::<bool>("return __port_instance.awaitingAdvance").unwrap());
            assert!(!runtime.eval::<bool>("return Cineractive.active").unwrap());
            assert!(runtime.eval::<bool>("return Objectives.visible").unwrap());
            assert_eq!(runtime.objectives().len(), 3);
            assert_eq!(
                runtime.objective_briefing().unwrap().file,
                format!("objectives/{OBJECTIVE_FILE}")
            );
            assert_eq!(count_cineractive_off(&presentations, 511), 1);
        }
        if tick == 530 {
            assert_eq!(runtime.eval::<u32>("return __port_instance.tutorialStep").unwrap(), 1);
            assert!(runtime.eval::<bool>("return __port_instance.tutorialStarted").unwrap());
            assert!(!runtime.finished());
        }
    }

    assert!(runtime.errors().is_empty(), "Lua errors: {:?}", runtime.errors());
    let unsupported = runtime.unsupported();
    assert!(unsupported.contains(&("stub global Debug".into(), 531)));
    assert!(unsupported.contains(&("stub field Debug.getKeyState".into(), 531)));
    assert!(unsupported.contains(&("stub global TextInput".into(), 1)));
    assert!(unsupported.contains(&("stub field TextInput.hook".into(), 1)));
    eprintln!("external UI Tutorial cineractive transition: unsupported diagnostics={unsupported:?}");
}

fn count_cineractive_off(presentations: &[(u64, Presentation)], tick: u64) -> usize {
    presentations
        .iter()
        .filter(|(at, presentation)| *at == tick && matches!(presentation, Presentation::Cineractive { on: false, .. }))
        .count()
}
