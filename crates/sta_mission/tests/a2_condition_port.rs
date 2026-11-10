//! Private ignored check of the unchanged external UI Tutorial port.
//! This covers setup and early timed rules only, not tutorial completion.

use std::path::PathBuf;

use sta_mission::api::{MissionConfig, PlayerNumbering};
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::present::Presentation;
use sta_sim::setup::Controller;
use sta_sim::view::WorldView;

#[test]
#[ignore = "requires STA64_A2_PORTS pointing to the external a2/missions folder"]
fn ui_tutorial_setup_and_early_timed_rules_emit_expected_presentations() {
    let root = PathBuf::from(std::env::var("STA64_A2_PORTS").expect("set STA64_A2_PORTS"));
    let source = std::fs::read_to_string(root.join("UI Tutorial.lua")).expect("read external UI Tutorial.lua");
    let observed = format!(
        "local register = MMM.register\n\
         MMM.register = function(object) __port_instance = object; return register(object) end\n\
         {source}"
    );
    let config = MissionConfig {
        player_numbering: PlayerNumbering::ArmadaTeams,
        view: Some(WorldView::default()),
        ..MissionConfig::default()
    };
    let mut runtime = MissionRuntime::from_source_with("missions/UI Tutorial.lua", &observed, 1, config);
    let mut presentations = Vec::new();

    for tick in 0..=400 {
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

        if tick == 0 {
            assert_eq!(runtime.eval::<u32>("return #__port_instance.ruleMonitors").unwrap(), 7);
            assert_eq!(
                runtime.eval::<u32>("return __port_instance.state['p_INTRO']").unwrap(),
                1
            );
            assert_eq!(count_call(&presentations, 0, "UI.addSubtitle"), 1);
        }
        if tick == 1 {
            assert_eq!(
                runtime.eval::<u32>("return __port_instance.state['p_INTRO']").unwrap(),
                0
            );
            assert!(runtime.eval::<bool>("return __port_instance.introRunning").unwrap());
            assert_eq!(count_music(&presentations, 1), 1);
        }
        if tick == 91 || tick == 211 {
            assert_eq!(count_call(&presentations, tick, "UI.addTitle"), 1, "tick {tick}");
        }
        if tick == 181 {
            assert_eq!(count_sound(&presentations, 181), 1);
        }
        if tick == 211 {
            assert_eq!(count_call(&presentations, 211, "UI.addSubtitle"), 2);
            assert_eq!(count_call_through(&presentations, 211, "UI.addSubtitle"), 3);
        }
        if tick == 271 {
            assert_eq!(count_call(&presentations, 271, "UI.addSubtitle"), 1);
            assert_eq!(count_call_through(&presentations, 271, "UI.addSubtitle"), 4);
        }
        if tick == 361 {
            assert_eq!(count_call(&presentations, 361, "UI.addSubtitle"), 1);
            assert_eq!(count_call_through(&presentations, 361, "UI.addSubtitle"), 5);
        }
    }

    assert!(runtime.errors().is_empty(), "Lua errors: {:?}", runtime.errors());
    assert_eq!(runtime.eval::<u32>("return __port_instance.tutorialStep").unwrap(), 0);
    assert!(!runtime.eval::<bool>("return __port_instance.tutorialStarted").unwrap());
    assert_eq!(count_call_through(&presentations, 400, "UI.addTitle"), 2);
    assert_eq!(count_call_through(&presentations, 400, "UI.addSubtitle"), 5);
    let unsupported = runtime.unsupported();
    assert!(unsupported.contains(&("stub global Debug".into(), 401)));
    assert!(unsupported.contains(&("stub field Debug.getKeyState".into(), 401)));
    assert!(!unsupported.iter().any(|(name, _)| name.contains("TextInput")));
    let unsupported_total: u32 = unsupported.iter().map(|(_, count)| count).sum();
    eprintln!(
        "external UI Tutorial early rules: unsupported lookup/operation count {unsupported_total} across {} names: {unsupported:?}; fallback diagnostics do not implement input",
        unsupported.len()
    );
}

fn count_call(presentations: &[(u64, Presentation)], tick: u64, name: &str) -> usize {
    presentations
        .iter()
        .filter(|(at, presentation)| {
            *at == tick && matches!(presentation, Presentation::Call { name: call, .. } if call == name)
        })
        .count()
}

fn count_call_through(presentations: &[(u64, Presentation)], tick: u64, name: &str) -> usize {
    presentations
        .iter()
        .filter(|(at, presentation)| {
            *at <= tick && matches!(presentation, Presentation::Call { name: call, .. } if call == name)
        })
        .count()
}

fn count_sound(presentations: &[(u64, Presentation)], tick: u64) -> usize {
    presentations
        .iter()
        .filter(|(at, presentation)| *at == tick && matches!(presentation, Presentation::Sound { .. }))
        .count()
}

fn count_music(presentations: &[(u64, Presentation)], tick: u64) -> usize {
    presentations
        .iter()
        .filter(|(at, presentation)| *at == tick && matches!(presentation, Presentation::Music { .. }))
        .count()
}
