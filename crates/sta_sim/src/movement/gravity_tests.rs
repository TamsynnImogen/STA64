//! Authored `RTS_CFG.h` dewarp fields suppress warp for planets and
//! black holes while a craft is inside the field plus its own bounding radius.

use std::sync::Arc;

use sta_content::{ContentDb, MoveStats, Profile, Rules, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Order};
use crate::movement::approach::{IN_WARP, WARP_OUT};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

fn at(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3 {
        x: n(x),
        y: n(y),
        z: n(z),
    }
}

fn authored_fixture(well_class: &str) -> (Sim, EntityId) {
    let (rules, warnings) = Rules::from_header(
        "int cfgPLANET_DEWARP_RADIUS_MULTIPLIER = 3;\n\
         int cfgBLACKHOLE_DEWARP_RADIUS_MULTIPLIER = 3;\n",
    );
    assert!(warnings.is_empty());

    let mut craft = UnitDef::new("gravity_test_craft", "craft");
    craft.race = "st".into();
    craft.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 1.0);
    craft.movement.warp_speed = n(400);
    craft.radius = n(12);
    craft.hull = n(1000);

    // These authored ODF class labels and positive bounds are the inputs used
    // by Rules::warp_suppression_radius (planet radius 100 * cfg 3).
    let well = UnitDef {
        raw: Arc::new(
            sta_odf::resolve("gravity_test_well", &mut |_| {
                Some(format!("classLabel = \"{well_class}\"\n"))
            })
            .unwrap(),
        ),
        radius: n(100),
        hull: n(1000),
        ..UnitDef::new("gravity_test_well", well_class)
    };

    let db = Arc::new(
        ContentDb::builder()
            .profile(Profile::Sta2Classic)
            .features(Profile::Sta2Classic.features())
            .unit(craft)
            .unit(well)
            .build()
            .expect("gravity fixture content")
            .with_rules(rules),
    );
    let seat = |id| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "st".into(),
        control: Control::Idle,
    };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 5,
        map: Arc::new(sta_content::MapDef::empty()),
        content: db.clone(),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    });
    sim.spawn_def(
        db.key("gravity_test_well").unwrap(),
        PlayerId::BENCH,
        at(0, 0, 0),
        (Fixed::ZERO, Fixed::ONE),
    )
    .unwrap();
    // A 200-unit lateral offset clears the well's physical 100-unit body,
    // while remaining within the 312-unit dewarp radius around its centre.
    let craft_id = sim
        .spawn_def(
            db.key("gravity_test_craft").unwrap(),
            PlayerId(1),
            at(200, 0, -1600),
            (Fixed::ZERO, Fixed::ONE),
        )
        .unwrap();
    (sim, craft_id)
}

fn move_warp(sim: &mut Sim, craft: EntityId, to: Vec3) {
    WorldSource::step(
        sim,
        &[Command {
            player: PlayerId(1),
            seq: 0,
            kind: CommandKind::Order(Order::MoveWarp {
                units: vec![craft],
                to,
                queue: false,
            }),
        }],
    );
}

fn assert_warp_is_suppressed_inside_authored_field(well_class: &str) {
    let (mut sim, craft) = authored_fixture(well_class);
    move_warp(&mut sim, craft, at(200, 0, 1600));

    let mut transitions = Vec::new();
    let mut previous_phase = None;
    let mut reached_impulse_inside = false;
    for _ in 0..30 * 30 {
        let position = sim.unit(craft).unwrap().pos;
        let centre_distance =
            ((position.x * position.x) + (position.y * position.y) + (position.z * position.z))
                .sqrt();
        // Steering samples the pre-step position.
        let inside = centre_distance < n(312);
        WorldSource::step(&mut sim, &[]);
        let unit = sim.unit(craft).unwrap();
        let phase = sim.approach_warp.phase(craft);
        if phase != previous_phase {
            transitions.extend(phase);
            previous_phase = phase;
        }
        if inside {
            assert_ne!(
                phase,
                Some(IN_WARP),
                "in warp inside {well_class} at {position:?}"
            );
            reached_impulse_inside |= unit.speed <= unit.stats.max_speed;
        }
        if unit.target.is_none() && unit.speed == Fixed::ZERO {
            break;
        }
    }
    assert!(
        reached_impulse_inside,
        "craft never reached impulse inside {well_class}"
    );
    assert_eq!(transitions, [IN_WARP, WARP_OUT, IN_WARP, WARP_OUT]);
}

#[test]
fn planet_field_suppresses_move_warp_and_allows_reentry_after_exit() {
    assert_warp_is_suppressed_inside_authored_field("planet");
}

#[test]
fn black_hole_field_suppresses_move_warp_and_allows_reentry_after_exit() {
    assert_warp_is_suppressed_inside_authored_field("blackhole_obj");
}

#[test]
fn warp_entry_checks_altitude_and_hull_overlap() {
    // (120, 288, 0) lies exactly 312 units from the well's centre.
    for (height, expected_phase) in [(287, None), (288, Some(IN_WARP))] {
        let (mut sim, craft) = authored_fixture("planet");
        let ship = sim.units.get_mut(craft).unwrap();
        ship.pos = at(120, height, 0);
        ship.speed = ship.stats.max_speed;

        move_warp(&mut sim, craft, at(120, height, 1600));
        assert_eq!(
            sim.approach_warp.phase(craft),
            expected_phase,
            "height {height}"
        );
    }
}
