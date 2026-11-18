//! Specular and emissive maps on hulls (RND-38), ported from the A2FOExtensions project
//! (https://github.com/TamsynnImogen/A2FOExtensions), so mods authored for it work unchanged.
//! Evidence grade: ported from the extension's source and documentation; not measured on a
//! running extension.
//!
//! # Declaration (read from the mod's `ART_CFG.h`, layered like any cfg header)
//!
//! `#define A2FO_SPECULAR_SUFFIX "_specular"` and `#define A2FO_EMISSIVE_SUFFIX "_emissive_"`
//! (aliases `SpecularTextureSuffix`, `EmissiveTextureSuffix`). An empty or missing suffix, or one
//! that is not 1 to 64 ASCII letters, digits, `_` or `-`, turns that convention off. Content that
//! declares neither is not looked at: no file is searched for and nothing is drawn differently.
//!
//! # Files (textures, `.dds` then `.tga`, either root, the mounted layers' override order)
//!
//! For a group whose diffuse is `ship`: the specular map is `ship_specular`; the emissive maps
//! are `ship_emissive_warp`, `_impulse`, `_shields`, `_life`, `_sensor`, `_weapons`. A faction
//! variant puts the faction suffix **last** (`ship_specular_b`, `ship_emissive_warp_b`) and
//! falls back to the unsuffixed map when no faction map exists. A missing map adds nothing.
//!
//! # Rules
//!
//! - Specular map: RGB is a mask on the highlight at the diffuse UVs (alpha unused). The
//!   highlight is Blinn-Phong per directional light, the light colour times `(N.H)^power`
//!   (front-facing, `N.L > 0`), times the highlight colour, times the mask, added after texturing.
//!   The colour and power are the lighting material's own when it has a specular colour above
//!   zero, else [`DEFAULT_STRENGTH`] on each channel and [`DEFAULT_POWER`]. Black adds nothing;
//!   white leaves the highlight as the material states it.
//! - Emissive: the six maps of a material are combined, the brightest value per RGB channel at
//!   each texel (the first map found sets the size, the rest are scaled to it, alpha ignored)
//!   into one texture, which is added to the lit colour in display (gamma) values, clipped at 1.
//!   It is self-lit: lights, shadows and the team colour do not touch it.
//!
//! Not ported yet (TODO.md): explicit craft-ODF `textureX` / `emissiveX<System>` declarations,
//! the per-system damage, disable and flicker states and engine-motion levels, the emissive
//! bloom, the graphics-options switches (here `STA64_SPECULAR_MAPS=0` / `STA64_EMISSIVE_MAPS=0`
//! at start), and block-compressed GPU upload of the maps (they are decoded and uploaded as
//! RGBA8).

use bevy::prelude::*;
use sta_assets::{pixels, Rgba};

use crate::render::textures::{upload, Loader};

/// The highlight colour on each channel when the material states none (the extension's
/// `SpecularMapStrength`).
pub const DEFAULT_STRENGTH: f32 = 0.5;
/// The highlight power when the material states none (the extension's `SpecularMapPower`).
pub const DEFAULT_POWER: f32 = 24.0;
/// The system words of an emissive map's name, after the suffix.
pub const EMISSIVE_WORDS: [&str; 6] = ["warp", "impulse", "shields", "life", "sensor", "weapons"];
/// The largest suffix the declaration accepts.
const MAX_SUFFIX: usize = 64;
/// The Borg faction suffix, known without a race declaring it.
pub const BORG_SUFFIX: &str = "_b";

/// The `ART_CFG.h` declarations: a map convention is on when its suffix is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapSuffixes {
    pub specular: Option<String>,
    pub emissive: Option<String>,
}

impl MapSuffixes {
    pub fn from_cfg(cfg: Option<&sta_assets::cfg_h::CfgH>) -> MapSuffixes {
        let Some(cfg) = cfg else { return MapSuffixes::default() };
        let read = |names: [&str; 4]| names.iter().find_map(|n| cfg.raw(n)).and_then(valid_suffix);
        MapSuffixes {
            specular: read(["A2FO_SPECULAR_SUFFIX", "a2fo_specular_suffix", "SpecularTextureSuffix", "speculartexturesuffix"]),
            emissive: read(["A2FO_EMISSIVE_SUFFIX", "a2fo_emissive_suffix", "EmissiveTextureSuffix", "emissivetexturesuffix"]),
        }
    }

    pub fn from_vfs(vfs: &sta_vfs::Vfs) -> MapSuffixes {
        MapSuffixes::from_cfg(sta_content::layered::cfg_h(vfs, "ART_CFG.h").as_ref())
    }

    pub fn any(&self) -> bool {
        self.specular.is_some() || self.emissive.is_some()
    }
}

fn valid_suffix(raw: &str) -> Option<String> {
    let s = raw.trim();
    (!s.is_empty() && s.len() <= MAX_SUFFIX && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')).then(|| s.to_owned())
}

/// The names to look for, most specific first: the faction map (`base + suffix + word + faction`),
/// then the base map. `word` is empty for a specular map.
pub fn names(base: &str, suffix: &str, word: &str, faction: Option<&str>) -> Vec<String> {
    let plain = format!("{base}{suffix}{word}");
    match faction.filter(|f| !f.is_empty()) {
        Some(f) => vec![format!("{plain}{f}"), plain],
        None => vec![plain],
    }
}

/// The user's switches (the extension's `[Effects] SpecularMaps` / `EmissiveMaps`, default on).
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapSwitches {
    pub specular: bool,
    pub emissive: bool,
}

impl Default for MapSwitches {
    fn default() -> Self {
        MapSwitches { specular: true, emissive: true }
    }
}

impl MapSwitches {
    /// `STA64_SPECULAR_MAPS` / `STA64_EMISSIVE_MAPS`: `0` turns one off.
    pub fn from_env() -> MapSwitches {
        let on = |k: &str| std::env::var(k).map_or(true, |v| v.trim() != "0");
        MapSwitches { specular: on("STA64_SPECULAR_MAPS"), emissive: on("STA64_EMISSIVE_MAPS") }
    }
}

/// The maps a hull part carries (on the part entity; absent for content that declares none).
#[derive(Component, Clone, Debug, Default)]
pub struct HullMaps {
    pub specular: Option<Handle<Image>>,
    pub emissive: Option<Handle<Image>>,
}

impl HullMaps {
    pub fn is_empty(&self) -> bool {
        self.specular.is_none() && self.emissive.is_none()
    }
}

impl Loader<'_> {
    /// The maps the mod's declarations give a group whose diffuse is `diffuse` (`faction`: the
    /// faction suffix of the variant drawn). `None` when nothing is declared or no file exists.
    pub fn hull_maps(&mut self, diffuse: &str, faction: Option<&str>) -> Option<HullMaps> {
        let suffixes = MapSuffixes::from_vfs(self.vfs);
        if !suffixes.any() {
            return None;
        }
        self.hull_maps_for(&suffixes, diffuse, faction)
    }

    pub fn hull_maps_for(&mut self, suffixes: &MapSuffixes, diffuse: &str, faction: Option<&str>) -> Option<HullMaps> {
        let base = diffuse.rsplit(['/', '\\']).next().unwrap_or(diffuse);
        let base = base.rsplit_once('.').filter(|(_, e)| ["dds", "tga", "png", "bmp"].iter().any(|x| e.eq_ignore_ascii_case(x))).map_or(base, |(s, _)| s);
        let mut maps = HullMaps::default();
        if let Some(s) = &suffixes.specular {
            maps.specular = self.mask_map(base, s, faction);
        }
        if let Some(s) = &suffixes.emissive {
            maps.emissive = self.emissive_map(base, s, faction);
        }
        (!maps.is_empty()).then_some(maps)
    }

    fn first_pixels(&self, candidates: &[String]) -> Option<Rgba> {
        candidates.iter().find_map(|name| {
            let (path, fmt) = sta_assets::find_texture(self.vfs, name)?;
            sta_assets::decode_rgba(self.vfs, &path, fmt).map_err(|e| warn!("{path}: {e}")).ok()
        })
    }

    fn mask_map(&mut self, base: &str, suffix: &str, faction: Option<&str>) -> Option<Handle<Image>> {
        let key = format!("m:s:{base}:{suffix}:{}", faction.unwrap_or("")).to_ascii_lowercase();
        if let Some(h) = self.cache.map.get(&key) {
            return h.clone();
        }
        let handle = self.first_pixels(&names(base, suffix, "", faction)).map(|px| self.images.add(upload(&pixels::generate_mips(&px), false)));
        self.cache.map.insert(key, handle.clone());
        handle
    }

    fn emissive_map(&mut self, base: &str, suffix: &str, faction: Option<&str>) -> Option<Handle<Image>> {
        let key = format!("m:e:{base}:{suffix}:{}", faction.unwrap_or("")).to_ascii_lowercase();
        if let Some(h) = self.cache.map.get(&key) {
            return h.clone();
        }
        let found: Vec<Rgba> = EMISSIVE_WORDS.iter().filter_map(|w| self.first_pixels(&names(base, suffix, w, faction))).collect();
        let handle = combine_emissive(&found).map(|px| self.images.add(upload(&pixels::generate_mips(&px), true)));
        self.cache.map.insert(key, handle.clone());
        handle
    }
}

/// The emissive maps of a material as one: the first sets the size, the others are scaled to it
/// (nearest texel), the brightest value per RGB channel is kept, alpha is ignored (opaque).
pub fn combine_emissive(maps: &[Rgba]) -> Option<Rgba> {
    let first = maps.first()?;
    let (w, h) = (first.width.max(1), first.height.max(1));
    let mut out = vec![0u8; (w * h * 4) as usize];
    for map in maps {
        if map.width == 0 || map.height == 0 || map.pixels.len() < (map.width * map.height * 4) as usize {
            continue;
        }
        for y in 0..h {
            let sy = (y as u64 * map.height as u64 / h as u64) as u32;
            for x in 0..w {
                let sx = (x as u64 * map.width as u64 / w as u64) as u32;
                let s = ((sy * map.width + sx) * 4) as usize;
                let d = ((y * w + x) * 4) as usize;
                for c in 0..3 {
                    out[d + c] = out[d + c].max(map.pixels[s + c]);
                }
            }
        }
    }
    for px in out.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Some(Rgba { width: w, height: h, has_alpha: false, pixels: out })
}

#[cfg(test)]
/// The rule the hull shader evaluates for one light, written out on the CPU (the tests and
/// the GPU check use it): `light * (N.H)^power * colour * mask`, zero unless `N.L > 0`.
/// `material` is the lighting material's specular colour and power.
pub fn highlight(n: Vec3, l: Vec3, v: Vec3, light: Vec3, material: (Vec3, f32), mask: Vec3) -> Vec3 {
    if n.dot(l) <= 0.0 {
        return Vec3::ZERO;
    }
    let (colour, power) = if material.0.max_element() > 0.0 { material } else { (Vec3::splat(DEFAULT_STRENGTH), DEFAULT_POWER) };
    let h = (l + v).normalize();
    light * n.dot(h).max(0.0).powf(power) * colour * mask
}

#[cfg(test)]
/// The emissive layer on a lit colour: added in display values, clipped at 1.
pub fn add_emissive(lit: Vec3, emissive: Vec3) -> Vec3 {
    (lit + emissive).clamp(Vec3::ZERO, Vec3::ONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(text: &str) -> MapSuffixes {
        MapSuffixes::from_cfg(Some(&sta_assets::cfg_h::CfgH::parse(text)))
    }

    #[test]
    fn the_declaration_turns_each_convention_on() {
        let s = cfg("#define A2FO_SPECULAR_SUFFIX \"_specular\"\n#define A2FO_EMISSIVE_SUFFIX \"_emissive_\"\n");
        assert_eq!((s.specular.as_deref(), s.emissive.as_deref()), (Some("_specular"), Some("_emissive_")));
        let alias = cfg("#define SpecularTextureSuffix \"_spec\"\n");
        assert_eq!((alias.specular.as_deref(), alias.emissive), (Some("_spec"), None));
    }

    #[test]
    fn nothing_declared_or_an_unusable_suffix_is_off() {
        assert!(!MapSuffixes::from_cfg(None).any());
        assert!(!cfg("#define SOMETHING_ELSE 1\n").any());
        assert!(!cfg("#define A2FO_SPECULAR_SUFFIX \"\"\n").any(), "empty disables");
        assert!(!cfg("#define A2FO_SPECULAR_SUFFIX \"_a b\"\n").any(), "a space is not allowed");
        assert!(!cfg("#define A2FO_SPECULAR_SUFFIX \"../x\"\n").any());
        let long = format!("#define A2FO_SPECULAR_SUFFIX \"{}\"\n", "a".repeat(65));
        assert!(!cfg(&long).any(), "over 64 characters");
        let ok = format!("#define A2FO_SPECULAR_SUFFIX \"{}\"\n", "a".repeat(64));
        assert!(cfg(&ok).any());
    }

    #[test]
    fn faction_suffix_goes_last_and_falls_back() {
        assert_eq!(names("ship", "_specular", "", None), ["ship_specular"]);
        assert_eq!(names("ship", "_specular", "", Some("_b")), ["ship_specular_b", "ship_specular"]);
        assert_eq!(names("ship", "_emissive_", "warp", Some("_b")), ["ship_emissive_warp_b", "ship_emissive_warp"]);
        assert_eq!(names("ship", "_emissive_", "warp", Some("")), ["ship_emissive_warp"]);
    }

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Rgba {
        Rgba { width: w, height: h, has_alpha: true, pixels: rgba.repeat((w * h) as usize) }
    }

    #[test]
    fn emissive_maps_combine_by_the_brightest_channel_and_ignore_alpha() {
        let a = solid(2, 2, [200, 10, 0, 7]);
        let b = solid(4, 4, [10, 150, 30, 255]);
        let c = combine_emissive(&[a, b]).unwrap();
        assert_eq!((c.width, c.height), (2, 2), "the first map sets the size");
        assert_eq!(&c.pixels[..4], &[200, 150, 30, 255]);
        assert!(combine_emissive(&[]).is_none());
    }

    #[test]
    fn the_highlight_is_masked_and_falls_back_to_the_defaults() {
        let (n, l, v) = (Vec3::Z, Vec3::Z, Vec3::Z);
        let one = Vec3::ONE;
        // A material with a specular colour: its colour, masked.
        let mat = (Vec3::new(0.0, 0.0, 1.0), 16.0);
        assert_eq!(highlight(n, l, v, one, mat, one), Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(highlight(n, l, v, one, mat, Vec3::ZERO), Vec3::ZERO, "a black map removes it");
        assert_eq!(highlight(n, l, v, one, mat, Vec3::new(1.0, 0.0, 0.0)), Vec3::ZERO, "a red map masks a blue highlight");
        // No material specular: the extension's defaults.
        assert_eq!(highlight(n, l, v, one, (Vec3::ZERO, 8.0), one), Vec3::splat(DEFAULT_STRENGTH));
        // Off-axis and sharp is weaker; a light behind the surface adds nothing.
        let grazing = (Vec3::new(1.0, 0.0, 1.0)).normalize();
        assert!(highlight(n, l, grazing, one, (Vec3::ONE, 64.0), one).x < 0.1);
        assert_eq!(highlight(n, -Vec3::Z, v, one, mat, one), Vec3::ZERO);
    }

    #[test]
    fn emissive_adds_and_clips() {
        assert_eq!(add_emissive(Vec3::splat(0.25), Vec3::new(0.5, 0.0, 1.0)), Vec3::new(0.75, 0.25, 1.0));
        assert_eq!(add_emissive(Vec3::splat(0.25), Vec3::ZERO), Vec3::splat(0.25));
    }

    /// A one-texel 24-bit TGA of the colour (r, g, b).
    fn tga(r: u8, g: u8, b: u8) -> Vec<u8> {
        let mut t = vec![0u8; 18];
        t[2] = 2;
        t[12] = 1;
        t[14] = 1;
        t[16] = 24;
        t[17] = 0x20;
        t.extend_from_slice(&[b, g, r]);
        t
    }

    struct Install(std::path::PathBuf);
    impl Install {
        fn new(tag: &str, art_cfg: Option<&str>, textures: &[(&str, [u8; 3])]) -> Self {
            let root = std::env::temp_dir().join(format!("sta64_mapped_{}_{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let data = root.join("Data");
            std::fs::create_dir_all(data.join("Textures/RGB")).unwrap();
            if let Some(cfg) = art_cfg {
                std::fs::write(data.join("ART_CFG.h"), cfg).unwrap();
            }
            for (name, c) in textures {
                std::fs::write(data.join(format!("Textures/RGB/{name}.tga")), tga(c[0], c[1], c[2])).unwrap();
            }
            Install(root)
        }
        fn maps(&self, diffuse: &str, faction: Option<&str>) -> (Option<HullMaps>, usize, Vec<Option<[u8; 4]>>) {
            let vfs = sta_vfs::Vfs::mount_install(&self.0, None).unwrap();
            let mut cache = crate::render::textures::TextureCache::default();
            let mut images = Assets::<Image>::default();
            let found = {
                let mut loader = Loader { vfs: &vfs, cache: &mut cache, images: &mut images, formats: bevy::image::CompressedImageFormats::NONE };
                loader.hull_maps(diffuse, faction)
            };
            let first_texel = |h: &Option<Handle<Image>>| h.as_ref().and_then(|h| images.get(h)).and_then(|i| i.data.as_ref().map(|d| [d[0], d[1], d[2], d[3]]));
            let texels = found.as_ref().map_or(vec![], |m| vec![first_texel(&m.specular), first_texel(&m.emissive)]);
            (found, cache.map.len(), texels)
        }
    }
    impl Drop for Install {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn content_that_declares_nothing_is_not_looked_at() {
        // The files exist, the mod declares no suffix: no maps, and not even a cache entry.
        let install = Install::new("none", Some("#define SOMETHING 1\n"), &[("ship_specular", [255; 3]), ("ship_emissive_warp", [255; 3])]);
        let (maps, cached, _) = install.maps("ship", None);
        assert!(maps.is_none());
        assert_eq!(cached, 0, "no search, nothing cached");
        let none = Install::new("nocfg", None, &[("ship_specular", [255; 3])]);
        assert!(none.maps("ship", None).0.is_none());
    }

    #[test]
    fn a_declared_specular_map_is_found_with_the_faction_map_first() {
        let cfg = "#define A2FO_SPECULAR_SUFFIX \"_specular\"\n";
        let install = Install::new("spec", Some(cfg), &[("ship_specular", [10, 20, 30]), ("ship_specular_b", [200, 100, 50])]);
        let (maps, _, texels) = install.maps("ship", None);
        assert!(maps.as_ref().unwrap().specular.is_some() && maps.as_ref().unwrap().emissive.is_none());
        assert_eq!(&texels[0].unwrap()[..3], &[10, 20, 30], "the base map");
        let (_, _, faction) = install.maps("Ship.tga", Some("_b"));
        assert_eq!(&faction[0].unwrap()[..3], &[200, 100, 50], "the faction map wins; the extension is dropped from the diffuse name");
        let base_only = Install::new("spec2", Some(cfg), &[("ship_specular", [10, 20, 30])]);
        let (_, _, fallback) = base_only.maps("ship", Some("_b"));
        assert_eq!(&fallback[0].unwrap()[..3], &[10, 20, 30], "no faction map: the base map");
        assert!(base_only.maps("other", None).0.is_none(), "another diffuse has no map");
    }

    #[test]
    fn declared_emissive_maps_are_combined() {
        let cfg = "#define A2FO_EMISSIVE_SUFFIX \"_emissive_\"\n";
        let install = Install::new("emit", Some(cfg), &[("ship_emissive_warp", [200, 0, 0]), ("ship_emissive_weapons", [0, 150, 9])]);
        let (maps, _, texels) = install.maps("ship", None);
        assert!(maps.as_ref().unwrap().emissive.is_some() && maps.as_ref().unwrap().specular.is_none());
        let t = texels[1].unwrap();
        // The upload is sRGB RGBA8: the stored bytes are the brightest per channel, opaque.
        assert_eq!(t, [200, 150, 9, 255]);
    }

    #[test]
    fn switches_default_on() {
        assert_eq!(MapSwitches::default(), MapSwitches { specular: true, emissive: true });
    }
}
