use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MoveStats, Profile, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Edit, Order};
use crate::replay::{Recorder, Replay};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn v(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::from_int(y),
        z: Fixed::from_int(z),
    }
}

fn setup(profile: Profile, defs: Vec<UnitDef>) -> GameSetup {
    let mut builder = ContentDb::builder().profile(profile);
    for def in defs {
        builder = builder.unit(def);
    }
    let mut map = MapDef::empty();
    map.bounds = (v(-500, -10, -500), v(500, 10, 500));
    GameSetup {
        seed: 22,
        content: Arc::new(builder.build().unwrap()),
        map: Arc::new(map),
        players: vec![PlayerSetup {
            id: PlayerId(0),
            team: TeamId(0),
            race: "synthetic".into(),
            control: Control::Idle,
        }],
        options: GameOptions::default(),
    }
}

fn def(name: &str, class: &str) -> UnitDef {
    let mut def = UnitDef::new(name, class);
    def.hull = Fixed::from_int(100);
    def
}

fn spawn(sim: &mut Sim, name: &str, position: Vec3) -> EntityId {
    let key = sim.content().unwrap().key(name).unwrap();
    sim.spawn_def(key, PlayerId(0), position, (Fixed::ONE, Fixed::ZERO))
        .unwrap()
}

fn edit(seq: u32, value: Edit) -> Command {
    Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(value),
    }
}

fn order(seq: u32, value: Order) -> Command {
    Command {
        player: PlayerId(0),
        seq,
        kind: CommandKind::Order(value),
    }
}

fn upper() -> Fixed {
    Fixed::from_int(400) - Fixed::from_bits(429496736)
}

#[test]
fn engine_disabled_immobile_craft_still_projects_all_axes() {
    let mut sim = Sim::from_setup(&setup(Profile::Fo4, vec![def("station", "starbase")]));
    let id = spawn(&mut sim, "station", v(450, 100, -450));
    assert_eq!(sim.force_onto_map(id), Some(true));
    sim.step(&[edit(0, Edit::Engines(id, false))]);
    let entity = sim.unit(id).unwrap();
    assert!(entity.flags.contains(crate::view::UnitFlags::ENGINES_OFF));
    assert_eq!(
        entity.pos,
        Vec3 {
            x: upper(),
            y: Fixed::from_int(10),
            z: Fixed::from_int(-400)
        }
    );
}

#[test]
fn plain_a1_rearm_uses_actual_callbacks_without_retroactive_y_projection() {
    let mut sim = Sim::from_setup(&setup(
        Profile::Sta1Classic,
        vec![def("plain", "constplaceholder")],
    ));
    let id = spawn(&mut sim, "plain", v(450, 100, 0));
    assert_eq!(sim.force_onto_map(id), Some(false));
    // A real update has entered padded XZ, independently of height.
    sim.units.get_mut(id).unwrap().pos = v(0, 100, 0);
    sim.confinement
        .position_updated(sim.units.get_mut(id).unwrap());
    assert_eq!(sim.force_onto_map(id), Some(true));
    assert_eq!(sim.unit(id).unwrap().pos.y, Fixed::from_int(100));
    sim.step(&[]);
    assert_eq!(
        sim.unit(id).unwrap().pos.y,
        Fixed::from_int(100),
        "plain object has no unconditional tick"
    );
    sim.confinement
        .position_updated(sim.units.get_mut(id).unwrap());
    assert_eq!(sim.unit(id).unwrap().pos.y, Fixed::from_int(10));
}

#[test]
fn queued_targets_are_projected_before_the_first_leg_is_completed() {
    let mut craft = def("craft", "craft");
    craft.movement = MoveStats::from_f32(120.0, 2.0, 2.0, 2.0, 1.0);
    let mut sim = Sim::from_setup(&setup(Profile::Fo4, vec![craft]));
    let id = spawn(&mut sim, "craft", Vec3::ZERO);
    sim.step(&[
        order(
            0,
            Order::Move {
                units: vec![id],
                to: v(250, 0, 0),
                queue: false,
            },
        ),
        order(
            1,
            Order::Move {
                units: vec![id],
                to: v(467, 100, 0),
                queue: true,
            },
        ),
    ]);
    assert_eq!(sim.unit(id).unwrap().target, Some(v(250, 0, 0)));
    assert_eq!(
        sim.movement.course(id).unwrap().waypoints,
        [Vec3 {
            x: upper(),
            y: Fixed::from_int(10),
            z: Fixed::ZERO
        }]
    );
}

#[test]
fn late_mining_target_uses_miner_flag_without_altering_terrain_node() {
    let mut miner = def("miner", "freighter");
    miner.movement = MoveStats::from_f32(120.0, 2.0, 2.0, 2.0, 1.0);
    miner.miner = true;
    let mut node = def("node", "scrap");
    node.resource = Some(0);
    let mut sim = Sim::from_setup(&setup(Profile::Fo4, vec![miner, node]));
    let node = spawn(&mut sim, "node", v(467, 0, 0));
    let miner = spawn(&mut sim, "miner", Vec3::ZERO);
    let output = sim.step(&[order(
        0,
        Order::Mine {
            units: vec![miner],
            node,
        },
    )]);
    assert!(
        !output
            .events
            .iter()
            .any(|e| matches!(e, crate::SimEvent::Rejected { .. }))
    );
    assert_eq!(sim.unit(miner).unwrap().target.unwrap().x, upper());
    assert_eq!(sim.unit(node).unwrap().pos, v(467, 0, 0));
    sim.step(&[edit(
        0,
        Edit::ForceOntoMap {
            id: miner,
            enabled: false,
        },
    )]);
    assert_eq!(sim.unit(miner).unwrap().target, Some(v(467, 0, 0)));
}

#[test]
fn actual_force_commands_replay_with_star1_and_seam1() {
    let mut craft = def("craft", "craft");
    craft.movement = MoveStats::from_f32(120.0, 2.0, 2.0, 2.0, 1.0);
    let mut setup = setup(Profile::Fo4, vec![craft]);
    Arc::make_mut(&mut setup.map)
        .objects
        .push(sta_content::MapObject {
            odf: "craft".into(),
            owner: Some(0),
            pos: Vec3::ZERO,
            heading: (Fixed::ONE, Fixed::ZERO),
            label: Some("craft".into()),
        });
    let mut recording = Recorder::with_interval(&setup, 10);
    let id = EntityId(1);
    for tick in 0..180 {
        let commands = match tick {
            0 => vec![
                order(
                    0,
                    Order::Move {
                        units: vec![id],
                        to: v(467, 100, 0),
                        queue: false,
                    },
                ),
                edit(0, Edit::ForceOntoMap { id, enabled: true }),
            ],
            45 => vec![edit(1, Edit::ForceOntoMap { id, enabled: false })],
            46 => vec![order(
                1,
                Order::Move {
                    units: vec![id],
                    to: v(467, 100, 0),
                    queue: false,
                },
            )],
            90 => vec![edit(2, Edit::ForceOntoMap { id, enabled: true })],
            _ => vec![],
        };
        recording.step(&commands);
    }
    let expected = recording.sim().snapshot();
    let replay = recording.finish();
    assert_eq!(replay.seam_rev, 1);
    let bytes = replay.encode();
    assert_eq!(&bytes[..6], b"STAR\x01\0");
    let played = Replay::decode(&bytes).unwrap().play(setup.content).unwrap();
    assert_eq!(played.snapshot(), expected);
}
