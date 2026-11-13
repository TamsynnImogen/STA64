//! Opt-in GPU test (`STA64_GPU_TESTS=1`, a hardware adapter): a ship inside a nebula
//! darkens the gas behind it (RND-15).
//!
//! Two frames of the same gas field lit from above, the ship (on a layer the camera does
//! not draw, so only its shadow shows) hidden in the first and in place in the second:
//! the gas in the ship's shadow column is darker and the gas outside it is the same.
//! Fails on any `ERROR` event (a shader or pipeline that does not build).
//! `STA64_GPU_OUT=<dir>` keeps the frames as PNG.

use std::sync::Mutex;

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::log::tracing::{self, Event, Subscriber};
use bevy::log::tracing_subscriber::{layer::Context, Layer};
use bevy::log::{BoxedLayer, LogPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::RenderPlugin;
use bevy::window::ExitCondition;

use super::hulls::{HullPart, HullPlugin};
use super::nebula::{apply_shadow, NebulaMaterial, NebulaParams, NebulaPlugin};
use super::shadows::{KeyShadow, ShadowPlugin, ShadowSet};

static ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static FRAMES: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());

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

const W: u32 = 160;
const H: u32 = 120;

fn capture(app: &mut App, target: &Handle<Image>, name: &str) -> Vec<u8> {
    let before = FRAMES.lock().unwrap().len();
    let owned = name.to_owned();
    app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |c: On<ScreenshotCaptured>| {
        FRAMES.lock().unwrap().push((owned.clone(), c.image.data.clone().unwrap_or_default()));
    });
    for _ in 0..60 {
        app.update();
        if FRAMES.lock().unwrap().len() > before {
            return FRAMES.lock().unwrap().last().unwrap().1.clone();
        }
    }
    panic!("no frame came back for {name}");
}

fn save(name: &str, data: &[u8]) {
    let Ok(dir) = std::env::var("STA64_GPU_OUT") else { return };
    let _ = std::fs::create_dir_all(&dir);
    let img = Image::new(Extent3d { width: W, height: H, depth_or_array_layers: 1 }, TextureDimension::D2, data.to_vec(), TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
    if let Ok(d) = img.try_into_dynamic() {
        let _ = d.to_rgb8().save(format!("{dir}/{name}.png"));
    }
}

/// Mean luminance over the columns `x0..x1`, all rows.
fn mean(frame: &[u8], x0: u32, x1: u32) -> f32 {
    let (mut sum, mut n) = (0.0, 0.0);
    for y in 0..H {
        for x in x0..x1 {
            let p = &frame[((y * W + x) * 4) as usize..][..3];
            sum += 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
            n += 1.0;
        }
    }
    sum / n
}

/// A uniform field of gas: the noise and envelope textures are the fallback (all ones),
/// which is dense gas everywhere inside the box.
fn gas(eye: Vec3, lo: Vec3, hi: Vec3) -> NebulaParams {
    NebulaParams {
        eye: eye.extend(10.0),
        gas_a: Vec4::new(0.55, 0.75, 1.0, 0.08),
        gas_b: Vec4::new(0.55, 0.75, 1.0, 0.08),
        glow: Vec4::new(0.1, 0.1, 0.1, 1.0),
        noise: Vec4::new(1.0 / 128.0, 0.0, 0.3, 0.5),
        vert: Vec4::new(0.0, 400.0, 0.0, 1.0 / 300.0),
        opac: Vec4::new(0.0008, 0.0, 0.0, 0.0),
        key: Vec3::Y.extend(1.0),
        scat: Vec4::new(1.0, 0.0, 0.0, 0.0),
        box_lo: lo.extend(1.0),
        box_hi: hi.extend(0.0),
        cam_fwd: Vec4::NEG_Z,
        env: Vec4::new(-1000.0, -1000.0, 1.0 / 2000.0, 1.0 / 2000.0),
        ..NebulaParams::default()
    }
}

#[test]
fn a_ship_inside_a_nebula_darkens_the_gas_behind_it() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU shadow test");
        return;
    }
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .set(LogPlugin { level: bevy::log::Level::WARN, custom_layer: error_layer, ..default() })
            .disable::<bevy::winit::WinitPlugin>(),
    );
    app.add_plugins((HullPlugin, NebulaPlugin, ShadowPlugin));
    // The test's gas is one material, not a `NebulaField`: it takes the map the way a field does.
    app.add_systems(
        PostUpdate,
        (|key: Res<KeyShadow>, mut materials: ResMut<Assets<NebulaMaterial>>| {
            for (_, m) in materials.iter_mut() {
                apply_shadow(&mut m.params, Some(&key));
                if m.shadow.as_ref() != Some(&key.image) {
                    m.shadow = Some(key.image.clone());
                }
            }
        })
        .after(ShadowSet),
    );
    app.insert_resource(ClearColor(Color::BLACK)).init_resource::<crate::clock::AnimClock>();

    let mut img = Image::new_target_texture(W, H, TextureFormat::Rgba8UnormSrgb, None);
    img.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(img);
    // The Key shines from above.
    app.world_mut().spawn((DirectionalLight { illuminance: 6000.0, ..default() }, Transform::from_xyz(0.0, 10.0, 0.0).looking_at(Vec3::ZERO, Vec3::Z)));
    let eye = Vec3::new(0.0, 100.0, 700.0);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        Projection::Perspective(PerspectiveProjection { near: 1.0, far: 5000.0, ..default() }),
        Transform::from_translation(eye).looking_at(Vec3::new(0.0, 100.0, 0.0), Vec3::Y),
    ));
    let (lo, hi) = (Vec3::new(-400.0, -200.0, -300.0), Vec3::new(400.0, 200.0, 300.0));
    let cube = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(1.0, 1.0, 1.0));
    let nebula = app.world_mut().resource_mut::<Assets<NebulaMaterial>>().add(NebulaMaterial { params: gas(eye, lo, hi), noise: None, envelope: None, shadow: None });
    app.world_mut().spawn((Mesh3d(cube.clone()), MeshMaterial3d(nebula), Transform::from_translation((lo + hi) * 0.5).with_scale(hi - lo)));
    // A ship above the gas, on a layer the camera does not draw: only its shadow (cast by
    // the proxy, which the shadow camera does draw) shows.
    let std = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
    let ship = app
        .world_mut()
        .spawn((
            Mesh3d(cube),
            MeshMaterial3d(std),
            HullPart::default(),
            RenderLayers::layer(2),
            Transform::from_xyz(0.0, 350.0, 0.0).with_scale(Vec3::new(200.0, 40.0, 200.0)),
            Visibility::Hidden,
        ))
        .id();

    app.finish();
    app.cleanup();
    for _ in 0..10 {
        app.update();
    }
    // The adapter lives in the render world, which has it once the first frames have run.
    let info = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned().or_else(|| {
        app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned())
    });
    let Some(info) = info else {
        eprintln!("SKIP: no render adapter");
        return;
    };
    eprintln!("GPU adapter: {} ({:?}, {:?})", info.name, info.device_type, info.backend);
    if crate::bench::frames::is_software_adapter(info.device_type, &info.name) {
        eprintln!("SKIP: only a software adapter is available ({})", info.name);
        return;
    }
    for _ in 0..6 {
        app.update();
    }
    let open = capture(&mut app, &target, "gas-open");
    app.world_mut().entity_mut(ship).insert(Visibility::Inherited);
    for _ in 0..6 {
        app.update();
    }
    let shaded = capture(&mut app, &target, "gas-shadowed");
    save("gas-open", &open);
    save("gas-shadowed", &shaded);

    // The shadow column is 200 wide at 700 away: the middle of the frame. The frame's
    // edge is outside it.
    let (mid_open, mid_shaded) = (mean(&open, 60, 100), mean(&shaded, 60, 100));
    let (edge_open, edge_shaded) = (mean(&open, 0, 20), mean(&shaded, 0, 20));
    eprintln!("gas luminance: middle {mid_open:.1} -> {mid_shaded:.1}, edge {edge_open:.1} -> {edge_shaded:.1}");
    assert!(mid_open > 8.0, "no gas drawn (luminance {mid_open})");
    assert!(mid_shaded < mid_open * 0.9, "the ship's shadow did not darken the gas: {mid_open:.1} -> {mid_shaded:.1}");
    assert!((edge_open - edge_shaded).abs() < mid_open * 0.03 + 0.5, "gas outside the shadow changed: {edge_open:.1} -> {edge_shaded:.1}");
    let errors = ERRORS.lock().unwrap().clone();
    assert!(errors.is_empty(), "shader/pipeline ERROR events:\n{}", errors.join("\n"));
}

/// The hulls take the same map (`hulls.wesl`): a deck lit from above, with a ship over
/// it casting a shadow on it. Open, the deck is lit (it does not shadow itself); with the
/// ship in place, the deck under it is darker and the deck outside the shadow is the same.
/// Run alone from other GPU tests of this crate (they share one logger).
#[test]
fn a_hull_in_a_ships_shadow_is_darker_and_does_not_shadow_itself() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU shadow test");
        return;
    }
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .set(LogPlugin { level: bevy::log::Level::WARN, custom_layer: error_layer, ..default() })
            .disable::<bevy::winit::WinitPlugin>(),
    );
    app.add_plugins((HullPlugin, ShadowPlugin));
    app.insert_resource(ClearColor(Color::BLACK));
    let mut img = Image::new_target_texture(W, H, TextureFormat::Rgba8UnormSrgb, None);
    img.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(img);
    app.world_mut().spawn((DirectionalLight { illuminance: 6000.0, ..default() }, Transform::from_xyz(0.0, 10.0, 0.0).looking_at(Vec3::ZERO, Vec3::Z)));
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        Projection::Perspective(PerspectiveProjection { near: 1.0, far: 5000.0, ..default() }),
        Transform::from_xyz(0.0, 500.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let cube = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(1.0, 1.0, 1.0));
    let std = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial { base_color: Color::WHITE, ..default() });
    // The deck, seen from above.
    app.world_mut().spawn((Mesh3d(cube.clone()), MeshMaterial3d(std.clone()), HullPart::default(), Transform::from_xyz(0.0, 0.0, 0.0).with_scale(Vec3::new(600.0, 10.0, 600.0))));
    // The ship over it, on a layer the camera does not draw.
    let ship = app
        .world_mut()
        .spawn((Mesh3d(cube), MeshMaterial3d(std), HullPart::default(), RenderLayers::layer(2), Transform::from_xyz(0.0, 150.0, 0.0).with_scale(Vec3::new(160.0, 30.0, 160.0)), Visibility::Hidden))
        .id();
    app.finish();
    app.cleanup();
    for _ in 0..10 {
        app.update();
    }
    let info = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned().or_else(|| {
        app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned())
    });
    let Some(info) = info else {
        eprintln!("SKIP: no render adapter");
        return;
    };
    if crate::bench::frames::is_software_adapter(info.device_type, &info.name) {
        eprintln!("SKIP: only a software adapter is available ({})", info.name);
        return;
    }
    for _ in 0..6 {
        app.update();
    }
    let open = capture(&mut app, &target, "deck-open");
    app.world_mut().entity_mut(ship).insert(Visibility::Inherited);
    for _ in 0..6 {
        app.update();
    }
    let shaded = capture(&mut app, &target, "deck-shadowed");
    save("deck-open", &open);
    save("deck-shadowed", &shaded);
    let (mid_open, mid_shaded) = (mean(&open, 72, 88), mean(&shaded, 72, 88));
    let (edge_open, edge_shaded) = (mean(&open, 0, 12), mean(&shaded, 0, 12));
    eprintln!("deck luminance: middle {mid_open:.1} -> {mid_shaded:.1}, edge {edge_open:.1} -> {edge_shaded:.1}");
    assert!(mid_open > 30.0 && edge_open > 30.0, "the deck is dark with nothing over it ({mid_open:.1}, {edge_open:.1}): it shadows itself");
    assert!(mid_shaded < mid_open * 0.8, "the ship's shadow did not darken the deck: {mid_open:.1} -> {mid_shaded:.1}");
    assert!((edge_open - edge_shaded).abs() < 2.0, "the deck outside the shadow changed: {edge_open:.1} -> {edge_shaded:.1}");
    let errors = ERRORS.lock().unwrap().clone();
    assert!(errors.is_empty(), "shader/pipeline ERROR events:\n{}", errors.join("\n"));
}
