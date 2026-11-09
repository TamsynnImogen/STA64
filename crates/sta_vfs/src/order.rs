//! Load order (CON-18): which mods mount, top first, for an active mod.
//!
//! 1. Each mod's [`Manifest`]: its loose `manifest.toml`, else one inside its
//!    root `.sta`, else generated from `info.ini` ([`Manifest::from_info`],
//!    so `ParentMod` is a dependency).
//! 2. From the active mod, every dependency is followed by id, transitively.
//!    A missing dependency, a version that does not meet the requirement, two
//!    mods claiming one id, and a dependency loop are errors naming the mods.
//! 3. The mods are ordered so each sits above everything it depends on. Where
//!    that leaves a choice, `[load] after/before` hints decide (a hint that
//!    would contradict a dependency is ignored), then the id.
//!
//! For legacy mods this is exactly [`crate::mod_chain`]: one parent each, so
//! the order is the child, then its parent, and so on.
//!
//! [`LoadOrder::identity`] hashes the order for content checks: each mod's
//! id and version, and for a packed mod each root `.sta`'s `content_hash`.
//! Loose folders contribute id and version only (hashing every loose file is
//! left to whoever needs it).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::manifest::{Manifest, ManifestError};
use crate::mods::ModInfo;
use crate::sta::StaArchive;
use crate::{Vfs, VfsError};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OrderError {
    #[error("mod {0:?} not found")]
    NotFound(String),
    #[error("{folder}: {error}")]
    Manifest { folder: String, error: ManifestError },
    #[error("{dependent} needs {dependency:?}, which is not installed")]
    MissingDependency { dependent: String, dependency: String },
    #[error("{dependent} needs {dependency} {requirement}, but {found} is installed")]
    WrongVersion { dependent: String, dependency: String, requirement: String, found: String },
    #[error("mods {0:?} all claim id {1:?}")]
    DuplicateId(Vec<String>, String),
    #[error("dependency loop: {0}")]
    Cycle(String),
}

/// A mod's root `.sta` archives: (file name, `content_hash`).
pub type StaIds = Vec<(String, [u8; 32])>;

/// One mod in a load order.
#[derive(Debug, Clone, PartialEq)]
pub struct Ordered {
    pub folder: String,
    pub path: PathBuf,
    pub manifest: Manifest,
    /// Root `.sta` archives: (file name, `content_hash`).
    pub sta: StaIds,
}

/// Mods to mount, highest priority first.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadOrder {
    pub mods: Vec<Ordered>,
}

impl LoadOrder {
    /// Folder names, top first (what [`crate::mod_chain`] returns for legacy mods).
    pub fn folders(&self) -> Vec<&str> {
        self.mods.iter().map(|m| m.folder.as_str()).collect()
    }

    /// A hash of the order: per mod, top first, its id, version and the
    /// `content_hash` of each root `.sta`.
    pub fn identity(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        for m in &self.mods {
            h.update(m.manifest.id.as_bytes());
            h.update([0]);
            h.update(m.manifest.version.to_string().as_bytes());
            h.update([0]);
            for (name, hash) in &m.sta {
                h.update(name.to_ascii_lowercase().as_bytes());
                h.update(hash);
            }
            h.update([0xff]);
        }
        h.finalize().into()
    }

    /// Mount the order over `data` (the install's `Data` folder), as
    /// [`Vfs::mount_install`] mounts a `ParentMod` chain.
    pub fn mount(&self, data: &Path) -> Result<Vfs, VfsError> {
        let mut vfs = Vfs::default();
        for m in &self.mods {
            vfs.push_dir(m.folder.clone(), m.path.clone())?;
        }
        vfs.push_dir("Data".into(), data.to_path_buf())?;
        Ok(vfs)
    }
}

fn child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e| e.path())
}

/// A mod's manifest and its root `.sta` identities.
pub fn manifest_for(info: &ModInfo) -> Result<(Manifest, StaIds), ManifestError> {
    let mut sta = Vec::new();
    let mut packed_manifest = None;
    for a in info.archives.iter().filter(|a| a.to_ascii_lowercase().ends_with(".sta")) {
        let Ok(archive) = StaArchive::open(&info.path.join(a)) else { continue };
        sta.push((a.clone(), archive.content_hash()));
        if packed_manifest.is_none()
            && let Some(i) = archive.find("manifest.toml")
            && let Ok(bytes) = archive.read(i)
        {
            packed_manifest = Some(String::from_utf8_lossy(&bytes).into_owned());
        }
    }
    let loose = child_ci(&info.path, "manifest.toml").and_then(|p| std::fs::read_to_string(p).ok());
    let manifest = match loose.or(packed_manifest) {
        Some(text) => Manifest::parse(&text)?,
        None => Manifest::from_info(info),
    };
    Ok((manifest, sta))
}

/// The load order for `active` among `mods` (from [`crate::mods::list_mods`]).
pub fn resolve(mods: &[ModInfo], active: &str) -> Result<LoadOrder, OrderError> {
    let start = mods.iter().find(|m| m.folder.eq_ignore_ascii_case(active)).ok_or_else(|| OrderError::NotFound(active.to_owned()))?;
    // Manifests are read lazily: a broken one only matters if it is reached.
    let mut cache: BTreeMap<String, Ordered> = BTreeMap::new(); // folder (lower) -> mod
    let mut load = |info: &ModInfo| -> Result<Ordered, OrderError> {
        let key = info.folder.to_ascii_lowercase();
        if let Some(o) = cache.get(&key) {
            return Ok(o.clone());
        }
        let (manifest, sta) = manifest_for(info).map_err(|error| OrderError::Manifest { folder: info.folder.clone(), error })?;
        let o = Ordered { folder: info.folder.clone(), path: info.path.clone(), manifest, sta };
        cache.insert(key, o.clone());
        Ok(o)
    };
    // id -> mods, built from every mod's manifest id; errors only for mods reached.
    let mut by_id: BTreeMap<String, Vec<&ModInfo>> = BTreeMap::new();
    for info in mods {
        let id = match load(info) {
            Ok(o) => o.manifest.id,
            Err(_) => crate::convert::mod_id(&info.folder),
        };
        by_id.entry(id).or_default().push(info);
    }
    let lookup = |id: &str, dependent: &str| -> Result<&ModInfo, OrderError> {
        match by_id.get(id).map(Vec::as_slice) {
            None | Some([]) => Err(OrderError::MissingDependency { dependent: dependent.to_owned(), dependency: id.to_owned() }),
            Some([one]) => Ok(one),
            Some(many) => Err(OrderError::DuplicateId(many.iter().map(|m| m.folder.clone()).collect(), id.to_owned())),
        }
    };

    // Depth-first from the active mod: version checks and loop detection.
    let mut set: BTreeMap<String, Ordered> = BTreeMap::new(); // id -> mod
    let mut stack: Vec<String> = Vec::new();
    fn visit<'m>(
        o: Ordered,
        set: &mut BTreeMap<String, Ordered>,
        stack: &mut Vec<String>,
        load: &mut dyn FnMut(&ModInfo) -> Result<Ordered, OrderError>,
        lookup: &dyn Fn(&str, &str) -> Result<&'m ModInfo, OrderError>,
    ) -> Result<(), OrderError> {
        let id = o.manifest.id.clone();
        if let Some(at) = stack.iter().position(|s| *s == id) {
            let mut path = stack[at..].to_vec();
            path.push(id);
            return Err(OrderError::Cycle(path.join(" -> ")));
        }
        if set.contains_key(&id) {
            return Ok(());
        }
        stack.push(id.clone());
        for (dep, req) in &o.manifest.dependencies {
            let info = lookup(dep, &o.folder)?;
            let d = load(info)?;
            if !d.manifest.version.satisfies(req) {
                return Err(OrderError::WrongVersion {
                    dependent: o.folder.clone(),
                    dependency: d.folder.clone(),
                    requirement: req.to_string(),
                    found: d.manifest.version.to_string(),
                });
            }
            visit(d, set, stack, load, lookup)?;
        }
        stack.pop();
        set.insert(id, o);
        Ok(())
    }
    let first = load(start)?;
    visit(first, &mut set, &mut stack, &mut load, &lookup)?;

    // Edges "a below b" (a must mount under b): every dependency under its
    // dependent; then hints that do not close a loop.
    let ids: Vec<String> = set.keys().cloned().collect();
    let mut below: BTreeMap<String, BTreeSet<String>> = ids.iter().map(|i| (i.clone(), BTreeSet::new())).collect(); // b -> {a below b}
    for (id, o) in &set {
        for dep in o.manifest.dependencies.keys() {
            below.get_mut(id).unwrap().insert(dep.clone());
        }
    }
    let reaches = |below: &BTreeMap<String, BTreeSet<String>>, from: &str, to: &str| -> bool {
        let mut seen = BTreeSet::new();
        let mut todo = vec![from.to_owned()];
        while let Some(x) = todo.pop() {
            if x == to {
                return true;
            }
            if seen.insert(x.clone()) {
                todo.extend(below.get(&x).into_iter().flatten().cloned());
            }
        }
        false
    };
    for (id, o) in &set {
        // `after X`: this mod mounts above X (X below this).
        for x in o.manifest.load_after.iter().filter(|x| set.contains_key(*x)) {
            if !reaches(&below, x, id) {
                below.get_mut(id).unwrap().insert(x.clone());
            }
        }
        // `before X`: this mod mounts below X.
        for x in o.manifest.load_before.iter().filter(|x| set.contains_key(*x)) {
            if !reaches(&below, id, x) {
                below.get_mut(x).unwrap().insert(id.clone());
            }
        }
    }
    // Bottom-up (Kahn): a mod is placed once everything below it is; ties by id.
    let mut placed: Vec<String> = Vec::new();
    let mut done = BTreeSet::new();
    while placed.len() < ids.len() {
        let next = ids
            .iter()
            .filter(|i| !done.contains(*i))
            .find(|i| below[*i].iter().all(|d| done.contains(d)))
            .cloned()
            .expect("the dependency graph was checked for loops");
        done.insert(next.clone());
        placed.push(next);
    }
    placed.reverse();
    Ok(LoadOrder { mods: placed.into_iter().map(|id| set.remove(&id).unwrap()).collect() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::list_mods;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn install(name: &str, mods: &[(&str, &str, Option<&str>)]) -> Tmp {
        let d = std::env::temp_dir().join(format!("sta_vfs_order_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Data/Mods")).unwrap();
        for (folder, info, manifest) in mods {
            let p = d.join("Data/Mods").join(folder);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("info.ini"), info).unwrap();
            if let Some(m) = manifest {
                std::fs::write(p.join("manifest.toml"), m).unwrap();
            }
        }
        Tmp(d)
    }

    fn order(t: &Tmp, active: &str) -> Result<Vec<String>, OrderError> {
        let (mods, _) = list_mods(&t.0.join("Data")).unwrap();
        resolve(&mods, active).map(|o| o.folders().into_iter().map(str::to_owned).collect())
    }

    #[test]
    fn legacy_chain_matches_mod_chain() {
        let t = install("legacy", &[("Fleet Ops 4.0", "[mod]\nVersion=4.0Z\n", None), ("Noxter", "[mod]\nParentMod=Fleet Ops 4.0\n", None), ("Sub", "[mod]\nParentMod=Noxter\n", None)]);
        assert_eq!(order(&t, "sub").unwrap(), ["Sub", "Noxter", "Fleet Ops 4.0"]);
        let chain: Vec<String> = crate::mod_chain(&t.0.join("Data"), "Sub").unwrap().into_iter().map(|(n, _)| n).collect();
        assert_eq!(order(&t, "Sub").unwrap(), chain);
    }

    #[test]
    fn diamond_ties_hints_and_versions() {
        // top needs left and right; both need base. Without hints: ties by id.
        let m = |id: &str, deps: &str, load: &str| format!("[mod]\nid='{id}'\nversion='1.4.0'\n[dependencies]\n{deps}\n[load]\n{load}\n");
        let t = install(
            "diamond",
            &[
                ("Top", "[mod]\n", Some(&m("top", "left='^1'\nright='>=1.2'", ""))),
                ("Left", "[mod]\n", Some(&m("left", "base='*'", ""))),
                ("Right", "[mod]\n", Some(&m("right", "base='1.4'", ""))),
                ("Base", "[mod]\n", Some(&m("base", "", ""))),
            ],
        );
        assert_eq!(order(&t, "Top").unwrap(), ["Top", "Right", "Left", "Base"]);
        // A hint flips the free pair; one contradicting a dependency is ignored.
        let t2 = install(
            "hints",
            &[
                ("Top", "[mod]\n", Some(&m("top", "left='*'\nright='*'", ""))),
                ("Left", "[mod]\n", Some(&m("left", "base='*'", "after=['right']"))),
                ("Right", "[mod]\n", Some(&m("right", "base='*'", ""))),
                ("Base", "[mod]\n", Some(&m("base", "", "after=['top']"))),
            ],
        );
        assert_eq!(order(&t2, "Top").unwrap(), ["Top", "Left", "Right", "Base"]);
    }

    #[test]
    fn refusals() {
        let t = install(
            "bad",
            &[
                ("Needs Gone", "[mod]\nParentMod=Gone\n", None),
                ("Old", "[mod]\n", Some("[mod]\nid='old'\nversion='1.0.0'\n")),
                ("Wants New", "[mod]\n", Some("[mod]\nid='wants-new'\n[dependencies]\nold='^2'\n")),
                ("Legacy Ver", "[mod]\nVersion=4.0Z\n", None),
                ("Wants Range", "[mod]\n", Some("[mod]\nid='wants-range'\n[dependencies]\nlegacy-ver='^4'\n")),
                ("A", "[mod]\nParentMod=B\n", None),
                ("B", "[mod]\nParentMod=A\n", None),
                ("Twin One", "[mod]\n", Some("[mod]\nid='twin'\n")),
                ("Twin Two", "[mod]\n", Some("[mod]\nid='twin'\n")),
                ("Wants Twin", "[mod]\n", Some("[mod]\nid='wants-twin'\n[dependencies]\ntwin='*'\n")),
                ("Broken", "[mod]\n", Some("[mod\n")),
            ],
        );
        assert_eq!(order(&t, "Needs Gone").unwrap_err(), OrderError::MissingDependency { dependent: "Needs Gone".into(), dependency: "gone".into() });
        assert!(matches!(order(&t, "Wants New").unwrap_err(), OrderError::WrongVersion { found, .. } if found == "1.0.0"));
        assert!(matches!(order(&t, "Wants Range").unwrap_err(), OrderError::WrongVersion { found, .. } if found == "4.0Z"));
        assert_eq!(order(&t, "A").unwrap_err(), OrderError::Cycle("a -> b -> a".into()));
        assert!(matches!(order(&t, "Wants Twin").unwrap_err(), OrderError::DuplicateId(_, id) if id == "twin"));
        assert!(matches!(order(&t, "Broken").unwrap_err(), OrderError::Manifest { .. }));
        assert_eq!(order(&t, "nope").unwrap_err(), OrderError::NotFound("nope".into()));
        // Broken mods elsewhere don't stop a valid order.
        assert_eq!(order(&t, "Old").unwrap(), ["Old"]);
    }

    #[test]
    fn packed_manifest_identity_and_mount() {
        let t = install("packed", &[("Base", "[mod]\nVersion=1.0.0\n", None)]);
        let p = t.0.join("Data/Mods/Packed");
        std::fs::create_dir_all(&p).unwrap();
        let sta = crate::sta_write::pack(
            vec![
                ("info.ini".into(), crate::sta_write::Content::Data(b"[mod]\nName=Packed\n".to_vec())),
                ("manifest.toml".into(), crate::sta_write::Content::Data(b"[mod]\nid='packed'\nversion='2.0.0'\n[dependencies]\nbase='1'\n".to_vec())),
                ("odf/ships/x.odf".into(), crate::sta_write::Content::Data(b"classLabel = \"craft\"\n".to_vec())),
            ],
            crate::sta_write::Compression::Auto,
        )
        .unwrap();
        std::fs::write(p.join("packed.sta"), &sta).unwrap();
        std::fs::create_dir_all(t.0.join("Data/odf")).unwrap();
        let (mods, _) = list_mods(&t.0.join("Data")).unwrap();
        let o = resolve(&mods, "Packed").unwrap();
        assert_eq!(o.folders(), ["Packed", "Base"]);
        assert_eq!(o.mods[0].manifest.id, "packed");
        assert_eq!(o.mods[0].sta.len(), 1);
        let id1 = o.identity();
        // Same content, same identity; a different version, a different one.
        assert_eq!(resolve(&mods, "Packed").unwrap().identity(), id1);
        let mut o2 = o.clone();
        o2.mods[1].manifest.version = crate::manifest::ModVersion::parse("1.0.1");
        assert_ne!(o2.identity(), id1);
        let vfs = o.mount(&t.0.join("Data")).unwrap();
        assert_eq!(vfs.read("odf/ships/x.odf").unwrap().unwrap(), b"classLabel = \"craft\"\n");
    }

    /// With STA64_TEST_INSTALL: every installed mod orders as mod_chain mounts it.
    #[test]
    fn real_install_if_available() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let data = child_ci(Path::new(&root), "Data").expect("Data");
        let (mods, _) = list_mods(&data).unwrap();
        let mut same = 0;
        for m in &mods {
            let ours = resolve(&mods, &m.folder).map(|o| o.folders().iter().map(|f| f.to_ascii_lowercase()).collect::<Vec<_>>());
            let theirs = crate::mod_chain(&data, &m.folder).map(|c| c.into_iter().map(|(n, _)| n.to_ascii_lowercase()).collect::<Vec<_>>());
            match (&ours, &theirs) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a, b, "{}", m.folder);
                    same += 1;
                }
                (Err(e), Err(_)) => eprintln!("  {}: both refuse ({e})", m.folder),
                _ => panic!("{}: order {ours:?}, mod_chain {theirs:?}", m.folder),
            }
        }
        eprintln!("{} mods, {same} orders equal to mod_chain", mods.len());
    }
}
