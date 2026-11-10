//! Private validation of an external clean-room port. No mission/game bytes
//! are bundled. This verifies the intro bridge, not native mission parity.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MapObject, UnitDef};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_mission::api::{MissionConfig, ObjectiveState};
use sta_mission::runtime::MissionRuntime;
use sta_sim::present::Presentation;
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{EntityId, Sim, SimEvent, WorldSource};

#[test]
#[ignore = "requires STA64_A2_PORTS pointing to the external armada_missions a2/missions folder"]
fn into_the_breach_intro_reaches_gameplay_on_a_minimal_fake_world() {
    run_port(false);
}

#[test]
#[ignore = "requires STA64_A2_PORTS and STA64_A2_OBJECTIVES external folders"]
fn into_the_breach_loads_goals_and_fires_the_first_goal_trigger() {
    run_port(true);
}

fn portal_region(source: &str) -> Vec3 {
    // This literal belongs to the external port, not a shipped map fixture.
    let values = source
        .lines()
        .find_map(|line| line.trim().strip_prefix("local PORTAL_POSITION = Vector("))
        .and_then(|line| line.trim_end().strip_suffix(')'))
        .expect("port's literal portal region");
    let v: Vec<f64> = values.split(',').map(|n| n.trim().parse().unwrap()).collect();
    assert_eq!(v.len(), 3);
    Vec3 {
        x: Fixed::from_f64(v[0]),
        y: Fixed::from_f64(v[1]),
        z: Fixed::from_f64(v[2]),
    }
}

fn run_port(with_goals: bool) {
    let root = std::path::PathBuf::from(std::env::var("STA64_A2_PORTS").expect("set STA64_A2_PORTS"));
    let source = std::fs::read_to_string(root.join("Into the Breach.lua")).unwrap();
    let region = portal_region(&source);
    let objective_root = with_goals
        .then(|| std::path::PathBuf::from(std::env::var("STA64_A2_OBJECTIVES").expect("set STA64_A2_OBJECTIVES")));
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
            pos: Vec3 {
                x: region.x + Fixed::from_int((i as i32 - 1) * 20_000),
                y: region.y,
                z: region.z,
            },
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
    let mut rt = MissionRuntime::load_with("missions/Into the Breach.lua", 1, cfg, move |path| {
        if path == "missions/Into the Breach.lua" {
            return Some(observed.as_bytes().to_vec());
        }
        let name = path.strip_prefix("objectives/")?;
        objective_root
            .as_ref()
            .and_then(|root| std::fs::read(root.join(name)).ok())
    });
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
    if with_goals {
        assert_eq!(rt.objectives().len(), 4);
        assert!(rt.objective_briefing().is_some());
        assert!(!rt.eval::<bool>("return __port_instance.gateFound or false").unwrap());
        // Place the synthetic Enterprise in the port's authored discovery
        // region. This is a fixture edit, not native movement or map proof.
        sim.units.get_mut(EntityId(1)).unwrap().pos = region;
        let mut commands = Vec::new();
        rt.think(&sim.view(), &events, &mut commands);
        let found = sim.step(&commands);
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        assert!(rt.eval::<bool>("return __port_instance.gateFound").unwrap());
        assert_eq!(rt.objectives()[0].state, ObjectiveState::Complete);
        assert!(
            found.events.iter().any(
                |e| matches!(e, SimEvent::Present(Presentation::Movie { name, .. }) if name == "a2_fed05_gate.bik")
            )
        );
        assert!(rt.unsupported().iter().all(|(name, _)| name == "Player.loadAIP"));
        eprintln!(
            "external A2 first goal: 4 file goals loaded; discovery trigger completes goal 1 on synthetic actors"
        );
    }
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

#[test]
#[ignore = "requires STA64_A2_OBJECTIVES external folder; reads assets without copying their text"]
fn installed_objective_text_corpus_loads_without_errors() {
    let root = Arc::new(std::path::PathBuf::from(
        std::env::var("STA64_A2_OBJECTIVES").expect("set STA64_A2_OBJECTIVES"),
    ));
    let (mut files, mut goals) = (0, 0);
    for entry in std::fs::read_dir(&*root).unwrap() {
        let entry = entry.unwrap();
        if !entry.file_type().unwrap().is_file()
            || !entry.path().extension().is_some_and(|e| e.eq_ignore_ascii_case("txt"))
        {
            continue;
        }
        let name = entry.file_name().into_string().unwrap();
        let r = root.clone();
        let rt = MissionRuntime::load("missions/T.lua", 1, move |path| {
            if path == "missions/T.lua" {
                return Some(Vec::new());
            }
            path.strip_prefix("objectives/")
                .and_then(|p| std::fs::read(r.join(p)).ok())
        });
        let load: mlua::Function = rt.eval("return function(name) Objectives:load(name) end").unwrap();
        load.call::<()>(name.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(rt.errors().is_empty(), "{name}: {:?}", rt.errors());
        assert!(rt.unsupported().is_empty(), "{name}: {:?}", rt.unsupported());
        assert!(rt.objective_briefing().is_some());
        files += 1;
        goals += rt.objectives().len();
    }
    assert!(files > 0);
    eprintln!("external objectives corpus: {files} files loaded; {goals} goals; zero errors");
}
