//! Refit's ordnance recipes (`ordnance.ini`, its `[Ordnance.<odf>]` sections): how
//! the special weapons' beams, bolts and cones look, keyed by the ordnance ODF's
//! name. Keys and defaults are those of Refit's `Recipe::load`
//! (`render3d/mods/ordnance/src/fx.rs`). A weapon with no section gets the defaults of
//! its kind, in a colour from its class (see `weapons.rs`).

use bevy::math::Vec3;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Beam,
    Bolt,
    Cone,
}

/// One recipe; Refit's names with its defaults. Colours are linear 0..1.
#[derive(Debug, Clone, PartialEq)]
pub struct Recipe {
    pub shape: Shape,
    pub col: Vec3,
    pub core: Vec3,
    pub bright: f32,
    pub flicker: f32,
    pub flicker_rate: f32,
    /// World units (a beam's width, a cone's narrow end).
    pub width: f32,
    pub core_w: f32,
    pub flow: [f32; 3],
    pub helix: [f32; 3],
    pub crackle: f32,
    /// Half widths: the flare at the emitter and at the impact.
    pub emitter: f32,
    pub impact: f32,
    pub star: f32,
    /// A bolt's radius, tail (radii), rays share and spin.
    pub size: f32,
    pub tail: f32,
    pub rays: f32,
    pub spin: f32,
    pub pulse: f32,
    pub pulse_rate: f32,
    /// Seconds to full brightness.
    pub fade_in: f32,
    /// A cone's wide end in target radii, and how far it runs past the end.
    pub spread: f32,
    pub beyond: f32,
    pub fans: [f32; 3],
    pub edge_px: f32,
    pub vapour: [f32; 3],
    pub fall: f32,
}

impl Recipe {
    pub fn new(shape: Shape) -> Recipe {
        Recipe {
            shape,
            col: rgb([120.0, 200.0, 255.0]),
            core: rgb([235.0, 245.0, 255.0]),
            bright: 1.0,
            flicker: 0.0,
            flicker_rate: 20.0,
            width: 6.0,
            core_w: 0.25,
            flow: [0.0, 0.15, 8.0],
            helix: [0.0, 0.12, 6.0],
            crackle: 0.0,
            emitter: 2.5,
            impact: 4.0,
            star: 0.4,
            size: 5.0,
            tail: 4.0,
            rays: 0.3,
            spin: 3.0,
            pulse: 0.0,
            pulse_rate: 8.0,
            fade_in: 0.15,
            spread: 0.8,
            beyond: 0.5,
            fans: [3.0, 0.3, 0.7],
            edge_px: 2.0,
            vapour: [0.5, 1.2, 0.5],
            fall: 0.25,
        }
    }

    fn set(&mut self, key: &str, v: &[f32]) {
        let one = v.first().copied().unwrap_or(0.0);
        let three = |d: [f32; 3]| [v.first().copied().unwrap_or(d[0]), v.get(1).copied().unwrap_or(d[1]), v.get(2).copied().unwrap_or(d[2])];
        match key {
            "colour" => self.col = rgb(three([0.0; 3])),
            "corecolour" => self.core = rgb(three([0.0; 3])),
            "brightness" => self.bright = one,
            "flicker" => self.flicker = one.clamp(0.0, 1.0),
            "flickerrate" => self.flicker_rate = one,
            "width" => self.width = one.max(0.01),
            "corewidth" => self.core_w = one.max(0.01),
            "flow" => self.flow = three(self.flow),
            "helix" => self.helix = three(self.helix),
            "crackle" => self.crackle = one,
            "emitter" => self.emitter = one,
            "impact" => self.impact = one,
            "star" => self.star = one,
            "size" => self.size = one.max(0.01),
            "tail" => self.tail = one,
            "rays" => self.rays = one,
            "spin" => self.spin = one,
            "pulse" => self.pulse = one,
            "pulserate" => self.pulse_rate = one,
            "fadein" => self.fade_in = one.max(0.001),
            "spread" => self.spread = one.max(0.01),
            "fans" => self.fans = three(self.fans),
            "vapour" => self.vapour = three(self.vapour),
            "fall" => self.fall = one.max(0.0),
            _ => {}
        }
    }

    /// The deterministic flicker: 1 down to `1 - flicker`, at `flicker_rate`.
    pub fn flicker_at(&self, now: f32, seed: f32) -> f32 {
        1.0 - self.flicker * (0.5 + 0.5 * (now * self.flicker_rate + seed * 17.0).sin())
    }
}

fn rgb(c: [f32; 3]) -> Vec3 {
    Vec3::from(c) / 255.0
}

/// Parse `ordnance.ini`: ODF name (lower case) to its recipe.
pub fn parse(text: &str) -> BTreeMap<String, Recipe> {
    let mut out = BTreeMap::new();
    let mut cur: Option<(String, Recipe)> = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(';')) {
        if let Some(sec) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if let Some((n, r)) = cur.take() {
                out.insert(n, r);
            }
            cur = sec.strip_prefix("Ordnance.").map(|n| (n.to_ascii_lowercase(), Recipe::new(Shape::Beam)));
        } else if let (Some((_, r)), Some((k, v))) = (cur.as_mut(), line.split_once('=')) {
            let key = k.trim().to_ascii_lowercase();
            if key == "kind" {
                let v = v.trim().to_ascii_lowercase();
                r.shape = if v.starts_with("bo") {
                    Shape::Bolt
                } else if v.starts_with('c') {
                    Shape::Cone
                } else {
                    Shape::Beam
                };
            } else {
                let nums: Vec<f32> = v.split(',').filter_map(|t| t.trim().parse().ok()).collect();
                r.set(&key, &nums);
            }
        }
    }
    if let Some((n, r)) = cur {
        out.insert(n, r);
    }
    out
}

/// Refit's recipes, parsed once.
pub fn bundled() -> &'static BTreeMap<String, Recipe> {
    static R: std::sync::OnceLock<BTreeMap<String, Recipe>> = std::sync::OnceLock::new();
    R.get_or_init(|| parse(include_str!("ordnance.ini")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_recipes_parse_with_their_kinds() {
        let r = bundled();
        assert_eq!(r.len(), 30);
        assert_eq!(r["tracbm"].shape, Shape::Cone);
        assert_eq!(r["tracbm"].fans, [4.0, 0.45, 1.1]);
        assert_eq!(r["ionstm"].shape, Shape::Bolt);
        assert_eq!(r["ionstm"].size, 8.0);
        assert_eq!(r["accelcannon"].shape, Shape::Beam);
        assert!((r["accelcannon"].width - 10.5).abs() < 1e-6);
        assert!((r["accelcannon"].col.x - 1.0).abs() < 1e-6);
    }
}
