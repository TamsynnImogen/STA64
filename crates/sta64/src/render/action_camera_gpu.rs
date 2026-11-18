//! Optional real-GPU regression for the action camera (UI-53): the picture shows the subject and
//! none of the overlay layer (rings, bars, grid), while the map camera shows both.
use super::*;
use bevy::camera::RenderTarget;
use bevy::log::tracing::{self, Event, Subscriber};
use bevy::log::tracing_subscriber::{layer::Context, Layer};
use bevy::log::{BoxedLayer, LogPlugin};
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};

/// Every `ERROR` event logged in this process (a render validation error is one).
static ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The two GPU tests share the process-wide log capture, so they run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

struct ErrorLog;

impl<S: Subscriber> Layer<S> for ErrorLog {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if *event.metadata().level() != tracing::Level::ERROR {
            return;
        }
        struct Msg(String);
        impl tracing::field::Visit for Msg {
            fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                self.0.push_str(&format!("{}={v:?} ", f.name()));
            }
        }
        let mut m = Msg(String::new());
        event.record(&mut m);
        if let Ok(mut e) = ERRORS.lock() {
            e.push(format!("{}: {}", event.metadata().target(), m.0));
        }
    }
}

fn error_layer(_: &mut App) -> Option<BoxedLayer> {
    Some(Box::new(ErrorLog))
}

fn count(image: &image::RgbaImage, f: impl Fn(&[u8; 4]) -> bool) -> usize {
    image.pixels().filter(|p| f(&p.0)).count()
}

fn shoot(app: &mut App, target: &Handle<Image>) -> image::RgbaImage {
    let out = Arc::new(Mutex::new(None::<Image>));
    let sink = Arc::clone(&out);
    app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| {
        *sink.lock().unwrap() = Some(shot.image.clone());
    });
    for _ in 0..60 {
        app.update();
        if out.lock().unwrap().is_some() {
            break;
        }
    }
    let image = out.lock().unwrap().clone().expect("a capture");
    let dynamic = image.try_into_dynamic().unwrap();
    if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
        std::fs::create_dir_all(&dir).unwrap();
        dynamic.save(std::path::Path::new(&dir).join(format!("action-{}.png", target.id().to_string().chars().take(8).collect::<String>()))).unwrap();
    }
    dynamic.to_rgba8()
}

#[test]
fn the_action_picture_shows_the_subject_and_leaves_the_overlay_layer_out_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(LogPlugin { level: bevy::log::Level::WARN, custom_layer: error_layer, ..default() })
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>(),
    );
    app.insert_resource(ClearColor(Color::BLACK)).insert_resource(SimEntities::default()).add_plugins((bevy::app::HierarchyPropagatePlugin::<RenderLayers>::new(PostUpdate), ActionCameraPlugin));

    let mut main_image = Image::new_target_texture(256, 192, TextureFormat::Rgba8UnormSrgb, None);
    main_image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let main_target = app.world_mut().resource_mut::<Assets<Image>>().add(main_image);
    // The map camera: far above, looking down; it draws the overlay layer too.
    app.world_mut().spawn((
        Camera3d::default(),
        RenderLayers::from_layers(&[0, OVERLAY_LAYER]),
        RenderTarget::from(main_target.clone()),
        Msaa::Off,
        Transform::from_xyz(0.0, 400.0, 0.0).looking_to(Vec3::NEG_Y, Vec3::NEG_Z),
    ));
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(20.0, 20.0, 20.0));
    let flat = |c: Color| StandardMaterial { base_color: c, unlit: true, ..default() };
    let magenta = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(flat(Color::srgb(1.0, 0.0, 1.0)));
    let green = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(flat(Color::srgb(0.0, 1.0, 0.0)));
    // The subject, and an overlay piece on the ground next to it, as a selection ring is.
    let unit = app.world_mut().spawn((Mesh3d(mesh.clone()), MeshMaterial3d(magenta), Transform::from_xyz(0.0, 0.0, 0.0), SelectRadius(14.0))).id();
    app.world_mut().spawn((
        Overlay { unit: EntityId(1), kind: crate::render::overlays::OverlayKind::Ring },
        Mesh3d(mesh),
        MeshMaterial3d(green),
        Transform::from_xyz(0.0, 0.0, 0.0).with_scale(Vec3::new(3.0, 0.2, 3.0)),
    ));
    app.world_mut().resource_mut::<SimEntities>().0.insert(EntityId(1), unit);
    app.world_mut().resource_mut::<ActionCandidates>().idle = vec![EntityId(1)];
    // The map camera must satisfy `MainCamera`; the node that asks for the picture has a size.
    app.world_mut().spawn((ActionPicture, InheritedVisibility::VISIBLE, ComputedNode { size: Vec2::new(226.0, 160.0), ..default() }));

    app.finish();
    app.cleanup();
    for _ in 0..12 {
        app.update();
    }
    let adapter = app
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .cloned()
        .or_else(|| app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned()));
    let Some(adapter) = adapter else {
        eprintln!("SKIP: no render adapter available");
        return;
    };
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP: only a software adapter is available ({})", adapter.name);
        return;
    }
    let view = app.world().resource::<ActionView>().image.clone();
    let is_magenta = |p: &[u8; 4]| p[0] > 100 && p[2] > 100 && (p[1] as u16) * 2 < p[0] as u16;
    let is_green = |p: &[u8; 4]| p[1] > 100 && p[1] > p[0].saturating_mul(2) && p[1] > p[2].saturating_mul(2);
    let main = shoot(&mut app, &main_target);
    assert!(count(&main, is_magenta) > 50, "the map camera sees the subject");
    assert!(count(&main, is_green) > 0, "the map camera draws the overlay layer");
    let action = shoot(&mut app, &view);
    assert_eq!((action.width(), action.height()), (226, 160), "the picture follows the node");
    let seen = count(&action, is_magenta);
    assert!(seen > 400, "the subject is framed in the picture ({seen} px)");
    assert_eq!(count(&action, is_green), 0, "overlay pieces are not in the picture");
}

fn set_visible(app: &mut App, picture: Entity, visible: bool) {
    app.world_mut().entity_mut(picture).insert(if visible { InheritedVisibility::VISIBLE } else { InheritedVisibility::HIDDEN });
}

fn set_node_size(app: &mut App, picture: Entity, size: Vec2) {
    app.world_mut().entity_mut(picture).insert(ComputedNode { size, ..default() });
}

/// A resized picture (the node changed size) and a hidden then shown one (a cineractive) render
/// without a render error: resizing the target must not ask the GPU to copy the old texture.
#[test]
fn the_action_picture_survives_a_resize_and_a_hide_and_show_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(LogPlugin { level: bevy::log::Level::WARN, custom_layer: error_layer, ..default() })
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>(),
    );
    app.insert_resource(ClearColor(Color::BLACK)).insert_resource(SimEntities::default()).add_plugins((bevy::app::HierarchyPropagatePlugin::<RenderLayers>::new(PostUpdate), ActionCameraPlugin));
    app.world_mut().spawn((Camera3d::default(), Camera { is_active: false, ..default() }, Transform::from_xyz(0.0, 400.0, 0.0).looking_to(Vec3::NEG_Y, Vec3::NEG_Z)));
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(20.0, 20.0, 20.0));
    let magenta = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial { base_color: Color::srgb(1.0, 0.0, 1.0), unlit: true, ..default() });
    let unit = app.world_mut().spawn((Mesh3d(mesh), MeshMaterial3d(magenta), Transform::default(), SelectRadius(14.0))).id();
    app.world_mut().resource_mut::<SimEntities>().0.insert(EntityId(1), unit);
    app.world_mut().resource_mut::<ActionCandidates>().idle = vec![EntityId(1)];
    let picture = app.world_mut().spawn((ActionPicture, InheritedVisibility::VISIBLE, ComputedNode { size: Vec2::new(226.0, 160.0), ..default() })).id();
    app.finish();
    app.cleanup();
    // Whatever the log plugin said while building (a second global logger is refused) is not a render error.
    ERRORS.lock().unwrap().clear();
    for _ in 0..12 {
        app.update();
    }
    let adapter = app
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .cloned()
        .or_else(|| app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned()));
    let Some(adapter) = adapter else {
        eprintln!("SKIP: no render adapter available");
        return;
    };
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP: only a software adapter is available ({})", adapter.name);
        return;
    }
    eprintln!("adapter: {}", adapter.name);
    let view = app.world().resource::<ActionView>().image.clone();
    let size_of = |app: &App| app.world().resource::<Assets<Image>>().get(&view).map(|i| (i.width(), i.height()));
    assert_eq!(size_of(&app), Some((226, 160)));
    // The node changes size.
    set_node_size(&mut app, picture, Vec2::new(300.0, 120.0));
    for _ in 0..8 {
        app.update();
    }
    assert_eq!(size_of(&app), Some((300, 120)), "the picture follows the node");
    // A cineractive hides the HUD; the picture comes back, and at another size.
    let cameras = |app: &mut App| -> Vec<(Entity, bool)> {
        let mut q = app.world_mut().query_filtered::<(Entity, &Camera), Or<(With<ActionCamera>, With<ActionSky>)>>();
        let mut v: Vec<_> = q.iter(app.world()).map(|(e, c)| (e, c.is_active)).collect();
        v.sort();
        v
    };
    let shown = cameras(&mut app);
    assert_eq!(shown.len(), 2, "a scene and a sky camera");
    assert!(shown.iter().all(|c| c.1), "on while the picture is shown");
    set_visible(&mut app, picture, false);
    for _ in 0..4 {
        app.update();
    }
    let hidden = cameras(&mut app);
    assert_eq!(hidden.iter().map(|c| c.0).collect::<Vec<_>>(), shown.iter().map(|c| c.0).collect::<Vec<_>>(), "the cameras are kept while the picture is hidden");
    assert!(hidden.iter().all(|c| !c.1), "and switched off");
    set_visible(&mut app, picture, true);
    set_node_size(&mut app, picture, Vec2::new(200.0, 140.0));
    for _ in 0..8 {
        app.update();
    }
    assert_eq!(size_of(&app), Some((200, 140)), "the picture follows the node after it reappears");
    assert_eq!(cameras(&mut app), shown, "the same cameras, on again");
    let errors = ERRORS.lock().unwrap().clone();
    assert!(errors.is_empty(), "render ERROR events:\n{}", errors.join("\n"));
}
