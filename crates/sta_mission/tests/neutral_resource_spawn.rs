//! Public mission/runtime regressions for neutral resource spawning.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{EntityId, PlayerId, TeamId};
use sta_mission::api::{MissionConfig, PlayerNumbering};
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{CommandKind, Edit};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::world::{EntityKind, NEUTRAL};
use sta_sim::{Sim, SimEvent, WorldSource};

fn setup(content: Arc<ContentDb>) -> GameSetup {
    GameSetup {
        seed: 43,
        map: Arc::new(MapDef::empty()),
        content,
        players: (0..8)
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

fn runtime(
    sim: &Sim,
    content: Arc<ContentDb>,
    numbering: PlayerNumbering,
    script: &str,
) -> MissionRuntime {
    MissionRuntime::from_source_with(
        "missions/neutral-resource.lua",
        script,
        43,
        MissionConfig {
            content: Some(content),
            view: Some((*sim.view()).clone()),
            player_numbering: numbering,
            ..MissionConfig::default()
        },
    )
}

#[test]
fn armada_neutral_team_spawns_classified_moon_with_receipt_and_matching_hint() {
    let content = Arc::new(fake::db());
    let moon = content.unit(content.key("fx_moon").unwrap()).unwrap();
    assert_eq!(
        EntityKind::of(&moon.class, &moon.movement),
        EntityKind::Resource
    );
    assert_eq!(moon.resource, None);
    let setup = setup(content.clone());
    let mut sim = Sim::from_setup(&setup);
    let mut rt = runtime(
        &sim,
        content,
        PlayerNumbering::ArmadaTeams,
        "moon = Entity.add('fx_moon', 0, Vector(500,0,700)); BEFORE = moon.player.number",
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(
        rt.eval::<(bool, bool, u8)>("return moon.id == nil, moon.pending, BEFORE")
            .unwrap(),
        (true, true, 0)
    );
    assert!(
        sim.view().units.is_empty(),
        "load-time request must not insert a world unit"
    );

    let request = rt.take_commands();
    assert_eq!(request.len(), 1);
    let request_id = match &request[0].kind {
        CommandKind::Edit(Edit::SpawnRequest {
            request: token,
            player,
            ..
        }) => {
            assert_eq!(*token, 1);
            assert_eq!(*player, NEUTRAL);
            assert_eq!(request[0].seq, 1);
            *token
        }
        other => panic!("unexpected spawn command: {other:?}"),
    };
    let out = sim.step(&request);
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    let entity = out
        .events
        .iter()
        .find_map(|event| match event {
            SimEvent::ScriptSpawned { request, id } if *request == request_id => Some(*id),
            _ => None,
        })
        .expect("successful neutral resource receipt");
    assert_eq!(out.view.unit(entity).unwrap().owner, NEUTRAL);

    rt.think(&out.view, &out.events, &mut Vec::new());
    assert_eq!(
        rt.eval::<(u32, bool, u8)>("return moon.id, moon.pending, moon.player.number")
            .unwrap(),
        (entity.0, false, 0)
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn slots_keep_craft_owned_and_normalize_resources_to_neutral() {
    let content = Arc::new(fake::db());
    let setup = setup(content.clone());
    let mut sim = Sim::from_setup(&setup);
    let mut rt = runtime(
        &sim,
        content,
        PlayerNumbering::Slots,
        r#"
        craft = Entity.add('fx_cruiser', 0, Vector(100,0,0))
        explicit = Entity.add('fx_moon', 253, Vector(200,0,0))
        owned = Entity.add('fx_moon', 0, Vector(300,0,0))
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(
        rt.eval::<(u8, u8, u8)>(
            "return craft.player.number, explicit.player.number, owned.player.number"
        )
        .unwrap(),
        (0, 253, 253)
    );
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 3);
    let payloads: Vec<_> = commands
        .iter()
        .map(|command| match &command.kind {
            CommandKind::Edit(Edit::SpawnRequest {
                request, player, ..
            }) => {
                assert_eq!(command.seq, *request as u32);
                *player
            }
            ref other => panic!("unexpected command: {other:?}"),
        })
        .collect();
    assert_eq!(payloads, [PlayerId(0), NEUTRAL, PlayerId(0)]);
    assert!(sim.view().units.is_empty());

    let out = sim.step(&commands);
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    let mut by_request = [EntityId::NONE; 3];
    for event in &out.events {
        if let SimEvent::ScriptSpawned { request, id } = event {
            by_request[*request as usize - 1] = *id;
        }
    }
    assert!(by_request.iter().all(|id| *id != EntityId::NONE));
    assert_eq!(out.view.unit(by_request[0]).unwrap().owner, PlayerId(0));
    assert_eq!(out.view.unit(by_request[1]).unwrap().owner, NEUTRAL);
    assert_eq!(out.view.unit(by_request[2]).unwrap().owner, NEUTRAL);
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert_eq!(
        rt.eval::<(u8, u8, u8)>(
            "return craft.player.number, explicit.player.number, owned.player.number"
        )
        .unwrap(),
        (0, 253, 253)
    );
    assert_eq!(
        rt.eval::<(u32, u32, u32)>("return craft.id, explicit.id, owned.id")
            .unwrap(),
        (by_request[0].0, by_request[1].0, by_request[2].0)
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn invalid_neutral_owners_fail_before_request_or_sequence_allocation() {
    let content = Arc::new(fake::db());
    let setup = setup(content.clone());
    let sim = Sim::from_setup(&setup);
    let mut rt = runtime(
        &sim,
        content,
        PlayerNumbering::Slots,
        r#"
        for _, call in ipairs({
            function() Entity.add('fx_cruiser', 253, Vector(0,0,0)) end,
            function() Entity.add('fx_station', 253, Vector(0,0,0)) end,
            function() Entity.add('fx_cruiser', 254, Vector(0,0,0)) end,
            function() Entity.add('fx_cruiser', 255, Vector(0,0,0)) end,
            function() Entity.add('fx_cruiser', 8, Vector(0,0,0)) end
        }) do assert(not pcall(call)) end
        valid = Entity.add('fx_moon', 253, Vector(0,0,0))
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(sim.view().units.is_empty());
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].seq, 1);
    assert!(matches!(
        commands[0].kind,
        CommandKind::Edit(Edit::SpawnRequest {
            request: 1,
            player: NEUTRAL,
            ..
        })
    ));
    assert!(rt.unsupported().is_empty());
}

fn database_with_metadata_resource() -> Arc<ContentDb> {
    let fake = fake::db();
    let mut metadata_resource = fake.unit(fake.key("fx_cruiser").unwrap()).unwrap().clone();
    metadata_resource.odf = "fx_metadata_resource".into();
    metadata_resource.resource = Some(2);
    assert_ne!(
        EntityKind::of(&metadata_resource.class, &metadata_resource.movement),
        EntityKind::Resource,
        "fixture must exercise resource metadata rather than class kind"
    );
    let mut builder = ContentDb::builder()
        .profile(fake.profile())
        .features(fake.features().clone());
    for unit in fake.units() {
        builder = builder.unit(unit.clone());
    }
    for weapon in fake.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    Arc::new(builder.unit(metadata_resource).build().unwrap())
}

#[test]
fn neutral_owner_is_allowed_for_a_craft_definition_with_resource_metadata() {
    let content = database_with_metadata_resource();
    let key = content.key("fx_metadata_resource").unwrap();
    let def = content.unit(key).unwrap();
    assert_ne!(
        EntityKind::of(&def.class, &def.movement),
        EntityKind::Resource
    );
    assert_eq!(def.resource, Some(2));
    let setup = setup(content.clone());
    let mut sim = Sim::from_setup(&setup);
    let mut rt = runtime(
        &sim,
        content,
        PlayerNumbering::Slots,
        "metadata_resource = Entity.add('fx_metadata_resource', 253, Vector(900,0,0))",
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(
        rt.eval::<u8>("return metadata_resource.player.number")
            .unwrap(),
        253
    );
    let request = rt.take_commands();
    assert_eq!(request.len(), 1);
    assert!(matches!(
        request[0].kind,
        CommandKind::Edit(Edit::SpawnRequest {
            request: 1,
            player: NEUTRAL,
            ..
        })
    ));
    let out = sim.step(&request);
    let id = out
        .events
        .iter()
        .find_map(|event| match event {
            SimEvent::ScriptSpawned { id, .. } => Some(*id),
            _ => None,
        })
        .expect("metadata resource receipt");
    assert_eq!(out.view.unit(id).unwrap().owner, NEUTRAL);
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert_eq!(rt.eval::<u32>("return metadata_resource.id").unwrap(), id.0);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty());
}
