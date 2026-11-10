use std::time::Duration;

use bevy::input::{
    InputPlugin,
    touch::{TouchInput, TouchPhase},
};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy::ui::Pressed;
use bevy::window::AppLifecycle;
use sta_sim::command::{CommandKind, LocalRouter, Order};
use sta_sim::{EntityId, Fixed, MoveStats, PlayerId, Sim, Vec3 as SimVec3};

use crate::app::{GameSet, configure_sets};
use crate::clock::AnimClock;
use crate::input::{
    orders::Aim,
    select::{AimState, SelectBox},
};
use crate::platform::foreground;
use crate::platform::lifecycle::AndroidLifecyclePlugin;
use crate::sim_link::{CommandQueue, SimLink, SimLinkPlugin};
use crate::ui::hud::{Armed, HudState, model::TargetMode};
use crate::ui::theme::{ButtonClicked, Toggle, ToggleChanged, UiButton, WidgetState, click};

fn headless() -> (App, EntityId) {
    let mut sim = Sim::new(17);
    let id = sim.spawn(
        0,
        SimVec3::ZERO,
        (Fixed::ZERO, Fixed::ONE),
        MoveStats::from_f32(140.0, 100.0, 1.0, 1.0, 1.0),
    );
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 30.0,
        )));
    configure_sets(&mut app);
    app.add_plugins(InputPlugin)
        .add_plugins(SimLinkPlugin)
        .add_plugins(AndroidLifecyclePlugin)
        .insert_resource(SimLink::new(
            Box::new(sim),
            Box::new(LocalRouter::new()),
            PlayerId(0),
        ));
    (app, id)
}

fn frame_seconds(app: &mut App, seconds: f64) {
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            seconds,
        )));
}

fn tick(app: &App) -> u64 {
    app.world().resource::<SimLink>().curr().tick
}

fn lifecycle(app: &mut App, event: AppLifecycle) {
    app.world_mut().write_message(event);
}

#[test]
fn background_and_first_resume_frame_do_not_advance_sim_or_wall_delta() {
    let (mut app, _) = headless();
    for _ in 0..3 {
        app.update();
    }
    let before = tick(&app);
    let wall_before = app.world().resource::<AnimClock>().wall();
    assert!(before > 0);

    frame_seconds(&mut app, 30.0);
    lifecycle(&mut app, AppLifecycle::Suspended);
    app.update();
    assert_eq!(tick(&app), before);
    assert_eq!(app.world().resource::<AnimClock>().wall(), wall_before);
    app.update();
    assert_eq!(tick(&app), before, "background time is not accumulated");
    assert_eq!(app.world().resource::<AnimClock>().wall(), wall_before);

    // Bevy/winit can publish Running directly after suspension.
    lifecycle(&mut app, AppLifecycle::Running);
    app.update();
    assert_eq!(tick(&app), before, "the first resumed frame is discarded");
    assert_eq!(app.world().resource::<AnimClock>().tick(), before);
    assert_eq!(app.world().resource::<AnimClock>().wall(), wall_before);

    frame_seconds(&mut app, 1.0 / 30.0);
    app.update();
    assert!(
        tick(&app) > before,
        "normal ticks resume after the gated frame"
    );
    assert!(app.world().resource::<AnimClock>().wall() > wall_before);
}

#[test]
fn desktop_without_lifecycle_gate_is_foreground() {
    assert!(foreground(None));
}

#[test]
fn lifecycle_gate_preserves_the_existing_player_pause() {
    let (mut app, _) = headless();
    for _ in 0..3 {
        app.update();
    }
    app.world_mut().resource_mut::<AnimClock>().set_paused(true);
    let before = tick(&app);

    frame_seconds(&mut app, 12.0);
    lifecycle(&mut app, AppLifecycle::Suspended);
    app.update();
    lifecycle(&mut app, AppLifecycle::WillResume);
    app.update();
    frame_seconds(&mut app, 1.0 / 30.0);
    app.update();
    assert_eq!(tick(&app), before);
    assert!(app.world().resource::<AnimClock>().paused());

    app.world_mut()
        .resource_mut::<AnimClock>()
        .set_paused(false);
    app.update();
    assert!(tick(&app) > before);
}

#[derive(Resource)]
struct UiProducer {
    calls: u32,
    sent: bool,
    actor: EntityId,
}

fn produce_one_ui_command(mut producer: ResMut<UiProducer>, mut queue: ResMut<CommandQueue>) {
    producer.calls += 1;
    if !producer.sent {
        queue.0.push(CommandKind::Order(Order::Stop {
            units: vec![producer.actor],
        }));
        producer.sent = true;
    }
}

#[derive(Resource)]
struct ClickProbe {
    fire: bool,
    button: Entity,
    toggle: Entity,
}

fn click_probe(world: &mut World) {
    let Some((fire, button, toggle)) = world
        .get_resource::<ClickProbe>()
        .map(|probe| (probe.fire, probe.button, probe.toggle))
    else {
        return;
    };
    if fire {
        world.resource_mut::<ClickProbe>().fire = false;
        click(world, button);
        click(world, toggle);
    }
}

#[test]
fn suspend_clears_transient_input_and_widgets_but_keeps_committed_commands() {
    let (mut app, actor) = headless();
    app.init_resource::<Aim>()
        .init_resource::<HudState>()
        .add_message::<ButtonClicked>()
        .add_message::<ToggleChanged>()
        .insert_resource(UiProducer {
            calls: 0,
            sent: false,
            actor,
        })
        .add_systems(Update, produce_one_ui_command.in_set(GameSet::Ui))
        .add_systems(PreUpdate, click_probe);
    let button = app.world_mut().spawn((UiButton, WidgetState::Pressed)).id();
    let toggle = app
        .world_mut()
        .spawn((Toggle { on: false }, WidgetState::Pressed))
        .id();
    app.world_mut().insert_resource(ClickProbe {
        fire: false,
        button,
        toggle,
    });
    let pressed = app.world_mut().spawn(Pressed).id();
    let selection_box = app.world_mut().spawn((SelectBox, Visibility::Visible)).id();

    app.update();
    assert_eq!(app.world().resource::<UiProducer>().calls, 1);
    assert_eq!(app.world().resource::<CommandQueue>().0.len(), 1);
    let before = tick(&app);

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyA);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.world_mut().write_message(TouchInput {
        phase: TouchPhase::Started,
        position: Vec2::new(10.0, 20.0),
        window: Entity::PLACEHOLDER,
        force: None,
        id: 77,
    });
    app.world_mut().resource_mut::<Aim>().0 = Some(AimState {
        cursor: Vec2::new(10.0, 20.0),
        ray: Ray3d::new(Vec3::ZERO, Dir3::NEG_Y),
        clip_from_world: Mat4::IDENTITY,
        size: Vec2::new(800.0, 600.0),
    });
    {
        let mut hud = app.world_mut().resource_mut::<HudState>();
        hud.placing = Some(Armed {
            builder: actor,
            what: sta_sim::ContentKey(1),
            odf: "fx_station".into(),
        });
        hud.targeting = Some(TargetMode::Attack);
    }
    app.world_mut().write_message(ButtonClicked(button));
    app.world_mut().write_message(ToggleChanged {
        entity: toggle,
        on: true,
    });
    app.world_mut().resource_mut::<ClickProbe>().fire = true;

    frame_seconds(&mut app, 15.0);
    lifecycle(&mut app, AppLifecycle::Suspended);
    app.update();
    assert_eq!(tick(&app), before);
    assert_eq!(
        app.world().resource::<UiProducer>().calls,
        1,
        "UI producer is gated"
    );
    assert_eq!(
        app.world().resource::<CommandQueue>().0.len(),
        1,
        "committed queue entry survives"
    );
    assert!(
        !app.world()
            .resource::<ButtonInput<KeyCode>>()
            .pressed(KeyCode::KeyA)
    );
    assert!(
        !app.world()
            .resource::<ButtonInput<MouseButton>>()
            .pressed(MouseButton::Left)
    );
    assert!(app.world().resource::<Touches>().iter().next().is_none());
    assert!(app.world().resource::<Aim>().0.is_none());
    let hud = app.world().resource::<HudState>();
    assert!(hud.placing.is_none() && hud.targeting.is_none());
    assert!(app.world().get::<Pressed>(pressed).is_none());
    assert_eq!(
        *app.world().get::<Visibility>(selection_box).unwrap(),
        Visibility::Hidden
    );
    assert_eq!(
        *app.world().get::<WidgetState>(button).unwrap(),
        WidgetState::Normal
    );
    assert_eq!(
        *app.world().get::<WidgetState>(toggle).unwrap(),
        WidgetState::Normal
    );
    assert!(!app.world().get::<Toggle>(toggle).unwrap().on);
    assert_eq!(app.world().resource::<Messages<ButtonClicked>>().len(), 0);
    assert_eq!(app.world().resource::<Messages<ToggleChanged>>().len(), 0);

    lifecycle(&mut app, AppLifecycle::WillResume);
    app.world_mut().resource_mut::<ClickProbe>().fire = true;
    app.update();
    assert_eq!(tick(&app), before, "resume frame is still gated");
    assert_eq!(app.world().resource::<UiProducer>().calls, 1);
    assert_eq!(app.world().resource::<CommandQueue>().0.len(), 1);
    assert!(!app.world().get::<Toggle>(toggle).unwrap().on);
    assert_eq!(app.world().resource::<Messages<ButtonClicked>>().len(), 0);

    frame_seconds(&mut app, 1.0 / 30.0);
    // Duration rounds one 30 Hz frame down to 33,333,333 ns. Two normal
    // frames supply a complete tick even when the initial clock debt is zero.
    app.update();
    app.update();
    assert!(tick(&app) > before);
    assert!(
        app.world().resource::<CommandQueue>().0.is_empty(),
        "queued command reaches SimLink"
    );
    click(app.world_mut(), button);
    click(app.world_mut(), toggle);
    assert_eq!(app.world().resource::<Messages<ButtonClicked>>().len(), 1);
    assert!(app.world().get::<Toggle>(toggle).unwrap().on);
    assert_eq!(app.world().resource::<Messages<ToggleChanged>>().len(), 1);
}
