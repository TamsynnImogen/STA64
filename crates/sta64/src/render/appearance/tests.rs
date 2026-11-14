//! Synthetic runtime seams for instance appearance. No installed game data.

use super::*;
use sta_assets::model::{Group, Maps, Mesh as SourceMesh, Node, NodeKind, SourceFormat};
use sta_content::{ContentDb, MapDef, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId};
use sta_sim::{
    Sim, SimEvent,
    command::{Command, CommandKind, Edit, LocalRouter},
    setup::{Control, GameSetup, PlayerSetup},
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
            "sta64_appearance_{}_{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .bytes()
                .fold(0_u64, |hash, byte| hash
                    .wrapping_mul(131)
                    .wrapping_add(u64::from(byte)))
        ));
        std::fs::create_dir_all(&path).unwrap();
        let write = |name: &str, data: &str| {
            let path = path.join("Data").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, data).unwrap();
        };
        write(
            "odf/other/races.odf",
            "numberOfRaces = 2\nrace0 = \"alpha.odf\"\nrace1 = \"beta.odf\"\n",
        );
        write(
            "odf/other/alpha.odf",
            "name = \"alphaNode\"\ninstantActionSlot = 0\nfactionTextureSuffix = \"_a\"\n",
        );
        write(
            "odf/other/beta.odf",
            "name = \"betaNode\"\ninstantActionSlot = 1\nfactionTextureSuffix = \"_b\"\n",
        );
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        tga[12] = 1;
        tga[14] = 1;
        tga[16] = 24;
        tga[17] = 0x20;
        tga.extend_from_slice(&[255, 0, 0]);
        std::fs::create_dir_all(path.join("Data/Textures/RGB")).unwrap();
        for name in ["paint_a.tga", "paint_b.tga"] {
            std::fs::write(path.join("Data/Textures/RGB").join(name), &tga).unwrap();
        }
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

fn unit(id: u32, owner: u8) -> UnitView {
    UnitView {
        id: EntityId(id),
        def: ContentKey::NONE,
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
    }
}

fn view(owner_a: u8, owner_b: u8, hp: Option<i32>) -> WorldView {
    let mut view = WorldView::default();
    view.units = vec![unit(1, owner_a), unit(2, owner_b)];
    view.appearance.factions = vec![(PlayerId(0), "alpha".into()), (PlayerId(1), "beta".into())];
    if let Some(hp) = hp {
        let system = sta_sim::appearance::SystemAppearance {
            enabled: true,
            health: [Fixed::from_int(hp), Fixed::from_int(10)],
        };
        view.appearance.systems = vec![(EntityId(1), [Some(system), None, None, None, None])];
    }
    view
}

fn source_model() -> Arc<Model> {
    let group = Group {
        material: "plain".into(),
        blend: "default".into(),
        maps: Maps {
            diffuse: Some("paint".into()),
            ..Default::default()
        },
        faces: vec![[(0, 0), (1, 1), (2, 2)]],
    };
    let mesh = SourceMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        uvs: vec![[0.0, 0.0]; 3],
        groups: vec![group],
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
            node("root", None, vec![1, 3, 4, 5], NodeKind::Null),
            node("engine", Some(0), vec![2], NodeKind::Mesh(mesh)),
            node("engineChild", Some(1), vec![], NodeKind::Null),
            node("sibling", Some(0), vec![], NodeKind::Null),
            node("alphaNode", Some(0), vec![], NodeKind::Null),
            node("betaNode", Some(0), vec![], NodeKind::Null),
        ],
        channels: Vec::new(),
        flipbooks: Vec::new(),
        warnings: Vec::new(),
        pbr: None,
    })
}

fn definition() -> UnitDef {
    let mut def = UnitDef::new("synthetic", "craft");
    def.raw = sta_odf::resolve("synthetic", &mut |_| {
        Some("engineMesh1 = \"engine\"\n".into())
    })
    .unwrap()
    .into();
    def
}

fn app(install: &Install, first: WorldView) -> (App, [Entity; 2], [Vec<Entity>; 2], Entity) {
    let model = source_model();
    let def = definition();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(install.mount());
    app.insert_resource(SimLink::new(
        Box::new(Views(Arc::new(first))),
        Box::new(LocalRouter::new()),
        PlayerId(0),
    ));
    app.init_resource::<Assets<StandardMaterial>>();
    app.init_resource::<Assets<Image>>();
    app.init_resource::<super::super::textures::TextureCache>();
    app.add_message::<effects::PartEffect>();
    app.add_systems(Update, sync);
    let base = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    let mut roots = Vec::new();
    let mut nodes_by_root = Vec::new();
    let mut part = Entity::PLACEHOLDER;
    for id in 1..=2 {
        let root = app
            .world_mut()
            .spawn((
                SimEntity(EntityId(id)),
                ModelInstance::new(ContentKey::NONE, model.clone(), 1.0, &def),
                HullShare(1.0),
                TeamColour(Color::WHITE),
            ))
            .id();
        let mut nodes = Vec::new();
        for (index, source) in model.nodes.iter().enumerate() {
            let entity = app
                .world_mut()
                .spawn((
                    ModelNode { root, index },
                    Name::new(source.name.clone()),
                    Transform::IDENTITY,
                    Visibility::Inherited,
                    ChildOf(if index == 0 {
                        root
                    } else {
                        nodes[source.parent.unwrap()]
                    }),
                ))
                .id();
            nodes.push(entity);
        }
        if id == 1 {
            part = app
                .world_mut()
                .spawn((
                    ModelPart {
                        node: 1,
                        group: 0,
                        base: base.clone(),
                    },
                    Transform::IDENTITY,
                    ChildOf(nodes[1]),
                ))
                .id();
        }
        roots.push(root);
        nodes_by_root.push(nodes);
    }
    (
        app,
        [roots[0], roots[1]],
        [nodes_by_root.remove(0), nodes_by_root.remove(0)],
        part,
    )
}

fn texture(app: &App, part: Entity) -> Option<Handle<Image>> {
    let private = app.world().get::<PrivateMaterial>(part)?;
    app.world()
        .resource::<Assets<StandardMaterial>>()
        .get(&private.handle)
        .and_then(|material| material.base_color_texture.clone())
}

fn faction_nodes(app: &App, nodes: &[Entity]) -> (Visibility, Visibility) {
    (
        *app.world().get::<Visibility>(nodes[4]).unwrap(),
        *app.world().get::<Visibility>(nodes[5]).unwrap(),
    )
}

#[test]
fn mission_unit_race_wins_and_survives_capture_until_its_row_is_absent() {
    let install = Install::new();
    let baseline = view(0, 1, None);
    let (mut app, _, nodes, part) = app(&install, baseline.clone());
    app.update();
    let alpha_texture = texture(&app, part).unwrap();
    assert_eq!(
        faction_nodes(&app, &nodes[0]),
        (Visibility::Inherited, Visibility::Hidden)
    );

    let mut override_view = baseline.clone();
    override_view.attrs.races = vec![(EntityId(1), "beta".into())];
    override_view.attrs.player_races = vec![(PlayerId(0), "alpha".into())];
    override_view.digest = 37;
    let supplied = Arc::new(override_view.clone());
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(supplied.clone());
    app.update();
    let beta_texture = texture(&app, part).unwrap();
    assert_ne!(beta_texture, alpha_texture);
    assert_eq!(
        faction_nodes(&app, &nodes[0]),
        (Visibility::Hidden, Visibility::Inherited)
    );
    assert_eq!(
        *supplied, override_view,
        "rendering must not change the supplied view or digest"
    );

    let mut captured = override_view.clone();
    captured.units[0].owner = PlayerId(1);
    captured
        .attrs
        .player_races
        .push((PlayerId(1), "alpha".into()));
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(captured.clone()));
    app.update();
    assert_eq!(texture(&app, part), Some(beta_texture.clone()));
    assert_eq!(
        faction_nodes(&app, &nodes[0]),
        (Visibility::Hidden, Visibility::Inherited)
    );

    captured.attrs.races.clear();
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(captured));
    app.update();
    assert_eq!(texture(&app, part), Some(alpha_texture.clone()));
    assert_eq!(
        faction_nodes(&app, &nodes[0]),
        (Visibility::Inherited, Visibility::Hidden)
    );
}

#[test]
fn recipient_seat_override_controls_capture_and_aliases_keep_material_stable() {
    let install = Install::new();
    let (mut app, _, nodes, part) = app(&install, view(0, 1, None));
    app.update();
    let first_private = app
        .world()
        .get::<PrivateMaterial>(part)
        .unwrap()
        .handle
        .clone();
    let alpha_texture = texture(&app, part).unwrap();

    let mut changed = view(0, 1, None);
    changed.attrs.player_races = vec![(PlayerId(0), "BeTa".into()), (PlayerId(1), "ALPHA".into())];
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(changed.clone()));
    app.update();
    let beta_texture = texture(&app, part).unwrap();
    assert_ne!(beta_texture, alpha_texture);
    assert_eq!(
        faction_nodes(&app, &nodes[0]),
        (Visibility::Hidden, Visibility::Inherited)
    );

    changed.attrs.player_races[0].1 = "BETANODE".into();
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(changed.clone()));
    app.update();
    assert_eq!(
        app.world().get::<PrivateMaterial>(part).unwrap().handle,
        first_private
    );
    assert_eq!(texture(&app, part), Some(beta_texture));

    changed.units[0].owner = PlayerId(1);
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(changed));
    app.update();
    assert_eq!(texture(&app, part), Some(alpha_texture));
    assert_eq!(
        faction_nodes(&app, &nodes[0]),
        (Visibility::Inherited, Visibility::Hidden)
    );
}

#[test]
fn explicit_unknown_or_empty_race_falls_back_and_later_recovers() {
    let install = Install::new();
    let (mut app, _, nodes, part) = app(&install, view(0, 1, None));
    app.update();
    let alpha_texture = texture(&app, part).unwrap();
    let mut changed = view(0, 1, None);
    changed.attrs.player_races = vec![(PlayerId(0), "beta".into())];
    for race in ["unknown", ""] {
        changed.attrs.races = vec![(EntityId(1), race.into())];
        app.world_mut()
            .resource_mut::<SimLink>()
            .feed(Arc::new(changed.clone()));
        app.update();
        assert_eq!(
            texture(&app, part),
            None,
            "explicit {race:?} must not use the seat"
        );
        assert_eq!(
            faction_nodes(&app, &nodes[0]),
            (Visibility::Hidden, Visibility::Hidden)
        );
    }
    changed.attrs.races = vec![(EntityId(1), "ALPHANODE".into())];
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(changed));
    app.update();
    assert_eq!(texture(&app, part), Some(alpha_texture));
    assert_eq!(
        faction_nodes(&app, &nodes[0]),
        (Visibility::Inherited, Visibility::Hidden)
    );
}

#[test]
fn missing_faction_texture_uses_base_without_changing_another_instance() {
    let install = Install::new();
    std::fs::remove_file(install.0.join("Data/Textures/RGB/paint_b.tga")).unwrap();
    let (mut app, _, nodes, first_part) = app(&install, view(0, 1, None));
    let base = app
        .world()
        .get::<ModelPart>(first_part)
        .unwrap()
        .base
        .clone();
    let second_part = app
        .world_mut()
        .spawn((
            ModelPart {
                node: 1,
                group: 0,
                base: base.clone(),
            },
            Transform::IDENTITY,
            ChildOf(nodes[1][1]),
        ))
        .id();
    app.update();
    let alpha_texture = texture(&app, first_part).unwrap();
    let second_private = app
        .world()
        .get::<PrivateMaterial>(second_part)
        .unwrap()
        .handle
        .clone();
    assert_eq!(
        texture(&app, second_part),
        None,
        "missing beta variant retains base map"
    );
    assert_eq!(
        faction_nodes(&app, &nodes[1]),
        (Visibility::Hidden, Visibility::Inherited)
    );
    assert!(
        app.world()
            .resource::<Assets<StandardMaterial>>()
            .get(&base)
            .unwrap()
            .base_color_texture
            .is_none()
    );

    let mut changed = view(0, 1, None);
    changed.attrs.races = vec![(EntityId(1), "beta".into())];
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(changed));
    app.update();
    assert_eq!(texture(&app, first_part), None);
    assert_eq!(texture(&app, second_part), None);
    assert_eq!(
        app.world()
            .get::<PrivateMaterial>(second_part)
            .unwrap()
            .handle,
        second_private
    );
    assert_eq!(
        faction_nodes(&app, &nodes[0]),
        (Visibility::Hidden, Visibility::Inherited)
    );

    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(view(0, 1, None)));
    app.update();
    assert_eq!(texture(&app, first_part), Some(alpha_texture));
    assert_eq!(texture(&app, second_part), None);
}

#[test]
fn simulation_race_edits_and_snapshot_feed_the_same_rendered_appearance() {
    let install = Install::new();
    let mut alpha = UnitDef::new("alpha_ship", "craft");
    alpha.hull = Fixed::from_int(100);
    alpha.race = "alpha".into();
    let mut beta = UnitDef::new("beta_ship", "craft");
    beta.hull = Fixed::from_int(100);
    beta.race = "beta".into();
    let content = Arc::new(ContentDb::builder().unit(alpha).unit(beta).build().unwrap());
    let setup = GameSetup {
        seed: 7,
        map: Arc::new(MapDef::empty()),
        content: content.clone(),
        players: vec![
            PlayerSetup {
                id: PlayerId(0),
                team: TeamId(0),
                race: "alpha".into(),
                control: Control::Idle,
            },
            PlayerSetup {
                id: PlayerId(1),
                team: TeamId(1),
                race: "beta".into(),
                control: Control::Idle,
            },
        ],
        options: Default::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let spawn = |sim: &mut Sim, name: &str, owner| {
        sim.spawn_def(
            content.key(name).unwrap(),
            owner,
            sta_core::Vec3::ZERO,
            (Fixed::ZERO, Fixed::ONE),
        )
        .unwrap()
    };
    let first = spawn(&mut sim, "alpha_ship", PlayerId(0));
    let second = spawn(&mut sim, "beta_ship", PlayerId(1));
    assert_eq!((first, second), (EntityId(1), EntityId(2)));
    let (mut app, _, nodes, part) = app(&install, sim.view().as_ref().clone());
    app.update();
    let alpha_texture = texture(&app, part).unwrap();

    let edit = |sim: &mut Sim, edit| {
        let out = sim.step(&[Command {
            player: PlayerId::SCRIPT,
            seq: 0,
            kind: CommandKind::Edit(edit),
        }]);
        assert!(
            !out.events
                .iter()
                .any(|event| matches!(event, SimEvent::Rejected { .. }))
        );
    };
    edit(&mut sim, Edit::PlayerRace(PlayerId(0), "BETA".into()));
    let seat_view = sim.view();
    app.world_mut().resource_mut::<SimLink>().feed(seat_view);
    app.update();
    let beta_texture = texture(&app, part).unwrap();
    assert_ne!(beta_texture, alpha_texture);
    assert_eq!(
        faction_nodes(&app, &nodes[0]),
        (Visibility::Hidden, Visibility::Inherited)
    );

    edit(&mut sim, Edit::UnitRace(first, "ALPHA".into()));
    let before_render = sim.digest();
    let live = sim.view();
    assert_eq!(live.attrs.player_races, vec![(PlayerId(0), "beta".into())]);
    assert_eq!(live.attrs.races, vec![(first, "alpha".into())]);
    app.world_mut().resource_mut::<SimLink>().feed(live.clone());
    app.update();
    assert_eq!(texture(&app, part), Some(alpha_texture.clone()));
    assert_eq!(
        faction_nodes(&app, &nodes[0]),
        (Visibility::Inherited, Visibility::Hidden)
    );
    assert_eq!(sim.digest(), before_render);
    assert_eq!(live.digest, before_render);

    let saved = sim.snapshot();
    let restored = Sim::from_snapshot(&saved, Some(content)).unwrap();
    assert_eq!(restored.digest(), before_render);
    assert_eq!(restored.view().attrs, live.attrs);
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(restored.view());
    app.update();
    assert_eq!(texture(&app, part), Some(alpha_texture));
    assert_eq!(
        faction_nodes(&app, &nodes[0]),
        (Visibility::Inherited, Visibility::Hidden)
    );
}

#[test]
fn per_instance_faction_node_gating_and_capture() {
    let install = Install::new();
    let (mut app, _, nodes, _) = app(&install, view(0, 1, None));
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(nodes[0][4]),
        Some(&Visibility::Inherited)
    );
    assert_eq!(
        app.world().get::<Visibility>(nodes[1][4]),
        Some(&Visibility::Hidden)
    );
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(view(1, 0, None)));
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(nodes[0][4]),
        Some(&Visibility::Hidden)
    );
    assert_eq!(
        app.world().get::<Visibility>(nodes[1][4]),
        Some(&Visibility::Inherited)
    );
}

#[test]
fn simultaneous_factions_use_private_materials_and_capture_restores_base() {
    let install = Install::new();
    let (mut app, _, nodes, first_part) = app(&install, view(0, 1, None));
    let base = app
        .world()
        .get::<ModelPart>(first_part)
        .unwrap()
        .base
        .clone();
    let normal = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::default());
    app.world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .get_mut(&base)
        .unwrap()
        .normal_map_texture = Some(normal.clone());
    app.world_mut()
        .entity_mut(first_part)
        .insert(super::super::sprites::UvFlipbook {
            animation: "synthetic".into(),
            offset: 0.0,
            material: base.clone(),
        });
    let second_part = app
        .world_mut()
        .spawn((
            ModelPart {
                node: 1,
                group: 0,
                base,
            },
            Transform::IDENTITY,
            ChildOf(nodes[1][1]),
        ))
        .id();
    app.update();
    let first_private = app
        .world()
        .get::<PrivateMaterial>(first_part)
        .unwrap()
        .handle
        .clone();
    let second_private = app
        .world()
        .get::<PrivateMaterial>(second_part)
        .unwrap()
        .handle
        .clone();
    assert_ne!(first_private, second_private);
    assert_eq!(
        app.world()
            .get::<super::super::sprites::UvFlipbook>(first_part)
            .unwrap()
            .material,
        first_private
    );
    let materials = app.world().resource::<Assets<StandardMaterial>>();
    let a = materials.get(&first_private).unwrap();
    let b = materials.get(&second_private).unwrap();
    assert!(a.base_color_texture.is_some() && b.base_color_texture.is_some());
    assert_ne!(a.base_color_texture, b.base_color_texture);
    assert_eq!(a.normal_map_texture.as_ref(), Some(&normal));
    assert_eq!(b.normal_map_texture.as_ref(), Some(&normal));
    let beta_texture = b.base_color_texture.clone();
    let animated_uv = bevy::math::Affine2::from_translation(Vec2::new(0.25, 0.5));
    app.world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .get_mut(&first_private)
        .unwrap()
        .uv_transform = animated_uv;
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(view(1, 1, None)));
    app.update();
    let captured = app
        .world()
        .resource::<Assets<StandardMaterial>>()
        .get(&first_private)
        .unwrap();
    assert_eq!(
        captured.base_color_texture, beta_texture,
        "capture switches faction suffix"
    );
    assert_eq!(captured.normal_map_texture.as_ref(), Some(&normal));
    assert_eq!(captured.uv_transform, animated_uv);
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(view(2, 1, None)));
    app.update();
    let materials = app.world().resource::<Assets<StandardMaterial>>();
    let a = materials.get(&first_private).unwrap();
    assert!(
        a.base_color_texture.is_none(),
        "unknown captured faction restores the base map"
    );
    assert_eq!(a.normal_map_texture.as_ref(), Some(&normal));
    assert!(
        materials
            .get(&second_private)
            .unwrap()
            .base_color_texture
            .is_some()
    );
}

#[test]
fn destroyed_part_hides_only_its_subtree_and_repair_scales_root() {
    let install = Install::new();
    let (mut app, _, nodes, part) = app(&install, view(0, 1, Some(10)));
    app.update();
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(view(0, 1, Some(0))));
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(nodes[0][1]),
        Some(&Visibility::Hidden)
    );
    assert_eq!(
        app.world().get::<Visibility>(nodes[0][2]),
        Some(&Visibility::Hidden)
    );
    assert_eq!(
        app.world().get::<Visibility>(nodes[0][3]),
        Some(&Visibility::Inherited)
    );
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(view(0, 1, Some(5))));
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(nodes[0][1]),
        Some(&Visibility::Inherited)
    );
    assert_eq!(
        app.world().get::<Visibility>(nodes[0][2]),
        Some(&Visibility::Hidden)
    );
    assert!((app.world().get::<Transform>(part).unwrap().scale.x - 0.5).abs() < 1e-6);
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(view(0, 1, Some(10))));
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(nodes[0][2]),
        Some(&Visibility::Inherited)
    );
    assert_eq!(app.world().get::<Transform>(part).unwrap().scale, Vec3::ONE);
}

#[test]
fn appearance_keeps_construction_decor_hidden_until_complete() {
    let install = Install::new();
    let mut first = view(0, 1, None);
    first.units[0].progress = Some(sta_core::Fixed::from_f64(0.5));
    let (mut app, roots, nodes, _) = app(&install, first);
    app.world_mut().entity_mut(nodes[0][3]).insert((
        super::super::construction::ConstructionDecor { owner: roots[0] },
        Visibility::Hidden,
    ));
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(nodes[0][3]),
        Some(&Visibility::Hidden)
    );
    app.world_mut()
        .resource_mut::<SimLink>()
        .feed(Arc::new(view(0, 1, None)));
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(nodes[0][3]),
        Some(&Visibility::Inherited)
    );
}
