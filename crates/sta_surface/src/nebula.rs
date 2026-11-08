//! Nebula gas: recipes, the tileable noise volume and a class's envelope.
//!
//! Ported from Armada II Refit's `render3d/src/mods/world/nebulae.rs` (MIT, Copyright
//! (c) 2026 Cedric Haase), itself Nebulae.asi: the recipe keys and their mapping to
//! the shader's constants, the spectral noise volume (an FFT of shaped random phases,
//! tileable on every axis), and the envelope over the map plane built from where a
//! class's nebulae sit (domes, a wider reach for filaments, their heights, cover).

const DEG: f32 = std::f32::consts::PI / 180.0;

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A `[Nebula]` recipe.
#[derive(Debug, Clone, PartialEq)]
pub struct NebulaRecipe {
    /// Radius override (0: the ODF's effectRadius).
    pub radius: f32,
    pub near_fade: f32,
    pub gas_a: [f32; 3],
    pub gas_b: [f32; 3],
    pub glow: [f32; 3],
    pub brightness: f32,
    pub hue_scale: f32,
    pub gamma: f32,
    pub inv_tile: f32,
    pub warp: f32,
    pub coverage: f32,
    pub softness: f32,
    pub seed: [f32; 3],
    pub detail: f32,
    pub ridge: f32,
    pub hue_mix: f32,
    pub warp_scale: f32,
    pub lumps: f32,
    pub knots: [f32; 4],
    /// Towards the gas's own shading light, times the shade step; w: shading.
    pub light: [f32; 4],
    pub fine_detail: f32,
    /// Flow: how far the gas churns (world units), its scale and speed.
    pub flow: f32,
    pub flow_scale: f32,
    pub flow_speed: f32,
    pub edge_warp: f32,
    pub filaments: [f32; 3],
    pub fil_reach: f32,
    pub fringe: [f32; 2],
    pub fringe_reach: f32,
    pub fuzz: f32,
    pub core: [f32; 2],
    pub key_share: f32,
    pub scatter: f32,
    pub opacity: f32,
    pub edge_dim: f32,
    pub extent: f32,
    pub height: f32,
}

impl NebulaRecipe {
    pub fn parse(text: &str) -> NebulaRecipe {
        let mut keys = std::collections::HashMap::new();
        let mut on = false;
        for line in text.lines() {
            let line = line.split(';').next().unwrap_or("").trim();
            if line.starts_with('[') {
                on = line.eq_ignore_ascii_case("[nebula]");
                continue;
            }
            if let (true, Some((k, v))) = (on, line.split_once('=')) {
                keys.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
            }
        }
        let vals = |key: &str| -> Vec<f32> {
            keys.get(&key.to_ascii_lowercase())
                .map(|s| s.split([',', ' ', '\t']).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect())
                .unwrap_or_default()
        };
        let one = |key: &str, def: f32| vals(key).first().copied().unwrap_or(def);
        let rgb = |key: &str| {
            let v = vals(key);
            if v.len() >= 3 { [v[0] / 255.0, v[1] / 255.0, v[2] / 255.0] } else { [0.0; 3] }
        };
        let seed = one("Seed", 1.0).abs();
        let l = {
            let v = vals("Light");
            if v.len() >= 2 { [v[0], v[1]] } else { [30.0, 60.0] }
        };
        let st = one("ShadeStep", 60.0);
        let (y, p) = (l[0] * DEG, l[1] * DEG);
        NebulaRecipe {
            radius: one("Radius", 0.0),
            near_fade: one("NearFade", 60.0),
            gas_a: rgb("GasA"),
            gas_b: rgb("GasB"),
            glow: rgb("Glow"),
            brightness: one("Brightness", 1.0),
            hue_scale: one("HueScale", 2.0),
            gamma: one("Gamma", 1.0).max(0.05),
            inv_tile: 1.0 / one("Tile", 900.0),
            warp: one("Warp", 0.35),
            coverage: one("Coverage", 0.0),
            softness: one("Softness", 2.5).max(0.01),
            seed: [(seed * 0.129898) % 1.0, (seed * 0.78233) % 1.0, (seed * 0.37719) % 1.0],
            detail: one("Detail", 0.3),
            ridge: one("Ridge", 0.0),
            hue_mix: one("HueMix", 0.4),
            warp_scale: one("WarpScale", 0.45),
            lumps: one("Lumps", 0.35),
            knots: [one("Knots", 1.5), one("KnotScale", 0.35), one("Lanes", 0.5), one("LaneScale", 1.7)],
            light: [p.cos() * y.sin() * st, p.sin() * st, p.cos() * y.cos() * st, one("Shading", 1.0)],
            fine_detail: one("FineDetail", 0.35),
            flow: one("Flow", 60.0),
            flow_scale: one("FlowScale", 2.0),
            flow_speed: one("FlowSpeed", 0.03),
            edge_warp: one("EdgeWarp", 0.35),
            filaments: [one("Filaments", 0.0), one("FilamentScale", 0.6), one("FilamentSharpness", 3.0)],
            fil_reach: one("FilamentReach", 1.8),
            fringe: [one("Fringe", 0.0), one("FringeScale", 0.5)],
            fringe_reach: one("FringeReach", 1.6),
            fuzz: one("Fuzz", 0.0),
            core: [one("Core", 0.0), one("CoreSharpness", 4.0)],
            key_share: one("KeyShare", -1.0).min(1.0),
            scatter: one("Scatter", -1.0).min(0.8),
            opacity: one("Opacity", 0.0).max(0.0),
            edge_dim: one("EdgeOnDim", 0.5),
            extent: one("Extent", 1.3),
            height: one("Height", 3.0),
        }
    }
}

// ---- the noise volume ------------------------------------------------------------

fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

struct Fft {
    tw_c: Vec<f32>,
    tw_s: Vec<f32>,
}

impl Fft {
    fn new(n: usize) -> Fft {
        let (tw_c, tw_s) = (0..n / 2)
            .map(|k| {
                let a = std::f32::consts::TAU * k as f32 / n as f32;
                (a.cos(), a.sin())
            })
            .unzip();
        Fft { tw_c, tw_s }
    }

    /// One line of n (a power of two), in place, radix 2.
    fn line(&self, re: &mut [f32], im: &mut [f32]) {
        let n = re.len();
        let mut j = 0usize;
        for i in 1..n {
            let mut b = n >> 1;
            while j & b != 0 {
                j ^= b;
                b >>= 1;
            }
            j ^= b;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let (h, step) = (len / 2, n / len);
            let mut i = 0;
            while i < n {
                for j in 0..h {
                    let (wr, wi) = (self.tw_c[j * step], self.tw_s[j * step]);
                    let (a, b) = (i + j, i + j + h);
                    let vr = re[b] * wr - im[b] * wi;
                    let vi = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - vr;
                    im[b] = im[a] - vi;
                    re[a] += vr;
                    im[a] += vi;
                }
                i += len;
            }
            len <<= 1;
        }
    }

    fn volume(&self, re: &mut [f32], im: &mut [f32], s: usize) {
        for a in 0..s * s {
            self.line(&mut re[a * s..a * s + s], &mut im[a * s..a * s + s]);
        }
        let mut lr = vec![0.0f32; s];
        let mut li = vec![0.0f32; s];
        for ax in 1..3 {
            let stride = if ax == 1 { s } else { s * s };
            for a in 0..s {
                for b in 0..s {
                    let base = if ax == 1 { a * s * s + b } else { a * s + b };
                    for k in 0..s {
                        lr[k] = re[base + k * stride];
                        li[k] = im[base + k * stride];
                    }
                    self.line(&mut lr, &mut li);
                    for k in 0..s {
                        re[base + k * stride] = lr[k];
                        im[base + k * stride] = li[k];
                    }
                }
            }
        }
    }
}

/// Random phases on a shaped spectrum (k^-2.5 between k0 and k1), transformed: a
/// field that tiles on every axis.
fn spectrum(fft: &Fft, re: &mut [f32], im: &mut [f32], s: usize, k0: f32, k1: f32, seed: u32) {
    let n2 = 3 * (s / 2) * (s / 2) + 1;
    let amp: Vec<f32> = (0..n2)
        .map(|i| {
            let k = (i as f32).sqrt();
            if k > 0.5 && k < k1 * 1.6 { k.powf(-2.5) * smooth(k0 * 0.6, k0, k) * (1.0 - smooth(k1, k1 * 1.6, k)) } else { 0.0 }
        })
        .collect();
    let h = s as i32 / 2;
    let f = |v: usize| if (v as i32) < h { v as i32 } else { v as i32 - s as i32 };
    for z in 0..s {
        for y in 0..s {
            for x in 0..s {
                let (kx, ky, kz) = (f(x), f(y), f(z));
                let a = amp[(kx * kx + ky * ky + kz * kz) as usize];
                let i = (z * s + y) * s + x;
                if a > 0.0 {
                    let ph = (hash32((i as u32).wrapping_mul(0x9e37_79b9) ^ seed) >> 22) as f32 / 1024.0 * std::f32::consts::TAU;
                    re[i] = a * ph.cos();
                    im[i] = a * ph.sin();
                } else {
                    re[i] = 0.0;
                    im[i] = 0.0;
                }
            }
        }
    }
    fft.volume(re, im, s);
}

/// A field into one byte of each texel, normalised to mean 0.5 and the given spread.
fn channel_out(dst: &mut [u8], v: &[f32], byte: usize, want: f32) {
    let n = v.len();
    let (sum, sq) = v.iter().fold((0f64, 0f64), |(s, q), &x| (s + x as f64, q + x as f64 * x as f64));
    let mean = (sum / n as f64) as f32;
    let sd = ((sq / n as f64 - (sum / n as f64).powi(2)) as f32).sqrt().max(1e-6);
    for (i, x) in v.iter().enumerate() {
        let b = ((0.5 + (x - mean) / sd * want) * 255.0 + 0.5) as i32;
        dst[i * 4 + byte] = b.clamp(0, 255) as u8;
    }
}

/// The noise volume, `s`^3 RGBA8 (s a power of two), tiling on every axis: r the gas
/// field, g, b, a slow warp fields. Level 0 only.
pub fn noise_volume(s: usize) -> Vec<u8> {
    let n = s * s * s;
    let fft = Fft::new(s);
    let mut out = vec![0u8; n * 4];
    let (mut re, mut im) = (vec![0.0f32; n], vec![0.0f32; n]);
    spectrum(&fft, &mut re, &mut im, s, 2.0, s as f32 / 4.0, 0x51f1_5e5d);
    channel_out(&mut out, &re, 0, 0.125);
    spectrum(&fft, &mut re, &mut im, s, 1.0, 4.0, 0x2545_f491);
    channel_out(&mut out, &re, 1, 0.2);
    channel_out(&mut out, &im, 2, 0.2);
    spectrum(&fft, &mut re, &mut im, s, 1.0, 4.0, 0x6a09_e667);
    channel_out(&mut out, &re, 3, 0.2);
    out
}

/// Box-filtered levels of a cubic RGBA8 volume, level 0 first.
pub fn volume_mips(s: usize, level0: Vec<u8>) -> Vec<Vec<u8>> {
    let mut levels = vec![level0];
    let mut n = s;
    while n > 1 {
        let h = n / 2;
        let src = levels.last().unwrap();
        let mut nxt = vec![0u8; h * h * h * 4];
        for z in 0..h {
            for y in 0..h {
                for x in 0..h {
                    for c in 0..4 {
                        let mut sum = 0u32;
                        for dz in 0..2 {
                            for dy in 0..2 {
                                for dx in 0..2 {
                                    sum += src[(((2 * z + dz) * n + (2 * y + dy)) * n + (2 * x + dx)) * 4 + c] as u32;
                                }
                            }
                        }
                        nxt[((z * h + y) * h + x) * 4 + c] = ((sum + 4) / 8) as u8;
                    }
                }
            }
        }
        levels.push(nxt);
        n = h;
    }
    levels
}

// ---- the envelope ----------------------------------------------------------------

/// A class's envelope over the map plane: RGBA8, r the domes (1 at a centre, 0 at the
/// reach), g the wider reach of filaments and fringe, b each place's height offset
/// (0.5 = the class's mean height, over +-`ydev`), a the cover.
pub struct Envelope {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
    /// x0, z0, x1, z1 of the envelope.
    pub rect: [f32; 4],
    /// The class's mean height and its spread.
    pub ey: f32,
    pub ydev: f32,
}

/// Build the envelope for nebulae at `pos` (x, y, z) with `radius`.
pub fn envelope(r: &NebulaRecipe, radius: f32, pos: &[[f32; 3]]) -> Envelope {
    let reach = radius * r.extent;
    let mut freach = reach * if r.filaments[0] > 0.0 && r.fil_reach > 1.0 { r.fil_reach } else { 1.0 };
    if r.fringe[0] > 0.0 && reach * r.fringe_reach > freach {
        freach = reach * r.fringe_reach;
    }
    let outer = freach + reach * r.edge_warp;
    let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut ys = 0.0;
    for p in pos {
        x0 = x0.min(p[0] - outer);
        x1 = x1.max(p[0] + outer);
        z0 = z0.min(p[2] - outer);
        z1 = z1.max(p[2] + outer);
        ys += p[1];
    }
    let ey = ys / pos.len().max(1) as f32;
    let ydev = pos.iter().fold(1.0f32, |m, p| m.max((p[1] - ey).abs()));
    const ENV_MAX: u32 = 1024;
    let cell = radius / 16.0;
    let w = (((x1 - x0) / cell) as u32 + 2).clamp(4, ENV_MAX);
    let h = (((z1 - z0) / cell) as u32 + 2).clamp(4, ENV_MAX);
    let mut px = vec![0u8; (w * h * 4) as usize];
    for z in 0..h {
        let wz = z0 + (z as f32 + 0.5) * (z1 - z0) / h as f32;
        for x in 0..w {
            let wx = x0 + (x as f32 + 0.5) * (x1 - x0) / w as f32;
            let (mut s, mut hh, mut g, mut wy, mut ww) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for p in pos {
                let (dx, dz) = (wx - p[0], wz - p[2]);
                let d2 = dx * dx + dz * dz;
                if d2 >= freach * freach {
                    continue;
                }
                let d = d2.sqrt();
                g += smooth(1.0, 0.3, d / freach);
                let q = smooth(1.0, 0.0, d / freach) + 1e-4;
                wy += q * (p[1] - ey);
                ww += q;
                if d >= reach {
                    continue;
                }
                s += smooth(1.0, 0.6, d / reach);
                let q = d / reach;
                hh = hh.max(1.0 - q * q * q);
            }
            let o = ((z * w + x) * 4) as usize;
            px[o] = (hh * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            px[o + 1] = (g.min(1.0) * 255.0 + 0.5) as u8;
            px[o + 2] = ((0.5 + 0.5 * if ww > 0.0 { wy / ww } else { 0.0 } / ydev) * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            px[o + 3] = (s.min(1.0) * 255.0 + 0.5) as u8;
        }
    }
    Envelope {
        w,
        h,
        pixels: px,
        rect: [x0, z0, x1, z1],
        ey,
        ydev,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_tiles_and_is_centred() {
        let s = 16;
        let v = noise_volume(s);
        let mean: f32 = v.chunks(4).map(|p| p[0] as f32).sum::<f32>() / (s * s * s) as f32;
        assert!((mean - 127.5).abs() < 3.0, "{mean}");
        // Tiling: the field's last column continues into its first (no seam) — the
        // FFT makes it periodic, so neighbours across the wrap differ like any pair.
        let at = |x: usize, y: usize, z: usize| v[((z * s + y) * s + x) * 4] as i32;
        let across: i32 = (0..s).map(|y| (at(s - 1, y, 3) - at(0, y, 3)).abs()).sum();
        let inside: i32 = (0..s).map(|y| (at(5, y, 3) - at(6, y, 3)).abs()).sum();
        assert!(across < inside * 3 + 64, "seam {across} vs {inside}");
        assert_eq!(volume_mips(s, v).len(), 5);
    }

    #[test]
    fn envelope_covers_its_nebulae() {
        let r = NebulaRecipe::parse("[Nebula]\nExtent=1.3\nHeight=3\n");
        let e = envelope(&r, 300.0, &[[0.0, 0.0, 0.0], [600.0, 50.0, 0.0]]);
        let at = |wx: f32, wz: f32| {
            let x = ((wx - e.rect[0]) / (e.rect[2] - e.rect[0]) * e.w as f32) as u32;
            let z = ((wz - e.rect[1]) / (e.rect[3] - e.rect[1]) * e.h as f32) as u32;
            let o = ((z * e.w + x) * 4) as usize;
            [e.pixels[o], e.pixels[o + 1], e.pixels[o + 2], e.pixels[o + 3]]
        };
        assert!(at(0.0, 0.0)[3] > 200 && at(0.0, 0.0)[0] > 200, "centre covered");
        assert_eq!(at(300.0, 500.0)[3], 0, "beyond every reach is empty");
        assert!((e.ey - 25.0).abs() < 1e-3 && (e.ydev - 25.0).abs() < 1e-3);
    }

    #[test]
    fn reads_recipe() {
        let r = NebulaRecipe::parse("; c\n[Nebula]\nSeed=3\nTile=900\nGasA=150,50,210\nLight=30,60\nShadeStep=60\nOpacity=2\n");
        assert!((r.inv_tile - 1.0 / 900.0).abs() < 1e-9);
        assert_eq!(r.gas_a, [150.0 / 255.0, 50.0 / 255.0, 210.0 / 255.0]);
        assert!((r.light[1] - (60.0f32 * DEG).sin() * 60.0).abs() < 1e-3);
        assert_eq!(r.opacity, 2.0);
        assert_eq!(r.key_share, -1.0, "unset: the global default");
    }
}
