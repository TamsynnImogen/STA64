//! Surfaces made from recipes: a planet's ground, a gas giant, the moons' and the
//! asteroids' rock, as functions of direction baked into three cube maps (colour,
//! relief, clouds).
//!
//! A port of Armada II Refit's `render3d/shaders/world_surface.hlsl` and the recipe
//! reader of `src/mods/world/surfaces.rs` (MIT, Copyright (c) 2026 Cedric Haase),
//! themselves Surfaces.asi. Refit bakes on the GPU; this bakes on the CPU across all
//! cores with the same maths, so it is deterministic and testable without a GPU.
//!
//! Directions are in the files' left-handed object space; faces are in Direct3D's
//! order and orientation (+x, -x, +y, -y, +z, -z), which wgpu's cube maps share.

mod noise;
pub mod nebula;
pub mod recipe;

pub use recipe::Recipe;
use noise::*;

/// The cube faces: the direction at a face's centre, and the directions its texel
/// column and row grow along.
pub const FACES: [[[f32; 3]; 3]; 6] = [
    [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, -1.0, 0.0]],
    [[-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]],
    [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
    [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]],
    [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0]],
    [[0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0]],
];

/// Six square faces of RGBA8 texels, face-major, rows top-down.
#[derive(Clone)]
pub struct Cube {
    pub size: u32,
    pub faces: [Vec<u8>; 6],
}

/// A baked surface.
pub struct Surface {
    /// rgb: colour; a: how wet (glint) or, with `Cut`, opacity.
    pub albedo: Cube,
    /// rgb: normal - direction + 0.5; a: glow.
    pub relief: Cube,
    /// Cloud density in every channel (None without clouds).
    pub clouds: Option<Cube>,
}

/// Texel (i, j) of an `s`-texel face holds face coordinates 2i/(s-1) - 1, so edge
/// texels hold the edge itself.
pub fn face_dir(face: usize, s: u32, i: u32, j: u32) -> V3 {
    let f = FACES[face];
    let k = 2.0 / (s - 1) as f32;
    let (u, v) = (i as f32 * k - 1.0, j as f32 * k - 1.0);
    norm(add(add(v3(f[0]), scale(v3(f[1]), u)), scale(v3(f[2]), v)))
}

/// A float cube on the CPU (the height field: height, glow), bilinear lookups by
/// direction, clamped at face edges like the bake's sampler.
struct FloatCube {
    s: u32,
    data: Vec<[f32; 2]>,
}

impl FloatCube {
    fn at(&self, face: usize, i: u32, j: u32) -> [f32; 2] {
        self.data[(face * (self.s * self.s) as usize) + (j * self.s + i) as usize]
    }

    fn sample(&self, d: V3) -> [f32; 2] {
        let (face, u, v) = cube_face_uv(d);
        // Texel centres sit on the face's 2i/(s-1)-1 lattice (edge texels on the edge).
        let s = self.s as f32;
        let x = ((u + 1.0) * 0.5 * (s - 1.0)).clamp(0.0, s - 1.0);
        let y = ((v + 1.0) * 0.5 * (s - 1.0)).clamp(0.0, s - 1.0);
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(self.s - 1), (y0 + 1).min(self.s - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let a = self.at(face, x0, y0);
        let b = self.at(face, x1, y0);
        let c = self.at(face, x0, y1);
        let e = self.at(face, x1, y1);
        std::array::from_fn(|k| lerp(lerp(a[k], b[k], fx), lerp(c[k], e[k], fx), fy))
    }
}

/// The face a direction falls on and its coordinates there (-1..1) along the face's
/// column and row axes.
fn cube_face_uv(d: V3) -> (usize, f32, f32) {
    let a = [d[0].abs(), d[1].abs(), d[2].abs()];
    let face = if a[0] >= a[1] && a[0] >= a[2] {
        if d[0] >= 0.0 { 0 } else { 1 }
    } else if a[1] >= a[2] {
        if d[1] >= 0.0 { 2 } else { 3 }
    } else if d[2] >= 0.0 {
        4
    } else {
        5
    };
    let f = FACES[face];
    let major = dot(d, v3(f[0]));
    let p = scale(d, 1.0 / major);
    (face, dot(p, v3(f[1])), dot(p, v3(f[2])))
}

/// The nearest texel of a baked cube in direction `d` (previews, tests).
pub fn sample_nearest(c: &Cube, d: V3) -> [u8; 4] {
    let (face, u, v) = cube_face_uv(norm(d));
    let s = c.size as f32;
    let i = (((u + 1.0) * 0.5 * (s - 1.0)).round() as u32).min(c.size - 1);
    let j = (((v + 1.0) * 0.5 * (s - 1.0)).round() as u32).min(c.size - 1);
    let o = ((j * c.size + i) * 4) as usize;
    let f = &c.faces[face];
    [f[o], f[o + 1], f[o + 2], f[o + 3]]
}

/// Run `f` for every texel of an `s`-texel cube on all cores, face by face.
fn par_faces<T: Send + Copy + Default>(s: u32, f: impl Fn(usize, u32, u32) -> T + Sync) -> Vec<T> {
    let n = (s * s) as usize;
    let mut out = vec![T::default(); n * 6];
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let rows_total = 6 * s as usize;
    let chunk_rows = rows_total.div_ceil(threads);
    std::thread::scope(|scope| {
        for (c, chunk) in out.chunks_mut(chunk_rows * s as usize).enumerate() {
            let f = &f;
            scope.spawn(move || {
                for (k, px) in chunk.iter_mut().enumerate() {
                    let idx = c * chunk_rows * s as usize + k;
                    let face = idx / n;
                    let r = idx % n;
                    *px = f(face, (r % s as usize) as u32, (r / s as usize) as u32);
                }
            });
        }
    });
    out
}

fn to_cube(s: u32, px: Vec<[u8; 4]>) -> Cube {
    let n = (s * s) as usize;
    let faces = std::array::from_fn(|f| px[f * n..(f + 1) * n].iter().flat_map(|p| *p).collect());
    Cube { size: s, faces }
}

fn byte(v: f32) -> u8 {
    (saturate(v) * 255.0 + 0.5) as u8
}

/// Bake a recipe: colour and relief at `face` texels a side, clouds at `cloud_face`.
pub fn bake(r: &Recipe, face: u32, cloud_face: u32) -> Surface {
    let face = face.max(8);
    let k = &r.k;
    let height = FloatCube {
        s: face,
        data: par_faces(face, |f, i, j| height_at(k, face_dir(f, face, i, j))),
    };
    let tap = 2.0 / (face - 1) as f32;
    let sea = k.sea[0];

    let albedo = par_faces(face, |f, i, j| {
        let d = face_dir(f, face, i, j);
        let hg = height.sample(d);
        let a = albedo_at(k, d, hg, tap, |p| height.sample(p)[0].max(sea));
        a.map(byte)
    });
    let relief = par_faces(face, |f, i, j| {
        let d = face_dir(f, face, i, j);
        let glow = height.sample(d)[1];
        let (t1, t2) = tangents(d, v3(FACES[f][1]));
        let e = tap;
        let h = |p: V3| height.sample(norm(p))[0].max(sea);
        let hx1 = h(add(d, scale(t1, e)));
        let hx0 = h(sub(d, scale(t1, e)));
        let hy1 = h(add(d, scale(t2, e)));
        let hy0 = h(sub(d, scale(t2, e)));
        let g = scale(add(scale(t1, hx1 - hx0), scale(t2, hy1 - hy0)), 1.0 / (2.0 * e));
        let n = norm(sub(d, scale(g, k.sea[2] * (1.0 - k.seed[3]))));
        let r = add(sub(n, d), [0.5; 3]);
        [byte(r[0]), byte(r[1]), byte(r[2]), byte(glow)]
    });
    let clouds = r.clouds.then(|| {
        let cf = cloud_face.max(8);
        to_cube(
            cf,
            par_faces(cf, |f, i, j| {
                let c = byte(cloud_at(k, face_dir(f, cf, i, j)));
                [c, c, c, c]
            }),
        )
    });
    Surface {
        albedo: to_cube(face, albedo),
        relief: to_cube(face, relief),
        clouds,
    }
}

/// Two tangents at d along the face's column axis.
fn tangents(d: V3, axis: V3) -> (V3, V3) {
    let t1 = norm(sub(axis, scale(d, dot(d, axis))));
    let t2 = norm(cross(d, t1));
    (t1, t2)
}

/// The height at direction d, in field units, and the glow there.
pub fn height_at(k: &recipe::Consts, d: V3) -> [f32; 2] {
    let (sc, warp, detail) = (k.noise[0], k.noise[1], k.noise[2]);
    let seed = [k.seed[0], k.seed[1], k.seed[2]];
    let p = add(scale(d, sc), seed);
    let q = [
        fbm3(add(p, [1.7, 9.2, 3.4]), 0.5) - 0.5,
        fbm3(add(p, [8.3, 2.8, 5.1]), 0.5) - 0.5,
        fbm3(add(p, [4.6, 7.1, 0.9]), 0.5) - 0.5,
    ];
    if k.seed[3] > 0.5 {
        let g = k.gas;
        let sp = add(scale([d[0], d[1] * g[3], d[2]], sc), seed);
        let t = fbm7(add(sp, scale(q, warp * 2.0)), detail) - 0.5;
        let lat = d[1] + 0.08 * (d[1] * 7.3 + k.seed[0]).sin();
        let b = lat * g[0] + t * g[1] + (q[0] + q[1]) * g[2];
        let sd = dot(d, [k.storm[0], k.storm[1], k.storm[2]]);
        let st = ((sd - 1.0) / (k.storm[3] * k.storm[3]).max(1e-4)).exp();
        return [b, st];
    }
    let mut n = (fbm7(add(p, scale(q, warp * 2.0)), detail) - 0.5) * 8.0;
    let zxy = [k.seed[2], k.seed[0], k.seed[1]];
    let yzx = [k.seed[1], k.seed[2], k.seed[0]];
    n += (vnoise(add(add(scale(d, sc * 0.3), zxy), [11.0; 3])) - 0.5) * 6.0 * k.sea[3];
    let land = smoothstep(k.sea[0], k.sea[0] + 1.5, n);
    n += k.noise[3] * land * (ridged(add(scale(p, 1.6), scale(q, 1.5))) - 0.3) * 5.0;
    let c = k.crater;
    n += craters(d, c[1], c[0], c[2], c[3], seed);
    n += craters(d, c[1] * 2.7, c[0], c[2] * 0.6, c[3], add(yzx, [3.1; 3])) * 0.7;
    let mut glow = 0.0;
    if k.crack[0] > 0.0 {
        let fp = add(add(scale(d, k.crack[0]), scale(q, k.fiss[0])), zxy);
        let a = fissure(k, fp, k.crack[1]);
        let b = fissure(k, add(add(scale(fp, 2.3), scale(q, k.fiss[0] * 0.6)), [4.1; 3]), k.crack[1] * 1.4);
        let b = [b[0] * k.cavity[2], b[1] * k.cavity[2]];
        let w = [a[0].max(b[0]), a[1].max(b[1])];
        n -= k.crack[2] * w[0];
        glow = k.crack[3] * w[1] * (0.7 + 0.3 * vnoise(scale(p, 6.0)));
    }
    [n, glow]
}

fn fissure(k: &recipe::Consts, p: V3, width: f32) -> [f32; 2] {
    let f = fissures(p);
    let t = 1.0 - k.fiss[1];
    let keep = smoothstep(t - 0.1, t + 0.1, f[1]);
    [(1.0 - smoothstep(0.0, width, f[0])) * keep, (1.0 - smoothstep(0.0, width * 0.35, f[0])) * keep]
}

fn ramp(r: &[[f32; 4]; 6], h: f32) -> [f32; 3] {
    let mut c = [r[0][0], r[0][1], r[0][2]];
    for i in 1..6 {
        let t = saturate((h - r[i - 1][3]) / (r[i][3] - r[i - 1][3]).max(1e-4));
        c = std::array::from_fn(|ch| lerp(c[ch], r[i][ch], t));
    }
    c
}

/// Colour and wetness (or opacity) at d. `hs` gives the sea-clamped height elsewhere.
fn albedo_at(k: &recipe::Consts, d: V3, hg: [f32; 2], tap: f32, hs: impl Fn(V3) -> f32) -> [f32; 4] {
    let h = hg[0];
    let seed = [k.seed[0], k.seed[1], k.seed[2]];
    let zxy = [k.seed[2], k.seed[0], k.seed[1]];
    let yzx = [k.seed[1], k.seed[2], k.seed[0]];
    let p = add(scale(d, k.noise[0]), seed);
    let region = smoothstep(-0.12, 0.12, fbm3(add(add(scale(d, k.region[0]), zxy), [5.0; 3]), 0.5) - 0.5) * k.region[1];
    let cn = 1.0 + (fbm3(add(scale(d, k.region[3]), yzx), 0.6) - 0.5) * 2.0 * k.region[2];
    if k.seed[3] > 0.5 {
        let t = 0.5 + 0.5 * (h * std::f32::consts::PI).sin();
        let a = ramp(&k.ramp, t);
        let b = ramp(&k.alt, t);
        let mut c: [f32; 3] = std::array::from_fn(|i| lerp(a[i], b[i], region) * cn);
        let s = saturate(hg[1] * k.storm_c[3]);
        c = std::array::from_fn(|i| lerp(c[i], k.storm_c[i], s));
        let pole = smoothstep(0.62, 0.92, d[1].abs() + (fbm3(add(scale(p, 2.0), [3.0; 3]), 0.5) - 0.5) * 0.2);
        c = std::array::from_fn(|i| lerp(c[i], k.deep[i], pole));
        return [saturate(c[0]), saturate(c[1]), saturate(c[2]), 0.0];
    }
    // Curvature from four taps three texels out.
    let face = cube_face_uv(d).0;
    let (t1, t2) = tangents(d, v3(FACES[face][1]));
    let e = tap * 3.0;
    let sea = k.sea[0];
    let hs0 = h.max(sea);
    let lap = hs(norm(add(d, scale(t1, e)))) + hs(norm(sub(d, scale(t1, e)))) + hs(norm(add(d, scale(t2, e)))) + hs(norm(sub(d, scale(t2, e))))
        - 4.0 * hs0;
    let cav = 1.0 - saturate(lap * k.cavity[0]) + saturate(-lap * k.cavity[1]);
    let a = ramp(&k.ramp, h);
    let b = ramp(&k.alt, h);
    let mut c: [f32; 3] = std::array::from_fn(|i| lerp(a[i], b[i], region) * cn * cav);
    let mut wet = 0.0;
    if sea > -50.0 {
        let depth = sea - h;
        let t = smoothstep(0.0, k.sea[1], depth);
        let life = 1.0 + (vnoise(scale(p, 3.0)) - 0.5) * 0.12;
        let w: [f32; 3] = std::array::from_fn(|i| lerp(k.shallow[i], k.deep[i], t) * life);
        wet = smoothstep(-0.04, 0.04, depth);
        c = std::array::from_fn(|i| lerp(c[i], w[i], wet));
    }
    let lat = d[1].abs() + (fbm3(add(scale(p, 1.7), [9.0; 3]), 0.55) - 0.5) * k.ice[2];
    let ice = smoothstep(k.ice[0], k.ice[0] + k.ice[1], lat) * k.ice[3];
    c = std::array::from_fn(|i| lerp(c[i], k.ice_col[i] * (0.9 + 0.1 * cav), ice));
    wet *= 1.0 - ice;
    if k.cavity[3] > 0.0 {
        wet = 1.0 - smoothstep(k.cavity[3] - 0.06, k.cavity[3] + 0.06, hg[1]);
    }
    [saturate(c[0]), saturate(c[1]), saturate(c[2]), wet]
}

/// Cloud density at d.
pub fn cloud_at(k: &recipe::Consts, d: V3) -> f32 {
    let (c, c2) = (k.cloud, k.cloud2);
    let yzx = [k.seed[1], k.seed[2], k.seed[0]];
    let sp = add(add(scale([d[0], d[1] * c2[0], d[2]], c[0]), yzx), [c2[3]; 3]);
    let q = [
        fbm3(add(sp, [2.1, 7.3, 1.1]), 0.5) - 0.5,
        fbm3(add(sp, [6.2, 1.4, 8.8]), 0.5) - 0.5,
        fbm3(add(sp, [3.3, 4.4, 5.5]), 0.5) - 0.5,
    ];
    let mut n = (fbm7(add(sp, scale(q, c[3] * 2.0)), c2[1]) - 0.5) * 8.0;
    n += c2[2] * (d[1] * 9.42).cos();
    smoothstep(c[1] - c[2], c[1] + c[2], n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_cover_the_sphere_consistently() {
        // Every texel's direction maps back to its own face and coordinates.
        let s = 9;
        for f in 0..6 {
            for j in 0..s {
                for i in 0..s {
                    let d = face_dir(f, s, i, j);
                    let (face, u, v) = cube_face_uv(d);
                    let k = 2.0 / (s - 1) as f32;
                    if (u.abs() - 1.0).abs() > 1e-4 && (v.abs() - 1.0).abs() > 1e-4 {
                        assert_eq!(face, f);
                        assert!((u - (i as f32 * k - 1.0)).abs() < 1e-4 && (v - (j as f32 * k - 1.0)).abs() < 1e-4);
                    }
                }
            }
        }
    }

    #[test]
    fn bakes_a_rock_recipe() {
        let r = Recipe::parse(
            "[Surface]\nKind=rock\nSeed=41\nScale=1.7\nWarp=0.2\nDetail=0.55\nRelief=0.06\nRidges=0.15\n\
             Ramp1=-4 44 42 41\nRamp6=5.0 158 155 150\nCraters=0.75 4 1.6 0.5\nCavity=0.4 0.15\n",
        );
        let s = bake(&r, 32, 16);
        assert_eq!(s.albedo.faces[0].len(), 32 * 32 * 4);
        assert!(s.clouds.is_none());
        // Rock: greyish, not flat, and the relief stays near "straight up" (0.5 + n - d).
        let px: Vec<u8> = s.albedo.faces.iter().flatten().copied().collect();
        let (lo, hi) = px.chunks(4).fold((255u8, 0u8), |(l, h), p| (l.min(p[0]), h.max(p[0])));
        assert!(hi > lo + 20, "colour should vary: {lo}..{hi}");
        let rel: Vec<u8> = s.relief.faces.iter().flatten().copied().collect();
        let mean: f32 = rel.chunks(4).map(|p| p[1] as f32).sum::<f32>() / (rel.len() / 4) as f32;
        assert!((mean - 128.0).abs() < 20.0, "relief offsets centre on 0.5: {mean}");
    }

    #[test]
    fn bake_is_deterministic() {
        let r = Recipe::parse("[Surface]\nKind=rock\nSeed=7\nSea=0\nClouds=3 0.8 1 0.6\n");
        let a = bake(&r, 16, 8);
        let b = bake(&r, 16, 8);
        assert_eq!(a.albedo.faces, b.albedo.faces);
        assert_eq!(a.clouds.unwrap().faces, b.clouds.unwrap().faces);
    }
}
