//! Per-instance construction reveal data and the shared weld frontier.
//!
//! Faces are sorted by centroid along the longest model-bounds axis (stable
//! node/group/face tie breaks), then split into equal-count spatial chunks.
//! This is deterministic presentation partitioning; it is not native-authored
//! build-section metadata.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use bevy::mesh::Indices;
use bevy::prelude::*;
use sta_sim::EntityId;

use crate::sim_link::{SimEntity, SimLink};

/// World-space edge-midpoint targets on each active station's next chunk.
#[derive(Resource, Default)]
pub struct ConstructionEdges {
    pub sites: BTreeMap<EntityId, Vec<Vec3>>,
}

/// Weld-gated visual start point for each station construction sequence.
/// Missing site entries leave progress ungated; an explicit `None` holds at 0.
#[derive(Resource, Default)]
pub struct ConstructionWork {
    pub starts: BTreeMap<EntityId, Option<f32>>,
}

/// Convert simulation progress to visual progress after the first authored weld.
/// A full simulation completion always restores the full original mesh.
pub(crate) fn effective_progress(progress: f32, start: Option<Option<f32>>) -> f32 {
    if progress.is_finite() && progress >= 1.0 {
        return 1.0;
    }
    if !progress.is_finite() {
        return 0.0;
    }
    let progress = progress.clamp(0.0, 1.0);
    match start {
        None => progress,
        Some(None) => 0.0,
        Some(Some(start)) if !start.is_finite() || start >= 1.0 => 0.0,
        Some(Some(start)) => {
            let start = start.clamp(0.0, 1.0);
            ((progress - start) / (1.0 - start)).clamp(0.0, 1.0)
        }
    }
}

/// CPU-side indices and real triangle-edge midpoints for one rendered part.
/// This Arc exists only on an unfinished instance's mesh entity.
pub(crate) struct ConstructionSource {
    pub(crate) stage_indices: Vec<Vec<u32>>,
    pub(crate) stage_edges: Vec<Vec<Vec3>>,
}

/// A mesh temporarily using a private partial index buffer.
#[derive(Component)]
pub struct ConstructionPart {
    pub(crate) owner: Entity,
    pub(crate) original: Handle<Mesh>,
    pub(crate) partial: Handle<Mesh>,
    pub(crate) source: Arc<ConstructionSource>,
    pub(crate) completed: usize,
}

/// A non-mesh node hidden until its owning construction job completes.
#[derive(Component)]
pub(crate) struct ConstructionDecor {
    pub(crate) owner: Entity,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct FaceId(usize, usize, usize);

type EdgeKey = [u32; 6];

#[derive(Clone, Copy)]
struct FaceRecord {
    id: FaceId,
    centroid: Vec3,
    points: [Vec3; 3],
    edges: [EdgeKey; 3],
}

pub(crate) struct ConstructionPlan {
    pub(crate) chunk_count: usize,
    face_stages: HashMap<FaceId, usize>,
    target_edges: Vec<HashSet<EdgeKey>>,
}

fn point_key(point: Vec3) -> [u32; 3] {
    point
        .to_array()
        .map(|v| if v == 0.0 { 0.0f32.to_bits() } else { v.to_bits() })
}

fn edge_key(a: Vec3, b: Vec3) -> EdgeKey {
    let (a, b) = (point_key(a), point_key(b));
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]
}

impl ConstructionPlan {
    pub(crate) fn build(
        model: &sta_assets::model::Model,
        requested: u16,
        scale: f32,
        visible_nodes: &HashSet<usize>,
    ) -> Option<Self> {
        let mut records = Vec::new();
        let (mut bounds_min, mut bounds_max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for (node_index, node) in model.nodes.iter().enumerate() {
            if !visible_nodes.contains(&node_index) {
                continue;
            }
            let Some(mesh) = node.mesh() else { continue };
            let transform = model.model_transform(node_index);
            for (group_index, group) in mesh.groups.iter().enumerate() {
                for (face_index, face) in group.faces.iter().enumerate() {
                    let mut points = [Vec3::ZERO; 3];
                    let mut valid = true;
                    for corner in 0..3 {
                        let (position, uv) = face[corner];
                        let Some(source) = mesh.positions.get(position as usize) else {
                            valid = false;
                            break;
                        };
                        if mesh.uvs.get(uv as usize).is_none() {
                            valid = false;
                            break;
                        }
                        let transformed = transform.transform_point(*source);
                        points[corner] = Vec3::new(transformed[0], transformed[1], -transformed[2]) * scale;
                        bounds_min = bounds_min.min(points[corner]);
                        bounds_max = bounds_max.max(points[corner]);
                    }
                    if !valid {
                        continue;
                    }
                    let centroid = (points[0] + points[1] + points[2]) / 3.0;
                    records.push(FaceRecord {
                        id: FaceId(node_index, group_index, face_index),
                        centroid,
                        points,
                        edges: [
                            edge_key(points[0], points[1]),
                            edge_key(points[1], points[2]),
                            edge_key(points[2], points[0]),
                        ],
                    });
                }
            }
        }
        if records.is_empty() {
            return Some(Self {
                chunk_count: 1,
                face_stages: HashMap::new(),
                target_edges: vec![HashSet::new()],
            });
        }

        let chunk_count = (requested as usize).clamp(1, 256).min(records.len());
        let spans = [
            bounds_max.x - bounds_min.x,
            bounds_max.y - bounds_min.y,
            bounds_max.z - bounds_min.z,
        ];
        let axis = (0..3).max_by(|&a, &b| spans[a].total_cmp(&spans[b])).unwrap_or(0);
        records.sort_by(|a, b| {
            a.centroid[axis]
                .total_cmp(&b.centroid[axis])
                .then_with(|| a.centroid.x.total_cmp(&b.centroid.x))
                .then_with(|| a.centroid.y.total_cmp(&b.centroid.y))
                .then_with(|| a.centroid.z.total_cmp(&b.centroid.z))
                .then_with(|| (a.id.0, a.id.1, a.id.2).cmp(&(b.id.0, b.id.1, b.id.2)))
        });
        let mut face_stages = HashMap::with_capacity(records.len());
        let mut edge_occurrences: HashMap<EdgeKey, Vec<(usize, Vec3)>> = HashMap::new();
        for (rank, record) in records.iter().enumerate() {
            let stage = rank * chunk_count / records.len();
            face_stages.insert(record.id, stage);
            for edge in 0..3 {
                edge_occurrences
                    .entry(record.edges[edge])
                    .or_default()
                    .push((stage, (record.points[edge] + record.points[(edge + 1) % 3]) * 0.5));
            }
        }

        let mut target_edges = vec![HashSet::new(); chunk_count];
        for (key, occurrences) in &edge_occurrences {
            for (stage, targets) in target_edges.iter_mut().enumerate() {
                let here = occurrences.iter().filter(|(s, _)| *s == stage).count();
                if here != 1 {
                    continue;
                } // Same-chunk shared edges are internal.
                let prior = occurrences.iter().any(|(s, _)| *s < stage);
                if prior {
                    targets.insert(*key);
                }
            }
        }
        for (stage, targets) in target_edges.iter_mut().enumerate() {
            if !targets.is_empty() {
                continue;
            }
            // With no revealed frontier, use true exterior edges first, then
            // edges against later chunks. These are actual mesh edge midpoints.
            for (key, occurrences) in &edge_occurrences {
                let here = occurrences.iter().filter(|(s, _)| *s == stage).count();
                if here == 1 && occurrences.len() == 1 {
                    targets.insert(*key);
                }
            }
            if !targets.is_empty() {
                continue;
            }
            for (key, occurrences) in &edge_occurrences {
                let here = occurrences.iter().filter(|(s, _)| *s == stage).count();
                let future = occurrences.iter().any(|(s, _)| *s > stage);
                if here == 1 && future {
                    targets.insert(*key);
                }
            }
            if !targets.is_empty() {
                continue;
            }
            // Closed single-chunk meshes have no mathematical boundary. Keep a
            // deterministic real edge midpoint as a usable final weld target.
            let fallback = edge_occurrences
                .iter()
                .filter(|(_, occurrences)| occurrences.iter().any(|(s, _)| *s == stage))
                .map(|(key, _)| *key)
                .min();
            if let Some(edge) = fallback {
                targets.insert(edge);
            }
        }
        Some(Self {
            chunk_count,
            face_stages,
            target_edges,
        })
    }

    pub(crate) fn part_source(
        &self,
        model: &sta_assets::model::Model,
        node_index: usize,
        group_index: usize,
        model_mesh: &sta_assets::model::Mesh,
        scale: f32,
    ) -> Arc<ConstructionSource> {
        let mut stage_indices = vec![Vec::new(); self.chunk_count];
        let mut stage_edges = vec![Vec::new(); self.chunk_count];
        let mut remap = HashMap::<(u32, u32), u32>::new();
        let mut edge_seen = vec![HashSet::<[u32; 3]>::new(); self.chunk_count];
        let transform = model.model_transform(node_index);
        for (face_index, face) in model_mesh.groups[group_index].faces.iter().enumerate() {
            let Some(&stage) = self.face_stages.get(&FaceId(node_index, group_index, face_index)) else {
                continue;
            };
            let mut output = [0u32; 3];
            let mut local = [Vec3::ZERO; 3];
            let mut valid = true;
            for corner in 0..3 {
                let (position, uv) = face[corner];
                let Some(source) = model_mesh.positions.get(position as usize) else {
                    valid = false;
                    break;
                };
                if model_mesh.uvs.get(uv as usize).is_none() {
                    valid = false;
                    break;
                }
                local[corner] = Vec3::new(source[0], source[1], -source[2]) * scale;
                let next = remap.len() as u32;
                output[corner] = *remap.entry((position, uv)).or_insert(next);
            }
            if !valid {
                continue;
            }
            stage_indices[stage].extend_from_slice(&output);
            for edge in 0..3 {
                let (p0, p1) = (face[edge].0 as usize, face[(edge + 1) % 3].0 as usize);
                let source0 = model_mesh.positions[p0];
                let source1 = model_mesh.positions[p1];
                let world0 = transform.transform_point(source0);
                let world1 = transform.transform_point(source1);
                let point0 = Vec3::new(world0[0], world0[1], -world0[2]) * scale;
                let point1 = Vec3::new(world1[0], world1[1], -world1[2]) * scale;
                let key = edge_key(point0, point1);
                if self.target_edges[stage].contains(&key) {
                    let midpoint = (local[edge] + local[(edge + 1) % 3]) * 0.5;
                    if edge_seen[stage].insert(point_key(midpoint)) {
                        stage_edges[stage].push(midpoint);
                    }
                }
            }
        }
        Arc::new(ConstructionSource {
            stage_indices,
            stage_edges,
        })
    }
}

fn stage_indices(source: &ConstructionSource, completed: usize) -> Vec<u32> {
    source
        .stage_indices
        .iter()
        .take(completed)
        .flat_map(|indices| indices.iter().copied())
        .collect()
}

fn completed_stages(progress: f32, count: usize) -> usize {
    (progress.clamp(0.0, 1.0) * count as f32).floor() as usize
}

/// Advance active partial meshes and publish the next chunk's transformed edge
/// midpoints. Runs after transform propagation so targets use current hierarchy.
#[allow(clippy::too_many_arguments)]
pub fn sync_reveal(
    mut commands: Commands,
    link: Res<SimLink>,
    work: Option<Res<ConstructionWork>>,
    roots: Query<(Entity, &SimEntity)>,
    mut parts: Query<(Entity, &GlobalTransform, &mut Mesh3d, &mut ConstructionPart)>,
    mut decorations: Query<(Entity, &ConstructionDecor, &mut Visibility)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut edges: ResMut<ConstructionEdges>,
) {
    edges.sites.clear();
    let by_root: HashMap<Entity, (EntityId, f32)> = roots
        .iter()
        .filter_map(|(entity, sim)| {
            link.curr().unit(sim.0).and_then(|unit| {
                unit.progress
                    .map(|progress| (entity, (sim.0, progress.to_f32().clamp(0.0, 1.0))))
            })
        })
        .collect();
    for (entity, decor, mut visibility) in &mut decorations {
        let complete = by_root.get(&decor.owner).is_none_or(|(_, progress)| *progress >= 1.0);
        if complete {
            *visibility = Visibility::Inherited;
            commands.entity(entity).remove::<ConstructionDecor>();
        }
    }
    let mut seen_targets = HashMap::<EntityId, HashSet<[u32; 3]>>::new();
    for (entity, transform, mut mesh_handle, mut construction) in &mut parts {
        let Some((site, sim_progress)) = by_root.get(&construction.owner).copied() else {
            restore(&mut commands, &mut meshes, entity, &mut mesh_handle, &construction);
            continue;
        };
        let gate = work.as_ref().and_then(|work| work.starts.get(&site).copied());
        let progress = effective_progress(sim_progress, gate);
        let count = construction.source.stage_indices.len();
        let completed = completed_stages(progress, count);
        if completed >= count {
            restore(&mut commands, &mut meshes, entity, &mut mesh_handle, &construction);
            continue;
        }
        if completed != construction.completed
            && let Some(mut mesh) = meshes.get_mut(&construction.partial)
        {
            mesh.insert_indices(Indices::U32(stage_indices(&construction.source, completed)));
            construction.completed = completed;
        }
        if let Some(targets) = construction.source.stage_edges.get(completed) {
            let site_targets = edges.sites.entry(site).or_default();
            let seen = seen_targets.entry(site).or_default();
            for point in targets {
                let world = transform.transform_point(*point);
                if seen.insert(point_key(world)) {
                    site_targets.push(world);
                }
            }
        }
    }
}

fn restore(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    entity: Entity,
    mesh: &mut Mesh3d,
    construction: &ConstructionPart,
) {
    restore_mesh(meshes, mesh, &construction.original, &construction.partial);
    commands.entity(entity).remove::<ConstructionPart>();
}

fn restore_mesh(meshes: &mut Assets<Mesh>, mesh: &mut Mesh3d, original: &Handle<Mesh>, partial: &Handle<Mesh>) {
    *mesh = Mesh3d(original.clone());
    meshes.remove(partial.id());
}

#[cfg(test)]
#[path = "construction_gpu.rs"]
mod gpu_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;
    use sta_assets::model::{Group, Mesh as ModelMesh, Model, Node, NodeKind, SourceFormat};

    fn model(groups: Vec<Group>, node_translation: [f32; 3]) -> Model {
        let mut transform = sta_assets::a2fmt::Matrix34::IDENTITY;
        transform.m[9..12].copy_from_slice(&node_translation);
        Model {
            format: SourceFormat::M3d,
            version: 2.06,
            materials: Vec::new(),
            nodes: vec![Node {
                name: "mesh".to_owned(),
                parent: None,
                children: Vec::new(),
                transform,
                kind: NodeKind::Mesh(ModelMesh {
                    positions: vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
                    uvs: vec![[0.0, 0.0]; 4],
                    groups,
                    two_sided: false,
                }),
            }],
            channels: Vec::new(),
            flipbooks: Vec::new(),
            warnings: Vec::new(),
            pbr: None,
        }
    }

    fn group(faces: Vec<[(u32, u32); 3]>) -> Group {
        Group {
            material: "hull".to_owned(),
            blend: "default".to_owned(),
            maps: Default::default(),
            faces,
        }
    }

    fn square_groups() -> Vec<Group> {
        vec![group(vec![[(0, 0), (1, 1), (2, 2)], [(0, 0), (2, 2), (3, 3)]])]
    }

    #[test]
    fn empty_partial_and_full_stages_assign_each_triangle_once() {
        let visible = HashSet::from([0]);
        let empty = model(vec![group(Vec::new())], [0.0; 3]);
        let empty_plan = ConstructionPlan::build(&empty, 24, 1.0, &visible).unwrap();
        assert_eq!(empty_plan.chunk_count, 1);
        assert!(empty_plan.face_stages.is_empty());

        let model = model(square_groups(), [0.0; 3]);
        let plan = ConstructionPlan::build(&model, 2, 1.0, &visible).unwrap();
        let mesh = model.nodes[0].mesh().unwrap();
        let source = plan.part_source(&model, 0, 0, mesh, 1.0);
        let count = |completed| stage_indices(&source, completed).len() / 3;
        assert_eq!((count(0), count(1), count(2)), (0, 1, 2));
        assert_eq!(source.stage_indices.iter().map(|s| s.len() / 3).sum::<usize>(), 2);
    }

    #[test]
    fn first_weld_gate_holds_then_maps_remaining_progress_without_a_jump() {
        let progress = 0.5;
        assert_eq!(effective_progress(progress, Some(None)), 0.0);
        let model = model(square_groups(), [0.0; 3]);
        let plan = ConstructionPlan::build(&model, 2, 1.0, &HashSet::from([0])).unwrap();
        let source = plan.part_source(&model, 0, 0, model.nodes[0].mesh().unwrap(), 1.0);
        assert_eq!(
            completed_stages(effective_progress(progress, Some(None)), plan.chunk_count),
            0
        );
        assert!(stage_indices(&source, 0).is_empty());
        assert!(
            !source.stage_edges[0].is_empty(),
            "waiting bees must still target the first chunk"
        );

        // Latching the first weld at the current sim progress starts at visual 0.
        assert_eq!(effective_progress(progress, Some(Some(progress))), 0.0);
        assert_eq!(
            completed_stages(effective_progress(progress, Some(Some(progress))), plan.chunk_count),
            0
        );
        assert_eq!(effective_progress(0.75, Some(Some(progress))), 0.5);
        assert_eq!(
            completed_stages(effective_progress(0.75, Some(Some(progress))), plan.chunk_count),
            1
        );
        assert_eq!(
            effective_progress(0.75, None),
            0.75,
            "missing work entry remains ungated"
        );
        assert_eq!(
            effective_progress(1.0, Some(None)),
            1.0,
            "sim completion always restores full geometry"
        );
        assert_eq!(effective_progress(0.5, Some(Some(f32::NAN))), 0.0);
        assert_eq!(effective_progress(0.5, Some(Some(1.0))), 0.0);
    }

    #[test]
    fn targets_are_real_pending_edges_transformed_through_hierarchy() {
        let model = model(square_groups(), [3.0, 4.0, 5.0]);
        let plan = ConstructionPlan::build(&model, 2, 1.0, &HashSet::from([0])).unwrap();
        let mesh = model.nodes[0].mesh().unwrap();
        let source = plan.part_source(&model, 0, 0, mesh, 1.0);
        let next_stage = 1;
        assert!(!source.stage_edges[next_stage].is_empty());
        let local_edge = source.stage_edges[next_stage][0];
        let node = crate::render::model_scene::to_bevy(&model.nodes[0].transform);
        let root = Transform::from_xyz(10.0, 20.0, 30.0);
        let world = GlobalTransform::from(root) * GlobalTransform::from(node);
        let target = world.transform_point(local_edge);
        assert!(target.distance(root.translation + node.transform_point(local_edge)) < 1e-5);
        // The pending stage contains exactly one half-edge of the shared diagonal.
        assert!(
            source.stage_edges[next_stage]
                .iter()
                .any(|point| point.distance(Vec3::new(1.0, 0.5, 0.0)) < 1e-5)
        );
    }

    #[test]
    fn sparse_groups_and_hidden_nodes_do_not_enter_the_plan() {
        let model = model(vec![group(Vec::new()), group(vec![[(0, 0), (1, 1), (2, 2)]])], [0.0; 3]);
        let plan = ConstructionPlan::build(&model, 24, 1.0, &HashSet::from([0])).unwrap();
        assert_eq!(
            plan.face_stages.keys().copied().collect::<Vec<_>>(),
            vec![FaceId(0, 1, 0)]
        );
        let mesh = model.nodes[0].mesh().unwrap();
        assert!(
            plan.part_source(&model, 0, 0, mesh, 1.0)
                .stage_indices
                .iter()
                .all(Vec::is_empty)
        );
        assert_eq!(
            plan.part_source(&model, 0, 1, mesh, 1.0)
                .stage_indices
                .iter()
                .map(|s| s.len())
                .sum::<usize>(),
            3
        );

        let hidden = ConstructionPlan::build(&model, 24, 1.0, &HashSet::new()).unwrap();
        assert!(hidden.face_stages.is_empty());
    }

    #[test]
    fn private_meshes_are_independent_and_completion_restores_cached_mesh() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>();
        let original = app.world_mut().resource_mut::<Assets<Mesh>>().add(
            Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
                .with_inserted_indices(Indices::U32(vec![0, 1, 2, 3, 4, 5])),
        );
        let make_partial = |assets: &mut Assets<Mesh>| {
            let mut mesh = assets.get(&original).unwrap().clone();
            mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
            assets.add(mesh)
        };
        let partial_a = make_partial(&mut app.world_mut().resource_mut::<Assets<Mesh>>());
        let partial_b = make_partial(&mut app.world_mut().resource_mut::<Assets<Mesh>>());
        assert_ne!(partial_a.id(), partial_b.id());
        {
            let assets = app.world().resource::<Assets<Mesh>>();
            assert_eq!(
                assets.get(&original).unwrap().indices(),
                Some(&Indices::U32(vec![0, 1, 2, 3, 4, 5]))
            );
            assert_eq!(
                assets.get(&partial_a).unwrap().indices(),
                Some(&Indices::U32(vec![0, 1, 2]))
            );
        }
        let mut mesh3d = Mesh3d(partial_a.clone());
        restore_mesh(
            &mut app.world_mut().resource_mut::<Assets<Mesh>>(),
            &mut mesh3d,
            &original,
            &partial_a,
        );
        assert_eq!(mesh3d.0.id(), original.id());
        let assets = app.world().resource::<Assets<Mesh>>();
        assert!(assets.get(&partial_a).is_none());
        assert!(assets.get(&partial_b).is_some());
        assert!(assets.get(&original).is_some());
    }
}
