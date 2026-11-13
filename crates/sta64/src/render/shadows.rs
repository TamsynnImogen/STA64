//! Ship shadows from the Key light (RND-15): the nebulae's gas and the hulls take them.
//!
//! After Armada II Refit's `render3d/mods/hulls/src/shadow.rs` (MIT, Copyright (c) 2026
//! Cedric Haase): every hull that can throw a shadow into the view is drawn into a depth
//! map from the Key, fitted round them (width in steps of 2^(1/4), centre on whole
//! texels), with this frame's poses. The nebula march (`nebula.wesl`) and the hull look
//! (`hulls.wesl`) read the map.
//!
//! - The map is a second, orthographic camera ([`ShadowCamera`]) looking along the Key
//!   at *proxies*: one child entity per hull mesh ([`HullPart`]) on [`SHADOW_LAYER`]
//!   only, drawn with [`ShadowCasterMaterial`], so the real hulls and their materials
//!   are untouched and the main camera never draws a proxy. A proxy follows its mesh's
//!   visibility and transform.
//! - The target is a half-float image (Bevy blits a camera's half-float main texture to
//!   it), so the depth is split over two channels (`shadows.wesl`); [`unpack`] is the
//!   Rust side of it.
//! - The fit, the camera and every consumer's parameters are written in one system, after
//!   transform propagation, so the matrix and the map are of the same frame.
//! - Off under the Stock profile (stock draws no ship shadows in gas; RND-22), and while the
//!   scene has no Key light or no hull in view: the camera is inactive and the consumers' `on` is 0.
//!
//! Not ported: the cut-out hulls cast solid (Refit's casters read the texture's alpha),
//! planets cast no shadow into the map, and hulls that are cloaked still cast.

use bevy::asset::{embedded_asset, AssetPath, RenderAssetUsages};
use bevy::camera::primitives::{Frustum, Sphere};
use bevy::camera::visibility::{RenderLayers, VisibilitySystems};
use bevy::camera::{Hdr, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::math::Vec3A;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Face, RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureFormat};
use bevy::shader::ShaderRef;

use crate::map::{BackgroundCamera, MainCamera};
use crate::render::hulls::{HullMaterial, HullPart, HullView};
use crate::render::profile::RenderProfile;

/// The map's size in texels (Refit's `ShadowSize` default).
pub const SIZE: u32 = 2048;
/// The render layer the proxies and the shadow camera share.
pub const SHADOW_LAYER: usize = 30;
/// Scale of the depth's rounding residual in the map's green channel (`shadows.wesl`).
#[cfg(test)]
pub const DEPTH_RESIDUAL_SCALE: f32 = 2048.0;
/// How soft a shadow's edge gets per unit of depth behind the caster, over the map's width
/// (Refit's `ShadowSoftness` default: the light is a disc, not a point).
pub const SOFTNESS: f32 = 0.02;

/// The depth the map holds, from one of its texels.
#[cfg(test)]
pub fn unpack(r: f32, g: f32) -> f32 {
    r + g / DEPTH_RESIDUAL_SCALE
}

/// The Key's shadow map and how to read it, for every consumer.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct KeyShadow {
    pub image: Handle<Image>,
    /// World to map: u, v (0..1 across the image, v down) and depth (0..1 along the Key).
    pub rows: [Vec4; 3],
    /// x 1 with a map, 0 without; y the depth bias; z the penumbra (u, v per unit of
    /// depth behind a caster); w one texel (u, v).
    pub params: Vec4,
}

impl KeyShadow {
    fn off(image: Handle<Image>) -> KeyShadow {
        KeyShadow { image, rows: [Vec4::ZERO; 3], params: Vec4::new(0.0, 0.0, 0.0, 1.0 / SIZE as f32) }
    }
}

/// The camera that draws the map.
#[derive(Component)]
pub struct ShadowCamera;

/// A hull mesh's shadow proxy (a child of the mesh).
#[derive(Component)]
struct ShadowProxy;

/// On a hull mesh that has its proxy.
#[derive(Component)]
struct Proxied(Entity);

/// The fit, after transform propagation.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShadowSet;

/// Draws a hull's depth along the Key (`shadows.wesl`).
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
pub struct ShadowCasterMaterial {}

impl Material for ShadowCasterMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/render/shadows.wesl").into_owned())
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    /// Every face, as Refit's map does: a hull's back faces are as good a caster.
    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None::<Face>;
        Ok(())
    }
}

#[derive(Resource)]
struct CasterMaterial(Handle<ShadowCasterMaterial>);

/// Where the map's camera goes and what it reads, from the casters (world spheres) and the
/// direction towards the Key.
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    pub rows: [Vec4; 3],
    pub right: Vec3,
    pub up: Vec3,
    /// The direction the Key's light travels.
    pub forward: Vec3,
    /// The camera's position.
    pub eye: Vec3,
    /// The map's width in world units.
    pub width: f32,
    /// The depth the map covers along the light, in world units.
    pub range: f32,
    /// The depth bias, in map depth.
    pub bias: f32,
    pub penumbra: f32,
}

/// Fits a square map of `size` texels about `casters` (centre, radius), looking along the
/// light. The width steps by 2^(1/4) and the centre sits on whole texels, so the map does
/// not shimmer as the casters move.
pub fn fit(casters: &[(Vec3, f32)], to_key: Vec3, size: u32) -> Option<Fit> {
    if casters.is_empty() {
        return None;
    }
    let w = (-to_key).try_normalize()?;
    let a = if w.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let right = w.cross(a).normalize();
    let up = right.cross(w);
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for &(c, r) in casters {
        let p = Vec3::new(c.dot(right), c.dot(up), c.dot(w));
        lo = lo.min(p - Vec3::splat(r));
        hi = hi.max(p + Vec3::splat(r));
    }
    let ext = (hi.x - lo.x).max(hi.y - lo.y).max(1.0) * 1.02;
    let width = ((ext.log2() * 4.0).ceil() / 4.0).exp2();
    let texel = width / size as f32;
    let cu = ((lo.x + hi.x) * 0.5 / texel).floor() * texel;
    let cv = ((lo.y + hi.y) * 0.5 / texel).floor() * texel;
    // The depth range on a coarse grid (an eighth of the width), so it changes as seldom as
    // the width does: every consumer's parameters change with it.
    let q = width / 8.0;
    let (z0, z1) = (((lo.z - 1.0) / q).floor() * q, ((hi.z + 1.0) / q).ceil() * q);
    let range = (z1 - z0).max(q);
    let rows = [
        (right / width).extend(0.5 - cu / width),
        (-up / width).extend(0.5 + cv / width),
        (w / range).extend(-z0 / range),
    ];
    Some(Fit {
        rows,
        right,
        up,
        forward: w,
        eye: w * z0 + right * cu + up * cv,
        width,
        range,
        // Refit doubles the hulls' bias for the gas, which has no faces to fight it.
        bias: 0.5 * texel / range,
        penumbra: SOFTNESS * range / width,
    })
}

impl Fit {
    /// A world point in the map: u, v, depth.
    #[cfg(test)]
    pub fn project(&self, p: Vec3) -> Vec3 {
        let q = p.extend(1.0);
        Vec3::new(q.dot(self.rows[0]), q.dot(self.rows[1]), q.dot(self.rows[2]))
    }
}

pub struct ShadowPlugin;

impl Plugin for ShadowPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shadows.wesl");
        app.add_plugins(MaterialPlugin::<ShadowCasterMaterial>::default())
            .init_resource::<crate::render::quality::RenderQuality>()
            .add_systems(Startup, setup)
            .add_systems(Update, add_proxies)
            .add_systems(
                PostUpdate,
                update
                    .in_set(ShadowSet)
                    .after(TransformSystems::Propagate)
                    .before(bevy::camera::CameraUpdateSystems)
                    .before(VisibilitySystems::UpdateFrusta),
            );
    }
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut materials: ResMut<Assets<ShadowCasterMaterial>>) {
    let mut img = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba16Float, None);
    img.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let image = images.add(img);
    commands.insert_resource(CasterMaterial(materials.add(ShadowCasterMaterial {})));
    commands.insert_resource(KeyShadow::off(image.clone()));
    commands.spawn((
        Name::new("shadow camera"),
        ShadowCamera,
        // Not the scene's camera: the queries for it (`MainCamera`, post fx) leave it out.
        BackgroundCamera,
        Camera3d::default(),
        Camera {
            order: -100,
            is_active: false,
            // Depth 1, no hull.
            clear_color: ClearColorConfig::Custom(Color::linear_rgba(1.0, 0.0, 0.0, 1.0)),
            ..default()
        },
        RenderTarget::from(image),
        Msaa::Off,
        // A half-float main texture, written to the half-float map as it is.
        Hdr,
        Tonemapping::None,
        DebandDither::Disabled,
        Projection::Orthographic(OrthographicProjection {
            near: 0.0,
            far: 1000.0,
            scaling_mode: ScalingMode::Fixed { width: 1000.0, height: 1000.0 },
            ..OrthographicProjection::default_3d()
        }),
        RenderLayers::layer(SHADOW_LAYER),
        Transform::default(),
    ));
}

/// Gives each hull mesh its proxy, and keeps the proxy's mesh the mesh's. A proxy whose mesh
/// was despawned the frame it was made has no parent (the hierarchy drops it): it goes.
#[allow(clippy::type_complexity)]
fn add_proxies(
    mut commands: Commands,
    caster: Res<CasterMaterial>,
    fresh: Query<(Entity, &Mesh3d), (With<HullPart>, Without<Proxied>, Without<ShadowProxy>)>,
    changed: Query<(&Mesh3d, &Proxied), Changed<Mesh3d>>,
    mut proxies: Query<&mut Mesh3d, (With<ShadowProxy>, Without<Proxied>)>,
    orphans: Query<Entity, (With<ShadowProxy>, Without<ChildOf>)>,
) {
    for e in &orphans {
        commands.entity(e).try_despawn();
    }
    for (e, mesh) in &fresh {
        let proxy = commands
            .spawn((ShadowProxy, Mesh3d(mesh.0.clone()), MeshMaterial3d(caster.0.clone()), RenderLayers::layer(SHADOW_LAYER), Transform::IDENTITY, ChildOf(e)))
            .id();
        commands.entity(e).try_insert(Proxied(proxy));
    }
    for (mesh, p) in &changed {
        if let Ok(mut m) = proxies.get_mut(p.0)
            && m.0 != mesh.0
        {
            m.0 = mesh.0.clone();
        }
    }
}

/// The Key: the brightest directional light, the direction towards it.
fn key_direction(lights: impl Iterator<Item = (Vec3, f32)>) -> Option<Vec3> {
    lights.max_by(|a, b| a.1.total_cmp(&b.1)).map(|(d, _)| d)
}

/// The world sphere of a proxy: its box's centre and the largest scale.
fn sphere(gt: &GlobalTransform, aabb: &bevy::camera::primitives::Aabb) -> (Vec3, f32) {
    let m = gt.affine().matrix3;
    let scale = m.x_axis.length().max(m.y_axis.length()).max(m.z_axis.length());
    (gt.transform_point(Vec3::from(aabb.center)), aabb.half_extents.length() * scale)
}

/// Whether a caster can shade something in view: its sphere, or the sphere stretched back
/// towards the Key, reaches the frustum (a hull just outside can still shade one inside).
fn can_shade(frustum: &Frustum, c: Vec3, r: f32, to_key: Vec3) -> bool {
    let hit = |c: Vec3, r: f32| frustum.intersects_sphere(&Sphere { center: Vec3A::from(c), radius: r }, false);
    hit(c, r * 2.0) || hit(c + to_key * (r * 4.0), r * 3.0)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update(
    profile: Option<Res<RenderProfile>>,
    quality: Res<crate::render::quality::RenderQuality>,
    main: Query<&Frustum, MainCamera>,
    lights: Query<(&GlobalTransform, &DirectionalLight), Without<ShadowCamera>>,
    proxies: Query<(&GlobalTransform, &bevy::camera::primitives::Aabb, &InheritedVisibility), (With<ShadowProxy>, Without<ShadowCamera>)>,
    mut camera: Query<(&mut Camera, &mut Transform, &mut GlobalTransform, &mut Projection), With<ShadowCamera>>,
    mut shadow: ResMut<KeyShadow>,
    mut hull_view: Option<ResMut<HullView>>,
    mut hulls: Option<ResMut<Assets<HullMaterial>>>,
) {
    let Ok((mut cam, mut tf, mut gt, mut proj)) = camera.single_mut() else { return };
    // Stock parity: only the Refit profile draws ship shadows in gas and on hulls (RND-22).
    let drawing = quality.shadows && profile.is_some_and(|p| *p == RenderProfile::Refit);
    let to_key = key_direction(lights.iter().map(|(t, l)| (t.back().as_vec3(), l.illuminance)));
    let fitted = (|| {
        if !drawing {
            return None;
        }
        let to_key = to_key?;
        let frustum = main.single().ok()?;
        let casters: Vec<(Vec3, f32)> = proxies
            .iter()
            .filter(|(_, _, vis)| vis.get())
            .map(|(g, a, _)| sphere(g, a))
            .filter(|&(c, r)| can_shade(frustum, c, r, to_key))
            .collect();
        fit(&casters, to_key, SIZE)
    })();
    let next = match &fitted {
        Some(f) => KeyShadow { image: shadow.image.clone(), rows: f.rows, params: Vec4::new(1.0, f.bias * 2.0, f.penumbra, 1.0 / SIZE as f32) },
        None => KeyShadow::off(shadow.image.clone()),
    };
    if cam.is_active != fitted.is_some() {
        cam.is_active = fitted.is_some();
    }
    if let Some(f) = &fitted {
        let t = Transform {
            translation: f.eye,
            rotation: Quat::from_mat3(&Mat3::from_cols(f.right, f.up, -f.forward)),
            scale: Vec3::ONE,
        };
        if *tf != t {
            *tf = t;
            // This frame's, not next frame's: the frusta are made from it.
            *gt = GlobalTransform::from(t);
        }
        let ortho = OrthographicProjection {
            near: 0.0,
            far: f.range,
            scaling_mode: ScalingMode::Fixed { width: f.width, height: f.width },
            ..OrthographicProjection::default_3d()
        };
        let p = Projection::Orthographic(ortho);
        if !matches!((&*proj, &p), (Projection::Orthographic(a), Projection::Orthographic(b)) if a.far == b.far && matches!(a.scaling_mode, ScalingMode::Fixed { width, .. } if width == f.width) && b.near == a.near) {
            *proj = p;
        }
    }
    if *shadow == next {
        return;
    }
    *shadow = next;
    // The hulls take the same map, in the same frame.
    if let (Some(view), Some(hulls)) = (hull_view.as_mut(), hulls.as_mut()) {
        view.set_shadow(&shadow);
        for (_, m) in hulls.iter_mut() {
            view.write(&mut m.params);
            m.shadow = Some(shadow.image.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residual_split_round_trips_to_better_than_a_half_float() {
        for d in [0.0f32, 0.1234567, 0.5, 0.987654] {
            // What the shader writes: the depth in half precision, and what it left over.
            let r = half_round(d);
            let g = (d - r) * DEPTH_RESIDUAL_SCALE;
            // The residual is itself stored in half precision.
            let back = unpack(r, half_round(g));
            assert!((back - d).abs() < 1e-6, "{d} -> {back}");
            assert!((r - d).abs() > 0.0 || d == 0.5 || d == 0.0 || d == 1.0);
        }
    }

    /// Round to the nearest half float (11 significant bits).
    fn half_round(x: f32) -> f32 {
        if x == 0.0 {
            return 0.0;
        }
        let e = x.abs().log2().floor();
        let q = (e - 10.0).exp2();
        (x / q).round() * q
    }

    #[test]
    fn the_fit_puts_every_caster_inside_the_map_and_snaps_the_centre() {
        let casters = [(Vec3::new(100.0, 20.0, -50.0), 30.0), (Vec3::new(-200.0, 0.0, 120.0), 25.0)];
        let to_key = Vec3::new(0.3, 0.8, 0.5).normalize();
        let f = fit(&casters, to_key, SIZE).unwrap();
        for (c, r) in casters {
            for d in [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X, -Vec3::Y, -Vec3::Z] {
                let p = f.project(c + d * r * 0.99);
                assert!((0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y), "{p:?} off the map");
                assert!((0.0..=1.0).contains(&p.z), "{p:?} outside the depth range");
            }
        }
        // A point nearer the Key has the smaller depth.
        let near = f.project(casters[0].0 + to_key * 10.0).z;
        let far = f.project(casters[0].0 - to_key * 10.0).z;
        assert!(near < far);
        // The width is a power of 2^(1/4).
        let steps = f.width.log2() * 4.0;
        assert!((steps - steps.round()).abs() < 1e-3, "width {}", f.width);
        // The camera's own axes agree with the rows: its eye is on the depth-0 plane.
        assert!(f.project(f.eye).z.abs() < 1e-4);
        assert!((f.project(f.eye).x - 0.5).abs() < 1e-3 && (f.project(f.eye).y - 0.5).abs() < 1e-3);
        // Handedness: right x up is back (Bevy's camera looks down -Z).
        assert!((f.right.cross(f.up) + f.forward).length() < 1e-4);
    }

    #[test]
    fn a_small_move_inside_a_step_leaves_the_width_and_snaps_the_centre() {
        let to_key = Vec3::Y;
        let a = fit(&[(Vec3::new(10.0, 0.0, 10.0), 20.0)], to_key, SIZE).unwrap();
        let b = fit(&[(Vec3::new(10.01, 0.0, 10.0), 20.0)], to_key, SIZE).unwrap();
        assert_eq!(a.width, b.width);
        assert_eq!(fit(&[], to_key, SIZE), None);
    }

    #[test]
    fn the_key_is_the_brightest_light() {
        let k = key_direction([(Vec3::X, 1000.0), (Vec3::Y, 6000.0), (Vec3::Z, 10.0)].into_iter());
        assert_eq!(k, Some(Vec3::Y));
        assert_eq!(key_direction(std::iter::empty()), None);
    }
}
