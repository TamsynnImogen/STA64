use super::*;
use std::sync::Arc;

use bevy::asset::AssetPlugin;
use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::RunSystemOnce;
use bevy::mesh::{Indices, PrimitiveTopology};
use sta_assets::a2fmt::spr::SpriteTable;
use sta_sim::command::{Command, LocalRouter};
use sta_sim::production::view::{ConstructionSiteView, ProductionJobView, ProductionView};
use sta_sim::{Fixed, TickOutput, WorldSource, WorldView};

struct Frozen {
    view: Arc<WorldView>,
    production: Vec<ProductionView>,
}

impl WorldSource for Frozen {
    fn view(&self) -> Arc<WorldView> {
        self.view.clone()
    }
    fn step(&mut self, _: &[Command]) -> TickOutput {
        TickOutput {
            view: self.view.clone(),
            events: Vec::new(),
        }
    }
    fn production_view(&self) -> Vec<ProductionView> {
        self.production.clone()
    }
}

fn source_view(db: &sta_content::ContentDb) -> Arc<WorldView> {
    sta_sim::fake::ScriptedSource::from_json5(
        r#"{units:[
            {id:1,def:"fx_yard",owner:0,hull:1000,rails:[{at:0,pos:{x:0,y:0,z:0}}]},
            {id:2,def:"fx_yard",owner:0,hull:1000,rails:[{at:0,pos:{x:300,y:0,z:0}}]},
            {id:3,def:"fx_cruiser",owner:0,hull:800,rails:[{at:0,pos:{x:600,y:0,z:0}}]},
            {id:4,def:"fx_station",owner:0,hull:1000,rails:[{at:0,pos:{x:900,y:0,z:0}}]}
        ]}"#,
        db,
    )
    .unwrap()
    .view()
}

fn row(
    builder: u32,
    def: ContentKey,
    progress: f32,
    paid: bool,
    started: bool,
    real: Option<EntityId>,
) -> ProductionView {
    ProductionView {
        builder: EntityId(builder),
        jobs: vec![ProductionJobView {
            def,
            progress: Fixed::from_f64(progress as f64),
            paid,
            started,
        }],
        site: Some(ConstructionSiteView {
            def,
            pos: sta_sim::Vec3::from_f32([builder as f32 * 100.0, 0.0, 20.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
            entity: real,
        }),
    }
}

fn install_link(app: &mut App, view: Arc<WorldView>, production: Vec<ProductionView>) {
    app.insert_resource(SimLink::new(
        Box::new(Frozen { view, production }),
        Box::new(LocalRouter::new()),
        PlayerId(0),
    ));
}

fn roots(app: &mut App) -> BTreeMap<EntityId, (Entity, ContentKey, Kind, Option<f32>)> {
    let world = app.world_mut();
    let mut q = world.query::<(Entity, &BuildVisual, Option<&ConstructionPreview>)>();
    q.iter(world)
        .map(|(e, v, p)| (v.builder, (e, v.def, v.kind, p.map(|p| p.0))))
        .collect()
}

#[test]
fn production_roots_follow_head_rewind_cancel_and_independent_yards() {
    let db = sta_content::fake::db();
    let cruiser = db.key("fx_cruiser").unwrap();
    let frigate = db.key("fx_frigate").unwrap();
    let station = db.key("fx_station").unwrap();
    let view = source_view(&db);
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .insert_resource(ContentRes(Arc::new(db)))
        .insert_resource(LocalPlayer(Some(PlayerId(0))))
        .add_systems(Update, sync_build_visuals);
    install_link(
        &mut app,
        view.clone(),
        vec![
            row(1, cruiser, 0.25, true, true, None),
            row(2, frigate, 0.6, true, true, None),
            row(3, station, 0.0, false, false, None),
        ],
    );
    app.update();
    let before = roots(&mut app);
    assert_eq!(before.len(), 3);
    assert_eq!(before[&EntityId(1)].2, Kind::Ship);
    assert_eq!(before[&EntityId(2)].3, Some(0.6));
    assert_eq!(before[&EntityId(3)].2, Kind::Blueprint);
    assert!(app.world().get::<Ghost>(before[&EntityId(3)].0).is_some());
    for (_, (e, _, _, _)) in &before {
        assert!(app.world().get::<crate::sim_link::SimEntity>(*e).is_none());
        assert!(
            app.world()
                .get::<crate::render::units::SelectRadius>(*e)
                .is_none()
        );
    }
    // A replaced head and a rewinding job each get a new model root. A third
    // independent builder keeps its blueprint and never grows bee entities.
    install_link(
        &mut app,
        view.clone(),
        vec![
            row(1, frigate, 0.1, true, true, None),
            row(2, frigate, 0.3, true, true, None),
            row(3, station, 0.0, false, false, None),
        ],
    );
    app.update();
    let after = roots(&mut app);
    assert_ne!(after[&EntityId(1)].0, before[&EntityId(1)].0);
    assert_ne!(after[&EntityId(2)].0, before[&EntityId(2)].0);
    assert_eq!(after[&EntityId(3)].0, before[&EntityId(3)].0);
    assert_eq!(after[&EntityId(1)].1, frigate);
    install_link(
        &mut app,
        view.clone(),
        vec![row(2, frigate, 0.4, true, true, None)],
    );
    app.update();
    let final_roots = roots(&mut app);
    assert_eq!(final_roots.len(), 1);
    assert_eq!(final_roots[&EntityId(2)].0, after[&EntityId(2)].0);
    let mut dead_view = (*view).clone();
    dead_view.units.retain(|unit| unit.id != EntityId(2));
    install_link(
        &mut app,
        Arc::new(dead_view),
        vec![row(2, frigate, 0.5, true, true, None)],
    );
    app.update();
    assert!(roots(&mut app).is_empty());
}

#[test]
fn replacing_head_drops_private_partial_mesh_asset() {
    let db = sta_content::fake::db();
    let cruiser = db.key("fx_cruiser").unwrap();
    let frigate = db.key("fx_frigate").unwrap();
    let view = source_view(&db);
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .insert_resource(ContentRes(Arc::new(db)))
        .insert_resource(LocalPlayer(Some(PlayerId(0))))
        .add_systems(Update, sync_build_visuals);
    install_link(
        &mut app,
        view.clone(),
        vec![row(1, cruiser, 0.2, true, true, None)],
    );
    app.update();
    let root = roots(&mut app)[&EntityId(1)].0;
    let original = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_indices(Indices::U32(vec![0, 1, 2]));
    let (original, partial) = {
        let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
        (meshes.add(original.clone()), meshes.add(original))
    };
    let source = Arc::new(crate::render::construction::ConstructionSource {
        stage_indices: vec![vec![0, 1, 2]],
        stage_edges: vec![vec![Vec3::X]],
    });
    app.world_mut().spawn((
        ChildOf(root),
        Mesh3d(partial.clone()),
        GlobalTransform::default(),
        ConstructionPart {
            owner: root,
            original: original.clone(),
            partial: partial.clone(),
            source,
            completed: 0,
        },
    ));
    install_link(&mut app, view, vec![row(1, frigate, 0.0, true, true, None)]);
    app.update();
    let meshes = app.world().resource::<Assets<Mesh>>();
    assert!(meshes.get(&partial).is_none());
    assert!(meshes.get(&original).is_some());
}

#[test]
fn hidden_yard_does_not_disclose_its_construction() {
    let db = sta_content::fake::db();
    let cruiser = db.key("fx_cruiser").unwrap();
    let mut view = (*source_view(&db)).clone();
    view.visibility = vec![(PlayerId(0), vec![EntityId(1), EntityId(3)])];
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .insert_resource(ContentRes(Arc::new(db)))
        .insert_resource(LocalPlayer(Some(PlayerId(0))))
        .add_systems(Update, sync_build_visuals);
    install_link(
        &mut app,
        Arc::new(view),
        vec![
            row(1, cruiser, 0.2, true, true, None),
            row(2, cruiser, 0.5, true, true, None),
        ],
    );
    app.update();
    let visible = roots(&mut app);
    assert_eq!(visible.len(), 1);
    assert!(visible.contains_key(&EntityId(1)));
}

#[test]
fn welded_station_blueprint_follows_station_after_builder_death_and_capture() {
    let db = sta_content::fake::db();
    let station = db.key("fx_station").unwrap();
    let mut view = (*source_view(&db)).clone();
    view.units.retain(|unit| unit.id != EntityId(3));
    let station_unit = view
        .units
        .iter_mut()
        .find(|unit| unit.id == EntityId(4))
        .unwrap();
    station_unit.progress = Some(Fixed::from_f64(0.4));
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .insert_resource(ContentRes(Arc::new(db)))
        .insert_resource(LocalPlayer(Some(PlayerId(0))))
        .add_systems(Update, sync_build_visuals);
    install_link(
        &mut app,
        Arc::new(view.clone()),
        vec![row(3, station, 0.4, true, true, Some(EntityId(4)))],
    );
    app.update();
    let before = roots(&mut app);
    assert_eq!(before.len(), 1);
    assert_eq!(before[&EntityId(3)].2, Kind::Blueprint);
    let station_unit = view
        .units
        .iter_mut()
        .find(|unit| unit.id == EntityId(4))
        .unwrap();
    station_unit.owner = PlayerId(1);
    install_link(
        &mut app,
        Arc::new(view),
        vec![row(3, station, 0.5, true, true, Some(EntityId(4)))],
    );
    app.update();
    assert!(roots(&mut app).is_empty());
}

#[test]
fn sparks_live_only_on_published_preview_edges() {
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<FxQuadMaterial>>()
        .init_resource::<ConstructionEdges>()
        .insert_resource(AnimClock::default());
    let root = app.world_mut().spawn(ConstructionPreview(0.4)).id();
    app.world_mut()
        .resource_mut::<ConstructionEdges>()
        .previews
        .insert(
            root,
            vec![Vec3::new(5.0, 6.0, 7.0), Vec3::new(8.0, 9.0, 10.0)],
        );
    app.world_mut().run_system_once(sync_sparks).unwrap();
    app.world_mut().flush();
    let world = app.world_mut();
    let mut q = world.query::<(&YardSpark, &Transform)>();
    let sparks: Vec<_> = q
        .iter(world)
        .map(|(s, t)| (s.root, t.translation))
        .collect();
    assert_eq!(sparks.len(), 3);
    assert!(
        sparks.iter().all(|(r, at)| *r == root
            && [Vec3::new(5.0, 6.0, 7.0), Vec3::new(8.0, 9.0, 10.0)].contains(at))
    );
    app.world_mut().entity_mut(root).despawn();
    app.world_mut()
        .resource_mut::<ConstructionEdges>()
        .previews
        .clear();
    app.world_mut().run_system_once(sync_sparks).unwrap();
    app.world_mut().flush();
    let world = app.world_mut();
    assert_eq!(world.query::<&YardSpark>().iter(world).count(), 0);
    assert_eq!(world.resource::<Assets<FxQuadMaterial>>().len(), 0);
}

#[test]
fn installed_ship_preview_attaches_partial_model_without_selection() {
    if std::env::var("STA64_CONSTRUCTION_TESTS").as_deref() != Ok("1") {
        eprintln!(
            "SKIP: set STA64_CONSTRUCTION_TESTS=1 and STA64_TEST_INSTALL for installed model attachment"
        );
        return;
    }
    let install = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
    let active_mod = std::env::var("STA64_TEST_MOD")
        .ok()
        .filter(|m| !m.is_empty());
    let vfs =
        sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = sta_content::ContentDb::load(&vfs, &index).unwrap();
    let model_key =
        std::env::var("STA64_CONSTRUCTION_SHIP_MODEL").unwrap_or_else(|_| "fdestroy".into());
    let def = db
        .units()
        .find(|d| {
            d.model.to_ascii_lowercase().trim_end_matches(".sod") == model_key
                && EntityKind::of_def(d) == EntityKind::Craft
        })
        .expect("installed mobile craft model");
    let key = def.key;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        .insert_resource(crate::viewer::Mounted(vfs))
        .insert_resource(ContentRes(Arc::new(db)))
        .insert_resource(crate::render::sprites::SpriteTables(Arc::new(
            SpriteTable::default(),
        )))
        .init_resource::<crate::render::moons::MoonVariants>()
        .init_resource::<crate::render::textures::TextureCache>()
        .init_resource::<crate::render::model_scene::PartCache>()
        .init_resource::<crate::render::units::UnitModels>();
    install_link(&mut app, Arc::new(WorldView::default()), Vec::new());
    let root = app
        .world_mut()
        .spawn((
            BuildVisual {
                builder: EntityId(1),
                def: key,
                kind: Kind::Ship,
                owner: PlayerId(0),
            },
            ConstructionPreview(0.3),
            NeedsModel(key),
            TeamColour(Color::WHITE),
            Transform::default(),
            Visibility::Inherited,
        ))
        .id();
    app.world_mut()
        .run_system_once(crate::render::units::attach_models)
        .unwrap();
    app.world_mut().flush();
    assert!(app.world().get::<NeedsModel>(root).is_none());
    assert!(
        app.world()
            .get::<crate::render::units::SelectRadius>(root)
            .is_none()
    );
    assert!(
        app.world()
            .get::<crate::sim_link::SimEntity>(root)
            .is_none()
    );
    let world = app.world_mut();
    let mut parts = world.query::<&ConstructionPart>();
    assert!(
        parts.iter(world).any(|part| part.owner == root),
        "installed craft should attach private partial hull geometry"
    );
}
