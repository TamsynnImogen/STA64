//! Public mission/runtime regressions for absolute hull writes.

use std::collections::BTreeMap;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MapObject, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_mission::api::MissionConfig;
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::UnitFlags;
use sta_sim::{Sim, SimEvent, WorldSource};

fn scene() -> (GameSetup, Sim, Arc<ContentDb>, [EntityId; 3]) {
    let content = Arc::new(fake::db());
    let mut map = MapDef::empty();
    map.objects = [("hull_a", -1000), ("hull_b", 0), ("hull_c", 1000)]
        .into_iter()
        .map(|(label, x)| MapObject {
            odf: "fx_cruiser".into(),
            owner: Some(0),
            pos: Vec3::from_f32([x as f32, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
            label: Some(label.into()),
        })
        .collect();
    let setup = GameSetup {
        seed: 47,
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
    let id_for = |name: &str| {
        sim.labels
            .iter()
            .find_map(|(id, label)| (label.as_str() == name).then_some(id))
            .expect("labeled map unit")
    };
    let ids = [id_for("hull_a"), id_for("hull_b"), id_for("hull_c")];
    (setup, sim, content, ids)
}

fn runtime(sim: &Sim, content: Arc<ContentDb>, script: &str) -> MissionRuntime {
    let labels = sim
        .labels
        .iter()
        .map(|(id, label)| (id, label.clone()))
        .collect::<BTreeMap<_, _>>();
    MissionRuntime::from_source_with(
        "missions/exact-hull.lua",
        script,
        47,
        MissionConfig {
            content: Some(content),
            labels,
            view: Some((*sim.view()).clone()),
            ..MissionConfig::default()
        },
    )
}

#[test]
fn repeated_hull_writes_apply_last_absolute_value_and_allow_partial_repair() {
    let (_, mut sim, content, [a, b, _]) = scene();
    let shields_a = sim.view().unit(a).unwrap().shields;
    let shields_b = sim.view().unit(b).unwrap().shields;
    let damage = Command {
        player: PlayerId::BENCH,
        seq: 1,
        kind: CommandKind::Edit(Edit::Damage(b, Fixed::from_int(600))),
    };
    let damaged = sim.step(&[damage]);
    assert!(
        !damaged
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert_eq!(sim.unit(b).unwrap().hull[0], Fixed::from_int(200));

    let mut rt = runtime(
        &sim,
        content,
        r#"
        a = EntityFinder():findOne('hull_a')
        b = EntityFinder():findOne('hull_b')
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.eval::<()>(
        r#"
        a.hull = 500
        a.hull = 800 -- equal to the cached view, but must overwrite the prior write
        b.hull = 350 -- partial repair above its current 200 hull
        "#,
    )
    .unwrap();
    assert_eq!(
        rt.eval::<(f64, f64)>("return a.hull, b.hull").unwrap(),
        (800.0, 200.0)
    );

    let commands = rt.take_commands();
    assert_eq!(commands.len(), 3);
    assert_eq!(
        commands
            .iter()
            .map(|command| command.player)
            .collect::<Vec<_>>(),
        [PlayerId::SCRIPT; 3]
    );
    assert_eq!(
        commands[0].kind,
        CommandKind::Edit(Edit::Hull(a, Fixed::from_int(500)))
    );
    assert_eq!(
        commands[1].kind,
        CommandKind::Edit(Edit::Hull(a, Fixed::from_int(800)))
    );
    assert_eq!(
        commands[2].kind,
        CommandKind::Edit(Edit::Hull(b, Fixed::from_int(350)))
    );
    let out = sim.step(&commands);
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert_eq!(out.view.unit(a).unwrap().hull[0], Fixed::from_int(800));
    assert_eq!(out.view.unit(b).unwrap().hull[0], Fixed::from_int(350));
    assert_eq!(out.view.unit(a).unwrap().shields, shields_a);
    assert_eq!(out.view.unit(b).unwrap().shields, shields_b);
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert_eq!(
        rt.eval::<(f64, f64)>("return a.hull, b.hull").unwrap(),
        (800.0, 350.0)
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn pending_hull_writes_wait_for_receipt_and_apply_to_the_actual_id() {
    let (_, mut sim, content, initial_ids) = scene();
    let mut rt = runtime(
        &sim,
        content,
        r#"
        pending = Entity.add('fx_cruiser', 0, Vector(2500,0,0))
        pending.hull = 300
        pending.hull = 450
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
    let out = sim.step(&first);
    let id = out
        .events
        .iter()
        .find_map(|event| match event {
            SimEvent::ScriptSpawned { request: got, id } if *got == request => Some(*id),
            _ => None,
        })
        .expect("spawn receipt");
    assert!(!initial_ids.contains(&id));
    let mut deferred = Vec::new();
    rt.think(&out.view, &out.events, &mut deferred);
    assert_eq!(deferred.len(), 2);
    assert_eq!(
        deferred[0].kind,
        CommandKind::Edit(Edit::Hull(id, Fixed::from_int(300)))
    );
    assert_eq!(
        deferred[1].kind,
        CommandKind::Edit(Edit::Hull(id, Fixed::from_int(450)))
    );
    assert_eq!(rt.eval::<u32>("return pending.id").unwrap(), id.0);

    let applied = sim.step(&deferred);
    assert!(
        !applied
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert_eq!(applied.view.unit(id).unwrap().hull[0], Fixed::from_int(450));
    let mut later = Vec::new();
    rt.think(&applied.view, &applied.events, &mut later);
    assert!(later.is_empty());
    assert_eq!(rt.eval::<f64>("return pending.hull").unwrap(), 450.0);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn a2_fractional_health_writes_are_hull_edits_visible_after_think() {
    let (_, mut sim, content, [id, _, _]) = scene();
    let mut rt = runtime(
        &sim,
        content,
        "unit = EntityFinder():findOne('hull_a'); unit_id = unit.id",
    );
    rt.eval::<()>("A2.SetCurrentHealth(unit_id, 250.5); A2.SetCurrentHealth(unit_id, 300.25)")
        .unwrap();
    assert_eq!(
        rt.eval::<f64>("return A2.GetCurrentHealth(unit_id)")
            .unwrap(),
        800.0
    );
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 2);
    assert_eq!(
        commands[0].kind,
        CommandKind::Edit(Edit::Hull(id, Fixed::from_f64(250.5)))
    );
    assert_eq!(
        commands[1].kind,
        CommandKind::Edit(Edit::Hull(id, Fixed::from_f64(300.25)))
    );
    let out = sim.step(&commands);
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert_eq!(out.view.unit(id).unwrap().hull[0], Fixed::from_f64(300.25));
    assert_eq!(
        rt.eval::<f64>("return A2.GetCurrentHealth(unit_id)")
            .unwrap(),
        800.0
    );
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert_eq!(
        rt.eval::<f64>("return A2.GetCurrentHealth(unit_id)")
            .unwrap(),
        300.25
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn malformed_health_writes_error_without_commands_or_world_changes() {
    let (_, mut sim, content, [id, _, _]) = scene();
    let mut rt = runtime(
        &sim,
        content.clone(),
        r#"
        unit = EntityFinder():findOne('hull_a')
        unit_id = unit.id
        local malformed = {
            function() A2.SetCurrentHealth() end,
            function() A2.SetCurrentHealth(unit_id, 1, 2) end,
            function() A2.SetCurrentHealth(-1, 10) end,
            function() A2.SetCurrentHealth(1.5, 10) end,
            function() A2.SetCurrentHealth('1', 10) end,
            function() A2.SetCurrentHealth(math.huge, 10) end,
            function() A2.SetCurrentHealth(0/0, 10) end,
            function() A2.SetCurrentHealth(unit_id, nil) end,
            function() A2.SetCurrentHealth(unit_id, true) end,
            function() A2.SetCurrentHealth(unit_id, '10') end,
            function() A2.SetCurrentHealth(unit_id, {}) end,
            function() A2.SetCurrentHealth(unit_id, math.huge) end,
            function() A2.SetCurrentHealth(unit_id, -math.huge) end,
            function() A2.SetCurrentHealth(unit_id, 0/0) end,
            function() A2.SetCurrentHealth(99999, 10) end,
            function() unit.hull = math.huge end,
            function() unit.hull = 0/0 end
        }
        for _, call in ipairs(malformed) do assert(not pcall(call)) end
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.take_commands().is_empty());
    assert_eq!(sim.unit(id).unwrap().hull[0], Fixed::from_int(800));
    assert!(sim.step(&[]).view.unit(id).is_some());

    let mut dying = (*sim.view()).clone();
    dying
        .units
        .iter_mut()
        .find(|unit| unit.id == id)
        .unwrap()
        .flags = UnitFlags::DYING;
    let labels = sim
        .labels
        .iter()
        .map(|(id, label)| (id, label.clone()))
        .collect();
    let mut dying_rt = MissionRuntime::from_source_with(
        "missions/dying-health.lua",
        &format!("unit_id = {}", id.0),
        47,
        MissionConfig {
            content: Some(content),
            labels,
            view: Some(dying),
            ..MissionConfig::default()
        },
    );
    assert!(dying_rt.errors().is_empty(), "{:?}", dying_rt.errors());
    assert!(
        dying_rt
            .eval::<bool>("return not pcall(function() A2.SetCurrentHealth(unit_id, 100) end)")
            .unwrap()
    );
    assert!(dying_rt.take_commands().is_empty());
}

#[test]
fn huge_finite_health_values_clamp_and_respect_immortal_lethal_rules() {
    let (_, mut sim, content, [a, b, c]) = scene();
    let shields_a = sim.view().unit(a).unwrap().shields;
    let shields_c = sim.view().unit(c).unwrap().shields;
    let mut rt = runtime(
        &sim,
        content,
        r#"
        local a = EntityFinder():findOne('hull_a')
        local b = EntityFinder():findOne('hull_b')
        A2.SetCurrentHealth(a.id, 1e300)
        A2.SetCurrentHealth(b.id, -1e300)
        immortal = EntityFinder():findOne('hull_c')
        immortal.invincible = true
        A2.SetCurrentHealth(immortal.id, -1e300)
        "#,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 4);
    assert!(
        commands
            .iter()
            .all(|command| command.player == PlayerId::SCRIPT)
    );
    assert_eq!(
        commands[0].kind,
        CommandKind::Edit(Edit::Hull(a, Fixed::from_f64(1e300)))
    );
    assert_eq!(
        commands[1].kind,
        CommandKind::Edit(Edit::Hull(b, Fixed::from_f64(-1e300)))
    );
    assert_eq!(commands[2].kind, CommandKind::Edit(Edit::Immortal(c, true)));
    assert_eq!(
        commands[3].kind,
        CommandKind::Edit(Edit::Hull(c, Fixed::from_f64(-1e300)))
    );

    let out = sim.step(&commands);
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert_eq!(
        out.view.unit(a).unwrap().hull[0],
        out.view.unit(a).unwrap().hull[1]
    );
    assert!(out.view.unit(b).is_none());
    assert_eq!(out.view.unit(c).unwrap().hull[0], Fixed::ONE);
    assert!(
        out.view
            .unit(c)
            .unwrap()
            .flags
            .contains(UnitFlags::IMMORTAL)
    );
    assert_eq!(out.view.unit(a).unwrap().shields, shields_a);
    assert_eq!(out.view.unit(c).unwrap().shields, shields_c);
    rt.think(&out.view, &out.events, &mut Vec::new());

    rt.eval::<()>("immortal.invincible = false; A2.SetCurrentHealth(immortal.id, -1e300)")
        .unwrap();
    let lethal = rt.take_commands();
    assert_eq!(lethal.len(), 2);
    assert_eq!(lethal[0].kind, CommandKind::Edit(Edit::Immortal(c, false)));
    assert_eq!(
        lethal[1].kind,
        CommandKind::Edit(Edit::Hull(c, Fixed::from_f64(-1e300)))
    );
    let out = sim.step(&lethal);
    assert!(out.view.unit(c).is_none());
    assert!(
        out.events
            .iter()
            .any(|event| matches!(event, SimEvent::Destroyed { id, .. } if *id == c))
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}
