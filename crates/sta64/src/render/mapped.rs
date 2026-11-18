//! Specular and emissive maps on hulls (RND-38), ported from the A2FOExtensions project
//! (https://github.com/TamsynnImogen/A2FOExtensions), so mods authored for it work unchanged
//! where the rules below are the extension's. Implements ADR 0004 D2 (the extension's content
//! conventions are adapted, none of its code runs), with D1, D3 and D11.
//!
//! Evidence grade: the declaration, the file names, the faction fallback, the file formats and
//! the emissive combine are ported from the extension's source and documentation. The highlight
//! equation is read from the comments of her renderer interface header and from her scene code;
//! the renderer's own shader is in neither repository, so the arithmetic is this port's. None of
//! it was measured on a running extension.
//!
//! # Declaration (the mod's `ART_CFG.h`)
//!
//! `#define A2FO_SPECULAR_SUFFIX "_specular"` and `#define A2FO_EMISSIVE_SUFFIX "_emissive_"`
//! (aliases `SpecularTextureSuffix`, `EmissiveTextureSuffix`). Like the extension, each setting
//! is merged separately over every copy of the file, the base data first, then parent mods, then
//! the active mod: a later copy overrides, an empty value turns the convention off, a value that
//! is not 1 to 64 ASCII letters, digits, `_` or `-` is ignored and keeps what was inherited.
//! (The game itself only merges copies a file `#include`s; the extension does not wait for that.)
//! Content that declares neither is never searched.
//!
//! # Files (`.dds`, `.tga`, `.png`, `.bmp` in that order, under textures, textures/rgb,
//! textures/index8 and textures/compressed, the mounted layers' override order)
//!
//! For a group whose diffuse is `ship`: the specular map is `ship_specular`; the emissive maps are
//! `ship_emissive_warp`, `_impulse`, `_shields`, `_life`, `_sensor`, `_weapons`. A faction variant
//! puts the faction suffix **last** (`ship_specular_b`) and falls back to the base map; a diffuse
//! that already ends in a faction suffix (`ship_b`) is reduced to its base first, so the order is
//! never `ship_b_specular`. A specular map over 32 MB is refused with a warning, as the
//! extension's loader does.
//!
//! # Rules
//!
//! - Specular map: RGB masks the highlight at the diffuse UVs (alpha unused). The highlight is
//!   Blinn-Phong per directional light, light colour x (N.H)^power, front facing, N.L > 0, added
//!   after texturing, times the mask. A Phong lighting material (model 2) always supplies its own
//!   specular colour and power, where a zero colour or a zero power means no highlight; any other
//!   part uses [`DEFAULT_STRENGTH`] on each channel and [`DEFAULT_POWER`].
//! - Emissive: the maps of a material are combined, the brightest value per RGB channel at each
//!   texel (the first sets the size, the rest are scaled to it, alpha ignored), and added to the
//!   lit colour in display (gamma) values, clipped at 1. Self-lit: lights, shadows and the team
//!   colour do not touch it. A lone map is used as authored (its mips kept).
//!
//! The extension's default for specular is a legacy flat overlay on bumped materials only; the
//! per-pixel rule ported here is the one of its Refit renderer path, which a mod author on the
//! defaults may never have seen. See the PR body and TODO.md for every difference.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageType;
use bevy::prelude::*;
use sta_assets::dds::DdsKind;
use sta_assets::{pixels, Rgba, TextureFormat as Fmt};

use crate::render::stock::hulls::StockLight;
use crate::render::textures::{repeat_sampler, upload, Loader};

/// The highlight colour on each channel of a part that is not Phong (the extension's
/// `SpecularMapStrength`).
pub const DEFAULT_STRENGTH: f32 = 0.5;
/// The highlight power of a part that is not Phong (the extension's `SpecularMapPower`).
pub const DEFAULT_POWER: f32 = 24.0;
/// The system words of an emissive map's name, after the suffix.
pub const EMISSIVE_WORDS: [&str; 6] = ["warp", "impulse", "shields", "life", "sensor", "weapons"];
/// The largest suffix the declaration accepts.
const MAX_SUFFIX: usize = 64;
/// The Borg faction suffix, known without a race declaring it.
pub const BORG_SUFFIX: &str = "_b";
/// The extension refuses a specular map file above this size.
pub const MAX_SPECULAR_BYTES: usize = 32 * 1024 * 1024;
const EXTENSIONS: [&str; 4] = ["dds", "tga", "png", "bmp"];
const ROOTS: [&str; 4] = ["textures", "textures/rgb", "textures/index8", "textures/compressed"];

/// What one copy of `ART_CFG.h` says about one suffix.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Setting {
    Absent,
    Off,
    On(String),
    /// Present but not usable: the inherited value stays.
    Invalid,
}

fn setting(cfg: &sta_assets::cfg_h::CfgH, names: [&str; 4]) -> Setting {
    match names.iter().find_map(|n| cfg.raw(n)) {
        None => Setting::Absent,
        Some(raw) if raw.trim().is_empty() => Setting::Off,
        Some(raw) => valid_suffix(raw).map_or(Setting::Invalid, Setting::On),
    }
}

const SPEC_NAMES: [&str; 4] = ["A2FO_SPECULAR_SUFFIX", "a2fo_specular_suffix", "SpecularTextureSuffix", "speculartexturesuffix"];
const EMIT_NAMES: [&str; 4] = ["A2FO_EMISSIVE_SUFFIX", "a2fo_emissive_suffix", "EmissiveTextureSuffix", "emissivetexturesuffix"];

/// The `ART_CFG.h` declarations: a map convention is on when its suffix is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapSuffixes {
    pub specular: Option<String>,
    pub emissive: Option<String>,
}

impl MapSuffixes {
    /// Merge the copies of `ART_CFG.h`, the lowest layer first.
    pub fn from_copies<'a>(texts: impl IntoIterator<Item = &'a str>) -> MapSuffixes {
        let mut out = MapSuffixes::default();
        for text in texts {
            let cfg = sta_assets::cfg_h::CfgH::parse(text);
            for (names, slot) in [(SPEC_NAMES, &mut out.specular), (EMIT_NAMES, &mut out.emissive)] {
                match setting(&cfg, names) {
                    Setting::On(s) => *slot = Some(s),
                    Setting::Off => *slot = None,
                    Setting::Absent | Setting::Invalid => {}
                }
            }
        }
        out
    }

    /// Every copy of `ART_CFG.h` in the mount, merged (the extension's Data, parent mods, active mod).
    pub fn from_vfs(vfs: &sta_vfs::Vfs) -> MapSuffixes {
        let copies = vfs.resolve_all("ART_CFG.h").len();
        let texts: Vec<String> = (0..copies).rev().filter_map(|v| sta_content::layered::resolve(vfs, "ART_CFG.h", v).ok().flatten()).map(|r| r.text).collect();
        MapSuffixes::from_copies(texts.iter().map(String::as_str))
    }

    pub fn any(&self) -> bool {
        self.specular.is_some() || self.emissive.is_some()
    }
}

/// Which copies of `ART_CFG.h` a mount has: the settings are read again when this changes.
fn fingerprint(vfs: &sta_vfs::Vfs) -> String {
    vfs.resolve_all("ART_CFG.h").iter().map(|(layer, _)| *layer).collect::<Vec<_>>().join("|")
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

/// The diffuse's name without directory and image extension.
pub fn stem(diffuse: &str) -> &str {
    let name = diffuse.rsplit(['/', '\\']).next().unwrap_or(diffuse);
    name.rsplit_once('.').filter(|(_, e)| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x))).map_or(name, |(s, _)| s)
}

/// A diffuse that already ends in a known faction suffix, reduced to its base (`ship_b` to
/// `ship`, `_b`): the map suffix goes between them, never after.
pub fn split_faction<'a>(stem: &'a str, known: &[&str]) -> (&'a str, Option<String>) {
    for f in known.iter().filter(|f| !f.is_empty()) {
        if stem.len() > f.len() && stem.is_char_boundary(stem.len() - f.len()) && stem[stem.len() - f.len()..].eq_ignore_ascii_case(f) {
            return (&stem[..stem.len() - f.len()], Some((*f).to_owned()));
        }
    }
    (stem, None)
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

    /// The environment's switches, read once.
    pub fn startup() -> MapSwitches {
        static CELL: std::sync::OnceLock<MapSwitches> = std::sync::OnceLock::new();
        *CELL.get_or_init(MapSwitches::from_env)
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

/// The highlight colour (rgb) and power (w) of a part with a specular map: a Phong material's own
/// (zero meaning no highlight), the defaults for anything else.
pub fn highlight_params(stock: Option<&StockLight>) -> Vec4 {
    match stock {
        Some(l) if l.model == 2 => Vec3::from(l.specular).extend(l.power),
        _ => Vec3::splat(DEFAULT_STRENGTH).extend(DEFAULT_POWER),
    }
}

impl Loader<'_> {
    /// The mount's declarations, read once per mount (the copies of `ART_CFG.h` it has).
    pub fn map_suffixes(&mut self) -> MapSuffixes {
        let fp = fingerprint(self.vfs);
        if let Some((f, s)) = &self.cache.map_config
            && *f == fp
        {
            return s.clone();
        }
        self.cache.map.retain(|k, _| !k.starts_with("m:"));
        let s = MapSuffixes::from_vfs(self.vfs);
        self.cache.map_config = Some((fp, s.clone()));
        s
    }

    /// The maps the mod's declarations give a group whose diffuse is `diffuse` (`faction`: the
    /// faction suffix of the variant drawn, `known`: the faction suffixes a diffuse may end in).
    /// `None` when nothing is declared or no file exists; nothing is searched when undeclared.
    pub fn hull_maps(&mut self, diffuse: &str, faction: Option<&str>, known: &[&str]) -> Option<HullMaps> {
        let suffixes = self.map_suffixes();
        if !suffixes.any() {
            return None;
        }
        self.hull_maps_with(&suffixes, diffuse, faction, known, MapSwitches::startup(), MAX_SPECULAR_BYTES)
    }

    pub fn hull_maps_with(&mut self, suffixes: &MapSuffixes, diffuse: &str, faction: Option<&str>, known: &[&str], switches: MapSwitches, cap: usize) -> Option<HullMaps> {
        let mut all = vec![BORG_SUFFIX];
        all.extend_from_slice(known);
        let (base, own) = split_faction(stem(diffuse), &all);
        let faction = faction.map(str::to_owned).or(own);
        let faction = faction.as_deref();
        let mut maps = HullMaps::default();
        if let (Some(s), true) = (&suffixes.specular, switches.specular) {
            maps.specular = self.mask_map(base, s, faction, cap);
        }
        if let (Some(s), true) = (&suffixes.emissive, switches.emissive) {
            maps.emissive = self.emissive_map(base, s, faction);
        }
        (!maps.is_empty()).then_some(maps)
    }

    fn first_file(&self, candidates: &[String]) -> Option<(String, &'static str)> {
        candidates.iter().find_map(|name| find_map_file(self.vfs, name))
    }

    fn pixels_of(&self, path: &str, ext: &str) -> Option<Rgba> {
        match ext {
            "dds" | "tga" => sta_assets::decode_rgba(self.vfs, path, if ext == "dds" { Fmt::Dds } else { Fmt::Tga }).map_err(|e| warn!("{path}: {e}")).ok(),
            "png" => {
                let data = sta_assets::read(self.vfs, path).ok()?;
                let img = image::load_from_memory(&data).map_err(|e| warn!("{path}: {e}")).ok()?.to_rgba8();
                Some(Rgba { width: img.width(), height: img.height(), pixels: img.into_raw(), has_alpha: true })
            }
            _ => sta_assets::bmp::load(self.vfs, path).map_err(|e| warn!("{path}: {e}")).ok(),
        }
    }

    /// One map as authored: a DDS keeps its mips, a TGA its mip files; others get generated mips.
    fn image_of(&self, path: &str, ext: &str, srgb: bool, cap: Option<usize>) -> Option<Image> {
        let data = sta_assets::read(self.vfs, path).ok()?;
        if cap.is_some_and(|c| data.len() > c) {
            warn!("{path}: map over {} MB, ignored", cap.unwrap_or(0) / (1024 * 1024));
            return None;
        }
        match ext {
            "dds" => {
                let kind = sta_assets::dds::kind(&data).ok()?;
                if matches!(kind, DdsKind::Bc1 | DdsKind::Bc2 | DdsKind::Bc3) && self.formats.contains(bevy::image::CompressedImageFormats::BC) {
                    Image::from_buffer(&data, ImageType::Extension("dds"), self.formats, srgb, repeat_sampler(), RenderAssetUsages::RENDER_WORLD).map_err(|e| warn!("{path}: {e}")).ok()
                } else {
                    let base = sta_assets::dds::decode(&data).map_err(|e| warn!("{path}: {e}")).ok()?;
                    Some(upload(&pixels::generate_mips(&base), srgb))
                }
            }
            "tga" => {
                let base = sta_assets::decode_tga(path, &data).map_err(|e| warn!("{path}: {e}")).ok()?;
                Some(upload(&sta_assets::tga_mip_chain(self.vfs, path, base), srgb))
            }
            _ => self.pixels_of(path, ext).map(|px| upload(&pixels::generate_mips(&px), srgb)),
        }
    }

    fn mask_map(&mut self, base: &str, suffix: &str, faction: Option<&str>, cap: usize) -> Option<Handle<Image>> {
        let key = format!("m:s:{base}:{suffix}:{}", faction.unwrap_or("")).to_ascii_lowercase();
        if let Some(h) = self.cache.map.get(&key) {
            return h.clone();
        }
        let handle = self.first_file(&names(base, suffix, "", faction)).and_then(|(p, e)| self.image_of(&p, e, false, Some(cap))).map(|i| self.images.add(i));
        self.cache.map.insert(key, handle.clone());
        handle
    }

    fn emissive_map(&mut self, base: &str, suffix: &str, faction: Option<&str>) -> Option<Handle<Image>> {
        let key = format!("m:e:{base}:{suffix}:{}", faction.unwrap_or("")).to_ascii_lowercase();
        if let Some(h) = self.cache.map.get(&key) {
            return h.clone();
        }
        let files: Vec<(String, &'static str)> = EMISSIVE_WORDS.iter().filter_map(|w| self.first_file(&names(base, suffix, w, faction))).collect();
        let image = match files.as_slice() {
            [] => None,
            [(p, e)] => self.image_of(p, e, true, None),
            many => {
                let px: Vec<Rgba> = many.iter().filter_map(|(p, e)| self.pixels_of(p, e)).collect();
                combine_emissive(&px).map(|c| upload(&pixels::generate_mips(&c), true))
            }
        };
        let handle = image.map(|i| self.images.add(i));
        self.cache.map.insert(key, handle.clone());
        handle
    }
}

/// `name` as a map file: the first extension (dds, tga, png, bmp) that exists in any root, the
/// layer that wins by the mount's order.
fn find_map_file(vfs: &sta_vfs::Vfs, name: &str) -> Option<(String, &'static str)> {
    for ext in EXTENSIONS {
        let mut best: Option<(usize, String)> = None;
        for root in ROOTS {
            let path = format!("{root}/{name}.{ext}").to_ascii_lowercase();
            if let Some((layer, _)) = vfs.resolve(&path)
                && best.as_ref().is_none_or(|(old, _)| layer < *old)
            {
                best = Some((layer, path));
            }
        }
        if let Some((_, path)) = best {
            return Some((path, ext));
        }
    }
    None
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
/// The rule the hull shader evaluates for one light, written out on the CPU: `light * (N.H)^power *
/// colour * mask`, zero unless `N.L > 0` and the colour and power are above zero. `params` is
/// [`highlight_params`].
pub fn highlight(n: Vec3, l: Vec3, v: Vec3, light: Vec3, params: Vec4, mask: Vec3) -> Vec3 {
    let (colour, power) = (params.truncate(), params.w);
    if n.dot(l) <= 0.0 || power <= 0.0 || colour.max_element() <= 0.0 {
        return Vec3::ZERO;
    }
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
        MapSuffixes::from_copies([text])
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
        assert!(!MapSuffixes::default().any());
        assert!(!cfg("#define SOMETHING_ELSE 1\n").any());
        assert!(!cfg("#define A2FO_SPECULAR_SUFFIX \"\"\n").any(), "empty disables");
        assert!(!cfg("#define A2FO_SPECULAR_SUFFIX \"_a b\"\n").any(), "a space is not allowed");
        assert!(!cfg("#define A2FO_SPECULAR_SUFFIX \"../x\"\n").any());
        let long = format!("#define A2FO_SPECULAR_SUFFIX \"{}\"\n", "a".repeat(65));
        assert!(!cfg(&long).any(), "over 64 characters");
        assert!(cfg(&format!("#define A2FO_SPECULAR_SUFFIX \"{}\"\n", "a".repeat(64))).any());
    }

    #[test]
    fn copies_merge_per_setting_and_an_invalid_value_keeps_the_inherited_one() {
        let base = "#define A2FO_SPECULAR_SUFFIX \"_specular\"\n#define A2FO_EMISSIVE_SUFFIX \"_emissive_\"\n";
        // A mod copy that declares neither inherits both.
        assert_eq!(MapSuffixes::from_copies([base, "#define OTHER 1\n"]), cfg(base));
        // One that changes one setting keeps the other.
        let m = MapSuffixes::from_copies([base, "#define A2FO_SPECULAR_SUFFIX \"_spec\"\n"]);
        assert_eq!((m.specular.as_deref(), m.emissive.as_deref()), (Some("_spec"), Some("_emissive_")));
        // An invalid value is ignored; an empty one turns the convention off.
        let m = MapSuffixes::from_copies([base, "#define A2FO_SPECULAR_SUFFIX \"bad value\"\n#define A2FO_EMISSIVE_SUFFIX \"\"\n"]);
        assert_eq!((m.specular.as_deref(), m.emissive), (Some("_specular"), None));
    }

    #[test]
    fn faction_suffix_goes_last_and_falls_back() {
        assert_eq!(names("ship", "_specular", "", None), ["ship_specular"]);
        assert_eq!(names("ship", "_specular", "", Some("_b")), ["ship_specular_b", "ship_specular"]);
        assert_eq!(names("ship", "_emissive_", "warp", Some("_b")), ["ship_emissive_warp_b", "ship_emissive_warp"]);
        assert_eq!(names("ship", "_emissive_", "warp", Some("")), ["ship_emissive_warp"]);
    }

    #[test]
    fn a_diffuse_ending_in_a_faction_suffix_is_reduced_to_its_base_first() {
        assert_eq!(split_faction("ship_b", &["_b"]), ("ship", Some("_b".into())));
        assert_eq!(split_faction("Ship_K", &["_b", "_k"]), ("Ship", Some("_k".into())));
        assert_eq!(split_faction("ship", &["_b"]), ("ship", None));
        assert_eq!(split_faction("_b", &["_b"]), ("_b", None), "nothing left of a bare suffix");
        assert_eq!(stem("Textures/RGB/Ship_B.TGA"), "Ship_B");
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("sta64_mapped_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
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

    /// A one-texel 24-bit BMP of the colour.
    fn bmp(r: u8, g: u8, b: u8) -> Vec<u8> {
        let mut f = b"BM".to_vec();
        f.extend_from_slice(&58u32.to_le_bytes());
        f.extend_from_slice(&[0; 4]);
        f.extend_from_slice(&54u32.to_le_bytes());
        f.extend_from_slice(&40u32.to_le_bytes());
        f.extend_from_slice(&1i32.to_le_bytes());
        f.extend_from_slice(&1i32.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&24u16.to_le_bytes());
        f.extend_from_slice(&[0; 24]);
        f.extend_from_slice(&[b, g, r, 0]);
        f
    }

    fn png(r: u8, g: u8, b: u8) -> Vec<u8> {
        let mut out = Vec::new();
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([r, g, b, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    /// A temp install: `Data/ART_CFG.h` (and optionally one on top in `Data/Mods/Top`) and textures.
    struct Install(std::path::PathBuf);
    impl Install {
        fn new(tag: &str, art_cfg: Option<&str>, files: &[(&str, Vec<u8>)]) -> Self {
            let root = scratch(tag);
            let data = root.join("Data");
            std::fs::create_dir_all(data.join("Textures/RGB")).unwrap();
            if let Some(cfg) = art_cfg {
                std::fs::write(data.join("ART_CFG.h"), cfg).unwrap();
            }
            for (name, bytes) in files {
                std::fs::write(data.join("Textures/RGB").join(name), bytes).unwrap();
            }
            Install(root)
        }
        fn top_cfg(&self, cfg: &str) {
            let top = self.0.join("Data/Mods/Top");
            std::fs::create_dir_all(&top).unwrap();
            std::fs::write(top.join("ART_CFG.h"), cfg).unwrap();
        }
        fn run<R>(&self, top: bool, f: impl FnOnce(&mut Loader) -> R) -> (R, usize) {
            let vfs = sta_vfs::Vfs::mount_install(&self.0, top.then_some("Top")).unwrap();
            let mut cache = crate::render::textures::TextureCache::default();
            let mut images = Assets::<Image>::default();
            let r = f(&mut Loader { vfs: &vfs, cache: &mut cache, images: &mut images, formats: bevy::image::CompressedImageFormats::NONE });
            (r, cache.map.len())
        }
        fn maps(&self, diffuse: &str, faction: Option<&str>) -> (Option<HullMaps>, usize) {
            self.run(false, |l| l.hull_maps(diffuse, faction, &[]))
        }
    }
    impl Drop for Install {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn first_texel(l: &Loader, h: &Option<Handle<Image>>) -> Option<[u8; 4]> {
        h.as_ref().and_then(|h| l.images.get(h)).and_then(|i| i.data.as_ref().map(|d| [d[0], d[1], d[2], d[3]]))
    }

    const SPEC_CFG: &str = "#define A2FO_SPECULAR_SUFFIX \"_specular\"\n";

    #[test]
    fn content_that_declares_nothing_is_not_looked_at() {
        let files = [("ship_specular.tga", tga(255, 255, 255)), ("ship_emissive_warp.tga", tga(255, 255, 255))];
        let install = Install::new("none", Some("#define SOMETHING 1\n"), &files);
        let (maps, cached) = install.maps("ship", None);
        assert!(maps.is_none());
        assert_eq!(cached, 0, "no search, nothing cached");
        assert!(Install::new("nocfg", None, &files).maps("ship", None).0.is_none());
    }

    #[test]
    fn a_declared_specular_map_is_found_with_the_faction_map_first() {
        let install = Install::new("spec", Some(SPEC_CFG), &[("ship_specular.tga", tga(10, 20, 30)), ("ship_specular_b.tga", tga(200, 100, 50))]);
        let ((base, faction), _) = install.run(false, |l| {
            let b = l.hull_maps("ship", None, &[]).unwrap();
            let f = l.hull_maps("Ship.tga", Some("_b"), &[]).unwrap();
            assert!(b.emissive.is_none());
            (first_texel(l, &b.specular), first_texel(l, &f.specular))
        });
        assert_eq!(&base.unwrap()[..3], &[10, 20, 30]);
        assert_eq!(&faction.unwrap()[..3], &[200, 100, 50], "the faction map wins; the extension is dropped from the diffuse name");
        let only_base = Install::new("spec2", Some(SPEC_CFG), &[("ship_specular.tga", tga(10, 20, 30))]);
        let (t, _) = only_base.run(false, |l| {
            let m = l.hull_maps("ship", Some("_b"), &[]).unwrap();
            first_texel(l, &m.specular)
        });
        assert_eq!(&t.unwrap()[..3], &[10, 20, 30], "no faction map: the base map");
        assert!(only_base.maps("other", None).0.is_none());
    }

    #[test]
    fn a_diffuse_already_carrying_the_faction_suffix_finds_map_role_first_names() {
        let install = Install::new("fsuf", Some(SPEC_CFG), &[("ship_specular_b.tga", tga(1, 2, 3)), ("ship_b_specular.tga", tga(9, 9, 9)), ("ship_specular_k.tga", tga(4, 5, 6))]);
        let (t, _) = install.run(false, |l| {
            let m = l.hull_maps("ship_b", None, &[]).unwrap();
            first_texel(l, &m.specular)
        });
        assert_eq!(&t.unwrap()[..3], &[1, 2, 3], "ship_specular_b, never ship_b_specular");
        let (k, _) = install.run(false, |l| {
            let m = l.hull_maps("ship_k", None, &["_k"]).unwrap();
            first_texel(l, &m.specular)
        });
        assert_eq!(&k.unwrap()[..3], &[4, 5, 6], "a suffix a race declares");
    }

    #[test]
    fn declared_emissive_maps_are_combined_and_a_lone_one_is_kept() {
        let cfg = "#define A2FO_EMISSIVE_SUFFIX \"_emissive_\"\n";
        let install = Install::new("emit", Some(cfg), &[("ship_emissive_warp.tga", tga(200, 0, 0)), ("ship_emissive_weapons.tga", tga(0, 150, 9)), ("lone_emissive_life.tga", tga(7, 8, 9))]);
        let (t, _) = install.run(false, |l| {
            let m = l.hull_maps("ship", None, &[]).unwrap();
            assert!(m.specular.is_none());
            first_texel(l, &m.emissive)
        });
        assert_eq!(t.unwrap(), [200, 150, 9, 255], "brightest per channel, opaque");
        let (t, _) = install.run(false, |l| {
            let m = l.hull_maps("lone", None, &[]).unwrap();
            first_texel(l, &m.emissive)
        });
        assert_eq!(&t.unwrap()[..3], &[7, 8, 9]);
    }

    #[test]
    fn png_and_bmp_maps_load_in_the_extensions_order() {
        let install = Install::new("fmt", Some(SPEC_CFG), &[("a_specular.png", png(11, 12, 13)), ("b_specular.bmp", bmp(21, 22, 23)), ("c_specular.png", png(31, 32, 33)), ("c_specular.tga", tga(41, 42, 43))]);
        let (v, _) = install.run(false, |l| {
            ["a", "b", "c"].map(|n| {
                let m = l.hull_maps(n, None, &[]).unwrap();
                first_texel(l, &m.specular).unwrap()
            })
        });
        assert_eq!(&v[0][..3], &[11, 12, 13], "png");
        assert_eq!(&v[1][..3], &[21, 22, 23], "bmp");
        assert_eq!(&v[2][..3], &[41, 42, 43], "tga before png");
    }

    #[test]
    fn a_specular_map_over_the_cap_is_refused_and_the_switches_skip_the_work() {
        let install = Install::new("cap", Some("#define A2FO_SPECULAR_SUFFIX \"_specular\"\n#define A2FO_EMISSIVE_SUFFIX \"_emissive_\"\n"), &[("ship_specular.tga", tga(1, 2, 3)), ("ship_emissive_warp.tga", tga(4, 5, 6))]);
        let s = MapSuffixes { specular: Some("_specular".into()), emissive: Some("_emissive_".into()) };
        let ((over, off), cached) = install.run(false, |l| {
            let over = l.hull_maps_with(&s, "ship", None, &[], MapSwitches::default(), 10);
            let off = l.hull_maps_with(&s, "ship", None, &[], MapSwitches { specular: false, emissive: false }, MAX_SPECULAR_BYTES);
            (over, off)
        });
        assert!(over.as_ref().is_some_and(|m| m.specular.is_none() && m.emissive.is_some()), "only the specular map is capped");
        assert!(off.is_none());
        assert_eq!(cached, 2, "the refusal is cached; the switched-off lookups cached nothing more");
    }

    #[test]
    fn layers_inherit_the_declaration_and_absent_lookups_are_cached() {
        let install = Install::new("layers", Some(SPEC_CFG), &[("ship_specular.tga", tga(1, 2, 3))]);
        install.top_cfg("#define UNRELATED 1\n");
        let (found, _) = install.run(true, |l| {
            assert_eq!(l.map_suffixes().specular.as_deref(), Some("_specular"), "the mod's ART_CFG.h does not hide the base one's");
            l.hull_maps("ship", None, &[]).is_some()
        });
        assert!(found);
        let ((), cached) = install.run(true, |l| {
            assert!(l.hull_maps("none", None, &[]).is_none());
            let n = l.cache.map.len();
            assert!(l.hull_maps("none", None, &[]).is_none());
            assert_eq!(l.cache.map.len(), n, "the miss is remembered");
        });
        assert_eq!(cached, 1);
    }

    fn phong(specular: [f32; 3], power: f32, model: u8) -> StockLight {
        StockLight { ambient: [0.0; 3], diffuse: [1.0; 3], specular, power, model }
    }

    #[test]
    fn a_phong_material_supplies_its_own_highlight_and_zero_means_none() {
        let (n, l, v, one) = (Vec3::Z, Vec3::Z, Vec3::Z, Vec3::ONE);
        let own = highlight_params(Some(&phong([0.0, 0.0, 1.0], 16.0, 2)));
        assert_eq!(highlight(n, l, v, one, own, one), Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(highlight(n, l, v, one, own, Vec3::ZERO), Vec3::ZERO, "a black map removes it");
        assert_eq!(highlight(n, l, v, one, own, Vec3::new(1.0, 0.0, 0.0)), Vec3::ZERO, "a red map masks a blue highlight");
        let no_colour = highlight_params(Some(&phong([0.0; 3], 16.0, 2)));
        assert_eq!(highlight(n, l, v, one, no_colour, one), Vec3::ZERO, "Phong with a zero specular: none, not the defaults");
        let no_power = highlight_params(Some(&phong([1.0; 3], 0.0, 2)));
        assert_eq!(highlight(n, l, v, one, no_power, one), Vec3::ZERO, "Phong with power 0: none");
    }

    #[test]
    fn anything_else_takes_the_defaults_whatever_its_specular_data() {
        let (n, l, v, one) = (Vec3::Z, Vec3::Z, Vec3::Z, Vec3::ONE);
        for p in [highlight_params(Some(&phong([0.9, 0.1, 0.1], 3.0, 1))), highlight_params(Some(&phong([0.0; 3], 0.0, 0))), highlight_params(None)] {
            assert_eq!(p, Vec4::new(0.5, 0.5, 0.5, 24.0));
            assert_eq!(highlight(n, l, v, one, p, one), Vec3::splat(DEFAULT_STRENGTH));
        }
        let grazing = Vec3::new(1.0, 0.0, 1.0).normalize();
        assert!(highlight(n, l, grazing, one, Vec4::new(1.0, 1.0, 1.0, 64.0), one).x < 0.1, "off-axis and sharp is weaker");
        assert_eq!(highlight(n, -Vec3::Z, v, one, Vec4::new(1.0, 1.0, 1.0, 8.0), one), Vec3::ZERO, "a light behind adds nothing");
    }

    #[test]
    fn emissive_adds_and_clips() {
        assert_eq!(add_emissive(Vec3::splat(0.25), Vec3::new(0.5, 0.0, 1.0)), Vec3::new(0.75, 0.25, 1.0));
        assert_eq!(add_emissive(Vec3::splat(0.25), Vec3::ZERO), Vec3::splat(0.25));
    }

    #[test]
    fn emissive_maps_combine_by_the_brightest_channel_and_ignore_alpha() {
        let solid = |w: u32, h: u32, c: [u8; 4]| Rgba { width: w, height: h, has_alpha: true, pixels: c.repeat((w * h) as usize) };
        let c = combine_emissive(&[solid(2, 2, [200, 10, 0, 7]), solid(4, 4, [10, 150, 30, 255])]).unwrap();
        assert_eq!((c.width, c.height), (2, 2), "the first map sets the size");
        assert_eq!(&c.pixels[..4], &[200, 150, 30, 255]);
        assert!(combine_emissive(&[]).is_none());
    }

    #[test]
    fn switches_default_on() {
        assert_eq!(MapSwitches::default(), MapSwitches { specular: true, emissive: true });
    }
}
