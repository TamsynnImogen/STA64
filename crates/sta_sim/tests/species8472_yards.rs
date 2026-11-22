//! Install-gated Species 8472 mobile-yard and Fluidic Gate regressions.
//!
//! Set `STA64_TEST_8472_YARDS_INSTALL` to an Armada II install. The active mod
//! defaults to `STA2 Classic`; the install's tech tree is omitted from this
//! fixture so the authored yard build lists can be exercised directly.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MapObject, OdfIndex};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::world::EntityKind;
use sta_sim::{Sim, SimEvent, WorldSource};
use sta_vfs::Vfs;

fn n(value: i32) -> Fixed {
    Fixed::from_int(value)
}

fn installed_db() -> Option<Arc<ContentDb>> {
    let root = std::env::var("STA64_TEST_8472_YARDS_INSTALL").ok()?;
    let active_mod = std::env::var("STA64_TEST_A2_MOD").unwrap_or_else(|_| "STA2 Classic".into());
    let active_mod = (!active_mod.is_empty()).then_some(active_mod);
    let vfs = Vfs::mount_install(Path::new(&root), active_mod.as_deref())
        .unwrap_or_else(|e| panic!("STA64_TEST_8472_YARDS_INSTALL {root:?}: {e}"));
    let (index, _) = OdfIndex::build(&vfs);
    let (loaded, _) = ContentDb::load(&vfs, &index).expect("load installed ODF content");

    // Preserve the installed ODFs, SOD launch points, profile and resource
    // slots, while leaving `tech_tree()` empty for this production fixture.
    let mut builder = ContentDb::builder()
        .profile(loaded.profile())
        .features(loaded.features().clone())
        .resources(loaded.resources().clone());
    for weapon in loaded.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    for unit in loaded.units() {
        builder = builder.unit(unit.clone());
    }
    let db = builder
        .build()
        .expect("rebuild installed definitions without tech tree");
    Some(Arc::new(db))
}

fn setup(db: Arc<ContentDb>, objects: &[(&str, Vec3)]) -> GameSetup {
    let map = MapDef {
        objects: objects
            .iter()
            .map(|(odf, pos)| MapObject {
                odf: (*odf).into(),
                owner: Some(0),
                pos: *pos,
                heading: (Fixed::ZERO, Fixed::ONE),
                label: None,
            })
            .collect(),
        ..MapDef::empty()
    };
    let seat = |id| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "species8472".into(),
        control: Control::Human,
    };
    GameSetup {
        seed: 8472,
        map: Arc::new(map),
        content: db,
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
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

fn command(seq: u32, order: Order) -> Command {
    Command {
        player: PlayerId(0),
        seq,
        kind: CommandKind::Order(order),
    }
}

fn fund_biomatter(sim: &mut Sim) {
    sim.step(&[Command {
        player: PlayerId::BENCH,
        seq: 0,
        kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 3, n(100_000))),
    }]);
}

#[test]
fn installed_8472_mothership_remains_a_mobile_producing_station() {
    let Some(db) = installed_db() else {
        eprintln!("STA64_TEST_8472_YARDS_INSTALL unset: 8472 yard regressions skipped");
        return;
    };
    let mut sim = Sim::from_setup(&setup(
        db.clone(),
        &[(
            "8472_mothership",
            Vec3 {
                x: n(100),
                y: Fixed::ZERO,
                z: n(100),
            },
        )],
    ));
    let mother = unit_id(&sim, "8472_mothership");
    assert_eq!(sim.unit(mother).unwrap().kind, EntityKind::Station);

    fund_biomatter(&mut sim);
    let product = db.key("8472_active").expect("mothership product");
    let start = sim.step(&[command(
        1,
        Order::Build {
            builder: mother,
            what: product,
            at: None,
        },
    )]);
    assert!(
        !start
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. })),
        "mothership build order should be admitted: {:?}",
        start.events
    );
    let queued = sim.production_view();
    let queue = queued
        .iter()
        .find(|row| row.builder == mother)
        .expect("yard queue");
    assert_eq!(queue.jobs[0].def, product);

    let before = sim.unit(mother).unwrap().pos;
    let moved = sim.step(&[command(
        2,
        Order::Move {
            units: vec![mother],
            to: Vec3 {
                x: n(700),
                y: Fixed::ZERO,
                z: n(100),
            },
            queue: false,
        },
    )]);
    assert!(
        !moved
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. })),
        "mobile mothership move should be admitted: {:?}",
        moved.events
    );
    for _ in 0..120 {
        sim.step(&[]);
    }
    let after = sim.unit(mother).unwrap();
    assert_ne!(after.pos, before, "the station-class mothership moves");
    assert_eq!(after.kind, EntityKind::Station);
    assert_eq!(
        sim.production_view()
            .iter()
            .find(|row| row.builder == mother)
            .unwrap()
            .jobs[0]
            .def,
        product,
        "movement preserves its existing build queue"
    );
}

#[test]
fn installed_fluidic_gate_builds_through_its_3d_aperture() {
    let Some(db) = installed_db() else {
        eprintln!("STA64_TEST_8472_YARDS_INSTALL unset: 8472 yard regressions skipped");
        return;
    };
    let gate_at = Vec3 {
        x: n(500),
        y: n(25),
        z: n(500),
    };
    let mut sim = Sim::from_setup(&setup(db.clone(), &[("8472_FluidicGate", gate_at)]));
    let builder = unit_id(&sim, "8472_FluidicGate");
    let builder_def = db.unit(db.key("8472_FluidicGate").unwrap()).unwrap();
    let wormhole = db
        .launch_wormhole(builder_def)
        .expect("gate resolves fluidicrift2");
    let launch = builder_def
        .launch
        .as_ref()
        .expect("installed buildHardpoint has SOD pose");
    assert_ne!(
        launch.height,
        Fixed::ZERO,
        "the installed buildHardpoint has authored height"
    );
    assert_eq!(
        wormhole.wormhole_open_ticks(),
        2 * u64::from(sta_core::TICK_HZ)
    );
    let product = db.key("8472_defender").expect("Fluidic Gate buildItem");
    assert!(builder_def
        .build
        .list
        .iter()
        .any(|item| item.key == product));

    fund_biomatter(&mut sim);
    let start = sim.step(&[command(
        1,
        Order::Build {
            builder,
            what: product,
            at: None,
        },
    )]);
    assert!(
        !start
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. })),
        "Fluidic Gate build order should be admitted: {:?}",
        start.events
    );
    let initial = sim
        .production_view()
        .into_iter()
        .find(|row| row.builder == builder)
        .unwrap();
    assert!(
        initial.portal.is_none(),
        "no aperture preview before full production"
    );
    assert!(
        initial.site.is_none(),
        "no stationary construction site preview"
    );
    assert_eq!(
        sim.view().units.len(),
        1,
        "no product child before completion"
    );

    let build_ticks = db.unit(product).unwrap().build.ticks;
    let mut ready = None;
    for _ in 0..=build_ticks {
        sim.step(&[]);
        let row = sim
            .production_view()
            .into_iter()
            .find(|row| row.builder == builder)
            .expect("yard queue remains until the aperture releases the child");
        if row.jobs[0].progress == Fixed::ONE {
            ready = Some(row);
            break;
        }
        assert!(
            row.site.is_none(),
            "no construction site preview before completion"
        );
        assert_eq!(sim.view().units.len(), 1, "no child before completion");
    }
    let ready = ready.expect("installed hull reaches full production");
    assert_eq!(
        ready.jobs[0].progress,
        Fixed::ONE,
        "the hull has finished production"
    );
    let portal = ready.portal.expect("finished hull derives its aperture");
    assert!(portal.open < Fixed::ONE, "the aperture still has to open");
    assert!(ready.site.is_none(), "a gate hull is not a site preview");
    assert_eq!(sim.view().units.len(), 1, "no child before aperture opens");

    // +Z-facing map pose makes the local hardpoint axes world aligned.
    let expected_pos = Vec3 {
        x: gate_at.x + launch.offset.0,
        y: gate_at.y + launch.height,
        z: gate_at.z + launch.offset.1,
    };
    assert_eq!(portal.def, wormhole.key);
    assert_eq!(
        portal.pos, expected_pos,
        "portal uses the full 3D buildHardpoint"
    );
    assert_eq!(
        portal.heading, launch.dir,
        "portal faces out along buildHardpoint"
    );

    for _ in 1..wormhole.wormhole_open_ticks() {
        sim.step(&[]);
        assert_eq!(
            sim.view().units.len(),
            1,
            "no child until the aperture opens"
        );
    }
    let opened = sim.step(&[]);
    let children: Vec<_> = opened
        .view
        .units
        .iter()
        .filter(|unit| unit.def == product)
        .collect();
    assert_eq!(children.len(), 1, "one finished child is released");
    let child = sim.unit(children[0].id).unwrap();
    assert_eq!(child.pos, expected_pos, "child appears at the aperture");
    let target = child
        .target
        .expect("released child receives an outgoing course");
    let course =
        (target.x - child.pos.x) * portal.heading.0 + (target.z - child.pos.z) * portal.heading.1;
    assert!(
        course > Fixed::ZERO,
        "outgoing course follows the aperture heading"
    );
    assert!(opened
        .events
        .iter()
        .any(|event| matches!(event, SimEvent::Built { builder: id, .. } if *id == builder)));
}
