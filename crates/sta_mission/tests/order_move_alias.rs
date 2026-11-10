//! Public mission/runtime regressions for the `Order.Move` alias.

use std::sync::Arc;

use sta_content::{MapDef, MapObject};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_mission::api::MissionConfig;
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

fn point(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

fn setup() -> GameSetup {
    let content = Arc::new(sta_content::fake::db());
    let mut map = MapDef::empty();
    map.objects = [(Some(0), point(-5000, 0)), (Some(1), point(5000, 0))]
        .into_iter()
        .map(|(owner, pos)| MapObject {
            odf: "fx_cruiser".into(),
            owner,
            pos,
            heading: (Fixed::ZERO, Fixed::ONE),
            label: None,
        })
        .collect();
    GameSetup {
        seed: 29,
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

fn units(sim: &Sim) -> (EntityId, EntityId) {
    let key = sim.content().unwrap().key("fx_cruiser").unwrap();
    let mut by_owner = [EntityId::NONE; 2];
    for unit in &sim.view().units {
        if unit.def == key && unit.owner.0 < 2 {
            by_owner[unit.owner.0 as usize] = unit.id;
        }
    }
    (by_owner[0], by_owner[1])
}

fn runtime(sim: &Sim, content: Arc<sta_content::ContentDb>, script: &str) -> MissionRuntime {
    MissionRuntime::from_source_with(
        "missions/MoveAlias.lua",
        script,
        29,
        MissionConfig {
            content: Some(content),
            view: Some((*sim.view()).clone()),
            ..MissionConfig::default()
        },
    )
}

#[test]
fn move_alias_matches_go_for_vectors_and_latest_entity_target_position() {
    let setup = setup();
    let mut sim = Sim::from_setup(&setup);
    let (actor, target) = units(&sim);
    let initial_target_pos = sim.view().unit(target).unwrap().pos;
    let mut rt = runtime(
        &sim,
        setup.content.clone(),
        r#"
        assert(rawequal(Order, GameObject.Order))
        assert(Order.Move == Order.Go)
        assert(GameObject.Order.Move == Order.Go)
        actor = EntityFinder():findOne({ player=0 })
        target = EntityFinder():findOne({ player=1 })
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());

    let travel = Command {
        player: PlayerId(1),
        seq: 1,
        kind: CommandKind::Order(Order::Move {
            units: vec![target],
            to: point(3000, 0),
            queue: false,
        }),
    };
    let mut out = sim.step(&[travel]);
    for _ in 0..120 {
        out = sim.step(&[]);
    }
    let latest_target_pos = out.view.unit(target).unwrap().pos;
    let expected_target_pos = Vec3::from_f32([
        latest_target_pos.x.to_f32(),
        latest_target_pos.y.to_f32(),
        latest_target_pos.z.to_f32(),
    ]);
    assert_ne!(
        latest_target_pos, initial_target_pos,
        "target should have moved in the sim"
    );
    rt.think(&out.view, &out.events, &mut Vec::new());

    rt.eval::<()>(
        r#"
        actor:giveOrder(Order.Move, Vector(100, 0, 200))
        actor:giveOrder(Order.Go, Vector(300, 0, 400))
        actor:giveOrder(GameObject.Order.Move, target)
        "#,
    )
    .unwrap();
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 3);
    assert!(commands.iter().all(|command| command.player == PlayerId(0)));
    assert_eq!(
        commands[0].kind,
        CommandKind::Order(Order::Move {
            units: vec![actor],
            to: point(100, 200),
            queue: false,
        })
    );
    assert_eq!(
        commands[1].kind,
        CommandKind::Order(Order::Move {
            units: vec![actor],
            to: point(300, 400),
            queue: false,
        })
    );
    assert_eq!(
        commands[2].kind,
        CommandKind::Order(Order::Move {
            units: vec![actor],
            to: expected_target_pos,
            queue: false,
        })
    );
    let result = sim.step(&commands);
    assert!(
        !result
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty());

    rt.eval::<()>("actor:giveOrder(Order.Move, 'not a point')")
        .unwrap();
    assert!(rt.take_commands().is_empty());
    assert_eq!(rt.unsupported(), [("giveOrder Go: no point".to_owned(), 1)]);
    rt.eval::<()>("actor:giveOrder('Move', Vector(100, 0, 200))")
        .unwrap();
    assert!(rt.take_commands().is_empty());
    assert_eq!(
        rt.unsupported(),
        [
            ("giveOrder Go: no point".to_owned(), 1),
            ("giveOrder Move".to_owned(), 1),
        ]
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn pending_move_alias_waits_for_the_spawn_receipt_and_uses_its_actual_id() {
    let setup = setup();
    let mut sim = Sim::from_setup(&setup);
    let mut rt = runtime(
        &sim,
        setup.content.clone(),
        r#"
        assert(Order.Move == Order.Go and GameObject.Order.Move == Order.Go)
        pending = Entity.add('fx_cruiser', 0, Vector(1000, 0, 0))
        pending:giveOrder(Order.Move, Vector(1300, 0, 100))
        "#,
    );
    let pending_state: (bool, bool) = rt
        .eval("return pending.pending, pending.id == nil")
        .unwrap();
    assert_eq!(pending_state, (true, true));
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());

    let first = rt.take_commands();
    assert_eq!(first.len(), 1);
    let request = match &first[0].kind {
        CommandKind::Edit(Edit::SpawnRequest { request, .. }) => *request,
        other => panic!("unexpected command before spawn receipt: {other:?}"),
    };
    let out = sim.step(&first);
    let actual_id = out
        .events
        .iter()
        .find_map(|event| match event {
            SimEvent::ScriptSpawned { request: got, id } if *got == request => Some(*id),
            _ => None,
        })
        .expect("successful spawn receipt");
    let mut deferred = Vec::new();
    rt.think(&out.view, &out.events, &mut deferred);
    assert_eq!(deferred.len(), 1);
    assert_eq!(
        deferred[0].kind,
        CommandKind::Order(Order::Move {
            units: vec![actual_id],
            to: point(1300, 100),
            queue: false,
        })
    );
    assert_eq!(deferred[0].player, PlayerId(0));
    assert_eq!(rt.eval::<u32>("return pending.id").unwrap(), actual_id.0);
    assert!(!rt.eval::<bool>("return pending.pending").unwrap());

    let applied = sim.step(&deferred);
    assert!(
        !applied
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    let mut later = Vec::new();
    rt.think(&applied.view, &applied.events, &mut later);
    assert!(
        later.is_empty(),
        "deferred move was dispatched more than once"
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty());
}
