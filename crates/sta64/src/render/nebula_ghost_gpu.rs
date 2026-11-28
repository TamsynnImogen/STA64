//! Optional real-GPU frame test of a nebula's gas out of sight (#1146): `STA64_GPU_TESTS=1` and a hardware adapter;
//! `STA64_GPU_OUT` keeps the PNGs outside the repository. One authored nebula, the real plugin and `update`
//! system, the envelope and light weights as `gate_fields` writes them. A nebula seen before and out of sight keeps
//! its shape (the same pixels are covered) and emits the inferred half of the displayed brightness; in sight is as
//! before (pixel for pixel, also with the weights texture absent).
use super::*;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::render::RenderPlugin;
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

/// How the gas is dimmed: nothing (in sight), or as `gate_fields` writes it for a nebula seen before and out of
/// sight; `NoTexture`: in sight with no weights texture on the material at all.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Look {
    InSight,
    NoTexture,
    OutOfSight,
}

/// The frame of one nebula's gas seen from afar, or `None` without a hardware adapter.
fn gas_frame(look: Look) -> Option<image::RgbaImage> {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    app.insert_resource(ClearColor(Color::BLACK));
    let mut image = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        Tonemapping::None,
        DepthPrepass,
        Transform::from_xyz(0.0, 600.0, 1400.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    app.add_plugins(NebulaPlugin);
    app.insert_resource(crate::clock::AnimClock::new(crate::clock::ClockMode::Bench { step: 0.0 }, 1));
    // One ray per pixel: the gas is drawn by its own material, without the low-resolution stand-in.
    app.insert_resource(crate::render::quality::RenderQuality { shadows: false, msaa: 1, depth_prepass: true, nebula_steps: 0, nebula_scale: 1 });
    let recipe = NebulaRecipe::parse("[Nebula]\nExtent=1.3\nHeight=3\nGasA=0.8 0.4 0.2\nGasB=0.2 0.4 0.8\nGlow=0.5 0.5 0.5\nBrightness=400\nCoverage=0.5\n");
    let centres = vec![[0.0f32, 0.0, 0.0]];
    let (r, c) = (recipe.clone(), centres.clone());
    app.add_systems(
        Startup,
        move |mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<NebulaMaterial>>, mut images: ResMut<Assets<Image>>, noise: Res<NoiseVolume>| {
            let parent = commands.spawn((Transform::IDENTITY, Visibility::Inherited)).id();
            spawn_field(&mut commands, &mut meshes, &mut materials, &mut images, &noise, r.clone(), 300.0, &c, parent);
        },
    );
    app.finish();
    app.cleanup();
    // The noise volume is made on a thread; wait for it.
    for _ in 0..1500 {
        app.update();
        if app.world().resource::<NoiseVolume>().image.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(app.world().resource::<NoiseVolume>().image.is_some(), "the noise volume was made");
    let (material, light) = {
        let mut q = app.world_mut().query::<&NebulaField>();
        let field = q.single(app.world()).unwrap();
        let m = field.material.clone();
        let l = app.world().resource::<Assets<NebulaMaterial>>().get(&m).unwrap().light.clone().unwrap();
        (m, l)
    };
    match look {
        Look::InSight => {}
        Look::NoTexture => app.world_mut().resource_mut::<Assets<NebulaMaterial>>().get_mut(&material).unwrap().light = None,
        Look::OutOfSight => {
            // As `gate_fields` writes it for the one nebula seen before and out of sight: the whole envelope, and the
            // linear factor of the inferred half of the displayed brightness in the light weights.
            let factor = crate::render::fog_objects::ghost::nebula_gas_linear();
            let bytes = sta_surface::nebula::light_weights(&recipe, 300.0, &centres, &[true], &[factor]);
            app.world_mut().resource_mut::<Assets<Image>>().get_mut(&light).unwrap().data = Some(bytes);
        }
    }
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
    let img = shoot(&mut app, &target);
    if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
        std::fs::create_dir_all(&dir).unwrap();
        img.save(std::path::Path::new(&dir).join(format!("gas-{look:?}.png"))).unwrap();
    }
    Some(img)
}

/// The mean displayed value over the whole frame (red, green, blue), and how many pixels show any light.
fn stats(img: &image::RgbaImage) -> (f32, usize) {
    let sum: f32 = img.pixels().map(|p| f32::from(p.0[0]) + f32::from(p.0[1]) + f32::from(p.0[2])).sum();
    let lit = img.pixels().filter(|p| p.0[0] > 0 || p.0[1] > 0 || p.0[2] > 0).count();
    (sum / (3.0 * (SIZE * SIZE) as f32), lit)
}

#[test]
fn a_nebulas_gas_out_of_sight_keeps_its_shape_at_half_the_displayed_brightness_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let Some(live) = gas_frame(Look::InSight) else { return };
    let Some(bare) = gas_frame(Look::NoTexture) else { return };
    let Some(again) = gas_frame(Look::InSight) else { return };
    let Some(ghost) = gas_frame(Look::OutOfSight) else { return };
    let ((l, ln), (g, gn)) = (stats(&live), stats(&ghost));
    // Over the pixels the gas lights to a mid displayed value in sight (not near black, where the transfer curve is
    // linear and the displayed ratio is the linear one; not saturated): the displayed ratio.
    let (mut sum_live, mut sum_ghost, mut mid) = (0.0f32, 0.0f32, 0);
    for (a, b) in live.pixels().zip(ghost.pixels()) {
        let top = a.0[0].max(a.0[1]).max(a.0[2]);
        if (60..=230).contains(&top) {
            mid += 1;
            sum_live += f32::from(a.0[0]) + f32::from(a.0[1]) + f32::from(a.0[2]);
            sum_ghost += f32::from(b.0[0]) + f32::from(b.0[1]) + f32::from(b.0[2]);
        }
    }
    let ratio = sum_ghost / sum_live.max(1.0);
    eprintln!("gas: in sight mean {l:.3} over {ln} lit pixels; out of sight mean {g:.3} over {gn} lit pixels; over {mid} mid-value pixels the displayed ratio is {ratio:.3}");
    assert!(l > 3.0 && ln > 200 && mid > 100, "the gas shows in sight: {l}, {ln} pixels, {mid} mid-value");
    assert_eq!(live.as_raw(), again.as_raw(), "the frame repeats");
    assert_eq!(live.as_raw(), bare.as_raw(), "in sight every pixel is the same with a white weights texture and with none");
    // About half of the displayed brightness (the factor is on the linear light: the band allows the transfer curve
    // and the 8-bit steps).
    assert!((0.4..0.6).contains(&ratio), "half of the displayed brightness: {ratio}");
    // The same pixels are covered: the silhouette within a few percent (a pixel at the edge may fall to zero).
    assert!((gn as f32) > ln as f32 * 0.9 && gn <= ln, "the silhouette is kept: {gn} against {ln} lit pixels");
}
