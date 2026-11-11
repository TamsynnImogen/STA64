//! LUA-05 controls exercised through the public mission host and shared Sim.

use std::sync::Arc;

use sta_content::{ConfinementOverrides, ContentDb, MapDef, MoveStats, Profile, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_mission::api::{MissionConfig, PlayerNumbering};
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::world::NEUTRAL;
use sta_sim::{Sim, SimEvent, WorldSource};

fn pos(x: i32) -> Vec3 {
    Vec3::from_f32([x as f32, 0.0, 0.0])
}

fn scene() -> (Sim, Arc<ContentDb>, EntityId, EntityId, EntityId) {
    let mut craft = UnitDef::new("control_craft", "craft");
    craft.hull = Fixed::from_int(100);
    craft.movement = MoveStats::from_f32(120.0, 120.0, 2.0, 2.0, 1.0);
    let mut disabled = craft.clone();
    disabled.odf = "control_disabled".into();
    disabled.confinement = ConfinementOverrides {
        capability: Some(false),
        ..ConfinementOverrides::default()
    };
    let mut resource = UnitDef::new("control_resource", "moon");
    resource.hull = Fixed::from_int(100);
    resource.resource = Some(0);
    let content = Arc::new(
        ContentDb::builder()
            .profile(Profile::Fo4)
            .unit(craft)
            .unit(disabled)
            .unit(resource)
            .build()
            .unwrap(),
    );
    let mut map = MapDef::empty();
    map.bounds = (Vec3::from_f32([-500.0; 3]), Vec3::from_f32([500.0; 3]));
    let setup = GameSetup {
        seed: 55,
        map: Arc::new(map),
        content: content.clone(),
        players: (0..8)
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
    let heading = (Fixed::ONE, Fixed::ZERO);
    let craft = sim
        .spawn_def(content.key("control_craft").unwrap(), PlayerId(0), pos(0), heading)
        .unwrap();
    let disabled = sim
        .spawn_def(content.key("control_disabled").unwrap(), PlayerId(0), pos(100), heading)
        .unwrap();
    let resource = sim
        .spawn_def(content.key("control_resource").unwrap(), NEUTRAL, pos(250), heading)
        .unwrap();
    (sim, content, craft, disabled, resource)
}

fn runtime(sim: &Sim, content: Arc<ContentDb>, numbering: PlayerNumbering) -> MissionRuntime {
    let rt = MissionRuntime::from_source_with(
        "missions/shared-controls.lua",
        "",
        55,
        MissionConfig {
            content: Some(content),
            view: Some((*sim.view()).clone()),
            player_numbering: numbering,
            ..MissionConfig::default()
        },
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

fn no_rejection(events: &[SimEvent]) {
    assert!(
        !events.iter().any(|event| matches!(event, SimEvent::Rejected { .. })),
        "{events:?}"
    );
}

#[test]
fn strict_a2_controls_reject_malformed_arguments_without_commands() {
    for numbering in [PlayerNumbering::Slots, PlayerNumbering::ArmadaTeams] {
        let (sim, content, craft, _, _) = scene();
        let mut rt = runtime(&sim, content, numbering);
        for script in [
            "A2.SetForceOntoMap()".to_owned(),
            "A2.SetForceOntoMap(0,true)".to_owned(),
            "A2.SetRealTeam(4294967295,1)".to_owned(),
            format!("A2.SetForceOntoMap({},1)", craft.0),
            format!("A2.SetForceOntoMap('{}',true)", craft.0),
            format!("A2.SetForceOntoMap({}.5,true)", craft.0),
            format!("A2.SetForceOntoMap({},true,false)", craft.0),
            "A2.SetRealTeam()".to_owned(),
            format!("A2.SetRealTeam({},'1')", craft.0),
            format!("A2.SetRealTeam({},1.5)", craft.0),
            format!("A2.SetRealTeam({},-1)", craft.0),
            format!("A2.SetRealTeam({},254)", craft.0),
            format!("A2.SetRealTeam({},9)", craft.0),
            format!("A2.SetRealTeam({},1,0)", craft.0),
        ] {
            assert!(rt.eval::<()>(&script).is_err(), "{script}");
            assert!(rt.take_commands().is_empty(), "{script} queued a command");
        }
        assert!(
            rt.eval::<()>("local h=EntityFinder():findOne({player=0}); h.forceOntoMap=1")
                .is_err()
        );
        assert!(rt.take_commands().is_empty());
    }
}

#[test]
fn feedback_ignores_unsubmitted_same_view_and_foreign_rejections() {
    let (mut sim, content, craft, disabled, _) = scene();
    let mut rt = runtime(&sim, content, PlayerNumbering::Slots);
    rt.eval::<()>(&format!(
        "A2.SetForceOntoMap({},true); A2.SetForceOntoMap({},false)",
        disabled.0, craft.0
    ))
    .unwrap();
    let same_view = sim.view();
    let stale = [SimEvent::Rejected {
        player: PlayerId::SCRIPT,
        seq: 1,
    }];
    let mut commands = Vec::new();
    rt.think(&same_view, &stale, &mut commands);
    assert!(rt.errors().is_empty());
    assert_eq!(commands.len(), 2);
    rt.think(&same_view, &stale, &mut Vec::new());
    assert!(rt.errors().is_empty());
    let out = sim.step(&commands);
    let mut events = out.events.clone();
    events.extend([
        SimEvent::Rejected {
            player: PlayerId(0),
            seq: commands[1].seq,
        },
        SimEvent::Rejected {
            player: PlayerId::SCRIPT,
            seq: u32::MAX,
        },
    ]);
    rt.think(&out.view, &events, &mut Vec::new());
    assert_eq!(rt.errors().len(), 1, "{:?}", rt.errors());
    assert!(rt.errors()[0].contains(&format!("seq {}", commands[0].seq)));
    let next = sim.step(&[]);
    rt.think(&next.view, &events, &mut Vec::new());
    assert_eq!(rt.errors().len(), 1);
}

#[test]
fn rejected_edit_errors_share_the_existing_cap_but_all_failures_are_counted() {
    let (mut sim, content, _, _, resource) = scene();
    let mut rt = runtime(&sim, content, PlayerNumbering::Slots);
    for expected in [130, 260] {
        rt.eval::<()>(&format!("for n=1,130 do A2.SetRealTeam({},1) end", resource.0))
            .unwrap();
        let out = sim.step(&rt.take_commands());
        rt.think(&out.view, &out.events, &mut Vec::new());
        assert_eq!(rt.errors().len(), 100);
        assert!(
            rt.unsupported()
                .iter()
                .any(|(key, count)| key == "A2.SetRealTeam: simulation rejected edit" && *count == expected)
        );
    }
}

#[test]
fn team_numbering_and_cached_reads_follow_the_shared_owner_change() {
    for (numbering, to, neutral) in [(PlayerNumbering::Slots, 7, 253), (PlayerNumbering::ArmadaTeams, 8, 0)] {
        let (mut sim, content, craft, _, _) = scene();
        let mut rt = runtime(&sim, content, numbering);
        rt.eval::<()>(&format!("A2.SetRealTeam({}, {})", craft.0, to)).unwrap();
        let commands = rt.take_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].kind, CommandKind::Edit(Edit::Team(craft, PlayerId(7))));
        assert_eq!(
            rt.eval::<u8>(&format!("return A2.GetRealTeam({})", craft.0)).unwrap(),
            if numbering == PlayerNumbering::Slots { 0 } else { 1 }
        );
        let out = sim.step(&commands);
        no_rejection(&out.events);
        assert_eq!(out.view.unit(craft).unwrap().owner, PlayerId(7));
        rt.think(&out.view, &out.events, &mut Vec::new());
        assert_eq!(
            rt.eval::<u8>(&format!("return A2.GetRealTeam({})", craft.0)).unwrap(),
            to
        );
        let denied = sim.step(&[Command {
            player: PlayerId(0),
            seq: 0,
            kind: CommandKind::Order(Order::Move {
                units: vec![craft],
                to: pos(300),
                queue: false,
            }),
        }]);
        assert!(denied.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
        let allowed = sim.step(&[Command {
            player: PlayerId(7),
            seq: 0,
            kind: CommandKind::Order(Order::Move {
                units: vec![craft],
                to: pos(300),
                queue: false,
            }),
        }]);
        no_rejection(&allowed.events);
        rt.eval::<()>(&format!("A2.SetRealTeam({}, {})", craft.0, neutral))
            .unwrap();
        let neutral_command = rt.take_commands();
        assert_eq!(neutral_command[0].kind, CommandKind::Edit(Edit::Team(craft, NEUTRAL)));
        no_rejection(&sim.step(&neutral_command).events);
    }
}

#[test]
fn force_controls_apply_to_existing_and_pending_entities_in_dispatch_order() {
    let (mut sim, content, craft, _, _) = scene();
    let mut rt = runtime(&sim, content, PlayerNumbering::Slots);
    rt.eval::<()>(&format!("A2.SetForceOntoMap({}, false)", craft.0))
        .unwrap();
    let first = rt.take_commands();
    assert_eq!(first.len(), 1);
    assert_eq!(
        first[0].kind,
        CommandKind::Edit(Edit::ForceOntoMap {
            id: craft,
            enabled: false
        })
    );
    let out = sim.step(&first);
    no_rejection(&out.events);
    assert_eq!(sim.force_onto_map(craft), Some(false));
    rt.think(&out.view, &out.events, &mut Vec::new());
    rt.eval::<()>(&format!("A2.SetForceOntoMap({}, true)", craft.0))
        .unwrap();
    let out = sim.step(&rt.take_commands());
    no_rejection(&out.events);
    assert_eq!(sim.force_onto_map(craft), Some(true));
    rt.think(&out.view, &out.events, &mut Vec::new());

    rt.eval::<()>("pending=Entity.add('control_craft',0,Vector(-300,0,0)); pending.forceOntoMap=false; pending:giveOrder(Order.Go,Vector(100,0,0)); pending.forceOntoMap=true").unwrap();
    let spawned = rt.take_commands();
    assert_eq!(spawned.len(), 1);
    assert!(matches!(spawned[0].kind, CommandKind::Edit(Edit::SpawnRequest { .. })));
    let receipt = sim.step(&spawned);
    no_rejection(&receipt.events);
    let id = receipt
        .events
        .iter()
        .find_map(|event| match event {
            SimEvent::ScriptSpawned { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    let mut deferred = Vec::new();
    rt.think(&receipt.view, &receipt.events, &mut deferred);
    assert_eq!(deferred.len(), 3);
    assert_eq!(
        deferred[0].kind,
        CommandKind::Edit(Edit::ForceOntoMap { id, enabled: false })
    );
    assert!(matches!(deferred[1].kind, CommandKind::Order(Order::Move { .. })));
    assert_eq!(
        deferred[2].kind,
        CommandKind::Edit(Edit::ForceOntoMap { id, enabled: true })
    );
    let applied = sim.step(&deferred);
    no_rejection(&applied.events);
    assert_eq!(sim.force_onto_map(id), Some(true));
    assert_eq!(rt.eval::<u32>("return pending.id").unwrap(), id.0);
}

#[test]
fn disabling_force_changes_the_actual_map_boundary_behavior() {
    let (mut unconfined, content, craft, _, _) = scene();
    let mut confined = unconfined.clone();
    let mut rt = runtime(&unconfined, content, PlayerNumbering::Slots);
    rt.eval::<()>(&format!("A2.SetForceOntoMap({}, false)", craft.0))
        .unwrap();
    no_rejection(&unconfined.step(&rt.take_commands()).events);
    assert_eq!(unconfined.force_onto_map(craft), Some(false));
    assert_eq!(confined.force_onto_map(craft), Some(true));
    let move_order = Command {
        player: PlayerId(0),
        seq: 0,
        kind: CommandKind::Order(Order::Move {
            units: vec![craft],
            to: pos(900),
            queue: false,
        }),
    };
    no_rejection(&unconfined.step(std::slice::from_ref(&move_order)).events);
    no_rejection(&confined.step(&[move_order]).events);
    for _ in 0..300 {
        unconfined.step(&[]);
        confined.step(&[]);
    }
    assert!(confined.unit(craft).unwrap().pos.x <= Fixed::from_int(400));
    assert!(unconfined.unit(craft).unwrap().pos.x > Fixed::from_int(400));
}

#[test]
fn core_rejections_report_only_submitted_script_edits_once() {
    let (mut sim, content, craft, disabled, resource) = scene();
    let mut rt = runtime(&sim, content, PlayerNumbering::Slots);
    rt.eval::<()>(&format!(
        "A2.SetForceOntoMap({}, true); A2.SetRealTeam({}, 1)",
        disabled.0, resource.0
    ))
    .unwrap();
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 2);
    assert!(commands.iter().all(|c| c.player == PlayerId::SCRIPT));
    let rejected = sim.step(&commands);
    assert_eq!(
        rejected
            .events
            .iter()
            .filter(|e| matches!(e, SimEvent::Rejected { .. }))
            .count(),
        2
    );
    assert!(rt.errors().is_empty());
    let unrelated = SimEvent::Rejected {
        player: PlayerId(0),
        seq: commands[0].seq,
    };
    let wrong_seq = SimEvent::Rejected {
        player: PlayerId::SCRIPT,
        seq: u32::MAX,
    };
    let mut events = rejected.events.clone();
    events.extend([unrelated, wrong_seq]);
    rt.think(&rejected.view, &events, &mut Vec::new());
    assert_eq!(rt.errors().len(), 2, "{:?}", rt.errors());
    for (label, command) in [("A2.SetForceOntoMap", &commands[0]), ("A2.SetRealTeam", &commands[1])] {
        assert!(
            rt.errors()
                .iter()
                .any(|e| e.contains(label) && e.contains(&format!("seq {}", command.seq))),
            "{:?}",
            rt.errors()
        );
    }
    let count = rt.errors().len();
    rt.think(&rejected.view, &events, &mut Vec::new());
    assert_eq!(rt.errors().len(), count);
    let later = sim.step(&[]);
    rt.think(&later.view, &events, &mut Vec::new());
    assert_eq!(rt.errors().len(), count);
    assert_eq!(sim.unit(craft).unwrap().owner, PlayerId(0));
}
