//! Synthetic lifecycle coverage for station-mounted research pods.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MoveStats, Profile, RepairAttachment, ResearchSlot, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::production::view::ResearchBlock;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};

fn n(value: i32) -> Fixed {
    Fixed::from_int(value)
}
fn at(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3 {
        x: n(x),
        y: n(y),
        z: n(z),
    }
}

fn game() -> Sim {
    let mut station = UnitDef::new("station", "research");
    station.hull = n(100);
    station.movement = MoveStats::from_f32(0.0, 0.0, 0.0, 0.0, 0.0);
    station.build.list = ["pod_a", "pod_b"].map(sta_content::DefRef::new).to_vec();
    station.research_slots = vec![
        ResearchSlot {
            node: "slot_a".into(),
            reserved_for: None,
            pose: Some(RepairAttachment {
                origin: at(10, 2, 4),
                axes: [at(1, 0, 0), at(0, 1, 0), at(0, 0, 1)],
            }),
        },
        ResearchSlot {
            node: "slot_b".into(),
            reserved_for: None,
            pose: Some(RepairAttachment {
                origin: at(-6, 3, 8),
                axes: [at(1, 0, 0), at(0, 1, 0), at(0, 0, 1)],
            }),
        },
    ];
    let pod = |name: &str| {
        let mut def = UnitDef::new(name, "pod");
        def.hull = n(20);
        def.build.ticks = 2;
        def
    };
    let mut yard = UnitDef::new("yard", "shipyard");
    yard.hull = n(100);
    yard.build.list.push(sta_content::DefRef::new("reward"));
    let mut reward = UnitDef::new("reward", "craft");
    reward.hull = n(10);
    reward.build.ticks = 2;
    let (tree, warnings) = sta_content::TechTree::parse(
        "tech1.tt",
        "station.odf 0\nyard.odf 0\npod_a.odf 1 station.odf\npod_b.odf 1 station.odf\nreward.odf 1 pod_a.odf\n",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let content = Arc::new(
        ContentDb::builder()
            .profile(Profile::Sta2Classic)
            .unit(station)
            .unit(pod("pod_a"))
            .unit(pod("pod_b"))
            .unit(yard)
            .unit(reward)
            .tech_tree(tree)
            .build()
            .unwrap(),
    );
    let mut map = MapDef::empty();
    map.bounds = (at(-1000, -100, -1000), at(1000, 100, 1000));
    Sim::from_setup(&GameSetup {
        seed: 91,
        map: Arc::new(map),
        content,
        players: vec![PlayerSetup {
            id: PlayerId(0),
            team: TeamId(0),
            race: "synthetic".into(),
            control: Control::Human,
        }],
        options: GameOptions::default(),
    })
}

fn key(sim: &Sim, name: &str) -> sta_core::ContentKey {
    sim.content().unwrap().key(name).unwrap()
}
fn build(sim: &mut Sim, builder: EntityId, product: &str, seq: u32) -> bool {
    let what = key(sim, product);
    let command = Command {
        player: PlayerId(0),
        seq,
        kind: CommandKind::Order(Order::Build {
            builder,
            what,
            at: None,
        }),
    };
    !sim.step(&[command])
        .events
        .iter()
        .any(|event| matches!(event, sta_sim::SimEvent::Rejected { .. }))
}
fn research(sim: &Sim, station: EntityId) -> sta_sim::production::view::ResearchView {
    sim.production_view()
        .into_iter()
        .find(|row| row.builder == station)
        .unwrap()
        .research
        .unwrap()
}

#[test]
fn pods_use_authored_slots_and_survive_snapshot_and_despawn_lifecycle() {
    let mut sim = game();
    let station_def = key(&sim, "station");
    let station = sim
        .spawn_def(
            station_def,
            PlayerId(0),
            at(100, 7, 200),
            (Fixed::ONE, Fixed::ZERO),
        )
        .unwrap();
    assert!(build(&mut sim, station, "pod_a", 1));
    assert!(
        !build(&mut sim, station, "pod_a", 2),
        "duplicate is rejected while queued"
    );
    for _ in 0..3 {
        sim.step(&[]);
    }
    let first = research(&sim, station);
    assert_eq!(first.pods.len(), 1);
    assert_eq!(first.pods[0].slot, 0);
    assert_eq!(
        first.unavailable,
        vec![(key(&sim, "pod_a"), ResearchBlock::AlreadyResearched)]
    );
    assert_eq!(first.pods[0].axes[0], at(0, 0, -1));
    let pod_a = first.pods[0].entity;
    let pod = sim.unit(pod_a).unwrap();
    assert_eq!(pod.pos, at(104, 9, 190));
    assert_eq!(pod.heading, (Fixed::ONE, Fixed::ZERO));
    assert!(
        !build(&mut sim, station, "pod_a", 3),
        "duplicate is rejected after completion"
    );

    assert!(build(&mut sim, station, "pod_b", 4));
    for _ in 0..3 {
        sim.step(&[]);
    }
    assert_eq!(
        research(&sim, station)
            .pods
            .iter()
            .map(|pod| pod.slot)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    let pod_b = research(&sim, station).pods[1].entity;
    assert!(sim.despawn(pod_a, None));
    assert!(build(&mut sim, station, "pod_a", 5));
    for _ in 0..3 {
        sim.step(&[]);
    }
    assert_eq!(
        research(&sim, station)
            .pods
            .iter()
            .map(|pod| pod.slot)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    let replacement = research(&sim, station)
        .pods
        .iter()
        .find(|pod| pod.entity != pod_b)
        .copied()
        .unwrap();
    assert_eq!(replacement.slot, 0, "first freed authored slot is reused");

    let db = sim.content().unwrap().clone();
    let bytes = sim.snapshot();
    let restored = Sim::from_snapshot(&bytes, Some(db)).unwrap();
    assert_eq!(research(&restored, station), research(&sim, station));
    assert_eq!(restored.snapshot(), bytes);
    assert!(sim.despawn(station, None));
    assert!(sim.unit(pod_b).is_none());
    assert!(sim.unit(replacement.entity).is_none());
}

#[test]
fn only_a_finished_pod_satisfies_a_tech_prerequisite() {
    let mut sim = game();
    let heading = (Fixed::ZERO, Fixed::ONE);
    let station = sim
        .spawn_def(key(&sim, "station"), PlayerId(0), at(0, 0, 0), heading)
        .unwrap();
    let yard = sim
        .spawn_def(key(&sim, "yard"), PlayerId(0), at(500, 0, 0), heading)
        .unwrap();
    assert!(build(&mut sim, station, "pod_a", 1));
    assert!(!build(&mut sim, yard, "reward", 2));
    sim.step(&[]);
    assert!(build(&mut sim, yard, "reward", 3));
    let pod = research(&sim, station).pods[0].entity;
    assert!(sim.despawn(pod, None));
    assert!(!build(&mut sim, yard, "reward", 4));
}
