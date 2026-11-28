use super::*;
use std::sync::Arc;

use bevy::asset::AssetPlugin;
use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::RunSystemOnce;
use bevy::mesh::{Indices, PrimitiveTopology};
use sta_assets::a2fmt::spr::SpriteTable;
use sta_sim::command::{Command, CommandKind, Edit, LocalRouter, Order};
use sta_sim::production::view::{ConstructionSiteView, ProductionJobView, ProductionView};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Fixed, Sim, TickOutput, WorldSource, WorldView};

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
        portal: None,
        research: None,
        officer_upgrades: None,
        builder: EntityId(builder),
        jobs: vec![ProductionJobView {
            queue_id: None,
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
fn gate_uses_the_authored_aperture_at_the_launch_pose() {
    let mut portal = sta_content::UnitDef::new("fx_gate", "wormhole_obj");
    portal.raw = Arc::new(sta_odf::resolve("fx_gate", &mut |_| {
        Some("ClosedScale = 0.125\nOpenScale = 2.375\n".into())
    }).unwrap());
    let fake = sta_content::fake::db();
    let mut builder = sta_content::ContentDb::builder().unit(portal);
    for unit in fake.units() {
        builder = builder.unit(unit.clone());
    }
    for weapon in fake.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    let db = builder.build().unwrap();
    let view = source_view(&db);
    let product = db.key("fx_cruiser").unwrap();
    let gate = db.key("fx_gate").unwrap();
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .insert_resource(ContentRes(Arc::new(db)))
        .insert_resource(LocalPlayer(Some(PlayerId(0))))
        .add_systems(Update, (sync_build_visuals, sync_launch_portals));
    let mut launch = row(1, product, 1.0, true, true, None);
    launch.site = None;
    let pos = sta_sim::Vec3::from_f32([10.0, 30.0, 50.0]);
    launch.portal = Some(LaunchPortalView {
        def: gate,
        pos,
        heading: (Fixed::ONE, Fixed::ZERO),
        open: Fixed::from_f64(0.5),
    });
    install_link(&mut app, view, vec![launch]);
    app.update();
    assert!(roots(&mut app).is_empty());
    let world = app.world_mut();
    let (model, transform) = world.query_filtered::<(&NeedsModel, &Transform), With<LaunchPortal>>()
        .single(world).unwrap();
    assert_eq!(model.0, gate);
    assert_eq!(transform.translation, to_bevy(pos.to_f32()));
    assert_eq!(transform.scale, Vec3::splat(1.25));
    assert!(transform.forward().abs_diff_eq(Vec3::X, 0.0001));
}

#[test]
fn a_loaded_cocoon_conceals_the_embryo_after_one_third_and_cancel_restores_it() {
    let db = sta_content::fake::db();
    let product = db.key("fx_cruiser").unwrap();
    let view = source_view(&db);
    let mut app = App::new();
    app.add_systems(Update, sync_embryo_meshes);
    let parent = app.world_mut().spawn(SimEntity(EntityId(1))).id();
    let mesh = app.world_mut().spawn((EmbryoMesh, Visibility::Inherited, ChildOf(parent))).id();
    let visual = BuildVisual { builder: EntityId(1), def: product, kind: Kind::Cocoon, owner: PlayerId(0) };
    install_link(&mut app, view.clone(), vec![row(1, product, 0.5, true, true, None)]);
    app.update();
    assert_eq!(app.world().get::<Visibility>(mesh), Some(&Visibility::Inherited));

    app.world_mut().spawn((visual, CocoonMesh));
    app.update();
    assert_eq!(app.world().get::<Visibility>(mesh), Some(&Visibility::Hidden));

    install_link(&mut app, view, Vec::new());
    app.update();
    assert_eq!(app.world().get::<Visibility>(mesh), Some(&Visibility::Inherited));
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
fn station_blueprint_survives_travel_then_clears_when_welding_starts() {
    let fake = sta_content::fake::db();
    let mut rig = sta_content::UnitDef::new("fx_rig", "constructionrig");
    rig.race = "fx".into();
    rig.movement = sta_content::MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.hull = Fixed::from_int(100);
    let mut builder = sta_content::ContentDb::builder()
        .profile(fake.profile())
        .features(fake.features().clone())
        .unit(rig);
    for weapon in fake.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    for unit in fake.units() {
        builder = builder.unit(unit.clone());
    }
    let db = Arc::new(builder.build().unwrap());
    let station = db.key("fx_station").unwrap();
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: db.clone(),
        players: vec![PlayerSetup {
            id: PlayerId(0),
            team: sta_sim::TeamId(0),
            race: "fx".into(),
            control: Control::Human,
        }],
        options: GameOptions::default(),
    });
    let step = |sim: &mut Sim, player, kind| {
        sim.step(&[Command {
            player,
            seq: 0,
            kind,
        }])
    };
    let spawn = step(
        &mut sim,
        PlayerId::BENCH,
        CommandKind::Edit(Edit::Spawn {
            odf: db.key("fx_rig").unwrap(),
            player: PlayerId(0),
            pos: sta_sim::Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    );
    let rig = spawn.view.units[0].id;
    step(
        &mut sim,
        PlayerId::BENCH,
        CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(2000))),
    );
    let ordered = step(
        &mut sim,
        PlayerId(0),
        CommandKind::Order(Order::Build {
            builder: rig,
            what: station,
            at: Some(sta_sim::Vec3::from_f32([1000.0, 0.0, 0.0])),
        }),
    );
    assert!(
        !ordered
            .events
            .iter()
            .any(|event| matches!(event, sta_sim::SimEvent::Rejected { .. })),
        "real station build order should be accepted"
    );
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .insert_resource(ContentRes(db))
        .insert_resource(LocalPlayer(Some(PlayerId(0))))
        .add_systems(Update, sync_build_visuals);
    app.insert_resource(SimLink::new(
        Box::new(sim),
        Box::new(LocalRouter::new()),
        PlayerId(0),
    ));
    let pending = &app.world().resource::<SimLink>().production()[0];
    assert!(!pending.jobs[0].started);
    assert!(pending.site.as_ref().unwrap().entity.is_none());
    app.update();
    let blueprint = roots(&mut app)[&rig].0;
    assert!(app.world().get::<PinnedBlueprint>(blueprint).is_some());
    let mut travel_ticks = 0;
    loop {
        app.world_mut().resource_mut::<SimLink>().tick().unwrap();
        app.update();
        let link = app.world().resource::<SimLink>();
        let row = &link.production()[0];
        if let Some(child) = row.site.as_ref().unwrap().entity {
            assert!(row.jobs[0].started);
            assert!(link.curr().unit(child).unwrap().progress.is_some());
            assert!(roots(&mut app).is_empty());
            assert!(app.world().get::<PinnedBlueprint>(blueprint).is_none());
            break;
        }
        assert!(!row.jobs[0].started);
        assert_eq!(roots(&mut app)[&rig].0, blueprint);
        travel_ticks += 1;
        assert!(travel_ticks < 3000, "rig never reached its site");
    }
    assert!(
        travel_ticks > 0,
        "blueprint should survive constructor travel"
    );
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
            "SKIP: set STA64_CONSTRUCTION_TESTS=1 and an install variable (docs/GATED_TESTS.md) for installed model attachment"
        );
        return;
    }
    let Some(found) = sta_testkit::install_with_mod(sta_testkit::Kind::Any) else { return };
    let install = found.root_string();
    let active_mod = found.active_mod.clone();
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

#[test]
fn leaving_a_match_removes_blueprints_ship_hulls_and_sparks_without_a_link() {
    let db = sta_content::fake::db();
    let cruiser = db.key("fx_cruiser").unwrap();
    let station = db.key("fx_station").unwrap();
    let view = source_view(&db);
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<FxQuadMaterial>>()
        .init_resource::<ConstructionEdges>()
        .init_resource::<crate::render::particles::Particles>()
        .init_resource::<crate::sim_link::SimEntities>()
        .insert_resource(AnimClock::default())
        .insert_resource(ContentRes(Arc::new(db)))
        .insert_resource(LocalPlayer(Some(PlayerId(0))));
    install_link(
        &mut app,
        view,
        vec![
            row(1, cruiser, 0.4, true, true, None),
            row(3, station, 0.0, true, false, None),
        ],
    );
    app.world_mut().run_system_once(sync_build_visuals).unwrap();
    app.world_mut().flush();
    let built = roots(&mut app);
    assert_eq!(built.len(), 2);
    let ship = built[&EntityId(1)].0;
    app.world_mut()
        .resource_mut::<ConstructionEdges>()
        .previews
        .insert(ship, vec![Vec3::ONE]);
    app.world_mut().run_system_once(sync_sparks).unwrap();
    app.world_mut().flush();
    assert_eq!(
        app.world_mut()
            .query::<&YardSpark>()
            .iter(app.world())
            .count(),
        3
    );
    // This is the real front-end transition: SimLink is removed before the
    // map's match-entity sweep, so per-tick production systems cannot clean up.
    app.world_mut().remove_resource::<SimLink>();
    app.world_mut()
        .run_system_once(crate::map::clear_match)
        .unwrap();
    app.world_mut().flush();
    assert!(roots(&mut app).is_empty());
    assert_eq!(
        app.world_mut()
            .query::<&YardSpark>()
            .iter(app.world())
            .count(),
        0
    );
    assert_eq!(
        app.world_mut()
            .query::<&PinnedBlueprint>()
            .iter(app.world())
            .count(),
        0
    );
}
