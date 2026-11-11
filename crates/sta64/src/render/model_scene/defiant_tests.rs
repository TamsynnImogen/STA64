//! Install-gated, headless trace of the Classic Federation Defiant model.

use super::*;
use bevy::asset::AssetPlugin;
use bevy::ecs::system::RunSystemOnce;
use bevy::image::CompressedImageFormats;

#[test]
fn classic_defiant_has_visible_opaque_hull_parts() {
    if std::env::var("STA64_DEFIANT_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_DEFIANT_TESTS=1 for the Classic Defiant fixture");
        return;
    }
    let (Ok(install), Ok(mod_name)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else {
        eprintln!("SKIP: set STA64_TEST_INSTALL and STA64_TEST_MOD (empty for base install)");
        return;
    };
    let vfs = sta_vfs::Vfs::mount_install(
        std::path::Path::new(&install),
        Some(mod_name.as_str()).filter(|name| !name.is_empty()),
    )
    .unwrap();
    let model_key = std::env::var("STA64_TEST_MODEL").unwrap_or_else(|_| "fdestroy".into());
    let (path, format) = sta_assets::find_model(&vfs, &model_key).expect("requested model in selected install/mod");
    let model = sta_assets::load_model(&vfs, &path, format).unwrap();
    println!(
        "Fixture model: {path} {format:?} v{}; {} nodes",
        model.version,
        model.nodes.len()
    );
    println!(
        "coarse LOD nodes: {:?}",
        model
            .coarse_lods()
            .iter()
            .map(|&i| model.nodes[i].name.as_str())
            .collect::<Vec<_>>()
    );
    for (i, node) in model.nodes.iter().enumerate() {
        println!(
            "node {i}: {} parent={:?} local={:?}",
            node.name,
            node.parent.map(|p| model.nodes[p].name.as_str()),
            node.transform
        );
        if let Some(mesh) = node.mesh() {
            for group in &mesh.groups {
                println!(
                    "  group {} blend={} faces={} diffuse={:?} self_illum={:?}",
                    group.material,
                    group.blend,
                    group.faces.len(),
                    group.maps.diffuse,
                    model.lighting_material(&group.material).map(|m| m.self_illuminating),
                );
            }
        }
    }
    for channel in &model.channels {
        let keys = match &channel.keys {
            Keys::Matrix(k) => k.len(),
            Keys::Scale(k) => k.len(),
        };
        println!("channel {}: period={} keys={keys}", channel.node, channel.period);
    }

    let diffuse = model
        .nodes
        .iter()
        .filter_map(|n| n.mesh())
        .flat_map(|m| &m.groups)
        .find(|g| g.blend == "opaque" && !g.faces.is_empty() && g.maps.diffuse.is_some())
        .and_then(|g| g.maps.diffuse.clone())
        .expect("an opaque hull group has a diffuse texture");
    let (texture_path, texture_format) =
        sta_assets::find_texture(&vfs, &diffuse).expect("Defiant diffuse texture resolves");
    let decoded = sta_assets::decode_rgba(&vfs, &texture_path, texture_format).unwrap();
    let alpha = decoded.pixels.chunks_exact(4).map(|px| px[3]).collect::<Vec<_>>();
    let (min_alpha, max_alpha) = (*alpha.iter().min().unwrap(), *alpha.iter().max().unwrap());
    println!(
        "diffuse {diffuse}: {texture_path} {}x{} alpha={min_alpha}..{max_alpha}",
        decoded.width, decoded.height
    );

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>();
    let root = app
        .world_mut()
        .spawn((Name::new("Defiant probe"), Transform::IDENTITY, Visibility::default()))
        .id();
    let stats = app
        .world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  mut meshes: ResMut<Assets<Mesh>>,
                  mut materials: ResMut<Assets<StandardMaterial>>,
                  mut images: ResMut<Assets<Image>>| {
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
                        formats: CompressedImageFormats::empty(),
                    },
                    team: Color::WHITE,
                    borg: false,
                    sprites: &sprites,
                    damaged: false,
                    normal_maps: false,
                    parts: None,
                    model_key: path.clone(),
                    unlit: false,
                    scale: 1.0,
                };
                let stats = spawn(&mut ctx, &model, root);
                (stats, cache.map)
            },
        )
        .unwrap();
    let (stats, texture_cache) = stats;
    println!(
        "spawn: meshes={} triangles={} missing={:?} centre={:?} radius={}",
        stats.meshes, stats.triangles, stats.missing_textures, stats.centre, stats.radius
    );
    assert!(stats.meshes > 0 && stats.triangles > 0, "Defiant spawned no geometry");
    assert!(stats.centre.is_finite() && stats.radius.is_finite() && stats.radius > 0.0);
    assert!(
        !stats
            .missing_textures
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&diffuse)),
        "Defiant diffuse was not loaded"
    );
    let cache_key = format!("c:0:{}", diffuse.to_ascii_lowercase());
    let image_handle = texture_cache
        .get(&cache_key)
        .and_then(Option::as_ref)
        .expect("Defiant diffuse uploaded");
    let uploaded = app
        .world()
        .resource::<Assets<Image>>()
        .get(image_handle)
        .expect("uploaded diffuse image asset");
    assert_eq!(
        &uploaded.data.as_ref().expect("CPU image bytes")[..decoded.pixels.len()],
        decoded.pixels.as_slice(),
        "diffuse RGBA including alpha changed during upload"
    );

    let world = app.world_mut();
    let mut q = world.query::<(
        Entity,
        &Mesh3d,
        &MeshMaterial3d<StandardMaterial>,
        Option<&crate::render::hulls::HullPart>,
    )>();
    let parts = q
        .iter(world)
        .map(|(e, mesh, material, hull)| (e, mesh.0.clone(), material.0.clone(), hull.copied()))
        .collect::<Vec<_>>();
    assert_eq!(parts.len(), stats.meshes);
    let mut opaque_hulls = 0;
    for (entity, mesh, material, hull) in parts {
        let mut current = entity;
        let mut names = Vec::new();
        for _ in 0..=model_node_limit(world) {
            if current == root {
                break;
            }
            let parent = world
                .get::<ChildOf>(current)
                .expect("mesh part must have a parent")
                .parent();
            names.push(format!(
                "{} {:?}",
                world.get::<Name>(parent).map(|n| n.as_str()).unwrap_or("<mesh part>"),
                world.get::<Transform>(parent)
            ));
            current = parent;
        }
        assert_eq!(current, root, "mesh part is not beneath probe root");
        let mesh_asset = world.resource::<Assets<Mesh>>().get(&mesh).expect("spawned mesh asset");
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh_asset.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("spawned mesh has no 3D positions");
        };
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for &position in positions {
            let position = Vec3::from(position);
            lo = lo.min(position);
            hi = hi.max(position);
        }
        assert!(lo.is_finite() && hi.is_finite(), "spawned mesh bounds are non-finite");
        let material_asset = world
            .resource::<Assets<StandardMaterial>>()
            .get(&material)
            .expect("spawned material asset");
        println!(
            "part {entity:?}: parents={names:?} vertices={} local_bounds={lo:?}..{hi:?} alpha={:?} texture={:?} hull={hull:?}",
            positions.len(),
            material_asset.alpha_mode,
            material_asset.base_color_texture
        );
        if material_asset.alpha_mode == AlphaMode::Opaque && material_asset.base_color_texture.is_some() {
            let hull = hull.expect("opaque Defiant mesh should be a hull part");
            assert!(!hull.cut_alpha, "opaque hull must not use texture alpha as cut-out");
            opaque_hulls += 1;
        }
    }
    assert!(opaque_hulls > 0, "no textured opaque hull parts were spawned");
}

fn model_node_limit(world: &World) -> usize {
    // A guard against accidentally following a malformed parent cycle.
    world.entities().len() as usize
}
