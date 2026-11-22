//! Installed STA2 Classic regression for freighter dropoff station selection.
//!
//! Set `STA64_TEST_DROPOFF_INSTALL` to an Armada II install. The active mod
//! defaults to `STA2 Classic`; set `STA64_TEST_DROPOFF_MOD` to another mod,
//! or to an empty value to mount the base install only. Unset install skips.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MapObject, OdfIndex, UnitDef};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{MinerPhase, Sim, SimEvent, WorldSource};
use sta_vfs::Vfs;

fn n(value: i32) -> Fixed {
    Fixed::from_int(value)
}

fn world_dock(def: &UnitDef, origin: Vec3, heading: (Fixed, Fixed)) -> Vec3 {
    let dock = def
        .dock
        .as_ref()
        .expect("installed station has a typed dock");
    let (x, z) = (
        dock.offset.0 * heading.1 + dock.offset.1 * heading.0,
        dock.offset.1 * heading.1 - dock.offset.0 * heading.0,
    );
    Vec3 {
        x: origin.x + x,
        y: origin.y + dock.height,
        z: origin.z + z,
    }
}

fn unit_id(sim: &Sim, odf: &str) -> sta_core::EntityId {
    let key = sim.content().unwrap().key(odf).expect("installed ODF");
    sim.view()
        .units
        .iter()
        .find(|unit| unit.def == key)
        .unwrap_or_else(|| panic!("map placed {odf}"))
        .id
}

fn command(order: Order) -> Command {
    Command {
        player: PlayerId(0),
        seq: 1,
        kind: CommandKind::Order(order),
    }
}

#[test]
fn classic_freighter_unloads_at_the_authored_fbase_dock() {
    let Ok(root) = std::env::var("STA64_TEST_DROPOFF_INSTALL") else {
        eprintln!("STA64_TEST_DROPOFF_INSTALL unset: installed freighter regression skipped");
        return;
    };
    let active_mod =
        std::env::var("STA64_TEST_DROPOFF_MOD").unwrap_or_else(|_| "STA2 Classic".into());
    let active_mod = (!active_mod.is_empty()).then_some(active_mod);
    let vfs = Vfs::mount_install(Path::new(&root), active_mod.as_deref())
        .unwrap_or_else(|e| panic!("STA64_TEST_DROPOFF_INSTALL {root:?}: {e}"));
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).expect("load installed game data");
    let base = db.unit(db.key("fbase").expect("Federation base")).unwrap();
    let dock = base.dock.as_ref().expect("fbase authored docking point");
    assert_eq!(
        dock.node.as_deref().map(str::to_ascii_lowercase).as_deref(),
        Some("hp31")
    );
    let heading = (Fixed::ZERO, Fixed::ONE);
    let freighter_pos = Vec3 {
        x: n(100),
        y: Fixed::ZERO,
        z: Fixed::ZERO,
    };
    let expected_dock = Vec3 {
        x: n(500),
        y: dock.height,
        z: Fixed::ZERO,
    };
    let base_pos = Vec3 {
        x: expected_dock.x - dock.offset.0,
        y: Fixed::ZERO,
        z: expected_dock.z - dock.offset.1,
    };
    assert_eq!(world_dock(base, base_pos, heading), expected_dock);

    let map = MapDef {
        objects: vec![
            MapObject {
                odf: "latinum".into(),
                owner: None,
                pos: Vec3::ZERO,
                heading,
                label: None,
            },
            MapObject {
                odf: "ffreight".into(),
                owner: Some(0),
                pos: freighter_pos,
                heading,
                label: None,
            },
            // Closer to the starting freighter, but does not receive Latinum.
            MapObject {
                odf: "fyard".into(),
                owner: Some(0),
                pos: Vec3 {
                    x: n(120),
                    y: Fixed::ZERO,
                    z: n(20),
                },
                heading,
                label: None,
            },
            // Also a closer non-receiver for Latinum.
            MapObject {
                odf: "fmining".into(),
                owner: Some(0),
                pos: Vec3 {
                    x: n(140),
                    y: Fixed::ZERO,
                    z: n(-20),
                },
                heading,
                label: None,
            },
            MapObject {
                odf: "fbase".into(),
                owner: Some(0),
                pos: base_pos,
                heading,
                label: None,
            },
        ],
        ..MapDef::empty()
    };
    let seat = |id| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "federation".into(),
        control: Control::Human,
    };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 0x000D_20FF,
        map: Arc::new(map),
        content: Arc::new(db),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let freighter = unit_id(&sim, "ffreight");
    let node = unit_id(&sim, "latinum");
    let order = command(Order::Mine {
        units: vec![freighter],
        node,
    });
    assert!(
        !sim.step(&[order])
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. })),
        "the normal Mine order should be accepted"
    );

    let mut unloading_dock = None;
    let mut delivered_latinum = false;
    for _ in 0..20_000 {
        for event in sim.step(&[]).events {
            match event {
                SimEvent::MinerPhase {
                    id,
                    phase: MinerPhase::Unloading,
                    dock: Some(at),
                    ..
                } if id == freighter => unloading_dock = Some(at),
                SimEvent::Delivered {
                    player: PlayerId(0),
                    resource: 1,
                    amount,
                } if amount > Fixed::ZERO => {
                    delivered_latinum = true;
                }
                _ => {}
            }
        }
        if delivered_latinum {
            break;
        }
    }
    assert_eq!(
        unloading_dock,
        Some(expected_dock),
        "unloading must use fbase hp31's authored world pose"
    );
    assert!(
        delivered_latinum,
        "the trip must deliver Latinum to the Federation stock"
    );
    assert!(sim.view().player(PlayerId(0)).unwrap().resources[1] > Fixed::ZERO);
}
