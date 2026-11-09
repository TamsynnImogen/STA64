//! The explosion material (`fire.wgsl`, ported from Refit's
//! `effects_explosions.hlsl`) and Refit's `[Explosions]` constants: a fireball's size
//! and growth from the dead unit's radius, the shock ring.

use bevy::asset::{AssetPath, embedded_asset};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, CompareFunction, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

// `[Explosions]` defaults (render3d/mods/effects/src/explosions.rs, `read`).
const BRIGHT: f32 = 1.0;
const SIZE: f32 = 1.0;
/// The smallest fireball radius, world units.
pub const MIN_RADIUS: f32 = 25.0;
/// Share of the life the fireball grows over.
const GROWTH: f32 = 0.4;
const SMOKE: f32 = 1.0;
const TURBULENCE: f32 = 1.0;
const COOLING: f32 = 1.0;
const OPACITY: f32 = 0.3;
const FLASH: f32 = 1.0;
const GLOW: f32 = 0.25;
const DETAIL: f32 = 1.6;
const RING_BRIGHT: f32 = 1.0;
const RING_WIDTH: f32 = 0.52;
const RING_BREAKUP: f32 = 1.0;
const HDR: f32 = 0.5;
const HDR_FROM: f32 = 1.5;
/// A fireball's life when its class gives none (`explosions.rs`: 2.5 s).
pub const DEFAULT_LIFE: f32 = 2.5;
/// The fireball's radius at `s` (0..1 of its life) for a full radius `rf`.
pub fn radius_now(rf: f32, s: f32) -> f32 {
    let g = 1.0 - (s / GROWTH).min(1.0);
    rf * (0.2 + 0.8 * (1.0 - g * g * g)) * (1.0 + 0.12 * s)
}

/// The full radius for a unit of bounding radius `r0`: `Size`, at least `MinRadius`.
pub fn full_radius(r0: f32) -> f32 {
    (SIZE * r0).max(MIN_RADIUS)
}

/// The radius of the sphere that covers the effect (Refit's `quad`): the gas is pushed
/// out to 2.4 times its radius, and for the first 0.6 s the flash needs 1.7 times the
/// full.
pub fn cover_radius(rf: f32, rn: f32, age: f32) -> f32 {
    let q = rn * 2.4;
    if age < 0.6 && rf * 1.7 > q { rf * 1.7 } else { q }
}

#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct FireParams {
    pub eye: Vec4,
    pub ctr: Vec4,
    pub time: Vec4,
    pub seed: Vec4,
    pub shape: Vec4,
    pub misc: Vec4,
    pub look: Vec4,
    pub ring: Vec4,
    pub ringb: Vec4,
    pub ringc: Vec4,
}

/// One frame of a fireball.
pub struct FireFrame {
    pub eye: Vec3,
    pub forward: Vec3,
    pub near: f32,
    pub centre: Vec3,
    /// Full radius, and the age in seconds over the life in seconds.
    pub rf: f32,
    pub age: f32,
    pub life: f32,
    /// This explosion's offset into the noise (0..1 each).
    pub seed: Vec3,
}

impl FireFrame {
    pub fn params(&self) -> FireParams {
        let s = (self.age / self.life).clamp(0.0, 1.0);
        let rn = radius_now(self.rf, s);
        FireParams {
            eye: self.eye.extend(self.near),
            ctr: self.centre.extend(self.rf),
            time: Vec4::new(rn, s, self.age, BRIGHT),
            seed: self.seed.extend(DETAIL * 0.5),
            shape: Vec4::new(SMOKE, TURBULENCE, COOLING, OPACITY),
            misc: Vec4::new(FLASH, GLOW, HDR, HDR_FROM),
            look: self.forward.extend(0.0),
            ..default()
        }
    }
}

/// One frame of a shock ring in the plane with normal `up`: radius now and max, and
/// how bright (Refit's `v1`: full to `fade` of the way out, then to nothing).
pub struct RingFrame {
    pub eye: Vec3,
    pub forward: Vec3,
    pub near: f32,
    pub centre: Vec3,
    pub up: Vec3,
    pub radius: f32,
    pub max: f32,
    pub age: f32,
    pub colour: Vec3,
    pub seed: f32,
    /// The share of the way out where it starts to fade (the class's `fadeFactor`).
    pub fade: f32,
}

impl RingFrame {
    pub fn params(&self) -> FireParams {
        let mut v1 = 1.0;
        if self.radius > self.fade * self.max && self.fade < 1.0 {
            v1 = (self.max - self.radius) / ((1.0 - self.fade) * self.max);
        }
        if self.age < 0.08 {
            v1 *= self.age / 0.08;
        }
        FireParams {
            eye: self.eye.extend(self.near),
            ctr: self.centre.extend(0.0),
            time: Vec4::new(0.0, 0.0, self.age, BRIGHT),
            look: self.forward.extend(1.0),
            ring: self.up.extend(self.radius),
            ringb: Vec4::new(RING_WIDTH, RING_BRIGHT * v1.max(0.0), self.max, self.seed * 8.0),
            ringc: self.colour.extend(RING_BREAKUP),
            ..default()
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct FireMaterial {
    #[uniform(0)]
    pub params: FireParams,
}

impl Material for FireMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/render/fx/fire.wgsl").into_owned())
    }
    /// ONE, ONE_MINUS_SRC_ALPHA, as Refit's `PREMUL`.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }
    /// The sphere's back faces with no depth test, as the nebulae: the march finds
    /// its own extent, so the gas shows from inside it too.
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = Some(Face::Front);
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_compare = Some(CompareFunction::Always);
            ds.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

pub struct FirePlugin;

impl Plugin for FirePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "fire.wgsl");
        app.add_plugins(MaterialPlugin::<FireMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fire_shader_validates() {
        crate::render::fx::validate_wgsl(include_str!("fire.wgsl"));
    }

    #[test]
    fn a_fireball_grows_then_holds() {
        let rf = full_radius(10.0);
        assert_eq!(rf, MIN_RADIUS);
        assert!(radius_now(rf, 0.0) < radius_now(rf, 0.2) && radius_now(rf, 0.2) < radius_now(rf, 0.4));
        assert!(radius_now(rf, 1.0) > rf);
        assert_eq!(cover_radius(rf, radius_now(rf, 0.0), 0.0), rf * 1.7);
    }
}
