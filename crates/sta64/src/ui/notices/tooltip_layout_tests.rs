use super::*;
use bevy::app::{HierarchyPropagatePlugin, PropagateSet};
use bevy::camera::{ComputedCameraValues, RenderTargetInfo, Viewport};
use bevy::text::{FontCx, RemSize, ScaleCx, TextPipeline};
use bevy::transform::StaticTransformOptimizations;
use bevy::ui::{
    ComputedUiRenderTargetInfo, ComputedUiTargetCamera, UiSystems, ui_layout_system,
    ui_surface::UiSurface, update::propagate_ui_target_cameras, update::update_clipping_system,
};

// Bevy's own layout tests use the same public systems, hierarchy propagation,
// resources, and dummy camera target. No Winit window or renderer is started.
fn layout_rig() -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(HierarchyPropagatePlugin::<ComputedUiTargetCamera>::new(
        PostUpdate,
    ));
    app.add_plugins(HierarchyPropagatePlugin::<ComputedUiRenderTargetInfo>::new(
        PostUpdate,
    ));
    app.init_resource::<UiScale>();
    app.init_resource::<UiSurface>();
    app.init_resource::<TextPipeline>();
    app.init_resource::<FontCx>();
    app.init_resource::<RemSize>();
    app.init_resource::<ScaleCx>();
    app.init_resource::<StaticTransformOptimizations>();
    app.configure_sets(
        PostUpdate,
        (
            UiSystems::Prepare,
            UiSystems::Propagate,
            UiSystems::Content,
            UiSystems::Layout,
            UiSystems::PostLayout,
        )
            .chain(),
    );
    app.configure_sets(
        PostUpdate,
        PropagateSet::<ComputedUiTargetCamera>::default()
            .after(propagate_ui_target_cameras)
            .before(ui_layout_system),
    );
    app.configure_sets(
        PostUpdate,
        PropagateSet::<ComputedUiRenderTargetInfo>::default()
            .after(propagate_ui_target_cameras)
            .before(ui_layout_system),
    );
    app.add_systems(
        PostUpdate,
        propagate_ui_target_cameras.in_set(UiSystems::Prepare),
    );
    app.add_systems(PostUpdate, ui_layout_system.in_set(UiSystems::Layout));
    app.add_systems(
        PostUpdate,
        update_clipping_system.in_set(UiSystems::PostLayout),
    );
    crate::app::configure_sets(&mut app);
    app.add_plugins(NoticePlugin);

    let camera = app
        .world_mut()
        .spawn((
            Camera2d,
            Camera {
                computed: ComputedCameraValues {
                    target_info: Some(RenderTargetInfo {
                        physical_size: UVec2::new(800, 600),
                        scale_factor: 1.0,
                    }),
                    ..default()
                },
                viewport: Some(Viewport {
                    physical_size: UVec2::new(800, 600),
                    ..default()
                }),
                ..default()
            },
        ))
        .id();
    let mut window = Window::default();
    window.resolution.set(800.0, 600.0);
    window.set_cursor_position(Some(Vec2::new(780.0, 570.0)));
    app.world_mut().spawn((window, PrimaryWindow));
    (app, camera)
}

fn tip(app: &mut App) -> (Entity, Vec2, Vec2) {
    let (entity, node, measured, global) = app
        .world_mut()
        .query_filtered::<(Entity, &Node, &ComputedNode, &UiGlobalTransform), With<TipBox>>()
        .single(app.world())
        .unwrap();
    assert_eq!(node.display, Display::Flex);
    (
        entity,
        measured.size() * measured.inverse_scale_factor(),
        global.translation - measured.size() * 0.5,
    )
}

fn assert_at_pointer(app: &mut App) -> (Entity, Vec2) {
    let (entity, _, physical_at) = tip(app);
    let (pointer, window_size) = {
        let mut query = app
            .world_mut()
            .query_filtered::<&Window, With<PrimaryWindow>>();
        let window = query.single(app.world()).unwrap();
        (
            window.cursor_position().unwrap(),
            Vec2::new(window.width(), window.height()),
        )
    };
    let node = app.world().entity(entity).get::<Node>().unwrap();
    let measured = app.world().entity(entity).get::<ComputedNode>().unwrap();
    let k = app.world().resource::<UiScale>().0;
    let wanted = place_tip(pointer, tip_size(measured, k), window_size, k);
    let physical_wanted = (wanted / (measured.inverse_scale_factor() * k)).round();
    assert!(
        (physical_at - physical_wanted).abs().max_element() < 0.02,
        "physical {physical_at:?} wanted {physical_wanted:?}"
    );
    assert_eq!(
        (node.left, node.top),
        (Val::Px(wanted.x / k), Val::Px(wanted.y / k))
    );
    (entity, physical_at)
}

#[test]
fn fresh_layout_positions_entire_tooltip_subtree_on_first_show_growth_resize_and_scale() {
    let (mut app, camera) = layout_rig();
    app.update();
    let box_entity = app
        .world_mut()
        .query_filtered::<Entity, With<TipBox>>()
        .single(app.world())
        .unwrap();
    let text_entity = app
        .world_mut()
        .query_filtered::<Entity, With<TipText>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(text_entity)
        .get_mut::<Node>()
        .unwrap()
        .width = Val::Px(120.0);
    app.world_mut()
        .entity_mut(text_entity)
        .get_mut::<Node>()
        .unwrap()
        .height = Val::Px(20.0);
    let glyph = app
        .world_mut()
        .spawn(Node {
            width: Val::Px(18.0),
            height: Val::Px(8.0),
            margin: UiRect::left(Val::Px(7.0)),
            ..default()
        })
        .id();
    let nested = app
        .world_mut()
        .spawn(Node {
            width: Val::Px(6.0),
            height: Val::Px(4.0),
            margin: UiRect::left(Val::Px(3.0)),
            ..default()
        })
        .id();
    app.world_mut().entity_mut(text_entity).add_child(glyph);
    app.world_mut().entity_mut(glyph).add_child(nested);
    let target = app
        .world_mut()
        .spawn(Tooltip::new("short").verbose("tall"))
        .id();
    *app.world_mut().resource_mut::<TipHover>() = TipHover {
        target: Some(target),
        rest: TIP_SHORT_SECS,
    };
    app.update();
    let (_, first) = assert_at_pointer(&mut app);
    let first_size = app
        .world()
        .entity(box_entity)
        .get::<ComputedNode>()
        .unwrap()
        .size();
    assert!(
        first_size.x > 120.0 && first_size.y > 20.0,
        "authored child was not laid out: {first_size:?}"
    );
    let offsets = |app: &App| {
        let root = app
            .world()
            .entity(box_entity)
            .get::<UiGlobalTransform>()
            .unwrap()
            .translation;
        [text_entity, glyph, nested].map(|entity| {
            app.world()
                .entity(entity)
                .get::<UiGlobalTransform>()
                .unwrap()
                .translation
                - root
        })
    };
    let before_offsets = offsets(&app);

    {
        let mut query = app
            .world_mut()
            .query_filtered::<&mut Window, With<PrimaryWindow>>();
        query
            .single_mut(app.world_mut())
            .unwrap()
            .set_cursor_position(Some(Vec2::new(30.0, 30.0)));
    }
    app.update();
    let (_, moved) = assert_at_pointer(&mut app);
    assert!(
        (moved - first).abs().max_element() > 100.0,
        "pointer move did not reposition tooltip"
    );
    for (before, after) in before_offsets.into_iter().zip(offsets(&app)) {
        assert!(
            (before - after).abs().max_element() < 0.02,
            "subtree changed shape during translation: {before:?} -> {after:?}"
        );
    }
    {
        let mut query = app
            .world_mut()
            .query_filtered::<&mut Window, With<PrimaryWindow>>();
        query
            .single_mut(app.world_mut())
            .unwrap()
            .set_cursor_position(Some(Vec2::new(780.0, 570.0)));
    }
    app.update();
    assert_at_pointer(&mut app);

    // Same-frame growth moves the tooltip above the pointer using the new height.
    app.world_mut()
        .entity_mut(text_entity)
        .get_mut::<Node>()
        .unwrap()
        .height = Val::Px(180.0);
    app.world_mut().resource_mut::<TipHover>().rest = TIP_VERBOSE_SECS;
    app.update();
    let (_, grown) = assert_at_pointer(&mut app);
    let grown_size = app
        .world()
        .entity(box_entity)
        .get::<ComputedNode>()
        .unwrap()
        .size();
    assert!(
        grown_size.y > first_size.y + 150.0,
        "growth was not measured: {first_size:?} -> {grown_size:?}"
    );
    assert!(
        grown.y < first.y - 150.0,
        "old tooltip height was used: {first:?} -> {grown:?}"
    );
    assert!((offsets(&app)[0].x - before_offsets[0].x).abs() < 0.02);

    // A resized render target and logical window must take effect in this update.
    {
        let mut camera = app.world_mut().entity_mut(camera);
        let mut c = camera.get_mut::<Camera>().unwrap();
        c.computed.target_info.as_mut().unwrap().physical_size = UVec2::new(1000, 700);
        c.viewport.as_mut().unwrap().physical_size = UVec2::new(1000, 700);
    }
    app.world_mut()
        .query_filtered::<&mut Window, With<PrimaryWindow>>()
        .single_mut(app.world_mut())
        .unwrap()
        .resolution
        .set(1000.0, 700.0);
    app.update();
    assert_at_pointer(&mut app);

    // The camera's target scale and UiScale together turn UI units into pixels.
    {
        let mut camera = app.world_mut().entity_mut(camera);
        let mut c = camera.get_mut::<Camera>().unwrap();
        let info = c.computed.target_info.as_mut().unwrap();
        info.scale_factor = 2.0;
        info.physical_size = UVec2::new(2000, 1400);
        c.viewport.as_mut().unwrap().physical_size = UVec2::new(2000, 1400);
    }
    app.world_mut().resource_mut::<UiScale>().0 = 1.25;
    {
        let mut query = app
            .world_mut()
            .query_filtered::<&mut Window, With<PrimaryWindow>>();
        let mut w = query.single_mut(app.world_mut()).unwrap();
        w.resolution.set_scale_factor_override(Some(2.0));
        w.resolution.set(1000.0, 700.0);
    }
    app.update();
    assert_at_pointer(&mut app);

    *app.world_mut().resource_mut::<TipHover>() = TipHover::default();
    app.update();
    assert_eq!(
        app.world()
            .entity(box_entity)
            .get::<Node>()
            .unwrap()
            .display,
        Display::None
    );
    *app.world_mut().resource_mut::<TipHover>() = TipHover {
        target: Some(target),
        rest: TIP_SHORT_SECS,
    };
    app.update();
    let (_, shown_again) = assert_at_pointer(&mut app);
    let shown_offsets = offsets(&app);
    app.update();
    let (_, unchanged) = assert_at_pointer(&mut app);
    assert!(
        (shown_again - unchanged).abs().max_element() < 0.02,
        "stationary tooltip drifted"
    );
    for (before, after) in shown_offsets.into_iter().zip(offsets(&app)) {
        assert!(
            (before - after).abs().max_element() < 0.02,
            "descendant drifted: {before:?} -> {after:?}"
        );
    }
}
