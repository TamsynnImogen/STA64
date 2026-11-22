//! Display-only occupied footprints while a station build site is being chosen.
//! Craft and station squares use the current construction placement radius;
//! resource and prop squares use their authored body radius when present.
//! Their outer bounds are also used by authoritative build admission.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use sta_content::map::RegionShape;
use sta_content::{ContentDb, MapDef};
use sta_sim::production::site;
use sta_sim::world::EntityKind;
use sta_sim::{ContentKey, EntityId};

use super::HudState;
use super::ghost::{self, FootprintAssets};
use super::model::HudFeed;
use crate::render::fog::FogState;

/// Typed regions belonging to the current map root; removed with that root.
#[derive(Component)]
pub(crate) struct MapFootprintRegions(pub Arc<MapDef>);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FootprintKey {
    Sim(EntityId),
    Pending(EntityId),
    Region(Entity, usize),
}

#[derive(Clone, Copy)]
struct FootprintPose {
    x: f32,
    z: f32,
    half: Vec2,
}

fn region_pose(shape: RegionShape) -> Option<FootprintPose> {
    let region = sta_content::Region {
        kind: sta_content::RegionKind::Nebula,
        odf: String::new(),
        shape,
    };
    let bounds = site::region_bounds(&region)?;
    let (x0, z0, x1, z1) = (
        bounds.min.0.to_f32(),
        bounds.min.1.to_f32(),
        bounds.max.0.to_f32(),
        bounds.max.1.to_f32(),
    );
    Some(FootprintPose {
        x: (x0 + x1) * 0.5,
        z: (z0 + z1) * 0.5,
        half: Vec2::new((x1 - x0) * 0.5, (z1 - z0) * 0.5),
    })
}

fn unit_radius(db: &ContentDb, def: ContentKey, _kind: EntityKind) -> f32 {
    site::obstacle_radius(db, def).to_f32()
}

fn pending_footprints(
    feed: &HudFeed,
    rows: &[sta_sim::production::view::ProductionView],
    replacing: Option<EntityId>,
) -> HashMap<FootprintKey, FootprintPose> {
    let Some(db) = feed.content.as_deref() else {
        return HashMap::new();
    };
    rows.iter()
        .filter(|row| Some(row.builder) != replacing)
        .filter(|row| {
            feed.view
                .unit(row.builder)
                .is_some_and(|unit| unit.owner == feed.player)
        })
        .filter_map(|row| {
            let pending = row.site.as_ref()?;
            let def = db.unit(pending.def)?;
            if pending.entity.is_some()
                || EntityKind::of_def(def) != EntityKind::Station
                || EntityKind::hidden(def)
                || row.jobs.first().is_none_or(|job| job.started)
            {
                return None;
            }
            let radius = site::obstacle_radius(db, pending.def).to_f32();
            Some((
                FootprintKey::Pending(row.builder),
                FootprintPose {
                    x: pending.pos.x.to_f32(),
                    z: pending.pos.z.to_f32(),
                    half: Vec2::splat(radius),
                },
            ))
        })
        .collect()
}

fn wanted(
    feed: &HudFeed,
    maps: &Query<(Entity, &MapFootprintRegions)>,
    fog: Option<&FogState>,
) -> HashMap<FootprintKey, FootprintPose> {
    let mut out = HashMap::new();
    let Some(db) = feed.content.as_deref() else {
        return out;
    };
    for unit in feed.view.visible_to(feed.player) {
        let Some(def) = db.unit(unit.def) else {
            continue;
        };
        let kind = EntityKind::of_def(def);
        if EntityKind::hidden(def) || kind == EntityKind::Projectile {
            continue;
        }
        let radius = unit_radius(db, unit.def, kind);
        let (x, z) = (unit.pos.x.to_f32(), unit.pos.z.to_f32());
        if x.is_finite() && z.is_finite() && radius.is_finite() && radius > 0.0 {
            out.insert(
                FootprintKey::Sim(unit.id),
                FootprintPose {
                    x,
                    z,
                    half: Vec2::splat(radius),
                },
            );
        }
    }
    for (source, map) in maps {
        for (index, region) in map.0.regions.iter().enumerate() {
            let Some(pose) = region_pose(region.shape) else {
                continue;
            };
            if !feed.view.visibility.is_empty()
                && !fog.is_some_and(|f| f.grid.visible_at(pose.x, pose.z))
            {
                continue;
            }
            out.insert(FootprintKey::Region(source, index), pose);
        }
    }
    out
}

/// Reconcile one noninteractive overlay per visible obstacle, and clear all of
/// them when placement or its supporting feed/assets disappear.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_object_footprints(
    mut commands: Commands,
    state: Res<HudState>,
    feed: Res<HudFeed>,
    maps: Query<(Entity, &MapFootprintRegions)>,
    fog: Option<Res<FogState>>,
    link: Option<Res<crate::sim_link::SimLink>>,
    mut assets: Option<ResMut<FootprintAssets>>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    existing: Query<(Entity, &FootprintKey, &Transform, &Mesh3d)>,
) {
    let active_y = ghost::placement_height(&state, &feed);
    if active_y.is_none() || feed.content.is_none() || assets.is_none() || meshes.is_none() {
        for (entity, ..) in &existing {
            commands.entity(entity).despawn();
        }
        return;
    }
    let y = active_y.unwrap() + 0.15;
    let mut desired = wanted(&feed, &maps, fog.as_deref());
    if let Some(link) = link.as_ref() {
        desired.extend(pending_footprints(
            &feed,
            link.production(),
            state.placing.as_ref().map(|armed| armed.builder),
        ));
    }
    let assets = assets.as_mut().unwrap();
    let meshes = meshes.as_mut().unwrap();
    for (entity, key, current, current_mesh) in &existing {
        let Some(pose) = desired.remove(key) else {
            commands.entity(entity).despawn();
            continue;
        };
        let (mesh, scale) = assets.grid(meshes, pose.half);
        let next = Transform::from_xyz(pose.x, y, -pose.z).with_scale(scale);
        if *current != next {
            commands.entity(entity).insert(next);
        }
        if current_mesh.0 != mesh {
            commands.entity(entity).insert(Mesh3d(mesh));
        }
    }
    for (key, pose) in desired {
        let (mesh, scale) = assets.grid(meshes, pose.half);
        commands.spawn((
            Name::new("occupied object footprint"),
            key,
            crate::map::MatchEntity,
            Mesh3d(mesh),
            MeshMaterial3d(assets.occupied.clone()),
            Transform::from_xyz(pose.x, y, -pose.z).with_scale(scale),
            Visibility::Inherited,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_content::{Region, RegionKind, UnitDef};
    use sta_sim::view::{UnitFlags, UnitView};
    use sta_sim::{Fixed, PlayerId, Vec3 as SimVec3, WorldView};

    fn unit(db: &ContentDb, id: u32, odf: &str, owner: u8, x: i32) -> UnitView {
        UnitView {
            id: EntityId(id),
            def: db.key(odf).unwrap(),
            owner: PlayerId(owner),
            pos: SimVec3::from_f32([x as f32, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2],
            shields: [[Fixed::ZERO; 2]; 4],
            flags: UnitFlags::SELECTABLE,
            progress: None,
            crew: None,
            energy: None,
            autonomy: None,
        }
    }

    fn content() -> Arc<ContentDb> {
        let mut hidden = UnitDef::new("hidden", "wormhole_obj");
        hidden.hidden = true;
        let mut planet = UnitDef::new("planet", "planet");
        planet.radius = Fixed::from_int(80);
        let mut gate = UnitDef::new("gate", "wormhole_obj");
        gate.radius = Fixed::from_int(35);
        Arc::new(
            ContentDb::builder()
                .unit(UnitDef::new("rig", "constructionrig"))
                .unit(UnitDef::new("station", "starbase"))
                .unit(planet)
                .unit(UnitDef::new("latinum", "latinum"))
                .unit(gate)
                .unit(hidden)
                .build()
                .unwrap(),
        )
    }

    fn feed(db: Arc<ContentDb>) -> HudFeed {
        let mut view = WorldView::default();
        view.units = vec![
            unit(&db, 1, "rig", 0, 0),
            unit(&db, 2, "station", 0, 100),
            unit(&db, 3, "planet", 1, 200),
            unit(&db, 4, "latinum", 1, 300),
            unit(&db, 5, "gate", 1, 400),
            unit(&db, 6, "hidden", 1, 500),
            unit(&db, 7, "gate", 1, 700),
            unit(&db, 8, "gate", 1, 750),
        ];
        view.units[6].flags = UnitFlags::SELECTABLE.with(UnitFlags::CLOAKED, true);
        view.visibility = vec![(PlayerId(0), (1..=6).map(EntityId).collect())];
        HudFeed {
            view: Arc::new(view),
            player: PlayerId(0),
            selected: Vec::new(),
            content: Some(db),
            ..Default::default()
        }
    }

    fn map() -> MapDef {
        let mut map = MapDef::empty();
        map.regions = vec![
            Region {
                kind: RegionKind::Nebula,
                odf: "synthetic_nebula".into(),
                shape: RegionShape::Sphere {
                    centre: SimVec3::from_f32([600.0, 0.0, 0.0]),
                    radius: Fixed::from_int(75),
                },
            },
            Region {
                kind: RegionKind::AsteroidField,
                odf: "synthetic_field".into(),
                shape: RegionShape::Box {
                    min: SimVec3::from_f32([700.0, 0.0, -40.0]),
                    max: SimVec3::from_f32([900.0, 0.0, 20.0]),
                },
            },
        ];
        map
    }

    fn overlays(world: &mut World) -> HashMap<FootprintKey, (Entity, Transform)> {
        let mut query = world.query::<(Entity, &FootprintKey, &Transform)>();
        query
            .iter(world)
            .map(|(e, key, t)| (*key, (e, *t)))
            .collect()
    }

    fn region_fog(visible: &[f32]) -> FogState {
        let mut grid = crate::render::fog::FogGrid::new([-1000.0, -1000.0], [1000.0, 1000.0]);
        for &x in visible {
            let cx = ((x - grid.min[0]) / grid.cell) as usize;
            let cz = ((0.0 - grid.min[1]) / grid.cell) as usize;
            grid.set_cell_for_test(cx, cz, true, true);
        }
        FogState { grid, updates: 1 }
    }

    #[test]
    fn pending_sites_show_other_owned_reservations_and_leave_when_replaced_or_started() {
        use sta_sim::production::view::{ConstructionSiteView, ProductionJobView, ProductionView};
        let db = content();
        let mut feed = feed(db.clone());
        let mut view = (*feed.view).clone();
        view.units.push(unit(&db, 9, "rig", 0, 1000));
        view.visibility[0].1.push(EntityId(9));
        feed.view = Arc::new(view);
        let mut rows = vec![ProductionView {
            portal: None,
            builder: EntityId(9),
            jobs: vec![ProductionJobView {
                def: db.key("station").unwrap(),
                progress: Fixed::ZERO,
                paid: true,
                started: false,
            }],
            site: Some(ConstructionSiteView {
                def: db.key("station").unwrap(),
                pos: SimVec3::from_f32([1500.0, 0.0, 200.0]),
                heading: (Fixed::ZERO, Fixed::ONE),
                entity: None,
            }),
        }];
        let pending = pending_footprints(&feed, &rows, Some(EntityId(1)));
        let pose = pending.get(&FootprintKey::Pending(EntityId(9))).unwrap();
        assert_eq!(
            (pose.x, pose.z, pose.half),
            (
                1500.0,
                200.0,
                Vec2::splat(site::footprint(&db, db.key("station").unwrap()).to_f32())
            )
        );
        assert!(pending_footprints(&feed, &rows, Some(EntityId(9))).is_empty());
        rows[0].jobs[0].started = true;
        assert!(
            pending_footprints(&feed, &rows, None).is_empty(),
            "in-yard hull is not a reserved station site"
        );
        rows[0].jobs[0].started = false;
        rows[0].site.as_mut().unwrap().entity = Some(EntityId(20));
        assert!(
            pending_footprints(&feed, &rows, None).is_empty(),
            "real station uses the unit footprint"
        );
        rows[0].site.as_mut().unwrap().entity = None;
        rows[0].builder = EntityId(3);
        assert!(
            pending_footprints(&feed, &rows, None).is_empty(),
            "foreign pending jobs stay private"
        );
        assert!(pending_footprints(&feed, &[], None).is_empty());
    }

    #[test]
    fn unit_classes_use_placement_or_authored_body_radius() {
        let db = content();
        for name in ["rig", "station"] {
            let key = db.key(name).unwrap();
            assert_eq!(
                unit_radius(&db, key, EntityKind::of_def(db.unit(key).unwrap())),
                site::footprint(&db, key).to_f32()
            );
        }
        for (name, radius) in [
            ("planet", 80.0),
            ("gate", 35.0),
            ("latinum", crate::map::FIELD_RADIUS),
        ] {
            let key = db.key(name).unwrap();
            assert_eq!(
                unit_radius(&db, key, EntityKind::of_def(db.unit(key).unwrap())),
                radius
            );
        }
    }

    #[test]
    fn overlays_follow_visible_objects_regions_movement_and_placement_lifecycle() {
        let db = content();
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .insert_resource(feed(db.clone()))
            .insert_resource(HudState::default())
            .insert_resource(region_fog(&[600.0, 800.0]))
            .add_systems(
                Update,
                (ghost::sync_footprints, sync_object_footprints).chain(),
            );
        let map_root = app
            .world_mut()
            .spawn(MapFootprintRegions(Arc::new(map())))
            .id();
        app.update();
        assert!(overlays(app.world_mut()).is_empty());
        app.world_mut().resource_mut::<HudState>().placing = Some(super::super::Armed {
            builder: EntityId(1),
            what: db.key("station").unwrap(),
            odf: "station".into(),
        });
        app.update();
        let first = overlays(app.world_mut());
        assert_eq!(first.len(), 7); // Five visible units and two typed regions.
        assert!(!first.contains_key(&FootprintKey::Sim(EntityId(6))));
        assert!(!first.contains_key(&FootprintKey::Sim(EntityId(7))));
        assert!(!first.contains_key(&FootprintKey::Sim(EntityId(8))));
        let region = first[&FootprintKey::Region(map_root, 1)].1;
        assert_eq!(region.translation, Vec3::new(800.0, 0.15, 10.0));
        let moving = first[&FootprintKey::Sim(EntityId(2))].0;
        assert!(
            app.world()
                .get::<crate::sim_link::SimEntity>(moving)
                .is_none()
        );
        assert!(
            app.world()
                .get::<crate::render::units::SelectRadius>(moving)
                .is_none()
        );
        let mut updated = (*app.world().resource::<HudFeed>().view).clone();
        updated.units[0].flags = UnitFlags::SELECTABLE.with(UnitFlags::CLOAKED, true);
        updated.units[1].pos.x = Fixed::from_int(150);
        updated.units[2].flags = UnitFlags::SELECTABLE.with(UnitFlags::CLOAKED, true);
        updated.units[4].owner = PlayerId(2);
        updated.units[4].flags = UnitFlags::SELECTABLE.with(UnitFlags::CLOAKED, true);
        app.world_mut().resource_mut::<HudFeed>().view = Arc::new(updated.clone());
        app.update();
        let second = overlays(app.world_mut());
        assert_eq!(second.len(), 7);
        assert!(second.contains_key(&FootprintKey::Sim(EntityId(1))));
        assert!(second.contains_key(&FootprintKey::Sim(EntityId(3))));
        assert!(second.contains_key(&FootprintKey::Sim(EntityId(5))));
        assert_eq!(second[&FootprintKey::Sim(EntityId(2))].0, moving);
        assert_eq!(
            second[&FootprintKey::Sim(EntityId(2))].1.translation.x,
            150.0
        );
        app.world_mut().entity_mut(map_root).despawn();
        app.update();
        assert_eq!(overlays(app.world_mut()).len(), 5);
        app.world_mut().resource_mut::<HudState>().placing = None;
        app.update();
        assert!(overlays(app.world_mut()).is_empty());
        app.world_mut().resource_mut::<HudState>().placing = Some(super::super::Armed {
            builder: EntityId(1),
            what: db.key("station").unwrap(),
            odf: "station".into(),
        });
        app.update();
        assert_eq!(overlays(app.world_mut()).len(), 5);
        let mut without_builder = (*app.world().resource::<HudFeed>().view).clone();
        without_builder.units.retain(|unit| unit.id != EntityId(1));
        app.world_mut().resource_mut::<HudFeed>().view = Arc::new(without_builder);
        app.update();
        assert!(overlays(app.world_mut()).is_empty());
        app.world_mut().resource_mut::<HudFeed>().view = Arc::new(updated);
        app.update();
        assert_eq!(overlays(app.world_mut()).len(), 5);
        app.world_mut().resource_mut::<HudFeed>().content = None;
        app.update();
        assert!(overlays(app.world_mut()).is_empty());
    }

    #[test]
    fn visibility_fog_and_missing_assets_fail_closed_and_clear_existing() {
        let db = content();
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .insert_resource(feed(db.clone()))
            .insert_resource(HudState::default())
            .insert_resource(region_fog(&[600.0, 800.0]))
            .add_systems(
                Update,
                (ghost::sync_footprints, sync_object_footprints).chain(),
            );
        let root = app
            .world_mut()
            .spawn(MapFootprintRegions(Arc::new(map())))
            .id();
        app.world_mut().resource_mut::<HudState>().placing = Some(super::super::Armed {
            builder: EntityId(1),
            what: db.key("station").unwrap(),
            odf: "station".into(),
        });
        app.update();
        assert_eq!(overlays(app.world_mut()).len(), 7);
        let mut view = (*app.world().resource::<HudFeed>().view).clone();
        view.visibility = vec![(PlayerId(0), vec![EntityId(1), EntityId(2)])];
        app.world_mut().resource_mut::<HudFeed>().view = Arc::new(view);
        app.world_mut().remove_resource::<FogState>();
        app.update();
        assert_eq!(overlays(app.world_mut()).len(), 2); // Fog absent: no static regions.
        app.insert_resource(region_fog(&[600.0]));
        app.update();
        assert_eq!(overlays(app.world_mut()).len(), 3);
        assert!(overlays(app.world_mut()).contains_key(&FootprintKey::Region(root, 0)));
        app.world_mut().remove_resource::<Assets<Mesh>>();
        app.update();
        assert!(overlays(app.world_mut()).is_empty());
    }

    #[test]
    fn typed_region_bounds_produce_exact_square_and_rectangle_extents() {
        let sphere = region_pose(RegionShape::Sphere {
            centre: SimVec3::from_f32([10.0, 5.0, -20.0]),
            radius: Fixed::from_int(30),
        })
        .unwrap();
        assert_eq!(
            (sphere.x, sphere.z, sphere.half),
            (10.0, -20.0, Vec2::splat(30.0))
        );
        let rectangle = region_pose(RegionShape::Box {
            min: SimVec3::from_f32([0.0, 0.0, -20.0]),
            max: SimVec3::from_f32([100.0, 5.0, 40.0]),
        })
        .unwrap();
        assert_eq!(
            (rectangle.x, rectangle.z, rectangle.half),
            (50.0, 10.0, Vec2::new(50.0, 30.0))
        );
    }

    #[test]
    fn invalid_region_extents_are_not_rendered() {
        assert!(
            region_pose(RegionShape::Sphere {
                centre: SimVec3::ZERO,
                radius: Fixed::ZERO,
            })
            .is_none()
        );
        assert!(
            region_pose(RegionShape::Box {
                min: SimVec3::from_f32([10.0, 0.0, 0.0]),
                max: SimVec3::from_f32([0.0, 0.0, 10.0]),
            })
            .is_none()
        );
    }
}
