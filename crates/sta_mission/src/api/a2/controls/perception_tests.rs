//! Public mission behavior for the shared perceived-team adapter.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{CommandKind, Edit, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::UnitFlags;
use sta_sim::world::NEUTRAL;
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::api::{MissionConfig, PlayerNumbering};
use crate::runtime::MissionRuntime;

fn scene() -> (Sim, Arc<ContentDb>, EntityId, EntityId) {
    let content = Arc::new(sta_content::fake::db());
    let setup = GameSetup {
        seed: 43,
        map: Arc::new(MapDef::empty()),
        content: content.clone(),
        players: [0, 1]
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "fx".into(),
                control: Control::Idle,
            })
            .to_vec(),
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let heading = (Fixed::ZERO, Fixed::ONE);
    let own = sim
        .spawn_def(content.key("fx_cruiser").unwrap(), PlayerId(0), Vec3::ZERO, heading)
        .unwrap();
    let other = sim
        .spawn_def(
            content.key("fx_frigate").unwrap(),
            PlayerId(1),
            Vec3::from_f32([3000.0, 0.0, 0.0]),
            heading,
        )
        .unwrap();
    (sim, content, own, other)
}

fn runtime(sim: &Sim, content: Arc<ContentDb>, numbering: PlayerNumbering) -> MissionRuntime {
    let rt = MissionRuntime::from_source_with(
        "perception.lua",
        "",
        43,
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
        !events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{events:?}"
    );
}

#[test]
fn strict_a2_and_entity_validation_queues_nothing_for_bad_values() {
    for numbering in [PlayerNumbering::Slots, PlayerNumbering::ArmadaTeams] {
        let (sim, content, own, _) = scene();
        let mut rt = runtime(&sim, content, numbering);
        for script in [
            "A2.SetPerceivedTeam()".to_owned(),
            format!("A2.SetPerceivedTeam({},1,2)", own.0),
            format!("A2.SetPerceivedTeam('{}',1)", own.0),
            format!("A2.SetPerceivedTeam({}.5,1)", own.0),
            format!("A2.SetPerceivedTeam({},'1')", own.0),
            format!("A2.SetPerceivedTeam({},1.5)", own.0),
            format!("A2.SetPerceivedTeam({},-1)", own.0),
            format!("A2.SetPerceivedTeam({},254)", own.0),
            format!("A2.SetPerceivedTeam({},true)", own.0),
            "A2.GetPerceivedTeam()".to_owned(),
            format!("A2.GetPerceivedTeam({},1)", own.0),
            "A2.GetPerceivedTeam('1')".to_owned(),
            "A2.GetPerceivedTeam(4294967296)".to_owned(),
        ] {
            assert!(rt.eval::<()>(&script).is_err(), "{script}");
            assert!(rt.take_commands().is_empty(), "{script} queued a command");
        }
        rt.eval::<()>(&format!(
            "actor=EntityFinder():findOne({{player={}}})",
            if numbering == PlayerNumbering::Slots { 0 } else { 1 }
        ))
        .unwrap();
        for value in ["nil", "true", "'1'", "1.5", "-1", "254"] {
            let script = format!("actor.perceivedTeam={value}");
            assert!(rt.eval::<()>(&script).is_err(), "{script}");
            assert!(rt.take_commands().is_empty());
        }
    }
}

#[test]
fn both_numbering_modes_read_stale_then_refreshed_shared_perception() {
    for (numbering, original, to, neutral) in [
        (PlayerNumbering::Slots, 0, 1, 253),
        (PlayerNumbering::ArmadaTeams, 1, 2, 0),
    ] {
        let (mut sim, content, own, _) = scene();
        let mut rt = runtime(&sim, content, numbering);
        rt.eval::<()>(&format!("actor=EntityFinder():findOne({{player={original}}})"))
            .unwrap();
        let initial: (u8, u8, u8) = rt
            .eval(&format!(
                "return A2.GetPerceivedTeam({}), actor.perceivedTeam.number, A2.GetRealTeam({})",
                own.0, own.0
            ))
            .unwrap();
        assert_eq!(initial, (original, original, original));
        rt.eval::<()>(&format!("A2.SetPerceivedTeam({}, {to})", own.0)).unwrap();
        let command = rt.take_commands();
        assert_eq!(command.len(), 1);
        assert_eq!(command[0].player, PlayerId::SCRIPT);
        assert_eq!(
            command[0].kind,
            CommandKind::Edit(Edit::PerceivedTeam(own, PlayerId(1)))
        );
        assert_eq!(
            rt.eval::<u8>(&format!("return A2.GetPerceivedTeam({})", own.0))
                .unwrap(),
            original
        );
        let out = sim.step(&command);
        no_rejection(&out.events);
        assert_eq!(sim.unit(own).unwrap().owner, PlayerId(0));
        rt.think(&out.view, &out.events, &mut Vec::new());
        let refreshed: (u8, u8, u8) = rt
            .eval(&format!(
                "return A2.GetPerceivedTeam({}), actor.perceivedTeam.number, A2.GetRealTeam({})",
                own.0, own.0
            ))
            .unwrap();
        assert_eq!(refreshed, (to, to, original));

        rt.eval::<()>(&format!("actor.perceivedTeam={neutral}")).unwrap();
        let neutral_edit = rt.take_commands();
        assert_eq!(
            neutral_edit[0].kind,
            CommandKind::Edit(Edit::PerceivedTeam(own, NEUTRAL))
        );
        let out = sim.step(&neutral_edit);
        no_rejection(&out.events);
        rt.think(&out.view, &out.events, &mut Vec::new());
        assert_eq!(
            rt.eval::<u8>(&format!("return A2.GetPerceivedTeam({})", own.0))
                .unwrap(),
            neutral
        );
        assert_eq!(sim.unit(own).unwrap().owner, PlayerId(0));
    }
}

#[test]
fn entity_player_value_and_live_construction_proxy_use_shared_edit() {
    let (mut sim, content, own, _) = scene();
    sim.units.get_mut(own).unwrap().progress = Some(Fixed::ONE / Fixed::from_int(2));
    let mut rt = runtime(&sim, content, PlayerNumbering::Slots);
    rt.eval::<()>(&format!(
        "actor=EntityFinder():findOne({{player=0}}); actor.perceivedTeam=Player(1); A2.SetPerceivedTeam({},0)",
        own.0
    ))
    .unwrap();
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 2);
    assert_eq!(
        commands[0].kind,
        CommandKind::Edit(Edit::PerceivedTeam(own, PlayerId(1)))
    );
    assert_eq!(
        commands[1].kind,
        CommandKind::Edit(Edit::PerceivedTeam(own, PlayerId(0)))
    );
    let out = sim.step(&commands);
    no_rejection(&out.events);
    assert_eq!(sim.unit(own).unwrap().progress, Some(Fixed::ONE / Fixed::from_int(2)));
    assert_eq!(sim.unit(own).unwrap().owner, PlayerId(0));
    assert_eq!(sim.perceived_owner(own), Some(PlayerId(0)));
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn pending_entity_actions_wait_for_receipt_and_dispatch_fifo() {
    let (mut sim, content, _, _) = scene();
    let mut rt = runtime(&sim, content, PlayerNumbering::Slots);
    rt.eval::<()>("pending=Entity.add('fx_frigate',0,Vector(5000,0,0)); pending.perceivedTeam=1; pending:giveOrder(Order.Go,Vector(5200,0,0)); pending.perceivedTeam=0").unwrap();
    assert!(rt
        .eval::<bool>("return pending.id == nil and pending.perceivedTeam == nil")
        .unwrap());
    let spawn = rt.take_commands();
    assert_eq!(spawn.len(), 1);
    assert!(matches!(spawn[0].kind, CommandKind::Edit(Edit::SpawnRequest { .. })));
    let out = sim.step(&spawn);
    no_rejection(&out.events);
    let id = out
        .events
        .iter()
        .find_map(|e| match e {
            SimEvent::ScriptSpawned { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    let mut deferred = Vec::new();
    rt.think(&out.view, &out.events, &mut deferred);
    assert_eq!(deferred.len(), 3);
    assert_eq!(
        deferred[0].kind,
        CommandKind::Edit(Edit::PerceivedTeam(id, PlayerId(1)))
    );
    assert!(matches!(deferred[1].kind, CommandKind::Order(Order::Move { .. })));
    assert_eq!(
        deferred[2].kind,
        CommandKind::Edit(Edit::PerceivedTeam(id, PlayerId(0)))
    );
    assert!(deferred
        .iter()
        .all(|c| !matches!(c.kind, CommandKind::Edit(Edit::PerceivedTeam(EntityId::NONE, _)))));
    let applied = sim.step(&deferred);
    no_rejection(&applied.events);
    assert_eq!(sim.perceived_owner(id), Some(PlayerId(0)));
    rt.think(&applied.view, &applied.events, &mut Vec::new());
    assert_eq!(rt.eval::<u32>("return pending.id").unwrap(), id.0);
    assert_eq!(rt.eval::<u8>("return pending.perceivedTeam.number").unwrap(), 0);
}

#[test]
fn failed_pending_spawn_discards_actions_without_none_id_edits() {
    let (mut sim, content, _, _) = scene();
    let mut rt = runtime(&sim, content, PlayerNumbering::Slots);
    rt.eval::<()>("pending=Entity.add('fx_frigate',0,Vector(5000,0,0)); pending.perceivedTeam=1")
        .unwrap();
    let mut spawn = rt.take_commands();
    assert_eq!(spawn.len(), 1);
    if let CommandKind::Edit(Edit::SpawnRequest { odf, .. }) = &mut spawn[0].kind {
        *odf = ContentKey::NONE;
    } else {
        panic!("expected spawn request");
    }
    let out = sim.step(&spawn);
    assert!(out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    let mut deferred = Vec::new();
    rt.think(&out.view, &out.events, &mut deferred);
    assert!(deferred.is_empty());
    assert!(rt
        .eval::<bool>("return pending.id == nil and pending.perceivedTeam == nil")
        .unwrap());
    assert!(
        rt.unsupported().iter().any(|(name, _)| name.contains("spawn")),
        "{:?}",
        rt.unsupported()
    );
}

#[test]
fn stale_missing_and_dying_handles_do_not_mutate_or_expose_perception() {
    let (mut sim, content, own, _) = scene();
    let mut rt = runtime(&sim, content, PlayerNumbering::Slots);
    rt.eval::<()>(&format!(
        "actor=EntityFinder():findOne({{player=0}}); assert(actor.id=={})",
        own.0
    ))
    .unwrap();
    assert!(rt
        .eval::<()>(&format!("A2.GetPerceivedTeam({})", EntityId(u32::MAX).0))
        .is_err());
    assert!(rt
        .eval::<()>(&format!("A2.SetPerceivedTeam({},1)", EntityId(u32::MAX).0))
        .is_err());
    assert!(rt.take_commands().is_empty());
    sim.units.get_mut(own).unwrap().flags = UnitFlags::DYING;
    let out = sim.step(&[]);
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert!(rt.eval::<bool>("return actor.perceivedTeam == nil").unwrap());
    assert!(rt.eval::<()>(&format!("A2.GetPerceivedTeam({})", own.0)).is_err());
    assert!(rt.eval::<()>(&format!("A2.SetPerceivedTeam({},1)", own.0)).is_err());
    assert!(rt.eval::<()>("actor.perceivedTeam=1").is_err());
    assert!(rt.take_commands().is_empty());
}

#[test]
fn configured_seat_rejection_reports_the_matching_submitted_edit() {
    let (mut sim, content, own, _) = scene();
    let mut rt = runtime(&sim, content, PlayerNumbering::Slots);
    rt.eval::<()>(&format!("A2.SetPerceivedTeam({},7)", own.0)).unwrap();
    let queued = rt.take_commands();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].kind, CommandKind::Edit(Edit::PerceivedTeam(own, PlayerId(7))));
    let out = sim.step(&queued);
    assert!(out
        .events
        .iter()
        .any(|e| matches!(e, SimEvent::Rejected { player: PlayerId::SCRIPT, seq } if *seq == queued[0].seq)));
    assert!(rt.errors().is_empty());
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert_eq!(rt.errors().len(), 1, "{:?}", rt.errors());
    assert!(
        rt.errors()[0].contains("A2.SetPerceivedTeam") && rt.errors()[0].contains(&format!("seq {}", queued[0].seq))
    );
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert_eq!(rt.errors().len(), 1);
    assert_eq!(sim.perceived_owner(own), Some(PlayerId(0)));
}
