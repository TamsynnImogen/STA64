//! Render authored emissive texels into a private mask, using the same mesh,
//! bindings and depth as the visible hull. Bloom never reads scene brightness.

use std::ops::Range;

use bevy::asset::UntypedAssetId;
use bevy::camera::{MainPassResolutionOverride, Viewport};
use bevy::core_pipeline::core_3d::{
    Opaque3dBatchSetKey, Opaque3dBinKey, main_opaque_pass_3d, main_transparent_pass_3d,
};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    DrawMaterial, MATERIAL_BIND_GROUP_INDEX, MeshPipeline, MeshPipelineKey, MeshPipelineSystems,
    RenderMaterialInstances, RenderMeshInstances, ViewKeyCache,
};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::batching::gpu_preprocessing::{GpuPreprocessingMode, GpuPreprocessingSupport};
use bevy::render::camera::ExtractedCamera;
use bevy::render::mesh::{RenderMesh, allocator::MeshAllocator};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{
    AddRenderCommand, BinnedPhaseItem, BinnedRenderPhasePlugin, BinnedRenderPhaseType,
    CachedRenderPipelinePhaseItem, DrawFunctionId, DrawFunctions, PhaseItem, PhaseItemExtraIndex,
    ViewBinnedRenderPhases,
};
use bevy::render::render_resource::{
    AsBindGroup, BindGroupLayoutDescriptor, CachedRenderPipelineId, ColorTargetState, ColorWrites,
    CompareFunction, Face, PipelineCache, RenderPassDescriptor, RenderPipelineDescriptor,
    SpecializedMeshPipeline, SpecializedMeshPipelineError, SpecializedMeshPipelines, StoreOp,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::sync_world::{MainEntity, RenderEntity};
use bevy::render::view::{
    ExtractedView, NoIndirectDrawing, RenderVisibleEntities, RetainedViewEntity,
    ViewDepthStencilTexture,
};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};

use super::{HullMaterial, emissive_blur};
use crate::render::post::PostFx;

pub(super) fn register(app: &mut App) {
    app.add_plugins(BinnedRenderPhasePlugin::<Emissive3d, MeshPipeline>::new(
        default(),
    ));
    emissive_blur::register(app);
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .init_resource::<EmissiveMaterials>()
        .init_resource::<SpecializedMeshPipelines<MaskPipeline>>()
        .init_resource::<DrawFunctions<Emissive3d>>()
        .add_render_command::<Emissive3d, DrawMaterial>()
        .add_systems(RenderStartup, init_pipeline.after(MeshPipelineSystems))
        .add_systems(ExtractSchedule, extract)
        .add_systems(Render, queue.in_set(RenderSystems::QueueMeshes))
        .add_systems(
            Core3d,
            (draw_mask, emissive_blur::composite)
                .chain()
                .in_set(Core3dSystems::MainPass)
                .after(main_opaque_pass_3d)
                .before(main_transparent_pass_3d),
        );
}

#[derive(Component)]
pub(super) struct EmissiveBloomStrength(pub f32);

#[derive(Resource, Default)]
struct EmissiveMaterials(HashMap<UntypedAssetId, bool>);

type Cameras<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        RenderEntity,
        &'static Camera,
        Has<NoIndirectDrawing>,
        Has<crate::map::BackgroundCamera>,
        Has<crate::render::action_camera::ActionCamera>,
    ),
    With<Camera3d>,
>;

fn extract(
    mut commands: Commands,
    fx: Extract<Option<Res<PostFx>>>,
    materials: Extract<Res<Assets<HullMaterial>>>,
    cameras: Extract<Cameras>,
    support: Res<GpuPreprocessingSupport>,
    mut emissive: ResMut<EmissiveMaterials>,
    mut phases: ResMut<ViewBinnedRenderPhases<Emissive3d>>,
) {
    let strength = fx
        .as_ref()
        .filter(|fx| fx.bloom)
        .map_or(0.0, |fx| fx.bloom_intensity.clamp(0.0, 1.0));
    emissive.0.clear();
    phases.clear();
    if strength > 0.0 {
        emissive
            .0
            .extend(materials.iter().filter_map(|(id, material)| {
                (material.emissive_map.is_some()
                    && (material.params.emission_a.max_element() > 0.0
                        || material.params.emission_b.x.max(material.params.emission_b.y) > 0.0))
                    .then_some((id.untyped(), material.double_sided))
            }));
    }

    for (main, render, camera, no_indirect, background, action) in &cameras {
        let Ok(mut entity) = commands.get_entity(render) else {
            continue;
        };
        if !emissive.0.is_empty() && camera.is_active && !background && !action {
            entity.insert(EmissiveBloomStrength(strength));
            let mode = if no_indirect {
                GpuPreprocessingMode::PreprocessingOnly
            } else {
                GpuPreprocessingMode::Culling
            };
            phases.prepare_for_new_frame(
                RetainedViewEntity::new(main.into(), None, 0),
                support.min(mode),
            );
        } else {
            entity.remove::<EmissiveBloomStrength>();
        }
    }
}

#[derive(Resource)]
struct MaskPipeline {
    mesh: MeshPipeline,
    material: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
}

fn init_pipeline(
    mut commands: Commands,
    mesh: Res<MeshPipeline>,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
) {
    commands.insert_resource(MaskPipeline {
        mesh: mesh.clone(),
        material: HullMaterial::bind_group_layout_descriptor(&device),
        shader: assets.load("embedded://sta64/render/hulls.wesl"),
    });
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct MaskKey {
    mesh: MeshPipelineKey,
    double_sided: bool,
}

impl SpecializedMeshPipeline for MaskPipeline {
    type Key = MaskKey;

    fn specialize(
        &self,
        key: MaskKey,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh.specialize(key.mesh, layout)?;
        descriptor.label = Some("emissive hull mask".into());
        descriptor
            .layout
            .insert(MATERIAL_BIND_GROUP_INDEX, self.material.clone());
        descriptor.primitive.cull_mode = if key.double_sided {
            None
        } else {
            Some(Face::Back)
        };
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_compare = Some(CompareFunction::Equal);
            depth.depth_write_enabled = Some(false);
        }
        if let Some(fragment) = &mut descriptor.fragment {
            fragment.shader = self.shader.clone();
            fragment.shader_defs.push("EMISSIVE_MASK".into());
            fragment.shader_defs.push(ShaderDefVal::UInt(
                "MATERIAL_BIND_GROUP".into(),
                MATERIAL_BIND_GROUP_INDEX as u32,
            ));
            fragment.entry_point = Some("emissive_mask".into());
            fragment.targets = vec![Some(ColorTargetState {
                format: emissive_blur::MASK_FORMAT,
                blend: None,
                write_mask: ColorWrites::ALL,
            })];
        }
        Ok(descriptor)
    }
}

pub(super) struct Emissive3d {
    batch: Opaque3dBatchSetKey,
    entity: (Entity, MainEntity),
    range: Range<u32>,
    extra: PhaseItemExtraIndex,
}

impl PhaseItem for Emissive3d {
    fn entity(&self) -> Entity {
        self.entity.0
    }
    fn main_entity(&self) -> MainEntity {
        self.entity.1
    }
    fn draw_function(&self) -> DrawFunctionId {
        self.batch.draw_function
    }
    fn batch_range(&self) -> &Range<u32> {
        &self.range
    }
    fn batch_range_mut(&mut self) -> &mut Range<u32> {
        &mut self.range
    }
    fn extra_index(&self) -> PhaseItemExtraIndex {
        self.extra.clone()
    }
    fn batch_range_and_extra_index_mut(&mut self) -> (&mut Range<u32>, &mut PhaseItemExtraIndex) {
        (&mut self.range, &mut self.extra)
    }
}

impl BinnedPhaseItem for Emissive3d {
    type BatchSetKey = Opaque3dBatchSetKey;
    type BinKey = Opaque3dBinKey;

    fn new(
        batch: Self::BatchSetKey,
        _: Self::BinKey,
        entity: (Entity, MainEntity),
        range: Range<u32>,
        extra: PhaseItemExtraIndex,
    ) -> Self {
        Self {
            batch,
            entity,
            range,
            extra,
        }
    }
}

impl CachedRenderPipelinePhaseItem for Emissive3d {
    fn cached_pipeline(&self) -> CachedRenderPipelineId {
        self.batch.pipeline
    }
}

#[allow(clippy::too_many_arguments)]
fn queue(
    emissive: Res<EmissiveMaterials>,
    material_instances: Res<RenderMaterialInstances>,
    instances: Res<RenderMeshInstances>,
    meshes: Res<RenderAssets<RenderMesh>>,
    allocator: Res<MeshAllocator>,
    draw: Res<DrawFunctions<Emissive3d>>,
    cache: Res<PipelineCache>,
    pipeline: Res<MaskPipeline>,
    mut specialized: ResMut<SpecializedMeshPipelines<MaskPipeline>>,
    keys: Res<ViewKeyCache>,
    views: Query<(&ExtractedView, &RenderVisibleEntities)>,
    mut phases: ResMut<ViewBinnedRenderPhases<Emissive3d>>,
) {
    if phases.is_empty() {
        return;
    }
    let draw_function = draw.read().id::<DrawMaterial>();
    for (view, visible) in &views {
        let id = &view.retained_view_entity;
        let (Some(phase), Some(&key), Some(visible)) =
            (phases.get_mut(id), keys.get(id), visible.get::<Mesh3d>())
        else {
            continue;
        };
        let visible_meshes = visible.entities_cpu_culling.iter().copied().chain(
            visible
                .entities_gpu_culling
                .iter()
                .map(|(&main, &entity)| (entity, main)),
        );
        for (entity, main) in visible_meshes {
            let Some(material) = material_instances.instances.get(&main) else {
                continue;
            };
            let Some(&double_sided) = emissive.0.get(&material.asset_id) else {
                continue;
            };
            let Some(instance) = instances.render_mesh_queue_data(main) else {
                continue;
            };
            let mesh_id = instance.mesh_asset_id();
            let (Some(mesh), Some(slabs)) = (meshes.get(mesh_id), allocator.mesh_slabs(&mesh_id))
            else {
                continue;
            };
            let key = MaskKey {
                mesh: key | MeshPipelineKey::from_bits_retain(mesh.key_bits.bits()),
                double_sided,
            };
            let Ok(pipeline_id) = specialized.specialize(&cache, &pipeline, key, &mesh.layout)
            else {
                continue;
            };
            phase.add(
                Opaque3dBatchSetKey {
                    pipeline: pipeline_id,
                    draw_function,
                    material_bind_group_index: None,
                    slabs,
                    lightmap_slab: None,
                },
                Opaque3dBinKey {
                    asset_id: mesh_id.untyped(),
                },
                (entity, main),
                instance.current_uniform_index,
                BinnedRenderPhaseType::UnbatchableMesh,
            );
        }
    }
}

fn draw_mask(
    world: &World,
    view: ViewQuery<(
        &ExtractedView,
        &ExtractedCamera,
        &ViewDepthStencilTexture,
        &emissive_blur::EmissiveBloomTargets,
        Option<&MainPassResolutionOverride>,
    )>,
    phases: Res<ViewBinnedRenderPhases<Emissive3d>>,
    mut ctx: RenderContext,
) {
    let entity = view.entity();
    let (view, camera, depth, targets, resolution) = view.into_inner();
    let Some(phase) = phases.get(&view.retained_view_entity) else {
        return;
    };
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("emissive hull mask"),
        color_attachments: &[Some(targets.mask_attachment())],
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        ..default()
    });
    if let Some(viewport) =
        Viewport::from_viewport_and_override(camera.viewport.as_ref(), resolution)
    {
        pass.set_camera_viewport(&viewport);
    }
    if let Err(err) = phase.render(&mut pass, world, entity) {
        error!("Could not draw emissive hull mask: {err:?}");
    }
}
