use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{EntityId, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::replay::{Recorder, Replay};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

use super::*;
use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

fn setup() -> (Sim, Arc<ContentDb>) {
    let db = Arc::new(fake::db());
    let seat = |n| PlayerSetup {
        id: PlayerId(n),
        team: TeamId(n),
        race: "fx".into(),
        control: Control::Idle,
    };
    let game = GameSetup {
        seed: 17,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    };
    (Sim::from_setup(&game), db)
}

fn runtime(sim: &Sim, db: Arc<ContentDb>, script: &str) -> MissionRuntime {
    let cfg = MissionConfig {
        content: Some(db),
        view: Some((*sim.view()).clone()),
        ..MissionConfig::default()
    };
    let rt = MissionRuntime::from_source_with("spawn.lua", script, 17, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

#[test]
fn two_pending_handles_resolve_before_callback_and_deferred_orders_replay() {
    let (mut sim, db) = setup();
    let record_setup = GameSetup {
        seed: 17,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: (0..=1)
            .map(|n| PlayerSetup {
                id: PlayerId(n),
                team: TeamId(n),
                race: "fx".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    };
    let mut recorder = Recorder::with_interval(&record_setup, 1);
    let script = r#"
        a = Entity.add('fx_cruiser', 0, Vector(1000, 0, 0))
        b = Entity.add('fx_cruiser', 1, Vector(2000, 0, 0))
        before = { a.valid, a.pending, a.id, a.position.x, b.valid, b.pending, b.id }
        a:giveOrder(Order.Go, Vector(1100, 0, 0))
        b:giveOrder(Order.Stop)
        MMM.register(function()
            seen = { a.id, b.id, a.pending, b.pending,
                a == EntityFinder():findOne({ odf='fx_cruiser', center=Vector(1000,0,0), radius=20 }) }
        end)
    "#;
    let mut rt = runtime(&sim, db.clone(), script);
    let initial: (bool, bool, bool, f64, bool, bool, bool) = rt
        .eval("return before[1], before[2], before[3] == nil, before[4], before[5], before[6], before[7] == nil")
        .unwrap();
    assert_eq!(initial, (true, true, true, 1000.0, true, true, true));
    let first = rt.take_commands();
    assert_eq!(first.len(), 2);
    let requests: Vec<_> = first
        .iter()
        .map(|c| match &c.kind {
            CommandKind::Edit(Edit::SpawnRequest { request, .. }) if c.player == PlayerId::SCRIPT => *request,
            other => panic!("pending action escaped before receipt: {other:?}"),
        })
        .collect();
    assert_eq!(requests, [1, 2]);
    assert!(sim.view().units.is_empty());

    let out = sim.step(&first);
    let replay_out = recorder.step(&first);
    assert_eq!(out.events, replay_out.events);
    let ids: Vec<_> = requests
        .iter()
        .map(|&request| {
            out.events
                .iter()
                .find_map(|e| match e {
                    SimEvent::ScriptSpawned { request: r, id } if *r == request => Some(*id),
                    _ => None,
                })
                .expect("real spawn receipt")
        })
        .collect();
    assert!(ids.iter().all(|id| *id != EntityId::NONE));
    let mut deferred = Vec::new();
    rt.think(&out.view, &out.events, &mut deferred);
    let seen: (u32, u32, bool, bool, bool) = rt.eval("return seen[1], seen[2], seen[3], seen[4], seen[5]").unwrap();
    assert_eq!(seen, (ids[0].0, ids[1].0, false, false, true));
    assert_eq!(deferred.len(), 2);
    assert_eq!(
        deferred[0].kind,
        CommandKind::Order(Order::Move {
            units: vec![ids[0]],
            to: Vec3::from_f32([1100.0, 0.0, 0.0]),
            queue: false
        })
    );
    assert_eq!(
        deferred[1].kind,
        CommandKind::Order(Order::Stop { units: vec![ids[1]] })
    );
    assert_eq!((deferred[0].player, deferred[1].player), (PlayerId(0), PlayerId(1)));
    let out = sim.step(&deferred);
    assert_eq!(out.events, recorder.step(&deferred).events);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    let bytes = recorder.finish().encode();
    let playback = Replay::decode(&bytes).unwrap().play(db).unwrap();
    assert_eq!(sim.digest(), playback.digest());
}

#[test]
fn rejected_spawn_and_same_tick_death_drop_buffered_actions() {
    let (_, db) = setup();
    let mut no_content = Sim::new(17);
    let mut rt = runtime(
        &no_content,
        db.clone(),
        "h=Entity.add('fx_cruiser',0,Vector(0,0,0)); h:giveOrder(Order.Stop)",
    );
    let first = rt.take_commands();
    assert_eq!(first.len(), 1);
    let out = no_content.step(&first);
    assert!(out.events.iter().any(|e| matches!(
        e,
        SimEvent::Rejected {
            player: PlayerId::SCRIPT,
            ..
        }
    )));
    let mut deferred = Vec::new();
    rt.think(&out.view, &out.events, &mut deferred);
    assert!(deferred.is_empty());
    let state: (bool, bool, bool) = rt.eval("return h.valid, h.pending, h.id == nil").unwrap();
    assert_eq!(state, (false, false, true));

    let (mut sim, db) = setup();
    let mut rt = runtime(
        &sim,
        db,
        "h=Entity.add('fx_cruiser',0,Vector(0,0,0)); h.invincible=true; h:giveOrder(Order.Stop)",
    );
    let mut commands = rt.take_commands();
    // Empty-map fixture allocates ID 1 for this first spawn; only the test
    // predicts it so the same batch can destroy the new unit after receipt.
    commands.push(Command {
        player: PlayerId::SCRIPT,
        seq: commands[0].seq + 1,
        kind: CommandKind::Edit(Edit::Kill(EntityId(1))),
    });
    let out = sim.step(&commands);
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, SimEvent::ScriptSpawned { id: EntityId(1), .. }))
    );
    assert!(out.view.unit(EntityId(1)).is_none());
    let mut deferred = Vec::new();
    rt.think(&out.view, &out.events, &mut deferred);
    assert!(deferred.is_empty());
    let state: (bool, bool, u32) = rt.eval("return h.valid, h.pending, h.id").unwrap();
    assert_eq!(state, (false, false, 1));
}

#[test]
fn unsolicited_and_conflicting_receipts_fail_closed() {
    let (sim, db) = setup();
    let mut c = Ctx::new(
        MissionConfig {
            content: Some(db.clone()),
            view: Some((*sim.view()).clone()),
            ..MissionConfig::default()
        },
        17,
    );
    let a = queue_spawn(&mut c, db.key("fx_cruiser").unwrap(), PlayerId(0), Vec3::ZERO).unwrap();
    c.view.tick = 1;
    c.events = vec![SimEvent::ScriptSpawned {
        request: 1,
        id: EntityId(9),
    }];
    process(&mut c);
    assert!(a.pending(), "queued request must not consume a receipt before drain");
    assert_eq!(c.spawns.outstanding.len(), 1);
    assert_eq!(c.unsupported.get("Entity.add: unexpected spawn receipt"), Some(&1));

    let submitted = drain(&mut c);
    assert_eq!(submitted.len(), 1);
    c.view.tick = 2;
    c.events = vec![
        SimEvent::ScriptSpawned {
            request: 1,
            id: EntityId(9),
        },
        SimEvent::ScriptSpawned {
            request: 1,
            id: EntityId(10),
        },
    ];
    process(&mut c);
    assert!(!a.pending() && a.id().is_none());
    assert!(c.spawns.outstanding.is_empty());
    assert_eq!(
        c.unsupported.get("Entity.add: missing or conflicting spawn receipt"),
        Some(&1)
    );

    let b = queue_spawn(&mut c, db.key("fx_cruiser").unwrap(), PlayerId(0), Vec3::ZERO).unwrap();
    let d = queue_spawn(&mut c, db.key("fx_cruiser").unwrap(), PlayerId(0), Vec3::ZERO).unwrap();
    drain(&mut c);
    c.view.tick = 3;
    c.events = vec![
        SimEvent::ScriptSpawned {
            request: 2,
            id: EntityId(11),
        },
        SimEvent::ScriptSpawned {
            request: 3,
            id: EntityId(11),
        },
    ];
    process(&mut c);
    assert!(b.id().is_none() && d.id().is_none());
    assert!(!b.pending() && !d.pending());
}

#[test]
fn token_and_sequence_exhaustion_leave_no_partial_spawn() {
    let (sim, db) = setup();
    let key = db.key("fx_cruiser").unwrap();
    let make = || {
        Ctx::new(
            MissionConfig {
                content: Some(db.clone()),
                view: Some((*sim.view()).clone()),
                ..MissionConfig::default()
            },
            17,
        )
    };
    let mut c = make();
    c.spawns.next = u64::MAX;
    assert!(matches!(
        queue_spawn(&mut c, key, PlayerId(0), Vec3::ZERO),
        Err("spawn request ids exhausted")
    ));
    assert!(c.commands.is_empty() && c.spawns.outstanding.is_empty());
    assert_eq!(c.spawns.next, u64::MAX);

    let mut c = make();
    c.seq.insert(PlayerId::SCRIPT, u32::MAX);
    assert!(matches!(
        queue_spawn(&mut c, key, PlayerId(0), Vec3::ZERO),
        Err("command sequence exhausted")
    ));
    assert!(c.commands.is_empty() && c.spawns.outstanding.is_empty());
    assert_eq!(c.spawns.next, 0);
    assert_eq!(c.seq.get(&PlayerId::SCRIPT), Some(&u32::MAX));
}

#[test]
fn malformed_add_calls_do_not_queue_and_position_lookup_can_reenter_finder() {
    let (sim, db) = setup();
    let mut rt = runtime(&sim, db, "");
    for script in [
        "Entity.add('fx_cruiser', 0)",
        "Entity.add('missing_odf', 0, Vector(0,0,0))",
        "Entity.add('fx_cruiser', 8, Vector(0,0,0))",
        "Entity.add('fx_cruiser', 0.5, Vector(0,0,0))",
        "Entity.add('fx_cruiser', 0, Vector(0/0,0,0))",
        "Entity.add('fx_cruiser', 0, Vector(2147483648,0,0))",
        "Entity.add('fx_cruiser', 0, {x=0,y='invalid',z=0})",
    ] {
        assert!(rt.eval::<()>(script).is_err(), "{script}");
        assert!(rt.take_commands().is_empty(), "{script} queued a command");
    }
    rt.eval::<()>("local p=setmetatable({x=50,z=0},{__index=function(_,k) EntityFinder():findOne('fx_cruiser'); return 0 end}); h=Entity.add('fx_cruiser',0,p)").unwrap();
    assert_eq!(rt.take_commands().len(), 1);
}

#[test]
fn existing_actor_keeps_attack_then_stop_order_while_waiting_for_a_pending_target() {
    let (mut sim, db) = setup();
    let hero = sim
        .spawn_def(
            db.key("fx_cruiser").unwrap(),
            PlayerId(0),
            Vec3::ZERO,
            (sta_core::Fixed::ZERO, sta_core::Fixed::ONE),
        )
        .unwrap();
    let mut rt = runtime(
        &sim,
        db,
        r#"
        target = Entity.add('fx_cruiser.odf', Player(1), Vector(3000, 0, 0))
        local actor = EntityFinder():findOne({ player=0 })
        actor:giveOrder(GameObject.Order.Attack, target)
        actor:giveOrder(GameObject.Order.Stop)
    "#,
    );
    let first = rt.take_commands();
    assert_eq!(first.len(), 1);
    assert!(matches!(first[0].kind, CommandKind::Edit(Edit::SpawnRequest { .. })));
    let output = sim.step(&first);
    let mut commands = Vec::new();
    rt.think(&output.view, &output.events, &mut commands);
    let id = EntityId(rt.eval::<u32>("return target.id").unwrap());
    assert_ne!(id, EntityId::NONE);
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].player, PlayerId(0));
    assert_eq!(
        commands[0].kind,
        CommandKind::Order(Order::Attack {
            units: vec![hero],
            target: id
        })
    );
    assert_eq!(commands[1].player, PlayerId(0));
    assert_eq!(commands[1].kind, CommandKind::Order(Order::Stop { units: vec![hero] }));
    let output = sim.step(&commands);
    assert!(!output.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert!(
        sim.unit(hero).unwrap().target.is_none(),
        "the later Stop must remain effective"
    );
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn pending_edits_use_allocated_ids_and_keep_no_actions_after_their_first_dispatch() {
    let (mut sim, db) = setup();
    let mut rt = runtime(
        &sim,
        db,
        r#"
        survivor = Entity.add('fx_cruiser', 0, Vector(2000, 0, 0))
        survivor.invincible = true
        survivor.hull = survivor.hull / 2
        removed = Entity.add('fx_cruiser', 1, Vector(-2000, 0, 0))
        removed:remove()
    "#,
    );
    let first = rt.take_commands();
    assert_eq!(first.len(), 2);
    assert!(
        first
            .iter()
            .all(|c| matches!(c.kind, CommandKind::Edit(Edit::SpawnRequest { .. })))
    );
    let output = sim.step(&first);
    let mut edits = Vec::new();
    rt.think(&output.view, &output.events, &mut edits);
    let (survivor, removed): (u32, u32) = rt.eval("return survivor.id, removed.id").unwrap();
    let (survivor, removed) = (EntityId(survivor), EntityId(removed));
    assert_eq!(edits.len(), 3);
    assert!(edits.iter().all(|c| c.player == PlayerId::SCRIPT));
    assert_eq!(edits[0].kind, CommandKind::Edit(Edit::Immortal(survivor, true)));
    assert!(matches!(edits[1].kind, CommandKind::Edit(Edit::Damage(id, _)) if id == survivor));
    assert_eq!(edits[2].kind, CommandKind::Edit(Edit::Kill(removed)));
    let output = sim.step(&edits);
    let unit = output.view.unit(survivor).unwrap();
    assert!(unit.flags.contains(sta_sim::view::UnitFlags::IMMORTAL));
    assert_eq!(unit.hull[0], unit.hull[1] / sta_core::Fixed::from_int(2));
    assert!(output.view.unit(removed).is_none());
    assert!(!output.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    let mut later = Vec::new();
    rt.think(&output.view, &output.events, &mut later);
    assert!(later.is_empty());
    assert!(!rt.eval::<bool>("return removed.valid or removed.pending").unwrap());
}
