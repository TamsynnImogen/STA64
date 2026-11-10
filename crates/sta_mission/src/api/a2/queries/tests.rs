use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::UnitFlags;
use sta_sim::{MoveStats, Sim, WorldSource, WorldView};

use crate::api::{MissionConfig, PlayerNumbering};
use crate::runtime::MissionRuntime;

fn scene_at(pos: Vec3) -> (Sim, Arc<ContentDb>, EntityId) {
    let db = Arc::new(fake::db());
    let setup = GameSetup {
        seed: 19,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![PlayerSetup {
            id: PlayerId(0),
            team: TeamId(0),
            race: "fx".into(),
            control: Control::Idle,
        }],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let id = sim
        .spawn_def(
            db.key("fx_cruiser").unwrap(),
            PlayerId(0),
            pos,
            (Fixed::ZERO, Fixed::ONE),
        )
        .unwrap();
    (sim, db, id)
}

fn scene() -> (Sim, Arc<ContentDb>, EntityId) {
    scene_at(Vec3::ZERO)
}

fn runtime(sim: &Sim, db: Option<Arc<ContentDb>>) -> MissionRuntime {
    let cfg = MissionConfig {
        content: db,
        view: Some((*sim.view()).clone()),
        ..MissionConfig::default()
    };
    let rt = MissionRuntime::from_source_with("queries.lua", "", 19, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

#[test]
fn health_tracks_fresh_snapshot_and_is_odf_matches_only_the_definition_name() {
    let (mut sim, db, id) = scene();
    let mut rt = runtime(&sim, Some(db));
    let before: (f64, f64, bool, bool, bool, bool) = rt.eval(&format!(
        "return A2.GetCurrentHealth({}), A2.GetMaxHealth({}), A2.IsOdf({},'fx_cruiser'), A2.IsOdf({},'FX_CRUISER.ODF'), A2.IsOdf({},'craft'), A2.IsOdf({},'other/fx_cruiser.odf')",
        id.0, id.0, id.0, id.0, id.0, id.0
    )).unwrap();
    assert_eq!(before, (800.0, 800.0, true, true, false, false));
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty());

    let out = sim.step(&[Command {
        player: PlayerId::SCRIPT,
        seq: 1,
        kind: CommandKind::Edit(Edit::Damage(id, Fixed::from_int(100))),
    }]);
    let stale: f64 = rt.eval(&format!("return A2.GetCurrentHealth({})", id.0)).unwrap();
    assert_eq!(stale, 800.0, "query should retain the previous view until think");
    let mut commands = Vec::new();
    rt.think(&out.view, &out.events, &mut commands);
    let after: (f64, f64) = rt
        .eval(&format!(
            "return A2.GetCurrentHealth({}), A2.GetMaxHealth({})",
            id.0, id.0
        ))
        .unwrap();
    assert_eq!(after, (700.0, 800.0));
    assert!(commands.is_empty());

    let out = sim.step(&[Command {
        player: PlayerId::SCRIPT,
        seq: 2,
        kind: CommandKind::Edit(Edit::Kill(id)),
    }]);
    rt.think(&out.view, &out.events, &mut commands);
    assert!(
        rt.eval::<f64>(&format!("return A2.GetCurrentHealth({})", id.0))
            .is_err()
    );
    assert!(rt.eval::<f64>(&format!("return A2.GetMaxHealth({})", id.0)).is_err());
    assert!(
        !rt.eval::<bool>(&format!("return A2.IsOdf({},'fx_cruiser')", id.0))
            .unwrap()
    );
    assert!(rt.take_commands().is_empty() && commands.is_empty() && rt.unsupported().is_empty());
}

#[test]
fn is_race_compares_only_the_live_definition_race_literal_case_insensitively() {
    let (sim, db, id) = scene();
    let mut rt = runtime(&sim, Some(db));
    let result: (bool, bool, bool, bool) = rt
        .eval(&format!(
            "return A2.IsRace({},'fx'), A2.IsRace({},'FX'), A2.IsRace({},'fx.odf'), A2.IsRace({},'other/fx')",
            id.0, id.0, id.0, id.0
        ))
        .unwrap();
    assert_eq!(result, (true, true, false, false));
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn malformed_inputs_are_rejected_without_commands() {
    let (sim, db, id) = scene();
    let mut rt = runtime(&sim, Some(db));
    for script in [
        "A2.GetCurrentHealth()",
        "A2.GetCurrentHealth(1,2)",
        "A2.GetCurrentHealth(-1)",
        "A2.GetMaxHealth(1.5)",
        "A2.GetMaxHealth('1')",
        "A2.GetCurrentHealth(4294967296)",
        "A2.GetMaxHealth(0/0)",
        "A2.GetCurrentHealth(99999)",
        "A2.IsOdf(1)",
        "A2.IsOdf(1,'fx_cruiser',true)",
        "A2.IsOdf(-1,'fx_cruiser')",
        "A2.IsOdf(1.5,'fx_cruiser')",
        "A2.IsOdf(1,123)",
        "A2.IsOdf(1,nil)",
        "A2.IsRace()",
        "A2.IsRace(1)",
        "A2.IsRace(1,'fx',true)",
        "A2.IsRace(-1,'fx')",
        "A2.IsRace(1.5,'fx')",
        "A2.IsRace('1','fx')",
        "A2.IsRace(4294967296,'fx')",
        "A2.IsRace(1,123)",
        "A2.IsRace(1,nil)",
        "A2.IsRace(1,string.char(255))",
    ] {
        assert!(rt.eval::<()>(script).is_err(), "{script}");
        assert!(rt.take_commands().is_empty(), "{script} queued a command");
    }
    assert!(!rt.eval::<bool>("return A2.IsOdf(99999,'fx_cruiser')").unwrap());
    assert!(!rt.eval::<bool>("return A2.IsRace(99999,'fx')").unwrap());
    assert!(rt.unsupported().is_empty());

    let mut dying = (*sim.view()).clone();
    dying.units.iter_mut().find(|u| u.id == id).unwrap().flags = UnitFlags::DYING;
    let cfg = MissionConfig {
        content: Some(Arc::new(fake::db())),
        view: Some(dying),
        ..MissionConfig::default()
    };
    let mut rt = MissionRuntime::from_source_with("dying.lua", "", 19, cfg);
    assert!(
        rt.eval::<f64>(&format!("return A2.GetCurrentHealth({})", id.0))
            .is_err()
    );
    assert!(rt.eval::<f64>(&format!("return A2.GetMaxHealth({})", id.0)).is_err());
    assert!(
        !rt.eval::<bool>(&format!("return A2.IsOdf({},'fx_cruiser')", id.0))
            .unwrap()
    );
    assert!(!rt.eval::<bool>(&format!("return A2.IsRace({},'fx')", id.0)).unwrap());
    assert!(rt.take_commands().is_empty() && rt.unsupported().is_empty());
}

#[test]
fn live_unit_without_content_has_health_but_is_odf_errors() {
    let mut sim = Sim::new(19);
    let id = sim.spawn(
        0,
        Vec3::ZERO,
        (Fixed::ZERO, Fixed::ONE),
        MoveStats::from_f32(20.0, 1.0, 1.0, 1.0, 0.2),
    );
    let mut rt = runtime(&sim, None);
    let health: (f64, f64) = rt
        .eval(&format!(
            "return A2.GetCurrentHealth({}), A2.GetMaxHealth({})",
            id.0, id.0
        ))
        .unwrap();
    assert_eq!(health.0, health.1);
    assert!(
        rt.eval::<bool>(&format!("return A2.IsOdf({},'fx_cruiser')", id.0))
            .is_err()
    );
    assert!(rt.eval::<bool>(&format!("return A2.IsRace({},'fx')", id.0)).is_err());
    assert!(rt.take_commands().is_empty() && rt.unsupported().is_empty());
}

fn distance_scene() -> (Sim, Arc<ContentDb>, EntityId, EntityId) {
    let (mut sim, db, first) = scene_at(Vec3::from_f32([-0.5, -1.25, -2.0]));
    let second = sim
        .spawn_def(
            db.key("fx_cruiser").unwrap(),
            PlayerId(0),
            Vec3::from_f32([2.5, 2.75, 10.0]),
            (Fixed::ZERO, Fixed::ONE),
        )
        .unwrap();
    (sim, db, first, second)
}

#[test]
fn get_distance_uses_three_axes_and_waits_for_the_latest_snapshot() {
    let (sim, db, first, second) = distance_scene();
    let mut rt = runtime(&sim, Some(db));
    let query = format!(
        "return A2.GetDistance({},{}), A2.GetDistance({},{}), A2.GetDistance({},{})",
        first.0, second.0, second.0, first.0, first.0, first.0
    );
    let values: (f64, f64, f64) = rt.eval(&query).unwrap();
    assert_eq!(
        values,
        (13.0, 13.0, 0.0),
        "negative and fractional coordinates still use 3D distance"
    );
    let mut fresh = (*sim.view()).clone();
    let new_pos = fresh.unit(second).unwrap().pos;
    fresh.units.iter_mut().find(|u| u.id == first).unwrap().pos = new_pos;
    fresh.tick += 1;
    assert_eq!(
        rt.eval::<f64>(&format!("return A2.GetDistance({},{})", first.0, second.0))
            .unwrap(),
        13.0
    );
    let mut commands = Vec::new();
    rt.think(&fresh, &[], &mut commands);
    assert_eq!(
        rt.eval::<f64>(&format!("return A2.GetDistance({},{})", first.0, second.0))
            .unwrap(),
        0.0
    );
    assert!(commands.is_empty() && rt.take_commands().is_empty() && rt.unsupported().is_empty());
}

#[test]
fn get_distance_rejects_invalid_handles_and_dying_objects_on_either_side() {
    let (sim, db, first, second) = distance_scene();
    let mut rt = runtime(&sim, Some(db.clone()));
    for script in [
        "A2.GetDistance()".to_owned(),
        format!("A2.GetDistance({})", first.0),
        format!("A2.GetDistance({},{},0)", first.0, second.0),
        format!("A2.GetDistance(99999,{})", first.0),
        format!("A2.GetDistance({},99999)", first.0),
    ] {
        assert!(rt.eval::<()>(&script).is_err(), "{script}");
    }
    for bad in [
        "-1",
        "1.5",
        "'1'",
        "0/0",
        "math.huge",
        "-math.huge",
        "4294967296",
        "true",
        "nil",
    ] {
        for script in [
            format!("A2.GetDistance({bad},{})", second.0),
            format!("A2.GetDistance({},{bad})", first.0),
        ] {
            assert!(rt.eval::<()>(&script).is_err(), "{script}");
        }
    }
    assert!(rt.take_commands().is_empty() && rt.unsupported().is_empty());

    for dying in [first, second] {
        let mut view = (*sim.view()).clone();
        view.units.iter_mut().find(|u| u.id == dying).unwrap().flags = UnitFlags::DYING;
        let cfg = MissionConfig {
            content: Some(db.clone()),
            view: Some(view),
            ..MissionConfig::default()
        };
        let mut rt = MissionRuntime::from_source_with("dying-distance.lua", "", 19, cfg);
        assert!(
            rt.eval::<f64>(&format!("return A2.GetDistance({},{})", first.0, second.0))
                .is_err()
        );
        assert!(rt.take_commands().is_empty() && rt.unsupported().is_empty());
    }
}

fn visible_runtime(view: WorldView, numbering: PlayerNumbering) -> MissionRuntime {
    let cfg = MissionConfig {
        player_numbering: numbering,
        view: Some(view),
        ..MissionConfig::default()
    };
    let rt = MissionRuntime::from_source_with("visibility.lua", "", 19, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

#[test]
fn is_visible_to_team_uses_configured_player_numbers_and_snapshot_membership() {
    let (sim, _, id) = scene();
    let mut view = (*sim.view()).clone();
    view.visibility = vec![(PlayerId(0), vec![id]), (PlayerId(1), vec![]), (PlayerId(7), vec![id])];
    // The supplied visibility list is authoritative, including for cloaked units.
    view.units.iter_mut().find(|unit| unit.id == id).unwrap().flags = UnitFlags::CLOAKED;
    for (numbering, visible, hidden, eighth) in [
        (PlayerNumbering::Slots, 0, 1, 7),
        (PlayerNumbering::ArmadaTeams, 1, 2, 8),
    ] {
        let mut rt = visible_runtime(view.clone(), numbering);
        let query = format!(
            "return A2.IsVisibleToTeam({},{visible}), A2.IsVisibleToTeam({},{hidden}), A2.IsVisibleToTeam({},{eighth})",
            id.0, id.0, id.0
        );
        assert_eq!(rt.eval::<(bool, bool, bool)>(&query).unwrap(), (true, false, true));
        assert!(
            !rt.eval::<bool>(&format!("return A2.IsVisibleToTeam(99999,{visible})"))
                .unwrap()
        );
        let mut fresh = view.clone();
        fresh.tick += 1;
        fresh.visibility = vec![(PlayerId(0), vec![]), (PlayerId(1), vec![id]), (PlayerId(7), vec![])];
        assert!(
            rt.eval::<bool>(&format!("return A2.IsVisibleToTeam({},{visible})", id.0))
                .unwrap()
        );
        let mut commands = Vec::new();
        rt.think(&fresh, &[], &mut commands);
        assert_eq!(
            rt.eval::<(bool, bool)>(&format!(
                "return A2.IsVisibleToTeam({},{visible}), A2.IsVisibleToTeam({},{hidden})",
                id.0, id.0
            ))
            .unwrap(),
            (false, true)
        );
        assert!(commands.is_empty() && rt.take_commands().is_empty() && rt.unsupported().is_empty());
    }
}

#[test]
fn is_visible_to_team_rejects_invalid_player_numbers_before_missing_entity_check() {
    let (sim, _, id) = scene();
    let view = (*sim.view()).clone();
    for (numbering, valid, invalid) in [
        (PlayerNumbering::Slots, 0, vec!["8", "253", "254", "255"]),
        (PlayerNumbering::ArmadaTeams, 1, vec!["0", "9", "253", "254", "255"]),
    ] {
        let mut rt = visible_runtime(view.clone(), numbering);
        for bad in invalid.into_iter().chain([
            "256",
            "-1",
            "1.5",
            "'1'",
            "0/0",
            "math.huge",
            "4294967296",
            "true",
            "nil",
        ]) {
            for object in [id.0, 99999] {
                let script = format!("A2.IsVisibleToTeam({object},{bad})");
                assert!(rt.eval::<()>(&script).is_err(), "{script}");
            }
        }
        for script in [
            "A2.IsVisibleToTeam()".to_owned(),
            format!("A2.IsVisibleToTeam({})", id.0),
            format!("A2.IsVisibleToTeam({id},{valid},0)", id = id.0),
        ] {
            assert!(rt.eval::<()>(&script).is_err(), "{script}");
        }
        for bad in ["-1", "1.5", "'1'", "0/0", "math.huge", "4294967296", "true", "nil"] {
            let script = format!("A2.IsVisibleToTeam({bad},{valid})");
            assert!(rt.eval::<()>(&script).is_err(), "{script}");
        }
        assert!(rt.take_commands().is_empty() && rt.unsupported().is_empty());
    }
}

#[test]
fn is_visible_to_team_missing_or_dying_is_false_and_empty_visibility_is_omniscient() {
    let (sim, _, id) = scene();
    let mut view = (*sim.view()).clone();
    view.visibility.clear();
    let mut rt = visible_runtime(view.clone(), PlayerNumbering::Slots);
    assert!(
        rt.eval::<bool>(&format!("return A2.IsVisibleToTeam({},7)", id.0))
            .unwrap()
    );
    assert!(!rt.eval::<bool>("return A2.IsVisibleToTeam(99999,7)").unwrap());
    view.visibility = vec![(PlayerId(0), vec![id, EntityId(99999)])];
    view.units.iter_mut().find(|unit| unit.id == id).unwrap().flags = UnitFlags::DYING;
    view.tick += 1;
    let mut commands = Vec::new();
    rt.think(&view, &[], &mut commands);
    assert!(
        !rt.eval::<bool>(&format!("return A2.IsVisibleToTeam({},0)", id.0))
            .unwrap()
    );
    assert!(!rt.eval::<bool>("return A2.IsVisibleToTeam(99999,0)").unwrap());
    assert!(commands.is_empty() && rt.take_commands().is_empty() && rt.unsupported().is_empty());
}
