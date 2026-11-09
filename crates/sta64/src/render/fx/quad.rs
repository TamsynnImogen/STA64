//! The quad effects' material (`quad.wgsl`, ported from Refit's ordnance and
//! welding shaders) and the helpers that lay a quad out: a billboard facing the
//! camera, or a ribbon along a line facing it as far as it can, as Refit's `fx.rs`
//! builds them. Every effect module owns its [`Look`] and writes its brightness and
//! clock into the material each frame; nothing here reads Bevy's `Time`.

use bevy::asset::{AssetPath, embedded_asset};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

/// The shader's kinds (`quad.wgsl`).
pub const K_FLARE: f32 = 0.0;
pub const K_BEAM: f32 = 2.0;
pub const K_BOLT: f32 = 3.0;
pub const K_CONE: f32 = 6.0;
pub const K_SPARK: f32 = 7.0;
pub const K_HOT: f32 = 8.0;
pub const K_WELD_BEAM: f32 = 10.0;
pub const K_ARC_FLARE: f32 = 11.0;
pub const K_CLOAK: f32 = 12.0;

#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct QuadParams {
    /// x kind, y brightness, z clock (s), w len.
    pub a: Vec4,
    /// rgb colour, w seed.
    pub col: Vec4,
    /// rgb second colour, w extra.
    pub col2: Vec4,
    pub p3: Vec4,
    pub p4: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct FxQuadMaterial {
    #[uniform(0)]
    pub params: QuadParams,
}

impl Material for FxQuadMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/render/fx/quad.wgsl").into_owned())
    }
    /// Premultiplied with alpha 0 is Refit's ONE, ONE: the effect only adds light.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None::<Face>;
        Ok(())
    }
}

/// Adds the quad shader and its material; the windowless tests leave it out.
pub struct FxQuadPlugin;

impl Plugin for FxQuadPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "quad.wgsl");
        app.add_plugins(MaterialPlugin::<FxQuadMaterial>::default());
    }
}

/// What a quad draws: the shader's kind and parameters (`quad.wgsl` says what each
/// kind takes), at brightness 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub kind: f32,
    pub len: f32,
    pub col: Vec3,
    pub seed: f32,
    pub col2: Vec3,
    pub extra: f32,
    pub p3: Vec4,
    pub p4: Vec4,
}

impl Look {
    pub fn new(kind: f32, col: Vec3) -> Look {
        Look { kind, len: 1.0, col, seed: 0.0, col2: Vec3::ZERO, extra: 0.0, p3: Vec4::ZERO, p4: Vec4::ZERO }
    }

    /// Refit's `Fx::flare`: core and glow sharpness, star share and sharpness, streak
    /// share, sharpness and colour.
    #[allow(clippy::too_many_arguments)]
    pub fn flare(col: Vec3, core: f32, glow: f32, star: f32, star_sharp: f32, streak: f32, streak_sharp: f32, angle: f32) -> Look {
        Look { p3: Vec4::new(core, glow, star, star_sharp), p4: Vec4::new(streak, streak_sharp, angle, 0.0), ..Look::new(K_FLARE, col) }
    }

    pub fn params(&self, bright: f32, clock: f32) -> QuadParams {
        QuadParams {
            a: Vec4::new(self.kind, bright, clock, self.len),
            col: self.col.extend(self.seed),
            col2: self.col2.extend(self.extra),
            p3: self.p3,
            p4: self.p4,
        }
    }
}

/// The clock the shaders see, wrapped so f32 keeps its fraction.
pub fn shader_clock(now: f64) -> f32 {
    (now % 1000.0) as f32
}

pub fn material(look: &Look) -> FxQuadMaterial {
    FxQuadMaterial { params: look.params(0.0, 0.0) }
}

/// Write the frame's brightness and clock; the material is touched only when they change.
pub fn drive(mats: &mut Assets<FxQuadMaterial>, handle: &Handle<FxQuadMaterial>, look: &Look, bright: f32, clock: f32) {
    let params = look.params(bright, clock);
    if mats.get(handle).is_some_and(|m| m.params != params)
        && let Some(mut m) = mats.get_mut(handle)
    {
        m.params = params;
    }
}

/// The 2 x 2 rectangle every quad scales.
pub fn rectangle() -> Mesh {
    Rectangle::new(2.0, 2.0).into()
}

/// A rectangle about `c`, half size `half`, facing the camera (`cam` is None in the
/// windowless tests: it faces +Z then).
pub fn billboard(cam: Option<Vec3>, c: Vec3, half: Vec2) -> Transform {
    let t = match cam {
        Some(eye) if eye.distance_squared(c) > 1e-6 => Transform::from_translation(c).with_rotation(face(eye - c)),
        _ => Transform::from_translation(c),
    };
    t.with_scale(half.extend(1.0))
}

/// Rotation turning +Z to `to`, with +Y as near up as it can.
fn face(to: Vec3) -> Quat {
    let z = to.normalize_or(Vec3::Z);
    let x = Vec3::Y.cross(z).try_normalize().unwrap_or(Vec3::X);
    Quat::from_mat3(&Mat3::from_cols(x, z.cross(x), z))
}

/// A rectangle along `p0..p1` (its local x), half width `hw`, facing the camera as
/// far as it can; `extend` more along either end (Refit's caps).
pub fn ribbon(cam: Option<Vec3>, p0: Vec3, p1: Vec3, hw: f32, extend: f32) -> Transform {
    let d = p1 - p0;
    let len = d.length();
    let x = if len > 1e-5 { d / len } else { Vec3::X };
    let mid = (p0 + p1) * 0.5;
    let view = cam.map_or(Vec3::Z, |e| e - mid);
    let z = (view - x * view.dot(x)).try_normalize().unwrap_or_else(|| x.any_orthonormal_vector());
    let y = z.cross(x);
    Transform { translation: mid, rotation: Quat::from_mat3(&Mat3::from_cols(x, y, z)), scale: Vec3::new(len * 0.5 + extend, hw, 1.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_shader_validates() {
        super::super::validate_wgsl(include_str!("quad.wgsl"));
    }

    #[test]
    fn a_ribbon_lies_along_its_line_facing_the_camera() {
        let t = ribbon(Some(Vec3::new(0.0, 100.0, 0.0)), Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), 2.0, 2.0);
        assert!((t.scale - Vec3::new(7.0, 2.0, 1.0)).length() < 1e-5);
        assert!((t.rotation * Vec3::X - Vec3::X).length() < 1e-5);
        assert!((t.rotation * Vec3::Z - Vec3::Y).length() < 1e-5);
    }
}
