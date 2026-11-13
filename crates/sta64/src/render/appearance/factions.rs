//! Static race appearance catalogue and faction texture variant lookup.

use std::collections::{BTreeMap, BTreeSet};

use bevy::log::warn;
use sta_assets::TextureFormat;
use sta_content::{OdfIndex, Races};
use sta_vfs::Vfs;

use super::policy::parse_appearance_policy;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Faction {
    /// The validated internal race name used by SOD nodes.
    pub name: String,
    pub suffix: Option<String>,
}

/// Built once per mounted content set. Aliases share their selected race's
/// appearance; no cached model or simulation state is changed here.
#[derive(Debug, Default)]
pub struct FactionCatalogue {
    factions: Vec<Faction>,
    aliases: BTreeMap<String, usize>,
    node_names: BTreeSet<String>,
}

impl FactionCatalogue {
    pub fn load(vfs: &Vfs) -> Self {
        let (index, index_warnings) = OdfIndex::build(vfs);
        let (races, race_warnings) = Races::load(vfs, &index);
        for warning in index_warnings.into_iter().chain(race_warnings) {
            warn!("faction catalogue: {warning}");
        }
        let mut catalogue = Self::default();
        let mut names = Vec::new();
        for race in races.iter() {
            if !valid_identifier(&race.name, 63) || !valid_identifier(&race.stem, 63) {
                warn!(
                    "faction catalogue: {}: invalid race name or stem",
                    race.stem
                );
                continue;
            }
            let odf = match sta_odf::resolve_layered(&race.stem, &mut |name, version| {
                index.load(vfs, name, version)
            }) {
                Ok(odf) => odf,
                Err(error) => {
                    warn!("faction catalogue: {}.odf: {error}", race.stem);
                    continue;
                }
            };
            let (policy, diagnostics) = parse_appearance_policy(&odf);
            for diagnostic in diagnostics
                .into_iter()
                .filter(|d| d.starts_with("factionTextureSuffix:"))
            {
                warn!("faction catalogue: {}.odf: {diagnostic}", race.stem);
            }
            let id = catalogue.factions.len();
            catalogue.factions.push(Faction {
                name: race.name.clone(),
                suffix: policy.faction_suffix,
            });
            catalogue.aliases.insert(race.stem.to_ascii_lowercase(), id);
            if !race.name.eq_ignore_ascii_case("borg") {
                catalogue.node_names.insert(race.name.to_ascii_lowercase());
            }
            names.push((race.name.to_ascii_lowercase(), id, race.slot.is_some()));
        }
        // Races::get gives a stem precedence, then a playable race's internal
        // name, then any race with that name. Preserve that alias ordering.
        for (name, id, _) in names.iter().filter(|(_, _, playable)| *playable) {
            catalogue.aliases.entry(name.clone()).or_insert(*id);
        }
        for (name, id, _) in names {
            catalogue.aliases.entry(name).or_insert(id);
        }
        catalogue
    }

    pub fn get(&self, identifier: &str) -> Option<&Faction> {
        let key = identifier.trim().to_ascii_lowercase();
        self.factions.get(*self.aliases.get(&key)?)
    }

    /// Whether a SOD node is a loaded race's internal node, excluding the
    /// reserved Borg node whose native policy remains authoritative.
    pub fn is_faction_node(&self, name: &str) -> bool {
        self.node_names.contains(&name.to_ascii_lowercase())
    }
}

fn valid_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Insert a validated suffix before a known image extension while preserving
/// the caller's directory and extension spelling. Unknown final dots remain
/// part of the stem, as with the asset resolver's `MbgDom1.3` convention.
pub fn suffixed_name(base: &str, suffix: &str) -> Option<String> {
    if base.is_empty() || !valid_identifier(suffix, 32) {
        return None;
    }
    let basename = base.rsplit(['/', '\\']).next()?;
    if basename.is_empty() {
        return None;
    }
    let extension_start = basename.rfind('.').filter(|&dot| {
        dot > 0
            && ["dds", "tga", "png", "bmp"]
                .iter()
                .any(|ext| basename[dot + 1..].eq_ignore_ascii_case(ext))
    });
    let stem_end = extension_start.map_or(base.len(), |dot| base.len() - basename.len() + dot);
    Some(format!(
        "{}{}{}",
        &base[..stem_end],
        suffix,
        &base[stem_end..]
    ))
}

/// Find only authored faction variants. DDS wins over TGA, including across
/// layers; within one format the mounted VFS decides the winning layer. These
/// are the same two flat roots used by `sta_assets::find_texture`.
pub fn find_faction_texture(
    vfs: &Vfs,
    base: &str,
    suffix: &str,
) -> Option<(String, TextureFormat)> {
    let name = suffixed_name(base, suffix)?;
    let basename = name.rsplit(['/', '\\']).next()?;
    let stem = basename
        .rfind('.')
        .filter(|&dot| {
            dot > 0
                && ["dds", "tga", "png", "bmp"]
                    .iter()
                    .any(|ext| basename[dot + 1..].eq_ignore_ascii_case(ext))
        })
        .map_or(basename, |dot| &basename[..dot]);
    for (format, extension) in [(TextureFormat::Dds, "dds"), (TextureFormat::Tga, "tga")] {
        let mut best: Option<(usize, String)> = None;
        for root in ["textures", "textures/rgb"] {
            let path = format!("{root}/{stem}.{extension}").to_ascii_lowercase();
            if let Some((layer, _)) = vfs.resolve(&path)
                && best.as_ref().is_none_or(|(old, _)| layer < *old)
            {
                best = Some((layer, path));
            }
        }
        if let Some((_, path)) = best {
            return Some((path, format));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Install(PathBuf);

    impl Install {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "sta64_factions_{}_{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, path: &str, data: &str) {
            let path = self.0.join("Data").join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, data).unwrap();
        }

        fn mount(&self) -> Vfs {
            Vfs::mount_install(&self.0, None).unwrap()
        }
    }

    impl Drop for Install {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn aliases_internal_names_and_missing_suffix_keep_node_policy() {
        let install = Install::new();
        install.file(
            "odf/other/races.odf",
            "numberOfRaces = 2\nrace0 = \"federation.odf\"\nrace1 = \"klingon.odf\"\n",
        );
        install.file(
            "odf/other/federation.odf",
            "name = \"UnitedFederation\"\ninstantActionSlot = 0\nfactionTextureSuffix = \"_fed\"\n",
        );
        install.file(
            "odf/other/klingon.odf",
            "name = \"Klingon\"\ninstantActionSlot = 1\nfactionTextureSuffix = \"../bad\"\n",
        );
        let catalogue = FactionCatalogue::load(&install.mount());
        assert_eq!(
            catalogue.get("FEDERATION"),
            catalogue.get("unitedfederation")
        );
        assert_eq!(
            catalogue.get("federation").unwrap().suffix.as_deref(),
            Some("_fed")
        );
        assert_eq!(catalogue.get("KlInGoN").unwrap().suffix, None);
        assert!(catalogue.is_faction_node("UNITEDFEDERATION"));
        assert!(catalogue.is_faction_node("klingon"));
        assert!(!catalogue.is_faction_node("federation"));
        assert!(!catalogue.is_faction_node("borg"));
    }

    #[test]
    fn suffixed_names_preserve_parent_and_known_extension() {
        assert_eq!(
            suffixed_name("Textures/RGB/Hull.TGA", "_fed"),
            Some("Textures/RGB/Hull_fed.TGA".into())
        );
        assert_eq!(
            suffixed_name("Textures\\Hull.png", "-a"),
            Some("Textures\\Hull-a.png".into())
        );
        assert_eq!(suffixed_name("MbgDom1.3", "_x"), Some("MbgDom1.3_x".into()));
        assert_eq!(suffixed_name("hull.dds", "../bad"), None);
        assert_eq!(suffixed_name("hull.dds", ""), None);
    }

    #[test]
    fn texture_variants_prefer_dds_then_tga_and_do_not_fall_back() {
        let install = Install::new();
        install.file("Textures/RGB/hull_fed.tga", "synthetic tga");
        install.file("Textures/hull_fed.dds", "synthetic dds");
        install.file("Textures/RGB/other_fed.tga", "synthetic tga");
        install.file("Textures/hull.dds", "base only");
        let vfs = install.mount();
        assert_eq!(
            find_faction_texture(&vfs, "Textures/RGB/Hull.TGA", "_fed"),
            Some(("textures/hull_fed.dds".into(), TextureFormat::Dds))
        );
        assert_eq!(
            find_faction_texture(&vfs, "other.png", "_fed"),
            Some(("textures/rgb/other_fed.tga".into(), TextureFormat::Tga))
        );
        assert_eq!(find_faction_texture(&vfs, "hull", "_missing"), None);
    }
}
