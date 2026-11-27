//! Optional real-GPU frame test of a planet out of sight (#1146): `STA64_GPU_TESTS=1` and a hardware adapter;
//! `STA64_GPU_OUT` keeps the PNGs outside the repository. An authored sphere with the planet material, the real
//! `update` system, one light. In sight the planet is as before; out of sight (its gate carries the marker) it is
//! drawn at the out-of-sight fraction of the brightness. Both looks.
use super::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
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

/// The planet's frame, or `None` without a hardware adapter.
fn frame(profile: RenderProfile, out_of_sight: bool) -> Option<image::RgbaImage> {
    frame_with(profile, out_of_sight, false, Tonemapping::None, true)
}

/// `fogged`: the planet in sight with the fog layer drawn over it as the scene camera drew it before #1145 (a quad
/// in front of the planet, the layer's own shader and opacity, an authored dark art colour, every cell unseen): the
/// look the out-of-sight copy replaces.
/// `tonemapping`: the camera's (the Refit look's real one is `Tonemapping::default()`, as `post.rs` gives it);
/// `gated`: the planet is under a gate (false: no gate at all, as before this change).
fn frame_with(profile: RenderProfile, out_of_sight: bool, fogged: bool, tonemapping: Tonemapping, gated: bool) -> Option<image::RgbaImage> {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>(),
    );
    embedded_asset!(app, "planet.wesl");
    app.add_plugins(MaterialPlugin::<PlanetMaterial>::default());
    if fogged {
        embedded_asset!(app, "fog.wesl");
        app.add_plugins(MaterialPlugin::<crate::render::fog::FogMaterial>::default());
    }
    let mut effects = ProfileEffects::default();
    effects.register_stock(Effect::Scenery);
    app.insert_resource(effects)
        .insert_resource(profile)
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(crate::clock::AnimClock::new(crate::clock::ClockMode::Wall, 1))
        .add_systems(Update, update);
    let mut image = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        tonemapping,
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    app.world_mut().spawn((DirectionalLight { illuminance: 6000.0, shadow_maps_enabled: false, ..default() }, Transform::IDENTITY.looking_to(Vec3::NEG_Z, Vec3::Y)));
    let material = app.world_mut().resource_mut::<Assets<PlanetMaterial>>().add(PlanetMaterial {
        params: base_params(),
        texture: None,
        albedo: None,
        relief: None,
        clouds: None,
        city: None,
        alpha: AlphaMode::Opaque,
    });
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(1.0).mesh().ico(4).unwrap());
    let gate = app.world_mut().spawn((Transform::IDENTITY, Visibility::Inherited)).id();
    if out_of_sight {
        app.world_mut().entity_mut(gate).insert(crate::render::fog_objects::OutOfSight);
    }
    app.world_mut().spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material.clone()),
        Transform::IDENTITY,
        PlanetBody { materials: vec![material], ground: vec![], shell: None, ground_name: String::new(), atmo_name: String::new(), applied: None, radius: 1.0 },
    ));
    if gated {
        let body = app.world_mut().query_filtered::<Entity, With<PlanetBody>>().single(app.world()).unwrap();
        app.world_mut().entity_mut(body).insert(ChildOf(gate));
    }
    if fogged {
        use crate::render::fog::{FogGrid, FogMaterial, Layer, mask_image, params, pattern_image};
        let grid = FogGrid::new([-10.0, -10.0], [10.0, 10.0]);
        let (art, size) = pattern_image(None, "none", [10, 10, 10]);
        let (mask, art) = {
            let mut images = app.world_mut().resource_mut::<Assets<Image>>();
            (images.add(mask_image(&grid, Layer::Fog)), images.add(art))
        };
        let material = app.world_mut().resource_mut::<Assets<FogMaterial>>().add(FogMaterial { params: params(Layer::Fog, &grid, size), mask, pattern: art, bias: 1.0e6 });
        let quad = app.world_mut().resource_mut::<Assets<Mesh>>().add(Rectangle::new(40.0, 40.0));
        app.world_mut().spawn((Mesh3d(quad), MeshMaterial3d(material), Transform::from_xyz(0.0, 0.0, 3.0)));
    }
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
        let name = if fogged { "fogged" } else if out_of_sight { "out" } else if gated { "in" } else { "ungated" };
        img.save(std::path::Path::new(&dir).join(format!("planet-{profile:?}-{tonemapping:?}-{name}.png"))).unwrap();
    }
    Some(img)
}

/// The mean of the red, green and blue values over the middle of the frame, where the planet is.
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

#[test]
fn a_planet_out_of_sight_is_dimmer_than_in_sight_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    for profile in [RenderProfile::Stock, RenderProfile::Refit] {
        let Some(live) = frame(profile, false) else { return };
        let Some(ghost) = frame(profile, true) else { return };
        let (l, g) = (mean(&live), mean(&ghost));
        eprintln!("{profile:?}: in sight {l:.2}, out of sight {g:.2}, ratio {:.3}", g / l);
        assert!(l > 20.0, "the planet is lit in sight: {l}");
        let ratio = g / l;
        // The factor is on the linear output so that the displayed values fall to the measured fraction (0.20):
        // read back from the 8-bit frame within a few hundredths (the mean over the disc's shading).
        let want = crate::render::fog_objects::ghost::PLANET;
        assert!((ratio - want).abs() < 0.06, "out of sight is the measured fraction of in sight: {ratio} against {want}");
        // What the fog layer made of the same planet (an authored dark art colour): STA64's own measurement of how
        // far the new look is from the old one.
        let Some(fogged) = frame_with(profile, false, true, Tonemapping::None, true) else { return };
        let f = mean(&fogged);
        eprintln!("{profile:?}: fogged (the old look, authored art) {f:.2}, ratio to in sight {:.3}; out-of-sight copy {g:.2}", f / l);
        assert!(f < l, "the fog layer dimmed the planet");
        // In sight is unchanged: the same frame as with no gate at all (what was drawn before this change).
        let Some(ungated) = frame_with(profile, false, false, Tonemapping::None, false) else { return };
        assert!((mean(&ungated) - l).abs() < 0.01, "{profile:?}: in sight is as before: {} against {l}", mean(&ungated));
        assert_eq!(ungated.as_raw(), live.as_raw(), "{profile:?}: in sight, every pixel as before");
        // The tone mapper the Refit look really has (`Tonemapping::default()`): in sight unchanged, out of sight
        // dimmer; the displayed ratio is reported (the mapper is not linear, so it is not the Stock look's 0.20).
        if profile == RenderProfile::Refit {
            let tm = Tonemapping::default();
            let Some(live_tm) = frame_with(profile, false, false, tm, true) else { return };
            let Some(ungated_tm) = frame_with(profile, false, false, tm, false) else { return };
            let Some(ghost_tm) = frame_with(profile, true, false, tm, true) else { return };
            assert_eq!(live_tm.as_raw(), ungated_tm.as_raw(), "Refit with its tone mapper: in sight, every pixel as before");
            let (a, b) = (mean(&live_tm), mean(&ghost_tm));
            eprintln!("Refit with the tone mapper: in sight {a:.2}, out of sight {b:.2}, ratio {:.3}", b / a);
            assert!(b < a * 0.6, "out of sight is dimmer in the tone-mapped look: {b} against {a}");
        }
    }
}
