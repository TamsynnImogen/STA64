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
    /// Pulse: how far the core brightens and dims (0: off), its speed (cycles a
    /// second) and how far the speed wanders.
    pub pulse: f32,
    pub pulse_speed: f32,
    pub pulse_wander: f32,
    /// Ring about the core: how far it tints (0: off), where (core profile), its width.
    pub ring: [f32; 3],
    /// Its patches: scale (of the tile), how much of the ring they cover, their
    /// softness, the colours' spread.
    pub ring_patch: [f32; 4],
    /// Seconds for its colours to go round once.
    pub ring_cycle: f32,
    /// Its four colours, in turn.
    pub ring_cols: [[f32; 3]; 4],
    /// Swirl: how far the gas wanders about its place (world units), its phases'
    /// scale (of the tile), the seconds one loop takes.
    pub swirl: f32,
    pub swirl_scale: f32,
    pub swirl_period: f32,
    /// Lightning: flashes a minute per nebula (0: none), a flash's size (of the
    /// reach), its brightness and colour.
    pub lightning: f32,
    pub lightning_size: f32,
    pub lightning_bright: f32,
    pub lightning_colour: [f32; 3],
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
            pulse: one("Pulse", 0.0),
            pulse_speed: one("PulseSpeed", 0.5),
            pulse_wander: one("PulseWander", 0.5),
            ring: [one("Ring", 0.0), one("RingAt", 0.4), one("RingWidth", 0.3).max(0.01)],
            ring_patch: [one("RingScale", 0.35), one("RingPatch", 0.5), one("RingSoftness", 0.15), one("RingSpread", 0.5)],
            ring_cycle: one("RingCycle", 6.0).max(0.1),
            ring_cols: [rgb("RingA"), rgb("RingB"), rgb("RingC"), rgb("RingD")],
            swirl: one("Swirl", 0.0).max(0.0),
            swirl_scale: one("SwirlScale", 1.0),
            swirl_period: one("SwirlPeriod", 40.0).max(1.0),
            lightning: one("Lightning", 0.0),
            lightning_size: one("LightningSize", 0.3),
            lightning_bright: one("LightningBrightness", 1.0),
            lightning_colour: {
                let c = rgb("LightningColour");
                if c.iter().sum::<f32>() <= 0.0 { [1.0, 0.92, 0.8] } else { c }
            },
        }
    }

    /// The pulse's phase (radians) at `sec`: the speed wanders as
    /// `1 + wander sin(0.37 s) cos(0.13 s)`, and this is its integral in closed form,
    /// so it is a function of the time alone, not of how the frames fell.
    pub fn pulse_phase(&self, sec: f32) -> f32 {
        let (s, w) = (sec as f64, self.pulse_wander as f64);
        let wander = 0.5 * w * ((1.0 - (s * 0.5).cos()) / 0.5 + (1.0 - (s * 0.24).cos()) / 0.24);
        (std::f64::consts::TAU * (self.pulse_speed as f64 * (s + wander)).rem_euclid(1.0)) as f32
    }

    /// The ring colours' phase (0..1) at `sec`.
    pub fn ring_phase(&self, sec: f32) -> f32 {
        (sec / self.ring_cycle).rem_euclid(1.0)
    }

    /// The swirl's two circles' phases (radians) at `sec`, the second the other way
    /// round in the shader and 1.618 times as fast.
    pub fn swirl_phases(&self, sec: f32) -> [f32; 2] {
        let t = std::f32::consts::TAU;
        [t * (sec / self.swirl_period).rem_euclid(1.0), t * (sec * 1.618_034 / self.swirl_period).rem_euclid(1.0)]
    }
}

/// A flash's brightness `t` seconds after it starts: three flickers, 65 ms half-lives.
pub fn flicker(t: f32) -> f32 {
    const AT: [f32; 3] = [0.0, 0.12, 0.3];
    const AMP: [f32; 3] = [1.0, 0.6, 0.85];
    let mut s = 0.0;
    for i in 0..3 {
        let u = t - AT[i];
        if (0.0..0.5).contains(&u) {
            s += AMP[i] * (-u * 15.0).exp2();
        }
    }
    s
}

/// How long a flash lasts.
pub const FLASH_SECS: f32 = 0.8;

/// One class's lightning: up to four flashes at once, scheduled as Nebulae.asi's.
#[derive(Debug, Clone)]
pub struct Lightning {
    seed: u32,
    pos: [[f32; 3]; 4],
    t0: [f32; 4],
    amp: [f32; 4],
}

impl Lightning {
    pub fn new(seed: u32) -> Lightning {
        Lightning { seed, pos: [[0.0; 3]; 4], t0: [0.0; 4], amp: [0.0; 4] }
    }

    fn rnd(&mut self) -> f32 {
        self.seed = hash32(self.seed.wrapping_add(0x6d2b_79f5));
        (self.seed >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    /// Advance to `now` (`dt` since the last call, 0 if there was none or it was long
    /// ago): maybe strike a new flash among the nebulae at `centres` (each within
    /// `reach`, `half_height` high), and return the four flashes as xyz and how bright
    /// each is now (0: dark).
    pub fn step(&mut self, r: &NebulaRecipe, centres: &[[f32; 3]], reach: f32, half_height: f32, now: f32, dt: f32) -> [[f32; 4]; 4] {
        let rs = [self.rnd(), self.rnd(), self.rnd(), self.rnd(), self.rnd(), self.rnd()];
        let mut out = [[0.0; 4]; 4];
        for (i, o) in out.iter_mut().enumerate() {
            if self.amp[i] > 0.0 {
                let a = now - self.t0[i];
                if !(0.0..=FLASH_SECS).contains(&a) {
                    self.amp[i] = 0.0;
                } else {
                    let p = self.pos[i];
                    *o = [p[0], p[1], p[2], self.amp[i] * flicker(a)];
                }
            }
        }
        if r.lightning > 0.0
            && !centres.is_empty()
            && dt > 0.0
            && dt <= 0.5
            && rs[0] < r.lightning / 60.0 * centres.len() as f32 * dt
            && let Some(i) = (0..4).find(|&i| self.amp[i] <= 0.0)
        {
            let c = centres[((rs[1] * centres.len() as f32) as usize) % centres.len()];
            let (a, d) = (rs[2] * std::f32::consts::TAU, rs[3].sqrt() * reach * 0.6);
            self.pos[i] = [c[0] + d * a.cos(), c[1] + (rs[4] * 2.0 - 1.0) * half_height * 0.45, c[2] + d * a.sin()];
            self.t0[i] = now;
            self.amp[i] = r.lightning_bright * (0.6 + 0.4 * rs[5]);
        }
        out
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
    fn bundled_recipes_carry_their_extras() {
        let l = NebulaRecipe::parse(include_str!("../nebulae/latinum.ini"));
        assert!(l.pulse > 0.0 && l.ring[0] > 0.0 && l.ring_cols[0] == [70.0 / 255.0, 230.0 / 255.0, 200.0 / 255.0]);
        assert_eq!(l.ring_cycle, 5.0);
        let m7 = NebulaRecipe::parse(include_str!("../nebulae/mnebula7.ini"));
        assert_eq!((m7.lightning, m7.lightning_size, m7.lightning_bright), (7.5, 0.6, 3.0));
        assert!(m7.lightning_colour[0] > 0.99 && m7.pulse == 0.0 && m7.swirl == 0.0);
        let m10 = NebulaRecipe::parse(include_str!("../nebulae/mnebula10.ini"));
        assert_eq!((m10.swirl, m10.swirl_scale, m10.swirl_period), (24.0, 1.0, 30.0));
        // Unset: the defaults, and lightning's colour a warm white.
        let d = NebulaRecipe::parse("");
        assert_eq!((d.pulse, d.ring[0], d.swirl, d.lightning), (0.0, 0.0, 0.0, 0.0));
        assert_eq!(d.lightning_colour, [1.0, 0.92, 0.8]);
    }

    #[test]
    fn pulse_is_the_integral_of_its_wandering_speed() {
        let r = NebulaRecipe::parse("[Nebula]\nPulse=1\nPulseSpeed=0.6\nPulseWander=0.5\n");
        // Numerically integrate speed(t) = s (1 + w sin(0.37 t) cos(0.13 t)).
        let (mut ph, dt) = (0.0f64, 0.001f64);
        let mut t = 0.0f64;
        while t < 20.0 {
            ph += 0.6 * (1.0 + 0.5 * (t * 0.37).sin() * (t * 0.13).cos()) * dt;
            t += dt;
        }
        let want = (ph.rem_euclid(1.0) * std::f64::consts::TAU) as f32;
        assert!((r.pulse_phase(20.0) - want).abs() < 0.01, "{} vs {want}", r.pulse_phase(20.0));
        assert_eq!(r.pulse_phase(0.0), 0.0);
        assert!((0.0..std::f32::consts::TAU).contains(&r.pulse_phase(1234.5)));
    }

    #[test]
    fn ring_and_swirl_phases_go_round_once_a_period() {
        let r = NebulaRecipe::parse("[Nebula]\nRingCycle=5\nSwirlPeriod=30\n");
        assert!((r.ring_phase(2.5) - 0.5).abs() < 1e-6 && r.ring_phase(5.0) < 1e-6);
        let [a, b] = r.swirl_phases(15.0);
        assert!((a - std::f32::consts::PI).abs() < 1e-4);
        assert!((b - (std::f32::consts::TAU * 0.809_017).rem_euclid(std::f32::consts::TAU)).abs() < 1e-3);
    }

    #[test]
    fn flicker_flashes_three_times_and_dies() {
        assert!((flicker(0.0) - 1.0).abs() < 1e-6);
        assert!(flicker(0.07) < 0.5 && flicker(0.12) > 0.55, "second flash");
        assert!(flicker(0.3) > 0.8, "third");
        assert_eq!(flicker(-0.1), 0.0);
        assert!(flicker(0.79) < 0.01 && flicker(0.81) == 0.0);
    }

    #[test]
    fn lightning_strikes_inside_the_field_and_fades() {
        let r = NebulaRecipe::parse("[Nebula]\nLightning=60\nLightningBrightness=2\n");
        let centres = [[100.0, 0.0, 0.0], [-500.0, 10.0, 300.0]];
        let mut l = Lightning::new(7);
        let (mut now, mut seen, mut max_lit) = (0.0f32, 0, 0);
        while now < 30.0 {
            let fl = l.step(&r, &centres, 200.0, 50.0, now, 1.0 / 30.0);
            max_lit = max_lit.max(fl.iter().filter(|f| f[3] > 0.0).count());
            for f in fl.iter().filter(|f| f[3] > 0.0) {
                seen += 1;
                let near = centres.iter().any(|c| {
                    ((f[0] - c[0]).powi(2) + (f[2] - c[2]).powi(2)).sqrt() <= 200.0 * 0.6 + 1e-2 && (f[1] - c[1]).abs() <= 50.0 * 0.45 + 1e-3
                });
                assert!(near, "{f:?}");
                assert!(f[3] <= 2.0 + 1e-4);
            }
            now += 1.0 / 30.0;
        }
        assert!(seen > 10 && (1..=4).contains(&max_lit), "{seen} {max_lit}");
        // Without lightning nothing strikes.
        let off = NebulaRecipe::parse("");
        let mut l2 = Lightning::new(1);
        for i in 0..300 {
            assert!(l2.step(&off, &centres, 200.0, 50.0, i as f32 / 30.0, 1.0 / 30.0).iter().all(|f| f[3] == 0.0));
        }
        // Every flash dies out once the clock passes its life.
        assert!(l.step(&off, &centres, 200.0, 50.0, now + 5.0, 0.0).iter().all(|f| f[3] == 0.0));
    }

    #[test]
    fn lightning_is_deterministic_per_seed() {
        let r = NebulaRecipe::parse("[Nebula]\nLightning=30\n");
        let run = |seed| {
            let mut l = Lightning::new(seed);
            (0..600).map(|i| l.step(&r, &[[0.0; 3]], 100.0, 10.0, i as f32 / 30.0, 1.0 / 30.0)).collect::<Vec<_>>()
        };
        assert_eq!(run(3), run(3));
        assert_ne!(run(3), run(4));
    }

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
