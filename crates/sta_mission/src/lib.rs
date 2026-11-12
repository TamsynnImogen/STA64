//! Which mission script a map runs (see `docs/MISSIONS.md`).
//!
//! A native STA64 build cannot load the 32-bit mission DLLs (`*R.drl`, `*S.dsl`,
//! `mmm_loader.dll`). Fleet Operations' MMM framework runs `missions/<title>.lua`
//! where the title is the game setup's map title; STA64 runs the same Lua in its
//! own runtime. Candidate names are tried in order and the first existing script
//! wins:
//!
//! 1. the map's titles in `label.map` (`+ GoodDayToDie.bzn {A Good Day to Die}`),
//! 2. the BZN file name without extension,
//! 3. the map's description string.
//!
//! If no Lua exists but the map names legacy DLL scripts, the map loads as a
//! plain scenario and is listed for conversion. Nothing here is ever an error
//! that stops a map loading.

use sta_content::strings::LabelMap;
use sta_vfs::Vfs;

/// `label.map`: `section { + key {Text} }`. Parsing lives in
/// [`sta_content::strings::LabelMap`] (CON-11); this wraps it for the resolver.
#[derive(Debug, Default)]
pub struct Labels(LabelMap);

impl Labels {
    pub fn parse(text: &str) -> Labels {
        Labels(LabelMap::parse(text))
    }

    /// Reads `label.map` from every VFS layer (mods extend the base game's).
    pub fn load(vfs: &Vfs) -> Labels {
        Labels(LabelMap::load(vfs))
    }

    pub fn get(&self, key: &str) -> &[String] {
        self.0.get(key)
    }
}

pub mod api;
pub mod math;
pub mod runtime;
pub mod sandbox;
pub mod script;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchedBy {
    Label,
    FileName,
    Description,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// Run this Lua script in the integrated MMM runtime.
    Lua { path: String, matched_by: MatchedBy },
    /// Only legacy DLL scripts exist: a plain scenario, listed for Lua conversion.
    LegacyDll { files: Vec<String> },
    /// No script: a plain scenario (instant action maps, `EmptyMission`, ...).
    None,
}

/// Mission script names that are framework loaders or placeholders, never content.
fn is_loader(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l == "mmm_loader.dll" || l.starts_with("emptymission")
}

/// Resolve the mission for `bzn` (a file name like `GoodDayToDie.bzn`).
pub fn resolve(vfs: &Vfs, labels: &Labels, bzn: &str, map: &sta_bzn::Map) -> Resolution {
    let stem = bzn.rsplit(['/', '\\']).next().unwrap_or(bzn);
    let stem = stem.strip_suffix(".bzn").or_else(|| stem.strip_suffix(".BZN")).unwrap_or(stem);
    let file = format!("{stem}.bzn");
    let mut candidates: Vec<(String, MatchedBy)> = labels.get(&file).iter().map(|t| (t.clone(), MatchedBy::Label)).collect();
    candidates.push((stem.to_owned(), MatchedBy::FileName));
    if !map.description.is_empty() {
        candidates.push((map.description.clone(), MatchedBy::Description));
    }
    for (name, by) in candidates {
        let path = format!("missions/{}.lua", name.trim());
        if vfs.resolve(&path).is_some() {
            return Resolution::Lua { path, matched_by: by };
        }
    }
    let legacy: Vec<String> = map
        .scripts
        .iter()
        .filter(|s| !is_loader(s))
        .filter(|s| vfs.resolve(&format!("missions/{s}")).is_some() || vfs.resolve(s).is_some())
        .cloned()
        .collect();
    if legacy.is_empty() { Resolution::None } else { Resolution::LegacyDll { files: legacy } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn labels_parse_sections_and_keys() {
        let l = Labels::parse("multi {\n\t+ title\t{ }\n    + GoodDayToDie.bzn {A Good Day to Die}\n\t+ kling4.bzn\t{A Good Day to Die}\n}\n");
        assert_eq!(l.get("gooddaytodie.BZN"), ["A Good Day to Die"]);
        assert!(l.get("title").is_empty(), "blank labels are skipped");
    }

    #[test]
    fn resolution_order() {
        let root = std::env::temp_dir().join(format!("sta_mission_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = |p: &str| {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        };
        w("Data/missions/A Good Day to Die.lua");
        w("Data/missions/Eviction.lua");
        w("Data/missions/a2_borg01R.drl");
        let vfs = Vfs::mount_install(&root, None).unwrap();
        let labels = Labels::parse("x {\n+ GoodDayToDie.bzn {A Good Day to Die}\n+ Eviction.bzn {Species 8472 vs Borg: Eviction}\n}\n");
        let map = sta_bzn::Map::default();
        assert!(matches!(resolve(&vfs, &labels, "GoodDayToDie.bzn", &map), Resolution::Lua { matched_by: MatchedBy::Label, .. }));
        assert!(matches!(resolve(&vfs, &labels, "bzn/Eviction.bzn", &map), Resolution::Lua { matched_by: MatchedBy::FileName, .. }));
        let borg = sta_bzn::Map {
            scripts: vec!["a2_borg01R.drl".into()],
            ..Default::default()
        };
        assert_eq!(resolve(&vfs, &labels, "a2_borg01.bzn", &borg), Resolution::LegacyDll { files: vec!["a2_borg01R.drl".into()] });
        let mmm = sta_bzn::Map {
            scripts: vec!["mmm_loader.dll".into()],
            ..Default::default()
        };
        assert_eq!(resolve(&vfs, &labels, "skirmish.bzn", &mmm), Resolution::None);
    }

    /// With STA64_TEST_INSTALL / STA64_TEST_MOD: resolve every map, print the tally.
    #[test]
    fn real_install_if_available() {
        let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else { return };
        let vfs = Vfs::mount_install(Path::new(&root), Some(&m)).unwrap();
        let labels = Labels::load(&vfs);
        let mut tally: std::collections::BTreeMap<String, usize> = Default::default();
        let mut lua_files = std::collections::BTreeSet::new();
        for p in vfs.list("bzn").into_iter().filter(|p| p.ends_with(".bzn")) {
            let Some(Ok(d)) = vfs.read(&p) else { continue };
            let Ok(map) = sta_bzn::parse(&d) else { continue };
            let r = resolve(&vfs, &labels, &p, &map);
            let k = match &r {
                Resolution::Lua { matched_by, path } => {
                    lua_files.insert(path.clone());
                    format!("lua by {matched_by:?}")
                }
                Resolution::LegacyDll { .. } => "legacy dll".into(),
                Resolution::None => "none".into(),
            };
            *tally.entry(k).or_default() += 1;
        }
        let all_lua = vfs.list("missions").into_iter().filter(|p| p.ends_with(".lua")).count();
        eprintln!("missions: {tally:?}; {} distinct Lua scripts used of {all_lua} in missions/", lua_files.len());
    }
}
