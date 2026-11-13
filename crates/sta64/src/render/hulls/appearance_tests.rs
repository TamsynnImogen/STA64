//! Headless integration of appearance's private StandardMaterial with the hull
//! material adapter. All files and image pixels are synthetic.

use super::*;
use crate::{
    app::AppState,
    clock::AnimClock,
    render::{
        appearance::{AppearancePlugin, ModelInstance, ModelNode, ModelPart, PrivateMaterial},
        model_scene::TeamPart,
        textures::TextureCache,
        units::{HullShare, TeamColour},
    },
    sim_link::{ContentRes, SimEntity, SimLink},
    viewer::Mounted,
};
use sta_assets::model::{Group, Maps, Mesh as SourceMesh, Model, Node, NodeKind, SourceFormat};
use sta_content::{ContentDb, UnitDef};
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
        let root = std::env::temp_dir().join(format!(
            "sta64_hull_appearance_{}_{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .bytes()
                .fold(0_u64, |hash, byte| hash
                    .wrapping_mul(131)
                    .wrapping_add(u64::from(byte)))
        ));
        let write = |name: &str, data: &[u8]| {
            let path = root.join("Data").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, data).unwrap();
        };
        write(
            "odf/other/races.odf",
            b"numberOfRaces = 2\nrace0 = \"alpha.odf\"\nrace1 = \"beta.odf\"\n",
        );
        write(
            "odf/other/alpha.odf",
            b"name = \"alphaNode\"\ninstantActionSlot = 0\nfactionTextureSuffix = \"_a\"\n",
        );
        write(
            "odf/other/beta.odf",
            b"name = \"betaNode\"\ninstantActionSlot = 1\nfactionTextureSuffix = \"_b\"\n",
        );
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        tga[12] = 1;
        tga[14] = 1;
        tga[16] = 24;
        tga[17] = 0x20;
        tga.extend_from_slice(&[0, 0, 255]);
        write("Textures/RGB/paint_a.tga", &tga);
        write("Textures/RGB/paint_b.tga", &tga);
        write("Textures/RGB/glow_b.tga", &tga);
        write("Textures/RGB/spec_b.tga", &tga);
        Self(root)
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

fn view(owner: u8) -> WorldView {
    let mut view = WorldView::default();
    view.units.push(UnitView {
        id: EntityId(1),
        def: sta_core::ContentKey::NONE,
        owner: PlayerId(owner),
        pos: sta_core::Vec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        speed: Fixed::ZERO,
        hull: [Fixed::from_int(100); 2],
        shields: [[Fixed::ZERO; 2]; 4],
        flags: UnitFlags::default(),
        progress: None,
        crew: None,
        energy: None,
        autonomy: None,
    });
    view.appearance.factions = vec![(PlayerId(0), "alpha".into()), (PlayerId(1), "beta".into())];
    view
}

fn model(auxiliary_maps: bool) -> Arc<Model> {
    Arc::new(Model {
        format: SourceFormat::M3d,
        version: 2.0,
        materials: Vec::new(),
        nodes: vec![Node {
            name: "hull".into(),
            parent: None,
            children: Vec::new(),
            transform: sta_assets::a2fmt::Matrix34::IDENTITY,
            kind: NodeKind::Mesh(SourceMesh {
                positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                uvs: vec![[0.0, 0.0]; 3],
                two_sided: false,
                groups: vec![Group {
                    material: "team_plain".into(),
                    blend: "default".into(),
                    maps: Maps {
                        diffuse: Some("paint".into()),
                        glow: auxiliary_maps.then(|| "glow".into()),
                        specular: auxiliary_maps.then(|| "spec".into()),
                        ..Default::default()
                    },
                    faces: vec![[(0, 0), (1, 1), (2, 2)]],
                }],
            }),
        }],
        channels: Vec::new(),
        flipbooks: Vec::new(),
        warnings: Vec::new(),
        pbr: None,
    })
}

#[test]
fn private_faction_material_converts_before_hull_apply_and_follows_capture() {
    run_capture_test(false);
}

#[test]
fn captured_auxiliary_faction_maps_switch_back_to_standard_and_restore_hull() {
    run_capture_test(true);
}

fn run_capture_test(auxiliary_maps: bool) {
    let install = Install::new();
    let mut def = UnitDef::new("synthetic", "craft");
    def.model = "synthetic".into();
    let db = ContentDb::builder().unit(def).build().unwrap();
    let key = db.key("synthetic").unwrap();
    let definition = db.unit(key).unwrap().clone();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin));
    app.init_state::<AppState>();
    app.insert_resource(AnimClock::default());
    app.insert_resource(install.mount());
    app.insert_resource(ContentRes(Arc::new(db)));
    app.insert_resource(SimLink::new(
        Box::new(Views(Arc::new(view(0)))),
        Box::new(LocalRouter::new()),
        PlayerId(0),
    ));
    app.init_resource::<Assets<StandardMaterial>>();
    app.init_resource::<Assets<HullMaterial>>();
    app.init_resource::<Assets<Image>>();
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<TextureCache>();
    app.init_resource::<HullMaterials>();
    app.init_resource::<HullView>();
    app.init_resource::<HullLighting>();
    app.add_message::<AssetEvent<StandardMaterial>>();
    app.add_plugins(AppearancePlugin);
    app.add_systems(Update, (apply.in_set(HullApply), follow_source).chain());
    if auxiliary_maps {
        app.world_mut().resource_mut::<HullView>().set_stock(true);
    }

    let normal = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::default());
    let base = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            normal_map_texture: Some(normal.clone()),
            ..default()
        });
    let root = app
        .world_mut()
        .spawn((
            SimEntity(EntityId(1)),
            ModelInstance::new(key, model(auxiliary_maps), 1.0, &definition),
            HullShare(1.0),
            TeamColour(Color::srgb(0.8, 0.4, 0.2)),
        ))
        .id();
    let node = app
        .world_mut()
        .spawn((
            ModelNode { root, index: 0 },
            Name::new("hull"),
            Transform::IDENTITY,
            Visibility::Inherited,
            ChildOf(root),
        ))
        .id();
    let part = app
        .world_mut()
        .spawn((
            ModelPart {
                node: 0,
                group: 0,
                base,
            },
            TeamPart,
            HullPart {
                stock: auxiliary_maps.then_some(crate::render::stock::hulls::StockLight {
                    ambient: [0.5; 3],
                    diffuse: [0.9, 0.8, 0.7],
                    specular: [0.2; 3],
                    power: 40.0,
                    model: 1,
                }),
                ..default()
            },
            MeshMaterial3d(Handle::<StandardMaterial>::default()),
            Transform::IDENTITY,
            ChildOf(node),
        ))
        .id();
    // The part must initially carry its actual base handle before sync clones it.
    let base_handle = app.world().get::<ModelPart>(part).unwrap().base.clone();
    app.world_mut()
        .entity_mut(part)
        .insert(MeshMaterial3d(base_handle));
    app.update();
    let private = app
        .world()
        .get::<PrivateMaterial>(part)
        .expect("appearance made a private material")
        .handle
        .clone();
    let source = app
        .world()
        .get::<HullSource>(part)
        .expect("same-frame hull conversion");
    assert_eq!(source.source, private);
    let hull_handle = source.hull.clone();
    assert!(
        app.world()
            .get::<MeshMaterial3d<HullMaterial>>(part)
            .is_some()
    );
    let first = app
        .world()
        .resource::<Assets<HullMaterial>>()
        .get(&hull_handle)
        .unwrap();
    let first_texture = first.texture.clone();
    assert!(first_texture.is_some());
    assert_eq!(first.normal.as_ref(), Some(&normal));
    let alpha_texture = first_texture.clone();
    let alpha = app
        .world()
        .resource::<Assets<StandardMaterial>>()
        .get(&private)
        .unwrap();
    assert!(alpha.emissive_texture.is_none());
    assert!(alpha.metallic_roughness_texture.is_none());

    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(view(1)));
    app.world_mut()
        .entity_mut(root)
        .insert(TeamColour(Color::srgb(0.1, 0.7, 0.9)));
    app.update();
    let standard = app
        .world()
        .resource::<Assets<StandardMaterial>>()
        .get(&private)
        .unwrap();
    let beta_texture = standard.base_color_texture.clone();
    assert!(beta_texture.is_some() && beta_texture != first_texture);
    let beta_glow = standard.emissive_texture.clone();
    let beta_specular = standard.metallic_roughness_texture.clone();
    if auxiliary_maps {
        assert!(beta_glow.is_some(), "beta capture loads its glow map");
        assert!(
            beta_specular.is_some(),
            "beta capture loads its specular map"
        );
        assert_eq!(standard.normal_map_texture.as_ref(), Some(&normal));
        assert!(app.world().get::<HullSource>(part).is_none());
        assert!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(part)
                .is_some()
        );
        assert!(
            app.world()
                .get::<MeshMaterial3d<HullMaterial>>(part)
                .is_none()
        );

        // Capturing alpha removes both faction-only maps and makes this same
        // private material eligible for hull conversion in the capture frame.
        app.world_mut()
            .resource_mut::<SimLink>()
            .feed(Arc::new(view(0)));
        app.world_mut()
            .entity_mut(root)
            .insert(TeamColour(Color::srgb(0.8, 0.4, 0.2)));
        app.update();
        let alpha = app
            .world()
            .resource::<Assets<StandardMaterial>>()
            .get(&private)
            .unwrap();
        assert!(alpha.emissive_texture.is_none());
        assert!(alpha.metallic_roughness_texture.is_none());
        assert_eq!(alpha.base_color_texture, alpha_texture);
        assert_eq!(alpha.normal_map_texture.as_ref(), Some(&normal));
        assert_eq!(app.world().get::<HullSource>(part).unwrap().source, private);
        assert!(
            app.world()
                .get::<MeshMaterial3d<HullMaterial>>(part)
                .is_some()
        );
        let source = app.world().get::<HullSource>(part).unwrap();
        let restored = app
            .world()
            .resource::<Assets<HullMaterial>>()
            .get(&source.hull)
            .unwrap();
        assert_eq!(restored.params.mamb.w, 1.0);
        assert_eq!(restored.texture, alpha_texture);
        assert_eq!(restored.normal.as_ref(), Some(&normal));
    } else {
        assert!(beta_glow.is_none());
        assert!(beta_specular.is_none());
        assert_eq!(app.world().get::<HullSource>(part).unwrap().source, private);
        assert_eq!(
            app.world()
                .resource::<Assets<HullMaterial>>()
                .get(&hull_handle)
                .unwrap()
                .texture,
            beta_texture,
            "capture refreshes the cached hull in the same frame"
        );
        // In this minimal app the asset event bridge is explicit; follow_source
        // observes it on the next update, just as the normal AssetPlugin does.
        app.world_mut()
            .write_message(AssetEvent::<StandardMaterial>::Modified { id: private.id() });
        app.update();
        let hull = app
            .world()
            .resource::<Assets<HullMaterial>>()
            .get(&hull_handle)
            .unwrap();
        assert_eq!(hull.texture, beta_texture);
        assert_eq!(hull.normal.as_ref(), Some(&normal));
        let tint = Color::srgb(0.1, 0.7, 0.9).to_linear();
        assert!((hull.params.tint.x - tint.red).abs() < 1e-6);
        assert!((hull.params.tint.y - tint.green).abs() < 1e-6);
        assert!((hull.params.tint.z - tint.blue).abs() < 1e-6);
    }
}
