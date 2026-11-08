//! The map backdrop and planet bodies, built the way the original engine builds
//! them (geometry as Armada II Refit documents it, `render3d/src/mods/sky` and
//! `mods/world/planets.rs`, MIT):
//!
//! - **Sky faces**: a background named by prefix is six quads at +-100 about the
//!   camera, face `i` showing `<prefix><i>` (or `<prefix>.<i+1>`), clamped, unlit.
//! - **Starfield**: the map's starfield ODF lists star sprites (`bstar.0 = 750`) and
//!   their scales (`mstar.2.scale = .40`); the stars sit on a sphere about the camera.
//!   Their angular size is our approximation.
//! - **Planets**: two hemispheres textured `<ground>1` and `<ground>2`, plus a cloud
//!   shell (`atmosphereTextureName`, else `PA_ECFR0`) just above, with the engine's
//!   gore UV layout (`GroundMesh::mSetUVs`).
//!
//! All geometry is generated in the files' left-handed space and mirrored on Z
//! like everything else.

use crate::textures::Loader;
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use sta_assets::a2fmt::spr::SpriteTable;
use std::collections::HashMap;
use std::f32::consts::PI;

fn mirror(p: [f32; 3]) -> [f32; 3] {
    [p[0], p[1], -p[2]]
}

fn clamp_sampler(images: &mut Assets<Image>, h: &Handle<Image>) {
    if let Some(mut img) = images.get_mut(h) {
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            ..ImageSamplerDescriptor::linear()
        });
    }
}

fn unlit(texture: Option<Handle<Image>>, alpha_mode: AlphaMode) -> StandardMaterial {
    StandardMaterial {
        base_color_texture: texture,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode,
        fog_enabled: false,
        ..default()
    }
}

/// The six prefix faces (`CreateBackgroundFace`): corners with UVs (0,0) (1,0) (1,1) (0,1).
const PREFIX_FACES: [[[f32; 3]; 4]; 6] = [
    [[-100.0, 100.0, 100.0], [100.0, 100.0, 100.0], [100.0, -100.0, 100.0], [-100.0, -100.0, 100.0]],
    [[100.0, 100.0, 100.0], [100.0, 100.0, -100.0], [100.0, -100.0, -100.0], [100.0, -100.0, 100.0]],
    [[100.0, 100.0, -100.0], [-100.0, 100.0, -100.0], [-100.0, -100.0, -100.0], [100.0, -100.0, -100.0]],
    [[-100.0, 100.0, -100.0], [-100.0, 100.0, 100.0], [-100.0, -100.0, 100.0], [-100.0, -100.0, -100.0]],
    [[-100.0, 100.0, -100.0], [100.0, 100.0, -100.0], [100.0, 100.0, 100.0], [-100.0, 100.0, 100.0]],
    [[-100.0, -100.0, 100.0], [100.0, -100.0, 100.0], [100.0, -100.0, -100.0], [-100.0, -100.0, -100.0]],
];

/// Spawn the six sky faces under `parent` (scale the parent to place them).
/// Returns how many faces found a texture.
pub fn sky_faces(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    loader: &mut Loader,
    prefix: &str,
    parent: Entity,
) -> usize {
    let dotted = sta_assets::find_texture(loader.vfs, &format!("{prefix}.1")).is_some();
    let mut found = 0;
    for (i, face) in PREFIX_FACES.iter().enumerate() {
        let name = if dotted { format!("{prefix}.{}", i + 1) } else { format!("{prefix}{i}") };
        let tex = loader.colour(&name, false);
        if let Some(h) = &tex {
            clamp_sampler(loader.images, h);
            found += 1;
        }
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, face.map(mirror).to_vec())
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4])
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
            .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(unlit(tex, AlphaMode::Opaque))),
            Transform::IDENTITY,
            ChildOf(parent),
        ));
    }
    found
}

/// How far off a star's `scale` is measured (see [`starfield`]).
const STAR_DISTANCE: f32 = 5000.0;

/// Small deterministic generator for star placement.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }
}

/// Spawn the starfield under `parent` at `radius`. `entries` are the starfield
/// ODF's `(sprite, count, scale)`. Returns the number of stars.
///
/// Stock builds its stars as world-space quads at random distances about the map
/// (`Starfield::mCreateStarSprite`), and how big they are is not recorded. Here a
/// star's `scale` is read as its half size in world units seen from
/// [`STAR_DISTANCE`] away: FO's point stars (scale 4-8) come out a few pixels wide,
/// its star clouds (250) about six degrees across.
#[allow(clippy::too_many_arguments)]
pub fn starfield(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    loader: &mut Loader,
    table: &SpriteTable,
    entries: &[(String, u32, f32)],
    radius: f32,
    parent: Entity,
) -> usize {
    #[derive(Default)]
    struct Batch {
        pos: Vec<[f32; 3]>,
        uv: Vec<[f32; 2]>,
        idx: Vec<u32>,
    }
    let mut batches: HashMap<String, Batch> = HashMap::new();
    let mut rng = Lcg(0x5741_3634);
    let mut total = 0;
    for (sprite_name, count, scale) in entries {
        let Some(sprite) = table.sprite(sprite_name) else { continue };
        let uv = sta_assets::spr_anim::sprite_uv(table, sprite, 0.0);
        let half = radius * scale / STAR_DISTANCE;
        let b = batches.entry(sprite.texture.to_ascii_lowercase()).or_default();
        for _ in 0..*count {
            // Uniform direction on the sphere.
            let z = rng.next() * 2.0 - 1.0;
            let a = rng.next() * 2.0 * PI;
            let r = (1.0 - z * z).sqrt();
            let dir = Vec3::new(r * a.cos(), z, r * a.sin());
            let right = dir.any_orthonormal_vector();
            let up = dir.cross(right);
            let c = dir * radius;
            let base = b.pos.len() as u32;
            for (corner, t) in [
                (c - right * half + up * half, [uv[0], uv[1]]),
                (c + right * half + up * half, [uv[0] + uv[2], uv[1]]),
                (c + right * half - up * half, [uv[0] + uv[2], uv[1] + uv[3]]),
                (c - right * half - up * half, [uv[0], uv[1] + uv[3]]),
            ] {
                b.pos.push(corner.to_array());
                b.uv.push(t);
            }
            b.idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            total += 1;
        }
    }
    for (texture, b) in batches {
        let tex = loader.colour(&texture, false);
        let n = b.pos.len();
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, b.pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; n])
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, b.uv)
            .with_inserted_indices(Indices::U32(b.idx));
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(unlit(tex, AlphaMode::Add))),
            Transform::IDENTITY,
            ChildOf(parent),
            bevy::camera::visibility::NoFrustumCulling,
        ));
    }
    total
}

/// Sphere share as one `GroundMesh` holds it: `count` quarter turns of longitude
/// from quarter `start`, UVs for a texture holding `n` quarters. Per quarter q, at
/// longitude l into it and polar angle t from +y:
/// u = lerp((2q+1)/8, (q pi/2 + l) / 2pi, w) * 4/n, v = t/pi, with w rising from 0
/// at a pole to 1 at 45 degrees from it.
pub fn sphere(start: usize, count: usize, n: usize, radius: f32) -> Mesh {
    const NL: usize = 24;
    const NT: usize = 48;
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut uv = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    for q in 0..count {
        let base = pos.len() as u32;
        for j in 0..=NL {
            let l = j as f32 / NL as f32 * PI * 0.5;
            let lon = (start + q) as f32 * PI * 0.5 + l;
            let local = q as f32 * PI * 0.5 + l;
            for i in 0..=NT {
                let t = i as f32 / NT as f32 * PI;
                let (st, ct) = (t.sin(), t.cos());
                let w = (t / (PI * 0.25)).min((PI - t) / (PI * 0.25)).min(1.0);
                let centre = (2 * q + 1) as f32 / 8.0;
                let u = (centre + (local / (2.0 * PI) - centre) * w) * (4.0 / n as f32);
                let p = mirror([lon.cos() * st, ct, lon.sin() * st]);
                nrm.push(p);
                pos.push(p.map(|v| v * radius));
                uv.push([u, t / PI]);
            }
        }
        let row = (NT + 1) as u32;
        for j in 0..NL as u32 {
            for i in 0..NT as u32 {
                let a = base + j * row + i;
                let b = base + (j + 1) * row + i;
                // Clockwise from outside in Direct3D: front-facing after the mirror.
                idx.extend_from_slice(&[a, b, a + 1, b, b + 1, a + 1]);
            }
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}
