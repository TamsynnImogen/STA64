//! Optional real-GPU frame tests of the objects out of sight that are not planets (#1146): a hull mesh and a rock
//! mesh under a gate that is out of sight (the nebula's gas has its own frame test, `nebula_ghost_gpu.rs`). `STA64_GPU_TESTS=1` and a hardware adapter;
//! `STA64_GPU_OUT` keeps the PNGs outside the repository. Everything is authored: an ico sphere, one light, no
//! textures, invented lighting keys. In sight each is as without a gate (every pixel); out of sight it is dimmer,
//! by the mesh tag's factor or, for a rock given the game's lighting keys, by those keys.
use super::{GhostLight, OutOfSight, tag_ghost_meshes};
use crate::render::hulls::{HullMaterial, HullPart, HullPlugin, HullView, hull_material};
use crate::render::rock::{RockMaterial, RockParams, with_ghost_keys};
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};

const SIZE: u32 = 96;

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

/// An app with a camera on a render target, one light and the system that tags meshes under a gate out of sight.
fn scene() -> (App, Handle<Image>) {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    app.insert_resource(ClearColor(Color::BLACK)).add_systems(Update, tag_ghost_meshes);
    let mut image = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        Tonemapping::None,
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    app.world_mut().spawn((DirectionalLight { illuminance: 6000.0, shadow_maps_enabled: false, ..default() }, Transform::IDENTITY.looking_to(Vec3::NEG_Z, Vec3::Y)));
    (app, target)
}

/// The frame, or `None` without a hardware adapter.
fn capture(mut app: App, target: &Handle<Image>, name: &str) -> Option<image::RgbaImage> {
    app.finish();
    app.cleanup();
    for _ in 0..12 {
        app.update();
    }
    snap(&mut app, target, name)
}

/// The frame of an app that has finished building and run a few frames.
fn snap(app: &mut App, target: &Handle<Image>, name: &str) -> Option<image::RgbaImage> {
    let adapter = app
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .cloned()
        .or_else(|| app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned()))?;
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP: only a software adapter is available ({})", adapter.name);
        return None;
    }
    let img = shoot(app, target);
    if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
        std::fs::create_dir_all(&dir).unwrap();
        img.save(std::path::Path::new(&dir).join(format!("ghost-{name}.png"))).unwrap();
    }
    Some(img)
}

/// The mean of the red, green and blue values over the middle of the frame, where the sphere is.
fn mean(img: &image::RgbaImage) -> f32 {
    let (lo, hi) = (SIZE / 3, SIZE * 2 / 3);
    let mut sum = 0.0;
    for y in lo..hi {
        for x in lo..hi {
            let p = img.get_pixel(x, y).0;
            sum += f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2]);
        }
    }
    sum / (3.0 * ((hi - lo) * (hi - lo)) as f32)
}

fn enabled() -> bool {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return false;
    }
    true
}

/// Puts the sphere under a gate (`None`: no gate at all, as before this change); `out` adds the out-of-sight
/// marker with the light factor.
fn gate(app: &mut App, gated: bool, out: Option<f32>) -> Option<Entity> {
    if !gated {
        return None;
    }
    let gate = app.world_mut().spawn((Transform::IDENTITY, Visibility::Inherited)).id();
    if let Some(factor) = out {
        app.world_mut().entity_mut(gate).insert((OutOfSight, GhostLight(factor)));
    }
    Some(gate)
}

fn hull_frame(gated: bool, out: Option<f32>) -> Option<image::RgbaImage> {
    let (mut app, target) = scene();
    app.add_plugins(HullPlugin);
    app.insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 100.0, ..default() });
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(1.0).mesh().ico(4).unwrap());
    let source = StandardMaterial { base_color: Color::srgb(0.8, 0.8, 0.8), ..default() };
    let material = app.world_mut().resource_mut::<Assets<HullMaterial>>().add(hull_material(&source, &HullPart::default(), &HullView::default()));
    let g = gate(&mut app, gated, out);
    let mut part = app.world_mut().spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform::IDENTITY));
    if let Some(g) = g {
        part.insert(ChildOf(g));
    }
    capture(app, &target, &format!("hull-{gated}-{out:?}"))
}

#[test]
fn a_hull_mesh_out_of_sight_is_dimmed_by_its_tag_and_in_sight_is_as_before_on_gpu() {
    if !enabled() {
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let Some(ungated) = hull_frame(false, None) else { return };
    let Some(live) = hull_frame(true, None) else { return };
    let factor = crate::render::fog_objects::ghost::planet_linear();
    let Some(ghost) = hull_frame(true, Some(factor)) else { return };
    let (u, l, g) = (mean(&ungated), mean(&live), mean(&ghost));
    eprintln!("hull: no gate {u:.2}, in sight {l:.2}, out of sight (factor {factor:.4}) {g:.2}, ratio {:.3}", g / l);
    assert!(l > 20.0, "the hull is lit: {l}");
    assert_eq!(ungated.as_raw(), live.as_raw(), "in sight, every pixel as before");
    assert!(g < l * 0.4 && g > 0.0, "out of sight is dimmer by the tag's factor: {g} against {l}");
}

/// A rock sphere with a key light along -Z and an authored ambient; `keys`: the lighting keys given to the material.
fn rock_frame(gated: bool, out: Option<f32>, keys: Option<sta_content::GhostLighting>) -> Option<image::RgbaImage> {
    let (mut app, target) = scene();
    embedded_asset!(app, "rock.wesl");
    app.add_plugins(MaterialPlugin::<RockMaterial>::default());
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(1.0).mesh().ico(4).unwrap());
    let params = RockParams {
        amb: Vec4::new(0.05, 0.05, 0.07, 1.0),
        shine: Vec4::new(0.7, 24.0, 3.0, 1.1),
        rim: Vec4::new(0.12, 0.14, 0.2, 0.0),
        ddir0: Vec4::new(0.0, 0.0, -1.0, 1.0),
        dcol0: Vec4::new(1.0, 1.0, 1.0, 0.0),
        ddir1: Vec4::new(0.0, 1.0, 0.0, 0.0),
        ext: Vec4::new(1.0, 1.0, 1.0, 1.0),
        misc: Vec4::new(0.0, 0.0, 0.0, 1.0),
        ..default()
    };
    let params = with_ghost_keys(params, keys.as_ref());
    let material = app.world_mut().resource_mut::<Assets<RockMaterial>>().add(RockMaterial { params, albedo: None, relief: None });
    let g = gate(&mut app, gated, out);
    let mut part = app.world_mut().spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform::IDENTITY));
    if let Some(g) = g {
        part.insert(ChildOf(g));
    }
    capture(app, &target, &format!("rock-{gated}-{out:?}-{}", keys.is_some()))
}

#[test]
fn a_rock_mesh_out_of_sight_is_dimmed_by_its_tag_or_lit_by_the_keys_and_in_sight_is_as_before_on_gpu() {
    if !enabled() {
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let interim = crate::render::fog_objects::ghost::INTERIM_LINEAR;
    // Authored keys (invented values, not the game's).
    let keys = |a: f32, d: f32| sta_content::GhostLighting { ambient: Some([a; 3]), diffuse: Some([d; 3]), specular: Some([0.0; 3]), team_factor: None };
    let Some(ungated) = rock_frame(false, None, None) else { return };
    let Some(live) = rock_frame(true, None, None) else { return };
    let Some(tag_only) = rock_frame(true, Some(interim), None) else { return };
    let Some(lit_a) = rock_frame(true, Some(interim), Some(keys(0.2, 0.4))) else { return };
    let Some(lit_b) = rock_frame(true, Some(interim), Some(keys(0.1, 0.2))) else { return };
    // In sight with the keys in the material: untouched (the shader reads them only under a tag).
    let Some(live_keys) = rock_frame(true, None, Some(keys(0.2, 0.4))) else { return };
    let (l, t, a, b) = (mean(&live), mean(&tag_only), mean(&lit_a), mean(&lit_b));
    eprintln!("rock: in sight {l:.2}, out of sight by the tag (factor {interim}) {t:.2} (ratio {:.3}), by authored keys 0.2/0.4 {a:.2} (ratio {:.3}), by authored keys 0.1/0.2 {b:.2} (ratio {:.3})", t / l, a / l, b / l);
    assert!(l > 20.0, "the rock is lit: {l}");
    assert_eq!(ungated.as_raw(), live.as_raw(), "in sight, every pixel as before");
    assert_eq!(live_keys.as_raw(), live.as_raw(), "in sight the keys change nothing");
    assert!(t < l * 0.8 && t > l * 0.4, "out of sight without the keys the tag's factor dims it: {t} against {l}");
    assert!(a != t && b != t, "the keys replace the tag's factor");
    assert!(b < a, "the lighting follows the keys: {b} against {a}");
    assert!(a < l, "a copy is dimmer than the rock in sight");
}

