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

#[test]
fn upgrade_research_replaces_the_matching_system_and_retains_its_prerequisites() {
    use sta_content::{DefRef, ResearchBuildSlot, SystemUpgrade, UpgradeSystem};
    use sta_core::ContentKey;

    // Content keys follow sorted definition names. Two distinct hardpoints
    // make it observable that the first built system need not be button zero.
    let mut station = UnitDef::new("station", "research");
    station.hull = n(100);
    station.build.list = vec![DefRef::new("engines2"), DefRef::new("shield2")];
    station.research_build = vec![
        ResearchBuildSlot { index: 0, primary: Some(ContentKey(0)), successors: vec![ContentKey(1), ContentKey(2)] },
        ResearchBuildSlot { index: 4, primary: Some(ContentKey(3)), successors: vec![ContentKey(4)] },
    ];
    station.research_slots = [10, 20].into_iter().map(|x| ResearchSlot {
        node: format!("slot_{x}"),
        reserved_for: None,
        pose: Some(RepairAttachment {
            origin: at(x, 2, 0),
            axes: [at(1, 0, 0), at(0, 1, 0), at(0, 0, 1)],
        }),
    }).collect();
    let upgrade = |name, system, tier, multiplier| {
        let mut pod = UnitDef::new(name, "pod");
        pod.hull = n(40);
        pod.build.ticks = 2;
        pod.system_upgrade = Some(SystemUpgrade { system, tier, multiplier });
        pod
    };
    let mut ship = UnitDef::new("ship", "craft");
    ship.hull = n(100);
    ship.shields.max = n(80);
    ship.movement.max_speed = n(100);
    let (tree, warnings) = sta_content::TechTree::parse("tech1.tt",
        "engines2.odf 1 station.odf\nengines3.odf 1 engines2.odf\nengines4.odf 2 engines2.odf engines3.odf\nshield2.odf 1 station.odf\nshield3.odf 1 shield2.odf\n");
    assert!(warnings.is_empty(), "{warnings:?}");
    let content = Arc::new(ContentDb::builder().profile(Profile::Sta2Classic)
        .unit(station)
        .unit(ship)
        .unit(upgrade("engines2", UpgradeSystem::Engines, 2, Fixed::from_f64(1.25)))
        .unit(upgrade("engines3", UpgradeSystem::Engines, 3, Fixed::from_f64(1.5)))
        .unit(upgrade("engines4", UpgradeSystem::Engines, 4, n(2)))
        .unit(upgrade("shield2", UpgradeSystem::Shields, 2, Fixed::from_f64(1.5)))
        .unit(upgrade("shield3", UpgradeSystem::Shields, 3, n(2)))
        .tech_tree(tree).build().unwrap());
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 21,
        map: Arc::new(MapDef::empty()),
        content: content.clone(),
        players: vec![PlayerSetup { id: PlayerId(0), team: TeamId(0), race: "test".into(), control: Control::Human }],
        options: GameOptions::default(),
    });
    let spawn = |sim: &mut Sim, name, pos| sim.spawn_def(key(sim, name), PlayerId(0), pos, (Fixed::ZERO, Fixed::ONE)).unwrap();
    let station = spawn(&mut sim, "station", Vec3::ZERO);
    let ship = spawn(&mut sim, "ship", at(200, 0, 0));

    assert!(!build(&mut sim, station, "engines4", 1));
    let finish = |sim: &mut Sim, what, seq| {
        assert!(build(sim, station, what, seq), "could not queue {what}");
        for _ in 0..3 { sim.step(&[]); }
    };
    finish(&mut sim, "shield2", 2);
    finish(&mut sim, "engines2", 3);
    let old = research(&sim, station).pods[1];
    assert_eq!(sim.unit(ship).unwrap().stats.max_speed, n(125));
    assert_eq!(research(&sim, station).build_slots, Some(vec![(0, key(&sim, "engines3")), (4, key(&sim, "shield3"))]));

    // The old pod keeps granting its benefits during construction and
    // reappears on cancellation without taking a different hardpoint.
    assert!(build(&mut sim, station, "engines3", 4));
    assert_eq!(research(&sim, station).replacing, Some(old.entity));
    assert_eq!(sim.unit(ship).unwrap().stats.max_speed, n(125));
    let row = sim.production_view().into_iter().find(|r| r.builder == station).unwrap();
    sim.step(&[Command {
        player: PlayerId(0), seq: 5,
        kind: CommandKind::Order(Order::CancelBuild { builder: station, slot: 0, job: row.jobs[0].queue_id.unwrap() }),
    }]);
    assert_eq!(research(&sim, station).replacing, None);
    assert_eq!(research(&sim, station).pods[1], old);
    finish(&mut sim, "engines3", 6);
    assert!(sim.unit(old.entity).is_none());
    assert_eq!(research(&sim, station).pods[1].slot, old.slot);
    assert_eq!(research(&sim, station).build_slots.unwrap()[0], (0, key(&sim, "engines4")));
    finish(&mut sim, "engines4", 7);
    finish(&mut sim, "shield3", 8);
    assert_eq!(sim.unit(ship).unwrap().stats.max_speed, n(200));
    assert_eq!(sim.unit(ship).unwrap().shields.max, n(160));
    assert_eq!(research(&sim, station).unavailable,
        [(key(&sim, "engines4"), ResearchBlock::AlreadyResearched),
         (key(&sim, "shield3"), ResearchBlock::AlreadyResearched)]);

    // An independent live pod for the same team is the fallback.
    let lower = spawn(&mut sim, "engines2", at(270, 0, 0));
    let restored = Sim::from_snapshot(&sim.snapshot(), Some(content)).unwrap();
    assert_eq!(sim.digest(), restored.digest());
    let mut sim = restored;
    let upgraded = research(&sim, station).pods[1].entity;
    sim.despawn(upgraded, None);
    assert_eq!(sim.unit(ship).unwrap().stats.max_speed, n(125));
    sim.despawn(lower, None);
    assert_eq!(sim.unit(ship).unwrap().stats.max_speed, n(100));

    // Map/script placements are visible and reflected in a snapshot before
    // the next tick, just like pods produced by the research queue.
    spawn(&mut sim, "engines2", at(270, 0, 0));
    assert_eq!(sim.unit(ship).unwrap().stats.max_speed, n(125));
    let restored = Sim::from_snapshot(&sim.snapshot(), sim.content().cloned()).unwrap();
    assert_eq!(restored.unit(ship).unwrap().stats.max_speed, n(125));
    assert_eq!(restored.digest(), sim.digest());
}
