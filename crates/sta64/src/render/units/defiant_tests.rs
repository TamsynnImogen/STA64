//! Install-gated, headless trace of unit model attachment after construction.

use super::*;
use bevy::asset::AssetPlugin;
use bevy::ecs::system::RunSystemOnce;
use sta_assets::a2fmt::spr::SpriteTable;

#[test]
fn installed_defiant_unit_attaches_visible_hull() {
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
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = sta_content::ContentDb::load(&vfs, &index).unwrap();
    let model_key = std::env::var("STA64_TEST_MODEL").unwrap_or_else(|_| "fdestroy".into());
    let defs = db
        .units()
        .filter(|def| def.model.to_ascii_lowercase().trim_end_matches(".sod") == model_key)
        .collect::<Vec<_>>();
    for def in &defs {
        println!(
            "Fixture definition: odf={} model={} ScaleSOD={} key={:?}",
            def.odf,
            def.model,
            def.scale.to_f32(),
            def.key
        );
    }
    let def = defs.first().expect("installed ContentDb has an fdestroy unit");
    let key = def.key;
    let expected_scale = if def.scale > sta_sim::Fixed::ZERO {
        def.scale.to_f32()
    } else {
        1.0
    };

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        .insert_resource(Mounted(vfs))
        .insert_resource(ContentRes(Arc::new(db)))
        .insert_resource(SpriteTables(Arc::new(SpriteTable::default())))
        .init_resource::<crate::render::moons::MoonVariants>()
        .init_resource::<TextureCache>()
        .init_resource::<PartCache>()
        .init_resource::<UnitModels>();
    let root = app
        .world_mut()
        .spawn((
            Name::new("constructed Defiant"),
            NeedsModel(key),
            SimEntity(EntityId(123)),
            UnitOwner(PlayerId(0)),
            TeamColour(Color::WHITE),
            HullShare(1.0),
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .id();
    app.world_mut().run_system_once(attach_models).unwrap();
    app.world_mut().run_system_once(show_damage).unwrap();
    let world = app.world_mut();
    assert!(world.get::<NeedsModel>(root).is_none(), "unit attachment did not run");
    let models = world.resource::<UnitModels>();
    let (path, model) = models
        .models
        .get(&model_key)
        .and_then(Option::as_ref)
        .expect("fdestroy was loaded by unit attachment");
    println!(
        "attached model: {path}; {} nodes; expected ScaleSOD={expected_scale}",
        model.nodes.len()
    );
    assert!(
        world.get::<SelectRadius>(root).is_some(),
        "attached unit has no selection radius"
    );

    let mut q = world.query::<(
        Entity,
        &Mesh3d,
        &MeshMaterial3d<StandardMaterial>,
        &crate::render::hulls::HullPart,
    )>();
    let parts = q
        .iter(world)
        .map(|(e, mesh, material, hull)| (e, mesh.0.clone(), material.0.clone(), *hull))
        .collect::<Vec<_>>();
    assert!(!parts.is_empty(), "attached Defiant has no hull mesh parts");
    let mut visible = 0;
    for (part, mesh, material, hull) in parts {
        let mut current = part;
        let mut hidden = false;
        let mut chain = Vec::new();
        for _ in 0..=world.entities().len() {
            let vis = world.get::<Visibility>(current).copied();
            hidden |= vis == Some(Visibility::Hidden);
            chain.push(format!(
                "{} {:?} {:?}",
                world.get::<Name>(current).map(|n| n.as_str()).unwrap_or("<part>"),
                world.get::<Transform>(current),
                vis
            ));
            if current == root {
                break;
            }
            current = world
                .get::<ChildOf>(current)
                .expect("hull part missing parent")
                .parent();
        }
        assert_eq!(current, root, "hull part not attached to unit root");
        assert!(
            world.resource::<Assets<Mesh>>().get(&mesh).is_some(),
            "hull mesh asset missing"
        );
        let mat = world
            .resource::<Assets<StandardMaterial>>()
            .get(&material)
            .expect("hull material asset");
        println!(
            "unit hull {part:?}: chain={chain:?} hidden={hidden} alpha={:?} texture={:?} hull={hull:?}",
            mat.alpha_mode, mat.base_color_texture
        );
        if !hidden {
            visible += 1;
        }
    }
    assert!(visible > 0, "all attached Defiant hull parts are hidden");
}

/// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, `STA64_TEST_MAP`, default `4alliance`): a skirmish
/// as the player sees it at the start. Every resource node the local seat sees is drawn from the
/// view the way the client does (`sync_units`, `attach_models`, `ensure_icons`) and must be the
/// unit a click on it picks (left click: selection; right click: Mine order for a miner; the Mine
/// command's target click: the same pick), from a high and from a low camera, whichever resource it
/// holds. Regression of a playtest report: a dilithium moon on STA2 Classic could not be selected.
#[test]
fn install_visible_resource_nodes_are_pickable_and_minable() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
    let map_name = std::env::var("STA64_TEST_MAP").unwrap_or_else(|_| "4alliance".into());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = sta_content::ContentDb::load(&vfs, &index).unwrap();
    let (races, _) = sta_content::Races::load(&vfs, &index);
    let db = Arc::new(db);
    let Some(Ok(data)) = vfs.read(&format!("bzn/{map_name}.bzn")) else { return };
    let bzn = sta_bzn::parse(&data).unwrap();
    let cfg = crate::game::SkirmishCfg { seat: 0, opponent: crate::app::Opponent::Skirmish, race: Some("federation".into()), opponent_race: None };
    let side = crate::map::build_sim(&bzn, &map_name, &db, &index, &vfs, Some(0), Some((&cfg, &races)));
    side.skirmish.expect("a skirmish").unwrap_or_else(|e| panic!("{e}"));
    let mut sim = side.sim;
    // Any mining ship of the install, the first by name (the Classic mod names its miners differently).
    let mut miners: Vec<_> = db.units().filter(|d| d.mines() && d.movement.max_speed > sta_sim::Fixed::ZERO).collect();
    miners.sort_by(|a, b| a.odf.cmp(&b.odf));
    let key = miners.first().expect("a miner in the content").key;
    let rig = sta_sim::WorldSource::view(&sim).units.iter().find(|u| u.owner == PlayerId(0)).expect("a start unit").pos;
    let miner_pos = sta_sim::Vec3::from_f32([rig.x.to_f32() + 150.0, rig.y.to_f32(), rig.z.to_f32()]);
    sim.spawn_def(key, PlayerId(0), miner_pos, (sta_sim::Fixed::ZERO, sta_sim::Fixed::ONE)).expect("spawned");
    let view = sta_sim::WorldSource::view(&sim);
    let miner = view.units.iter().find(|u| u.def == key && u.owner == PlayerId(0)).expect("the miner").id;

    struct Frozen(Arc<sta_sim::WorldView>);
    impl sta_sim::WorldSource for Frozen {
        fn step(&mut self, _: &[sta_sim::command::Command]) -> sta_sim::TickOutput {
            sta_sim::TickOutput { view: self.0.clone(), events: vec![] }
        }
        fn view(&self) -> Arc<sta_sim::WorldView> {
            self.0.clone()
        }
    }
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        .insert_resource(Mounted(vfs))
        .insert_resource(ContentRes(db.clone()))
        .insert_resource(SpriteTables(Arc::new(SpriteTable::default())))
        .init_resource::<crate::render::moons::MoonVariants>()
        .init_resource::<TextureCache>()
        .init_resource::<PartCache>()
        .init_resource::<UnitModels>()
        .init_resource::<SimEntities>()
        .insert_resource(LocalPlayer(Some(PlayerId(0))))
        .insert_resource(crate::sim_link::SimLink::new(Box::new(Frozen(view.clone())), Box::new(sta_sim::command::LocalRouter::new()), PlayerId(0)));
    app.world_mut().run_system_once(sync_units).unwrap();
    app.world_mut().flush();
    app.world_mut().run_system_once(attach_models).unwrap();
    app.world_mut().flush();
    app.world_mut().run_system_once(crate::render::strategic::ensure_icons).unwrap();
    app.world_mut().flush();
    let w = app.world_mut();
    let placed: Vec<(Entity, Transform)> = w.query_filtered::<(Entity, &Transform), With<SimEntity>>().iter(w).map(|(e, t)| (e, *t)).collect();
    for (e, t) in placed {
        w.entity_mut(e).insert(GlobalTransform::from(t));
    }

    let seen: Vec<_> = view.visible_to(PlayerId(0)).into_iter().filter(|u| db.unit(u.def).is_some_and(|d| d.resource.is_some())).collect();
    println!("{} resource nodes visible to seat 0", seen.len());
    assert!(!seen.is_empty(), "the start sees a resource node");
    let mut failed = Vec::new();
    for node in seen {
        let at = to_bevy(node.pos.to_f32());
        for (from, height) in [(Vec3::new(75.0, 780.0, 580.0), "high"), (Vec3::new(40.0, 150.0, 350.0), "low")] {
            let eye = at + from;
            // The cursor a little off the node's centre, as a player clicks.
            let target = at + Vec3::new(10.0, 0.0, 8.0);
            let ray = Ray3d::new(eye, Dir3::new((target - eye).normalize()).unwrap());
            let hit = app.world_mut().run_system_once(move |u: crate::input::select::Targetable| crate::input::select::unit_under(Vec2::ZERO, ray, &u)).unwrap();
            if hit != Some(node.id) {
                failed.push(format!("{:?} {} {height}: picked {hit:?}", node.id, db.unit(node.def).unwrap().odf));
            }
        }
        // The right click (and the Mine command's target click) on that unit.
        let order = crate::input::context::context_order(Some(&*db), &view, PlayerId(0), &[miner], node.id);
        if !matches!(&order, Some(sta_sim::command::Order::Mine { node: n, .. }) if *n == node.id) {
            failed.push(format!("{:?}: context order {order:?}", node.id));
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}
