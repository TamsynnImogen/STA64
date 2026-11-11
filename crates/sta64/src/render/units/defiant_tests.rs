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
    let defs = db
        .units()
        .filter(|def| def.model.to_ascii_lowercase().trim_end_matches(".sod") == "fdestroy")
        .collect::<Vec<_>>();
    for def in &defs {
        println!(
            "Defiant definition: odf={} model={} ScaleSOD={} key={:?}",
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
        .get("fdestroy")
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
