//! The bake's noise and small vector maths (Refit's world_surface.hlsl, after
//! sky.hlsl): a sine-free lattice hash, quintic value noise, fBm and ridged sums,
//! crater lattices and a Voronoi fissure net.

pub type V3 = [f32; 3];

pub fn v3(a: [f32; 3]) -> V3 {
    a
}
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 0.0 { scale(a, 1.0 / l) } else { a }
}
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
pub fn saturate(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = saturate((x - e0) / (e1 - e0));
    t * t * (3.0 - 2.0 * t)
}
fn step(edge: f32, x: f32) -> f32 {
    if x >= edge { 1.0 } else { 0.0 }
}
fn frac(x: f32) -> f32 {
    x - x.floor()
}

pub fn hash13(p: V3) -> f32 {
    let mut p = [
        frac(p[0] * 0.1373 + 0.27),
        frac(p[1] * 0.1171 + 0.61),
        frac(p[2] * 0.1559 + 0.43),
    ];
    let yzx = [p[1], p[2], p[0]];
    let d = dot(p, [yzx[0] * 7.31 + 19.73, yzx[1] * 7.31 + 19.73, yzx[2] * 7.31 + 19.73]);
    p = [p[0] + d, p[1] + d, p[2] + d];
    frac(p[0] * p[1] * 11.0 + p[2] * (p[0] + 3.7))
}

pub fn hash33(p: V3) -> V3 {
    [hash13(p), hash13(add(p, [17.3, 5.1, 9.7])), hash13(add(p, [3.9, 23.1, 13.3]))]
}

pub fn vnoise(p: V3) -> f32 {
    let i = [p[0].floor(), p[1].floor(), p[2].floor()];
    let f = sub(p, i);
    let u: V3 = std::array::from_fn(|k| f[k] * f[k] * f[k] * (f[k] * (f[k] * 6.0 - 15.0) + 10.0));
    let h = |x: f32, y: f32, z: f32| hash13(add(i, [x, y, z]));
    let (a, b, c, d) = (h(0.0, 0.0, 0.0), h(1.0, 0.0, 0.0), h(0.0, 1.0, 0.0), h(1.0, 1.0, 0.0));
    let (e, g, hh, k) = (h(0.0, 0.0, 1.0), h(1.0, 0.0, 1.0), h(0.0, 1.0, 1.0), h(1.0, 1.0, 1.0));
    lerp(
        lerp(lerp(a, b, u[0]), lerp(c, d, u[0]), u[1]),
        lerp(lerp(e, g, u[0]), lerp(hh, k, u[0]), u[1]),
        u[2],
    )
}

/// HLSL's `mul(k_rot, p)` with the row-major rotation.
fn rot(p: V3) -> V3 {
    [
        0.00 * p[0] + 0.80 * p[1] + 0.60 * p[2],
        -0.80 * p[0] + 0.36 * p[1] - 0.48 * p[2],
        -0.60 * p[0] - 0.48 * p[1] + 0.64 * p[2],
    ]
}

fn fbm(mut p: V3, gain: f32, octaves: usize) -> f32 {
    let (mut s, mut a, mut n) = (0.0, 1.0, 0.0);
    for _ in 0..octaves {
        s += a * vnoise(p);
        n += a;
        a *= gain;
        p = scale(rot(p), 2.03);
    }
    s / n
}

pub fn fbm3(p: V3, gain: f32) -> f32 {
    fbm(p, gain, 3)
}

pub fn fbm7(p: V3, gain: f32) -> f32 {
    fbm(p, gain, 7)
}

/// Ridged noise: sharp crests, each octave weighted by the one before.
pub fn ridged(mut p: V3) -> f32 {
    let (mut s, mut a, mut w, mut n) = (0.0, 0.5, 1.0, 0.0);
    for _ in 0..6 {
        let mut r = 1.0 - (vnoise(p) * 2.0 - 1.0).abs();
        r *= r;
        s += a * r * w;
        n += a;
        w = saturate(r * 1.6);
        a *= 0.55;
        p = scale(rot(p), 2.07);
    }
    s / n
}

/// One lattice of craters, `cells` across: bowl, raised rim, faint apron.
pub fn craters(d: V3, cells: f32, share: f32, depth: f32, rim: f32, off: V3) -> f32 {
    let q = add(scale(d, cells), off);
    let iq = [q[0].floor(), q[1].floor(), q[2].floor()];
    let mut h = 0.0;
    for z in -1..=1 {
        for y in -1..=1 {
            for x in -1..=1 {
                let c = add(iq, [x as f32, y as f32, z as f32]);
                let r3 = hash33(c);
                let on = step(r3[0], share);
                if on == 0.0 {
                    continue;
                }
                let j = hash33(add(c, [7.7; 3]));
                let ctr = add(add(c, [0.5; 3]), scale(sub(j, [0.5; 3]), 0.5));
                let rad = 0.1 + 0.2 * r3[1] * r3[1];
                let t = dot(sub(q, ctr), sub(q, ctr)).sqrt() / rad;
                let bowl = (t * t - 1.0) * step(t, 1.0) * (1.0 - 0.5 * r3[2]);
                let lip = (-(t - 1.0) * (t - 1.0) * 14.0).exp() * rim;
                let apron = (-(t - 1.0).max(0.0) * 3.0).exp() * 0.08 * step(1.0, t);
                h += (bowl + lip + apron) * depth * (0.5 + rad * 2.5);
            }
        }
    }
    h
}

/// The fissure net: distance to the nearest Voronoi border and a number per border.
pub fn fissures(p: V3) -> [f32; 2] {
    let ip = [p[0].floor(), p[1].floor(), p[2].floor()];
    let (mut d1, mut d2, mut i1, mut i2) = (8.0f32, 8.0f32, 0.0f32, 0.0f32);
    for z in -1..=1 {
        for y in -1..=1 {
            for x in -1..=1 {
                let c = add(ip, [x as f32, y as f32, z as f32]);
                let r = sub(add(c, hash33(c)), p);
                let dd = dot(r, r);
                let id = hash13(add(c, [31.0, 17.0, 5.0]));
                if dd <= d1 {
                    d2 = d1;
                    i2 = i1;
                    d1 = dd;
                    i1 = id;
                } else if dd <= d2 {
                    d2 = dd;
                    i2 = id;
                }
            }
        }
    }
    [d2.sqrt() - d1.sqrt(), frac(i1 + i2)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_ranges() {
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for i in 0..2000 {
            let p = [i as f32 * 0.37, i as f32 * 0.11 - 4.0, (i % 97) as f32 * 0.53];
            let n = vnoise(p);
            lo = lo.min(n);
            hi = hi.max(n);
            assert!((0.0..=1.0).contains(&hash13(p)));
        }
        assert!(lo >= 0.0 && hi <= 1.0 && hi - lo > 0.5, "{lo}..{hi}");
    }
}
