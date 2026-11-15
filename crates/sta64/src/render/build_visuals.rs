//! Presentation-only construction at the production site. Preview roots have
//! no simulation identity, selection radius, hull, or collision state.

use std::collections::{BTreeMap, HashSet};

use bevy::prelude::*;
use sta_sim::production::view::{ConstructionSiteView, ProductionJobView, ProductionView};
use sta_sim::view::UnitFlags;
use sta_sim::world::EntityKind;
use sta_sim::{ContentKey, EntityId, PlayerId};

use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::construction::{ConstructionEdges, ConstructionPart, ConstructionPreview};
use crate::render::fx::quad::{self, FxQuadMaterial, Look};
use crate::render::units::{LocalPlayer, NeedsModel, TeamColour, to_bevy};
use crate::sim_link::{ContentRes, SimLink};
use crate::ui::hud::ghost::{Ghost, PinnedBlueprint};
use crate::viewer::TeamColours;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildVisual {
    builder: EntityId,
    def: ContentKey,
    kind: Kind,
    owner: PlayerId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blueprint,
    Ship,
}

fn active_head(row: &ProductionView) -> Option<(&ProductionJobView, &ConstructionSiteView)> {
    let (job, site) = (row.jobs.first()?, row.site.as_ref()?);
    (job.def == site.def).then_some((job, site))
}

fn classify(kind: EntityKind, paid: bool, started: bool, real_site: bool) -> Option<Kind> {
    match kind {
        EntityKind::Craft => (paid && started && !real_site).then_some(Kind::Ship),
        EntityKind::Station => (!started || real_site).then_some(Kind::Blueprint),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct Wanted {
    key: BuildVisual,
    pos: Vec3,
    heading: Vec2,
    progress: f32,
}

/// Register after units' view systems; the caller also registers the module.
pub struct BuildVisualsPlugin;

impl Plugin for BuildVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            sync_build_visuals
                .after(crate::render::units::sync_units)
                .before(crate::render::units::attach_models)
                .in_set(GameSet::Interpolate)
                .run_if(resource_exists::<SimLink>)
                .run_if(resource_exists::<ContentRes>),
        );
        app.add_systems(
            PostUpdate,
            sync_sparks
                .after(crate::render::construction::sync_reveal)
                .run_if(resource_exists::<SimLink>),
        );
    }
}

/// Select only a production row's active head, at its resolved site. No queued
/// item can leave a preview behind when the active job changes or is cancelled.
fn wanted(link: &SimLink, content: &ContentRes, local: &LocalPlayer) -> Vec<Wanted> {
    let curr = link.curr();
    let visible: HashSet<EntityId> = local
        .0
        .map(|p| curr.visible_to(p).into_iter().map(|u| u.id).collect())
        .unwrap_or_else(|| curr.units.iter().map(|u| u.id).collect());
    let mut out = Vec::new();
    for row in link.production() {
        let Some((job, site)) = active_head(row) else {
            continue;
        };
        let Some(def) = content.0.unit(job.def) else {
            continue;
        };
        let Some(kind) = classify(
            EntityKind::of_def(def),
            job.paid,
            job.started,
            site.entity.is_some(),
        ) else {
            continue;
        };
        let shown = match (kind, site.entity) {
            (Kind::Blueprint, Some(id)) => {
                let Some(station) = curr.unit(id) else {
                    continue;
                };
                if station.def != job.def
                    || station.owner != link.player()
                    || !visible.contains(&id)
                    || !station
                        .progress
                        .is_some_and(|progress| progress < sta_sim::Fixed::ONE)
                {
                    continue;
                }
                station
            }
            (Kind::Blueprint, None) => {
                let Some(builder) = curr.unit(row.builder) else {
                    continue;
                };
                if builder.owner != link.player() || !visible.contains(&builder.id) {
                    continue;
                }
                builder
            }
            (Kind::Ship, None) => {
                let Some(builder) = curr.unit(row.builder) else {
                    continue;
                };
                if !visible.contains(&builder.id)
                    || (builder.flags.contains(UnitFlags::CLOAKED)
                        && local.0 != Some(builder.owner))
                {
                    continue;
                }
                builder
            }
            (Kind::Ship, Some(_)) => continue,
        };
        out.push(Wanted {
            key: BuildVisual {
                builder: row.builder,
                def: job.def,
                kind,
                owner: shown.owner,
            },
            pos: to_bevy(site.pos.to_f32()),
            heading: Vec2::new(site.heading.0.to_f32(), site.heading.1.to_f32()),
            progress: job.progress.to_f32().clamp(0.0, 1.0),
        });
    }
    out
}

fn pose(pos: Vec3, heading: Vec2) -> Transform {
    Transform::from_translation(pos).looking_to(
        Vec3::new(heading.x, 0.0, -heading.y).normalize_or(Vec3::NEG_Z),
        Vec3::Y,
    )
}

#[allow(clippy::too_many_arguments)]
fn sync_build_visuals(
    mut commands: Commands,
    link: Res<SimLink>,
    content: Res<ContentRes>,
    local: Res<LocalPlayer>,
    teams: Option<Res<TeamColours>>,
    mut existing: Query<(
        Entity,
        &BuildVisual,
        &mut Transform,
        Option<&mut ConstructionPreview>,
    )>,
    children: Query<&Children>,
    parts: Query<&ConstructionPart>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut last_world: Local<Option<(sta_sim::Tick, u64)>>,
) {
    let current_world = (link.curr().tick, link.curr().digest);
    let replaced = last_world.is_some_and(|old| {
        current_world.0 < old.0 || (current_world.0 == old.0 && current_world.1 != old.1)
    });
    *last_world = Some(current_world);
    let desired = wanted(&link, &content, &local);
    let by_builder: BTreeMap<EntityId, Wanted> =
        desired.iter().map(|w| (w.key.builder, *w)).collect();
    let mut retained = HashSet::new();
    for (entity, visual, mut transform, preview) in &mut existing {
        let next = by_builder.get(&visual.builder);
        let same = next.is_some_and(|w| w.key == *visual);
        let rewound = next.is_some_and(|w| preview.as_ref().is_some_and(|p| w.progress < p.0));
        if replaced || content.is_changed() || !same || rewound {
            // Bevy removes descendants with the root, but the private partial
            // mesh assets need explicit removal from Assets<Mesh> first.
            for child in children.iter_descendants(entity) {
                if let Ok(part) = parts.get(child) {
                    meshes.remove(part.partial.id());
                }
            }
            commands.entity(entity).despawn();
            continue;
        }
        let Some(next) = next else { continue };
        *transform = pose(next.pos, next.heading);
        if let Some(mut preview) = preview {
            preview.0 = next.progress;
        }
        retained.insert(visual.builder);
    }
    for next in desired {
        if retained.contains(&next.key.builder) {
            continue;
        }
        let colour = teams
            .as_ref()
            .and_then(|t| t.owner_colour(next.key.owner))
            .unwrap_or(Color::WHITE);
        let mut root = commands.spawn((
            Name::new(match next.key.kind {
                Kind::Ship => "yard construction hull",
                Kind::Blueprint => "station blueprint",
            }),
            next.key,
            crate::map::MatchEntity,
            NeedsModel(next.key.def),
            TeamColour(colour),
            pose(next.pos, next.heading),
            Visibility::Inherited,
        ));
        match next.key.kind {
            Kind::Ship => {
                root.insert(ConstructionPreview(next.progress));
            }
            Kind::Blueprint => {
                root.insert((PinnedBlueprint, Ghost::pinned(next.key.def)));
            }
        }
    }
}

#[derive(Component)]
struct YardSpark {
    root: Entity,
    slot: usize,
    material: Handle<FxQuadMaterial>,
}

#[derive(Resource)]
struct SparkMesh(Handle<Mesh>);

#[allow(clippy::too_many_arguments)]
fn sync_sparks(
    mut commands: Commands,
    clock: Res<AnimClock>,
    edges: Res<ConstructionEdges>,
    roots: Query<(), With<ConstructionPreview>>,
    mut sparks: Query<(Entity, &YardSpark, &mut Transform, &mut Visibility)>,
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    mut mats: Option<ResMut<Assets<FxQuadMaterial>>>,
    mut meshes: ResMut<Assets<Mesh>>,
    shared: Option<Res<SparkMesh>>,
) {
    let Some(ref mut mats) = mats else { return };
    let mesh = match shared {
        Some(shared) => shared.0.clone(),
        None => {
            let mesh = meshes.add(quad::rectangle());
            commands.insert_resource(SparkMesh(mesh.clone()));
            mesh
        }
    };
    let now = clock.now();
    let eye = camera.single().ok().map(|t| t.translation());
    let mut present = HashSet::new();
    for (entity, spark, mut transform, mut visibility) in &mut sparks {
        let points = edges.previews.get(&spark.root);
        if roots.get(spark.root).is_err() || points.is_none() {
            mats.remove(spark.material.id());
            commands.entity(entity).despawn();
            continue;
        }
        present.insert((spark.root, spark.slot));
        let points = points.unwrap();
        if points.is_empty() {
            *visibility = Visibility::Hidden;
            continue;
        }
        *visibility = Visibility::Inherited;
        let phase = (now * 9.0).floor() as usize;
        let point = points[(phase + spark.slot * 7) % points.len()];
        let jitter = ((now * 17.0 + spark.slot as f64 * 2.1).sin() as f32) * 0.35;
        *transform = quad::billboard(
            eye,
            point + Vec3::Y * jitter,
            Vec2::splat(0.55 + spark.slot as f32 * 0.18),
        );
        let look = Look::new(quad::K_SPARK, Vec3::new(1.0, 0.72, 0.33));
        quad::drive(
            mats,
            &spark.material,
            &look,
            1.4 + jitter.abs(),
            quad::shader_clock(now),
        );
    }
    for (&root, points) in &edges.previews {
        if points.is_empty() || roots.get(root).is_err() {
            continue;
        }
        for slot in 0..3 {
            if present.contains(&(root, slot)) {
                continue;
            }
            let look = Look::new(quad::K_SPARK, Vec3::new(1.0, 0.72, 0.33));
            let material = mats.add(quad::material(&look));
            commands.spawn((
                Name::new("yard weld spark"),
                crate::map::MatchEntity,
                YardSpark {
                    root,
                    slot,
                    material: material.clone(),
                },
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                quad::billboard(eye, points[slot % points.len()], Vec2::splat(0.55)),
                Visibility::Inherited,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_sim::Fixed;

    #[test]
    fn only_active_head_with_matching_site_is_selected() {
        let key = ContentKey(7);
        let other = ContentKey(8);
        let row = ProductionView {
            builder: EntityId(3),
            jobs: vec![
                ProductionJobView {
                    def: key,
                    progress: Fixed::ZERO,
                    paid: true,
                    started: true,
                },
                ProductionJobView {
                    def: other,
                    progress: Fixed::ZERO,
                    paid: true,
                    started: true,
                },
            ],
            site: Some(ConstructionSiteView {
                def: key,
                pos: sta_sim::Vec3::ZERO,
                heading: (Fixed::ZERO, Fixed::ONE),
                entity: None,
            }),
        };
        assert_eq!(active_head(&row).map(|(job, _)| job.def), Some(key));
        let mut wrong_site = row.clone();
        wrong_site.site.as_mut().unwrap().def = other;
        assert!(active_head(&wrong_site).is_none());
        let mut no_jobs = row;
        no_jobs.jobs.clear();
        assert!(active_head(&no_jobs).is_none());
    }

    #[test]
    fn phase_filters_keep_pending_blueprints_and_paid_started_hulls() {
        assert_eq!(
            classify(EntityKind::Craft, true, true, false),
            Some(Kind::Ship)
        );
        assert_eq!(classify(EntityKind::Craft, false, true, false), None);
        assert_eq!(classify(EntityKind::Craft, true, false, false), None);
        assert_eq!(classify(EntityKind::Craft, true, true, true), None);
        assert_eq!(
            classify(EntityKind::Station, false, false, false),
            Some(Kind::Blueprint)
        );
        assert_eq!(
            classify(EntityKind::Station, true, true, true),
            Some(Kind::Blueprint)
        );
        assert_eq!(classify(EntityKind::Station, true, true, false), None);
        assert_eq!(classify(EntityKind::Prop, true, true, false), None);
    }
}

#[cfg(test)]
#[path = "build_visuals_tests.rs"]
mod integration_tests;
