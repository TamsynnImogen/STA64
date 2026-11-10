//! Deferred camera spots must use actual spawn receipts, never pending hints.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_mission::api::MissionConfig;
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::present::{Arg, Presentation, Spot};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

fn scene() -> (Sim, Arc<ContentDb>) {
    let db = Arc::new(fake::db());
    let game = GameSetup {
        seed: 29,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
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
    (Sim::from_setup(&game), db)
}

fn runtime(sim: &Sim, db: Arc<ContentDb>, source: &str) -> MissionRuntime {
    let cfg = MissionConfig {
        content: Some(db),
        view: Some((*sim.view()).clone()),
        ..MissionConfig::default()
    };
    let rt = MissionRuntime::from_source_with("pending-camera.lua", source, 29, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

fn cameras(commands: &[Command]) -> Vec<Presentation> {
    commands
        .iter()
        .filter_map(|command| match &command.kind {
            CommandKind::Edit(Edit::Present(p @ (Presentation::Shot { .. } | Presentation::Call { .. }))) => {
                Some(p.clone())
            }
            _ => None,
        })
        .collect()
}

fn receipt(events: &[SimEvent]) -> EntityId {
    events
        .iter()
        .find_map(|event| match event {
            SimEvent::ScriptSpawned { id, .. } => Some(*id),
            _ => None,
        })
        .expect("actual spawn receipt")
}

#[test]
fn pending_source_and_target_emit_real_ids_after_receipt_before_callback() {
    let (mut sim, db) = scene();
    let mut rt = runtime(
        &sim,
        db,
        r#"
        h = Entity.add('fx_cruiser', 0, Vector(100,0,0))
        Cineractive.active = true
        Cineractive:lookFrom(h)
        Cineractive:setCameraOffset(Vector(0,10,0))
        Cineractive:lookAt(h)
        MMM.register(function() Seen = h.id end)
    "#,
    );
    let first = rt.take_commands();
    assert_eq!(
        first
            .iter()
            .filter(|c| matches!(c.kind, CommandKind::Edit(Edit::SpawnRequest { .. })))
            .count(),
        1
    );
    assert!(cameras(&first).is_empty(), "pending spot escaped at load");
    assert!(first.iter().any(|c| matches!(
        c.kind,
        CommandKind::Edit(Edit::Present(Presentation::CameraOffset { .. }))
    )));
    let out = sim.step(&first);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    let id = receipt(&out.events);
    let mut second = Vec::new();
    rt.think(&out.view, &out.events, &mut second);
    assert_eq!(rt.eval::<u32>("return Seen").unwrap(), id.0);
    assert_eq!(
        cameras(&second),
        [
            Presentation::Call {
                name: "Cineractive.lookFrom".into(),
                args: vec![Arg::Entity(id)]
            },
            Presentation::Shot {
                from: Spot::Entity(id),
                at: Spot::Entity(id),
                seconds: Fixed::ZERO
            },
        ]
    );
    let applied = sim.step(&second);
    assert!(!applied.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert_eq!(
        applied
            .events
            .iter()
            .filter(|e| matches!(e, SimEvent::Present(Presentation::Shot { .. })))
            .count(),
        1
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn pending_extra_entity_arguments_and_later_look_at_preserve_order() {
    let (mut sim, db) = scene();
    let mut rt = runtime(
        &sim,
        db,
        r#"
        a = Entity.add('fx_cruiser', 0, Vector(100,0,0))
        b = Entity.add('fx_cruiser', 0, Vector(200,0,0))
        Cineractive:lookFrom(a, b)
        Cineractive:lookAt(b, a)
        Cineractive:lookAt(Vector(5,6,7))
    "#,
    );
    let requests = rt.take_commands();
    assert_eq!(requests.len(), 2);
    assert!(cameras(&requests).is_empty());
    let out = sim.step(&requests);
    let ids: Vec<_> = out
        .events
        .iter()
        .filter_map(|event| match event {
            SimEvent::ScriptSpawned { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 2);
    let mut emitted = Vec::new();
    rt.think(&out.view, &out.events, &mut emitted);
    assert_eq!(
        cameras(&emitted),
        [
            Presentation::Call {
                name: "Cineractive.lookFrom".into(),
                args: vec![Arg::Entity(ids[0]), Arg::Entity(ids[1])]
            },
            Presentation::Call {
                name: "Cineractive.lookAt".into(),
                args: vec![Arg::Entity(ids[1]), Arg::Entity(ids[0])]
            },
            Presentation::Call {
                name: "Cineractive.lookAt".into(),
                args: vec![Arg::Position(Vec3::from_f32([5., 6., 7.]))]
            },
        ]
    );
    assert!(
        !sim.step(&emitted)
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
}

#[test]
fn new_source_and_inactive_mode_cancel_older_deferred_scenes() {
    let (mut sim, db) = scene();
    let mut rt = runtime(
        &sim,
        db,
        r#"
        old = Entity.add('fx_cruiser', 0, Vector(100,0,0))
        new = Entity.add('fx_cruiser', 0, Vector(200,0,0))
        Cineractive.active = true
        Cineractive:lookFrom(old); Cineractive:lookAt(Vector(1,0,0))
        Cineractive:lookFrom(new); Cineractive:lookAt(Vector(2,0,0))
    "#,
    );
    let first = rt.take_commands();
    let out = sim.step(&first);
    let ids: Vec<_> = out
        .events
        .iter()
        .filter_map(|e| match e {
            SimEvent::ScriptSpawned { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 2);
    let mut emitted = Vec::new();
    rt.think(&out.view, &out.events, &mut emitted);
    assert_eq!(
        cameras(&emitted),
        [
            Presentation::Call {
                name: "Cineractive.lookFrom".into(),
                args: vec![Arg::Entity(ids[1])]
            },
            Presentation::Shot {
                from: Spot::Entity(ids[1]),
                at: Spot::Position(Vec3::from_f32([2., 0., 0.])),
                seconds: Fixed::ZERO
            },
        ]
    );

    let (mut sim, db) = scene();
    let mut rt = runtime(
        &sim,
        db,
        "h=Entity.add('fx_cruiser',0,Vector(0,0,0)); Cineractive.active=true; Cineractive:lookFrom(h); Cineractive:lookAt(h); Cineractive.active=false",
    );
    let first = rt.take_commands();
    assert!(cameras(&first).is_empty());
    let out = sim.step(&first);
    let mut emitted = Vec::new();
    rt.think(&out.view, &out.events, &mut emitted);
    assert!(cameras(&emitted).is_empty());
}

#[test]
fn canceled_pending_source_is_not_reused_by_a_later_target() {
    let (mut sim, db) = scene();
    let mut rt = runtime(
        &sim,
        db,
        "h=Entity.add('fx_cruiser',0,Vector(0,0,0)); Cineractive:lookFrom(h); Cineractive.active=false; Cineractive:lookAt(Vector(5,0,0))",
    );
    let first = rt.take_commands();
    assert_eq!(
        cameras(&first),
        [Presentation::Call {
            name: "Cineractive.lookAt".into(),
            args: vec![Arg::Position(Vec3::from_f32([5., 0., 0.]))]
        }]
    );
    let out = sim.step(&first);
    let mut emitted = Vec::new();
    rt.think(&out.view, &out.events, &mut emitted);
    assert!(
        cameras(&emitted).is_empty(),
        "a canceled source must not reappear after its receipt"
    );
    assert!(rt.errors().is_empty());
}

#[test]
fn missing_receipt_rejected_spawn_and_same_tick_death_never_emit_fake_spots() {
    let (mut sim, db) = scene();
    let mut rt = runtime(
        &sim,
        db,
        "h=Entity.add('fx_cruiser',0,Vector(0,0,0)); Cineractive:lookFrom(h); Cineractive:lookAt(h)",
    );
    let request = rt.take_commands();
    let out = sim.step(&request);
    let mut missing = Vec::new();
    rt.think(&out.view, &[], &mut missing);
    assert!(cameras(&missing).is_empty(), "no receipt must not guess an entity ID");
    let mut after_receipt = Vec::new();
    rt.think(&out.view, &out.events, &mut after_receipt);
    assert!(cameras(&after_receipt).iter().all(|p| match p {
        Presentation::Call { args, .. } => args.iter().all(|a| !matches!(a, Arg::Entity(EntityId::NONE))),
        Presentation::Shot { from, at, .. } =>
            !matches!(from, Spot::Entity(EntityId::NONE)) && !matches!(at, Spot::Entity(EntityId::NONE)),
        _ => true,
    }));

    let mut no_content = Sim::from_setup(&GameSetup {
        seed: 29,
        map: Arc::new(MapDef::empty()),
        content: Arc::new(ContentDb::builder().build().unwrap()),
        players: (0..2)
            .map(|n| PlayerSetup {
                id: PlayerId(n),
                team: TeamId(n),
                race: "fx".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    });
    let (sim, db) = scene();
    let mut rt = runtime(
        &sim,
        db,
        "h=Entity.add('fx_cruiser',0,Vector(0,0,0)); Cineractive:lookFrom(h); Cineractive:lookAt(h)",
    );
    let request = rt.take_commands();
    let rejected = no_content.step(&request);
    assert!(rejected.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    let mut commands = Vec::new();
    rt.think(&rejected.view, &rejected.events, &mut commands);
    assert!(cameras(&commands).is_empty());
    assert!(!rt.unsupported().is_empty(), "failed scene should be diagnosed");

    let (mut sim, db) = scene();
    let mut rt = runtime(
        &sim,
        db,
        "h=Entity.add('fx_cruiser',0,Vector(0,0,0)); Cineractive:lookFrom(h); Cineractive:lookAt(h)",
    );
    let mut request = rt.take_commands();
    // This empty-map fixture allocates ID 1; only this test predicts it.
    request.push(Command {
        player: PlayerId::SCRIPT,
        seq: request[0].seq + 1,
        kind: CommandKind::Edit(Edit::Kill(EntityId(1))),
    });
    let dead = sim.step(&request);
    assert!(
        dead.events
            .iter()
            .any(|e| matches!(e, SimEvent::ScriptSpawned { id: EntityId(1), .. }))
    );
    assert!(dead.view.unit(EntityId(1)).is_none());
    let mut commands = Vec::new();
    rt.think(&dead.view, &dead.events, &mut commands);
    assert!(cameras(&commands).is_empty());
    assert!(!rt.unsupported().is_empty(), "dead scene should be diagnosed");
}
