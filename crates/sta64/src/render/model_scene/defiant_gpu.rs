//! Optional, windowless render of the installed Defiant through the real model path.
use super::*;
use bevy::camera::RenderTarget;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};

#[test]
fn installed_defiant_draws_offscreen() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1")
        || std::env::var("STA64_DEFIANT_TESTS").as_deref() != Ok("1")
    {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 STA64_DEFIANT_TESTS=1 and STA64_TEST_INSTALL/STA64_TEST_MOD");
        return;
    }
    let root = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let active_mod = std::env::var("STA64_TEST_MOD").expect("STA64_TEST_MOD");
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(&active_mod)).unwrap();
    let (path, fmt) = sta_assets::find_model(&vfs, "fdestroy").expect("Defiant model");
    let model = sta_assets::load_model(&vfs, &path, fmt).unwrap();
    let (lo, hi) = model.bounds().expect("model bounds");
    let centre = Vec3::from(mirror(std::array::from_fn(|i| (lo[i] + hi[i]) * 0.5))) * 0.8;
    let radius = (Vec3::from(hi) - Vec3::from(lo)).length() * 0.4;
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>(),
    );
    app.add_plugins(crate::render::hulls::HullPlugin);
    app.insert_resource(ClearColor(Color::BLACK));
    app.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 100.0,
        ..default()
    });
    let mut target_image = Image::new_target_texture(
        256,
        256,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    target_image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(target_image);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        Transform::from_translation(centre + Vec3::new(0.0, radius * 2.5, radius * 1.2)).looking_at(centre, Vec3::Y),
    ));
    app.world_mut().spawn((
        DirectionalLight {
            illuminance: 6000.0,
            ..default()
        },
        Transform::from_translation(centre + Vec3::splat(radius * 2.0)).looking_at(centre, Vec3::Y),
    ));
    app.add_systems(
        Startup,
        move |mut commands: Commands,
              mut meshes: ResMut<Assets<Mesh>>,
              mut materials: ResMut<Assets<StandardMaterial>>,
              mut images: ResMut<Assets<Image>>| {
            let root = commands.spawn((Transform::IDENTITY, Visibility::default())).id();
            let mut cache = crate::render::textures::TextureCache::default();
            let sprites = SpriteTable::default();
            let mut ctx = SpawnCtx {
                commands: &mut commands,
                meshes: &mut meshes,
                materials: &mut materials,
                textures: Loader {
                    vfs: &vfs,
                    cache: &mut cache,
                    images: &mut images,
                    formats: bevy::image::CompressedImageFormats::NONE,
                },
                team: Color::WHITE,
                borg: false,
                sprites: &sprites,
                damaged: false,
                normal_maps: true,
                parts: None,
                model_key: path.clone(),
                unlit: false,
                scale: 0.8,
            };
            let stats = spawn(&mut ctx, &model, root);
            assert!(stats.meshes > 0 && stats.triangles > 0);
            assert!(stats.missing_textures.is_empty(), "{:?}", stats.missing_textures);
        },
    );
    app.finish();
    app.cleanup();
    let frames = Arc::new(Mutex::new(Vec::<(String, Image)>::new()));
    for (name, on) in [("hull-fresh", true), ("pbr", false), ("hull", true)] {
        app.world_mut().resource_mut::<crate::render::hulls::HullLighting>().on = on;
        for _ in 0..12 {
            app.update();
        }
        let frames_out = Arc::clone(&frames);
        let owned = name.to_owned();
        app.world_mut()
            .spawn(Screenshot::image(target.clone()))
            .observe(move |shot: On<ScreenshotCaptured>| {
                frames_out.lock().unwrap().push((owned.clone(), shot.image.clone()));
            });
        let before = frames.lock().unwrap().len();
        for _ in 0..60 {
            app.update();
            if frames.lock().unwrap().len() > before {
                break;
            }
        }
        assert!(frames.lock().unwrap().len() > before, "no {name} frame");
    }
    for (name, image) in frames.lock().unwrap().iter() {
        let dynamic = image.clone().try_into_dynamic().unwrap();
        let rgba = dynamic.to_rgba8();
        let lit = rgba.pixels().filter(|p| p[0].max(p[1]).max(p[2]) > 20).count();
        eprintln!("Defiant {name}: {lit} lit pixels");
        if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
            std::fs::create_dir_all(&dir).unwrap();
            dynamic
                .save(std::path::Path::new(&dir).join(format!("defiant-{name}.png")))
                .unwrap();
        }
        assert!(lit > 500, "Defiant {name} invisible: {lit} lit pixels");
    }
}
