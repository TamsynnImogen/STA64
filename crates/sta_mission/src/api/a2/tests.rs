//! An asset-free intro using observed conversion call shapes, through a real
//! controller and sim. This is not a stock mission or a native parity test.

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;
use sta_core::{Fixed, PlayerId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::present::{Presentation, Spot};
use sta_sim::setup::Controller;
use sta_sim::{MoveStats, Sim, SimEvent, WorldSource};

const INTRO: &str = r#"
    local hero = EntityFinder():findOne('hero')
    Cineractive.active = true
    Cineractive.skippable = true
    Cineractive:fadeIn(3.0)
    Cineractive:lookFrom(hero)
    Cineractive:setCameraOffset(Vector(-350, 150, -500))
    Cineractive:lookAt(hero)
    UI:addTitle('Test mission', 50, 50, 7.0, true)
    UI:addSubtitle('Test sector', 50, 85, 7.0, true)
    Media:playMusic('test-music.ogg')
    Monitor.timer(0.1, function() Media:playSound('test-voice.ogg') end)
    Monitor.timer(1.0, function()
        Media:stopSounds()
        Cineractive.active = false
        Objectives.objectives:add('Protect the hero')
        Objectives.visible = true
    end)
"#;

fn run_intro() -> (Vec<Vec<Command>>, Vec<Vec<SimEvent>>) {
    let mut sim = Sim::new(1);
    let hero = sim.spawn(
        0,
        Vec3::ZERO,
        (Fixed::ZERO, Fixed::ONE),
        MoveStats::from_f32(10., 1., 1., 1., 0.2),
    );
    let mut control = sim.clone();
    let cfg = MissionConfig {
        labels: [(hero, "hero".into())].into_iter().collect(),
        view: Some((*sim.view()).clone()),
        ..MissionConfig::default()
    };
    let mut rt = MissionRuntime::from_source_with("intro.lua", INTRO, 1, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let (mut batches, mut timeline) = (Vec::new(), Vec::new());
    let mut events = Vec::new();
    for _ in 0..40 {
        let mut commands = Vec::new();
        rt.think(&sim.view(), &events, &mut commands);
        let expected: Vec<_> = commands
            .iter()
            .map(|c| {
                assert_eq!(c.player, PlayerId::SCRIPT);
                match &c.kind {
                    CommandKind::Edit(Edit::Present(p)) => SimEvent::Present(p.clone()),
                    other => panic!("unexpected intro command {other:?}"),
                }
            })
            .collect();
        events = sim.step(&commands).events;
        let presented: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, SimEvent::Present(_)))
            .cloned()
            .collect();
        assert_eq!(presented, expected);
        let world_events: Vec<_> = events
            .iter()
            .filter(|e| !matches!(e, SimEvent::Present(_)))
            .cloned()
            .collect();
        assert_eq!(world_events, control.step(&[]).events);
        assert_eq!(sim.digest(), control.digest(), "presentation changed world state");
        timeline.push(events.clone());
        batches.push(commands);
    }
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
    assert_eq!(rt.objectives()[0].title, "Protect the hero");
    let flattened: Vec<_> = timeline.iter().flatten().collect();
    assert!(flattened.iter().any(|e| matches!(e, SimEvent::Present(Presentation::Shot { from: Spot::Entity(a), at: Spot::Entity(b), .. }) if *a == hero && *b == hero)));
    assert!(
        flattened
            .iter()
            .any(|e| matches!(e, SimEvent::Present(Presentation::Sound { id: 1, .. })))
    );
    assert!(
        flattened
            .iter()
            .any(|e| matches!(e, SimEvent::Present(Presentation::StopSound { id: 1 })))
    );
    (batches, timeline)
}

#[test]
fn converted_intro_call_shapes_reach_the_sim_and_first_objective_repeatably() {
    let (commands, events) = run_intro();
    assert_eq!(run_intro(), (commands.clone(), events.clone()));
    // Feeding the recorded commands alone reproduces every event; the Lua
    // controller need not run on playback (D3).
    let mut replay = Sim::new(1);
    replay.spawn(
        0,
        Vec3::ZERO,
        (Fixed::ZERO, Fixed::ONE),
        MoveStats::from_f32(10., 1., 1., 1., 0.2),
    );
    for (batch, expected) in commands.iter().zip(events) {
        assert_eq!(replay.step(batch).events, expected);
    }
}

#[test]
fn unknown_presentation_methods_remain_stubbed_and_are_reported() {
    let rt = MissionRuntime::from_source(
        "T.lua",
        r#"
        Cineractive:unimplementedShot()
        Media:unimplementedAudio()
        UI:unimplementedText()
    "#,
        1,
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(
        rt.unsupported(),
        [
            ("Cineractive.unimplementedShot".into(), 1),
            ("Media.unimplementedAudio".into(), 1),
            ("UI.unimplementedText".into(), 1),
        ]
    );
}
