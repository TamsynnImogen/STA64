//! Surface recipes (`[Surface]` ini files), read as Surfaces.asi and Refit read them
//! (Refit `src/mods/world/surfaces.rs`, `read_recipe`; keys in Refit's
//! `render3d/README.md`, "Recipes").

use std::collections::HashMap;

const DEG: f32 = std::f32::consts::PI / 180.0;

/// The bake's constants (Refit's k_seed .. k_fiss).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Consts {
    /// xyz: the noise domain's offset; w: 1 for a gas giant.
    pub seed: [f32; 4],
    /// scale, warp, detail, ridges
    pub noise: [f32; 4],
    /// sea level (-99: none), shore depth, relief, continents
    pub sea: [f32; 4],
    /// share of cells with a crater, cells across, depth, rim
    pub crater: [f32; 4],
    /// cells across, width, depth, glow
    pub crack: [f32; 4],
    /// latitude it starts at, softness, ragged, amount
    pub ice: [f32; 4],
    pub ice_col: [f32; 4],
    pub deep: [f32; 4],
    pub shallow: [f32; 4],
    /// rgb + the stop's height
    pub ramp: [[f32; 4]; 6],
    pub alt: [[f32; 4]; 6],
    /// region scale, share of the second ramp, colour noise, its scale
    pub region: [f32; 4],
    /// scale, coverage, softness, warp
    pub cloud: [f32; 4],
    /// stretch along longitude, detail, latitude belts, seed
    pub cloud2: [f32; 4],
    /// bands, band warp, turbulence, stretch
    pub gas: [f32; 4],
    /// xyz: direction; w: size (radians)
    pub storm: [f32; 4],
    pub storm_c: [f32; 4],
    /// cavity darkening, ridge lightening, fissure branches, Cut
    pub cavity: [f32; 4],
    /// fissure warp, share of borders kept
    pub fiss: [f32; 4],
}

/// A recipe: the bake's constants and how the result is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Recipe {
    pub k: Consts,
    pub clouds: bool,
    /// Face size asked for (0: the caller's default).
    pub face: u32,
    /// Cloud drift, radians per second. The recipe's `CloudDrift` is degrees per second
    /// (Refit's `surfaces.rs` turns the clouds by `CloudDrift * time` degrees).
    pub drift: f32,
    /// rgb: glow colour; a: how much shows by day.
    pub glow: [f32; 4],
    /// rgb: cloud colour; a: opacity.
    pub cloud_colour: [f32; 4],
    /// Cloud shadow strength and reach.
    pub cloud_shadow: [f32; 2],
    /// Relief strength (`Depth`).
    pub depth: f32,
    /// Cut: alpha-tested model, fissure strength cut through (0: none).
    pub cut: f32,
    /// Triplanar mapping for models (rocks).
    pub triplanar: bool,
    pub gloss: f32,
}

fn keys(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut in_surface = false;
    for line in text.lines() {
        let line = line.split(';').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_surface = line.eq_ignore_ascii_case("[surface]");
            continue;
        }
        if !in_surface {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    out
}

impl Recipe {
    pub fn parse(text: &str) -> Recipe {
        let f = keys(text);
        let vals = |key: &str, n: usize| -> Option<Vec<f32>> {
            let s = f.get(&key.to_ascii_lowercase())?;
            let v: Vec<f32> = s.split([',', ' ', '\t']).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect();
            (v.len() >= n).then(|| v[..n].to_vec())
        };
        let one = |key: &str, def: f32| vals(key, 1).map_or(def, |v| v[0]);
        let four = |key: &str, def: [f32; 4]| match vals(key, 4) {
            Some(v) => ([v[0], v[1], v[2], v[3]], true),
            None => (def, false),
        };
        let rgb = |key: &str, def: [f32; 3]| {
            let v = vals(key, 3).map_or(def, |v| [v[0], v[1], v[2]]);
            [v[0] / 255.0, v[1] / 255.0, v[2] / 255.0, 0.0]
        };
        let ramp = |prefix: &str, r: &mut [[f32; 4]; 6]| -> usize {
            let mut n = 0;
            for i in 0..6 {
                if let Some(v) = vals(&format!("{prefix}{}", i + 1), 4) {
                    r[i] = [v[1] / 255.0, v[2] / 255.0, v[3] / 255.0, v[0]];
                    n += 1;
                } else if i > 0 {
                    r[i] = r[i - 1];
                    r[i][3] += 0.001;
                } else {
                    r[0] = [0.5, 0.5, 0.5, -10.0];
                }
                if i > 0 && r[i][3] <= r[i - 1][3] {
                    r[i][3] = r[i - 1][3] + 0.001;
                }
            }
            n
        };

        let kind = f.get("kind").cloned().unwrap_or_default();
        let seed = one("Seed", 1.0).abs();
        let mut k = Consts {
            seed: [(seed * 12.9898) % 97.0, (seed * 78.233) % 89.0, (seed * 37.719) % 83.0, if kind.eq_ignore_ascii_case("gas") { 1.0 } else { 0.0 }],
            noise: [one("Scale", 2.5), one("Warp", 0.4), one("Detail", 0.5), one("Ridges", 0.0)],
            sea: [one("Sea", -99.0), one("Shore", 1.0).max(0.01), one("Relief", 0.02), one("Continents", 0.0)],
            crater: four("Craters", [0.0, 12.0, 1.0, 0.3]).0,
            crack: four("Fissures", [0.0, 0.05, 1.0, 1.0]).0,
            ice: four("Ice", [0.8, 0.1, 0.15, 0.0]).0,
            ice_col: rgb("IceColour", [235.0, 240.0, 245.0]),
            deep: rgb("Deep", [10.0, 30.0, 60.0]),
            shallow: rgb("Shallow", [30.0, 90.0, 110.0]),
            region: four("Region", [1.5, 0.0, 0.15, 12.0]).0,
            gas: four("Bands", [8.0, 1.5, 0.3, 4.0]).0,
            ..Default::default()
        };
        ramp("Ramp", &mut k.ramp);
        if ramp("Alt", &mut k.alt) == 0 {
            k.alt = k.ramp;
        }
        let (cl, clouds) = four("Clouds", [3.0, 0.8, 1.0, 0.6]);
        k.cloud = cl;
        let cs = four("CloudShape", [2.0, 0.55, 0.5, 0.0]).0;
        k.cloud2 = [cs[0], cs[1], cs[2], (cs[3].abs() * 7.31 + 13.0) % 71.0];
        let st = four("Storm", [0.0, -20.0, 8.0, 0.0]).0;
        let (y, p) = (st[0] * DEG, st[1] * DEG);
        k.storm = [p.cos() * y.sin(), p.sin(), p.cos() * y.cos(), st[2] * DEG];
        let sc = rgb("StormColour", [200.0, 120.0, 90.0]);
        k.storm_c = [sc[0], sc[1], sc[2], st[3]];
        let cav = vals("Cavity", 2).unwrap_or(vec![0.25, 0.15]);
        k.cavity = [cav[0], cav[1], one("Branches", 0.6), one("Cut", 0.0)];
        let fs = vals("FissureShape", 2).unwrap_or(vec![2.5, 0.33]);
        k.fiss = [fs[0], fs[1], 0.0, 0.0];
        let face = one("Face", 0.0) as i32;
        let mut glow = rgb("Glow", [0.0, 0.0, 0.0]);
        glow[3] = one("GlowDay", 0.3);
        let mut cloud_colour = rgb("CloudColour", [255.0, 255.0, 255.0]);
        cloud_colour[3] = if clouds { one("CloudOpacity", 0.9) } else { 0.0 };
        let shadow = vals("CloudShadow", 2).unwrap_or(vec![0.5, 0.012]);
        let cut = k.cavity[3];
        Recipe {
            k,
            clouds,
            face: if face <= 0 { 0 } else { face.clamp(64, 4096) as u32 },
            drift: one("CloudDrift", 0.4).to_radians(),
            glow,
            cloud_colour,
            cloud_shadow: [shadow[0], shadow[1]],
            depth: one("Depth", 1.0),
            cut,
            triplanar: f.get("mapping").is_some_and(|m| m.eq_ignore_ascii_case("triplanar")),
            gloss: one("Gloss", 1.0).max(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_keys_and_defaults() {
        let r = Recipe::parse("; comment\n[Surface]\nKind=gas\nSeed=41\nRamp1=-4  44 42 41\nClouds=3 0.8 1 0.6\nFace=1024\n");
        assert_eq!(r.k.seed[3], 1.0);
        assert!((r.k.seed[0] - (41.0f32 * 12.9898) % 97.0).abs() < 1e-4);
        assert_eq!(r.k.ramp[0], [44.0 / 255.0, 42.0 / 255.0, 41.0 / 255.0, -4.0]);
        assert!(r.k.ramp[1][3] > r.k.ramp[0][3], "missing stops repeat a little higher");
        assert_eq!(r.k.alt, r.k.ramp, "no Alt ramp: the first one");
        assert!(r.clouds && r.face == 1024);
        assert_eq!(r.k.sea[0], -99.0, "no sea by default");
    }

    /// `CloudDrift` is degrees per second (Refit's recipe unit): 0.35 turns the clouds
    /// once in about 17 minutes, not once in 18 seconds.
    #[test]
    fn cloud_drift_is_degrees_per_second() {
        let r = Recipe::parse("[Surface]\nKind=rock\nClouds=3 1 1 0.5\nCloudDrift=0.36\n");
        assert!((r.drift - 0.36f32.to_radians()).abs() < 1e-9);
        let turn_s = std::f32::consts::TAU / r.drift;
        assert!((turn_s - 1000.0).abs() < 0.5, "{turn_s}");
        let d = Recipe::parse("[Surface]\nKind=rock\n");
        assert!((d.drift - 0.4f32.to_radians()).abs() < 1e-9, "default 0.4 degrees a second");
    }
}
