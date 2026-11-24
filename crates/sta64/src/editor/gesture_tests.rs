use super::*;
use super::tests::{app, chord, hotkey, mouse, point, press};

fn pickable(app: &mut App, id: EntityId) -> Entity {
    app.world_mut().spawn((
        crate::sim_link::SimEntity(id),
        crate::render::units::SelectRadius(20.0),
        Transform::IDENTITY,
        GlobalTransform::IDENTITY,
    )).id()
}

#[test]
fn held_drag_previews_at_pointer_offset_then_commits_one_undoable_move() {
    let (mut app, id) = app();
    app.add_systems(Update, crate::render::units::place.in_set(GameSet::Interpolate));
    let drawn = pickable(&mut app, id);
    hotkey(&mut app);
    let cursor = Vec2::new(500.0, 400.0);
    point(&mut app, cursor, Vec3::new(10.0, 0.0, 0.0));
    mouse(&mut app, true);
    assert_eq!(app.world().resource::<Selection>().0, [id]);
    point(&mut app, cursor + Vec2::X * 4.0, Vec3::new(20.0, 0.0, -5.0));
    mouse(&mut app, true);
    assert!(app.world().resource::<Editing>().preview_position(id).is_none(), "small travel remains a click");
    point(&mut app, cursor + Vec2::X * 35.0, Vec3::new(42.0, 0.0, -5.0));
    mouse(&mut app, true);
    let expected = [32.0, 0.0, 5.0];
    assert_eq!(app.world().resource::<Editing>().preview_position(id).unwrap().to_f32(), expected);
    assert_eq!(app.world().get::<Transform>(drawn).unwrap().translation, Vec3::new(32.0, 0.0, -5.0));
    assert_eq!(app.world().resource::<SimLink>().real_curr().unit(id).unwrap().pos.to_f32(), [0.0; 3],
        "holding must not edit the simulation");
    mouse(&mut app, false);
    assert!(app.world().resource::<Editing>().preview_position(id).is_none());
    assert_eq!(app.world().resource::<SimLink>().real_curr().unit(id).unwrap().pos.to_f32(), expected);
    chord(&mut app, KeyCode::KeyZ);
    assert_eq!(app.world().resource::<SimLink>().real_curr().unit(id).unwrap().pos.to_f32(), [0.0; 3]);
    chord(&mut app, KeyCode::KeyZ);
    assert_eq!(app.world().resource::<Editing>().status, "Nothing to undo");
}

#[test]
fn panel_capture_cancels_held_preview_and_release() {
    let (mut app, id) = app();
    pickable(&mut app, id);
    hotkey(&mut app);
    let cursor = Vec2::new(500.0, 400.0);
    point(&mut app, cursor, Vec3::ZERO);
    mouse(&mut app, true);
    point(&mut app, cursor + Vec2::X * 35.0, Vec3::new(42.0, 0.0, -5.0));
    mouse(&mut app, true);
    assert!(app.world().resource::<Editing>().preview_position(id).is_some());
    let panel = app.world_mut().query_filtered::<Entity, With<panel::Root>>().single(app.world()).unwrap();
    app.world_mut().get_mut::<bevy::ui::RelativeCursorPosition>(panel).unwrap().cursor_over = true;
    app.update();
    assert!(app.world().resource::<Editing>().preview_position(id).is_none());
    mouse(&mut app, false);
    assert_eq!(app.world().resource::<SimLink>().real_curr().unit(id).unwrap().pos.to_f32(), [0.0; 3]);
}

#[test]
fn hud_capture_blocks_press_and_cancels_release() {
    let (mut app, id) = app();
    pickable(&mut app, id);
    app.insert_resource(crate::ui::hud::HudPointer(true));
    hotkey(&mut app);
    let cursor = Vec2::new(500.0, 400.0);
    point(&mut app, cursor, Vec3::ZERO);
    mouse(&mut app, true);
    assert!(app.world().resource::<Selection>().0.is_empty());
    mouse(&mut app, false);

    app.world_mut().resource_mut::<crate::ui::hud::HudPointer>().0 = false;
    mouse(&mut app, true);
    assert_eq!(app.world().resource::<Selection>().0, [id]);
    point(&mut app, cursor + Vec2::X * 35.0, Vec3::new(42.0, 0.0, -5.0));
    mouse(&mut app, true);
    assert!(app.world().resource::<Editing>().preview_position(id).is_some());
    app.world_mut().resource_mut::<crate::ui::hud::HudPointer>().0 = true;
    mouse(&mut app, false);
    assert!(app.world().resource::<Editing>().preview_position(id).is_none());
    assert_eq!(app.world().resource::<SimLink>().real_curr().unit(id).unwrap().pos.to_f32(), [0.0; 3]);
}

#[test]
fn escape_cancels_a_held_drag_without_a_move() {
    let (mut app, id) = app();
    pickable(&mut app, id);
    hotkey(&mut app);
    let cursor = Vec2::new(500.0, 400.0);
    point(&mut app, cursor, Vec3::ZERO);
    mouse(&mut app, true);
    point(&mut app, cursor + Vec2::X * 35.0, Vec3::new(42.0, 0.0, -5.0));
    mouse(&mut app, true);
    assert!(app.world().resource::<Editing>().preview_position(id).is_some());
    press(&mut app, KeyCode::Escape);
    assert!(app.world().resource::<Editing>().preview_position(id).is_none());
    mouse(&mut app, false);
    assert_eq!(app.world().resource::<SimLink>().real_curr().unit(id).unwrap().pos.to_f32(), [0.0; 3]);
}
