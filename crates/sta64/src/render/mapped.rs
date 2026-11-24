//! Specular and emissive maps on hulls (RND-38), ported from the A2FOExtensions project
//! (https://github.com/TamsynnImogen/A2FOExtensions), so mods authored for it work unchanged
//! where the rules below are the extension's. Implements ADR 0004 D2 (the extension's content
//! conventions are adapted, none of its code runs), with D1, D3 and D11.
//!
//! Evidence grade: the declaration, the file names, the faction fallback, the file formats and
//! the emissive map selection follow the extension's source and documentation. The highlight
//! equation is read from the comments of her renderer interface header and from her scene code;
//! the renderer's own shader is in neither repository, so the arithmetic is this port's. None of
//! it was measured on a running extension.
//!
//! # Declaration (the mod's `ART_CFG.h`)
//!
//! `#define A2FO_SPECULAR_SUFFIX "_specular"` and `#define A2FO_EMISSIVE_SUFFIX "_emissive_"`
//! (aliases `SpecularTextureSuffix`, `EmissiveTextureSuffix`). Read the way STA64 reads every cfg
//! header (ADR 0004 D11, `sta_content::layered`): the top copy of the file, plus the copies it
//! reaches through `#include` of its own name; a copy without that include replaces the file.
//! (The extension instead merges each setting over every copy; see the PR body.) In the effective
//! file an empty value turns the convention off, and a value that is not 1 to 64 ASCII letters,
//! digits, `_` or `-` also leaves it off, because under that reading the invalid assignment is the
//! last one and nothing is inherited past it. A class can also name emissive textures with
//! its ODF. `textureX` selects the material for `emissiveXWarp`, `emissiveXImpulse` and the
//! other channel keys. Without indexed materials, the unnumbered keys are used.
//!
//! # Files (`.dds`, `.tga`, `.png`, `.bmp`; the search order is the extension's: each mounted layer
//! in precedence order (active mod first, then its parents, then the base data), within it textures,
//! textures/rgb, textures/index8, textures/compressed, then the root, and the extension innermost)
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
//!   after texturing, times the mask. The default strength and power match A2FOExtensions' Refit
//!   path. `STA64_SPECULAR_MATERIAL_COLOURS=1` opts into a Phong material's own colour and power.
//!   With that option, a zero colour or power means no highlight.
//! - Emissive: the six subsystems are retained as separate array layers. The first present map
//!   sets their common size, other maps are scaled to it, and each layer gets its own mips.
//!   Runtime state chooses which layers emit. Alpha is ignored; lights, shadows and team colour
//!   do not touch self-lit pixels.
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
use crate::render::textures::{repeat_sampler, upload, upload_emissive, Loader};

/// The default highlight colour on each channel (the extension's `SpecularMapStrength`).
pub const DEFAULT_STRENGTH: f32 = 0.5;
/// The default highlight power (the extension's `SpecularMapPower`).
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
/// The extension's directory order under one layer; the empty one is the root itself.
const ROOTS: [&str; 5] = ["textures", "textures/rgb", "textures/index8", "textures/compressed", ""];

/// What the effective `ART_CFG.h` says about one suffix.
fn setting(cfg: &sta_assets::cfg_h::CfgH, names: [&str; 4]) -> Option<String> {
    names.iter().find_map(|n| cfg.raw(n)).and_then(valid_suffix)
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
    /// The declarations of one effective (already layered) `ART_CFG.h`. An empty or invalid value
    /// is off.
    pub fn from_cfg(cfg: &sta_assets::cfg_h::CfgH) -> MapSuffixes {
        MapSuffixes { specular: setting(cfg, SPEC_NAMES), emissive: setting(cfg, EMIT_NAMES) }
    }

    /// The text of one `ART_CFG.h` as the effective file.
    #[cfg(test)]
    pub fn from_text(text: &str) -> MapSuffixes {
        MapSuffixes::from_cfg(&sta_assets::cfg_h::CfgH::parse(text))
    }

    /// The mount's `ART_CFG.h` through the layered reading (D11): top copy and its self-includes.
    pub fn from_vfs(vfs: &sta_vfs::Vfs) -> MapSuffixes {
        sta_content::layered::cfg_h(vfs, "ART_CFG.h").map(|c| MapSuffixes::from_cfg(&c)).unwrap_or_default()
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
    // Longest first: a declared `_ab` wins over a shorter `_b` that is its tail.
    let mut known: Vec<&str> = known.iter().copied().filter(|f| !f.is_empty()).collect();
    known.sort_by_key(|f| std::cmp::Reverse(f.len()));
    for f in known {
        if stem.len() > f.len() && stem.is_char_boundary(stem.len() - f.len()) && stem[stem.len() - f.len()..].eq_ignore_ascii_case(f) {
            return (&stem[..stem.len() - f.len()], Some(f.to_owned()));
        }
    }
    (stem, None)
}

/// The user's switches (the extension's `[Effects] SpecularMaps` / `EmissiveMaps`, default on).
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapSwitches {
    pub specular: bool,
    pub emissive: bool,
    pub specular_material_colours: bool,
}

impl Default for MapSwitches {
    fn default() -> Self {
        MapSwitches { specular: true, emissive: true, specular_material_colours: false }
    }
}

impl MapSwitches {
    /// `STA64_SPECULAR_MAPS` / `STA64_EMISSIVE_MAPS`: `0` turns one off.
    /// `STA64_SPECULAR_MATERIAL_COLOURS=1` opts into authored Phong highlights.
    pub fn from_env() -> MapSwitches {
        let on = |k: &str| std::env::var(k).map_or(true, |v| v.trim() != "0");
        MapSwitches {
            specular: on("STA64_SPECULAR_MAPS"),
            emissive: on("STA64_EMISSIVE_MAPS"),
            specular_material_colours: std::env::var("STA64_SPECULAR_MATERIAL_COLOURS").is_ok_and(|value| value.trim() == "1"),
        }
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

/// The highlight colour (rgb) and power (w) of a part with a specular map.
/// A2FOExtensions defaults to the map's highlight settings even on Phong materials.
pub fn highlight_params(stock: Option<&StockLight>, use_material_colours: bool) -> Vec4 {
    match stock {
        Some(l) if use_material_colours && l.model == 2 => Vec3::from(l.specular).extend(l.power),
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
    #[cfg(test)]
    fn hull_maps(&mut self, diffuse: &str, faction: Option<&str>, known: &[&str]) -> Option<HullMaps> {
        self.hull_maps_for_part(diffuse, faction, known, None)
    }

    /// Resolve one material's maps, with inherited class ODF entries when available.
    pub fn hull_maps_for_part(&mut self, diffuse: &str, faction: Option<&str>, known: &[&str], raw: Option<&sta_odf::Odf>) -> Option<HullMaps> {
        let suffixes = self.map_suffixes();
        if !suffixes.any() && raw.is_none() {
            return None;
        }
        // The spawn path has no faction catalogue at hand: the races' declared suffixes are read
        // once per mount and joined to what the caller knows.
        let races = self.race_suffixes();
        let mut all: Vec<&str> = known.to_vec();
        all.extend(races.iter().map(String::as_str).filter(|r| !known.contains(r)));
        self.hull_maps_with_raw(&suffixes, diffuse, faction, &all, (MapSwitches::startup(), MAX_SPECULAR_BYTES), raw)
    }

    /// The faction suffixes the mount's races declare, read once per mount.
    fn race_suffixes(&mut self) -> Vec<String> {
        let key = self.vfs.layers().iter().map(|l| l.name()).collect::<Vec<_>>().join("|");
        if let Some((k, v)) = &self.cache.race_suffixes
            && *k == key
        {
            return v.clone();
        }
        let v: Vec<String> = crate::render::appearance::factions::FactionCatalogue::load(self.vfs).suffixes().into_iter().map(str::to_owned).collect();
        self.cache.race_suffixes = Some((key, v.clone()));
        v
    }

    #[cfg(test)]
    fn hull_maps_with(&mut self, suffixes: &MapSuffixes, diffuse: &str, faction: Option<&str>, known: &[&str], switches: MapSwitches, cap: usize) -> Option<HullMaps> {
        self.hull_maps_with_raw(suffixes, diffuse, faction, known, (switches, cap), None)
    }

    fn hull_maps_with_raw(&mut self, suffixes: &MapSuffixes, diffuse: &str, faction: Option<&str>, known: &[&str], (switches, cap): (MapSwitches, usize), raw: Option<&sta_odf::Odf>) -> Option<HullMaps> {
        let mut all = vec![BORG_SUFFIX];
        all.extend_from_slice(known);
        let (base, own) = split_faction(stem(diffuse), &all);
        let faction = faction.map(str::to_owned).or(own);
        let faction = faction.as_deref();
        let mut maps = HullMaps::default();
        if let (Some(s), true) = (&suffixes.specular, switches.specular) {
            maps.specular = self.mask_map(base, s, faction, cap);
        }
        if switches.emissive && (suffixes.emissive.is_some() || raw.is_some()) {
            maps.emissive = self.emissive_map(base, suffixes.emissive.as_deref(), faction, raw, diffuse);
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

    fn emissive_map(&mut self, base: &str, suffix: Option<&str>, faction: Option<&str>, raw: Option<&sta_odf::Odf>, diffuse: &str) -> Option<Handle<Image>> {
        const ODF_WORDS: [&str; 6] = ["Warp", "Impulse", "Shields", "LifeSupport", "Sensors", "Weapons"];
        let indexed = raw.is_some_and(|odf| (0..64).any(|index| odf.str(&format!("texture{index}")).is_some()));
        let selected = raw.and_then(|odf| {
            (0..64).find(|index| odf.str(&format!("texture{index}")).is_some_and(|name| stem(name).eq_ignore_ascii_case(stem(diffuse))))
        });
        let files: [Option<(String, &'static str)>; 6] = std::array::from_fn(|index| {
            let explicit = raw.and_then(|odf| {
                let key = if indexed { format!("emissive{}{word}", selected?, word = ODF_WORDS[index]) }
                          else { format!("emissive{}", ODF_WORDS[index]) };
                odf.str(&key)
            });
            if let Some(name) = explicit {
                // An explicit declaration is authoritative even when its file is missing.
                return find_explicit_map_file(self.vfs, name);
            }
            if indexed && selected.is_none() { return None; }
            let suffix = suffix?;
            self.first_file(&names(base, suffix, EMISSIVE_WORDS[index], faction))
                .or_else(|| {
                    // Some installed maps use the ODF word “Sensors” as their suffix.
                    (EMISSIVE_WORDS[index] == "sensor").then(|| {
                        self.first_file(&names(base, suffix, "sensors", faction))
                    }).flatten()
                })
        });
        let key = format!("m:e:{}", files.iter().map(|file| file.as_ref().map_or("-".to_owned(), |(path, _)| {
            let origin = self.vfs.resolve(path).and_then(|(layer, _)| self.vfs.layers().get(layer)).map_or("unknown", |layer| layer.name());
            format!("{origin}:{path}")
        })).collect::<Vec<_>>().join("|"));
        if let Some(h) = self.cache.map.get(&key) {
            return h.clone();
        }
        let channels = files.map(|file| file.and_then(|(path, ext)| self.pixels_of(&path, ext)));
        let image = upload_emissive(&channels);
        let handle = image.map(|i| self.images.add(i));
        self.cache.map.insert(key, handle.clone());
        handle
    }
}

fn find_explicit_map_file(vfs: &sta_vfs::Vfs, name: &str) -> Option<(String, &'static str)> {
    let normalized = name.replace('\\', "/").to_ascii_lowercase();
    let basename = normalized.rsplit('/').next().unwrap_or(&normalized);
    if basename.contains('.') {
        let ext = EXTENSIONS.iter().copied().find(|ext| normalized.ends_with(&format!(".{ext}")))?;
        if vfs.resolve(&normalized).is_some() { return Some((normalized, ext)); }
        return find_map_file(vfs, stem(&normalized));
    }
    if normalized.contains('/') {
        return EXTENSIONS.iter().copied().find_map(|ext| {
            let path = format!("{normalized}.{ext}");
            vfs.resolve(&path).map(|_| (path, ext))
        });
    }
    find_map_file(vfs, &normalized)
}

/// `name` as a map file, in the extension's order: the layer that wins by the mount's precedence
/// first, within it the directory, and the file extension last (so a mod's `.tga` beats a base
/// layer's `.dds`, and the textures directory's `.tga` beats a subdirectory's `.dds`).
fn find_map_file(vfs: &sta_vfs::Vfs, name: &str) -> Option<(String, &'static str)> {
    let mut best: Option<((usize, usize, usize), String, &'static str)> = None;
    for (d, root) in ROOTS.iter().enumerate() {
        for (e, ext) in EXTENSIONS.iter().enumerate() {
            let path = if root.is_empty() { format!("{name}.{ext}") } else { format!("{root}/{name}.{ext}") }.to_ascii_lowercase();
            // The top layer with this exact path; a whiteout hides the ones below it.
            let Some((layer, _)) = vfs.resolve(&path) else { continue };
            let key = (layer, d, e);
            if best.as_ref().is_none_or(|(k, _, _)| key < *k) {
                best = Some((key, path, ext));
            }
        }
    }
    best.map(|(_, p, e)| (p, e))
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
        MapSuffixes::from_text(text)
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
        assert_eq!(split_faction("ship_ab", &["_b", "_ab"]), ("ship", Some("_ab".into())), "the longer suffix wins over its tail");
        assert_eq!(split_faction("ship_ab", &["_ab", "_b"]), ("ship", Some("_ab".into())));
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
    fn declared_emissive_maps_keep_independent_subsystem_layers() {
        let cfg = "#define A2FO_EMISSIVE_SUFFIX \"_emissive_\"\n";
        let install = Install::new("emit", Some(cfg), &[("ship_emissive_warp.tga", tga(200, 0, 0)), ("ship_emissive_weapons.tga", tga(0, 150, 9)), ("lone_emissive_life.tga", tga(7, 8, 9))]);
        let (t, _) = install.run(false, |l| {
            let m = l.hull_maps("ship", None, &[]).unwrap();
            assert!(m.specular.is_none());
            let image = l.images.get(m.emissive.as_ref().unwrap()).unwrap();
            assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 6);
            image.data.as_ref().unwrap().clone()
        });
        assert_eq!(&t[0..3], &[200, 0, 0]);
        assert_eq!(&t[5 * 4..5 * 4 + 3], &[0, 150, 9]);
        let (t, _) = install.run(false, |l| {
            let m = l.hull_maps("lone", None, &[]).unwrap();
            l.images.get(m.emissive.as_ref().unwrap()).unwrap().data.as_ref().unwrap().clone()
        });
        assert_eq!(&t[3 * 4..3 * 4 + 3], &[7, 8, 9]);
    }

    #[test]
    fn explicit_indexed_emissive_works_without_cfg_and_does_not_bleed() {
        let install = Install::new("indexed", None, &[("fconst_emissive_warp.tga", tga(31, 0, 0)), ("fconst_emissive_impulse.tga", tga(0, 42, 0))]);
        let raw = sta_odf::resolve("test.odf", &mut |_| Some("texture0 = \"fconst\"\nemissive0Warp = \"fconst_emissive_warp\"\nemissive0Impulse = \"fconst_emissive_impulse.tga\"\n".into())).unwrap();
        let (found, _) = install.run(false, |l| {
            let mapped = l.hull_maps_for_part("fconst", None, &[], Some(&raw)).unwrap();
            let data = l.images.get(mapped.emissive.as_ref().unwrap()).unwrap().data.as_ref().unwrap();
            (data[0], data[4 + 1], l.hull_maps_for_part("unrelated", None, &[], Some(&raw)).is_none())
        });
        assert_eq!(found, (31, 42, true));
    }

    #[test]
    fn installed_test_mod_fakula_emissive_channels() {
        let Ok(mod_path) = std::env::var("STA64_EMISSIVE_INSTALL") else { return };
        let mod_path = std::path::Path::new(&mod_path);
        assert_eq!(mod_path.file_name().and_then(|name| name.to_str()), Some("test"));
        let root = mod_path.ancestors().nth(3).expect("Data/Mods/test install path");
        let vfs = sta_vfs::Vfs::mount_install(root, Some("test")).unwrap();
        let (model_path, format) = sta_assets::find_model(&vfs, "fakula").expect("installed test model");
        let model = sta_assets::load_model(&vfs, &model_path, format).unwrap();
        let diffuse = model.nodes.iter().filter_map(|node| node.mesh())
            .flat_map(|mesh| &mesh.groups).filter_map(|group| group.maps.diffuse.as_deref())
            .find(|name| stem(name).eq_ignore_ascii_case("fakula")).expect("Akula diffuse material");
        let mut cache = crate::render::textures::TextureCache::default();
        let mut images = Assets::<Image>::default();
        let mut loader = Loader { vfs: &vfs, cache: &mut cache, images: &mut images, formats: bevy::image::CompressedImageFormats::NONE };
        let maps = loader.hull_maps_for_part(diffuse, None, &[], None).expect("authored FAkula emissive maps");
        let image = loader.images.get(maps.emissive.as_ref().unwrap()).unwrap();
        assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 6);
        let data = image.data.as_ref().unwrap();
        let per_layer = data.len() / 6;
        for (index, expected) in [true, true, false, true, true, false].into_iter().enumerate() {
            let active = data[index * per_layer..(index + 1) * per_layer].chunks_exact(4).any(|pixel| pixel[..3] != [0, 0, 0]);
            assert_eq!(active, expected, "FAkula channel {}", EMISSIVE_WORDS[index]);
        }
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
            let off = l.hull_maps_with(&s, "ship", None, &[], MapSwitches { specular: false, emissive: false, ..MapSwitches::default() }, MAX_SPECULAR_BYTES);
            (over, off)
        });
        assert!(over.as_ref().is_some_and(|m| m.specular.is_none() && m.emissive.is_some()), "only the specular map is capped");
        assert!(off.is_none());
        assert_eq!(cached, 2, "the refusal is cached; the switched-off lookups cached nothing more");
    }

    #[test]
    fn a_mod_copy_inherits_the_declaration_only_through_its_self_include() {
        let install = Install::new("layers", Some(SPEC_CFG), &[("ship_specular.tga", tga(1, 2, 3))]);
        // D11: a copy without the include replaces the file, the base declaration is gone.
        install.top_cfg("#define UNRELATED 1\n");
        let (found, _) = install.run(true, |l| {
            assert_eq!(l.map_suffixes(), MapSuffixes::default());
            l.hull_maps("ship", None, &[]).is_some()
        });
        assert!(!found, "replaced without the include: the convention is off");
        // With the self-include it inherits, and may change one setting.
        install.top_cfg("#include \"ART_CFG.h\"\n#define A2FO_EMISSIVE_SUFFIX \"_glow_\"\n");
        let (found, _) = install.run(true, |l| {
            let s = l.map_suffixes();
            assert_eq!((s.specular.as_deref(), s.emissive.as_deref()), (Some("_specular"), Some("_glow_")));
            l.hull_maps("ship", None, &[]).is_some()
        });
        assert!(found);
        // An invalid value in the effective file is off, not the inherited one.
        install.top_cfg("#include \"ART_CFG.h\"\n#define A2FO_SPECULAR_SUFFIX \"bad value\"\n");
        let (s, _) = install.run(true, |l| l.map_suffixes());
        assert_eq!(s, MapSuffixes::default());
        // Without a mod the base copy applies as it is.
        let (s, _) = install.run(false, |l| l.map_suffixes());
        assert_eq!(s.specular.as_deref(), Some("_specular"));
    }

    #[test]
    fn absent_lookups_are_cached() {
        let install = Install::new("miss", Some(SPEC_CFG), &[("ship_specular.tga", tga(1, 2, 3))]);
        let ((), cached) = install.run(false, |l| {
            assert!(l.hull_maps("none", None, &[]).is_none());
            let n = l.cache.map.len();
            assert!(l.hull_maps("none", None, &[]).is_none());
            assert_eq!(l.cache.map.len(), n, "the miss is remembered");
        });
        assert_eq!(cached, 1);
    }

    fn put(install: &Install, rel: &str, bytes: &[u8]) {
        let p = install.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn spec_texel(install: &Install, top: bool) -> [u8; 3] {
        let (t, _) = install.run(top, |l| {
            let m = l.hull_maps("ship", None, &[]).unwrap();
            first_texel(l, &m.specular).unwrap()
        });
        [t[0], t[1], t[2]]
    }

    #[test]
    fn a_mods_tga_beats_a_base_layers_dds() {
        // The layer comes before the extension: the active mod's `.tga` over the base `.dds`.
        let install = Install::new("order_layer", Some(SPEC_CFG), &[("ship_specular.tga", tga(1, 1, 1))]);
        put(&install, "Data/Textures/RGB/ship_specular.dds", b"not a real dds");
        install.top_cfg(&format!("#include \"ART_CFG.h\"\n{SPEC_CFG}"));
        put(&install, "Data/Mods/Top/Textures/RGB/ship_specular.tga", &tga(7, 8, 9));
        assert_eq!(spec_texel(&install, true), [7, 8, 9]);
    }

    #[test]
    fn the_textures_directory_beats_a_subdirectory_whatever_the_extension() {
        let install = Install::new("order_dir", Some(SPEC_CFG), &[("ship_specular.dds", b"not a real dds".to_vec())]);
        put(&install, "Data/Textures/ship_specular.tga", &tga(4, 5, 6));
        assert_eq!(spec_texel(&install, false), [4, 5, 6]);
        // And the root itself comes last.
        let root_only = Install::new("order_root", Some(SPEC_CFG), &[]);
        put(&root_only, "Data/ship_specular.tga", &tga(2, 3, 4));
        assert_eq!(spec_texel(&root_only, false), [2, 3, 4]);
        put(&root_only, "Data/Textures/Compressed/ship_specular.tga", &tga(5, 5, 5));
        assert_eq!(spec_texel(&root_only, false), [5, 5, 5]);
    }

    #[test]
    fn the_spawn_path_knows_the_races_suffixes_and_the_longest_wins() {
        let install = Install::new("spawn", Some(SPEC_CFG), &[("ship_specular_ab.tga", tga(1, 2, 3)), ("ship_specular_b.tga", tga(9, 9, 9))]);
        let (t, _) = install.run(false, |l| {
            // As if the mount's races declared `_b` and `_ab`; the spawn path passes no list.
            let key = l.vfs.layers().iter().map(|x| x.name()).collect::<Vec<_>>().join("|");
            l.cache.race_suffixes = Some((key, vec!["_b".into(), "_ab".into()]));
            let m = l.hull_maps("ship_ab", None, &[]).unwrap();
            first_texel(l, &m.specular).unwrap()
        });
        assert_eq!(&t[..3], &[1, 2, 3], "ship_ab reduces to ship with `_ab`, not `ship_a` with `_b`");
    }

    fn phong(specular: [f32; 3], power: f32, model: u8) -> StockLight {
        StockLight { ambient: [0.0; 3], diffuse: [1.0; 3], specular, power, model }
    }

    #[test]
    fn opted_in_phong_material_supplies_its_own_highlight_and_zero_means_none() {
        let (n, l, v, one) = (Vec3::Z, Vec3::Z, Vec3::Z, Vec3::ONE);
        let own = highlight_params(Some(&phong([0.0, 0.0, 1.0], 16.0, 2)), true);
        assert_eq!(highlight(n, l, v, one, own, one), Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(highlight(n, l, v, one, own, Vec3::ZERO), Vec3::ZERO, "a black map removes it");
        assert_eq!(highlight(n, l, v, one, own, Vec3::new(1.0, 0.0, 0.0)), Vec3::ZERO, "a red map masks a blue highlight");
        let no_colour = highlight_params(Some(&phong([0.0; 3], 16.0, 2)), true);
        assert_eq!(highlight(n, l, v, one, no_colour, one), Vec3::ZERO, "Phong with a zero specular: none, not the defaults");
        let no_power = highlight_params(Some(&phong([1.0; 3], 0.0, 2)), true);
        assert_eq!(highlight(n, l, v, one, no_power, one), Vec3::ZERO, "Phong with power 0: none");
    }

    #[test]
    fn anything_else_takes_the_defaults_whatever_its_specular_data() {
        let (n, l, v, one) = (Vec3::Z, Vec3::Z, Vec3::Z, Vec3::ONE);
        let zero_gloss = phong([0.0; 3], 4.0, 2);
        let use_material_colours = MapSwitches::default().specular_material_colours;
        for p in [
            highlight_params(Some(&zero_gloss), use_material_colours),
            highlight_params(Some(&phong([0.9, 0.1, 0.1], 3.0, 1)), true),
            highlight_params(Some(&phong([0.0; 3], 0.0, 0)), true),
            highlight_params(None, true),
        ] {
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
    fn switches_default_on() {
        assert_eq!(MapSwitches::default(), MapSwitches { specular: true, emissive: true, specular_material_colours: false });
    }
}
