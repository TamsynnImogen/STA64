//! Public mission/runtime regressions for static `Entity.remove(handle)`.

use std::sync::Arc;

use sta_content::{MapDef, MapObject};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_mission::api::MissionConfig;
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{CommandKind, Edit};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

fn setup() -> GameSetup {
    let content = Arc::new(sta_content::fake::db());
    let mut map = MapDef::empty();
    map.objects = [Some(0), Some(1)]
        .into_iter()
        .enumerate()
        .map(|(i, owner)| MapObject {
            odf: "fx_cruiser".into(),
            owner,
            pos: Vec3::from_f32([i as f32 * 500.0, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
            label: None,
        })
        .collect();
    GameSetup {
        seed: 37,
        map: Arc::new(map),
        content,
        players: (0..2)
            .map(|n| PlayerSetup {
                id: PlayerId(n),
                team: TeamId(n),
                race: "fx".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    }
}

fn ids(sim: &Sim) -> [EntityId; 2] {
    let key = sim.content().unwrap().key("fx_cruiser").unwrap();
    let mut by_owner = [EntityId::NONE; 2];
    for unit in &sim.view().units {
        if unit.def == key && unit.owner.0 < 2 {
            by_owner[unit.owner.0 as usize] = unit.id;
        }
    }
    by_owner
}

fn runtime(sim: &Sim, setup: &GameSetup, script: &str) -> MissionRuntime {
    MissionRuntime::from_source_with(
        "missions/static-remove.lua",
        script,
        37,
        MissionConfig {
            content: Some(setup.content.clone()),
            view: Some((*sim.view()).clone()),
            ..MissionConfig::default()
        },
    )
}

#[test]
fn static_and_instance_removal_queue_equivalent_kills_and_keep_stale_behavior() {
    let setup = setup();
    let mut sim = Sim::from_setup(&setup);
    let [static_id, method_id] = ids(&sim);
    let mut rt = runtime(
        &sim,
        &setup,
        r#"
        static_target = EntityFinder():findOne({ player=0 })
        method_target = EntityFinder():findOne({ player=1 })
        Entity.remove(static_target)
        method_target:remove()
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 2);
    assert!(commands.iter().all(|command| {
        command.player == PlayerId::SCRIPT
            && matches!(command.kind, CommandKind::Edit(Edit::Kill(_)))
    }));
    assert_eq!(commands[0].kind, CommandKind::Edit(Edit::Kill(static_id)));
    assert_eq!(commands[1].kind, CommandKind::Edit(Edit::Kill(method_id)));
    assert!(sim.view().unit(static_id).is_some());
    assert!(sim.view().unit(method_id).is_some());

    let out = sim.step(&commands);
    for id in [static_id, method_id] {
        assert!(
            out.events
                .iter()
                .any(|event| matches!(event, SimEvent::Destroyed { id: dead, .. } if *dead == id))
        );
        assert!(out.view.unit(id).is_none());
    }
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert_eq!(
        rt.eval::<(bool, bool, bool, bool)>(
            "return static_target.valid, static_target.destroyed, method_target.valid, method_target.destroyed"
        )
        .unwrap(),
        (false, true, false, true)
    );

    rt.eval::<()>("method_target:remove()").unwrap();
    let stale = rt.take_commands();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].kind, CommandKind::Edit(Edit::Kill(method_id)));
    let rejected = sim.step(&stale);
    assert!(
        rejected
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn pending_static_removal_waits_for_receipt_then_kills_the_actual_id_once() {
    let setup = setup();
    let mut sim = Sim::from_setup(&setup);
    let initial_ids = ids(&sim);
    let mut rt = runtime(
        &sim,
        &setup,
        r#"
        pending = Entity.add('fx_cruiser', 0, Vector(1500, 0, 0))
        Entity.remove(pending)
        "#,
    );
    assert_eq!(
        rt.eval::<(bool, bool)>("return pending.pending, pending.id == nil")
            .unwrap(),
        (true, true)
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let first = rt.take_commands();
    assert_eq!(first.len(), 1);
    let request = match &first[0].kind {
        CommandKind::Edit(Edit::SpawnRequest { request, .. }) => *request,
        other => panic!("unexpected command before receipt: {other:?}"),
    };
    assert!(
        !first
            .iter()
            .any(|command| matches!(command.kind, CommandKind::Edit(Edit::Kill(_))))
    );

    let out = sim.step(&first);
    let actual_id = out
        .events
        .iter()
        .find_map(|event| match event {
            SimEvent::ScriptSpawned { request: got, id } if *got == request => Some(*id),
            _ => None,
        })
        .expect("successful spawn receipt");
    assert!(!initial_ids.contains(&actual_id));
    let mut deferred = Vec::new();
    rt.think(&out.view, &out.events, &mut deferred);
    assert_eq!(deferred.len(), 1);
    assert_eq!(deferred[0].player, PlayerId::SCRIPT);
    assert_eq!(deferred[0].kind, CommandKind::Edit(Edit::Kill(actual_id)));
    assert_eq!(rt.eval::<u32>("return pending.id").unwrap(), actual_id.0);

    let removed = sim.step(&deferred);
    assert!(removed.view.unit(actual_id).is_none());
    assert!(
        removed
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Destroyed { id, .. } if *id == actual_id))
    );
    let mut later = Vec::new();
    rt.think(&removed.view, &removed.events, &mut later);
    assert!(later.is_empty(), "pending static remove dispatched twice");
    assert!(rt.eval::<bool>("return pending.destroyed").unwrap());
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn malformed_static_removal_errors_before_queuing_or_mutating() {
    let setup = setup();
    let mut sim = Sim::from_setup(&setup);
    let [actor, _] = ids(&sim);
    let mut rt = runtime(
        &sim,
        &setup,
        r#"
        actor = EntityFinder():findOne({ player=0 })
        local malformed = {
            function() Entity.remove() end,
            function() Entity.remove(actor, actor) end,
            function() Entity.remove(nil) end,
            function() Entity.remove(1) end,
            function() Entity.remove('1') end,
            function() Entity.remove({ actor }) end,
            function() Entity.remove(Player(0)) end
        }
        for _, call in ipairs(malformed) do assert(not pcall(call)) end
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty());
    assert!(rt.take_commands().is_empty());
    assert!(sim.view().unit(actor).is_some());
    assert!(sim.step(&[]).view.unit(actor).is_some());
}
