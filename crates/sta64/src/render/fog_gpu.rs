//! Optional real-GPU frame test of the fog and shroud layers' place in the frame (#1145): `STA64_GPU_TESTS=1` and
//! a hardware adapter; `STA64_GPU_OUT` keeps the PNGs outside the repository. Authored content only: an unlit
//! tall box on ground in sight, the ground behind it never explored, and a camera at a low angle.
//!
//! Rule under test (read from the executable, static; not measured): the layers are drawn under the scene, so
//! the box is never covered by them. The frame is drawn twice: as the map view draws it (the ground camera draws
//! the layers, the scene camera the box over its picture), and with the layers on the scene camera's own layer, as
//! they were drawn before the fix, which reproduces the report: from a low angle the box's lower part is covered
//! by the shroud of the ground behind it.
use super::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};

const SIZE: u32 = 256;
/// The authored colours of the two layers' art.
const FOG_ART: [u8; 3] = [0, 0, 200];
const SHROUD_ART: [u8; 3] = [200, 0, 200];
const GREEN: [u8; 3] = [0, 255, 0];

#[derive(Clone, Copy, PartialEq)]
enum Arrangement {
    /// The map view's: the layers on the ground layer, a ground camera under the scene camera.
    Ground,
    /// Before the fix: the layers on the scene camera's own layer.
    SameCamera,
}

fn shoot(app: &mut App, target: &Handle<Image>) -> image::RgbaImage {
    let out = Arc::new(Mutex::new(None::<Image>));
    let sink = Arc::clone(&out);
    app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| {
        *sink.lock().unwrap() = Some(shot.image.clone());
    });
    for _ in 0..240 {
        app.update();
        if out.lock().unwrap().is_some() {
            break;
        }
    }
    let image = out.lock().unwrap().clone().expect("a capture");
    image.try_into_dynamic().unwrap().to_rgba8()
}

/// Where a world point lands in the frame (the camera's pose and the default 45 degree view).
fn project(pose: &Transform, p: Vec3) -> (u32, u32) {
    let v = pose.to_matrix().inverse().transform_point3(p);
    let half = (std::f32::consts::FRAC_PI_4 * 0.5).tan();
    let (x, y) = (v.x / -v.z / half, v.y / -v.z / half);
    (((x * 0.5 + 0.5) * SIZE as f32) as u32, ((1.0 - (y * 0.5 + 0.5)) * SIZE as f32) as u32)
}

/// The authored scene and its frame, or `None` without a hardware adapter. The box stands at render (0, 200, -300),
/// 400 tall, on ground in sight (a sensor of 700 at the sim origin); the ground beyond it is never explored.
fn frame(name: &str, arrangement: Arrangement, tonemapping: Tonemapping, layers: bool) -> Option<(image::RgbaImage, Transform)> {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>(),
    );
    add_material(&mut app);
    app.insert_resource(ClearColor(Color::BLACK));
    let mut target_image = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
    target_image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(target_image);
    let pose = Transform::from_xyz(0.0, 120.0, 600.0).looking_to(Vec3::NEG_Z, Vec3::Y);
    match arrangement {
        Arrangement::Ground => {
            app.world_mut().spawn((
                Camera3d::default(),
                RenderLayers::layer(GROUND_LAYER),
                Camera { order: 0, ..default() },
                RenderTarget::from(target.clone()),
                Msaa::Off,
                tonemapping,
                pose,
            ));
            app.world_mut().spawn((
                Camera3d::default(),
                RenderLayers::layer(0),
                Camera { order: 1, clear_color: ClearColorConfig::None, ..default() },
                RenderTarget::from(target.clone()),
                Msaa::Off,
                tonemapping,
                pose,
            ));
        }
        Arrangement::SameCamera => {
            app.world_mut().spawn((Camera3d::default(), RenderLayers::layer(0), RenderTarget::from(target.clone()), Msaa::Off, tonemapping, pose));
        }
    }
    // The layers: the grid sees a sensor of 700 around the sim origin; the rest of the map was never in sight.
    let mut grid = FogGrid::new([-2000.0, -2000.0], [2000.0, 2000.0]);
    grid.set_sensor_for_test(Vec2::ZERO, 700.0);
    let grid = grid.lifted(false, false);
    for layer in [Layer::Fog, Layer::Shroud].into_iter().filter(|_| layers) {
        let (art, size) = pattern_image(None, "none", if layer == Layer::Fog { FOG_ART } else { SHROUD_ART });
        let (mask, art) = {
            let mut images = app.world_mut().resource_mut::<Assets<Image>>();
            (images.add(mask_image(&grid, layer)), images.add(art))
        };
        let parts = {
            let world = app.world_mut();
            let mut materials = world.remove_resource::<Assets<FogMaterial>>().unwrap();
            let mut meshes = world.remove_resource::<Assets<Mesh>>().unwrap();
            let parts = layer_parts(layer, &grid, mask, art, size, &mut meshes, &mut materials);
            let e = world.spawn(parts).id();
            world.insert_resource(materials);
            world.insert_resource(meshes);
            e
        };
        if arrangement == Arrangement::SameCamera {
            app.world_mut().entity_mut(parts).insert(RenderLayers::layer(0));
        }
    }
    let cube = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(100.0, 400.0, 100.0));
    let green = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial { base_color: Color::srgb(0.0, 1.0, 0.0), unlit: true, ..default() });
    app.world_mut().spawn((Mesh3d(cube), MeshMaterial3d(green), Transform::from_xyz(0.0, 200.0, -300.0)));
    app.finish();
    app.cleanup();
    for _ in 0..12 {
        app.update();
    }
    let adapter = app
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .cloned()
        .or_else(|| app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned()))?;
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP: only a software adapter is available ({})", adapter.name);
        return None;
    }
    eprintln!("adapter: {} ({:?})", adapter.name, adapter.device_type);
    let img = shoot(&mut app, &target);
    if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
        std::fs::create_dir_all(&dir).unwrap();
        img.save(std::path::Path::new(&dir).join(format!("fog-layers-{name}.png"))).unwrap();
    }
    Some((img, pose))
}

fn near(p: [u8; 4], want: [u8; 3]) -> bool {
    (0..3).all(|c| (i32::from(p[c]) - i32::from(want[c])).abs() <= 4)
}

#[test]
fn objects_cover_the_fog_and_shroud_layers_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let Some((under, pose)) = frame("under-the-scene", Arrangement::Ground, Tonemapping::None, true) else { return };
    let px = |img: &image::RgbaImage, p: Vec3| {
        let (x, y) = project(&pose, p);
        img.get_pixel(x.min(SIZE - 1), y.min(SIZE - 1)).0
    };
    // The box's front face: low (its ray meets never-explored ground far behind it) and high (above the horizon).
    let (low, high) = (Vec3::new(0.0, 60.0, -250.0), Vec3::new(0.0, 330.0, -250.0));
    // Ground far beside it, never explored: the shroud is drawn.
    let ground = Vec3::new(400.0, 0.0, -1500.0);
    eprintln!("under the scene: low {:?} high {:?} ground {:?}", px(&under, low), px(&under, high), px(&under, ground));
    assert!(near(px(&under, low), GREEN), "the box's lower part is not covered by the shroud behind it: {:?}", px(&under, low));
    assert!(near(px(&under, high), GREEN), "the box's upper part is not covered: {:?}", px(&under, high));
    assert!(near(px(&under, ground), SHROUD_ART), "the shroud layer is drawn over ground never explored: {:?}", px(&under, ground));
    // Before the fix (the layers on the scene camera's own layer, drawn over the box): the same frame shows the
    // report. This pins the cause; it is the picture that is wrong, not the test.
    let Some((over, _)) = frame("over-the-scene", Arrangement::SameCamera, Tonemapping::None, true) else { return };
    eprintln!("over the scene: low {:?} high {:?}", px(&over, low), px(&over, high));
    assert!(near(px(&over, low), SHROUD_ART), "before the fix the shroud covers the box's lower part: {:?}", px(&over, low));
    // The Remaster look's tone mapper: the box's pixels with the layers are those of the same frame without them
    // (the layers never touch the box), and the shroud still shows over the ground.
    let Some((with, _)) = frame("remaster-with-layers", Arrangement::Ground, Tonemapping::default(), true) else { return };
    let Some((without, _)) = frame("remaster-without-layers", Arrangement::Ground, Tonemapping::default(), false) else { return };
    for p in [low, high] {
        let (a, b) = (px(&with, p), px(&without, p));
        assert!((0..3).all(|c| (i32::from(a[c]) - i32::from(b[c])).abs() <= 2), "the box is as without the layers: {a:?} against {b:?}");
    }
    assert!(px(&with, ground) != px(&without, ground), "the layers draw over the ground");
}
