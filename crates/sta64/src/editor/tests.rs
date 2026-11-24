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

fn press(app: &mut App, key: KeyCode) {
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(key);
    app.update();
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().reset_all();
}

fn point(app: &mut App, cursor: Vec2, at: Vec3) {
    app.world_mut().resource_mut::<Aim>().0 = Some(crate::input::select::AimState {
        cursor,
        ray: Ray3d::new(at + Vec3::Y * 45.0, Dir3::NEG_Y),
        clip_from_world: Mat4::IDENTITY,
        size: Vec2::new(1000.0, 800.0),
    });
}

fn mouse(app: &mut App, pressed: bool) {
    let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
    buttons.clear();
    if pressed {
        buttons.press(MouseButton::Left);
    } else {
        buttons.release(MouseButton::Left);
    }
    app.update();
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
fn editor_shortcuts_do_not_reach_play_shortcuts() {
    #[derive(Default, Resource)]
    struct Seen(Vec<KeyCode>);

    let (mut app, _) = app();
    app.init_resource::<Seen>().add_systems(Update, |keys: Res<ButtonInput<KeyCode>>, mut seen: ResMut<Seen>| {
        seen.0.extend([KeyCode::F10, KeyCode::Tab].into_iter().filter(|key| keys.just_pressed(*key)));
    });
    press(&mut app, KeyCode::F10);
    press(&mut app, KeyCode::Tab);
    hotkey(&mut app);
    app.world_mut().resource_mut::<Editing>().palette.root = vec![palette::Entry {
        slot: 9,
        label: "Planets".into(),
        kind: EntryKind::Branch(Vec::new()),
    }];
    press(&mut app, KeyCode::F10);
    assert_eq!(app.world().resource::<Editing>().directory, [0]);
    press(&mut app, KeyCode::Tab);
    assert!(app.world().resource::<Editing>().directory.is_empty());
    assert_eq!(app.world().resource::<Seen>().0, [KeyCode::F10, KeyCode::Tab]);
}

#[test]
fn function_keys_navigate_sparse_slots_and_place_at_the_current_pointer() {
    let (mut app, _) = app();
    let map = sta_bzn::Map { version: 3013, size: [1000.0, 100.0, 1000.0], ..default() };
    let bytes = sta_bzn::write::write(&map).unwrap();
    let doc = Document::new("bzn/test.bzn".into(), bytes, map).unwrap();
    app.insert_resource(doc).insert_resource(AuthoredIds(BTreeMap::new()));
    hotkey(&mut app);

    let branch = |slot, children| palette::Entry { slot, label: "Group".into(), kind: EntryKind::Branch(children) };
    let leaf = palette::Entry { slot: 5, label: "Cruiser".into(), kind: EntryKind::Leaf { odf: "fx_cruiser".into(), neutral: true } };
    app.world_mut().resource_mut::<Editing>().palette.root = vec![branch(1, vec![branch(11, vec![leaf])])];

    press(&mut app, KeyCode::F1);
    assert!(app.world().resource::<Editing>().directory.is_empty());
    press(&mut app, KeyCode::F2);
    press(&mut app, KeyCode::F12);
    assert_eq!(app.world().resource::<Editing>().directory.len(), 2);
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.world().resource::<Editing>().directory.len(), 1);
    press(&mut app, KeyCode::F12);
    press(&mut app, KeyCode::F6);
    assert!(app.world().resource::<Document>().objects.is_empty(), "missing aim cannot place");
    point(&mut app, Vec2::new(500.0, 400.0), Vec3::new(21.0, 0.0, -76.0));
    let root = app.world_mut().query_filtered::<Entity, With<panel::Root>>().single(app.world()).unwrap();
    app.world_mut().get_mut::<bevy::ui::RelativeCursorPosition>(root).unwrap().cursor_over = true;
    press(&mut app, KeyCode::F6);
    assert!(app.world().resource::<Document>().objects.is_empty(), "the menu covers the pointer");
    // Mouse buttons just select; an invalid placement attempt never arms one.
    mouse(&mut app, true);
    mouse(&mut app, false);
    assert!(app.world().resource::<Document>().objects.is_empty());
    press(&mut app, KeyCode::F6);
    let id = app.world().resource::<Selection>().0[0];
    let index = app.world().resource::<AuthoredIds>().0[&id];
    let at = [21.0, 0.0, 76.0];
    let unit = app.world().resource::<SimLink>().curr().unit(id).unwrap();
    assert_eq!(unit.pos.to_f32(), at);
    assert_eq!(unit.owner, sta_sim::world::NEUTRAL);

    let doc = app.world().resource::<Document>();
    let saved = sta_bzn::parse(&doc.bytes().unwrap()).unwrap();
    assert_eq!(&saved.objects[0].transform[9..], &at);
    assert_eq!(saved.objects[0].team, 0);
    assert_eq!(doc.object(index).unwrap().odf, "fx_cruiser");

    press(&mut app, KeyCode::Delete);
    assert!(app.world().resource::<SimLink>().curr().unit(id).is_none());
    assert!(app.world().resource::<Document>().object(index).is_none());

    let mut map = app.world().resource::<Document>().map.clone();
    map.objects.push(sta_bzn::MapObject { entity_id: i32::MAX, ..saved.objects[0].clone() });
    let bytes = sta_bzn::write::write(&map).unwrap();
    app.insert_resource(Document::new("bzn/test.bzn".into(), bytes, map).unwrap());
    let count = app.world().resource::<SimLink>().curr().units.len();
    press(&mut app, KeyCode::F6);
    assert_eq!(app.world().resource::<SimLink>().curr().units.len(), count,
        "a placement that cannot be saved must not appear in the world");
}

#[test]
fn two_completed_clicks_on_one_object_open_its_stats() {
    let (mut app, id) = app();
    hotkey(&mut app);
    app.world_mut().spawn((
        crate::sim_link::SimEntity(id),
        crate::render::units::SelectRadius(20.0),
        GlobalTransform::IDENTITY,
    ));
    let cursor = Vec2::new(540.0, 370.0);
    point(&mut app, cursor, Vec3::ZERO);
    mouse(&mut app, true);
    mouse(&mut app, false);
    assert_eq!(app.world().resource::<Selection>().0, [id]);
    assert!(app.world().resource::<Editing>().inspected.is_none());

    mouse(&mut app, true);
    assert!(app.world().resource::<Editing>().inspected.is_none());
    mouse(&mut app, false);
    assert_eq!(app.world().resource::<Editing>().inspected, Some(id));
    let close = app.world_mut().query::<&panel::Act>().iter(app.world())
        .any(|act| matches!(act, panel::Act::CloseStats));
    assert!(close, "the object stats panel opened");
    press(&mut app, KeyCode::Delete);
    assert!(app.world().resource::<SimLink>().curr().unit(id).is_some(), "stats holds editor tools");
    press(&mut app, KeyCode::Tab);
    assert!(app.world().resource::<Editing>().inspected.is_none());

    // A move starting from the object is never the second click of a double-click.
    mouse(&mut app, true);
    mouse(&mut app, false);
    mouse(&mut app, true);
    point(&mut app, cursor + Vec2::X * 65.0, Vec3::new(62.0, 0.0, 43.0));
    mouse(&mut app, false);
    assert!(app.world().resource::<Editing>().inspected.is_none());
    assert_eq!(app.world().resource::<SimLink>().curr().unit(id).unwrap().pos.to_f32(),
        [62.0, 0.0, -43.0]);
}
