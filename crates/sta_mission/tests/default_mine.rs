//! Public mission/runtime regressions for omitted-target Mine orders.

use std::collections::BTreeMap;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MapObject, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_mission::api::MissionConfig;
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{CommandKind, Edit, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

fn point(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::from_int(y),
        z: Fixed::from_int(z),
    }
}

fn setup(objects: &[(&str, u8, Vec3)]) -> (GameSetup, Sim, Arc<ContentDb>) {
    let content = Arc::new(fake::db());
    let mut map = MapDef::empty();
    map.objects = objects
        .iter()
        .enumerate()
        .map(|(index, (odf, owner, pos))| MapObject {
            odf: (*odf).into(),
            owner: Some(*owner),
            pos: *pos,
            heading: (Fixed::ZERO, Fixed::ONE),
            label: Some(format!("obj_{index}")),
        })
        .collect();
    let setup = GameSetup {
        seed: 59,
        map: Arc::new(map),
        content: content.clone(),
        players: (0..2)
            .map(|n| PlayerSetup {
                id: PlayerId(n),
                team: TeamId(n),
                race: "fx".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    };
    let sim = Sim::from_setup(&setup);
    (setup, sim, content)
}

fn labeled(sim: &Sim, label: &str) -> EntityId {
    sim.labels
        .iter()
        .find_map(|(id, value)| (value == label).then_some(id))
        .expect("labeled map unit")
}

fn runtime(sim: &Sim, content: Arc<ContentDb>, script: &str) -> MissionRuntime {
    let labels = sim
        .labels
        .iter()
        .map(|(id, label)| (id, label.clone()))
        .collect::<BTreeMap<_, _>>();
    MissionRuntime::from_source_with(
        "missions/default-mine.lua",
        script,
        59,
        MissionConfig {
            content: Some(content),
            labels,
            view: Some((*sim.view()).clone()),
            ..MissionConfig::default()
        },
    )
}

#[test]
fn omitted_mine_target_uses_nearest_xz_and_breaks_world_ties_by_id() {
    let (_, mut sim, content) = setup(&[
        ("fx_miner", 0, point(0, 0, 0)),
        ("fx_moon", 253, point(100, 5_000, 0)),
        ("fx_moon", 253, point(0, -5_000, 100)),
        ("fx_moon", 253, point(300, 0, 0)),
    ]);
    let actor = labeled(&sim, "obj_0");
    let first_tie = labeled(&sim, "obj_1");
    // Map order assigns IDs; the first two resources tie in XZ but differ in Y.
    let tied_ids = sim
        .view()
        .units
        .iter()
        .filter(|unit| unit.id != actor)
        .map(|unit| unit.id)
        .collect::<Vec<_>>();
    let expected = *tied_ids.iter().min().expect("resource ids");
    assert_eq!(first_tie, expected);

    let mut rt = runtime(
        &sim,
        content,
        "miner = EntityFinder():findOne({ odf='fx_miner' }); pending = Entity.add('fx_moon', 253, Vector(-100,0,0)); miner:giveOrder(Order.Mine)",
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 2);
    assert!(matches!(
        commands[0].kind,
        CommandKind::Edit(Edit::SpawnRequest { .. })
    ));
    assert_eq!(
        commands[1].kind,
        CommandKind::Order(Order::Mine {
            units: vec![actor],
            node: expected,
        })
    );
    assert!(rt.unsupported().is_empty());
    let out = sim.step(&commands);
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn pending_actor_and_nodes_resolve_to_actual_ids_and_keep_fifo_order() {
    let (_, mut sim, content) = setup(&[]);
    let mut rt = runtime(
        &sim,
        content,
        r#"
        miner = Entity.add('fx_miner', 0, Vector(0,0,0))
        first = Entity.add('fx_moon', 253, Vector(100,8000,0))
        second = Entity.add('fx_moon', 253, Vector(0,-8000,100))
        miner:giveOrder(Order.Mine)
        miner:giveOrder(Order.Stop)
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(
        rt.eval::<(bool, bool, bool)>("return miner.pending, first.pending, second.pending")
            .unwrap(),
        (true, true, true)
    );

    let requests = rt.take_commands();
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|command| matches!(command.kind, CommandKind::Edit(Edit::SpawnRequest { .. })))
    );
    let tokens = requests
        .iter()
        .map(|command| match command.kind {
            CommandKind::Edit(Edit::SpawnRequest { request, .. }) => request,
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    assert_eq!(tokens, [1, 2, 3]);

    let out = sim.step(&requests);
    let ids = out
        .events
        .iter()
        .filter_map(|event| match event {
            SimEvent::ScriptSpawned { request, id } => Some((*request, *id)),
            _ => None,
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(ids.len(), 3);
    let miner_id = ids[&tokens[0]];
    let first_id = ids[&tokens[1]];
    let second_id = ids[&tokens[2]];
    assert_ne!(first_id, second_id);

    let mut deferred = Vec::new();
    rt.think(&out.view, &out.events, &mut deferred);
    assert_eq!(deferred.len(), 2);
    assert_eq!(
        deferred[0].kind,
        CommandKind::Order(Order::Mine {
            units: vec![miner_id],
            node: first_id,
        })
    );
    assert_eq!(
        deferred[1].kind,
        CommandKind::Order(Order::Stop {
            units: vec![miner_id],
        })
    );
    assert_eq!(
        rt.eval::<(u32, u32, u32)>("return miner.id, first.id, second.id")
            .unwrap(),
        (miner_id.0, first_id.0, second_id.0)
    );
    let applied = sim.step(&deferred);
    assert!(
        !applied
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn explicit_target_is_preserved_invalid_target_and_missing_resource_do_not_order() {
    let (_, sim, content) = setup(&[
        ("fx_miner", 0, point(0, 0, 0)),
        ("fx_moon", 253, point(100, 0, 0)),
        ("fx_moon", 253, point(500, 0, 0)),
    ]);
    let actor = labeled(&sim, "obj_0");
    let moon = labeled(&sim, "obj_2");
    let mut rt = runtime(
        &sim,
        content.clone(),
        r#"
        miner = EntityFinder():findOne({ odf='fx_miner' })
        moon = EntityFinder():findOne({label='obj_2'})
        miner:giveOrder(Order.Mine, moon)
        miner:giveOrder(Order.Mine, 'not an entity')
        "#,
    );
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].kind,
        CommandKind::Order(Order::Mine {
            units: vec![actor],
            node: moon,
        })
    );
    assert_eq!(rt.unsupported().len(), 1);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());

    let (_, sim_without_resource, content) = setup(&[("fx_miner", 0, point(0, 0, 0))]);
    let mut no_resource = runtime(
        &sim_without_resource,
        content,
        "miner = EntityFinder():findOne({ odf='fx_miner' }); miner:giveOrder(Order.Mine, nil)",
    );
    assert!(no_resource.take_commands().is_empty());
    assert_eq!(no_resource.unsupported().len(), 1);
    assert!(
        no_resource.errors().is_empty(),
        "{:?}",
        no_resource.errors()
    );
}

#[test]
fn failed_selected_pending_node_does_not_retarget_to_world_resource() {
    let (_, mut sim, content) = setup(&[
        ("fx_miner", 0, point(0, 0, 0)),
        ("fx_moon", 253, point(2_000, 0, 0)),
    ]);
    let mut rt = runtime(
        &sim,
        content,
        r#"
        miner = EntityFinder():findOne({ odf='fx_miner' })
        pending = Entity.add('fx_moon', 253, Vector(100,0,0))
        miner:giveOrder(Order.Mine)
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let request = rt.take_commands();
    assert_eq!(request.len(), 1);
    let CommandKind::Edit(Edit::SpawnRequest { .. }) = request[0].kind else {
        panic!("expected pending resource spawn request")
    };
    // The host withholds the submitted batch. On the next tick the selected
    // handle has no receipt; the runtime must not choose the world moon.
    let out = sim.step(&[]);
    let mut after_failure = Vec::new();
    rt.think(&out.view, &out.events, &mut after_failure);
    assert!(after_failure.is_empty());
    assert!(rt.take_commands().is_empty());
    assert!(!rt.unsupported().is_empty());
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}
