//! Opt-in GPU test: the rock, planet and nebula materials must build real pipelines.
//!
//! Bevy loads `.wgsl` as plain WGSL, so a shader that still used naga_oil's
//! `#{MATERIAL_BIND_GROUP}` or `#ifdef` fails to compile; the failure is only logged
//! (`ERROR`), the process exits 0 and nothing draws. The unit tests next to each
//! material validate the text with naga, but only a pipeline made on a GPU, through
//! Bevy's own shader loader, shows what the game does. This one draws a cuboid per
//! material (the nebula under both MSAA settings, with a depth prepass) and fails on
//! any `ERROR` event.
//!
//! Needs a hardware adapter: prints `SKIP` and passes unless `STA64_GPU_TESTS=1`,
//! and skips when the only adapter is a software one. The adapter is read after the
//! warm-up frames, from the render world, which is where it lives.

use std::sync::Mutex;

use bevy::asset::embedded_asset;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::log::tracing::{self, Event, Subscriber};
use bevy::log::tracing_subscriber::{layer::Context, Layer};
use bevy::log::{BoxedLayer, LogPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::asset::RenderAssetUsages;
use bevy::render::RenderPlugin;
use bevy::window::ExitCondition;

use super::nebula::{NebulaMaterial, NebulaParams};
use super::planet::{PlanetMaterial, PlanetParams};
use super::rock::{RockMaterial, RockParams};

static ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Keeps every ERROR event's message.
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

fn camera(commands: &mut Commands, image: Handle<Image>, msaa: Msaa, order: isize) {
    commands.spawn((
        Camera3d::default(),
        Camera { order, ..default() },
        RenderTarget::from(image),
        msaa,
        DepthPrepass,
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

#[test]
fn rock_planet_and_nebula_pipelines_build() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU shader test");
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
    // The shaders as the game registers them (`embedded://sta64/render/*.wesl`).
    embedded_asset!(app, "rock.wesl");
    embedded_asset!(app, "planet.wesl");
    embedded_asset!(app, "nebula.wesl");
    app.add_plugins((
        MaterialPlugin::<RockMaterial>::default(),
        MaterialPlugin::<PlanetMaterial>::default(),
        MaterialPlugin::<NebulaMaterial>::default(),
    ));

    app.add_systems(
        Startup,
        |mut commands: Commands,
              mut images: ResMut<Assets<Image>>,
              mut meshes: ResMut<Assets<Mesh>>,
              mut rocks: ResMut<Assets<RockMaterial>>,
              mut planets: ResMut<Assets<PlanetMaterial>>,
              mut nebulae: ResMut<Assets<NebulaMaterial>>| {
            for (i, msaa) in [Msaa::Off, Msaa::Sample4].into_iter().enumerate() {
                let mut img = Image::new_target_texture(64, 64, TextureFormat::Rgba8UnormSrgb, None);
                img.asset_usage = RenderAssetUsages::RENDER_WORLD;
                camera(&mut commands, images.add(img), msaa, i as isize);
            }
            let mesh = meshes.add(Cuboid::default());
            commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(rocks.add(RockMaterial { params: RockParams::default(), albedo: None, relief: None }))));
            // A moon: cut-out cracks lit from inside, with the ravine's parallax march (RND-05).
            let moon = RockParams {
                misc: Vec4::new(0.02, 0.0, 1.0, 0.0),
                ext: Vec4::new(1.0, 1.0, 1.0, 1.0),
                core: Vec4::new(0.0, 2.0, 4.0, 0.22),
                spill: Vec4::new(1.0, 1.0, 0.02, 3.0),
                row0: Vec4::X,
                row1: Vec4::Y,
                row2: Vec4::Z,
                ..RockParams::default()
            };
            commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(rocks.add(RockMaterial { params: moon, albedo: None, relief: None }))));
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(planets.add(PlanetMaterial {
                    params: PlanetParams::default(),
                    texture: None,
                    albedo: None,
                    relief: None,
                    clouds: None,
                    alpha: AlphaMode::Opaque,
                })),
            ));
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(nebulae.add(NebulaMaterial { params: NebulaParams::default(), noise: None, envelope: None, shadow: None })),
            ));
            // The nebula with its extras on: the core's pulse and ring, the swirl and
            // lightning (RND-05). Their branches are uniform-driven, so this is the same
            // pipeline, but it runs them.
            let extras = NebulaParams {
                pulse: Vec4::new(0.5, 1.0, 1.0, 0.0),
                ring: Vec4::new(0.85, 0.55, 0.3, 0.25),
                rpatch: Vec4::new(0.4, 0.35, 0.15, 0.6),
                ring_a: Vec4::new(0.3, 0.9, 0.8, 0.0),
                ring_b: Vec4::new(0.5, 0.4, 1.0, 0.0),
                ring_c: Vec4::new(0.9, 0.4, 1.0, 0.0),
                ring_d: Vec4::new(0.6, 1.0, 0.4, 0.0),
                swirl: Vec4::new(24.0, 1.0, 1.0, 2.0),
                flash0: Vec4::new(0.0, 0.0, 0.0, 1.0),
                flash_c: Vec4::new(1.0, 0.9, 0.8, 0.01),
                ..NebulaParams::default()
            };
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(nebulae.add(NebulaMaterial { params: extras, noise: None, envelope: None, shadow: None })),
            ));
        },
    );
    // Pipelines are made the first frames a material is drawn; leave slack.
    app.finish();
    app.cleanup();
    for _ in 0..30 {
        app.update();
    }
    // The adapter lives in the render world, which has it once the first frames have run.
    let info = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned().or_else(|| {
        app.get_sub_app(bevy::render::RenderApp)
            .and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned())
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
    let errors = ERRORS.lock().unwrap().clone();
    assert!(errors.is_empty(), "shader/pipeline ERROR events:\n{}", errors.join("\n"));
}
