//! Optional windowless, installed-model and placement-grid brightness check.
use super::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};

#[derive(Resource)]
struct ModelRoot(Entity);

#[test]
fn installed_ghost_is_readable_without_lights() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1")
        || std::env::var("STA64_GHOST_TESTS").as_deref() != Ok("1")
    {
        eprintln!(
            "SKIP: set STA64_GPU_TESTS=1 STA64_GHOST_TESTS=1 and STA64_TEST_INSTALL/STA64_TEST_MOD"
        );
        return;
    }
    let install = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let active_mod = std::env::var("STA64_TEST_MOD")
        .ok()
        .filter(|m| !m.is_empty());
    let vfs =
        sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).unwrap();
    let model_key = std::env::var("STA64_TEST_MODEL").unwrap_or_else(|_| "fyard".into());
    let (path, format) = sta_assets::find_model(&vfs, &model_key).expect("requested yard model");
    let model = sta_assets::load_model(&vfs, &path, format).unwrap();
    let (lo, hi) = model.bounds().expect("model bounds");
    let centre = Vec3::new(
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        -(lo[2] + hi[2]) * 0.5,
    );
    let radius = (Vec3::from(hi) - Vec3::from(lo)).length() * 0.5;
    use sta_sim::WorldSource;
    let mut def = sta_content::UnitDef::new("gpu_blueprint", "shipyard");
    def.launch = Some(sta_content::LaunchPoint {
        offset: (sta_sim::Fixed::ZERO, sta_sim::Fixed::ZERO),
        dir: (sta_sim::Fixed::ZERO, sta_sim::Fixed::ONE),
        clear: sta_sim::Fixed::from_f64(f64::from(radius) * 4.0 / 3.0),
        node: None,
    });
    let db = std::sync::Arc::new(
        sta_content::ContentDb::builder()
            .unit(def)
            .unit(sta_content::UnitDef::new("gpu_builder", "constructionrig"))
            .unit(sta_content::UnitDef::new("gpu_object", "starbase"))
            .build()
            .unwrap(),
    );
    let def_key = db.key("gpu_blueprint").unwrap();
    let view = sta_sim::fake::ScriptedSource::from_json5(
        r#"{units:[{id:1,def:"gpu_builder",owner:0,hull:100,rails:[{at:0,pos:{x:0,y:0,z:0}}]}]}"#,
        &db,
    )
    .unwrap()
    .view();
    let grid_cfg = sta_content::layered::cfg_h(&vfs, "RTS_CFG.h");
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
        .add_systems(
            Update,
            (
                tint_ghost,
                sync_footprints,
                super::super::object_footprints::sync_object_footprints,
            )
                .chain(),
        )
        .insert_resource(crate::viewer::Mounted(vfs))
        .insert_resource(HudFeed {
            view,
            player: sta_sim::PlayerId(0),
            selected: Vec::new(),
            content: Some(db.clone()),
            ..Default::default()
        })
        .insert_resource(HudState {
            placing: Some(Armed {
                builder: sta_sim::EntityId(1),
                what: def_key,
                odf: "gpu_blueprint".into(),
            }),
            ..default()
        })
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 0.0,
            ..default()
        });
    let mut target_image = Image::new_target_texture(
        256,
        256,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    target_image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(target_image);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        bevy::core_pipeline::prepass::DepthPrepass,
        Transform::from_translation(centre + Vec3::new(radius * 0.3, radius * 2.5, radius * 1.2))
            .looking_at(centre, Vec3::Y),
    ));
    app.add_systems(
        Startup,
        move |mut commands: Commands,
              mounted: Res<crate::viewer::Mounted>,
              mut meshes: ResMut<Assets<Mesh>>,
              mut materials: ResMut<Assets<StandardMaterial>>,
              mut images: ResMut<Assets<Image>>| {
            let root = commands
                .spawn((Transform::IDENTITY, Visibility::Hidden))
                .id();
            commands.insert_resource(ModelRoot(root));
            let mut cache = crate::render::textures::TextureCache::default();
            let sprites = sta_assets::a2fmt::spr::SpriteTable::default();
            let mut parts = crate::render::model_scene::PartCache::default();
            let mut ctx = crate::render::model_scene::SpawnCtx {
                commands: &mut commands,
                meshes: &mut meshes,
                materials: &mut materials,
                textures: crate::render::textures::Loader {
                    vfs: &mounted.0,
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
            let stats = crate::render::model_scene::spawn(&mut ctx, &model, root);
            assert!(stats.meshes > 0 && stats.triangles > 0);
            assert!(
                stats.missing_textures.is_empty(),
                "{:?}",
                stats.missing_textures
            );
        },
    );
    app.finish();
    app.cleanup();
    for _ in 0..12 {
        app.update();
    }
    let adapter = app
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .cloned()
        .or_else(|| {
            app.get_sub_app(bevy::render::RenderApp).and_then(|r| {
                r.world()
                    .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
                    .cloned()
            })
        });
    let Some(adapter) = adapter else {
        eprintln!("SKIP: no render adapter available");
        return;
    };
    eprintln!("ghost GPU adapter available: {}", adapter.name);
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!(
            "SKIP: only a software adapter is available ({})",
            adapter.name
        );
        return;
    }
    let root = app.world().resource::<ModelRoot>().0;
    let mut descendants = Vec::new();
    let mut pending = vec![root];
    while let Some(parent) = pending.pop() {
        if let Some(children) = app.world().get::<Children>(parent) {
            for child in children.iter() {
                descendants.push(child);
                pending.push(child);
            }
        }
    }
    let sources: Vec<_> = descendants
        .iter()
        .filter_map(|&e| app.world().get::<HullSource>(e))
        .map(|s| s.source.clone())
        .collect();
    assert!(
        !sources.is_empty(),
        "fixture did not exercise the HullMaterial conversion path"
    );
    let originals: Vec<_> = sources
        .iter()
        .map(|h| {
            let m = app
                .world()
                .resource::<Assets<StandardMaterial>>()
                .get(h)
                .unwrap();
            (
                h.clone(),
                m.base_color,
                m.base_color_texture.clone(),
                m.alpha_mode,
            )
        })
        .collect();
    app.world_mut()
        .entity_mut(root)
        .insert(Ghost {
            def: def_key,
            status: Some(Ok(())),
            painted: None,
            own: Vec::new(),
        })
        .insert(Visibility::Inherited);
    let captures = Arc::new(Mutex::new(Vec::<Image>::new()));
    for (stage, status) in [Ok(()), Err(SiteError::Blocked), Ok(()), Ok(())]
        .into_iter()
        .enumerate()
    {
        if stage == 2 {
            app.world_mut().entity_mut(root).insert(PinnedBlueprint);
        }
        if stage == 3 {
            let world = app.world_mut();
            let mut grids = world.query_filtered::<&mut Visibility, With<FootprintGrid>>();
            for mut visibility in grids.iter_mut(world) {
                *visibility = Visibility::Hidden;
            }
        }
        app.world_mut().get_mut::<Ghost>(root).unwrap().status = Some(status);
        for _ in 0..12 {
            app.update();
        }
        for &e in &descendants {
            assert!(
                app.world().get::<HullSource>(e).is_none(),
                "ghost retained HullSource"
            );
            assert!(
                app.world().get::<MeshMaterial3d<HullMaterial>>(e).is_none(),
                "ghost retained HullMaterial"
            );
        }
        let output = Arc::clone(&captures);
        app.world_mut()
            .spawn(Screenshot::image(target.clone()))
            .observe(move |shot: On<ScreenshotCaptured>| {
                output.lock().unwrap().push(shot.image.clone());
            });
        for _ in 0..60 {
            app.update();
            if captures.lock().unwrap().len() > stage {
                break;
            }
        }
        assert_eq!(
            captures.lock().unwrap().len(),
            stage + 1,
            "no ghost frame {stage}"
        );
    }
    for (stage, image) in captures.lock().unwrap().iter().enumerate() {
        let dynamic = image.clone().try_into_dynamic().unwrap();
        let rgba = dynamic.to_rgba8();
        let lit = rgba
            .pixels()
            .filter(|p| {
                let channel = p[if stage == 1 { 0 } else { 1 }] as f32;
                channel > 80.0
                    && channel > 1.5 * p[if stage == 1 { 1 } else { 0 }] as f32
                    && channel > 1.5 * p[2] as f32
            })
            .count();
        eprintln!("{model_key} ghost {stage}: {lit} dominant pixels");
        assert!(lit > 100, "ghost {stage} too dim or wrong tint: {lit}");
        if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
            std::fs::create_dir_all(&dir).unwrap();
            dynamic
                .save(std::path::Path::new(&dir).join(format!("ghost-{model_key}-{stage}.png")))
                .unwrap();
        }
    }
    let frames = captures.lock().unwrap();
    let green_sum = |image: &Image| -> u64 {
        image
            .clone()
            .try_into_dynamic()
            .unwrap()
            .to_rgba8()
            .pixels()
            .map(|p| u64::from(p[1]))
            .sum()
    };
    assert!(
        green_sum(&frames[2]) < green_sum(&frames[0]),
        "pinned blueprint must be fainter than the cursor ghost"
    );
    // Check the grid itself: the ghost's bright hull must not make a missing
    // overlay pass this test. Frames differ only by the grid's visibility.
    let with_grid = frames[2].clone().try_into_dynamic().unwrap().to_rgba8();
    let without_grid = frames[3].clone().try_into_dynamic().unwrap().to_rgba8();
    let grid_pixels = with_grid
        .pixels()
        .zip(without_grid.pixels())
        .filter(|(a, b)| i16::from(a[1]) - i16::from(b[1]) > 30)
        .count();
    eprintln!("{model_key} placement grid: {grid_pixels} distinct visible pixels");
    assert!(
        grid_pixels > 100,
        "placement grid missing or obscured: {grid_pixels}"
    );
    let assets = app.world().resource::<FootprintAssets>();
    assert_eq!(assets.cell_size, footprint_cell_size(grid_cfg.as_ref()));
    drop(frames);

    // With the cursor ghost hidden, the existing station, constructor and map
    // region must still render their own footprints. Disarming removes all.
    app.world_mut()
        .get_mut::<Visibility>(root)
        .unwrap()
        .set_if_neq(Visibility::Hidden);
    let span = radius * 0.65;
    let mut view = (*app.world().resource::<HudFeed>().view).clone();
    view.units[0].pos = sta_sim::Vec3::from_f32([-span, 0.0, 0.0]);
    let mut station = view.units[0].clone();
    station.id = sta_sim::EntityId(2);
    station.def = db.key("gpu_object").unwrap();
    station.pos = sta_sim::Vec3::from_f32([span, 0.0, 0.0]);
    view.units.push(station);
    app.world_mut().resource_mut::<HudFeed>().view = Arc::new(view);
    let mut map = sta_content::MapDef::empty();
    map.regions.push(sta_content::Region {
        kind: sta_content::RegionKind::Nebula,
        odf: "gpu_region".into(),
        shape: sta_content::RegionShape::Sphere {
            centre: sta_sim::Vec3::from_f32([0.0, 0.0, -span]),
            radius: sta_sim::Fixed::from_int(30),
        },
    });
    app.world_mut()
        .spawn(super::super::object_footprints::MapFootprintRegions(
            Arc::new(map),
        ));
    let object_frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    for stage in 0..2 {
        if stage == 1 {
            app.world_mut().resource_mut::<HudState>().placing = None;
        }
        for _ in 0..12 {
            app.update();
        }
        let world = app.world_mut();
        let mut overlays =
            world.query_filtered::<Entity, (With<crate::map::MatchEntity>, With<Mesh3d>)>();
        assert_eq!(overlays.iter(world).count(), if stage == 0 { 3 } else { 0 });
        let output = Arc::clone(&object_frames);
        app.world_mut()
            .spawn(Screenshot::image(target.clone()))
            .observe(move |shot: On<ScreenshotCaptured>| {
                output.lock().unwrap().push(shot.image.clone());
            });
        for _ in 0..60 {
            app.update();
            if object_frames.lock().unwrap().len() > stage {
                break;
            }
        }
        assert_eq!(object_frames.lock().unwrap().len(), stage + 1);
    }
    let objects = object_frames.lock().unwrap();
    for (stage, image) in objects.iter().enumerate() {
        let dynamic = image.clone().try_into_dynamic().unwrap();
        let pixels = dynamic.to_rgba8().pixels().filter(|p| p[2] > 30).count();
        eprintln!("occupied object footprints {stage}: {pixels} visible pixels");
        if stage == 0 {
            assert!(pixels > 100, "existing object footprints are invisible");
        } else {
            assert_eq!(pixels, 0, "footprints remain after leaving placement mode");
        }
        if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
            dynamic
                .save(std::path::Path::new(&dir).join(format!("object-footprints-{stage}.png")))
                .unwrap();
        }
    }
    for (handle, color, texture, alpha) in originals {
        let m = app
            .world()
            .resource::<Assets<StandardMaterial>>()
            .get(&handle)
            .unwrap();
        assert_eq!(m.base_color, color);
        assert_eq!(m.base_color_texture, texture);
        assert_eq!(m.alpha_mode, alpha);
    }
}
