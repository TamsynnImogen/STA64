//! Exact A2 presentation requests are typed, ordered presentation commands.

use std::sync::Arc;

use sta_content::{MapDef, MapObject};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{CommandKind, Edit};
use sta_sim::present::{Arg, Presentation};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

fn scene() -> (GameSetup, Sim, EntityId) {
    let content = Arc::new(sta_content::fake::db());
    let mut map = MapDef::empty();
    map.objects = vec![MapObject {
        odf: "fx_cruiser".into(),
        owner: Some(0),
        pos: Vec3::from_f32([0.0, 0.0, 0.0]),
        heading: (Fixed::ZERO, Fixed::ONE),
        label: None,
    }];
    let setup = GameSetup {
        seed: 72,
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
    let id = sim.view().units[0].id;
    (setup, sim, id)
}

fn runtime(sim: &Sim, setup: &GameSetup) -> MissionRuntime {
    let rt = MissionRuntime::from_source_with(
        "presentation-requests.lua",
        "",
        72,
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
    Presentation::Call { name: format!("A2.{name}"), args }
}

#[test]
fn seven_signatures_keep_typed_payloads_in_script_order_and_reach_the_sim() {
    let (setup, mut sim, id) = scene();
    let mut control = sim.clone();
    let mut rt = runtime(&sim, &setup);
    rt.eval::<()>(&format!(
        "A2.AddTransporterEffect({}); A2.RemoveBillboardEffect(-3); A2.GridVisible(false); \
         A2.RemoveFromAdLog(7); A2.Starfield_Load_New_Background_Geometry('a','b'); \
         A2.MinimapMessage(1,2,0.5); A2.MinimapMessage('txt',3,-1.5,4)",
        id.0
    ))
    .unwrap();
    let commands = rt.take_commands();
    let expected = vec![
        call("AddTransporterEffect", vec![Arg::Entity(id)]),
        call("RemoveBillboardEffect", vec![Arg::Int(-3)]),
        call("GridVisible", vec![Arg::Bool(false)]),
        call("RemoveFromAdLog", vec![Arg::Int(7)]),
        call(
            "Starfield_Load_New_Background_Geometry",
            vec![Arg::Text("a".into()), Arg::Text("b".into())],
        ),
        call(
            "MinimapMessage",
            vec![Arg::Int(1), Arg::Int(2), Arg::Num(Fixed::from_f64(0.5))],
        ),
        call(
            "MinimapMessage",
            vec![
                Arg::Text("txt".into()),
                Arg::Int(3),
                Arg::Num(Fixed::from_f64(-1.5)),
                Arg::Int(4),
            ],
        ),
    ];
    assert_eq!(commands.len(), expected.len());
    for (i, (command, presentation)) in commands.iter().zip(&expected).enumerate() {
        assert_eq!(command.player, PlayerId::SCRIPT);
        assert_eq!(command.seq, i as u32 + 1);
        assert_eq!(command.kind, CommandKind::Edit(Edit::Present(presentation.clone())));
    }
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());

    let out = sim.step(&commands);
    let quiet = control.step(&[]);
    assert_eq!(out.view.digest, quiet.view.digest, "presentation changes no world state");
    let presented: Vec<_> = out
        .events
        .iter()
        .filter(|e| matches!(e, SimEvent::Present(_)))
        .cloned()
        .collect();
    assert_eq!(presented, expected.into_iter().map(SimEvent::Present).collect::<Vec<_>>());
}

#[test]
fn invalid_arity_types_ranges_and_handles_queue_nothing() {
    let (setup, sim, id) = scene();
    let mut rt = runtime(&sim, &setup);
    let bad = [
        "A2.AddTransporterEffect()".to_owned(),
        format!("A2.AddTransporterEffect({},1)", id.0),
        "A2.AddTransporterEffect(999999)".to_owned(),
        "A2.AddTransporterEffect('1')".to_owned(),
        "A2.RemoveBillboardEffect(2147483648)".to_owned(),
        "A2.RemoveBillboardEffect(1.5)".to_owned(),
        "A2.RemoveBillboardEffect('1')".to_owned(),
        "A2.GridVisible(1)".to_owned(),
        "A2.GridVisible()".to_owned(),
        "A2.RemoveFromAdLog(nil)".to_owned(),
        "A2.Starfield_Load_New_Background_Geometry('a')".to_owned(),
        "A2.Starfield_Load_New_Background_Geometry('a', 1)".to_owned(),
        "A2.Starfield_Load_New_Background_Geometry(string.char(0), 'b')".to_owned(),
        "A2.MinimapMessage(1,2)".to_owned(),
        "A2.MinimapMessage(1,2,3,4,5)".to_owned(),
        "A2.MinimapMessage(1.5,2,3)".to_owned(),
        "A2.MinimapMessage(1,2,1/0)".to_owned(),
        "A2.MinimapMessage('t',2,3,'x')".to_owned(),
        "A2.MinimapMessage(1,2,3,4)".to_owned(),
    ];
    for script in bad {
        assert!(rt.eval::<()>(&script).is_err(), "{script}");
        assert!(rt.take_commands().is_empty(), "partial command from {script}");
    }
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}
