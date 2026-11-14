use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{CommandKind, Edit, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::UnitFlags;
use sta_sim::{MoveStats, Sim, WorldSource};
use std::sync::Arc;

fn scene() -> (Sim, EntityId, EntityId) {
    let mut sim = Sim::new(7);
    let stats = MoveStats::from_f32(20.0, 1.0, 1.0, 1.0, 0.2);
    let a = sim.spawn(
        0,
        Vec3::from_f32([100.0, 20.0, -30.0]),
        (Fixed::ZERO, Fixed::ONE),
        stats,
    );
    let b = sim.spawn(
        1,
        Vec3::from_f32([200.0, 0.0, 0.0]),
        (Fixed::ZERO, Fixed::ONE),
        stats,
    );
    (sim, a, b)
}

fn runtime(sim: &Sim) -> MissionRuntime {
    let cfg = MissionConfig {
        view: Some((*sim.view()).clone()),
        ..MissionConfig::default()
    };
    let rt = MissionRuntime::from_source_with("objects.lua", "", 7, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

#[test]
fn existence_queries_use_view_membership_even_for_dying_or_zero_hull_units() {
    let (mut sim, live, dying) = scene();
    let zero_hull = sim.spawn(
        0,
        Vec3::from_f32([300.0, 0.0, 0.0]),
        (Fixed::ZERO, Fixed::ONE),
        MoveStats::STATIC,
    );
    let mut view = (*sim.view()).clone();
    let dying_unit = view.units.iter_mut().find(|u| u.id == dying).unwrap();
    dying_unit.flags = UnitFlags::DYING;
    view.units
        .iter_mut()
        .find(|u| u.id == zero_hull)
        .unwrap()
        .hull[0] = Fixed::ZERO;
    let cfg = MissionConfig {
        view: Some(view),
        ..MissionConfig::default()
    };
    let mut rt = MissionRuntime::from_source_with("exists.lua", "", 7, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());

    for (id, exists, alive) in [
        (live.0, true, true),
        (dying.0, true, false),
        (zero_hull.0, true, true),
        (0, false, false),
    ] {
        let result: (bool, bool, bool) = rt
            .eval(&format!(
                "return A2.Exists({id}), A2.DoesNotExist({id}), A2.IsAlive({id})"
            ))
            .unwrap();
        assert_eq!(result, (exists, !exists, alive), "id {id}");
    }
    assert!(rt.take_commands().is_empty());
}

#[test]
fn existence_queries_keep_the_old_view_until_think_receives_removal() {
    let (mut sim, id, _) = scene();
    let mut rt = runtime(&sim);
    rt.eval::<()>(&format!("A2.RemoveObject({})", id.0))
        .unwrap();
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].kind, CommandKind::Edit(Edit::Kill(id)));
    let out = sim.step(&commands);
    assert!(out.view.unit(id).is_none());

    let before: (bool, bool) = rt
        .eval(&format!(
            "return A2.Exists({0}), A2.DoesNotExist({0})",
            id.0
        ))
        .unwrap();
    assert_eq!(before, (true, false));
    rt.think(&out.view, &out.events, &mut Vec::new());
    let after: (bool, bool) = rt
        .eval(&format!(
            "return A2.Exists({0}), A2.DoesNotExist({0})",
            id.0
        ))
        .unwrap();
    assert_eq!(after, (false, true));
    assert!(rt.take_commands().is_empty());
}

#[test]
fn existence_queries_reject_bad_handles_without_queuing_commands() {
    let (sim, _, _) = scene();
    let mut rt = runtime(&sim);
    for name in ["Exists", "DoesNotExist"] {
        for args in ["", "1, 2", "-1", "1.5", "4294967296", "'1'", "0/0", "1/0"] {
            let script = format!("A2.{name}({args})");
            assert!(rt.eval::<()>(&script).is_err(), "{script}");
            assert!(rt.take_commands().is_empty(), "{script}");
        }
    }
}

#[test]
fn object_calls_queue_commands_and_read_the_latest_snapshot() {
    let (mut sim, a, b) = scene();
    let mut replay = sim.clone();
    let mut rt = runtime(&sim);
    let alive: bool = rt.eval(&format!("return A2.IsAlive({})", a.0)).unwrap();
    assert!(alive);
    let location: (f64, f64, f64) = rt.eval(&format!("local p = A2.GetLocation({}); return p.x, p.y, p.z", a.0)).unwrap();
    assert_eq!(location, (100.0, 20.0, -30.0));
    let owner: i64 = rt.eval(&format!("return A2.GetRealTeam({})", a.0)).unwrap();
    assert_eq!(owner, 0);

    rt.eval::<()>(&format!(
        "A2.CraftCannotDie({}, true); A2.Cloak({}); A2.Decloak({}); A2.RemoveObject({})",
        a.0, a.0, a.0, b.0
    ))
    .unwrap();
    let still_alive: bool = rt.eval(&format!("return A2.IsAlive({})", b.0)).unwrap();
    assert!(still_alive, "queued kill must not change the snapshot");
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 4);
    assert_eq!(
        (commands[0].player, &commands[0].kind),
        (
            PlayerId::SCRIPT,
            &CommandKind::Edit(Edit::Immortal(a, true))
        )
    );
    assert_eq!(
        (commands[1].player, &commands[1].kind),
        (
            PlayerId(0),
            &CommandKind::Order(Order::Cloak {
                units: vec![a],
                on: true
            })
        )
    );
    assert_eq!(
        (commands[2].player, &commands[2].kind),
        (
            PlayerId(0),
            &CommandKind::Order(Order::Cloak {
                units: vec![a],
                on: false
            })
        )
    );
    assert_eq!(
        (commands[3].player, &commands[3].kind),
        (PlayerId::SCRIPT, &CommandKind::Edit(Edit::Kill(b)))
    );

    let output = sim.step(&commands);
    assert_eq!(replay.step(&commands).events, output.events);
    assert_eq!(sim.digest(), replay.digest());
    assert!(
        sim.view()
            .unit(a)
            .unwrap()
            .flags
            .contains(UnitFlags::IMMORTAL)
    );
    assert!(sim.view().unit(b).is_none());
    // Cloak without content is rejected by Sim; the bridge proves that it
    // dispatched the owner commands, while the replay proves the outcome.
    let mut emitted = Vec::new();
    rt.think(&output.view, &output.events, &mut emitted);
    assert!(emitted.is_empty());
    let alive_after: bool = rt.eval(&format!("return A2.IsAlive({})", b.0)).unwrap();
    assert!(!alive_after);
}

#[test]
fn invalid_arguments_and_stale_handles_never_queue_commands() {
    let (mut sim, a, _) = scene();
    let unowned = sim.spawn(
        255,
        Vec3::ZERO,
        (Fixed::ZERO, Fixed::ONE),
        MoveStats::STATIC,
    );
    let mut rt = runtime(&sim);
    let missing: bool = rt.eval("return A2.IsAlive(0)").unwrap();
    assert!(!missing);
    let bad = [
        "A2.IsAlive(-1)",
        "A2.IsAlive(1.5)",
        "A2.IsAlive(4294967296)",
        "A2.IsAlive('1')",
        "A2.GetLocation(1, 2)",
        "A2.GetRealTeam(99999)",
        "A2.CraftCannotDie(1, 1)",
        "A2.CraftCannotDie(99999, true)",
        "A2.RemoveObject(99999)",
        "A2.Cloak(99999)",
        "A2.Decloak(99999)",
    ];
    for script in bad {
        assert!(rt.eval::<()>(script).is_err(), "{script}");
        assert!(
            rt.take_commands().is_empty(),
            "{script} partly queued a command"
        );
    }
    assert!(rt.eval::<()>(&format!("A2.Cloak({})", unowned.0)).is_err());
    assert!(rt.take_commands().is_empty());
    let mut dying = (*sim.view()).clone();
    dying.units.iter_mut().find(|u| u.id == a).unwrap().flags = UnitFlags::DYING;
    let cfg = MissionConfig {
        view: Some(dying),
        ..MissionConfig::default()
    };
    let mut rt = MissionRuntime::from_source_with("dying.lua", "", 7, cfg);
    assert!(
        !rt.eval::<bool>(&format!("return A2.IsAlive({})", a.0))
            .unwrap()
    );
    assert!(rt.eval::<()>(&format!("A2.RemoveObject({})", a.0)).is_err());
    assert!(rt.take_commands().is_empty());
}

#[test]
fn cloak_and_decloak_commands_apply_and_replay_with_capable_content() {
    let db = Arc::new(sta_sim::specials::fixture::db());
    let setup = GameSetup {
        seed: 7,
        map: Arc::new(sta_content::MapDef::empty()),
        content: db.clone(),
        players: vec![PlayerSetup {
            id: PlayerId(0),
            team: TeamId(0),
            race: "fx".into(),
            control: Control::Human,
        }],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let id = sim
        .spawn_def(
            db.key("fx_cloaker_loud").unwrap(),
            PlayerId(0),
            Vec3::ZERO,
            (Fixed::ZERO, Fixed::ONE),
        )
        .unwrap();
    let mut replay = sim.clone();
    let mut rt = runtime(&sim);

    rt.eval::<()>(&format!("A2.Cloak({})", id.0)).unwrap();
    let engage = rt.take_commands();
    assert_eq!(engage.len(), 1);
    assert_eq!(engage[0].player, PlayerId(0));
    assert_eq!(
        engage[0].kind,
        CommandKind::Order(Order::Cloak {
            units: vec![id],
            on: true
        })
    );
    let out = sim.step(&engage);
    assert_eq!(replay.step(&engage).events, out.events);
    assert!(
        !out.events
            .iter()
            .any(|e| matches!(e, sta_sim::SimEvent::Rejected { .. }))
    );
    assert!(
        out.view
            .unit(id)
            .unwrap()
            .flags
            .contains(UnitFlags::CLOAKED)
    );

    let mut emitted = Vec::new();
    rt.think(&out.view, &out.events, &mut emitted);
    rt.eval::<()>(&format!("A2.Decloak({})", id.0)).unwrap();
    let disengage = rt.take_commands();
    assert_eq!(disengage.len(), 1);
    assert_eq!(disengage[0].player, PlayerId(0));
    assert_eq!(
        disengage[0].kind,
        CommandKind::Order(Order::Cloak {
            units: vec![id],
            on: false
        })
    );
    let out = sim.step(&disengage);
    assert_eq!(replay.step(&disengage).events, out.events);
    assert!(
        !out.events
            .iter()
            .any(|e| matches!(e, sta_sim::SimEvent::Rejected { .. }))
    );
    assert!(
        !out.view
            .unit(id)
            .unwrap()
            .flags
            .contains(UnitFlags::CLOAKED)
    );
    assert_eq!(sim.digest(), replay.digest());
}
