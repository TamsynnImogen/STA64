//! Optional real-asset, windowless check of the station reveal and depth prepass.
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};

#[derive(Resource, Default)]
struct Stages(Vec<Entity>);

struct Frozen(std::sync::Arc<sta_sim::WorldView>);

impl sta_sim::WorldSource for Frozen {
    fn view(&self) -> std::sync::Arc<sta_sim::WorldView> {
        self.0.clone()
    }
    fn step(&mut self, _: &[sta_sim::command::Command]) -> sta_sim::TickOutput {
        sta_sim::TickOutput {
            view: self.0.clone(),
            events: Vec::new(),
        }
    }
}

fn install_progress(app: &mut App, progress: Option<f32>) {
    use sta_sim::WorldSource;
    let db = sta_content::fake::db();
    let source = sta_sim::fake::ScriptedSource::from_json5(
        r#"{units:[{id:1,def:"fx_cruiser",owner:0,hull:800,rails:[{at:0,pos:{x:0,y:0,z:0}}]}]}"#,
        &db,
    )
    .unwrap();
    let mut view = (*source.view()).clone();
    view.units[0].progress = progress.map(|p| sta_sim::Fixed::from_f64(f64::from(p)));
    app.insert_resource(crate::sim_link::SimLink::new(
        Box::new(Frozen(Arc::new(view))),
        Box::new(sta_sim::command::LocalRouter::new()),
        sta_sim::PlayerId(0),
    ));
}

#[test]
fn installed_station_reveals_opaque_chunks_offscreen() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1")
        || std::env::var("STA64_CONSTRUCTION_TESTS").as_deref() != Ok("1")
    {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 STA64_CONSTRUCTION_TESTS=1 and STA64_TEST_INSTALL/STA64_TEST_MOD");
        return;
    }
    let root = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), active_mod.as_deref()).unwrap();
    let model_key = std::env::var("STA64_TEST_MODEL").unwrap_or_else(|_| "fbase".into());
    let (path, fmt) = sta_assets::find_model(&vfs, &model_key).expect("requested station model");
    let model = sta_assets::load_model(&vfs, &path, fmt).unwrap();
    let (lo, hi) = model.bounds().expect("station bounds");
    let centre = Vec3::new((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, -(lo[2] + hi[2]) * 0.5);
    let radius = (Vec3::from(hi) - Vec3::from(lo)).length() * 0.5;
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
    app.add_plugins(crate::render::hulls::HullPlugin)
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 100.0,
            ..default()
        })
        .init_resource::<Stages>()
        .init_resource::<super::ConstructionEdges>()
        .init_resource::<super::ConstructionWork>()
        .add_systems(
            PostUpdate,
            super::sync_reveal.after(bevy::transform::TransformSystems::Propagate),
        );
    install_progress(&mut app, Some(0.0));
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
        bevy::core_pipeline::prepass::DepthPrepass,
        Transform::from_translation(centre + Vec3::new(radius * 0.3, radius * 2.5, radius * 1.2))
            .looking_at(centre, Vec3::Y),
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
              mut stages: ResMut<Stages>,
              mut meshes: ResMut<Assets<Mesh>>,
              mut materials: ResMut<Assets<StandardMaterial>>,
              mut images: ResMut<Assets<Image>>| {
            let mut cache = crate::render::textures::TextureCache::default();
            let sprites = sta_assets::a2fmt::spr::SpriteTable::default();
            let mut parts = crate::render::model_scene::PartCache::default();
            for progress in [0.0, 1.0] {
                let root = commands.spawn((Transform::IDENTITY, Visibility::Hidden)).id();
                if progress == 0.0 {
                    commands
                        .entity(root)
                        .insert(crate::sim_link::SimEntity(sta_sim::EntityId(1)));
                }
                stages.0.push(root);
                let mut ctx = crate::render::model_scene::SpawnCtx {
                    commands: &mut commands,
                    meshes: &mut meshes,
                    materials: &mut materials,
                    textures: crate::render::textures::Loader {
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
                    parts: Some(&mut parts),
                    model_key: path.clone(),
                    unlit: false,
                    scale: 1.0,
                };
                let stats = crate::render::model_scene::spawn_construction(&mut ctx, &model, root, 24, progress);
                assert!(stats.meshes > 0 && stats.triangles > 0);
                assert!(stats.missing_textures.is_empty(), "{:?}", stats.missing_textures);
            }
        },
    );
    app.finish();
    app.cleanup();
    app.update();
    let roots = app.world().resource::<Stages>().0.clone();
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let captures = [
        (Some(0.5), Some(None)),
        (Some(0.5), Some(Some(0.5))),
        (Some(0.75), Some(Some(0.5))),
        (Some(0.5), None), // Missing work entry keeps ordinary progress unblocked.
        (Some(1.0), Some(None)),
        (None, None), // Unblocked full reference instance.
    ];
    for (stage, (progress, start)) in captures.into_iter().enumerate() {
        if stage < 5 {
            install_progress(&mut app, progress);
        }
        {
            let mut work = app.world_mut().resource_mut::<super::ConstructionWork>();
            work.starts.clear();
            if let Some(start) = start {
                work.starts.insert(sta_sim::EntityId(1), start);
            }
        }
        let visible = roots[usize::from(stage == 5)];
        for &root in &roots {
            *app.world_mut().get_mut::<Visibility>(root).unwrap() = if root == visible {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
        for _ in 0..12 {
            app.update();
        }
        let out = Arc::clone(&frames);
        app.world_mut()
            .spawn(Screenshot::image(target.clone()))
            .observe(move |shot: On<ScreenshotCaptured>| {
                out.lock().unwrap().push(shot.image.clone());
            });
        for _ in 0..60 {
            app.update();
            if frames.lock().unwrap().len() > stage {
                break;
            }
        }
        assert_eq!(
            frames.lock().unwrap().len(),
            stage + 1,
            "no station stage {stage} capture"
        );
    }
    let mut masks = Vec::new();
    for (stage, image) in frames.lock().unwrap().iter().enumerate() {
        let dynamic = image.clone().try_into_dynamic().unwrap();
        let rgba = dynamic.to_rgba8();
        let mask: Vec<bool> = rgba.pixels().map(|p| p[0].max(p[1]).max(p[2]) > 20).collect();
        eprintln!(
            "{model_key} construction stage {stage}: {} lit pixels",
            mask.iter().filter(|&&p| p).count()
        );
        masks.push(mask);
        if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
            std::fs::create_dir_all(&dir).unwrap();
            dynamic
                .save(std::path::Path::new(&dir).join(format!("construction-{model_key}-{stage}.png")))
                .unwrap();
        }
    }
    let counts: Vec<usize> = masks.iter().map(|m| m.iter().filter(|&&p| p).count()).collect();
    assert_eq!(counts[0], 0, "pre-weld progress must not display the hull");
    assert_eq!(counts[0], counts[1], "latched first weld must not jump visual progress");
    assert!(
        counts[2] > 100 && counts[2] < counts[4],
        "station fails partial/full reveal: {counts:?}"
    );
    assert_eq!(
        counts[2], counts[3],
        "missing work entry must use ordinary sim progress"
    );
    let outside = masks[2]
        .iter()
        .zip(&masks[4])
        .filter(|(partial, full)| **partial && !**full)
        .count();
    assert!(
        outside * 100 <= counts[2],
        "partial geometry escaped full station silhouette: {outside}"
    );
    let mismatch = masks[4]
        .iter()
        .zip(&masks[5])
        .filter(|(finished, original)| finished != original)
        .count();
    assert_eq!(mismatch, 0, "completed station differs from the original shared mesh");
    let world = app.world_mut();
    assert_eq!(
        world.query::<&super::ConstructionPart>().iter(world).count(),
        0,
        "private reveal data survives completion"
    );
}
