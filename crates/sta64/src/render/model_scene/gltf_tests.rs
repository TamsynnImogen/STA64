//! CON-09: exercise the public VFS loader and the actual scene spawn path.
use super::*;
use crate::render::textures::TextureCache;
use bevy::ecs::world::CommandQueue;
use bevy::image::CompressedImageFormats;
use bevy::mesh::VertexAttributeValues;
use bevy::render::render_resource::TextureFormat;
use sta_assets::ModelFormat;
use std::path::PathBuf;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sta64_gltf_render_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(path.join("sod")).unwrap();
        std::fs::write(
            path.join("sod/fx_cruiser.glb"),
            include_bytes!("../../../../../fixtures/models/fx_cruiser.glb"),
        )
        .unwrap();
        Self(path)
    }
    fn vfs(&self) -> sta_vfs::Vfs {
        let mut vfs = sta_vfs::Vfs::default();
        vfs.push_dir("synthetic".into(), self.0.clone()).unwrap();
        vfs
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn load(vfs: &sta_vfs::Vfs) -> Model {
    let (path, format) = sta_assets::find_model(vfs, "fx_cruiser").unwrap();
    assert_eq!(format, ModelFormat::Glb);
    sta_assets::load_model(vfs, &path, format).unwrap()
}

#[allow(clippy::too_many_arguments)] // Mirrors the shared scene spawn seam under test.
fn spawn_fixture(
    world: &mut World,
    model: &Model,
    vfs: &sta_vfs::Vfs,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    texture_cache: &mut TextureCache,
    parts: &mut PartCache,
    normal_maps: bool,
    unlit: bool,
) -> (SpawnStats, Entity) {
    let root = world
        .spawn((Transform::IDENTITY, Visibility::default()))
        .id();
    let sprites = SpriteTable::default();
    let mut queue = CommandQueue::default();
    let stats = {
        let mut commands = Commands::new(&mut queue, world);
        let mut ctx = SpawnCtx {
            commands: &mut commands,
            meshes,
            materials,
            textures: Loader {
                vfs,
                cache: texture_cache,
                images,
                formats: CompressedImageFormats::NONE,
            },
            team: Color::WHITE,
            borg: false,
            sprites: &sprites,
            damaged: false,
            normal_maps,
            parts: Some(parts),
            model_key: "fx_cruiser.glb".into(),
            unlit,
            scale: 1.0,
        };
        spawn(&mut ctx, model, root)
    };
    queue.apply(world);
    (stats, root)
}

#[test]
fn glb_spawns_textured_pbr_with_authored_channels_and_distinct_cache_modes() {
    let fixture = Fixture::new();
    let vfs = fixture.vfs();
    let model = load(&vfs);
    let mut world = World::new();
    let (mut meshes, mut materials, mut images) = (
        Assets::<Mesh>::default(),
        Assets::<StandardMaterial>::default(),
        Assets::<Image>::default(),
    );
    let (mut texture_cache, mut parts) = (TextureCache::default(), PartCache::default());
    let (lit, _) = spawn_fixture(
        &mut world,
        &model,
        &vfs,
        &mut meshes,
        &mut materials,
        &mut images,
        &mut texture_cache,
        &mut parts,
        true,
        false,
    );
    eprintln!(
        "glTF normal source: {:?}; spawn missing: {:?}",
        model
            .pbr
            .as_ref()
            .and_then(|p| p.materials.values().next())
            .and_then(|m| m.normal_texture),
        lit.missing_textures
    );
    let mut query = world.query::<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>();
    let rendered: Vec<_> = query
        .iter(&world)
        .map(|(m, mat)| (m.0.clone(), mat.0.clone()))
        .collect();
    assert_eq!(rendered.len(), 1);
    let mesh = meshes.get(&rendered[0].0).unwrap();
    assert_eq!(mesh.count_vertices(), 24);
    assert!(
        matches!(mesh.attribute(Mesh::ATTRIBUTE_NORMAL), Some(VertexAttributeValues::Float32x3(v)) if v.len() == 24 && v[0] == [0.0, 0.0, 1.0])
    );
    assert!(
        matches!(mesh.attribute(Mesh::ATTRIBUTE_TANGENT), Some(VertexAttributeValues::Float32x4(v)) if v.len() == 24)
    );
    assert!(
        matches!(mesh.attribute(Mesh::ATTRIBUTE_UV_1), Some(VertexAttributeValues::Float32x2(v)) if v.len() == 24)
    );
    assert!(
        matches!(mesh.attribute(Mesh::ATTRIBUTE_COLOR), Some(VertexAttributeValues::Float32x4(v)) if v.len() == 24)
    );
    let lit_material = materials.get(&rendered[0].1).unwrap();
    eprintln!(
        "spawn normal handle: {:?}; image formats: {:?}",
        lit_material.normal_map_texture,
        lit_material
            .normal_map_texture
            .as_ref()
            .and_then(|h| images.get(h))
            .map(|image| image.texture_descriptor.format)
    );
    assert!(
        lit.missing_textures.is_empty(),
        "{:?}",
        lit.missing_textures
    );
    assert_eq!((lit.meshes, lit.triangles, lit.bump_meshes), (1, 12, 1));
    assert_eq!(lit_material.metallic, 0.2);
    assert_eq!(lit_material.perceptual_roughness, 0.7);
    assert!((lit_material.base_color.to_linear().alpha - 0.9).abs() < 1e-6);
    assert!(matches!(lit_material.alpha_mode, AlphaMode::Mask(c) if (c - 0.5).abs() < 1e-6));
    assert!(lit_material.double_sided && !lit_material.unlit);
    assert_eq!(lit_material.base_color_channel, bevy::mesh::UvChannel::Uv1);
    assert!(lit_material.normal_map_texture.is_some());
    let color = images
        .get(lit_material.base_color_texture.as_ref().unwrap())
        .unwrap();
    let normal = images
        .get(lit_material.normal_map_texture.as_ref().unwrap())
        .unwrap();
    assert_eq!(
        color.texture_descriptor.format,
        TextureFormat::Rgba8UnormSrgb
    );
    assert_eq!(normal.texture_descriptor.format, TextureFormat::Rgba8Unorm);
    let pixels = color.data.as_ref().unwrap();
    assert_eq!(&pixels[0..4], &[225, 35, 25, 255]);
    assert_eq!(&pixels[16..20], &[25, 80, 230, 255]);
    let bevy::image::ImageSampler::Descriptor(sampler) = &color.sampler else {
        panic!("glTF sampler was not preserved");
    };
    assert_eq!(
        sampler.address_mode_u,
        bevy::image::ImageAddressMode::ClampToEdge
    );
    assert_eq!(sampler.mag_filter, bevy::image::ImageFilterMode::Nearest);
    assert!(
        world
            .query::<&crate::render::hulls::HullPart>()
            .iter(&world)
            .next()
            .is_none()
    );

    // Same model/team/scale; the two switches differ in opposite directions.
    let (flat, _) = spawn_fixture(
        &mut world,
        &model,
        &vfs,
        &mut meshes,
        &mut materials,
        &mut images,
        &mut texture_cache,
        &mut parts,
        false,
        true,
    );
    assert_eq!((flat.meshes, flat.triangles), (1, 12));
    let rendered: Vec<_> = query
        .iter(&world)
        .map(|(m, mat)| (m.0.clone(), mat.0.clone()))
        .collect();
    assert_eq!(rendered.len(), 2);
    assert_ne!(
        rendered[0].1, rendered[1].1,
        "cache must distinguish both switches"
    );
    let flat_material = materials.get(&rendered[1].1).unwrap();
    assert!(flat_material.unlit);
    assert!(flat_material.normal_map_texture.is_none());
    assert!(
        world
            .query::<&crate::render::hulls::HullPart>()
            .iter(&world)
            .next()
            .is_none()
    );
}

#[test]
fn gltf_names_do_not_trigger_legacy_damage_or_borg_visibility() {
    use bevy::ecs::system::RunSystemOnce;
    let fixture = Fixture::new();
    let vfs = fixture.vfs();
    let mut model = load(&vfs);
    assert_eq!(model.nodes.len(), 2);
    model.nodes[0].name = "engines".into();
    model.nodes[1].name = "borg".into();
    let mut world = World::new();
    let (mut meshes, mut materials, mut images) = (
        Assets::<Mesh>::default(),
        Assets::<StandardMaterial>::default(),
        Assets::<Image>::default(),
    );
    let (mut textures, mut parts) = (TextureCache::default(), PartCache::default());
    let (stats, root) = spawn_fixture(
        &mut world,
        &model,
        &vfs,
        &mut meshes,
        &mut materials,
        &mut images,
        &mut textures,
        &mut parts,
        false,
        false,
    );
    assert_eq!((stats.meshes, stats.triangles), (1, 12));
    world
        .entity_mut(root)
        .insert(crate::render::units::HullShare(1.0));
    world
        .run_system_once(crate::render::units::show_damage)
        .unwrap();
    let mut nodes = world.query::<(&Name, &Visibility, &NativeNode)>();
    let named: Vec<_> = nodes
        .iter(&world)
        .map(|(name, visibility, _)| (name.as_str().to_owned(), *visibility))
        .collect();
    for expected in ["engines", "borg"] {
        assert!(
            named
                .iter()
                .any(|(name, visibility)| name == expected && *visibility != Visibility::Hidden),
            "{expected} hidden or missing: {named:?}"
        );
    }
}

/// Opt-in real renderer check; never creates a window or starts an input loop.
#[test]
fn glb_vfs_draws_checker_offscreen() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 for the windowless GLB draw test");
        return;
    }
    use bevy::asset::RenderAssetUsages;
    use bevy::camera::RenderTarget;
    use bevy::render::RenderPlugin;
    use bevy::render::render_resource::TextureFormat;
    use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
    use bevy::window::ExitCondition;
    use std::sync::Mutex;

    static FRAME: Mutex<Option<Vec<u8>>> = Mutex::new(None);
    const SIZE: u32 = 128;
    let fixture = Fixture::new();
    let vfs = fixture.vfs();
    let model = load(&vfs);
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
    app.insert_resource(ClearColor(Color::srgb(0.01, 0.01, 0.025)));
    let mut meshes = app.world_mut().remove_resource::<Assets<Mesh>>().unwrap();
    let mut materials = app
        .world_mut()
        .remove_resource::<Assets<StandardMaterial>>()
        .unwrap();
    let mut images = app.world_mut().remove_resource::<Assets<Image>>().unwrap();
    let mut target_image =
        Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
    target_image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = images.add(target_image);
    let mut cache = TextureCache::default();
    let mut parts = PartCache::default();
    let (stats, _) = spawn_fixture(
        app.world_mut(),
        &model,
        &vfs,
        &mut meshes,
        &mut materials,
        &mut images,
        &mut cache,
        &mut parts,
        false,
        true,
    );
    assert_eq!((stats.meshes, stats.triangles), (1, 12));
    assert!(
        stats.missing_textures.is_empty(),
        "{:?}",
        stats.missing_textures
    );
    app.world_mut().insert_resource(meshes);
    app.world_mut().insert_resource(materials);
    app.world_mut().insert_resource(images);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    app.finish();
    app.cleanup();
    for _ in 0..10 {
        app.update();
    }
    let info = app
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
    if let Some(info) = info {
        eprintln!(
            "GPU adapter: {} ({:?}, {:?})",
            info.name, info.device_type, info.backend
        );
        if crate::bench::frames::is_software_adapter(info.device_type, &info.name) {
            eprintln!("SKIP: only a software adapter is available ({})", info.name);
            return;
        }
    } else {
        eprintln!("SKIP: no render adapter");
        return;
    }
    *FRAME.lock().unwrap() = None;
    app.world_mut()
        .spawn(Screenshot::image(target))
        .observe(|c: On<ScreenshotCaptured>| {
            *FRAME.lock().unwrap() = c.image.data.clone();
        });
    for _ in 0..40 {
        app.update();
        if FRAME.lock().unwrap().is_some() {
            break;
        }
    }
    let frame = FRAME
        .lock()
        .unwrap()
        .take()
        .expect("offscreen GLB frame did not return");
    assert_eq!(frame.len(), (SIZE * SIZE * 4) as usize);
    if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
        std::fs::create_dir_all(&dir).unwrap();
        let image = Image::new(
            bevy::render::render_resource::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            bevy::render::render_resource::TextureDimension::D2,
            frame.clone(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image
            .try_into_dynamic()
            .unwrap()
            .to_rgb8()
            .save(format!("{dir}/gltf-checker.png"))
            .unwrap();
    }
    let (mut red, mut blue) = (0, 0);
    for px in frame.chunks_exact(4) {
        if px[0] > 100 && u16::from(px[0]) > u16::from(px[2]) * 2 {
            red += 1;
        }
        if px[2] > 100 && u16::from(px[2]) > u16::from(px[0]) * 2 {
            blue += 1;
        }
    }
    assert!(
        red > 30 && blue > 30,
        "GLB checker not rendered: red={red}, blue={blue}"
    );
}
