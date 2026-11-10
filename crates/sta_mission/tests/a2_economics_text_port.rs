//! Private ignored check of the external Economics Tutorial text-input flow.
//! Uses a blank fake-content world; it does not validate an installed map,
//! physical tutorial actions or client playback.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use sta_core::{PlayerId, TeamId};
use sta_mission::api::{MissionConfig, PlayerNumbering};
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{Command, CommandKind};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

const MISSION_PATH: &str = "missions/Economics Tutorial.lua";
const OBJECTIVE_FILES: [&str; 9] = [
    "a2_tutorial2_text_minstation.txt",
    "a2_tutorial2_text_latneb.txt",
    "a2_tutorial2_text_metal.txt",
    "a2_tutorial2_text_colonyship.txt",
    "a2_tutorial2_text_colonize.txt",
    "a2_tutorial2_text_trastation.txt",
    "a2_tutorial2_text_buymetal.txt",
    "a2_tutorial2_text_trading.txt",
    "a2_tutorial2_text_crew.txt",
];

#[test]
#[ignore = "requires external STA64_A2_PORTS and STA64_A2_OBJECTIVES folders"]
fn economics_tutorial_text_commands_advance_authored_steps_and_finish_on_timer() {
    let ports = PathBuf::from(std::env::var("STA64_A2_PORTS").expect("set STA64_A2_PORTS"));
    let objective_root = PathBuf::from(std::env::var("STA64_A2_OBJECTIVES").expect("set STA64_A2_OBJECTIVES"));
    let source =
        std::fs::read_to_string(ports.join("Economics Tutorial.lua")).expect("read external Economics Tutorial.lua");
    // Observe the registered instance without changing any of its methods.
    let observed = format!(
        "local register = MMM.register\n\
         MMM.register = function(object) __port_instance = object; return register(object) end\n\
         {source}"
    )
    .into_bytes();
    let objective_bytes = OBJECTIVE_FILES
        .iter()
        .map(|name| {
            let name = *name;
            (name.to_owned(), std::fs::read(objective_root.join(name)).expect(name))
        })
        .collect::<BTreeMap<_, _>>();

    let setup = GameSetup {
        seed: 29,
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
    let config = MissionConfig {
        seat: Some(PlayerId(0)),
        player_numbering: PlayerNumbering::ArmadaTeams,
        view: Some((*sim.view()).clone()),
        ..MissionConfig::default()
    };
    let mut runtime = MissionRuntime::load_with(MISSION_PATH, 29, config, move |path| {
        if path == MISSION_PATH {
            return Some(observed.clone());
        }
        let name = path.strip_prefix("objectives/").unwrap_or(path);
        if name.contains('/') || name.contains('\\') {
            return None;
        }
        objective_bytes.get(name).cloned()
    });
    let mut commands = Vec::new();
    runtime.think(&sim.view(), &[], &mut commands);
    assert_no_lua_errors(&runtime);

    // The authored 12-second opening timer naturally shows step one.
    for _ in 0..361 {
        tick(&mut runtime, &mut sim, &mut commands);
        if step_number(&runtime) == 1 {
            break;
        }
    }
    assert_eq!(step_number(&runtime), 1, "opening timer must show step one");
    assert!(runtime.eval::<bool>("return __port_instance.awaitingAdvance").unwrap());
    let mut seen_files = vec![current_briefing_file(&runtime)];
    assert_eq!(seen_files[0], format!("objectives/{}", OBJECTIVE_FILES[0]));

    // A valid mission-text event from another seat and unmatched text from
    // the mission seat both leave the authored step unchanged.
    let events = send_text(&mut runtime, &mut sim, &mut commands, PlayerId(1), 1, "NEXT");
    assert_eq!(mission_text_events(&events), 1);
    assert_eq!(step_number(&runtime), 1);
    assert_eq!(current_briefing_file(&runtime), seen_files[0]);
    let events = send_text(&mut runtime, &mut sim, &mut commands, PlayerId(0), 2, "continue");
    assert_eq!(mission_text_events(&events), 1);
    assert_eq!(step_number(&runtime), 1);
    assert_eq!(current_briefing_file(&runtime), seen_files[0]);

    // Nine accepted, seat-zero text commands advance through all nine authored
    // files and into the finish routine without calling a mission helper.
    for (index, expected_step) in (2usize..=10).enumerate() {
        let events = send_text(
            &mut runtime,
            &mut sim,
            &mut commands,
            PlayerId(0),
            index as u32 + 3,
            "nExT",
        );
        assert_eq!(mission_text_events(&events), 1);
        assert_eq!(step_number(&runtime), expected_step);
        assert!(!runtime.finished(), "Mission.finish is timer-delayed");
        if expected_step <= 9 {
            let file = current_briefing_file(&runtime);
            assert_eq!(file, format!("objectives/{}", OBJECTIVE_FILES[expected_step - 1]));
            assert_ne!(seen_files.last(), Some(&file));
            seen_files.push(file);
        } else {
            assert_eq!(
                current_briefing_file(&runtime),
                format!("objectives/{}", OBJECTIVE_FILES[8])
            );
        }
        assert_no_lua_errors(&runtime);
    }
    assert_eq!(seen_files.len(), 9);
    assert!(runtime.eval::<bool>("return __port_instance.finished").unwrap());
    assert!(!runtime.finished(), "Mission.finish waits for its authored timer");

    // The final authored timer is eight seconds; verify it is not early and
    // then allow a small bounded margin for the runtime's next-tick dispatch.
    for _ in 0..239 {
        tick(&mut runtime, &mut sim, &mut commands);
    }
    assert!(!runtime.finished(), "the authored eight-second timer has not elapsed");
    for _ in 0..2 {
        if runtime.finished() {
            break;
        }
        tick(&mut runtime, &mut sim, &mut commands);
    }
    assert!(
        runtime.finished(),
        "authored timer should call Mission.finish within 241 ticks"
    );
    assert_no_lua_errors(&runtime);
    eprintln!(
        "external Economics Tutorial: {} distinct objective files, 9 accepted seat-zero MissionText events, wrong-seat/unmatched events ignored; unsupported={:?}",
        seen_files.len(),
        runtime.unsupported()
    );
}

fn tick(runtime: &mut MissionRuntime, sim: &mut Sim, commands: &mut Vec<Command>) -> Vec<SimEvent> {
    let batch = std::mem::take(commands);
    let output = sim.step(&batch);
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. })),
        "rejected simulation commands: {:?}",
        output.events
    );
    runtime.think(&output.view, &output.events, commands);
    assert_no_lua_errors(runtime);
    output.events
}

fn send_text(
    runtime: &mut MissionRuntime,
    sim: &mut Sim,
    commands: &mut Vec<Command>,
    player: PlayerId,
    seq: u32,
    text: &str,
) -> Vec<SimEvent> {
    commands.push(Command {
        player,
        seq,
        kind: CommandKind::MissionText { text: text.into() },
    });
    tick(runtime, sim, commands)
}

fn mission_text_events(events: &[SimEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, SimEvent::MissionText { .. }))
        .count()
}

fn step_number(runtime: &MissionRuntime) -> usize {
    runtime.eval::<usize>("return __port_instance.step").unwrap()
}

fn current_briefing_file(runtime: &MissionRuntime) -> String {
    runtime.objective_briefing().expect("step objective briefing").file
}

fn assert_no_lua_errors(runtime: &MissionRuntime) {
    assert!(
        runtime.errors().is_empty(),
        "uncaught Lua errors: {:?}",
        runtime.errors()
    );
}
