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

use crate::render::textures::Loader;
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

/// How big and how far the engine's stars are, from `ART_CFG.h`
/// (`DETAIL_n_STARFIELD_BACKGROUND_STAR_*`, equal on every detail level of every
/// shipped file): a star is `MAX_SIZE - rand * SIZE_VARIATION` world units across
/// times its ODF `scale`, `MAX_DISTANCE - rand * DISTANCE_VARIATION` away.
///
/// That is how Fleet Operations' ODFs read: stock's point stars (`bstar.0.scale = .4`,
/// 750 of them) come out about a pixel wide (75 * .4 = 30 units at 40000), and the
/// tile sprites of Fleet Ops (`bstar.0 = 36`, `scale = 250`: a 1024 sheet of many
/// small stars) about 25 degrees across, which is what lets 59 of them fill the sky.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StarSizing {
    pub size: f32,
    pub size_variation: f32,
    pub distance: f32,
    pub distance_variation: f32,
}

impl Default for StarSizing {
    fn default() -> Self {
        StarSizing { size: 75.0, size_variation: 35.0, distance: 40000.0, distance_variation: 10000.0 }
    }
}

impl StarSizing {
    /// Read `ART_CFG.h` (detail level 2, the middle one; the levels agree).
    pub fn from_cfg(cfg: Option<&sta_assets::cfg_h::CfgH>) -> StarSizing {
        let d = StarSizing::default();
        let Some(c) = cfg else { return d };
        let f = |k: &str, v: f32| c.f32(&format!("DETAIL_2_STARFIELD_BACKGROUND_STAR_{k}")).unwrap_or(v);
        StarSizing {
            size: f("MAX_SIZE", d.size),
            size_variation: f("SIZE_VARIATION", d.size_variation),
            distance: f("MAX_DISTANCE", d.distance),
            distance_variation: f("DISTANCE_VARIATION", d.distance_variation),
        }
    }
}

/// A sprite at least this many texels wide is a sheet of many stars, not one star.
const TILE_TEXELS: f32 = 512.0;

/// How much larger than the ODF's size a star sheet is drawn. The ODF sizes (Fleet Ops:
/// `scale = 250`) make a 1024 sheet about 20 degrees across, which puts two or three
/// of its texels on every pixel of a 1080p view at the 45 degree field: the 1 to 2
/// texel stars shrink through the mip chain to nearly nothing, and what is left shows
/// only where several sheets overlap, as patches. Doubled it is about 40 degrees,
/// a texel to a pixel (Refit's menu sky, a 1024 tile over 38 degrees at 1080p, reads
/// the same), and the stars stand in even density across the whole sky.
const TILE_GAIN: f32 = 2.0;

/// The size factor for a star sprite `texels` wide: [`TILE_GAIN`] for a sheet, 1 for a star.
pub fn tile_gain(texels: f32) -> f32 {
    if texels >= TILE_TEXELS { TILE_GAIN } else { 1.0 }
}

/// One star quad of the sky sphere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StarQuad {
    /// Index into the entries it came from.
    pub entry: usize,
    /// Unit direction from the camera.
    pub dir: Vec3,
    /// Half the quad's angular size, as a tangent: its half edge over its distance.
    pub half_tan: f32,
    /// Turn of the quad about its direction, radians.
    pub roll: f32,
}

/// Small deterministic generator for star placement.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }
}

/// Where the stars of `entries` (`(sprite, count, scale)`) stand. Each entry's stars
/// are spread over the whole sphere evenly (a golden-angle lattice, every star
/// jittered within its cell, the lattice turned at random), so a handful of large
/// tiles still leaves no part of the sky bare; stock's purely random scatter is
/// that, clumps and gaps included, which a few big tiles cannot afford.
pub fn star_quads(entries: &[(String, u32, f32)], sizing: &StarSizing, seed: u64) -> Vec<StarQuad> {
    let mut rng = Lcg(seed);
    let golden = PI * (3.0 - 5.0f32.sqrt());
    let mut out = Vec::new();
    for (entry, (_, count, scale)) in entries.iter().enumerate() {
        let n = *count as usize;
        let turn = rng.next() * 2.0 * PI;
        let tilt = rng.next() * 2.0 - 1.0;
        for i in 0..n {
            let cell = 2.0 / n as f32;
            let z = (1.0 - (i as f32 + 0.5) * cell + (rng.next() - 0.5) * cell * 0.7).clamp(-1.0, 1.0);
            let a = turn + golden * i as f32 + (rng.next() - 0.5) * 0.7 * (2.0 * PI / (n as f32).sqrt());
            let r = (1.0 - z * z).sqrt();
            let lattice = Vec3::new(r * a.cos(), z, r * a.sin());
            // The lattice, turned about x by a random angle so its poles are anywhere.
            let dir = Quat::from_rotation_x(tilt * PI) * lattice;
            let size = (sizing.size - rng.next() * sizing.size_variation).max(0.0) * scale;
            let dist = (sizing.distance - rng.next() * sizing.distance_variation).max(1.0);
            out.push(StarQuad { entry, dir, half_tan: size * 0.5 / dist, roll: rng.next() * 2.0 * PI });
        }
    }
    out
}

/// Spawn the starfield under `parent` at `radius`. `entries` are the starfield
/// ODF's `(sprite, count, scale)`. Returns the number of stars drawn (a sprite whose texture
/// cannot be loaded is skipped with a warning).
///
/// Stock builds its stars as world-space quads about the map
/// (`Starfield::mCreateStarSprite`); here each is a quad on a sphere of `radius`
/// about the camera, sized by [`StarSizing`] so it subtends what the engine's would.
#[allow(clippy::too_many_arguments)]
pub fn starfield(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    loader: &mut Loader,
    table: &SpriteTable,
    entries: &[(String, u32, f32)],
    sizing: &StarSizing,
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
    // Entries whose sprite the table lacks are not drawn. A sheet-sized sprite is a tile
    // of many 1 to 2 texel stars, which the sky shows at about a texel a pixel (see
    // [`TILE_GAIN`]); a sprite of one star keeps the ODF's size.
    let drawn: Vec<(String, u32, f32)> = entries
        .iter()
        .filter_map(|(n, count, scale)| {
            let sprite = table.sprite(n)?;
            Some((n.clone(), *count, scale * tile_gain(sprite.rect[2])))
        })
        .collect();
    for q in star_quads(&drawn, sizing, 0x5741_3634) {
        let Some(sprite) = table.sprite(&drawn[q.entry].0) else { continue };
        let uv = sta_assets::spr_anim::sprite_uv(table, sprite, 0.0);
        let b = batches.entry(sprite.texture.to_ascii_lowercase()).or_default();
        let half = radius * q.half_tan;
        let right = Quat::from_axis_angle(q.dir, q.roll) * q.dir.any_orthonormal_vector();
        let up = q.dir.cross(right);
        let c = q.dir * radius;
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
    }
    let mut drawn_stars = 0;
    for (texture, b) in batches {
        // A texture that cannot be found or decoded is not drawn: the additive material of an
        // untextured quad is an opaque white square, never a star.
        let Some(tex) = loader.colour(&texture, false) else {
            warn!("starfield: texture {texture:?} not found or not readable; its {} stars are not drawn", b.pos.len() / 4);
            continue;
        };
        drawn_stars += b.pos.len() / 4;
        let n = b.pos.len();
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, b.pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; n])
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, b.uv)
            .with_inserted_indices(Indices::U32(b.idx));
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(unlit(Some(tex), AlphaMode::Add))),
            Transform::IDENTITY,
            ChildOf(parent),
            bevy::camera::visibility::NoFrustumCulling,
        ));
    }
    drawn_stars
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Fleet Ops' `stars_default.odf`: 36 + 23 tile sprites at scale 250, 27 + 41 point
    /// stars at 8 and 4 (127 stars; the mod's sprite table draws the first two groups).
    fn default_odf() -> Vec<(String, u32, f32)> {
        [("bstar.0", 36, 250.0), ("bstar.9", 23, 250.0), ("bstar.6", 27, 8.0), ("bstar.7", 41, 4.0)]
            .map(|(n, c, s)| (n.to_owned(), c, s))
            .to_vec()
    }

    /// A star sprite whose texture is nowhere in the mount draws nothing (no white quads).
    #[test]
    fn stars_without_a_texture_are_not_drawn() {
        use bevy::image::CompressedImageFormats;
        let table = SpriteTable::load("sprites.spr", |_| Some(b"sprite_table\n@tmaterial=additive\n@reference=128\nbstar.0 no_such_texture 0 0 16 16\n".to_vec()));
        assert!(table.sprite("bstar.0").is_some());
        assert_eq!(star_quads(&[("bstar.0".to_owned(), 50, 1.0)], &StarSizing::default(), 1).len(), 50, "stars are generated; only the texture is missing");
        let mut world = World::new();
        let (mut meshes, mut materials, mut images) = (Assets::<Mesh>::default(), Assets::<StandardMaterial>::default(), Assets::<Image>::default());
        let mut cache = crate::render::textures::TextureCache::default();
        let vfs = sta_vfs::Vfs::default();
        let parent = world.spawn_empty().id();
        let n = {
            let mut commands = world.commands();
            let mut loader = Loader { vfs: &vfs, cache: &mut cache, images: &mut images, formats: CompressedImageFormats::NONE };
            starfield(&mut commands, &mut meshes, &mut materials, &mut loader, &table, &[("bstar.0".to_owned(), 50, 1.0)], &StarSizing::default(), 1000.0, parent)
        };
        world.flush();
        assert_eq!(n, 0, "no star is counted as drawn");
        assert_eq!(world.query::<&Mesh3d>().iter(&world).count(), 0, "no untextured quad is spawned");
        assert!(materials.is_empty());
    }

    #[test]
    fn sheets_are_drawn_larger_and_single_stars_as_listed() {
        assert_eq!(tile_gain(1024.0), TILE_GAIN);
        assert_eq!(tile_gain(128.0), 1.0);
    }

    #[test]
    fn stars_cover_every_octant_and_the_odf_counts() {
        let q = star_quads(&default_odf(), &StarSizing::default(), 1);
        assert_eq!(q.len(), 127);
        for entry in 0..4 {
            let mut octants = [0u32; 8];
            for s in q.iter().filter(|s| s.entry == entry) {
                octants[usize::from(s.dir.x > 0.0) + 2 * usize::from(s.dir.y > 0.0) + 4 * usize::from(s.dir.z > 0.0)] += 1;
            }
            let n = [36u32, 23, 27, 41][entry];
            assert!(octants.iter().all(|c| *c >= n / 16), "entry {entry}: {octants:?}");
        }
    }

    /// The tile sprites are about 25 degrees across and, laid evenly, leave little bare.
    #[test]
    fn tiles_follow_the_odf_scale_and_fill_the_sky() {
        let sizing = StarSizing::default();
        let q = star_quads(&default_odf(), &sizing, 7);
        let tiles: Vec<_> = q.iter().filter(|s| s.entry < 2).collect();
        for s in &tiles {
            // size = (75 - rand*35) * 250 over a distance of 30000..40000.
            let (lo, hi) = (40.0 * 250.0 * 0.5 / 40000.0, 75.0 * 250.0 * 0.5 / 30000.0);
            assert!((lo..=hi).contains(&s.half_tan), "{}", s.half_tan);
        }
        // Point stars follow their scale too: 4 and 8 times 40 to 75 units.
        let point = q.iter().find(|s| s.entry == 3).unwrap();
        assert!(point.half_tan < 0.01, "a star is a blob of a few pixels, not a patch");
        // Directions inside the inscribed circle of some tile (the squares cover about a third more).
        let mut rng = Lcg(99);
        let (mut hit, total) = (0, 4000);
        for _ in 0..total {
            let z = rng.next() * 2.0 - 1.0;
            let a = rng.next() * 2.0 * PI;
            let r = (1.0 - z * z).sqrt();
            let d = Vec3::new(r * a.cos(), z, r * a.sin());
            hit += usize::from(tiles.iter().any(|t| t.dir.dot(d) >= t.half_tan.atan().cos()));
        }
        assert!(hit as f32 / total as f32 > 0.4, "tiles cover {hit}/{total} of the sphere");
    }

    #[test]
    fn sizing_reads_art_cfg() {
        let c = sta_assets::cfg_h::CfgH::parse(
            "float DETAIL_2_STARFIELD_BACKGROUND_STAR_MAX_SIZE = 90.0;\nfloat DETAIL_2_STARFIELD_BACKGROUND_STAR_MAX_DISTANCE = 50000.0;\n",
        );
        let s = StarSizing::from_cfg(Some(&c));
        assert_eq!((s.size, s.distance, s.size_variation), (90.0, 50000.0, 35.0));
        assert_eq!(StarSizing::from_cfg(None), StarSizing::default());
    }

    /// Install-gated: Fleet Ops 4.0's own `ART_CFG.h` and `stars_default.odf`.
    #[test]
    fn fo4_files_give_the_expected_star_sizes() {
        let roots = std::path::Path::new("/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations Roots");
        let Ok(vfs) = sta_vfs::Vfs::mount_install(roots, Some("Fleet Ops 4.0")) else { return };
        let cfg = sta_content::layered::cfg_h(&vfs, "ART_CFG.h");
        assert_eq!(StarSizing::from_cfg(cfg.as_ref()), StarSizing::default());
        let odf = vfs.list("odf").into_iter().find(|p| p.to_ascii_lowercase().ends_with("stars_default.odf")).unwrap();
        let text = String::from_utf8_lossy(&vfs.read(&odf).unwrap().unwrap()).into_owned();
        let sum: u32 = text.lines().filter(|l| !l.contains(".scale")).filter_map(|l| l.split('=').nth(1)?.trim().parse::<u32>().ok()).sum();
        assert_eq!(sum, 127);
        assert!(text.contains("bstar.0.scale = 250"));
    }
}
