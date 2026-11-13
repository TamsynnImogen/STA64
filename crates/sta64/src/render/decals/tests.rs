//! Runtime projection checks with authored synthetic geometry and image bytes.

use super::*;
use bevy::mesh::Indices;
use sta_assets::model::{Group, Maps, Mesh as SourceMesh, Model, Node, NodeKind, SourceFormat};
use sta_content::UnitDef;
use sta_core::{EntityId, Fixed, PlayerId};
use sta_sim::{
    command::{Command, LocalRouter},
    source::WorldSource,
    view::{TickOutput, UnitFlags, UnitView, WorldView},
};
use std::{path::PathBuf, sync::Arc};

struct Views(Arc<WorldView>);
impl WorldSource for Views {
    fn step(&mut self, _: &[Command]) -> TickOutput {
        TickOutput {
            view: self.0.clone(),
            events: Vec::new(),
        }
    }
    fn view(&self) -> Arc<WorldView> {
        self.0.clone()
    }
}

struct Install(PathBuf);
impl Install {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sta64_decal_runtime_{}_{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .bytes()
                .fold(0_u64, |hash, byte| hash
                    .wrapping_mul(131)
                    .wrapping_add(u64::from(byte)))
        ));
        std::fs::create_dir_all(path.join("Data/Textures/RGB")).unwrap();
        // One opaque top-down blue pixel in an uncompressed 24-bit TGA.
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        tga[12] = 1;
        tga[14] = 1;
        tga[16] = 24;
        tga[17] = 0x20;
        tga.extend_from_slice(&[255, 0, 0]);
        std::fs::write(path.join("Data/Textures/RGB/scorch.tga"), tga).unwrap();
        Self(path)
    }
    fn mount(&self) -> Mounted {
        Mounted(sta_vfs::Vfs::mount_install(&self.0, None).unwrap())
    }
}
impl Drop for Install {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn definition() -> UnitDef {
    let mut def = UnitDef::new("synthetic", "craft");
    def.model = "synthetic".into();
    def.raw = sta_odf::resolve("synthetic", &mut |_| {
        Some(
            concat!(
                "hullScorch1 = \"scorch\"\n",
                "hullScorch1Hardpoint = \"anchor\"\n",
                "hullScorch1Size = \"2 2\"\n",
                "hullScorch1Depth = 2\n",
                "damageDecalPreview = 1\n",
            )
            .into(),
        )
    })
    .unwrap()
    .into();
    def
}

fn model() -> Arc<Model> {
    let mesh = SourceMesh {
        positions: vec![
            [-1.5, -0.7, 0.0],
            [-1.5, 0.7, 0.0],
            [0.0, -0.7, 0.2],
            [0.0, 0.7, 0.2],
            [1.5, -0.7, 0.0],
            [1.5, 0.7, 0.0],
        ],
        // Deliberately unrelated UVs: projection must derive its own.
        uvs: vec![[9.0, -4.0]; 6],
        groups: vec![Group {
            material: "plain".into(),
            blend: "default".into(),
            maps: Maps::default(),
            faces: vec![
                [(0, 0), (1, 1), (2, 2)],
                [(2, 2), (1, 1), (3, 3)],
                [(2, 2), (3, 3), (4, 4)],
                [(4, 4), (3, 3), (5, 5)],
            ],
        }],
        two_sided: false,
    };
    let node = |name: &str, parent: Option<usize>, children: Vec<usize>, kind: NodeKind| Node {
        name: name.into(),
        parent,
        children,
        transform: sta_assets::a2fmt::Matrix34::IDENTITY,
        kind,
    };
    Arc::new(Model {
        format: SourceFormat::M3d,
        version: 2.0,
        materials: Vec::new(),
        nodes: vec![
            node("anchor", None, vec![1], NodeKind::Null),
            node("hull", Some(0), vec![], NodeKind::Mesh(mesh)),
        ],
        channels: Vec::new(),
        flipbooks: Vec::new(),
        warnings: Vec::new(),
        pbr: None,
    })
}

fn view(visible: bool, building: bool) -> WorldView {
    let mut view = WorldView::default();
    let unit = UnitView {
        id: EntityId(1),
        def: sta_core::ContentKey::NONE,
        owner: PlayerId(0),
        pos: sta_core::Vec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        speed: Fixed::ZERO,
        hull: [Fixed::from_int(100); 2],
        shields: [[Fixed::ZERO; 2]; 4],
        flags: UnitFlags::default(),
        progress: building.then_some(Fixed::from_f64(0.5)),
        crew: None,
        energy: None,
        autonomy: None,
    };
    view.units.push(unit);
    if !visible {
        view.visibility = vec![(PlayerId(0), vec![])];
    }
    view
}

fn app(install: &Install) -> (App, Entity, Entity, Entity) {
    let def = definition();
    let db = sta_content::ContentDb::builder().unit(def).build().unwrap();
    let key = db.key("synthetic").unwrap();
    let model = model();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin));
    app.insert_resource(install.mount());
    app.insert_resource(ContentRes(Arc::new(db)));
    app.insert_resource(SimLink::new(
        Box::new(Views(Arc::new(view(true, false)))),
        Box::new(LocalRouter::new()),
        PlayerId(0),
    ));
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.init_resource::<Assets<Image>>();
    app.init_resource::<super::super::textures::TextureCache>();
    app.init_resource::<Cache>();
    app.add_systems(
        PostUpdate,
        update.after(bevy::transform::TransformSystems::Propagate),
    );
    let def = app
        .world()
        .resource::<ContentRes>()
        .0
        .unit(key)
        .unwrap()
        .clone();
    let root = app
        .world_mut()
        .spawn((
            SimEntity(EntityId(1)),
            ModelInstance::new(key, model, 1.0, &def),
            Transform::IDENTITY,
        ))
        .id();
    let anchor = app
        .world_mut()
        .spawn((
            ModelNode { root, index: 0 },
            Transform::IDENTITY,
            Visibility::Inherited,
            ChildOf(root),
        ))
        .id();
    let receiver = app
        .world_mut()
        .spawn((
            ModelNode { root, index: 1 },
            Transform::IDENTITY,
            Visibility::Inherited,
            ChildOf(anchor),
        ))
        .id();
    let part = app
        .world_mut()
        .spawn((
            ModelPart {
                node: 1,
                group: 0,
                base: Handle::default(),
            },
            Transform::IDENTITY,
            ChildOf(receiver),
        ))
        .id();
    (app, root, receiver, part)
}

fn one_projection(app: &mut App) -> (Entity, Handle<Mesh>) {
    let mut query = app
        .world_mut()
        .query::<(Entity, &ProjectionPart, &Mesh3d)>();
    let rows: Vec<_> = query
        .iter(app.world())
        .map(|(entity, _, mesh)| (entity, mesh.0.clone()))
        .collect();
    assert_eq!(rows.len(), 1, "expected exactly one clipped projection");
    rows[0].clone()
}

#[test]
fn update_projects_multiple_source_triangles_with_new_uvs_and_reuses_mesh() {
    let install = Install::new();
    let (mut app, root, receiver, _) = app(&install);
    app.update();
    let (decal, handle) = one_projection(&mut app);
    let mesh = app.world().resource::<Assets<Mesh>>().get(&handle).unwrap();
    let Indices::U32(indices) = mesh.indices().unwrap() else {
        panic!("u32 indices expected")
    };
    assert!(indices.len() > 3, "must cover multiple receiver triangles");
    let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) =
        mesh.attribute(Mesh::ATTRIBUTE_UV_0)
    else {
        panic!("projection UVs missing")
    };
    assert!(
        uvs.iter()
            .all(|uv| (0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]))
    );
    assert!(uvs.iter().all(|uv| *uv != [9.0, -4.0]));
    app.world_mut()
        .entity_mut(root)
        .insert(Transform::from_xyz(10.0, 0.0, 0.0));
    app.update();
    let (same, same_handle) = one_projection(&mut app);
    assert_eq!(same, decal);
    assert_eq!(same_handle, handle);
    app.world_mut()
        .entity_mut(receiver)
        .insert(Transform::from_xyz(0.2, 0.0, 0.0));
    app.update();
    let (refreshed, _) = one_projection(&mut app);
    assert_ne!(
        refreshed, decal,
        "relative receiver animation rebuilds geometry"
    );
    assert!(
        app.world().resource::<Assets<Mesh>>().len() <= 1,
        "old projection mesh handle must be released"
    );
}

#[test]
fn construction_and_hidden_receiver_hide_projection_without_free_quad() {
    let install = Install::new();
    let (mut app, _, receiver, _) = app(&install);
    app.update();
    let (decal, _) = one_projection(&mut app);
    app.world_mut()
        .entity_mut(receiver)
        .insert(Visibility::Hidden);
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(decal),
        Some(&Visibility::Hidden)
    );
    app.world_mut()
        .entity_mut(receiver)
        .insert(Visibility::Inherited);
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(view(true, true)));
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(decal),
        Some(&Visibility::Hidden)
    );
}

#[test]
fn despawned_owner_releases_projection_cache_entries() {
    let install = Install::new();
    let (mut app, root, _, _) = app(&install);
    app.update();
    one_projection(&mut app);
    assert_eq!(app.world().resource::<Cache>().geometry.len(), 1);
    app.world_mut().entity_mut(root).despawn();
    app.update();
    assert!(app.world().resource::<Cache>().geometry.is_empty());
    assert!(app.world().resource::<Cache>().sources.is_empty());
    assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 0);
    let mut query = app.world_mut().query::<&ProjectionPart>();
    assert_eq!(query.iter(app.world()).count(), 0);
}

#[test]
fn refreshed_projection_has_receiver_world_transform_on_its_first_frame() {
    let install = Install::new();
    let (mut app, root, receiver, part) = app(&install);
    app.world_mut()
        .entity_mut(root)
        .insert(Transform::from_xyz(40.0, 2.0, -15.0));
    app.update();
    let (first, _) = one_projection(&mut app);
    let expected = app
        .world()
        .get::<GlobalTransform>(part)
        .unwrap()
        .to_matrix();
    assert!(
        app.world()
            .get::<GlobalTransform>(first)
            .unwrap()
            .to_matrix()
            .abs_diff_eq(expected, 1e-6)
    );
    app.world_mut()
        .entity_mut(receiver)
        .insert(Transform::from_xyz(0.2, 0.0, 0.0));
    app.update();
    let (second, _) = one_projection(&mut app);
    assert_ne!(first, second);
    let expected = app
        .world()
        .get::<GlobalTransform>(part)
        .unwrap()
        .to_matrix();
    assert!(
        app.world()
            .get::<GlobalTransform>(second)
            .unwrap()
            .to_matrix()
            .abs_diff_eq(expected, 1e-6)
    );
}

#[test]
fn registry_images_and_text_follow_original_rows_with_explicit_orientation_control() {
    let install = Install::new();
    std::fs::create_dir_all(install.0.join("Data/Sprites")).unwrap();
    std::fs::write(install.0.join("Data/Sprites/FontSmall.spr"),
        "@animation f_uv\n@keyframes\n0 0\n0 0\n\n@animation f_w\n@keyframes\n0\n1\n\npagea\tscorch\t1\t@anim=f_uv\t@anim=f_w\n").unwrap();
    let mounted = install.mount();
    let mut images = Assets::<Image>::default();
    let mut cache = super::super::textures::TextureCache::default();
    let mut materials = Assets::<StandardMaterial>::default();
    let mut loader = super::super::textures::Loader {
        vfs: &mounted.0,
        images: &mut images,
        cache: &mut cache,
        formats: bevy::image::CompressedImageFormats::BC,
    };
    let mut seen = [false; 2];
    for flip in [false, true] {
        let raw = format!(
            "possibleCraftNames = \"First\" \"\" \"Second\"\npossibleCraftRegistry = \"!\" \"unselected\" \"!!\"\nlogoFileNames = \"scorch\" \"wrong\" \"\"\nlogoDecal1Hardpoint = \"anchor\"\nlogoDecal2Hardpoint = \"anchor\"\nlogoDecal2Text = \"registry\"\nlogoDecal2FlipU = {}\n",
            u8::from(flip)
        );
        let mut def = UnitDef::new("synthetic", "craft");
        def.raw = sta_odf::resolve("synthetic", &mut |_| Some(raw.clone()))
            .unwrap()
            .into();
        let instance = ModelInstance::new(sta_core::ContentKey::NONE, model(), 1.0, &def);
        for id in 1..=16 {
            let identity = crate::ui::hud::model::craft_names::identity(&def, EntityId(id));
            let first = identity.name == "First";
            seen[usize::from(!first)] = true;
            let output = sources(&instance, EntityId(id), &def, &mut loader, &mut materials);
            assert_eq!(
                output.len(),
                if first { 2 } else { 1 },
                "blank image rows stay blank"
            );
            if first {
                assert!(!output[0].flip_u, "image U convention is preserved");
            }
            let text = output.last().unwrap();
            assert_eq!(
                text.flip_u, !flip,
                "generated text reads from the receiving side; FlipU reverses it"
            );
            let material = materials.get(&text.material).unwrap();
            let image = loader
                .images
                .get(material.base_color_texture.as_ref().unwrap())
                .unwrap();
            assert_eq!(
                image.texture_descriptor.size.width,
                if first { 1 } else { 2 },
                "text raster must follow the same original name row"
            );
        }
    }
    assert_eq!(seen, [true, true]);
}
