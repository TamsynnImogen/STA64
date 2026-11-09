//! The shield-hit material (`shield.wesl`, ported from Refit's
//! `effects_shields.hlsl`) and Refit's `[Shields]` constants: what a hit looks like
//! at a given age (`Shields::hit_now`), the faction colours of `effects.ini`.

use bevy::asset::{AssetPath, embedded_asset};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

// `[Shields]` defaults (render3d/mods/effects/src/shields.rs, `read`).
const SPOT: f32 = 1.0;
const SPOT_SIZE: f32 = 0.18;
const FLASH: f32 = 1.5;
const FLASH_TIME: f32 = 0.08;
/// Seconds the spot lasts (`Duration`), and the share it sustains while a beam holds.
pub const DURATION: f32 = 0.45;
const SUSTAIN: f32 = 0.55;
const RING: f32 = 1.1;
const RING_SPEED: f32 = 2.4;
const RING_WIDTH: f32 = 0.09;
pub const RING_TIME: f32 = 0.55;
const RING_PERIOD: f32 = 0.4;
const LATTICE: f32 = 0.25;
const GLOW_REACH: f32 = 0.35;
pub const GLOW_TIME: f32 = 0.6;
const GLOW_SUSTAIN: f32 = 0.3;
const RIM: f32 = 0.8;
const RIM_POWER: f32 = 3.0;
const CORE: f32 = 0.35;
/// The damage a hit of strength 1 does, and the strength's bounds.
const DAMAGE_REF: f32 = 60.0;
const STRENGTH_MIN: f32 = 0.6;
const STRENGTH_MAX: f32 = 1.8;
/// Seconds a held beam's hit fades over once the beam ends.
pub const FADE_OUT: f32 = 0.25;
const MIST: f32 = 0.45;
const MIST_REACH: f32 = 0.35;
const MIST_SHARE: f32 = 1.0;
const MIST_CONTRAST: f32 = 1.6;
const MIST_SCALE: f32 = 3.0;
const MIST_DRIFT: f32 = 0.08;
const EDGE: f32 = 0.15;
const RAYS: f32 = 1.2;
const RAY_LENGTH: f32 = 0.55;
const RAY_SHARP: f32 = 2.0;

/// Seconds a hit is kept: Refit's `gather` keep time.
pub fn keep_time() -> f32 {
    FADE_OUT.max(RING_TIME).max(GLOW_TIME * 3.0).max(DURATION * 3.0)
}

/// A faction's shield colour (0..1) and whether its hits throw rays: `effects.ini`'s
/// `[Factions]` by the ODF's first letter, `RayFactions = Klingon`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Faction {
    pub col: Vec3,
    pub rays: bool,
}

impl Faction {
    pub fn of_odf(odf: &str) -> Faction {
        let c = |r: f32, g: f32, b: f32| Vec3::new(r, g, b) / 255.0;
        match odf.chars().next().map(|c| c.to_ascii_lowercase()) {
            Some('f') => Faction { col: c(90.0, 170.0, 255.0), rays: false },
            Some('k') => Faction { col: c(12.0, 255.0, 6.0), rays: true },
            Some('r') => Faction { col: c(90.0, 255.0, 130.0), rays: false },
            Some('b') => Faction { col: c(110.0, 255.0, 120.0), rays: false },
            Some('c') => Faction { col: c(255.0, 200.0, 80.0), rays: false },
            Some('8') => Faction { col: c(255.0, 140.0, 40.0), rays: false },
            _ => Faction { col: c(140.0, 200.0, 255.0), rays: false },
        }
    }
}

/// A hit's strength from the damage it did to the shields (Refit's `strength`).
pub fn strength(damage: f32) -> f32 {
    (damage / DAMAGE_REF).sqrt().clamp(STRENGTH_MIN, STRENGTH_MAX)
}

/// What a hit adds at `age` seconds: spot, ring radius (shield radii), ring, glow.
/// `held` is how long a beam holds its hit (`None` for a shot that lands once):
/// Refit's `hit_now`.
pub fn hit_now(strength: f32, age: f32, held: Option<f32>) -> (f32, f32, f32, f32) {
    let age = age.max(0.0);
    let flash = 1.0 + FLASH * (-age / FLASH_TIME).exp();
    let (spot, glow, te);
    match held {
        Some(h) if age <= h => {
            spot = strength * flash * (SUSTAIN + (1.0 - SUSTAIN) * (-age / DURATION).exp());
            glow = strength * (GLOW_SUSTAIN + (1.0 - GLOW_SUSTAIN) * (-age / GLOW_TIME).exp());
            te = (age / RING_PERIOD).floor() * RING_PERIOD;
        }
        Some(h) => {
            let out = (1.0 - (age - h) / FADE_OUT).clamp(0.0, 1.0);
            spot = strength * (SUSTAIN + (1.0 - SUSTAIN) * (-h / DURATION).exp()) * out;
            glow = strength * (GLOW_SUSTAIN + (1.0 - GLOW_SUSTAIN) * (-h / GLOW_TIME).exp()) * out;
            te = (h / RING_PERIOD).floor() * RING_PERIOD;
        }
        None => {
            spot = strength * flash * (-age / DURATION).exp();
            glow = strength * (-age / GLOW_TIME).exp();
            te = 0.0;
        }
    }
    let tr = age - te;
    let ring = if tr < RING_TIME {
        let f = 1.0 - tr / RING_TIME;
        strength * f * f * if held.is_some() && te > 0.0 { 0.6 } else { 1.0 }
    } else {
        0.0
    };
    (spot, RING_SPEED * tr, ring, glow)
}

#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct ShieldParams {
    pub eye: Vec4,
    pub col: Vec4,
    pub shape: Vec4,
    pub mixes: Vec4,
    pub core: Vec4,
    pub mist: Vec4,
    pub ray: Vec4,
    pub drift: Vec4,
    pub hit: Vec4,
    pub now: Vec4,
    pub unit: Vec4,
    pub inv0: Vec4,
    pub inv1: Vec4,
    pub inv2: Vec4,
}

/// The shield's place and the hit on it for one frame.
pub struct ShieldFrame {
    pub eye: Vec3,
    pub clock: f64,
    pub faction: Faction,
    /// The ellipsoid: centre, orientation, semi-axes.
    pub centre: Vec3,
    pub rot: Quat,
    pub scale: Vec3,
    /// Where the hit struck, on the unit sphere, and a seed 0..1.
    pub unit: Vec3,
    pub seed: f32,
    /// Spot, ring radius, ring, glow now.
    pub now: (f32, f32, f32, f32),
    pub brightness: f32,
}

impl ShieldFrame {
    pub fn params(&self) -> ShieldParams {
        let model = Mat4::from_scale_rotation_translation(self.scale, self.rot, self.centre);
        let inv = model.inverse();
        let hit = model.transform_point3(self.unit);
        let mean = (self.scale.x * self.scale.y * self.scale.z).cbrt().max(1e-3);
        let dt = (self.clock % 2000.0) as f32 * MIST_DRIFT;
        let (spot, ring_r, ring, glow) = self.now;
        ShieldParams {
            eye: self.eye.extend(1.0 / mean),
            col: self.faction.col.extend(self.brightness),
            shape: Vec4::new(1.0 / SPOT_SIZE, 1.0 / RING_WIDTH, 1.0 / GLOW_REACH, RIM_POWER),
            mixes: Vec4::new(SPOT, RING, LATTICE, RIM),
            core: Vec4::new(CORE, 0.0, 0.0, EDGE),
            mist: Vec4::new(MIST, MIST_SHARE, MIST_CONTRAST, 1.0 / MIST_REACH),
            ray: Vec4::new(if self.faction.rays { RAYS } else { 0.0 }, 1.0 / RAY_LENGTH, RAY_SHARP, (self.clock % 1000.0) as f32),
            drift: Vec4::new(dt * 0.6, dt * 0.35, -dt * 0.72, MIST_SCALE),
            hit: hit.extend(spot),
            now: Vec4::new(ring_r, ring, glow, 0.0),
            unit: self.unit.extend(self.seed),
            inv0: inv.row(0),
            inv1: inv.row(1),
            inv2: inv.row(2),
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct ShieldMaterial {
    #[uniform(0)]
    pub params: ShieldParams,
}

impl Material for ShieldMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/render/fx/shield.wesl").into_owned())
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }
    /// Both faces: Refit's shader takes `abs(dot(n, v))`.
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

pub struct ShieldPlugin;

impl Plugin for ShieldPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shield.wesl");
        app.add_plugins(MaterialPlugin::<ShieldMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shield_shader_validates() {
        crate::render::fx::validate_wgsl(include_str!("shield.wesl"));
    }

    #[test]
    fn a_hit_peaks_at_once_and_dies_away() {
        let s = strength(60.0);
        assert_eq!(s, 1.0);
        let a = hit_now(s, 0.0, None);
        let b = hit_now(s, 0.3, None);
        assert!(a.0 > 2.4 && a.3 > b.3 && b.0 < a.0);
        // The ring runs out and is gone after RING_TIME.
        assert!(hit_now(s, 0.2, None).1 > 0.4 && hit_now(s, 0.2, None).2 > 0.0);
        assert_eq!(hit_now(s, 0.6, None).2, 0.0);
        // A held beam keeps its spot, then fades once it ends.
        let held = hit_now(s, 1.0, Some(2.0)).0;
        assert!(held > 0.5 * s);
        assert_eq!(hit_now(s, 2.25, Some(2.0)).0, 0.0);
    }

    #[test]
    fn factions_follow_the_odf_initial() {
        assert!(Faction::of_odf("kvorcha").rays);
        assert!(!Faction::of_odf("fgalaxy").rays);
        assert_eq!(Faction::of_odf("8472bio").col, Vec3::new(255.0, 140.0, 40.0) / 255.0);
    }
}
