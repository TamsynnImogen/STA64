//! Presentation-only subsystem explosions and repair glints. Events are emitted
//! by appearance sync, after it has selected an actual model node.

use bevy::prelude::*;
use bevy::transform::TransformSystems;
use sta_assets::model::{Model, NodeKind};

use crate::app::AppState;
use crate::clock::AnimClock;
use crate::render::units::{Cloaked, NeedsModel, TeamColour};
use crate::sim_link::ContentRes;

use super::{ModelInstance, ModelNode};

const REPAIR_LIFE_TICKS: u64 = 15;
const EXPLOSION_LIFE_TICKS: u64 = 90;

#[derive(Message, Clone, Debug)]
pub struct PartEffect {
    pub root: Entity,
    pub node: usize,
    pub kind: PartEffectKind,
    pub seed: u32,
}

#[derive(Clone, Debug)]
pub enum PartEffectKind {
    Destroyed { explosion: Option<String> },
    Repair,
    Completed,
}

#[derive(Component)]
struct Lifetime {
    owner: Entity,
    born: u64,
    ticks: u64,
}

#[derive(Resource, Default)]
struct RepairAssets {
    mesh: Option<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
}

/// Register the message and the post-transform adapter. The lead emits
/// `PartEffect` from appearance sync and calls this once during app setup.
pub fn install(app: &mut App) {
    app.add_message::<PartEffect>()
        .init_resource::<RepairAssets>()
        .add_systems(
            PostUpdate,
            (spawn_effects, expire_effects)
                .chain()
                .after(TransformSystems::Propagate)
                .run_if(resource_exists::<AnimClock>),
        )
        .add_systems(OnExit(AppState::InGame), clear_effects);
}

#[allow(clippy::too_many_arguments)]
fn spawn_effects(
    mut commands: Commands,
    mut events: MessageReader<PartEffect>,
    clock: Res<AnimClock>,
    roots: Query<(&ModelInstance, Option<&Cloaked>, Option<&Visibility>)>,
    nodes: Query<(&ModelNode, &GlobalTransform)>,
    content: Option<Res<ContentRes>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut repair_assets: ResMut<RepairAssets>,
) {
    let (mut meshes, mut materials) = (meshes, materials);
    for event in events.read() {
        let Ok((instance, cloaked, visibility)) = roots.get(event.root) else {
            continue;
        };
        if cloaked.is_some() || visibility.is_some_and(|v| *v == Visibility::Hidden) {
            continue;
        }
        let Some((_, node_global)) = nodes
            .iter()
            .find(|(node, _)| node.root == event.root && node.index == event.node)
        else {
            continue;
        };
        match &event.kind {
            PartEffectKind::Destroyed { explosion } => {
                let Some(explosion) = explosion else { continue };
                let Some(key) = content.as_ref().and_then(|db| db.0.key(explosion)) else {
                    warn!("subsystem explosion ODF unavailable: {explosion}");
                    continue;
                };
                if !content.as_ref().is_some_and(|db| db.0.unit(key).is_some()) {
                    warn!("subsystem explosion ODF is not a unit: {explosion}");
                    continue;
                }
                commands.spawn((
                    NeedsModel(key),
                    TeamColour(Color::WHITE),
                    node_global.compute_transform(),
                    Visibility::default(),
                    Lifetime {
                        owner: event.root,
                        born: clock.tick(),
                        ticks: EXPLOSION_LIFE_TICKS,
                    },
                ));
            }
            PartEffectKind::Repair | PartEffectKind::Completed => {
                let (Some(meshes), Some(materials)) =
                    (meshes.as_deref_mut(), materials.as_deref_mut())
                else {
                    continue;
                };
                let mesh = repair_assets
                    .mesh
                    .get_or_insert_with(|| meshes.add(Sphere::new(1.0)))
                    .clone();
                let material = repair_assets
                    .material
                    .get_or_insert_with(|| {
                        let colour = Color::srgb(0.42, 0.9, 1.0);
                        materials.add(StandardMaterial {
                            base_color: colour.with_alpha(0.8),
                            emissive: colour.to_linear() * 3.0,
                            unlit: true,
                            alpha_mode: AlphaMode::Add,
                            ..default()
                        })
                    })
                    .clone();
                let count = if matches!(&event.kind, PartEffectKind::Repair) {
                    2
                } else {
                    8
                };
                let radius = instance
                    .model
                    .bounds()
                    .map(|(lo, hi)| {
                        (Vec3::from_array(hi) - Vec3::from_array(lo)).length()
                            * instance.scale.abs()
                            * 0.004
                    })
                    .unwrap_or(0.1)
                    .clamp(0.025, 2.0);
                for ordinal in 0..count {
                    let local =
                        match surface_point(&instance.model, event.node, event.seed, ordinal) {
                            Some(point) => point,
                            None if !matches!(
                                instance.model.nodes.get(event.node).map(|node| &node.kind),
                                Some(NodeKind::Mesh(_))
                            ) =>
                            {
                                Vec3::ZERO
                            }
                            None => continue,
                        } * Vec3::new(instance.scale, instance.scale, -instance.scale);
                    let world = node_global.transform_point(local);
                    if !world.is_finite() {
                        continue;
                    }
                    commands.spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_translation(world).with_scale(Vec3::splat(radius)),
                        Visibility::default(),
                        Lifetime {
                            owner: event.root,
                            born: clock.tick(),
                            ticks: REPAIR_LIFE_TICKS,
                        },
                    ));
                }
            }
        }
    }
}

/// A deterministic point on a real source triangle. `None` is only for a
/// group or node without usable mesh triangles; callers then use node origin.
fn surface_point(model: &Model, node: usize, seed: u32, ordinal: usize) -> Option<Vec3> {
    let NodeKind::Mesh(mesh) = &model.nodes.get(node)?.kind else {
        return None;
    };
    let faces: Vec<_> = mesh
        .groups
        .iter()
        .flat_map(|group| &group.faces)
        .filter(|face| {
            face.iter()
                .all(|corner| mesh.positions.get(corner.0 as usize).is_some())
        })
        .collect();
    if faces.is_empty() {
        return None;
    }
    let mut state = (seed as u64) ^ (ordinal as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    state = mix(state);
    let face = faces[(state % faces.len() as u64) as usize];
    let a = Vec3::from_array(mesh.positions[face[0].0 as usize]);
    let b = Vec3::from_array(mesh.positions[face[1].0 as usize]);
    let c = Vec3::from_array(mesh.positions[face[2].0 as usize]);
    if !a.is_finite() || !b.is_finite() || !c.is_finite() {
        return None;
    }
    let u = ((mix(state ^ 0xA5A5) >> 40) as f32) / (1u32 << 24) as f32;
    let v = ((mix(state ^ 0x5A5A) >> 40) as f32) / (1u32 << 24) as f32;
    let root = u.sqrt();
    Some(a * (1.0 - root) + b * (root * (1.0 - v)) + c * (root * v))
}

fn mix(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn expire_effects(
    mut commands: Commands,
    clock: Res<AnimClock>,
    effects: Query<(Entity, &Lifetime)>,
    owners: Query<(Option<&Cloaked>, Option<&Visibility>), With<ModelInstance>>,
) {
    for (entity, lifetime) in &effects {
        let hidden = owners
            .get(lifetime.owner)
            .map_or(true, |(cloak, visibility)| {
                cloak.is_some() || visibility.is_some_and(|v| *v == Visibility::Hidden)
            });
        if hidden || clock.tick().saturating_sub(lifetime.born) >= lifetime.ticks {
            commands.entity(entity).try_despawn();
        }
    }
}

fn clear_effects(
    mut commands: Commands,
    effects: Query<Entity, With<Lifetime>>,
    mut assets: ResMut<RepairAssets>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    for entity in &effects {
        commands.entity(entity).try_despawn();
    }
    if let (Some(mut meshes), Some(handle)) = (meshes, assets.mesh.take()) {
        meshes.remove(&handle);
    }
    if let (Some(mut materials), Some(handle)) = (materials, assets.material.take()) {
        materials.remove(&handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_assets::model::{Group, Maps, Mesh as ModelMesh, Node, SourceFormat};

    fn one_triangle() -> Model {
        Model {
            format: SourceFormat::M3d,
            version: 2.0,
            materials: Vec::new(),
            nodes: vec![Node {
                name: "part".into(),
                parent: None,
                children: Vec::new(),
                transform: sta_assets::a2fmt::Matrix34::IDENTITY,
                kind: NodeKind::Mesh(ModelMesh {
                    positions: vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
                    uvs: vec![[0.0, 0.0]; 3],
                    groups: vec![Group {
                        material: "synthetic".into(),
                        blend: "default".into(),
                        maps: Maps::default(),
                        faces: vec![[(0, 0), (1, 1), (2, 2)]],
                    }],
                    two_sided: false,
                }),
            }],
            channels: Vec::new(),
            flipbooks: Vec::new(),
            warnings: Vec::new(),
            pbr: None,
        }
    }

    #[test]
    fn deterministic_burst_points_stay_on_selected_triangle() {
        let model = one_triangle();
        for ordinal in 0..8 {
            let point = surface_point(&model, 0, 77, ordinal).unwrap();
            assert!(point.x >= 0.0 && point.y >= 0.0 && point.x + point.y <= 2.000001);
            assert_eq!(point.z, 0.0);
            assert_eq!(surface_point(&model, 0, 77, ordinal), Some(point));
        }
        assert_eq!(surface_point(&model, 99, 77, 0), None);
    }

    #[test]
    fn expired_and_missing_owner_effects_are_removed() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<AnimClock>();
        app.init_resource::<RepairAssets>();
        app.add_systems(Update, expire_effects);
        let owner = app
            .world_mut()
            .spawn((
                ModelInstance {
                    def: sta_core::ContentKey::NONE,
                    model: std::sync::Arc::new(one_triangle()),
                    scale: 1.0,
                    policy: std::sync::Arc::new(Default::default()),
                },
                Visibility::default(),
            ))
            .id();
        let gone = app
            .world_mut()
            .spawn(Lifetime {
                owner,
                born: 0,
                ticks: 0,
            })
            .id();
        app.update();
        assert!(app.world().get_entity(gone).is_err());
        let hidden = app
            .world_mut()
            .spawn(Lifetime {
                owner,
                born: 0,
                ticks: 99,
            })
            .id();
        app.world_mut().entity_mut(owner).insert(Cloaked);
        app.update();
        assert!(app.world().get_entity(hidden).is_err());
        let fog_gone = app
            .world_mut()
            .spawn(Lifetime {
                owner,
                born: 0,
                ticks: 99,
            })
            .id();
        app.world_mut().entity_mut(owner).despawn();
        app.update();
        assert!(app.world().get_entity(fog_gone).is_err());
    }
}
