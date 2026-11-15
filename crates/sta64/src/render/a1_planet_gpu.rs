//! Opt-in, windowless frames of an Armada I campaign map's background planet from a camera pose swept in distance:
//! `STA64_GPU_TESTS=1`, `STA64_TEST_INSTALL` (an Armada I install), `STA64_TEST_MAP` (default `fed1`), `STA64_GPU_OUT`
//! (a directory for frames; nothing of the install is kept). It logs, per step, whether the planet's mesh entities are
//! in view (`ViewVisibility`) and what colour the screen shows at the planet's centre.
use super::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::primitives::Aabb;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};

const W: u32 = 1024;
const H: u32 = 512;

#[derive(Resource, Default)]
struct Pose {
    /// Eye and look-at target in Bevy space; `None` leaves the camera alone.
    eye: Option<(Vec3, Vec3)>,
}

#[derive(Resource)]
struct Target(Handle<Image>);

#[derive(Component)]
struct Done;

#[allow(clippy::type_complexity)]
fn retarget(mut commands: Commands, target: Res<Target>, mut cameras: Query<(Entity, &mut RenderTarget), (With<Camera3d>, Without<Done>)>) {
    for (e, mut rt) in &mut cameras {
        if matches!(*rt, RenderTarget::Window(_)) {
            *rt = RenderTarget::from(target.0.clone());
            commands.entity(e).insert(Done);
        }
    }
}

fn place(pose: Res<Pose>, mut camera: Query<(&mut Transform, &Camera), crate::map::MainCamera>) {
    let Some((eye, at)) = pose.eye else { return };
    if let Some(mut best) = camera.iter_mut().max_by_key(|(_, c)| c.order) {
        *best.0 = Transform::from_translation(eye).looking_at(at, Vec3::Y);
    }
}

#[test]
fn a1_background_planet_over_camera_distance() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 and STA64_TEST_INSTALL");
        return;
    }
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        eprintln!("SKIP: STA64_TEST_INSTALL names no install");
        return;
    };
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), None).expect("mount");
    let map_name = std::env::var("STA64_TEST_MAP").unwrap_or_else(|_| "fed1".into());
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .set(ImagePlugin::default_linear())
            .disable::<bevy::winit::WinitPlugin>(),
    );
    let info = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned();
    if let Some(info) = &info
        && crate::bench::frames::is_software_adapter(info.device_type, &info.name)
    {
        eprintln!("SKIP: only a software adapter is available ({})", info.name);
        return;
    }
    crate::app::configure_sets(&mut app);
    let rts = sta_content::layered::cfg_h(&vfs, "RTS_CFG.h");
    let table = sta_assets::load_sprite_table(&vfs);
    let maps = crate::map::browser(&vfs, Some(&map_name), "a1 planet frame test".into());
    assert!(maps.maps.iter().any(|m| m.eq_ignore_ascii_case(&map_name)), "{map_name} is not in the install");
    app.insert_resource(crate::clock::AnimClock::new(crate::clock::ClockMode::Bench { step: 1.0 / 30.0 }, 0))
        .add_plugins(crate::sim_link::SimLinkPlugin)
        .insert_resource(crate::render::sprites::SpriteTables(Arc::new(table)))
        .insert_resource(crate::viewer::TeamColours::load(&vfs))
        .insert_resource(crate::map::MapSeat(None))
        .insert_resource(crate::map::CameraLimits::from_cfg(rts.as_ref()))
        .insert_resource(crate::viewer::Mounted(vfs))
        .insert_resource(maps)
        .insert_resource(crate::sim_link::BenchDriven)
        .insert_resource(Pose::default());
    app.add_plugins(crate::map::MapPlugin)
        .add_plugins(crate::render::units::UnitsRenderPlugin)
        .add_plugins(crate::render::fx::FxMaterialsPlugin)
        .add_plugins(crate::render::hulls::HullPlugin)
        .add_plugins(crate::render::profile::ProfilePlugin);
    let mut target = Image::new_target_texture(W, H, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
    target.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(target);
    app.insert_resource(Target(target.clone()))
        .add_systems(Update, retarget)
        .add_systems(PostUpdate, place.before(TransformSystems::Propagate));
    app.finish();
    app.cleanup();
    for _ in 0..30 {
        app.update();
    }

    // The planet: the map's object at (5018, -737, 2277) in map space, mirrored in z for Bevy. Found by place, not by name.
    let want = Vec3::new(5018.0, -737.0, -2277.0);
    let planet = {
        let mut q = app.world_mut().query::<(Entity, &GlobalTransform)>();
        q.iter(app.world()).filter(|(_, g)| g.translation().distance(want) < 3.0).map(|(e, _)| e).collect::<Vec<_>>()
    };
    eprintln!("entities at the planet's place: {}", planet.len());
    assert!(!planet.is_empty(), "no entity at the planet's place");

    // The capture's pose: eye and focus, Bevy space.
    let focus = Vec3::new(4777.03, 38.5, -1657.41);
    let eye0 = Vec3::new(4803.99, 972.68, -815.16);
    let out_dir = std::env::var("STA64_GPU_OUT").ok();
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    for scale in [0.15f32, 0.3, 0.5, 0.7, 0.85, 1.0, 1.3, 1.8, 2.5] {
        let eye = focus + (eye0 - focus) * scale;
        app.world_mut().resource_mut::<Pose>().eye = Some((eye, focus));
        for _ in 0..10 {
            app.update();
        }
        // The planet's meshes: every descendant with a mesh.
        let mut seen = Vec::new();
        let mut stack = planet.clone();
        while let Some(e) = stack.pop() {
            if let Some(c) = app.world().get::<Children>(e) {
                stack.extend(c.iter());
            }
            let w = app.world();
            if w.get::<Mesh3d>(e).is_some() {
                let vis = w.get::<ViewVisibility>(e).map(|v| v.get());
                let inh = w.get::<InheritedVisibility>(e).map(|v| v.get());
                let aabb = w.get::<Aabb>(e).map(|a| (a.center, a.half_extents));
                let layers = w.get::<bevy::camera::visibility::RenderLayers>(e).cloned();
                seen.push(format!("view={vis:?} inherited={inh:?} aabb={aabb:?} layers={layers:?}"));
            }
        }
        let dist = (eye - focus).length();
        eprintln!("scale {scale} cam-distance {dist:.0} eye {eye:?}: {} mesh entities", seen.len());
        for s in seen.iter().take(6) {
            eprintln!("    {s}");
        }
        let before = frames.lock().unwrap().len();
        let out = Arc::clone(&frames);
        app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| out.lock().unwrap().push(shot.image.clone()));
        for _ in 0..60 {
            app.update();
            if frames.lock().unwrap().len() > before {
                break;
            }
        }
        assert_eq!(frames.lock().unwrap().len(), before + 1, "no capture at scale {scale}");
        let image = frames.lock().unwrap().last().unwrap().clone().try_into_dynamic().unwrap();
        if let Some(dir) = &out_dir {
            std::fs::create_dir_all(dir).unwrap();
            image.save(std::path::Path::new(dir).join(format!("fed1-scale{:03}.png", (scale * 100.0) as u32))).unwrap();
        }
    }
}
