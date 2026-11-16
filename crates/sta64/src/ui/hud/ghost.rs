//! The building ghost: while a build waits for its site ([`HudState::placing`])
//! a translucent copy of the building's model follows the cursor on the
//! ground at the builder's height, green where the site is clear and red where
//! it is not (stock Armada's blueprint, which the refit notes describe as green
//! when clear and red when blocked).
//!
//! Whether the site is clear is [`sta_sim::production::site::check_site`], the
//! rule the sim applies to the order, fed with the same view, so the ghost and
//! the sim agree; [`site_status`] is the one entry the click uses too
//! (both go through [`crate::placement::ghost_site`]).

use bevy::prelude::*;
use sta_sim::production::site::{self, SiteError};
use sta_sim::{ContentKey, Vec3 as SimVec3};

use super::model::HudFeed;
use super::{Armed, HudState, ground_point};
use crate::input::orders::Aim;
use crate::render::hulls::{HullMaterial, HullSource};
use crate::render::units::{NeedsModel, TeamColour, to_bevy};

/// The map's playable volume, for build sites (`None`: unbounded). Set when a
/// map loads, from the same bounds the sim was given.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct PlacementBounds(pub Option<(SimVec3, SimVec3)>);

/// Tint of a ghost over a clear site.
pub const CLEAR: Color = Color::srgba(0.35, 1.0, 0.4, 0.75);
/// Tint of a ghost over a blocked site.
pub const BLOCKED: Color = Color::srgba(1.0, 0.25, 0.2, 0.75);

/// Placed stations awaiting the start of construction appear as faint blueprints.
pub const PINNED: Color = Color::srgba(0.35, 1.0, 0.4, 0.15);

/// A station blueprint owned by the construction renderer, not the cursor.
#[derive(Component, Debug)]
pub(crate) struct PinnedBlueprint;

/// The ghost entity; `status` is whether the site under the cursor is clear.
#[derive(Component, Debug)]
pub struct Ghost {
    pub def: ContentKey,
    pub status: Option<Result<(), SiteError>>,
    painted: Option<Result<(), SiteError>>,
    own: Vec<(Entity, Handle<StandardMaterial>)>,
}

impl Ghost {
    pub(crate) fn pinned(def: ContentKey) -> Self {
        Self {
            def,
            status: Some(Ok(())),
            painted: None,
            own: Vec::new(),
        }
    }
}

#[derive(Component)]
pub struct FootprintGrid;

/// Existing objects/regions have a neutral overlay; placement legality keeps
/// its green/red colours on the proposed station itself.
pub(super) const OCCUPIED: Color = Color::srgba(0.65, 0.8, 1.0, 0.65);

#[derive(Resource)]
pub(crate) struct FootprintAssets {
    meshes: std::collections::BTreeMap<(u32, u32), Handle<Mesh>>,
    cell_size: f32,
    clear: Handle<StandardMaterial>,
    blocked: Handle<StandardMaterial>,
    pub(super) occupied: Handle<StandardMaterial>,
}

impl FootprintAssets {
    pub(super) fn grid(&mut self, meshes: &mut Assets<Mesh>, half: Vec2) -> (Handle<Mesh>, Vec3) {
        let (_, spacing) = footprint_shape(half.max_element(), self.cell_size);
        let count = (half * 2.0 / spacing)
            .ceil()
            .clamp(Vec2::splat(2.0), Vec2::splat(128.0));
        let (columns, rows) = (count.x as u32, count.y as u32);
        let mesh = self
            .meshes
            .entry((columns, rows))
            .or_insert_with(|| {
                meshes.add(if columns == rows {
                    super::footprint::mesh(columns)
                } else {
                    super::footprint::rectangular_mesh(columns, rows)
                })
            })
            .clone();
        (mesh, Vec3::splat(spacing))
    }
}

/// A live, owned builder placing a station supplies the construction plane.
pub(super) fn placement_height(state: &HudState, feed: &HudFeed) -> Option<f32> {
    let armed = state.placing.as_ref()?;
    let db = feed.content.as_deref()?;
    let def = db.unit(armed.what)?;
    if sta_sim::world::EntityKind::of_def(def) != sta_sim::world::EntityKind::Station {
        return None;
    }
    let builder = feed.view.unit(armed.builder)?;
    (builder.owner == feed.player).then(|| builder.pos.y.to_f32())
}

fn footprint_material(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        cull_mode: None,
        // Keep this placement overlay ahead of coplanar map surfaces.
        depth_bias: 2.0,
        ..default()
    }
}

/// Display-only cell spacing from the mounted header, including mod inheritance
/// (D11). Missing or invalid data uses the stock header's 50-unit spacing.
fn footprint_cell_size(cfg: Option<&sta_assets::cfg_h::CfgH>) -> f32 {
    cfg.and_then(|c| c.f32("STATION_PLACEMENT_GRID_SIZE"))
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(50.0)
}

fn footprint_shape(radius: f32, cell_size: f32) -> (u32, f32) {
    let cells = (2.0 * radius / cell_size).ceil().max(2.0);
    // Bound geometry for malformed/extreme content without shrinking its area.
    let count = cells.min(128.0) as u32;
    (count, cell_size.max(2.0 * radius / count as f32))
}

/// Give visible ghosts a square placement grid, sharing meshes by cell count.
/// Its extent encloses the current simulator footprint; this overlay does not
/// introduce tile-based simulation occupancy or snap the accepted build site.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_footprints(
    mut commands: Commands,
    state: Res<HudState>,
    feed: Res<HudFeed>,
    ghosts: Query<(Entity, &Ghost, &Visibility)>,
    children: Query<&Children>,
    grids: Query<
        (
            Entity,
            &MeshMaterial3d<StandardMaterial>,
            &Transform,
            &Mesh3d,
        ),
        With<FootprintGrid>,
    >,
    mut assets: Option<ResMut<FootprintAssets>>,
    mounted: Option<Res<crate::viewer::Mounted>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    if placement_height(&state, &feed).is_none() {
        for (entity, ..) in &grids {
            commands.entity(entity).despawn();
        }
        return;
    }
    let Some(db) = feed.content.as_deref() else {
        return;
    };
    let Some(mut meshes) = meshes else { return };
    let read_spacing = || {
        footprint_cell_size(
            mounted
                .as_ref()
                .and_then(|m| sta_content::layered::cfg_h(&m.0, "RTS_CFG.h"))
                .as_ref(),
        )
    };
    if assets.is_none() {
        let Some(mut materials) = materials else {
            return;
        };
        let assets = FootprintAssets {
            meshes: Default::default(),
            cell_size: read_spacing(),
            clear: materials.add(footprint_material(CLEAR)),
            blocked: materials.add(footprint_material(BLOCKED)),
            occupied: materials.add(footprint_material(OCCUPIED)),
        };
        commands.insert_resource(assets);
        return;
    }
    let assets = assets.as_mut().unwrap();
    if mounted.as_ref().is_some_and(|m| m.is_changed()) {
        assets.cell_size = read_spacing();
    }
    for (root, ghost, visibility) in &ghosts {
        if *visibility == Visibility::Hidden || ghost.status.is_none() {
            continue;
        }
        let radius = site::footprint(db, ghost.def).to_f32();
        let (mesh, scale) = assets.grid(&mut meshes, Vec2::splat(radius));
        let material = if ghost.status.is_some_and(|s| s.is_ok()) {
            &assets.clear
        } else {
            &assets.blocked
        };
        let existing = children
            .iter_descendants(root)
            .find_map(|e| grids.get(e).ok());
        if let Some((entity, current, transform, current_mesh)) = existing {
            if current.0 != *material {
                commands
                    .entity(entity)
                    .insert(MeshMaterial3d(material.clone()));
            }
            if current_mesh.0 != mesh {
                commands.entity(entity).insert(Mesh3d(mesh));
            }
            if transform.scale != scale {
                commands.entity(entity).insert(Transform {
                    scale,
                    ..*transform
                });
            }
        } else {
            commands.spawn((
                Name::new("building footprint grid"),
                FootprintGrid,
                ChildOf(root),
                Mesh3d(mesh),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(0.0, 0.15, 0.0).with_scale(scale),
            ));
        }
    }
}

/// Where `armed` would be built for a cursor at `at`, and whether the site is legal: the same
/// [`crate::placement::ghost_site`] rule the ghost draws, so the click and the ghost agree (an
/// orbital station snaps onto a planet's orbit); the order itself carries the raw cursor point, which the
/// sim snaps the same way. No content loaded: the cursor point, `Ok`.
pub fn site_status(
    feed: &HudFeed,
    bounds: Option<&(SimVec3, SimVec3)>,
    armed: &Armed,
    at: SimVec3,
) -> (SimVec3, Result<(), SiteError>) {
    let Some(db) = feed.content.as_deref() else {
        return (at, Ok(()));
    };
    let site = crate::placement::ghost_site(db, bounds, &feed.view, armed.what, armed.builder, at);
    (site.at, site.status)
}

/// The click's result for a cursor at `at`: the site the ghost shows (for the destination line) and the
/// Build order to queue, or why the site is refused. The order carries the raw cursor point: the sim snaps
/// it itself, once (a snapped point snapped again can land on another planet).
pub fn build_order(
    feed: &HudFeed,
    bounds: Option<&(SimVec3, SimVec3)>,
    armed: &Armed,
    at: SimVec3,
) -> Result<(SimVec3, sta_sim::command::Order), SiteError> {
    let (site_at, status) = site_status(feed, bounds, armed, at);
    status?;
    Ok((
        site_at,
        sta_sim::command::Order::Build {
            builder: armed.builder,
            what: armed.what,
            at: Some(at),
        },
    ))
}

/// Whether the player can pay for `armed` now (the sim refuses a rig's order otherwise).
pub fn affordable(feed: &HudFeed, armed: &Armed) -> bool {
    let (Some(db), Some(p)) = (feed.content.as_deref(), feed.view.player(feed.player)) else {
        return true;
    };
    let Some(def) = db.unit(armed.what) else {
        return true;
    };
    def.build
        .cost
        .iter()
        .enumerate()
        .all(|(i, &c)| c <= sta_sim::Fixed::ZERO || p.resources.get(i).is_some_and(|&h| h >= c))
}

fn paint(color: Color) -> impl Fn(&mut StandardMaterial) {
    move |m| {
        // A dark diffuse map or its cut-out alpha must not attenuate the
        // blueprint. These changes are made only to the ghost's material copy.
        m.base_color_texture = None;
        m.normal_map_texture = None;
        m.metallic_roughness_texture = None;
        m.occlusion_texture = None;
        m.emissive_texture = None;
        m.base_color = color;
        m.alpha_mode = AlphaMode::Blend;
        m.unlit = true;
        m.emissive = LinearRgba::BLACK;
        m.metallic = 0.0;
        m.reflectance = 0.0;
        m.perceptual_roughness = 1.0;
    }
}

/// Keep one ghost while a placement is armed: spawn it, move it to the
/// cursor, set its status, remove it when the placement ends.
#[allow(clippy::too_many_arguments)]
pub fn update_ghost(
    mut commands: Commands,
    state: Res<HudState>,
    feed: Res<HudFeed>,
    aim: Option<Res<Aim>>,
    bounds: Option<Res<PlacementBounds>>,
    mut ghosts: Query<
        (Entity, &mut Ghost, &mut Transform, &mut Visibility),
        Without<PinnedBlueprint>,
    >,
) {
    let armed = state
        .placing
        .as_ref()
        .filter(|a| feed.view.unit(a.builder).is_some());
    let Some(armed) = armed else {
        for (e, ..) in &ghosts {
            commands.entity(e).despawn();
        }
        return;
    };
    let mut have = false;
    for (e, mut g, mut t, mut vis) in &mut ghosts {
        if have || g.def != armed.what {
            commands.entity(e).despawn();
            continue;
        }
        have = true;
        let y = feed
            .view
            .unit(armed.builder)
            .map_or(0.0, |b| b.pos.y.to_f32());
        let at = aim
            .as_ref()
            .and_then(|a| a.0)
            .and_then(|a| ground_point(a.ray, y));
        match at {
            Some(at) => {
                // The placement rule the sim applies: an orbital station snaps onto a planet's orbit.
                let (site_at, status) = site_status(
                    &feed,
                    bounds.as_deref().and_then(|b| b.0.as_ref()),
                    armed,
                    at,
                );
                t.translation = to_bevy(site_at.to_f32());
                g.status = Some(status);
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
    if !have {
        commands.spawn((
            Name::new("building ghost"),
            Ghost {
                def: armed.what,
                status: None,
                painted: None,
                own: Vec::new(),
            },
            NeedsModel(armed.what),
            TeamColour(Color::WHITE),
            // Faces the way the finished building will (heading +z).
            Transform::IDENTITY.looking_to(Vec3::NEG_Z, Vec3::Y),
            Visibility::Hidden,
        ));
    }
}

/// Give the ghost's model translucent materials of its own (the model's parts
/// share theirs with real units) and keep them tinted by the site's status.
pub fn tint_ghost(
    mut commands: Commands,
    mut ghosts: Query<(Entity, &mut Ghost, Option<&PinnedBlueprint>)>,
    children: Query<&Children>,
    mut handles: Query<&mut MeshMaterial3d<StandardMaterial>>,
    hull_sources: Query<&HullSource>,
    footprints: Query<(), With<FootprintGrid>>,
    assets: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let Some(mut assets) = assets else { return };
    for (root, mut g, pinned) in &mut ghosts {
        let Some(status) = g.status else { continue };
        let mut new_parts = false;
        for d in children.iter_descendants(root) {
            if footprints.get(d).is_ok() {
                continue;
            }
            if let Some((_, own)) = g.own.iter().find(|(entity, _)| *entity == d) {
                if hull_sources.get(d).is_ok() {
                    commands
                        .entity(d)
                        .remove::<HullSource>()
                        .remove::<MeshMaterial3d<HullMaterial>>()
                        .insert(MeshMaterial3d(own.clone()));
                }
                continue;
            }
            let source = hull_sources.get(d).ok().map(|h| h.source.clone());
            let original = source
                .clone()
                .or_else(|| handles.get(d).ok().map(|h| h.0.clone()));
            let Some(copy) = original.and_then(|h| assets.get(&h).cloned()) else {
                continue;
            };
            let own = assets.add(copy);
            if source.is_some() {
                commands
                    .entity(d)
                    .remove::<HullSource>()
                    .remove::<MeshMaterial3d<HullMaterial>>()
                    .insert(MeshMaterial3d(own.clone()));
            } else if let Ok(mut h) = handles.get_mut(d) {
                h.0 = own.clone();
            }
            g.own.push((d, own));
            new_parts = true;
        }
        if g.painted == Some(status) && !new_parts && !g.own.is_empty() {
            continue;
        }
        let color = if pinned.is_some() {
            PINNED
        } else if status.is_ok() {
            CLEAR
        } else {
            BLOCKED
        };
        let apply = paint(color);
        for (_, h) in &g.own {
            if let Some(mut m) = assets.get_mut(h) {
                apply(&mut m);
            }
        }
        if !g.own.is_empty() {
            g.painted = Some(status);
        }
    }
}

/// Why a click on a blocked site is refused, as the notice says it.
pub fn refusal(e: SiteError) -> &'static str {
    match e {
        SiteError::OffMap => "Cannot build here: off the map",
        SiteError::Blocked => "Cannot build here: the site is blocked",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn placement_view(db: &sta_content::ContentDb) -> std::sync::Arc<sta_sim::WorldView> {
        use sta_sim::WorldSource;
        sta_sim::fake::ScriptedSource::from_json5(
            r#"{units:[{id:1,def:"fx_cruiser",owner:0,hull:100,rails:[{at:0,pos:{x:0,y:0,z:0}}]}]}"#,
            db,
        ).unwrap().view()
    }

    #[test]
    fn grid_spacing_uses_header_and_extreme_content_keeps_its_extent() {
        use sta_assets::cfg_h::CfgH;
        let cfg = CfgH::parse("float STATION_PLACEMENT_GRID_SIZE = 25.0;\n");
        assert_eq!(footprint_cell_size(Some(&cfg)), 25.0);
        for value in ["0", "-5", "nan", "inf", "nonsense"] {
            let cfg = CfgH::parse(&format!("float STATION_PLACEMENT_GRID_SIZE = {value};\n"));
            assert_eq!(footprint_cell_size(Some(&cfg)), 50.0);
        }
        assert_eq!(footprint_cell_size(None), 50.0);
        assert_eq!(footprint_shape(100.0, 25.0), (8, 25.0));
        let (cells, spacing) = footprint_shape(100_000.0, 25.0);
        assert_eq!(cells, 128);
        assert!(cells as f32 * spacing >= 200_000.0);
    }

    #[test]
    fn footprint_tracks_status_and_is_removed_with_its_ghost() {
        use sta_sim::PlayerId;
        use std::sync::Arc;

        let db = sta_content::fake::db();
        let def = db.key("fx_station").unwrap();
        let radius = site::footprint(&db, def).to_f32();
        let mut world = World::new();
        world.insert_resource(HudFeed {
            view: placement_view(&db),
            player: PlayerId(0),
            selected: Vec::new(),
            content: Some(Arc::new(db)), ..Default::default()
        });
        world.insert_resource(HudState {
            placing: Some(Armed {
                builder: sta_sim::EntityId(1),
                what: def,
                odf: "fx_station".into(),
            }),
            ..default()
        });
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        let root = world
            .spawn((
                Ghost {
                    def,
                    status: Some(Ok(())),
                    painted: None,
                    own: Vec::new(),
                },
                Transform::from_xyz(42.0, 0.0, 7.0),
                Visibility::Inherited,
            ))
            .id();

        world.run_system_once(sync_footprints).unwrap(); // initialize shared assets
        world.run_system_once(sync_footprints).unwrap();
        let ring = world.get::<Children>(root).unwrap().iter().next().unwrap();
        assert!(world.get::<FootprintGrid>(ring).is_some());
        let local = world.get::<Transform>(ring).unwrap();
        let (cells, spacing) = footprint_shape(radius, 50.0);
        assert_eq!(local.scale, Vec3::splat(spacing));
        assert!(cells as f32 * spacing >= 2.0 * radius);
        assert_eq!(local.translation.x, 0.0);
        assert_eq!(local.translation.z, 0.0);
        let clear = world
            .get::<MeshMaterial3d<StandardMaterial>>(ring)
            .unwrap()
            .0
            .clone();
        let mesh = world.get::<Mesh3d>(ring).unwrap().0.clone();
        assert_eq!(
            world
                .resource::<Assets<StandardMaterial>>()
                .get(&clear)
                .unwrap()
                .base_color,
            CLEAR
        );

        world.get_mut::<Ghost>(root).unwrap().status = Some(Err(SiteError::Blocked));
        world.run_system_once(sync_footprints).unwrap();
        let blocked = world
            .get::<MeshMaterial3d<StandardMaterial>>(ring)
            .unwrap()
            .0
            .clone();
        assert_ne!(blocked, clear);
        assert_eq!(
            world
                .resource::<Assets<StandardMaterial>>()
                .get(&blocked)
                .unwrap()
                .base_color,
            BLOCKED
        );
        assert_eq!(world.get::<Children>(root).unwrap().len(), 1);
        assert_eq!(world.get::<Mesh3d>(ring).unwrap().0, mesh);

        world.entity_mut(root).despawn();
        assert!(world.get_entity(ring).is_err());
    }

    #[test]
    fn pinned_blueprint_survives_cursor_disarm_and_is_faint() {
        use sta_sim::PlayerId;
        use std::sync::Arc;

        let db = sta_content::fake::db();
        let def = db.key("fx_station").unwrap();
        let mut world = World::new();
        world.insert_resource(HudFeed {
            view: placement_view(&db),
            player: PlayerId(0),
            selected: Vec::new(),
            content: Some(Arc::new(db)), ..Default::default()
        });
        world.insert_resource(HudState::default());
        let root = world
            .spawn((
                PinnedBlueprint,
                Ghost::pinned(def),
                Transform::default(),
                Visibility::Inherited,
            ))
            .id();
        world.run_system_once(update_ghost).unwrap();
        assert!(world.get::<Ghost>(root).is_some());
        world.resource_mut::<HudState>().placing = Some(Armed {
            builder: sta_sim::EntityId(1),
            what: def,
            odf: "fx_station".into(),
        });

        let mut materials = Assets::<StandardMaterial>::default();
        let source = materials.add(StandardMaterial::default());
        world.insert_resource(materials);
        world.insert_resource(Assets::<Mesh>::default());
        let child = world.spawn(MeshMaterial3d(source)).id();
        world.entity_mut(root).add_child(child);
        world.run_system_once(sync_footprints).unwrap();
        world.run_system_once(sync_footprints).unwrap();
        world.run_system_once(tint_ghost).unwrap();
        let ring = world
            .get::<Children>(root)
            .unwrap()
            .iter()
            .find(|&e| world.get::<FootprintGrid>(e).is_some())
            .unwrap();
        let ring_material = &world
            .get::<MeshMaterial3d<StandardMaterial>>(ring)
            .unwrap()
            .0;
        assert_eq!(
            world
                .resource::<Assets<StandardMaterial>>()
                .get(ring_material)
                .unwrap()
                .base_color,
            CLEAR
        );
        let own = &world
            .get::<MeshMaterial3d<StandardMaterial>>(child)
            .unwrap()
            .0;
        assert_eq!(
            world
                .resource::<Assets<StandardMaterial>>()
                .get(own)
                .unwrap()
                .base_color,
            PINNED
        );
        world.resource_mut::<HudState>().placing = None;
        world.run_system_once(sync_footprints).unwrap();
        assert!(world.get::<Ghost>(root).is_some());
        assert!(
            world.get_entity(ring).is_err(),
            "footprints end with placement mode"
        );
    }

    #[test]
    fn blueprint_copy_ignores_dark_transparent_source_and_preserves_original() {
        let source = StandardMaterial {
            base_color: Color::srgba(0.0, 0.0, 0.0, 0.0),
            base_color_texture: Some(Handle::default()),
            normal_map_texture: Some(Handle::default()),
            metallic_roughness_texture: Some(Handle::default()),
            occlusion_texture: Some(Handle::default()),
            emissive_texture: Some(Handle::default()),
            ..default()
        };
        let mut world = World::new();
        let mut materials = Assets::<StandardMaterial>::default();
        let original = materials.add(source);
        world.insert_resource(materials);
        let child = world.spawn(MeshMaterial3d(original.clone())).id();
        let converted = world
            .spawn((
                HullSource::test_source(original.clone()),
                MeshMaterial3d::<HullMaterial>(Handle::default()),
            ))
            .id();
        let root = world
            .spawn(Ghost {
                def: ContentKey::NONE,
                status: Some(Ok(())),
                painted: None,
                own: Vec::new(),
            })
            .add_children(&[child, converted])
            .id();
        world.run_system_once(tint_ghost).unwrap();
        assert!(world.get::<HullSource>(converted).is_none());
        assert!(
            world
                .get::<MeshMaterial3d<HullMaterial>>(converted)
                .is_none()
        );
        let converted_material = world
            .get::<MeshMaterial3d<StandardMaterial>>(converted)
            .unwrap()
            .0
            .clone();
        assert_ne!(converted_material, original);
        assert_eq!(
            world
                .resource::<Assets<StandardMaterial>>()
                .get(&converted_material)
                .unwrap()
                .base_color,
            CLEAR
        );
        let own = world
            .get::<MeshMaterial3d<StandardMaterial>>(child)
            .unwrap()
            .0
            .clone();
        assert_ne!(own, original);
        assert_eq!(
            world
                .resource::<Assets<StandardMaterial>>()
                .get(&own)
                .unwrap()
                .base_color,
            CLEAR
        );
        world.get_mut::<Ghost>(root).unwrap().status = Some(Err(SiteError::Blocked));
        world.run_system_once(tint_ghost).unwrap();
        assert_eq!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(child)
                .unwrap()
                .0,
            own
        );

        let materials = world.resource::<Assets<StandardMaterial>>();
        let source = materials.get(&original).unwrap();
        let blocked = materials.get(&own).unwrap();
        assert_eq!(source.base_color, Color::srgba(0.0, 0.0, 0.0, 0.0));
        assert!(source.base_color_texture.is_some());
        assert_eq!(blocked.base_color, BLOCKED);
        assert!(blocked.unlit);
        assert!(matches!(blocked.alpha_mode, AlphaMode::Blend));
        assert!(blocked.base_color_texture.is_none());
        assert!(blocked.normal_map_texture.is_none());
        assert!(blocked.metallic_roughness_texture.is_none());
        assert!(blocked.occlusion_texture.is_none());
        assert!(blocked.emissive_texture.is_none());
        assert_eq!(blocked.emissive, LinearRgba::BLACK);
        assert_ne!(source.base_color, blocked.base_color);
    }

    /// An orbital station's ghost follows the sim's placement rule (it snaps onto the planet's
    /// orbit and is refused in open space), not the ordinary site rule.
    #[test]
    fn an_orbital_ghost_snaps_to_the_planet_and_is_refused_in_open_space() {
        use crate::input::select::AimState;
        use sta_content::{ContentDb, MoveStats, UnitDef};
        use sta_sim::view::UnitFlags;
        use sta_sim::view::UnitView;
        use sta_sim::{EntityId, Fixed, PlayerId, WorldView};
        use std::sync::Arc;

        let fake = sta_content::fake::db();
        let mut rig = UnitDef::new("fx_rig", "constructionrig");
        rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
        let mut orbital = UnitDef::new("fx_orbital", "p_mining");
        orbital.orbit_time = Some(Fixed::from_int(120));
        let mut planet = UnitDef::new("fx_world", "planet");
        planet.radius = Fixed::from_int(200);
        planet.resource = Some(2);
        planet.can_have_orbital_station = true;
        let mut b = ContentDb::builder()
            .profile(fake.profile())
            .features(fake.features().clone())
            .unit(rig)
            .unit(orbital)
            .unit(planet);
        for w in fake.weapons() {
            b = b.weapon(w.clone());
        }
        for u in fake.units() {
            b = b.unit(u.clone());
        }
        let db = b.build().unwrap();
        let unit = |id, odf: &str, x: i32| UnitView {
            id: EntityId(id),
            def: db.key(odf).unwrap(),
            owner: PlayerId(0),
            pos: SimVec3 {
                x: Fixed::from_int(x),
                y: Fixed::ZERO,
                z: Fixed::ZERO,
            },
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2],
            shields: [[Fixed::ZERO; 2]; 4],
            crew: None,
            energy: None,
            autonomy: None,
            flags: UnitFlags::SELECTABLE,
            progress: None,
        };
        let what = db.key("fx_orbital").unwrap();
        let view = WorldView {
            units: vec![unit(1, "fx_world", 1000), unit(2, "fx_rig", 0)],
            ..Default::default()
        };
        let aim_at = |x: f32, z: f32| {
            Aim(Some(AimState {
                cursor: Vec2::ZERO,
                ray: Ray3d::new(Vec3::new(x, 100.0, -z), Dir3::NEG_Y),
                clip_from_world: Mat4::IDENTITY,
                size: Vec2::new(800.0, 600.0),
            }))
        };

        let mut world = World::new();
        world.insert_resource(HudFeed {
            view: Arc::new(view),
            player: PlayerId(0),
            selected: Vec::new(),
            content: Some(Arc::new(db)),
            ..Default::default()
        });
        let state = HudState {
            placing: Some(Armed {
                builder: EntityId(2),
                what,
                odf: "fx_orbital".into(),
            }),
            ..Default::default()
        };
        world.insert_resource(state);
        let ghost = world
            .spawn((
                Ghost {
                    def: what,
                    status: None,
                    painted: None,
                    own: Vec::new(),
                },
                Transform::default(),
                Visibility::Hidden,
            ))
            .id();

        world.insert_resource(aim_at(1100.0, 0.0));
        world.run_system_once(update_ghost).unwrap();
        assert_eq!(world.get::<Ghost>(ghost).unwrap().status, Some(Ok(())));
        // Radius 200 + 75 from the planet's centre, on the cursor's bearing.
        assert_eq!(world.get::<Transform>(ghost).unwrap().translation.x, 1275.0);

        world.insert_resource(aim_at(3000.0, 3000.0));
        world.run_system_once(update_ghost).unwrap();
        assert_eq!(
            world.get::<Ghost>(ghost).unwrap().status,
            Some(Err(SiteError::Blocked))
        );
        assert_eq!(world.get::<Transform>(ghost).unwrap().translation.x, 3000.0);

        world.insert_resource(Aim(None));
        world.run_system_once(update_ghost).unwrap();
        assert_eq!(world.get::<Visibility>(ghost), Some(&Visibility::Hidden));

        // The click asks the same rule: the site the ghost shows OK is accepted, open space refused.
        let feed = world.resource::<HudFeed>();
        let armed = world.resource::<HudState>().placing.clone().unwrap();
        let sv = |x: i32, z: i32| SimVec3 {
            x: Fixed::from_int(x),
            y: Fixed::ZERO,
            z: Fixed::from_int(z),
        };
        let (snapped, ok) = site_status(feed, None, &armed, sv(1100, 0));
        assert_eq!((ok, snapped), (Ok(()), sv(1275, 0)));
        assert_eq!(
            site_status(feed, None, &armed, sv(3000, 3000)).1,
            Err(SiteError::Blocked)
        );
    }
}

#[cfg(test)]
#[path = "ghost_gpu.rs"]
mod gpu_tests;
