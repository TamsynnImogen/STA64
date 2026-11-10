//! Private validation of an external clean-room port. No mission/game bytes
//! are bundled. This verifies the intro bridge, not native mission parity.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MapObject, UnitDef};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_mission::api::MissionConfig;
use sta_mission::runtime::MissionRuntime;
use sta_sim::present::Presentation;
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{EntityId, Sim, SimEvent, WorldSource};

#[test]
#[ignore = "requires STA64_A2_PORTS pointing to the external armada_missions a2/missions folder"]
fn into_the_breach_intro_reaches_gameplay_on_a_minimal_fake_world() {
    let root = std::path::PathBuf::from(std::env::var("STA64_A2_PORTS").expect("set STA64_A2_PORTS"));
    let source = std::fs::read_to_string(root.join("Into the Breach.lua")).unwrap();
    let mut b = ContentDb::builder();
    for odf in ["fente", "bportal", "fbase"] {
        // Intentionally synthetic craft: no native class hierarchy, assets,
        // weapons, mission-map layout or physics are being validated here.
        let mut u = UnitDef::new(odf, "craft");
        u.hull = Fixed::from_int(100);
        u.crew = Fixed::from_int(10);
        b = b.unit(u);
    }
    let db = Arc::new(b.build().unwrap());
    let mut map = MapDef::empty();
    map.objects = ["fente", "bportal", "fbase"]
        .into_iter()
        .enumerate()
        .map(|(i, odf)| MapObject {
            odf: odf.into(),
            owner: Some(if i == 1 { 4 } else { 1 }),
            pos: Vec3::from_f32([(i as f32 - 1.0) * 20_000.0, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
            label: None,
        })
        .collect();
    let setup = GameSetup {
        seed: 1,
        map: Arc::new(map),
        content: db.clone(),
        players: (0..5)
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "test".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let cfg = MissionConfig {
        seat: Some(PlayerId(1)),
        content: Some(db),
        view: Some((*sim.view()).clone()),
        ..MissionConfig::default()
    };
    // Observe registration without changing the source file or its methods.
    let observed = format!(
        "local register=MMM.register\nMMM.register=function(o) __port_instance=o; return register(o) end\n{source}"
    );
    let mut rt = MissionRuntime::from_source_with("Into the Breach.lua", &observed, 1, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let (mut events, mut timeline) = (Vec::new(), Vec::new());
    for _ in 0..901 {
        let tick = sim.tick;
        let mut commands = Vec::new();
        rt.think(&sim.view(), &events, &mut commands);
        events = sim.step(&commands).events;
        assert!(
            !events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })),
            "{tick}: {events:?}"
        );
        timeline.extend(events.iter().cloned().map(|e| (tick, e)));
    }
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(rt.eval::<String>("return __port_instance.phase").unwrap(), "gameplay");
    assert_eq!(
        rt.eval::<u32>("return __port_instance.enterprise.id").unwrap(),
        EntityId(1).0
    );
    assert_eq!(
        rt.eval::<u32>("return __port_instance.portal.id").unwrap(),
        EntityId(2).0
    );
    assert_eq!(rt.eval::<u32>("return __port_instance.base.id").unwrap(), EntityId(3).0);
    assert!(timeline.iter().any(|(tick, e)| *tick == 60
        && matches!(e, SimEvent::Present(Presentation::Sound { name, .. }) if name == "a2_fed05_intro.wav")));
    assert!(
        timeline.iter().any(
            |(tick, e)| *tick == 720 && matches!(e, SimEvent::Present(Presentation::Cineractive { on: false, .. }))
        )
    );
    eprintln!(
        "external A2 intro: gameplay at 30 seconds; unsupported={:?}",
        rt.unsupported()
    );
    assert!(
        rt.unsupported()
            .iter()
            .all(|(name, _)| name.starts_with("Objectives.load") || name.starts_with("Player.loadAIP"))
    );
}
