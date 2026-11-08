//! Where every ODF lives (S5). Seam type; CON-02 writes the index building
//! (all ODF folders, recursive, A1 `addon/`, duplicate warnings).
//!
//! ODFs are found by file name, wherever they sit: every `.odf` under `odf/`
//! (any depth, any folder name, loose or inside an FPQ) and under A1's
//! `addon/` is a candidate. This is the rule of A2FOExtensions'
//! A2FOFeaturePack recursive ODF index. The winner for a name is chosen by,
//! in order:
//!
//! 1. **Mod root**: the active mod, then its nearest parent, ..., then
//!    `Data` (lower [`Vfs::layers`] index first). Where a file sits never
//!    beats which root it is in: a child's `odf/other/races.odf` wins over a
//!    parent's `odf/system_system/races.odf` (the Noxter bug A2FOExtensions
//!    had to fix).
//! 2. Within one root only: A1's `addon/` (an override folder), then the
//!    built-in folders (`odf/system` for FO; `commands`, `other`, `ships`,
//!    `special_weapons`, `stations`, `weapons` for Armada II), then any other
//!    folder.
//! 3. Loose before packed (FPQ or A1 ZFS), then path order, so the pick is
//!    deterministic.
//!
//! Two files of the same name in one root are ambiguous and give a
//! [`Warning`]; a child overriding a parent is normal and does not.
//!
//! The VFS mounts loose folders recursively without following directory
//! symlinks, so a link loop cannot recurse.
//!
//! [`Vfs::layers`]: sta_vfs::Vfs::layers

use std::collections::BTreeMap;

use sta_vfs::{Source, Vfs};

use crate::Warning;

/// One ODF file found in the mounted install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OdfEntry {
    /// VFS path of the file.
    pub path: String,
    /// Index into `Vfs::layers()`. Note: the VFS puts the active mod at 0 and
    /// `Data` last, so a *lower* index wins.
    pub layer: usize,
}

/// ODF stem (lower-cased, no extension) to the file that wins, plus every
/// stem found more than once.
#[derive(Debug, Clone, Default)]
pub struct OdfIndex {
    entries: BTreeMap<String, OdfEntry>,
    /// Stems with more than one file, each with every file found, winner first.
    duplicates: BTreeMap<String, Vec<OdfEntry>>,
}

/// Armada II's and Fleet Operations' own ODF folders (directly under `odf/`).
const BUILT_IN: [&str; 7] = ["commands", "other", "ships", "special_weapons", "stations", "weapons", "system"];

/// Path below `odf/`. FPQ entries are mounted under the archive's stem, so
/// `Data/ODF.fpq`'s `odf/system/x.odf` appears as `odf/odf/system/x.odf`; both
/// spellings are the same file to the game (the loose copy overrides the
/// packed one), so they compare equal here.
fn odf_rel(path: &str) -> &str {
    let mut rel = path;
    while let Some(r) = rel.strip_prefix("odf/") {
        rel = r;
    }
    rel
}

/// Within-root rank: 0 `addon/`, 1 built-in folder, 2 any other folder.
fn folder_rank(path: &str) -> u8 {
    if path.starts_with("addon/") {
        return 0;
    }
    let rel = odf_rel(path);
    // Armada II nests its own folders (`weapons/phasers/`), so any depth counts.
    match rel.split_once('/') {
        Some((folder, _)) if BUILT_IN.contains(&folder) => 1,
        _ => 2,
    }
}

/// Sort key within one stem: (layer, folder rank, packed, path); smallest wins.
type Rank = (usize, u8, bool, String);

fn stem_of(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    let stem = name.strip_suffix(".odf")?;
    (!stem.is_empty()).then(|| stem.to_owned())
}

impl OdfIndex {
    /// Index every ODF of `vfs`: recursive under `odf/`, plus A1 `addon/`.
    /// Warnings name stems that occur more than once within one root.
    pub fn build(vfs: &Vfs) -> (OdfIndex, Vec<Warning>) {
        let layer_of: BTreeMap<&str, usize> = vfs.layers().iter().enumerate().map(|(i, l)| (l.name(), i)).collect();
        // stem -> (sort key, entry)
        let mut found: BTreeMap<String, Vec<(Rank, OdfEntry)>> = BTreeMap::new();
        for path in vfs.list("odf").into_iter().chain(vfs.list("addon")) {
            let Some(stem) = stem_of(&path) else { continue };
            let rank = folder_rank(&path);
            for (layer_name, source) in vfs.resolve_all(&path) {
                let Some(&layer) = layer_of.get(layer_name) else { continue };
                let packed = !matches!(source, Source::Loose(_));
                let entry = OdfEntry { path: path.clone(), layer };
                found.entry(stem.clone()).or_default().push(((layer, rank, packed, path.clone()), entry));
            }
        }

        let mut index = OdfIndex::default();
        let mut warnings = Vec::new();
        for (stem, mut all) in found {
            all.sort_by(|a, b| a.0.cmp(&b.0));
            // Distinct files per root; a loose file over its own packed copy is one.
            // So is an `addon/` file over a packed copy: patching the archives
            // (A1's `assets.zfs`) is what `addon/` is for.
            let mut by_layer: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
            for ((_, rank, packed, _), e) in &all {
                let paths = by_layer.entry(e.layer).or_default();
                let patched = *packed && paths.first().is_some_and(|p| folder_rank(p) == 0) && *rank > 0;
                if !patched && !paths.iter().any(|p| odf_rel(p) == odf_rel(&e.path)) {
                    paths.push(&e.path);
                }
            }
            for (layer, paths) in by_layer.iter().filter(|(_, p)| p.len() > 1) {
                warnings.push(Warning::new(
                    paths[0],
                    format!(
                        "{stem}.odf found {} times in {}; using it over {}",
                        paths.len(),
                        vfs.layers()[*layer].name(),
                        paths[1..].join(", ")
                    ),
                ));
            }
            let entries: Vec<OdfEntry> = all.into_iter().map(|(_, e)| e).collect();
            index.entries.insert(stem.clone(), entries[0].clone());
            if entries.len() > 1 {
                index.duplicates.insert(stem, entries);
            }
        }
        (index, warnings)
    }

    pub fn get(&self, stem: &str) -> Option<&OdfEntry> {
        self.entries.get(&stem_key(stem))
    }

    /// Every file of `stem`, winner first: the order `sta_odf::resolve_layered`
    /// counts versions in (0 = winner, 1 = the one it hides, ...).
    pub fn versions(&self, stem: &str) -> Vec<&OdfEntry> {
        let key = stem_key(stem);
        match self.duplicates.get(&key) {
            Some(all) => all.iter().collect(),
            None => self.entries.get(&key).into_iter().collect(),
        }
    }

    /// Read `entry` from its own layer (not whichever layer wins its path).
    pub fn read(vfs: &Vfs, entry: &OdfEntry) -> Option<Vec<u8>> {
        let layer = vfs.layers().get(entry.layer)?;
        let (_, source) = vfs.resolve_all(&entry.path).into_iter().find(|(n, _)| *n == layer.name())?;
        vfs.read_from(entry.layer, source).ok()
    }

    /// Text of `name`'s `version`-th file, for `sta_odf::resolve_layered`.
    pub fn load(&self, vfs: &Vfs, name: &str, version: usize) -> Option<String> {
        let entry = *self.versions(name).get(version)?;
        Self::read(vfs, entry).map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// Stems and their winning files, in stem order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &OdfEntry)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Stems found more than once, in stem order.
    pub fn duplicates(&self) -> impl Iterator<Item = (&str, &[OdfEntry])> {
        self.duplicates.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// `"Fed.ODF"`, `"odf/ships/fed.odf"` and `"fed"` all name stem `fed`.
fn stem_key(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    match name.strip_suffix(".odf") {
        Some(s) => s.to_owned(),
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    struct Install(PathBuf);

    impl Install {
        fn new(name: &str) -> Install {
            let d = std::env::temp_dir().join(format!("sta_content_index_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            Install(d)
        }
        fn file(&self, rel: &str, text: &str) -> &Install {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
            self
        }
        /// `Data/Mods/Child` (ParentMod = Parent) over `Data/Mods/Parent` over `Data`.
        fn chain(&self) -> &Install {
            self.file("Data/Mods/Child/info.ini", "[Mod]\nParentMod=Parent\n")
                .file("Data/Mods/Parent/info.ini", "[Mod]\nName=Parent\n")
        }
        fn mount(&self) -> (Vfs, OdfIndex, Vec<Warning>) {
            let vfs = Vfs::mount_install(&self.0, Some("Child")).unwrap();
            let (index, warnings) = OdfIndex::build(&vfs);
            (vfs, index, warnings)
        }
    }

    impl Drop for Install {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn winner<'a>(vfs: &'a Vfs, index: &OdfIndex, stem: &str) -> (&'a str, String) {
        let e = index.get(stem).unwrap();
        (vfs.layers()[e.layer].name(), e.path.clone())
    }

    #[test]
    fn child_built_in_beats_parent_recursive() {
        // The Noxter case.
        let i = Install::new("noxter");
        i.chain()
            .file("Data/Mods/Child/odf/other/races.odf", "who = child\n")
            .file("Data/Mods/Parent/odf/system_system/races.odf", "who = parent\n");
        let (vfs, index, warnings) = i.mount();
        assert_eq!(winner(&vfs, &index, "races"), ("Child", "odf/other/races.odf".into()));
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn child_recursive_beats_parent_built_in() {
        let i = Install::new("inverse");
        i.chain()
            .file("Data/Mods/Child/odf/my_pack/deep/er/races.odf", "who = child\n")
            .file("Data/Mods/Parent/odf/system/races.odf", "who = parent\n")
            .file("Data/odf/ships/races.odf", "who = data\n");
        let (vfs, index, _) = i.mount();
        assert_eq!(winner(&vfs, &index, "races"), ("Child", "odf/my_pack/deep/er/races.odf".into()));
        let order: Vec<_> = index.versions("races").iter().map(|e| vfs.layers()[e.layer].name()).collect();
        assert_eq!(order, ["Child", "Parent", "Data"]);
    }

    #[test]
    fn same_kind_folders_across_roots() {
        let i = Install::new("same");
        i.chain()
            .file("Data/Mods/Child/odf/ships/a.odf", "")
            .file("Data/Mods/Parent/odf/ships/a.odf", "")
            .file("Data/Mods/Child/odf/x/b.odf", "")
            .file("Data/Mods/Parent/odf/y/b.odf", "")
            .file("Data/odf/z/b.odf", "");
        let (vfs, index, warnings) = i.mount();
        assert_eq!(winner(&vfs, &index, "a").0, "Child");
        assert_eq!(winner(&vfs, &index, "b"), ("Child", "odf/x/b.odf".into()));
        assert_eq!(index.versions("b").len(), 3);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn within_one_root_addon_then_built_in_then_other() {
        let i = Install::new("within");
        i.file("Data/addon/fed.odf", "")
            .file("Data/odf/ships/fed.odf", "")
            .file("Data/odf/extra/fed.odf", "")
            .file("Data/odf/ships/kli.odf", "")
            .file("Data/odf/ships/sub/kli.odf", "");
        let vfs = Vfs::mount_install(&i.0, None).unwrap();
        let (index, warnings) = OdfIndex::build(&vfs);
        assert_eq!(index.get("FED.odf").unwrap().path, "addon/fed.odf");
        let fed: Vec<_> = index.versions("fed").iter().map(|e| e.path.as_str()).collect();
        assert_eq!(fed, ["addon/fed.odf", "odf/ships/fed.odf", "odf/extra/fed.odf"]);
        // Same rank (both under `ships/`): path order decides.
        assert_eq!(index.get("kli").unwrap().path, "odf/ships/kli.odf");
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].message.contains("fed.odf found 3 times in Data"), "{}", warnings[0]);
    }

    #[test]
    fn non_odf_and_other_trees_ignored() {
        let i = Install::new("ignore");
        i.file("Data/odf/ships/fed.txt", "").file("Data/sod/fed.odf", "").file("Data/odf/ships/.odf", "");
        let vfs = Vfs::mount_install(&i.0, None).unwrap();
        let (index, _) = OdfIndex::build(&vfs);
        assert!(index.is_empty(), "{:?}", index.iter().collect::<Vec<_>>());
    }

    #[test]
    fn layered_resolve_through_the_index() {
        // A child self-include reaches the parent's copy in another folder.
        let i = Install::new("resolve");
        i.chain()
            .file("Data/Mods/Child/odf/other/fcruiser.odf", "#include \"fcruiser.odf\"\nmaxHealth = 900\n")
            .file("Data/Mods/Parent/odf/system/fcruiser.odf", "#include \"fedbase\"\nmaxHealth = 500\nclassLabel = \"craft\"\n")
            .file("Data/odf/ships/fedbase.odf", "race = \"federation\"\n");
        let (vfs, index, _) = i.mount();
        let odf = sta_odf::resolve_layered("fcruiser", &mut |n, v| index.load(&vfs, n, v)).unwrap();
        assert_eq!(odf.i64("maxHealth"), Some(900));
        assert_eq!(odf.class_label(), Some("craft"));
        assert_eq!(odf.str("race"), Some("federation"));
        assert_eq!(OdfIndex::read(&vfs, index.versions("fcruiser")[1]).unwrap(), std::fs::read(i.0.join("Data/Mods/Parent/odf/system/fcruiser.odf")).unwrap());
    }

    #[test]
    fn folder_ranks() {
        assert_eq!(folder_rank("addon/x.odf"), 0);
        assert_eq!(folder_rank("odf/system/x.odf"), 1);
        assert_eq!(folder_rank("odf/odf/system/x.odf"), 1); // Data/ODF.fpq
        assert_eq!(folder_rank("odf/special_weapons/x.odf"), 1);
        assert_eq!(folder_rank("odf/weapons/phasers/x.odf"), 1);
        assert_eq!(folder_rank("odf/system_system/x.odf"), 2);
        assert_eq!(folder_rank("odf/x.odf"), 2);
        assert_eq!(stem_key("odf\\Ships\\Fed.ODF"), "fed");
        assert_eq!(odf_rel("odf/odf/system/x.odf"), odf_rel("odf/system/x.odf"));
    }
}
