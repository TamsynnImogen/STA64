//! Run against an installed Armada II data tree with STA64_TEST_RESEARCH_INSTALL.
//! STA64_TEST_RESEARCH_MOD optionally mounts a mod, such as STA2 Classic.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MapObject, OdfIndex};
use sta_core::{ContentKey, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::production::view::ResearchBlock;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

#[test]
fn federation_research_uses_the_installed_socket_and_tech_tree() {
    let Ok(root) = std::env::var("STA64_TEST_RESEARCH_INSTALL") else {
        eprintln!("STA64_TEST_RESEARCH_INSTALL unset, installed research test skipped");
        return;
    };
    let active = std::env::var("STA64_TEST_RESEARCH_MOD").ok();
    let vfs = sta_vfs::Vfs::mount_install(Path::new(&root), active.as_deref())
        .expect("installed Armada II");
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).expect("installed game data");
    let station = db.unit(db.key("fresear").unwrap()).unwrap();
    let socket = &station.research_slots[0];
    assert_eq!(socket.node.to_ascii_lowercase(), "hp01");
    let hardpoint = socket.pose.expect("hp01 resolves in the mounted SOD");
    assert!(
        hardpoint.origin.y > Fixed::ZERO,
        "the stock socket lies above the station origin"
    );
    let scale = if station.scale > Fixed::ZERO {
        station.scale
    } else {
        Fixed::ONE
    };
    let expected = Vec3 {
        x: hardpoint.origin.x * scale,
        y: hardpoint.origin.y * scale,
        z: hardpoint.origin.z * scale,
    };
    let pod = db
        .key("fedpod5")
        .expect("Federation Tachyon Detection Grid pod");
    let ticks = db.unit(pod).unwrap().build.ticks;
    assert!(db.tech_tree().is_some());
    let seat = |n| PlayerSetup {
        id: PlayerId(n),
        team: TeamId(n),
        race: "federation".into(),
        control: Control::Human,
    };
    let map = MapDef {
        objects: vec![MapObject {
            odf: "fresear".into(),
            owner: Some(0),
            pos: Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
            label: None,
        }],
        ..MapDef::empty()
    };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 42,
        map: Arc::new(map),
        content: Arc::new(db),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let station = sim
        .view()
        .units
        .iter()
        .find(|u| u.owner == PlayerId(0))
        .unwrap()
        .id;
    let funding: Vec<_> = (0..5)
        .map(|slot| Command {
            player: PlayerId::BENCH,
            seq: slot,
            kind: CommandKind::Edit(Edit::Cargo(
                PlayerId(0),
                slot as u8,
                Fixed::from_int(100_000),
            )),
        })
        .collect();
    sim.step(&funding);
    let build = |seq, what: ContentKey| Command {
        player: PlayerId(0),
        seq,
        kind: CommandKind::Order(Order::Build {
            builder: station,
            what,
            at: None,
        }),
    };
    let first = sim.step(&[build(1, pod)]);
    assert!(
        first
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::BuildStarted { what, .. } if *what == pod)),
        "{:?}",
        first.events
    );
    assert!(
        first.view.units.iter().all(|unit| unit.def != pod),
        "research in progress does not grant a pod"
    );
    for _ in 0..=ticks {
        sim.step(&[]);
    }
    let rows = sim.production_view();
    let research = rows
        .iter()
        .find(|row| row.builder == station)
        .unwrap()
        .research
        .as_ref()
        .unwrap();
    assert_eq!(research.pods.len(), 1);
    let mounted = research.pods[0];
    assert_eq!(mounted.slot, 0);
    assert_eq!(sim.unit(mounted.entity).unwrap().pos, expected);
    assert_eq!(mounted.axes, hardpoint.axes);
    assert!(
        research
            .unavailable
            .contains(&(pod, ResearchBlock::AlreadyResearched))
    );
    let repeat = sim.step(&[build(2, pod)]);
    assert!(
        repeat
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    sim.despawn(mounted.entity, None);
    let retry = sim.step(&[build(3, pod)]);
    assert!(
        retry
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::BuildStarted { what, .. } if *what == pod))
    );
}
