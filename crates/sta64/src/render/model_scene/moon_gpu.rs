//! Optional, windowless frames of a dilithium moon under the Stock look (install-gated, nothing committed from it).
//! `STA64_GPU_TESTS=1` and `STA64_TEST_INSTALL` (an Armada II or Armada I install); `STA64_GPU_OUT` keeps the PNGs
//! outside the repository. The moon is drawn through the real spawn path and the Stock hull look, lit by one
//! white directional light, with no sprites (the nimbus is a separate sprite node).
use super::*;
use bevy::camera::RenderTarget;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};

#[test]
fn installed_moon_has_no_highlight_and_a_lit_core() {
    let (Ok("1"), Ok(root)) = (std::env::var("STA64_GPU_TESTS").as_deref(), std::env::var("STA64_TEST_INSTALL")) else {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 and STA64_TEST_INSTALL");
        return;
    };
    let active_mod = std::env::var("STA64_TEST_MOD").ok();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), active_mod.as_deref()).unwrap();
    let model_key = std::env::var("STA64_TEST_MODEL").unwrap_or_else(|_| "mdmoon".into());
    eprintln!("GPU fixture: {model_key}");
    let (path, fmt) = sta_assets::find_model(&vfs, &model_key).expect("requested model");
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
    app.insert_resource(crate::render::profile::RenderProfile::Stock);
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
    let _camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            RenderTarget::from(target.clone()),
            Msaa::Off,
            Transform::from_translation(centre + Vec3::new(0.0, radius * 2.5, radius * 1.2))
                .looking_at(centre, Vec3::Y),
        ))
        .id();
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
                diffuse_tint: false,
            };
            let stats = spawn(&mut ctx, &model, root);
            assert!(stats.meshes > 0 && stats.triangles > 0);
            assert!(stats.missing_textures.is_empty(), "{:?}", stats.missing_textures);
        },
    );
    app.finish();
    app.cleanup();
    let frames = Arc::new(Mutex::new(Vec::<(String, Image)>::new()));
    let mut stats = Vec::new();
    for (name, lit_from) in [("lit-front", Vec3::new(0.0, 1.0, 1.0)), ("lit-side", Vec3::new(1.0, 0.2, 0.2))] {
        {
            let mut q = app.world_mut().query_filtered::<&mut Transform, With<DirectionalLight>>();
            for mut t in q.iter_mut(app.world_mut()) {
                *t = Transform::from_translation(centre + lit_from.normalize() * radius * 4.0).looking_at(centre, Vec3::Y);
            }
        }
        for _ in 0..12 {
            app.update();
        }
        let frames_out = Arc::clone(&frames);
        let owned = name.to_owned();
        app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| {
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
        let image = frames.lock().unwrap().last().unwrap().1.clone();
        let rgba = image.try_into_dynamic().unwrap().to_rgba8();
        if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
            std::fs::create_dir_all(&dir).unwrap();
            rgba.save(std::path::Path::new(&dir).join(format!("moon-{name}.png"))).unwrap();
        }
        stats.push((name, MoonFrame::of(&rgba)));
    }
    for (name, s) in &stats {
        eprintln!("moon {model_key} {name}: {s:?}");
    }
    for (name, s) in &stats {
        assert!(s.lit > 500, "{name}: the moon is invisible: {s:?}");
        assert!(s.core > 50, "{name}: no glow shell shows through the rock: {s:?}");
        assert!(s.core_mean_b > s.rock_mean, "{name}: the core is not brighter than the rock: {s:?}");
    }
}

/// What a frame of the moon shows, read from its pixels.
#[derive(Debug)]
#[allow(dead_code)]
struct MoonFrame {
    /// Pixels brighter than the clear colour.
    lit: usize,
    /// Blue-dominant pixels (the glow shell seen through the rock's cut-outs).
    core: usize,
    /// Mean of the blue channel over the core pixels.
    core_mean_b: f32,
    /// Mean of the brightest channel over the other (rock) pixels.
    rock_mean: f32,
    /// Core pixels that are nearly white: a highlight on the shell.
    core_white: usize,
    /// Rock pixels brighter than 0.9: a highlight on the rock.
    rock_hot: usize,
}

impl MoonFrame {
    fn of(img: &image::RgbaImage) -> MoonFrame {
        let (mut lit, mut core, mut core_b, mut rock, mut rock_sum, mut core_white, mut rock_hot) = (0, 0, 0u64, 0, 0u64, 0, 0);
        for p in img.pixels() {
            let (r, g, b) = (p[0] as u32, p[1] as u32, p[2] as u32);
            let top = r.max(g).max(b);
            if top <= 20 {
                continue;
            }
            lit += 1;
            if b > r + 40 && b + 40 >= g {
                core += 1;
                core_b += b as u64;
                core_white += (r > 250) as usize;
            } else {
                rock += 1;
                rock_sum += top as u64;
                rock_hot += (top > 230) as usize;
            }
        }
        MoonFrame { lit, core, core_mean_b: core_b as f32 / core.max(1) as f32, rock_mean: rock_sum as f32 / rock.max(1) as f32, core_white, rock_hot }
    }
}

/// Install-gated, no GPU: the moon's rock and glow shell, built from the install's lighting materials, take no
/// specular colour under the Stock look (their lighting model is not 2), whatever specular their data carries.
#[test]
fn installed_moon_materials_draw_no_highlight_in_the_stock_look() {
    use crate::render::hulls::{hull_material, HullPart, HullView};
    use crate::render::stock::hulls::{draws_specular, StockLight};
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_INSTALL");
        return;
    };
    let active_mod = std::env::var("STA64_TEST_MOD").ok();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), active_mod.as_deref()).unwrap();
    let Some((path, fmt)) = sta_assets::find_model(&vfs, "mdmoon") else {
        eprintln!("SKIP: no moon model in this install");
        return;
    };
    let model = sta_assets::load_model(&vfs, &path, fmt).unwrap();
    let mut view = HullView::default();
    view.set_stock(true);
    let mut seen = 0;
    for node in &model.nodes {
        let sta_assets::model::NodeKind::Mesh(mesh) = &node.kind else { continue };
        for group in mesh.groups.iter() {
            let Some(lm) = model.lighting_material(&group.material) else { continue };
            seen += 1;
            let part = HullPart { stock: Some(StockLight::from_material(lm)), ..default() };
            let m = hull_material(&StandardMaterial::default(), &part, &view);
            assert!(!draws_specular(lm.model), "{}: a moon material of model 2", group.material);
            assert_eq!(m.params.mspc.truncate(), Vec3::ZERO, "{}: highlight colour", group.material);
        }
    }
    assert!(seen >= 2, "the moon's rock and shell materials were not found");
}
