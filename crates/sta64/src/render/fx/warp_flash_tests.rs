use super::{WarpFlash, register, warp_reach};

use crate::app::configure_sets;
use crate::clock::AnimClock;
use crate::sim_link::{ContentRes, SimEntity, SimLink};
use bevy::prelude::*;
use sta_content::{ContentDb, MapDef, MoveStats, UnitDef};
use sta_sim::command::{CommandKind, LocalRouter, Order};
use sta_sim::setup::{Control, GameSetup, PlayerSetup};
use sta_sim::{EntityId, Fixed, PlayerId, Sim, TeamId, Vec3 as SimVec3};
use std::sync::Arc;

#[test]
fn reach_uses_clamped_stretch_and_speed_limits() {
    // The stretch is just under 3; duration uses the acceleration span.
    assert!((warp_reach(10.0, 20.0, 100.0, 0.0).unwrap() - 39.7015).abs() < 0.01);
    // At impulse, acceleration span falls to 80, making the reach longer.
    assert!((warp_reach(10.0, 20.0, 100.0, 20.0).unwrap() - 49.6269).abs() < 0.01);
    // Speed is clamped to the impulse cap before calculating the span.
    assert_eq!(
        warp_reach(10.0, 20.0, 100.0, 200.0),
        warp_reach(10.0, 20.0, 100.0, 20.0)
    );
    assert_eq!(
        warp_reach(10.0, 20.0, 100.0, -5.0),
        warp_reach(10.0, 20.0, 100.0, 0.0)
    );
}

#[test]
fn reach_rejects_non_warping_or_radiusless_inputs() {
    assert_eq!(warp_reach(0.0, 20.0, 100.0, 0.0), None);
    assert_eq!(warp_reach(-1.0, 20.0, 100.0, 0.0), None);
    assert_eq!(warp_reach(10.0, -1.0, 100.0, 0.0), None);
    assert_eq!(warp_reach(10.0, 20.0, 20.0, 0.0), None);
    assert_eq!(warp_reach(10.0, 20.0, 10.0, 0.0), None);
}

fn live_app() -> (App, EntityId) {
    let mut def = UnitDef::new("warp_flash_test", "craft");
    def.hull = Fixed::from_int(100);
    def.radius = Fixed::from_int(20);
    def.movement = MoveStats::from_f32(20.0, 1.0, 1.0, 1.0, 1.0);
    def.movement.warp_speed = Fixed::from_int(100);
    let content = Arc::new(ContentDb::builder().unit(def).build().unwrap());
    let setup = GameSetup {
        seed: 41,
        map: Arc::new(MapDef::empty()),
        content: content.clone(),
        players: vec![PlayerSetup {
            id: PlayerId(0),
            team: TeamId(0),
            race: "federation".into(),
            control: Control::Human,
        }],
        options: Default::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let id = sim
        .spawn_def(
            content.key("warp_flash_test").unwrap(),
            PlayerId(0),
            SimVec3::ZERO,
            (Fixed::ZERO, Fixed::ONE),
        )
        .unwrap();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<AnimClock>()
        .insert_resource(ContentRes(content))
        .insert_resource(SimLink::new(
            Box::new(sim),
            Box::new(LocalRouter::new()),
            PlayerId(0),
        ));
    configure_sets(&mut app);
    register(&mut app);
    (app, id)
}

fn flash_count(app: &mut App) -> usize {
    let mut query = app.world_mut().query::<&WarpFlash>();
    query.iter(app.world()).count()
}

fn step(app: &mut App) {
    app.world_mut()
        .resource_mut::<SimLink>()
        .tick()
        .expect("local sim advances");
    app.update();
}

fn order(app: &mut App, id: EntityId, warp: bool) {
    let mut link = app.world_mut().resource_mut::<SimLink>();
    link.submit(CommandKind::Order(if warp {
        Order::MoveWarp {
            units: vec![id],
            to: SimVec3 {
                x: Fixed::ZERO,
                y: Fixed::ZERO,
                z: Fixed::from_int(1200),
            },
            queue: false,
        }
    } else {
        Order::Move {
            units: vec![id],
            to: SimVec3 {
                x: Fixed::ZERO,
                y: Fixed::ZERO,
                z: Fixed::from_int(1200),
            },
            queue: false,
        }
    }));
}

#[test]
fn a_live_move_warp_creates_one_flash_and_warp_out_does_not_retrigger_it() {
    let (mut app, id) = live_app();
    app.update(); // Seed the tracker's first observed tick.
    order(&mut app, id, false);
    step(&mut app);
    for _ in 0..10 {
        step(&mut app);
    }
    assert_eq!(
        flash_count(&mut app),
        0,
        "plain impulse movement has no flash"
    );
    order(&mut app, id, true);

    let mut entered = false;
    let mut exited = false;
    let mut max_flashes = 0;
    for _ in 0..1500 {
        step(&mut app);
        let phase = app.world().resource::<SimLink>().warp_phase(id);
        if !entered && phase == Some(true) {
            assert_eq!(flash_count(&mut app), 1, "the entry edge creates the flash");
            entered = true;
        }
        if phase == Some(false) {
            assert_eq!(flash_count(&mut app), 0, "warp-out does not create a flash");
        }
        exited |= entered && phase == Some(false);
        max_flashes = max_flashes.max(flash_count(&mut app));
        assert!(max_flashes <= 1, "more than one active flash for one ship");
        if exited && flash_count(&mut app) == 0 {
            break;
        }
    }
    assert!(entered, "the real MoveWarp order entered warp");
    assert!(exited, "the real MoveWarp order exposed warp-out");
    assert_eq!(max_flashes, 1, "entry produced exactly one logical flash");
    assert_eq!(
        flash_count(&mut app),
        0,
        "the effect expired after the ship cleared it"
    );
}

#[test]
fn long_warp_flash_expires_while_hull_stays_stretched_until_dropout() {
    let (mut app, id) = live_app();
    let root_transform = Transform::from_xyz(7.0, 11.0, 13.0)
        .with_rotation(Quat::from_rotation_y(0.37))
        .with_scale(Vec3::new(2.0, 3.0, 4.0));
    let root = app.world_mut().spawn((SimEntity(id), root_transform)).id();
    let centre = Vec3::Z * 7.0;
    let hull = app
        .world_mut()
        .spawn((
            super::WarpHull { pivot_z: centre.z },
            Transform::IDENTITY,
            ChildOf(root),
        ))
        .id();

    app.update();
    order(&mut app, id, true);
    let mut flashed = false;
    let mut cruised_after_flash = false;
    let mut dropping_out = false;
    let mut restored = false;
    let mut previous_stretch = 1.0;
    for _ in 0..1500 {
        step(&mut app);
        let phase = app.world().resource::<SimLink>().warp_phase(id);
        let transform = *app.world().get::<Transform>(hull).unwrap();
        let stretch = transform.scale.z;
        let flash_active = flash_count(&mut app) > 0;
        flashed |= flash_active;
        assert!((transform.transform_point(centre) - centre).length() < 0.001);
        match phase {
            Some(true) if flashed && !flash_active => {
                assert!(
                    stretch > 2.9,
                    "the hull holds its full warp stretch after the flash"
                );
                cruised_after_flash = true;
            }
            Some(false) => {
                assert!(
                    stretch <= previous_stretch,
                    "warp-out winds down the stretch"
                );
                dropping_out = true;
            }
            None if dropping_out => {
                assert_eq!(transform, Transform::IDENTITY);
                restored = true;
                break;
            }
            _ => {}
        }
        previous_stretch = stretch;
    }

    assert!(flashed && cruised_after_flash && dropping_out && restored);
    assert_eq!(*app.world().get::<Transform>(root).unwrap(), root_transform);
}

#[test]
fn observing_an_already_warping_match_does_not_play_the_entry_again() {
    let (mut app, id) = live_app();
    app.update();
    order(&mut app, id, true);
    for _ in 0..1000 {
        step(&mut app);
        if app.world().resource::<SimLink>().warp_phase(id) == Some(true) {
            assert_eq!(flash_count(&mut app), 1);
            let link = app.world_mut().remove_resource::<SimLink>().unwrap();
            app.update();
            assert_eq!(flash_count(&mut app), 0);

            app.insert_resource(link);
            app.update();
            assert_eq!(flash_count(&mut app), 0, "loaded mid-warp");
            step(&mut app);
            assert_eq!(flash_count(&mut app), 0, "still in the same warp");
            return;
        }
    }
    panic!("the live warp move should enter warp");
}
