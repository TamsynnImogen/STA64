//! Display-only occupied footprints while a station build site is being chosen.
//! Craft and station squares use the current construction placement radius;
//! resource and prop squares use their authored body radius when present.
//! These descriptive squares do not change the sim's radial site rules.

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
    Region(Entity, usize),
}

#[derive(Clone, Copy)]
struct FootprintPose {
    x: f32,
    z: f32,
    half: Vec2,
}

fn region_pose(shape: RegionShape) -> Option<FootprintPose> {
    let (x, z, half) = match shape {
        RegionShape::Sphere { centre, radius } => {
            let radius = radius.to_f32();
            (centre.x.to_f32(), centre.z.to_f32(), Vec2::splat(radius))
        }
        RegionShape::Box { min, max } => {
            let (x0, z0, x1, z1) = (
                min.x.to_f32(),
                min.z.to_f32(),
                max.x.to_f32(),
                max.z.to_f32(),
            );
            (
                (x0 + x1) * 0.5,
                (z0 + z1) * 0.5,
                Vec2::new((x1 - x0) * 0.5, (z1 - z0) * 0.5),
            )
        }
        _ => return None,
    };
    (x.is_finite()
        && z.is_finite()
        && half.x.is_finite()
        && half.y.is_finite()
        && half.x > 0.0
        && half.y > 0.0)
        .then_some(FootprintPose { x, z, half })
}

fn unit_radius(db: &ContentDb, def: ContentKey, kind: EntityKind) -> f32 {
    let authored = db.unit(def).map_or(0.0, |unit| unit.radius.to_f32());
    match kind {
        EntityKind::Craft | EntityKind::Station => site::footprint(db, def).to_f32(),
        EntityKind::Resource if authored <= 0.0 => crate::map::resource_pick_radius(authored),
        EntityKind::Resource | EntityKind::Prop if authored > 0.0 => authored,
        _ => site::footprint(db, def).to_f32(),
    }
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
