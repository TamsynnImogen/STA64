use super::{EXIT_PULSE_SECONDS, WarpFlash, ramp_progress, register, warp_reach};
use crate::render::profile::RenderProfile;
use sta_sim::view::{WarpPhase, WarpRow};

use crate::app::configure_sets;
use crate::clock::AnimClock;
use crate::sim_link::{ContentRes, SimEntity, SimLink};
use bevy::prelude::*;
use sta_content::{ContentDb, MapDef, MoveStats, UnitDef};
use sta_sim::command::{CommandKind, LocalRouter, Order};
use sta_sim::setup::{Control, GameSetup, PlayerSetup};
use sta_sim::{EntityId, Fixed, PlayerId, Sim, TeamId, Vec3 as SimVec3};
use std::sync::Arc;

/// The stretch factor of the authored speeds (impulse 20, warp 100): 0.6 x 100 / 20.1.
const S: f32 = 2.985_07;

#[test]
fn reach_uses_clamped_stretch_and_speed_limits() {
    // The stretch is just under 3; duration uses the acceleration span.
    assert!((warp_reach(10.0, 20.0, 100.0, 0.0, S).unwrap() - 39.7015).abs() < 0.01);
    // At impulse, acceleration span falls to 80, making the reach longer.
    assert!((warp_reach(10.0, 20.0, 100.0, 20.0, S).unwrap() - 49.6269).abs() < 0.01);
    // Speed is clamped to the impulse cap before calculating the span.
    assert_eq!(
        warp_reach(10.0, 20.0, 100.0, 200.0, S),
        warp_reach(10.0, 20.0, 100.0, 20.0, S)
    );
    assert_eq!(
        warp_reach(10.0, 20.0, 100.0, -5.0, S),
        warp_reach(10.0, 20.0, 100.0, 0.0, S)
    );
}

#[test]
fn reach_rejects_non_warping_or_radiusless_inputs() {
    assert_eq!(warp_reach(0.0, 20.0, 100.0, 0.0, S), None);
    assert_eq!(warp_reach(-1.0, 20.0, 100.0, 0.0, S), None);
    assert_eq!(warp_reach(10.0, -1.0, 100.0, 0.0, S), None);
    assert_eq!(warp_reach(10.0, 20.0, 20.0, 0.0, S), None);
    assert_eq!(warp_reach(10.0, 20.0, 10.0, 0.0, S), None);
}

fn live_app() -> (App, EntityId) {
    live_app_with(None)
}

fn live_app_with(profile: Option<RenderProfile>) -> (App, EntityId) {
    live_app_in(profile, sta_content::Profile::Native)
}

fn live_app_in(profile: Option<RenderProfile>, content_profile: sta_content::Profile) -> (App, EntityId) {
    let mut def = UnitDef::new("warp_flash_test", "craft");
    def.hull = Fixed::from_int(100);
    def.radius = Fixed::from_int(20);
    def.movement = MoveStats::from_f32(20.0, 1.0, 1.0, 1.0, 1.0);
    def.movement.warp_speed = Fixed::from_int(100);
    let content = Arc::new(ContentDb::builder().profile(content_profile).unit(def).build().unwrap());
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
    if let Some(profile) = profile {
        app.insert_resource(profile);
    }
    configure_sets(&mut app);
    register(&mut app);
    (app, id)
}

fn flash_count(app: &mut App) -> usize {
    let mut query = app.world_mut().query::<&WarpFlash>();
    query.iter(app.world()).count()
}

fn flash_exit_started(app: &mut App) -> Option<sta_sim::Tick> {
    let mut query = app.world_mut().query::<&WarpFlash>();
    query
        .iter(app.world())
        .next()
        .and_then(|flash| flash.exit_started)
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
            mode: None,
        }
    }));
}

#[test]
fn a_live_move_warp_creates_entry_and_exit_pulses_once_each() {
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
    let mut entry_pulses = 0;
    let mut exit_pulses = 0;
    let mut exit_started_tick = None;
    let mut max_flashes = 0;
    for _ in 0..1500 {
        step(&mut app);
        let phase = app.world().resource::<SimLink>().warp_phase(id);
        if !entered && phase == Some(true) {
            assert_eq!(flash_count(&mut app), 1, "the entry edge creates the flash");
            assert_eq!(
                flash_exit_started(&mut app),
                None,
                "entry pulse is immediate"
            );
            entry_pulses += 1;
            entered = true;
        }
        if phase == Some(false) && !exited {
            assert_eq!(flash_count(&mut app), 1, "warp-out creates one exit pulse");
            let started = flash_exit_started(&mut app).expect("exit pulse has a start tick");
            exit_started_tick = Some(started);
            let elapsed = app
                .world()
                .resource::<SimLink>()
                .real_curr()
                .tick
                .saturating_sub(started);
            assert!(
                elapsed as f64 / f64::from(sta_sim::TICK_HZ) < EXIT_PULSE_SECONDS,
                "exit pulse is active only inside its fixed lifetime"
            );
            exit_pulses += 1;
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
    assert_eq!(entry_pulses, 1, "one entry transition produced one pulse");
    assert_eq!(exit_pulses, 1, "one exit transition produced one pulse");
    assert_eq!(max_flashes, 1, "each transition owns at most one flash");
    let elapsed = app
        .world()
        .resource::<SimLink>()
        .real_curr()
        .tick
        .saturating_sub(exit_started_tick.expect("the exit pulse started"));
    let lifetime_ticks = (EXIT_PULSE_SECONDS * f64::from(sta_sim::TICK_HZ)).ceil() as sta_sim::Tick;
    assert!(
        elapsed >= lifetime_ticks && elapsed <= lifetime_ticks + 1,
        "exit pulse expires on its fixed sim-tick lifetime"
    );
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

#[test]
fn the_stretch_runs_up_over_the_entry_ramp_holds_at_warp_and_runs_down_over_the_exit_ramp() {
    let row = |phase, start, len| WarpRow { id: EntityId(1), phase, ramp_start: start, ramp_len: len, stretch: sta_sim::Fixed::from_int(3) };
    let entering = row(WarpPhase::Entering, 100, 10);
    assert_eq!(ramp_progress(&entering, 100.0), 0.0);
    assert_eq!(ramp_progress(&entering, 105.0), 0.5);
    assert_eq!(ramp_progress(&entering, 110.0), 1.0);
    assert_eq!(ramp_progress(&entering, 99.0), 0.0, "before the ramp");
    assert_eq!(ramp_progress(&entering, 500.0), 1.0, "after it");
    // At warp the row keeps the entry's ramp, long over: full.
    assert_eq!(ramp_progress(&row(WarpPhase::AtWarp, 100, 10), 900.0), 1.0);
    // Leaving mirrors it.
    let leaving = row(WarpPhase::Leaving, 200, 8);
    assert_eq!(ramp_progress(&leaving, 200.0), 1.0);
    assert_eq!(ramp_progress(&leaving, 204.0), 0.5);
    assert_eq!(ramp_progress(&leaving, 208.0), 0.0);
    // A row with no ramp is at its end: entering full, leaving none.
    assert_eq!(ramp_progress(&row(WarpPhase::Entering, 5, 0), 5.0), 1.0);
    assert_eq!(ramp_progress(&row(WarpPhase::Leaving, 5, 0), 5.0), 0.0);
}

/// Runs a warp move to its end; the hull's stretch and the number of exit pulses per frame.
fn trip(profile: Option<RenderProfile>) -> (Vec<(Option<WarpPhase>, f32)>, bool) {
    let (mut app, id) = live_app_with(profile);
    let root = app.world_mut().spawn((SimEntity(id), Transform::IDENTITY)).id();
    let hull = app.world_mut().spawn((super::WarpHull { pivot_z: 0.0 }, Transform::IDENTITY, ChildOf(root))).id();
    app.update();
    order(&mut app, id, true);
    let (mut rows, mut pulse) = (Vec::new(), false);
    for _ in 0..1500 {
        step(&mut app);
        pulse |= flash_exit_started(&mut app).is_some();
        let phase = app.world().resource::<SimLink>().real_curr().warp_of(id).map(|r| r.phase);
        rows.push((phase, app.world().get::<Transform>(hull).unwrap().scale.z));
    }
    (rows, pulse)
}

#[test]
fn under_stock_there_is_no_exit_pulse_and_the_stretch_follows_the_ramp_and_the_remaster_look_keeps_its_own() {
    let (stock, pulse) = trip(Some(RenderProfile::Stock));
    assert!(!pulse, "no exit pulse under Stock");
    assert!(stock.iter().any(|r| r.0 == Some(WarpPhase::AtWarp)), "the craft reached warp");
    let full = stock.iter().filter(|r| r.0 == Some(WarpPhase::AtWarp)).map(|r| r.1).fold(0.0, f32::max);
    assert!(full > 1.4, "the hull is stretched at warp: {full}");
    // At warp the stretch is full and flat; entering it never runs backwards; leaving it never grows.
    let mut previous = 1.0;
    for (phase, stretch) in &stock {
        match phase {
            Some(WarpPhase::Entering) => assert!(*stretch >= previous - 1e-4 && *stretch <= full + 1e-4, "entering {stretch} after {previous}"),
            Some(WarpPhase::AtWarp) => assert!((stretch - full).abs() < 1e-3, "at warp {stretch} of {full}"),
            Some(WarpPhase::Leaving) => assert!(*stretch <= previous + 1e-4, "leaving {stretch} after {previous}"),
            None => {}
        }
        previous = *stretch;
    }
    // The Remaster look, and a run without the profile resources, keep today's pulse.
    assert!(trip(Some(RenderProfile::Refit)).1, "the Refit look keeps its exit pulse");
    assert!(trip(None).1);
}

#[test]
fn without_the_shape_switch_the_stretch_from_the_derived_ramp_matches_the_speed_driven_one_over_a_whole_trip() {
    // The authored craft has the switch off: its rows are derived from the old smooth ramp. The Stock look reads them by
    // time, the Refit look by speed: they draw the same hull to within a fifth of the stretch (the ramp has a whole number of ticks, the speed does not).
    const TOLERANCE: f32 = 0.2;
    let (stock, _) = trip(Some(RenderProfile::Stock));
    let (refit, _) = trip(Some(RenderProfile::Refit));
    assert_eq!(stock.len(), refit.len());
    let worst = stock.iter().zip(&refit).map(|(a, b)| (a.1 - b.1).abs()).fold(0.0, f32::max);
    assert!(stock.iter().any(|r| r.1 > 1.5), "the craft was stretched");
    assert!(worst <= TOLERANCE, "the two stretches differ by {worst}");
}

/// `(the hull's stretch, the bubble's and the cap's)` of a craft at warp, under the look and the content profile.
fn stretches_at_warp(look: Option<RenderProfile>, profile: sta_content::Profile) -> (f32, f32) {
    use bevy::ecs::system::RunSystemOnce;
    let (mut app, id) = live_app_in(look, profile);
    app.update();
    order(&mut app, id, true);
    let mut at_warp = 0;
    for _ in 0..1500 {
        step(&mut app);
        if app.world().resource::<SimLink>().real_curr().warp_of(id).is_some_and(|r| r.phase == WarpPhase::AtWarp) {
            at_warp += 1;
            if at_warp > 20 {
                break;
            }
        }
    }
    assert!(at_warp > 20, "the craft reached warp");
    app.world_mut().run_system_once(move |w: super::WarpStretch| (w.of(id), w.of_stock(id))).unwrap()
}

#[test]
fn the_bubble_and_the_cap_take_the_hulls_stretch_under_the_stock_look_of_stock_armada_ii_only() {
    use sta_content::Profile;
    let (hull, shared) = stretches_at_warp(Some(RenderProfile::Stock), Profile::Sta2Classic);
    assert!(hull > 1.4, "the hull is stretched at warp: {hull}");
    assert_eq!(shared, hull, "one source: the bubble and the cap take the hull's factor");
    assert_eq!(stretches_at_warp(Some(RenderProfile::Refit), Profile::Sta2Classic).1, 1.0, "the Remaster look: unchanged");
    assert_eq!(stretches_at_warp(Some(RenderProfile::Stock), Profile::Native).1, 1.0, "no stock Armada II content: unchanged");
    assert_eq!(stretches_at_warp(None, Profile::Sta2Classic).1, 1.0, "no profile resources: unchanged");
}

#[test]
fn the_bubble_and_the_cap_are_gated_where_the_hull_carries_its_stretch() {
    use sta_content::Profile;
    assert!(super::carries_hull_stretch(Profile::Sta2Classic));
    // The mod family and the rest: no hull stretch, so no bubble or cap stretch either (not read for the mod family).
    for p in [Profile::Fo3, Profile::Fo4, Profile::Sta1Classic, Profile::Native] {
        assert!(!super::carries_hull_stretch(p), "{p:?}");
    }
}
