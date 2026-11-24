//! Screen-space halo from the emissive-only hull mask.

use bevy::asset::{AssetServer, load_embedded_asset};
use bevy::camera::{MainPassResolutionOverride, Viewport};
use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::render_phase::ViewBinnedRenderPhases;
use bevy::render::render_resource::binding_types::{texture_2d, uniform_buffer};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntry, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BindingResource,
    BlendComponent, BlendFactor, BlendOperation, BlendState, CachedRenderPipelineId,
    ColorTargetState, ColorWrites, Extent3d, FragmentState, LoadOp, MultisampleState, Operations,
    PipelineCache, RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor,
    ShaderStages, ShaderType, SpecializedRenderPipeline, SpecializedRenderPipelines, StoreOp,
    TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    UniformBuffer,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;

use super::emissive_bloom::{Emissive3d, EmissiveBloomStrength};

pub(super) const MASK_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
// Keep the faint tail of the bloom filter smooth after lifting small lights.
const BLUR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

#[derive(Clone, Copy, ShaderType)]
struct BlurParams {
    strength: Vec4,
}

#[derive(Resource)]
struct BlurPipelines {
    layout: BindGroupLayoutDescriptor,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Stage {
    Down,
    Horizontal,
    Vertical,
    Composite { format: TextureFormat, samples: u32 },
}

const ADD_RGB: BlendState = BlendState {
    color: BlendComponent {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::One,
        operation: BlendOperation::Add,
    },
    alpha: BlendComponent {
        src_factor: BlendFactor::Zero,
        dst_factor: BlendFactor::One,
        operation: BlendOperation::Add,
    },
};

impl SpecializedRenderPipeline for BlurPipelines {
    type Key = Stage;

    fn specialize(&self, stage: Stage) -> RenderPipelineDescriptor {
        let (label, entry, format, blend, samples) = match stage {
            Stage::Down => ("emissive downsample", "downsample", BLUR_FORMAT, None, 1),
            Stage::Horizontal => (
                "emissive blur horizontal",
                "blur_horizontal",
                BLUR_FORMAT,
                None,
                1,
            ),
            Stage::Vertical => (
                "emissive blur vertical",
                "blur_vertical",
                BLUR_FORMAT,
                None,
                1,
            ),
            Stage::Composite { format, samples } => (
                "emissive halo composite",
                "composite",
                format,
                Some(ADD_RGB),
                samples,
            ),
        };
        RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![self.layout.clone()],
            vertex: self.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend,
                    write_mask: if blend.is_some() {
                        ColorWrites::RED | ColorWrites::GREEN | ColorWrites::BLUE
                    } else {
                        ColorWrites::ALL
                    },
                })],
                ..default()
            }),
            multisample: MultisampleState {
                count: samples,
                ..default()
            },
            ..default()
        }
    }
}

pub(super) fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "emissive_blur.wesl");
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<SpecializedRenderPipelines<BlurPipelines>>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(
                Render,
                prepare_targets.in_set(RenderSystems::PrepareBindGroups),
            );
    }
}

fn init_pipelines(
    mut commands: Commands,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
) {
    commands.insert_resource(BlurPipelines {
        layout: BindGroupLayoutDescriptor::new(
            "emissive blur source and strength",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    uniform_buffer::<BlurParams>(false),
                ),
            ),
        ),
        fullscreen: fullscreen.clone(),
        shader: load_embedded_asset!(assets.as_ref(), "emissive_blur.wesl"),
    });
}

#[derive(Component)]
pub(super) struct EmissiveBloomTargets {
    mask: CachedTexture,
    mask_msaa: Option<CachedTexture>,
    small: CachedTexture,
    horizontal: CachedTexture,
    vertical: CachedTexture,
    down_group: BindGroup,
    horizontal_group: BindGroup,
    vertical_group: BindGroup,
    composite_group: BindGroup,
    down: CachedRenderPipelineId,
    horizontal_pipeline: CachedRenderPipelineId,
    vertical_pipeline: CachedRenderPipelineId,
    composite_pipeline: CachedRenderPipelineId,
    viewport: [u32; 4],
}

impl EmissiveBloomTargets {
    pub(super) fn mask_attachment(&self) -> RenderPassColorAttachment<'_> {
        RenderPassColorAttachment {
            view: self
                .mask_msaa
                .as_ref()
                .map_or(&self.mask.default_view, |t| &t.default_view),
            depth_slice: None,
            resolve_target: self.mask_msaa.as_ref().map(|_| &*self.mask.default_view),
            ops: Operations {
                load: LoadOp::Clear(default()),
                store: StoreOp::Store,
            },
        }
    }
}

type BloomViews<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static ExtractedView,
        &'static ExtractedCamera,
        &'static ViewDepthStencilTexture,
        &'static ViewTarget,
        &'static Msaa,
        Option<&'static EmissiveBloomStrength>,
        Option<&'static MainPassResolutionOverride>,
    ),
>;

#[allow(clippy::too_many_arguments)]
fn prepare_targets(
    mut commands: Commands,
    views: BloomViews,
    phases: Res<ViewBinnedRenderPhases<Emissive3d>>,
    pipelines: Option<Res<BlurPipelines>>,
    mut specialized: ResMut<SpecializedRenderPipelines<BlurPipelines>>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut textures: ResMut<TextureCache>,
) {
    for (entity, view, camera, depth, target, msaa, strength, resolution) in &views {
        let active = strength.is_some_and(|s| s.0 > 0.0)
            && phases
                .get(&view.retained_view_entity)
                .is_some_and(|p| !p.is_empty());
        if !active {
            commands.entity(entity).remove::<EmissiveBloomTargets>();
            continue;
        }
        let Some(pipelines) = pipelines.as_ref() else {
            commands.entity(entity).remove::<EmissiveBloomTargets>();
            continue;
        };
        let size = depth.texture().size();
        let (width, height) = (size.width, size.height);
        if width == 0 || height == 0 {
            commands.entity(entity).remove::<EmissiveBloomTargets>();
            continue;
        }
        // A wider screen footprint at lower resolution keeps the halo legible
        // against a lit sky and reduces the cost of both blur passes.
        let reduced = Extent3d {
            width: width.div_ceil(4),
            height: height.div_ceil(4),
            depth_or_array_layers: 1,
        };
        let full = Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let samples = msaa.samples();
        let mut make = |label: &'static str, size: Extent3d, format, count: u32, sampled: bool| {
            textures.get(
                &device,
                TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: count,
                    dimension: TextureDimension::D2,
                    format,
                    usage: TextureUsages::RENDER_ATTACHMENT
                        | if sampled {
                            TextureUsages::TEXTURE_BINDING
                        } else {
                            TextureUsages::empty()
                        },
                    view_formats: &[],
                },
            )
        };
        let mask = make("emissive full-size mask", full, MASK_FORMAT, 1, true);
        let mask_msaa = (samples > 1).then(|| {
            make(
                "emissive multisample mask",
                full,
                MASK_FORMAT,
                samples,
                false,
            )
        });
        let small = make(
            "emissive quarter-size source",
            reduced,
            BLUR_FORMAT,
            1,
            true,
        );
        let horizontal = make("emissive horizontal blur", reduced, BLUR_FORMAT, 1, true);
        let vertical = make("emissive vertical blur", reduced, BLUR_FORMAT, 1, true);
        let mut params = UniformBuffer::from(BlurParams {
            strength: Vec4::new(strength.unwrap().0.clamp(0.0, 1.0), 0.0, 0.0, 0.0),
        });
        params.write_buffer(&device, &queue);
        let Some(uniform) = params.binding() else {
            commands.entity(entity).remove::<EmissiveBloomTargets>();
            continue;
        };
        let layout = cache.get_bind_group_layout(&pipelines.layout);
        let group = |label, source: &CachedTexture| {
            device.create_bind_group(
                label,
                &layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&source.default_view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: uniform.clone(),
                    },
                ],
            )
        };
        let viewport = Viewport::from_viewport_and_override(camera.viewport.as_ref(), resolution)
            .map_or([0, 0, width, height], |v| {
                let bounds = UVec2::new(width, height);
                let pos = v.physical_position.min(bounds);
                let size = v.physical_size.min(bounds - pos);
                [pos.x, pos.y, size.x, size.y]
            });
        commands.entity(entity).insert(EmissiveBloomTargets {
            down_group: group("emissive mask input", &mask),
            horizontal_group: group("emissive quarter-size input", &small),
            vertical_group: group("emissive horizontal input", &horizontal),
            composite_group: group("emissive blurred input", &vertical),
            down: specialized.specialize(&cache, pipelines, Stage::Down),
            horizontal_pipeline: specialized.specialize(&cache, pipelines, Stage::Horizontal),
            vertical_pipeline: specialized.specialize(&cache, pipelines, Stage::Vertical),
            composite_pipeline: specialized.specialize(
                &cache,
                pipelines,
                Stage::Composite {
                    format: target.main_texture_format(),
                    samples,
                },
            ),
            mask,
            mask_msaa,
            small,
            horizontal,
            vertical,
            viewport,
        });
    }
}

pub(super) fn composite(
    view: ViewQuery<(&ViewTarget, &EmissiveBloomTargets)>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, plan) = view.into_inner();
    let (Some(down), Some(horizontal), Some(vertical), Some(composite)) = (
        cache.get_render_pipeline(plan.down),
        cache.get_render_pipeline(plan.horizontal_pipeline),
        cache.get_render_pipeline(plan.vertical_pipeline),
        cache.get_render_pipeline(plan.composite_pipeline),
    ) else {
        return;
    };
    let passes = [
        ("emissive downsample", &plan.small, down, &plan.down_group),
        (
            "emissive horizontal blur",
            &plan.horizontal,
            horizontal,
            &plan.horizontal_group,
        ),
        (
            "emissive vertical blur",
            &plan.vertical,
            vertical,
            &plan.vertical_group,
        ),
    ];
    for (label, output, pipeline, group) in passes {
        let attachment = RenderPassColorAttachment {
            view: &output.default_view,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Clear(default()),
                store: StoreOp::Store,
            },
        };
        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(attachment)],
                ..default()
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("emissive halo composite"),
            color_attachments: &[Some(target.get_color_attachment())],
            ..default()
        });
    pass.set_pipeline(composite);
    pass.set_bind_group(0, &plan.composite_group, &[]);
    let [x, y, w, h] = plan.viewport;
    if w > 0 && h > 0 {
        pass.set_scissor_rect(x, y, w, h);
        pass.draw(0..3, 0..1);
    }
}
