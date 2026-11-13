//! Finite hull-surface projections, independent of a receiving model's UVs.

pub mod projection;
pub mod text;

use super::{
    appearance::{
        ModelInstance, ModelNode, ModelPart,
        policy::{self, DamageTarget, ProjectionPlacement},
    },
    textures::{LoaderParams, upload},
};
use crate::{
    app::AppState,
    sim_link::{ContentRes, SimEntity, SimLink},
    viewer::Mounted,
};
use bevy::prelude::*;
use bevy::{asset::RenderAssetUsages, mesh::Indices, render::render_resource::PrimitiveTopology};
use projection::{Projector, SurfaceTriangle, project_surface};
use std::collections::{HashMap, HashSet};

pub struct DecalsPlugin;
impl Plugin for DecalsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Cache>();
        app.add_systems(
            PostUpdate,
            update
                .after(bevy::transform::TransformSystems::Propagate)
                .after(super::construction::sync_reveal)
                .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate)
                .run_if(crate::game::flow::match_running)
                .run_if(resource_exists::<SimLink>)
                .run_if(resource_exists::<Mounted>)
                .run_if(resource_exists::<ContentRes>),
        );
        app.add_systems(OnExit(AppState::InGame), clear);
    }
}

/// Actual clipped hull triangles, never a free-standing image plane.
#[derive(Component)]
pub struct ProjectionPart {
    pub root: Entity,
    pub receiver: Entity,
    pub placement: usize,
}

struct Source {
    placement: ProjectionPlacement,
    flip_u: bool,
    damage: Option<(DamageTarget, u8)>,
    material: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
struct Cache {
    sources: HashMap<Entity, Vec<Source>>,
    geometry: HashMap<(Entity, Entity, usize), Geometry>,
}
struct Geometry {
    matrix: [u32; 16],
    entity: Option<Entity>,
    mesh: Option<Handle<Mesh>>,
}

fn clear(
    mut commands: Commands,
    parts: Query<Entity, With<ProjectionPart>>,
    mut cache: ResMut<Cache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for entity in &parts {
        commands.entity(entity).try_despawn();
    }
    release_cache(&mut cache, &mut meshes, &mut materials);
}

fn release_cache(
    cache: &mut Cache,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    for g in cache.geometry.values() {
        if let Some(handle) = &g.mesh {
            meshes.remove(handle.id());
        }
    }
    for sources in cache.sources.values() {
        for source in sources {
            materials.remove(source.material.id());
        }
    }
    *cache = Cache::default();
}

fn tint(raw: Option<&str>) -> Option<[u8; 4]> {
    let Some(raw) = raw else {
        return Some([255; 4]);
    };
    let nums: Vec<_> = raw
        .trim_matches('"')
        .split_whitespace()
        .map(str::parse::<u8>)
        .collect::<Result<_, _>>()
        .ok()?;
    match nums.as_slice() {
        [r, g, b] => Some([*r, *g, *b, 255]),
        [r, g, b, a] => Some([*r, *g, *b, *a]),
        _ => None,
    }
}

fn sources(
    instance: &ModelInstance,
    id: sta_core::EntityId,
    def: &sta_content::UnitDef,
    loader: &mut super::textures::Loader,
    materials: &mut Assets<StandardMaterial>,
) -> Vec<Source> {
    let mut result = Vec::new();
    let identity = crate::ui::hud::model::craft_names::identity(def, id);
    for logo in &instance.policy.logos {
        let stem = format!("logoDecal{}", logo.index);
        let texture = if let Some(mode) = def.raw.str(&format!("{stem}Text")) {
            if !mode.eq_ignore_ascii_case("registry") {
                warn!("{}: {stem}Text must be registry", def.odf);
                continue;
            }
            let Some(text) = &identity.registry else {
                continue;
            };
            let font = def.raw.str(&format!("{stem}Font")).unwrap_or("FontSmall");
            let Some(colour) = tint(def.raw.raw(&format!("{stem}Color"))) else {
                warn!("{}: invalid {stem}Color", def.odf);
                continue;
            };
            let key = format!("registry-text:{font}:{text}:{colour:?}");
            if let Some(handle) = loader.cache.map.get(&key) {
                handle.clone()
            } else {
                let handle = match text::from_vfs(loader.vfs, font, text, colour) {
                    Ok(pixels) => Some(
                        loader
                            .images
                            .add(upload(&sta_assets::pixels::generate_mips(&pixels), true)),
                    ),
                    Err(error) => {
                        warn!("{}: {stem}: {error}", def.odf);
                        None
                    }
                };
                loader.cache.map.insert(key, handle.clone());
                handle
            }
        } else {
            let Some(name) = &identity.logo else {
                continue;
            };
            let name = match &logo.suffix {
                Some(suffix) => super::appearance::factions::suffixed_name(name, suffix),
                None => Some(name.clone()),
            };
            name.and_then(|name| loader.decal_colour(&name, logo.colour_key))
        };
        if let Some(texture) = texture {
            result.push(Source {
                placement: logo.projection.clone(),
                flip_u: logo.flip_u,
                damage: None,
                material: materials.add(decal_material(texture)),
            });
        }
    }
    let mut damage = instance.policy.damage.clone();
    let global: Vec<_> = (1..=64)
        .filter_map(|i| def.raw.str(&format!("scorchTexture{i}")))
        .filter(|name| loader.decal_colour(name, None).is_some())
        .map(str::to_owned)
        .collect();
    damage.extend(policy::legacy_damage(&def.raw, &global));
    for decal in damage {
        if let Some(texture) = loader.decal_colour(&decal.projection.texture, None) {
            result.push(Source {
                placement: decal.projection,
                flip_u: false,
                damage: Some((decal.target, decal.index)),
                material: materials.add(decal_material(texture)),
            });
        }
    }
    result
}

fn decal_material(texture: Handle<Image>) -> StandardMaterial {
    StandardMaterial {
        base_color_texture: Some(texture),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 1.0,
        reflectance: 0.0,
        ..default()
    }
}

/// Authored XYZ degrees apply Z, then Y, then X, followed by the offset.
/// The legacy Z reflection is already applied to source coordinates. The
/// exposed image faces local -Z; the projector casts +Z into the surface.
pub fn placement_matrix(p: &ProjectionPlacement, scale: f32) -> Mat4 {
    let [x, y, z] = p.rotation_degrees.unwrap_or([0.0; 3]).map(f32::to_radians);
    let mirror = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    let offset = Vec3::from(p.offset.unwrap_or([0.0; 3])) * scale;
    mirror
        * Mat4::from_translation(offset)
        * Mat4::from_rotation_x(x)
        * Mat4::from_rotation_y(y)
        * Mat4::from_rotation_z(z)
        * mirror
}

fn node_matrix(
    index: usize,
    instance: &ModelInstance,
    local: &HashMap<usize, (Entity, Mat4, bool)>,
) -> Option<Mat4> {
    let mut current = index;
    let mut matrix = Mat4::IDENTITY;
    for _ in 0..=instance.model.nodes.len() {
        matrix = local.get(&current)?.1 * matrix;
        match instance.model.nodes.get(current)?.parent {
            Some(parent) => current = parent,
            None => return Some(matrix),
        }
    }
    None
}

fn visible(
    index: usize,
    instance: &ModelInstance,
    local: &HashMap<usize, (Entity, Mat4, bool)>,
) -> bool {
    let mut current = index;
    for _ in 0..=instance.model.nodes.len() {
        if !local.get(&current).is_some_and(|r| r.2) {
            return false;
        }
        match instance.model.nodes.get(current).and_then(|n| n.parent) {
            Some(parent) => current = parent,
            None => return true,
        }
    }
    false
}

fn damage_visible(
    source: &Source,
    unit: &sta_sim::UnitView,
    view: &sta_sim::WorldView,
    policy: &policy::AppearancePolicy,
) -> bool {
    let Some((target, index)) = source.damage else {
        return true;
    };
    if policy.damage_preview {
        return true;
    }
    let Some(threshold) = policy.damage_threshold else {
        return false;
    };
    let health = match target {
        DamageTarget::Hull => Some(unit.hull),
        target => {
            let i = match target {
                DamageTarget::Engines => 0,
                DamageTarget::Weapons => 1,
                DamageTarget::ShieldGenerator => 2,
                DamageTarget::LifeSupport => 3,
                DamageTarget::Sensors => 4,
                DamageTarget::Hull => unreachable!(),
            };
            view.appearance
                .systems(unit.id)
                .and_then(|systems| systems[i])
                .map(|s| s.health)
        }
    };
    health.is_some_and(|[now, max]| {
        max > sta_core::Fixed::ZERO
            && 1.0 - now.to_f32() / max.to_f32() + 1e-6 >= threshold * f32::from(index)
    })
}

fn triangles(instance: &ModelInstance, part: &ModelPart) -> Vec<SurfaceTriangle> {
    let Some(node) = instance.model.nodes.get(part.node) else {
        return Vec::new();
    };
    let sta_assets::model::NodeKind::Mesh(mesh) = &node.kind else {
        return Vec::new();
    };
    let Some(group) = mesh.groups.get(part.group) else {
        return Vec::new();
    };
    group
        .faces
        .iter()
        .filter_map(|face| {
            let mut positions = [Vec3::ZERO; 3];
            for (i, &(p, _)) in face.iter().enumerate() {
                let [x, y, z] = *mesh.positions.get(p as usize)?;
                positions[i] = Vec3::new(x, y, -z) * instance.scale;
            }
            let normal = (positions[1] - positions[0])
                .cross(positions[2] - positions[0])
                .try_normalize()?;
            Some(SurfaceTriangle {
                positions,
                normals: [normal; 3],
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update(
    mut commands: Commands,
    roots: Query<(Entity, &SimEntity, &ModelInstance)>,
    nodes: Query<(Entity, &ModelNode, &Transform, &Visibility)>,
    parts: Query<
        (
            Entity,
            &ModelPart,
            &ChildOf,
            &Transform,
            Option<&GlobalTransform>,
            Option<&super::construction::ConstructionPart>,
        ),
        Without<ModelNode>,
    >,
    content: Res<ContentRes>,
    link: Res<SimLink>,
    vfs: Res<Mounted>,
    mut textures: LoaderParams,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cache: ResMut<Cache>,
    projections: Query<(Entity, &ProjectionPart)>,
) {
    if vfs.is_changed() {
        for old in cache.geometry.values().filter_map(|g| g.entity) {
            commands.entity(old).try_despawn();
        }
        release_cache(&mut cache, &mut meshes, &mut materials);
    }
    let live: HashSet<_> = roots.iter().map(|r| r.0).collect();
    let gone: Vec<_> = cache
        .sources
        .keys()
        .filter(|root| !live.contains(root))
        .copied()
        .collect();
    for root in gone {
        if let Some(sources) = cache.sources.remove(&root) {
            for source in sources {
                materials.remove(source.material.id());
            }
        }
    }
    for (entity, projection) in &projections {
        if !live.contains(&projection.root) || !parts.contains(projection.receiver) {
            trace!(
                "retire hull projection {} on {:?}",
                projection.placement, projection.receiver
            );
            commands.entity(entity).try_despawn();
        }
    }
    cache.geometry.retain(|(root, receiver, _), g| {
        if live.contains(root) && parts.contains(*receiver) {
            true
        } else {
            if let Some(e) = g.entity {
                commands.entity(e).try_despawn();
            }
            if let Some(mesh) = &g.mesh {
                meshes.remove(mesh.id());
            }
            false
        }
    });
    for (root, sim, instance) in &roots {
        if let std::collections::hash_map::Entry::Vacant(entry) = cache.sources.entry(root) {
            let Some(def) = content.0.unit(instance.def) else {
                continue;
            };
            entry.insert(sources(
                instance,
                sim.0,
                def,
                &mut textures.loader(),
                &mut materials,
            ));
        }
    }
    if cache.sources.values().all(Vec::is_empty) {
        return;
    }
    let mut node_groups: HashMap<Entity, Vec<(Entity, usize)>> = HashMap::new();
    for (entity, node, _, _) in &nodes {
        node_groups
            .entry(node.root)
            .or_default()
            .push((entity, node.index));
    }
    let mut part_groups: HashMap<Entity, Vec<Entity>> = HashMap::new();
    for (entity, _, parent, _, _, _) in &parts {
        if let Ok((_, node, _, _)) = nodes.get(parent.parent()) {
            part_groups.entry(node.root).or_default().push(entity);
        }
    }
    let view = link.curr();
    for (root, sim, instance) in &roots {
        let Some(unit) = view.unit(sim.0) else {
            continue;
        };
        let active = !unit.flags.contains(sta_sim::view::UnitFlags::CLOAKED)
            && !unit.flags.contains(sta_sim::view::UnitFlags::DYING)
            && !unit.progress.is_some_and(|p| p < sta_core::Fixed::ONE);

        // Move the source list out temporarily so geometry updates can borrow
        // the cache independently; it returns before processing another root.
        let sources = cache.sources.remove(&root).unwrap_or_default();
        if sources.is_empty() {
            cache.sources.insert(root, sources);
            continue;
        }
        let local: HashMap<_, _> = node_groups
            .get(&root)
            .into_iter()
            .flatten()
            .filter_map(|(e, index)| {
                nodes
                    .get(*e)
                    .ok()
                    .map(|(_, _, t, v)| (*index, (*e, t.to_matrix(), *v != Visibility::Hidden)))
            })
            .collect();
        for part_entity in part_groups.get(&root).into_iter().flatten() {
            let Ok((part_entity, part, parent, part_transform, part_global, construction)) =
                parts.get(*part_entity)
            else {
                continue;
            };
            if !local
                .get(&part.node)
                .is_some_and(|n| n.0 == parent.parent())
            {
                continue;
            }
            let Some(receiver) = node_matrix(part.node, instance, &local) else {
                continue;
            };
            let receiver = receiver * part_transform.to_matrix();
            for (i, source) in sources.iter().enumerate() {
                let key = (root, part_entity, i);
                let show = active
                    && construction.is_none()
                    && visible(part.node, instance, &local)
                    && part_transform.scale.min_element() > 0.0
                    && damage_visible(source, unit, view, &instance.policy);
                if !show {
                    if let Some(e) = cache.geometry.get(&key).and_then(|g| g.entity) {
                        commands.entity(e).insert(Visibility::Hidden);
                    }
                    continue;
                }
                let Some(hp) = instance
                    .model
                    .nodes
                    .iter()
                    .position(|n| n.name.eq_ignore_ascii_case(&source.placement.hardpoint))
                else {
                    cache.geometry.entry(key).or_insert_with(|| {
                        warn!(
                            "projection hardpoint unavailable: {}",
                            source.placement.hardpoint
                        );
                        Geometry {
                            matrix: [0; 16],
                            entity: None,
                            mesh: None,
                        }
                    });
                    continue;
                };
                let Some(anchor) = node_matrix(hp, instance, &local) else {
                    continue;
                };
                let projector = anchor * placement_matrix(&source.placement, instance.scale);
                let matrix = projector.inverse() * receiver;
                let fingerprint = matrix.to_cols_array().map(f32::to_bits);
                if let Some(entry) = cache.geometry.get(&key)
                    && entry.matrix == fingerprint
                {
                    if let Some(e) = entry.entity {
                        commands.entity(e).insert(Visibility::Inherited);
                    }
                    continue;
                }
                if let Some(old) = cache.geometry.remove(&key) {
                    if let Some(e) = old.entity {
                        commands.entity(e).try_despawn();
                    }
                    if let Some(mesh) = old.mesh {
                        meshes.remove(mesh.id());
                    }
                }
                let p = &source.placement;
                let projected = project_surface(
                    &triangles(instance, part),
                    &Projector {
                        receiver_to_projector: matrix,
                        width: p.size[0] * instance.scale,
                        height: p.size[1] * instance.scale,
                        depth: p.depth.unwrap_or(p.size[0].min(p.size[1]) * 0.25) * instance.scale,
                        facing_threshold: 0.15,
                        normal_bias: 0.002 * instance.scale,
                        flip_u: source.flip_u,
                    },
                );
                let mut owned_mesh = None;
                let entity = match projected {
                    Ok(mesh) if !mesh.indices.is_empty() => {
                        let mesh = Mesh::new(
                            PrimitiveTopology::TriangleList,
                            RenderAssetUsages::default(),
                        )
                        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, mesh.positions)
                        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, mesh.normals)
                        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, mesh.uvs)
                        .with_inserted_indices(Indices::U32(mesh.indices));
                        let handle = meshes.add(mesh);
                        owned_mesh = Some(handle.clone());
                        Some(
                            commands
                                .spawn((
                                    Mesh3d(handle),
                                    MeshMaterial3d(source.material.clone()),
                                    Transform::IDENTITY,
                                    part_global.copied().unwrap_or_default(),
                                    Visibility::Inherited,
                                    ChildOf(part_entity),
                                    ProjectionPart {
                                        root,
                                        receiver: part_entity,
                                        placement: i,
                                    },
                                ))
                                .id(),
                        )
                    }
                    Err(error) => {
                        warn!("invalid hull projection: {error:?}");
                        None
                    }
                    _ => None,
                };
                cache.geometry.insert(
                    key,
                    Geometry {
                        matrix: fingerprint,
                        entity,
                        mesh: owned_mesh,
                    },
                );
            }
        }
        cache.sources.insert(root, sources);
    }
}

#[cfg(test)]
mod gpu_tests;
#[cfg(test)]
mod tests;
