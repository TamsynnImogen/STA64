use std::{sync::Arc, time::Duration};

use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use sta_sim::command::LocalRouter;
use sta_sim::setup::{GameOptions, GameSetup};
use sta_sim::{Fixed, PlayerId, Sim};

use super::*;
use crate::clock::{AnimClock, ClockMode};
use crate::sim_link::SimLinkPlugin;

fn app() -> (App, EntityId) {
    let content = Arc::new(sta_content::fake::db());
    let setup = GameSetup {
        seed: 1,
        players: Vec::new(),
        map: Arc::new(sta_content::fake::two_bases()),
        content,
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let id = sim.spawn_def(
        setup.content.key("fx_cruiser").unwrap(),
        PlayerId(0),
        SimVec3::ZERO,
        (Fixed::ZERO, Fixed::ONE),
    ).unwrap();
    let mut app = App::new();
    app.set_error_handler(bevy::ecs::error::panic)
        .add_plugins((MinimalPlugins, StatesPlugin))
        .init_state::<AppState>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<Aim>()
        .init_resource::<crate::ui::theme::Theme>()
        .add_message::<ButtonClicked>()
        .add_message::<Notice>()
        .add_message::<crate::game::flow::LeaveMatch>()
        .insert_resource(ContentRes(setup.content))
        .insert_resource(AnimClock::new(ClockMode::Wall, 0))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)))
        .insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)))
        .add_plugins((SimLinkPlugin, EditorPlugin));
    crate::app::configure_sets(&mut app);
    app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::InGame);
    app.update();
    (app, id)
}

/// Write Bevy's input resource in-process. No OS keyboard events are involved.
fn hotkey(app: &mut App) {
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::ControlLeft);
        keys.press(KeyCode::KeyE);
    }
    app.update();
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().reset_all();
}

#[test]
fn ctrl_e_pauses_and_resumes_the_same_local_world() {
    let (mut app, id) = app();
    let tick = app.world().resource::<SimLink>().curr().tick;
    hotkey(&mut app);
    assert!(app.world().contains_resource::<Editing>());
    assert!(app.world().resource::<SimLink>().fog_lifted());
    let at = SimVec3::from_f32([29.0, 0.0, 43.0]);
    assert_eq!(
        app.world_mut().resource_mut::<SimLink>().edit_placement(PlacementEdit::Transform { id, at, quarter_turns: 1 }),
        Some(id),
    );
    app.update();
    {
        let link = app.world().resource::<SimLink>();
        assert_eq!(link.curr().tick, tick);
        let unit = link.curr().unit(id).unwrap();
        assert_eq!(unit.pos, at);
        assert_eq!(unit.heading, (Fixed::ONE, Fixed::ZERO));
    }
    hotkey(&mut app);
    app.update();
    assert!(!app.world().contains_resource::<Editing>());
    let link = app.world().resource::<SimLink>();
    assert!(!link.fog_lifted());
    assert!(link.curr().tick > tick);
    assert_eq!(link.curr().unit(id).unwrap().pos, at);
}

#[test]
fn placement_buttons_edit_the_paused_world_and_saveable_document() {
    let (mut app, _) = app();
    let map = sta_bzn::Map { version: 3013, size: [1000.0, 100.0, 1000.0], ..default() };
    let bytes = sta_bzn::write::write(&map).unwrap();
    let doc = Document::new("bzn/test.bzn".into(), bytes, map).unwrap();
    app.insert_resource(doc).insert_resource(AuthoredIds(BTreeMap::new()));
    hotkey(&mut app);

    app.world_mut().resource_mut::<Editing>().placing = Some(("fx_cruiser".into(), false));
    app.world_mut().resource_mut::<Aim>().0 = Some(crate::input::select::AimState {
        cursor: Vec2::new(500.0, 400.0),
        ray: Ray3d::new(Vec3::new(21.0, 45.0, -76.0), Dir3::NEG_Y),
        clip_from_world: Mat4::IDENTITY,
        size: Vec2::new(1000.0, 800.0),
    });
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
    app.update();
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().reset_all();
    let id = app.world().resource::<Selection>().0[0];
    let index = app.world().resource::<AuthoredIds>().0[&id];
    let at = [21.0, 0.0, 76.0];
    assert_eq!(app.world().resource::<SimLink>().curr().unit(id).unwrap().pos.to_f32(), at);

    let doc = app.world().resource::<Document>();
    let saved = sta_bzn::parse(&doc.bytes().unwrap()).unwrap();
    assert_eq!(&saved.objects[0].transform[9..], &at);
    assert_eq!(doc.object(index).unwrap().odf, "fx_cruiser");

    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Delete);
    app.update();
    assert!(app.world().resource::<SimLink>().curr().unit(id).is_none());
    assert!(app.world().resource::<Document>().object(index).is_none());
}
