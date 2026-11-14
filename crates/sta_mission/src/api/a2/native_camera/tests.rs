//! Exact native A2 camera requests are typed, ordered presentation commands.

use std::sync::Arc;

use sta_content::{MapDef, MapObject};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::present::{Arg, Presentation};
use sta_sim::replay::{Recorder, Replay};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::UnitFlags;
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

fn scene() -> (GameSetup, Sim, EntityId, EntityId) {
    let content = Arc::new(sta_content::fake::db());
    let mut map = MapDef::empty();
    map.objects = [("fx_cruiser", 0, 0), ("fx_frigate", 1, 3000)]
        .into_iter()
        .map(|(odf, owner, x)| MapObject {
            odf: odf.into(),
            owner: Some(owner),
            pos: Vec3::from_f32([x as f32, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
            label: None,
        })
        .collect();
    let setup = GameSetup {
        seed: 71,
        map: Arc::new(map),
        content,
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
    let sim = Sim::from_setup(&setup);
    let first = sim.view().units.iter().find(|u| u.owner == PlayerId(0)).unwrap().id;
    let second = sim.view().units.iter().find(|u| u.owner == PlayerId(1)).unwrap().id;
    (setup, sim, first, second)
}

fn runtime(sim: &Sim, setup: &GameSetup) -> MissionRuntime {
    let rt = MissionRuntime::from_source_with(
        "camera-requests.lua",
        "",
        71,
        MissionConfig {
            content: Some(setup.content.clone()),
            view: Some((*sim.view()).clone()),
            ..MissionConfig::default()
        },
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

fn call(name: &str, args: Vec<Arg>) -> Presentation {
    Presentation::Call {
        name: format!("A2.{name}"),
        args,
    }
}

fn payloads(first: EntityId, second: EntityId) -> Vec<Presentation> {
    vec![
        call("CenterCamera", vec![Arg::Entity(first)]),
        call("SetCameraZoom", vec![Arg::Num(Fixed::from_f64(-0.5))]),
        call("CineractiveConstantSpeed", vec![Arg::Bool(false)]),
        call(
            "CineractiveCameraOffset",
            vec![
                Arg::Num(Fixed::ONE),
                Arg::Num(Fixed::from_int(-2)),
                Arg::Num(Fixed::ZERO),
                Arg::Bool(true),
            ],
        ),
        call(
            "CineractiveCameraOffset",
            vec![
                Arg::Text("CamPath".into()),
                Arg::Num(Fixed::from_f64(-3.5)),
                Arg::Bool(false),
            ],
        ),
        call("CineractiveShot", vec![Arg::Entity(first), Arg::Entity(second)]),
        call(
            "CineractiveShot",
            vec![Arg::Text("CamPath".into()), Arg::Int(0), Arg::Entity(second)],
        ),
        call(
            "CineractiveShot",
            vec![Arg::Text(String::new()), Arg::Num(Fixed::ZERO), Arg::Entity(second)],
        ),
    ]
}

fn requests(rt: &mut MissionRuntime, first: EntityId, second: EntityId) -> Vec<Command> {
    rt.eval::<()>(&format!(
        "A2.CenterCamera({}); A2.SetCameraZoom(-0.5); A2.CineractiveConstantSpeed(false); \
         A2.CineractiveCameraOffset(1,-2,0,true); A2.CineractiveCameraOffset('CamPath',-3.5,false); \
         A2.CineractiveShot({},{}); A2.CineractiveShot('CamPath',0,{}); A2.CineractiveShot('',0.0,{})",
        first.0, first.0, second.0, second.0, second.0
    ))
    .unwrap();
    rt.take_commands()
}

#[test]
fn eight_signatures_keep_exact_typed_payloads_and_script_sequence() {
    let (setup, sim, first, second) = scene();
    let mut rt = runtime(&sim, &setup);
    let commands = requests(&mut rt, first, second);
    let expected = payloads(first, second);
    assert_eq!(commands.len(), expected.len());
    for (i, (command, presentation)) in commands.iter().zip(&expected).enumerate() {
        assert_eq!(command.player, PlayerId::SCRIPT);
        assert_eq!(command.seq, i as u32 + 1);
        assert_eq!(command.kind, CommandKind::Edit(Edit::Present(presentation.clone())));
    }
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn sim_delivers_requests_without_world_digest_change_and_replay_repeats_events() {
    let (setup, mut sim, first, second) = scene();
    let mut control = sim.clone();
    let mut rt = runtime(&sim, &setup);
    let commands = requests(&mut rt, first, second);
    let out = sim.step(&commands);
    let quiet = control.step(&[]);
    assert_eq!(out.view.digest, quiet.view.digest);
    assert_eq!(sim.digest(), control.digest());
    let presented: Vec<_> = out
        .events
        .iter()
        .filter(|e| matches!(e, SimEvent::Present(_)))
        .cloned()
        .collect();
    assert_eq!(
        presented,
        payloads(first, second)
            .into_iter()
            .map(SimEvent::Present)
            .collect::<Vec<_>>()
    );
    let other: Vec<_> = out
        .events
        .iter()
        .filter(|e| !matches!(e, SimEvent::Present(_)))
        .cloned()
        .collect();
    assert_eq!(other, quiet.events);

    let mut recorder = Recorder::with_interval(&setup, 1);
    let replay_out = recorder.step(&commands);
    assert_eq!(replay_out.events, out.events);
    assert_eq!(replay_out.view.digest, out.view.digest);
    let final_digest = recorder.sim().digest();
    let replay = recorder.finish();
    let decoded = Replay::decode(&replay.encode()).unwrap();
    assert_eq!(decoded.play(setup.content.clone()).unwrap().digest(), final_digest);
    assert_eq!(decoded.commands.len(), commands.len());
    for (logged, command) in decoded.commands.iter().zip(commands) {
        assert_eq!(logged.command, command);
    }
}

#[test]
fn numeric_categories_and_boolean_values_are_not_reinterpreted() {
    let (setup, sim, _, second) = scene();
    let mut rt = runtime(&sim, &setup);
    rt.eval::<()>(&format!(
        "A2.SetCameraZoom(0); A2.CineractiveConstantSpeed(true); A2.CineractiveCameraOffset(0,-1,2,false); \
         A2.CineractiveShot('p',0,{}); A2.CineractiveShot('p',0.0,{})",
        second.0, second.0
    ))
    .unwrap();
    let presentations: Vec<_> = rt
        .take_commands()
        .into_iter()
        .map(|c| match c.kind {
            CommandKind::Edit(Edit::Present(p)) => p,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        presentations,
        vec![
            call("SetCameraZoom", vec![Arg::Num(Fixed::ZERO)]),
            call("CineractiveConstantSpeed", vec![Arg::Bool(true)]),
            call(
                "CineractiveCameraOffset",
                vec![
                    Arg::Num(Fixed::ZERO),
                    Arg::Num(Fixed::from_int(-1)),
                    Arg::Num(Fixed::from_int(2)),
                    Arg::Bool(false)
                ]
            ),
            call(
                "CineractiveShot",
                vec![Arg::Text("p".into()), Arg::Int(0), Arg::Entity(second)]
            ),
            call(
                "CineractiveShot",
                vec![Arg::Text("p".into()), Arg::Num(Fixed::ZERO), Arg::Entity(second)]
            ),
        ]
    );
}

#[test]
fn invalid_arity_types_range_strings_and_handles_queue_nothing() {
    let (setup, sim, first, _) = scene();
    let mut rt = runtime(&sim, &setup);
    let bad = [
        "A2.CenterCamera()".to_owned(),
        format!("A2.CenterCamera({},1)", first.0),
        "A2.CenterCamera('P', -1)".to_owned(),
        "A2.CenterCamera('P', 1.5)".to_owned(),
        "A2.CenterCamera(1, 2, 3)".to_owned(),
        "A2.CenterCamera(-1)".to_owned(),
        "A2.CenterCamera('1')".to_owned(),
        "A2.CenterCamera(4294967296)".to_owned(),
        "A2.CenterCamera(999999)".to_owned(),
        "A2.SetCameraZoom()".to_owned(),
        "A2.SetCameraZoom(2147483648)".to_owned(),
        "A2.SetCameraZoom(-2147483649)".to_owned(),
        "A2.SetCameraZoom(1/0)".to_owned(),
        "A2.SetCameraZoom('1')".to_owned(),
        "A2.CineractiveConstantSpeed(1)".to_owned(),
        "A2.CineractiveConstantSpeed(true,false)".to_owned(),
        "A2.CineractiveCameraOffset(1,2,3)".to_owned(),
        "A2.CineractiveCameraOffset(1,2,3,1)".to_owned(),
        "A2.CineractiveCameraOffset('p',1,0)".to_owned(),
        "A2.CineractiveCameraOffset(string.char(0),1,true)".to_owned(),
        "A2.CineractiveCameraOffset(string.char(255),1,true)".to_owned(),
        format!("A2.CineractiveShot({}, {})", first.0, "'missing'"),
        format!("A2.CineractiveShot('p',-1,{})", first.0),
        format!("A2.CineractiveShot('p',2147483648,{})", first.0),
        "A2.CineractiveShot('p',0,999999)".to_owned(),
        format!("A2.CineractiveShot('p',string.char(0),{})", first.0),
        format!("A2.CineractiveShot(string.char(0),0,{})", first.0),
        format!("A2.CineractiveShot(string.char(255),0,{})", first.0),
    ];
    for script in bad {
        assert!(rt.eval::<()>(&script).is_err(), "{script}");
        assert!(rt.take_commands().is_empty(), "partial command from {script}");
    }
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn dying_entity_validation_uses_the_current_view() {
    let (setup, mut sim, first, second) = scene();
    let mut rt = runtime(&sim, &setup);
    sim.units.get_mut(second).unwrap().flags = UnitFlags::DYING;
    let out = sim.step(&[]);
    rt.think(&out.view, &out.events, &mut Vec::new());
    for script in [
        format!("A2.CenterCamera({})", second.0),
        format!("A2.CineractiveShot({}, {})", first.0, second.0),
        format!("A2.CineractiveShot('p',0,{})", second.0),
    ] {
        assert!(rt.eval::<()>(&script).is_err(), "{script}");
        assert!(rt.take_commands().is_empty());
    }
}

#[test]
fn center_camera_on_a_path_point_keeps_the_path_name_and_point() {
    let (setup, sim, _, _) = scene();
    let mut rt = runtime(&sim, &setup);
    rt.eval::<()>("A2.CenterCamera('Camera_1', 0)").unwrap();
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        &commands[0].kind,
        CommandKind::Edit(Edit::Present(p)) if *p == call("CenterCamera", vec![Arg::Text("Camera_1".into()), Arg::Int(0)])
    ));
}
