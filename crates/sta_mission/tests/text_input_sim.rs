//! Player text goes through real Sim validation before reaching Lua hooks.
use sta_core::{PlayerId, TeamId};
use sta_mission::api::MissionConfig;
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};
use std::sync::Arc;

#[test]
fn accepted_seat_text_runs_before_monitor_and_queues_replayable_presentation() {
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
    let mut runtime = MissionRuntime::from_source_with(
        "text-input-sim.lua",
        r#"
        owner = { count = 0 }
        TextInput:hook('next', owner, function(self, text, arg)
            assert(arg == nil)
            self.count = self.count + 1
            UI:addSubtitle(text, 1, 2, 3, false)
        end, nil, '^[Nn][Ee][Xx][Tt]$')
        m = Monitor('input-before-monitors', false, nil, function() return owner.count > 0 end, nil)
        m:hook('finish', MonitorHook.Type.OnTrue, nil, function() Mission:finish(true) end, nil)
    "#,
        17,
        MissionConfig {
            view: Some((*sim.view()).clone()),
            ..MissionConfig::default()
        },
    );
    assert!(runtime.errors().is_empty(), "{:?}", runtime.errors());
    let commands = [
        Command {
            player: PlayerId(1),
            seq: 0,
            kind: CommandKind::MissionText {
                text: "NEXT".into(),
            },
        },
        Command {
            player: PlayerId(0),
            seq: 0,
            kind: CommandKind::MissionText {
                text: "nExT".into(),
            },
        },
        Command {
            player: PlayerId::SCRIPT,
            seq: 0,
            kind: CommandKind::MissionText {
                text: "NEXT".into(),
            },
        },
    ];
    let tick = sim.step(&commands);
    assert_eq!(
        tick.events
            .iter()
            .filter(|e| matches!(e, SimEvent::MissionText { .. }))
            .count(),
        2
    );
    assert!(tick.events.contains(&SimEvent::Rejected {
        player: PlayerId::SCRIPT,
        seq: 0
    }));
    let mut out = Vec::new();
    runtime.think(&tick.view, &tick.events, &mut out);
    assert_eq!(runtime.eval::<u32>("return owner.count").unwrap(), 1);
    assert!(
        runtime.finished(),
        "text callback must precede condition-monitor poll"
    );
    assert!(!out.is_empty());
    assert!(
        out.iter().all(|c| c.player == PlayerId::SCRIPT
            && matches!(c.kind, CommandKind::Edit(Edit::Present(_))))
    );
    assert!(
        !sim.step(&out)
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    assert!(runtime.errors().is_empty(), "{:?}", runtime.errors());
}
