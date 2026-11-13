//! The nebula march at a fraction of the view's resolution (RND-34).
//!
//! The Refit look marches the gas per pixel (`nebula.wesl`). Under a wide window that is
//! tens of milliseconds of GPU, and it falls with the pixel count. With
//! `RenderQuality::nebula_scale` above 1 the march runs into a target 1/n the size
//! (per axis) and `composite` lays it over the view with a depth-aware upsample:
//!
//! 1. `downsample`: the scene's depth prepass reduced to the target's size (the farthest
//!    depth of each n x n block: how far a ray may march);
//! 2. the march: one fullscreen triangle per field, scissored to the box's projection,
//!    farthest field first, into a cleared half-float target, premultiplied over;
//! 3. `composite`: onto the view's colour, after the opaque pass and before the
//!    transparent one (so beams and particles in front of the gas still draw over it).
//!
//! The fields' material is the ordinary `NebulaMaterial`; its bind group is made by its
//! own `AsBindGroup`, so both paths read one parameter block. While the low-resolution
//! path runs, the fields' boxes are kept out of every camera's layers (`nebula.rs`); with
//! `nebula_scale` 1 nothing here is extracted and the box path draws as before.

use bevy::asset::{load_embedded_asset, AssetServer};
use bevy::core_pipeline::core_3d::{main_opaque_pass_3d, main_transparent_pass_3d};
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::FullscreenShader;
use bevy::ecs::system::StaticSystemParam;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::material_bind_groups::FallbackBuffer;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer};
use bevy::render::render_resource::{
    AsBindGroup, BindGroup, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BlendComponent, BlendFactor, BlendOperation, BlendState,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, Extent3d, FragmentState, LoadOp, MultisampleState, Operations,
    PipelineCache, RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, ShaderStages, ShaderType,
    SpecializedRenderPipeline, SpecializedRenderPipelines, StoreOp, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    UniformBuffer,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::storage::GpuShaderBuffer;
use bevy::render::sync_world::RenderEntity;
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{ExtractedView, Msaa, ViewTarget};
use bevy::render::{Extract, ExtractSchedule, GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};

use crate::render::nebula::{NebulaField, NebulaMaterial};

/// Marks the camera whose nebulae are marched at `1 / divisor` of its size (2 or 4).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct NebulaLowRes {
    pub divisor: u32,
}

/// The shader's `LowView`: the view's clip to world, and the march target's width and
/// height, the divisor, the camera's near plane.
#[derive(Clone, Copy, ShaderType)]
struct LowView {
    world_from_clip: Mat4,
    size: Vec4,
}

/// The fields in view this frame, as the main world has them.
#[derive(Resource, Default)]
struct ExtractedFields(Vec<ExtractedField>);

struct ExtractedField {
    material: NebulaMaterial,
}

pub struct NebulaLowResPlugin;

impl Plugin for NebulaLowResPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "nebula_lowres.wesl");
        let Some(render) = app.get_sub_app_mut(RenderApp) else { return };
        render
            .init_resource::<ExtractedFields>()
            .init_gpu_resource::<SpecializedRenderPipelines<LowPipelines>>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(ExtractSchedule, extract)
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareBindGroups))
            .add_systems(
                Core3d,
                nebula_lowres.in_set(Core3dSystems::MainPass).after(main_opaque_pass_3d).before(main_transparent_pass_3d),
            );
    }
}

fn extract(
    mut commands: Commands,
    mut out: ResMut<ExtractedFields>,
    fields: Extract<Query<(&NebulaField, &InheritedVisibility)>>,
    materials: Extract<Res<Assets<NebulaMaterial>>>,
    cameras: Extract<Query<(RenderEntity, Option<&NebulaLowRes>), With<Camera3d>>>,
) {
    out.0.clear();
    for (f, vis) in &fields {
        if vis.get()
            && let Some(m) = materials.get(&f.material)
        {
            out.0.push(ExtractedField { material: m.clone() });
        }
    }
    for (e, low) in &cameras {
        let Ok(mut c) = commands.get_entity(e) else { continue };
        match low {
            Some(l) => {
                c.insert(*l);
            }
            None => {
                c.remove::<NebulaLowRes>();
            }
        }
    }
}

#[derive(Resource)]
struct LowPipelines {
    /// Group 0 of the march: the reduced depth and the `LowView`.
    march_view: BindGroupLayoutDescriptor,
    /// The fields' own layout (group 1 of the march).
    material: BindGroupLayoutDescriptor,
    /// The depth reduction's group: `LowView` and the scene's depth.
    down: [BindGroupLayoutDescriptor; 2],
    /// The composite's group: `LowView`, the gas, the reduced depth and the scene's depth.
    comp: [BindGroupLayoutDescriptor; 2],
    fullscreen: FullscreenShader,
    nebula: Handle<Shader>,
    screen: Handle<Shader>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Stage {
    March,
    Down { multisampled: bool },
    Comp { multisampled: bool, format: TextureFormat, samples: u32 },
}

const GAS_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
const DEPTH_FORMAT: TextureFormat = TextureFormat::R32Float;

fn init_pipelines(mut commands: Commands, device: Res<RenderDevice>, assets: Res<AssetServer>, fullscreen: Res<FullscreenShader>) {
    let unfilterable = TextureSampleType::Float { filterable: false };
    let frag = ShaderStages::FRAGMENT;
    let march_view = BindGroupLayoutDescriptor::new(
        "nebula low march view",
        &BindGroupLayoutEntries::with_indices(frag, ((30, texture_2d(unfilterable)), (31, uniform_buffer::<LowView>(false)))),
    );
    let down = |ms: bool| {
        BindGroupLayoutDescriptor::new(
            "nebula low depth",
            &BindGroupLayoutEntries::with_indices(frag, ((0, uniform_buffer::<LowView>(false)), (3, if ms { texture_depth_2d_multisampled() } else { texture_depth_2d() }))),
        )
    };
    let comp = |ms: bool| {
        BindGroupLayoutDescriptor::new(
            "nebula low composite",
            &BindGroupLayoutEntries::with_indices(
                frag,
                (
                    (0, uniform_buffer::<LowView>(false)),
                    (1, texture_2d(unfilterable)),
                    (2, texture_2d(unfilterable)),
                    (3, if ms { texture_depth_2d_multisampled() } else { texture_depth_2d() }),
                ),
            ),
        )
    };
    commands.insert_resource(LowPipelines {
        march_view,
        material: NebulaMaterial::bind_group_layout_descriptor(&device),
        down: [down(false), down(true)],
        comp: [comp(false), comp(true)],
        fullscreen: fullscreen.clone(),
        nebula: load_embedded_asset!(assets.as_ref(), "nebula.wesl"),
        screen: load_embedded_asset!(assets.as_ref(), "nebula_lowres.wesl"),
    });
}

/// Premultiplied alpha over: the gas's light added, what lies behind dimmed by its alpha.
const OVER: BlendState = BlendState {
    color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::OneMinusSrcAlpha, operation: BlendOperation::Add },
    alpha: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::OneMinusSrcAlpha, operation: BlendOperation::Add },
};

impl SpecializedRenderPipeline for LowPipelines {
    type Key = Stage;

    fn specialize(&self, key: Stage) -> RenderPipelineDescriptor {
        let target = |format, blend| vec![Some(ColorTargetState { format, blend, write_mask: ColorWrites::ALL })];
        let ms_def = |ms: bool| if ms { vec!["MULTISAMPLED".into()] } else { vec![] };
        match key {
            Stage::March => RenderPipelineDescriptor {
                label: Some("nebula low march".into()),
                layout: vec![self.march_view.clone(), self.material.clone()],
                vertex: self.fullscreen.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: self.nebula.clone(),
                    shader_defs: vec![ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 1)],
                    entry_point: Some("fragment_low".into()),
                    targets: target(GAS_FORMAT, Some(OVER)),
                    constants: vec![],
                }),
                ..default()
            },
            Stage::Down { multisampled } => RenderPipelineDescriptor {
                label: Some("nebula low depth".into()),
                layout: vec![self.down[multisampled as usize].clone()],
                vertex: self.fullscreen.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: self.screen.clone(),
                    shader_defs: ms_def(multisampled),
                    entry_point: Some("downsample".into()),
                    targets: target(DEPTH_FORMAT, None),
                    constants: vec![],
                }),
                ..default()
            },
            Stage::Comp { multisampled, format, samples } => RenderPipelineDescriptor {
                label: Some("nebula low composite".into()),
                layout: vec![self.comp[multisampled as usize].clone()],
                vertex: self.fullscreen.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: self.screen.clone(),
                    shader_defs: ms_def(multisampled),
                    entry_point: Some("composite".into()),
                    targets: target(format, Some(OVER)),
                    constants: vec![],
                }),
                multisample: MultisampleState { count: samples, ..default() },
                ..default()
            },
        }
    }
}

/// What one view draws this frame.
#[derive(Component)]
struct LowPlan {
    march: CachedRenderPipelineId,
    down: CachedRenderPipelineId,
    comp: CachedRenderPipelineId,
    gas: CachedTexture,
    depth: CachedTexture,
    down_group: BindGroup,
    comp_group: BindGroup,
    view_group: BindGroup,
    /// Farthest first: the field's bind group and its scissor rectangle (x, y, w, h).
    fields: Vec<(BindGroup, [u32; 4])>,
}

/// The pixel rectangle of a world-space box on a `w` x `h` target, or `None` when it is
/// off screen. A box with a corner at or behind the eye covers the whole target.
fn box_rect(clip_from_world: &Mat4, lo: Vec3, hi: Vec3, w: u32, h: u32) -> Option<[u32; 4]> {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for i in 0..8 {
        let p = Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
        let c = *clip_from_world * p.extend(1.0);
        if c.w <= 1e-3 {
            return Some([0, 0, w, h]);
        }
        let (nx, ny) = (c.x / c.w, c.y / c.w);
        x0 = x0.min(nx);
        x1 = x1.max(nx);
        y0 = y0.min(ny);
        y1 = y1.max(ny);
    }
    // Normalised device (y up) to pixels (y down), a texel of margin for the filter.
    let px = |n: f32, len: u32| (n * 0.5 + 0.5) * len as f32;
    let py = |n: f32, len: u32| (0.5 - n * 0.5) * len as f32;
    let left = (px(x0, w) - 2.0).floor().max(0.0);
    let right = (px(x1, w) + 2.0).ceil().min(w as f32);
    let top = (py(y1, h) - 2.0).floor().max(0.0);
    let bottom = (py(y0, h) + 2.0).ceil().min(h as f32);
    (right > left && bottom > top).then(|| [left as u32, top as u32, (right - left) as u32, (bottom - top) as u32])
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn prepare(
    mut commands: Commands,
    fields: Res<ExtractedFields>,
    views: Query<(Entity, &NebulaLowRes, &ViewTarget, &ExtractedView, &ViewPrepassTextures, &Msaa), With<ExtractedCamera>>,
    pipelines: Option<Res<LowPipelines>>,
    mut specialized: ResMut<SpecializedRenderPipelines<LowPipelines>>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut textures: ResMut<TextureCache>,
    fallback: Res<FallbackBuffer>,
    shader_buffers: Res<RenderAssets<GpuShaderBuffer>>,
    mut param: StaticSystemParam<<NebulaMaterial as AsBindGroup>::Param>,
) {
    let Some(pipelines) = pipelines else { return };
    for (entity, low, target, view, prepass, msaa) in &views {
        let plan = (|| {
            let depth_view = prepass.depth_only_view()?;
            let full = target.main_texture().size();
            let n = low.divisor.max(2);
            let (lw, lh) = (full.width.div_ceil(n), full.height.div_ceil(n));
            let clip_from_world = view.clip_from_world.unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse());
            let eye = view.world_from_view.translation();
            // The fields the view sees, farthest first.
            let mut seen: Vec<(f32, &ExtractedField, [u32; 4])> = fields
                .0
                .iter()
                .filter_map(|f| {
                    let p = &f.material.params;
                    let rect = box_rect(&clip_from_world, p.box_lo.truncate(), p.box_hi.truncate(), lw, lh)?;
                    let centre = (p.box_lo.truncate() + p.box_hi.truncate()) * 0.5;
                    Some((centre.distance_squared(eye), f, rect))
                })
                .collect();
            if seen.is_empty() {
                return None;
            }
            seen.sort_by(|a, b| b.0.total_cmp(&a.0));
            let near = view.clip_from_view.w_axis.z;
            let lv = LowView { world_from_clip: clip_from_world.inverse(), size: Vec4::new(lw as f32, lh as f32, n as f32, near) };
            let mut ub = UniformBuffer::from(lv);
            ub.write_buffer(&device, &queue);
            let samples = msaa.samples();
            let ms = samples > 1;
            let mut make = |label: &'static str, format| {
                textures.get(
                    &device,
                    TextureDescriptor {
                        label: Some(label),
                        size: Extent3d { width: lw, height: lh, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: TextureDimension::D2,
                        format,
                        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                )
            };
            let gas = make("nebula low gas", GAS_FORMAT);
            let depth = make("nebula low depth", DEPTH_FORMAT);
            let group = |layout: &BindGroupLayoutDescriptor, entries: &[(u32, bevy::render::render_resource::BindingResource)]| {
                let e: Vec<_> = entries.iter().map(|(b, r)| bevy::render::render_resource::BindGroupEntry { binding: *b, resource: r.clone() }).collect();
                device.create_bind_group("nebula low", &cache.get_bind_group_layout(layout), &e)
            };
            let uni = ub.binding()?;
            let down_group = group(&pipelines.down[ms as usize], &[(0, uni.clone()), (3, bevy::render::render_resource::BindingResource::TextureView(depth_view))]);
            let comp_group = group(
                &pipelines.comp[ms as usize],
                &[
                    (0, uni.clone()),
                    (1, bevy::render::render_resource::BindingResource::TextureView(&gas.default_view)),
                    (2, bevy::render::render_resource::BindingResource::TextureView(&depth.default_view)),
                    (3, bevy::render::render_resource::BindingResource::TextureView(depth_view)),
                ],
            );
            let view_group = group(&pipelines.march_view, &[(30, bevy::render::render_resource::BindingResource::TextureView(&depth.default_view)), (31, uni)]);
            let layout = &pipelines.material;
            let mut groups = Vec::new();
            for (_, f, rect) in seen {
                // A field whose image is not ready yet is left out this frame.
                if let Ok(prepared) = f.material.as_bind_group(layout, &device, &cache, &fallback, &shader_buffers, &mut param) {
                    groups.push((prepared.bind_group, rect));
                }
            }
            if groups.is_empty() {
                return None;
            }
            let format = target.main_texture_format();
            let march = specialized.specialize(&cache, &pipelines, Stage::March);
            let down = specialized.specialize(&cache, &pipelines, Stage::Down { multisampled: ms });
            let comp = specialized.specialize(&cache, &pipelines, Stage::Comp { multisampled: ms, format, samples });
            Some(LowPlan { march, down, comp, gas, depth, down_group, comp_group, view_group, fields: groups })
        })();
        match plan {
            Some(p) => {
                commands.entity(entity).insert(p);
            }
            None => {
                commands.entity(entity).remove::<LowPlan>();
            }
        }
    }
}

fn nebula_lowres(view: ViewQuery<(&ViewTarget, &LowPlan)>, cache: Res<PipelineCache>, mut ctx: RenderContext) {
    let (target, plan) = view.into_inner();
    let (Some(march), Some(down), Some(comp)) =
        (cache.get_render_pipeline(plan.march), cache.get_render_pipeline(plan.down), cache.get_render_pipeline(plan.comp))
    else {
        return;
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let clear = || Operations { load: LoadOp::Clear(default()), store: StoreOp::Store };
    {
        let desc = RenderPassDescriptor {
            label: Some("nebula_depth_low"),
            color_attachments: &[Some(RenderPassColorAttachment { view: &plan.depth.default_view, depth_slice: None, resolve_target: None, ops: clear() })],
            ..default()
        };
        let mut pass = ctx.command_encoder().begin_render_pass(&desc);
        let span = diagnostics.pass_span(&mut pass, "nebula_depth_low");
        pass.set_pipeline(down);
        pass.set_bind_group(0, &plan.down_group, &[]);
        pass.draw(0..3, 0..1);
        span.end(&mut pass);
    }
    {
        let desc = RenderPassDescriptor {
            label: Some("nebula_march_low"),
            color_attachments: &[Some(RenderPassColorAttachment { view: &plan.gas.default_view, depth_slice: None, resolve_target: None, ops: clear() })],
            ..default()
        };
        let mut pass = ctx.command_encoder().begin_render_pass(&desc);
        let span = diagnostics.pass_span(&mut pass, "nebula_march_low");
        pass.set_pipeline(march);
        pass.set_bind_group(0, &plan.view_group, &[]);
        for (group, [x, y, w, h]) in &plan.fields {
            pass.set_bind_group(1, group, &[]);
            pass.set_scissor_rect(*x, *y, *w, *h);
            pass.draw(0..3, 0..1);
        }
        span.end(&mut pass);
    }
    {
        let desc = RenderPassDescriptor {
            label: Some("nebula_composite"),
            color_attachments: &[Some(target.get_color_attachment())],
            ..default()
        };
        let mut pass = ctx.command_encoder().begin_render_pass(&desc);
        let span = diagnostics.pass_span(&mut pass, "nebula_composite");
        pass.set_pipeline(comp);
        pass.set_bind_group(0, &plan.comp_group, &[]);
        pass.draw(0..3, 0..1);
        span.end(&mut pass);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> Mat4 {
        // A camera at the origin looking down -Z, reverse-Z infinite perspective, 1:1.
        Mat4::perspective_infinite_reverse_rh(std::f32::consts::FRAC_PI_2, 1.0, 1.0)
    }

    #[test]
    fn a_box_ahead_covers_its_projection_and_one_behind_nothing() {
        let m = view();
        let r = box_rect(&m, Vec3::new(-10.0, -10.0, -110.0), Vec3::new(10.0, 10.0, -90.0), 100, 100).unwrap();
        // About 10/90 of the half width either side of the centre, plus the margin.
        assert!(r[0] >= 30 && r[0] <= 45 && r[2] >= 10 && r[2] <= 40, "{r:?}");
        assert_eq!(box_rect(&m, Vec3::new(-10.0, -10.0, 90.0), Vec3::new(10.0, 10.0, 110.0), 100, 100), Some([0, 0, 100, 100]));
        // Far off to the side, off screen.
        assert_eq!(box_rect(&m, Vec3::new(1000.0, -10.0, -110.0), Vec3::new(1010.0, 10.0, -90.0), 100, 100), None);
    }

    #[test]
    fn an_eye_inside_a_box_covers_the_whole_target() {
        let m = view();
        assert_eq!(box_rect(&m, Vec3::splat(-50.0), Vec3::splat(50.0), 64, 32), Some([0, 0, 64, 32]));
    }
}
