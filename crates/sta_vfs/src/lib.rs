//! Layered virtual filesystem.
//!
//! An install is mounted as an ordered stack of layers, highest priority first:
//! `active mod -> its ParentMod chain -> Data`. Lookups are case-insensitive on
//! every platform, because Armada-era content mixes case freely and was only
//! ever tested on Windows.
//!
//! Each layer is a directory plus any `*.fpq` and Armada I `*.zfs` archives at
//! its root. An archive named `odf.fpq` is mounted under `odf/`. ZFS archives
//! have no folders; their entries mount where Armada II keeps each type:
//! `.odf` under `odf/`, `.tt` under `techtree/`, anything else at the root.
//! STA64's own `*.sta` archives (`docs/STA_ARCHIVE.md`) mount at the layer's
//! root with their paths as written.
//!
//! Inside one layer, loose files win over `.sta` entries, which win over FPQ
//! entries, which win over ZFS entries (loose over FPQ matches Fleet
//! Operations; loose over `.sta` lets a modder test a change without
//! repacking). A `.sta` whiteout hides its path in every lower layer.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub mod sta;
pub mod sta_write;
pub mod zfs;

#[derive(Debug, thiserror::Error)]
pub enum VfsError {
    #[error("no Data directory under {0}")]
    NoDataDir(PathBuf),
    #[error("mod {0:?} not found under Data/Mods")]
    ModNotFound(String),
    #[error("ParentMod cycle: {0}")]
    ParentCycle(String),
    #[error("archive {path}: {source}")]
    Archive {
        path: PathBuf,
        source: sta_fpq::FpqError,
    },
    #[error("archive {path}: {source}")]
    Zfs {
        path: PathBuf,
        source: zfs::ZfsError,
    },
    #[error("archive {path}: {source}")]
    Sta {
        path: PathBuf,
        source: sta::StaError,
    },
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// One mounted layer: a directory and the FPQ and ZFS archives at its root.
pub struct Layer {
    pub name: String,
    pub root: PathBuf,
    pub archives: Vec<(PathBuf, sta_fpq::Fpq)>,
    pub zfs: Vec<(PathBuf, zfs::Zfs)>,
    pub sta: Vec<(PathBuf, sta::StaArchive)>,
}

impl Layer {
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Layer")
            .field("name", &self.name)
            .field("root", &self.root)
            .field("archives", &self.archives.iter().map(|(p, _)| p).collect::<Vec<_>>())
            .field("zfs", &self.zfs.iter().map(|(p, _)| p).collect::<Vec<_>>())
            .field("sta", &self.sta.iter().map(|(p, _)| p).collect::<Vec<_>>())
            .finish()
    }
}

/// Where a virtual file physically lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Loose(PathBuf),
    /// An entry of the layer's `archives[archive]` (FPQ).
    Packed { archive: usize, entry: usize },
    /// An entry of the layer's `zfs[archive]` (Armada I).
    Zfs { archive: usize, entry: usize },
    /// An entry of the layer's `sta[archive]` (STA64).
    Sta { archive: usize, entry: usize },
    /// A `.sta` whiteout: the path is hidden in this layer and every lower one.
    /// [`Vfs::resolve`], [`Vfs::resolve_all`] and [`Vfs::list`] never hand it out.
    Whiteout,
}

/// Where a ZFS entry's flat name mounts (lower-cased): Armada II's folder for
/// its type.
fn zfs_mount_path(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    match lower.rsplit_once('.').map(|(_, e)| e) {
        Some("odf") => format!("odf/{lower}"),
        Some("tt") => format!("techtree/{lower}"),
        _ => lower,
    }
}

/// Normalise a virtual path: forward slashes, lower case, no leading slash.
pub fn normalize(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

#[derive(Debug, Default)]
pub struct Vfs {
    layers: Vec<Layer>,
    /// Per layer: normalised virtual path -> source. Built at mount time.
    index: Vec<HashMap<String, Source>>,
}

impl Vfs {
    /// Mount a Fleet Operations / Armada II style install.
    ///
    /// `active_mod` is a folder name under `Data/Mods`; `None` mounts only `Data`.
    pub fn mount_install(install_root: &Path, active_mod: Option<&str>) -> Result<Self, VfsError> {
        let data = find_child_ci(install_root, "Data")
            .ok_or_else(|| VfsError::NoDataDir(install_root.to_path_buf()))?;

        let mut vfs = Vfs::default();
        if let Some(name) = active_mod {
            for (mod_name, dir) in mod_chain(&data, name)? {
                vfs.push_dir(mod_name, dir)?;
            }
        }
        vfs.push_dir("Data".into(), data)?;
        Ok(vfs)
    }

    /// Append a lower-priority layer: a directory plus its root `*.fpq` archives.
    pub fn push_dir(&mut self, name: String, root: PathBuf) -> Result<(), VfsError> {
        let mut map = HashMap::new();
        index_dir(&root, &root, &mut map)?;

        // STA64 archives next: loose files keep priority over them.
        let mut sta_paths: Vec<PathBuf> = fs::read_dir(&root)
            .map_err(|source| VfsError::Io { path: root.clone(), source })?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sta")))
            .collect();
        sta_paths.sort();
        let mut sta_archives = Vec::new();
        for path in sta_paths {
            let a = sta::StaArchive::open(&path).map_err(|source| VfsError::Sta { path: path.clone(), source })?;
            let archive = sta_archives.len();
            for (entry, e) in a.entries().iter().enumerate() {
                let src = match e.kind {
                    sta::Kind::File => Source::Sta { archive, entry },
                    sta::Kind::Whiteout => Source::Whiteout,
                };
                map.entry(e.key.clone()).or_insert(src);
            }
            sta_archives.push((path, a));
        }

        let mut fpqs: Vec<PathBuf> = fs::read_dir(&root)
            .map_err(|source| VfsError::Io { path: root.clone(), source })?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("fpq")))
            .collect();
        fpqs.sort();
        let mut archives = Vec::new();
        for path in fpqs {
            let fpq = sta_fpq::Fpq::open(&path).map_err(|source| VfsError::Archive {
                path: path.clone(),
                source,
            })?;
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
            let archive = archives.len();
            for entry in 0..fpq.len() {
                let rel = fpq.path(entry).map_err(|source| VfsError::Archive {
                    path: path.clone(),
                    source,
                })?;
                let key = normalize(&format!("{stem}/{rel}"));
                // Loose files were indexed first and keep priority.
                map.entry(key).or_insert(Source::Packed { archive, entry });
            }
            archives.push((path, fpq));
        }

        let mut zfs_paths: Vec<PathBuf> = fs::read_dir(&root)
            .map_err(|source| VfsError::Io { path: root.clone(), source })?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zfs")))
            .collect();
        zfs_paths.sort();
        let mut zfs_archives = Vec::new();
        for path in zfs_paths {
            let z = zfs::Zfs::open(&path).map_err(|source| VfsError::Zfs { path: path.clone(), source })?;
            let archive = zfs_archives.len();
            for entry in 0..z.len() {
                // Loose files and FPQ entries were indexed first and keep priority.
                map.entry(zfs_mount_path(&z.entry(entry).name)).or_insert(Source::Zfs { archive, entry });
            }
            zfs_archives.push((path, z));
        }

        self.layers.push(Layer { name, root, archives, zfs: zfs_archives, sta: sta_archives });
        self.index.push(map);
        Ok(())
    }

    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// The highest-priority source for `path`, with its layer index; `None`
    /// when no layer has it or a whiteout hides it.
    pub fn resolve(&self, path: &str) -> Option<(usize, &Source)> {
        let key = normalize(path);
        let (i, s) = self.index.iter().enumerate().find_map(|(i, m)| m.get(&key).map(|s| (i, s)))?;
        (*s != Source::Whiteout).then_some((i, s))
    }

    /// All layers providing `path`, highest priority first (for override
    /// diagnostics and next-version-down reads), stopping at a whiteout.
    pub fn resolve_all(&self, path: &str) -> Vec<(&str, &Source)> {
        let key = normalize(path);
        self.layers
            .iter()
            .zip(&self.index)
            .filter_map(|(l, m)| m.get(&key).map(|s| (l.name(), s)))
            .take_while(|(_, s)| **s != Source::Whiteout)
            .collect()
    }

    pub fn read(&self, path: &str) -> Option<std::io::Result<Vec<u8>>> {
        let (layer, source) = self.resolve(path)?;
        Some(self.read_from(layer, source))
    }

    /// Read one source of layer `layer` (as [`Vfs::resolve`] and
    /// [`Vfs::resolve_all`] hand them out), whichever layer wins its path.
    pub fn read_from(&self, layer: usize, source: &Source) -> std::io::Result<Vec<u8>> {
        let invalid = |e: String| std::io::Error::new(std::io::ErrorKind::InvalidData, e);
        let l = self.layers.get(layer).ok_or_else(|| invalid(format!("no layer {layer}")))?;
        match source {
            Source::Loose(p) => fs::read(p),
            Source::Packed { archive, entry } => {
                let (_, fpq) = l.archives.get(*archive).ok_or_else(|| invalid(format!("no archive {archive}")))?;
                fpq.read(*entry).map_err(|e| invalid(e.to_string()))
            }
            Source::Zfs { archive, entry } => {
                let (_, z) = l.zfs.get(*archive).ok_or_else(|| invalid(format!("no zfs {archive}")))?;
                z.read(*entry).map_err(|e| invalid(e.to_string()))
            }
            Source::Sta { archive, entry } => {
                let (_, a) = l.sta.get(*archive).ok_or_else(|| invalid(format!("no sta {archive}")))?;
                a.read(*entry).map_err(|e| invalid(e.to_string()))
            }
            Source::Whiteout => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "hidden by a whiteout")),
        }
    }

    /// Index of the layer named `name` (layer names are unique).
    pub fn layer_index(&self, name: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.name == name)
    }

    /// Every visible virtual path under `dir` (after overrides), sorted.
    pub fn list(&self, dir: &str) -> Vec<String> {
        let mut prefix = normalize(dir);
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        // The top-most layer having a path decides; a whiteout there hides it.
        let mut decided: HashSet<&str> = HashSet::new();
        let mut seen: Vec<&str> = Vec::new();
        for m in &self.index {
            for (k, s) in m {
                if k.starts_with(&prefix) && decided.insert(k) && *s != Source::Whiteout {
                    seen.push(k);
                }
            }
        }
        let mut out: Vec<String> = seen.into_iter().map(str::to_owned).collect();
        out.sort();
        out
    }
}

/// Resolve `name` and its `ParentMod` ancestors, child first.
pub fn mod_chain(data: &Path, name: &str) -> Result<Vec<(String, PathBuf)>, VfsError> {
    let mods = find_child_ci(data, "Mods").ok_or_else(|| VfsError::ModNotFound(name.into()))?;
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut current = Some(name.to_owned());
    while let Some(n) = current {
        if !seen.insert(n.to_ascii_lowercase()) {
            let names: Vec<_> = chain.iter().map(|(n, _): &(String, PathBuf)| n.as_str()).collect();
            return Err(VfsError::ParentCycle(format!("{} -> {n}", names.join(" -> "))));
        }
        let dir = find_child_ci(&mods, &n).ok_or_else(|| VfsError::ModNotFound(n.clone()))?;
        current = find_child_ci(&dir, "info.ini")
            .and_then(|p| fs::read(&p).ok())
            .and_then(|b| parent_mod(&String::from_utf8_lossy(&b)));
        chain.push((n, dir));
    }
    Ok(chain)
}

/// Extract `ParentMod` from the `[Mod]` section of an `info.ini`.
pub fn parent_mod(ini: &str) -> Option<String> {
    let mut in_mod = false;
    for line in ini.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_mod = line.eq_ignore_ascii_case("[mod]");
            continue;
        }
        if !in_mod {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case("ParentMod")
        {
            let v = v.trim().trim_matches('"').trim();
            return (!v.is_empty()).then(|| v.to_owned());
        }
    }
    None
}

fn find_child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        e.file_name()
            .to_str()
            .filter(|n| n.eq_ignore_ascii_case(name))
            .map(|_| e.path())
    })
}

fn index_dir(root: &Path, dir: &Path, out: &mut HashMap<String, Source>) -> Result<(), VfsError> {
    let rd = fs::read_dir(dir).map_err(|source| VfsError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in rd.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            // Mods are mounted as explicit layers, never as part of Data.
            if dir == root && entry.file_name().eq_ignore_ascii_case("mods") {
                continue;
            }
            index_dir(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            let key = normalize(&rel.to_string_lossy());
            // Within one layer, keep the first spelling encountered.
            out.entry(key).or_insert(Source::Loose(path));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sta_vfs_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: PathBuf, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    #[test]
    fn zfs_entries_mount_by_type_below_loose_files() {
        let root = tmp("zfs");
        let zfs = crate::zfs::tests::build(
            &[("Fed.odf", b"classLabel = \"craft\""), ("borg4.tt", b"bbase.odf 0"), ("b1ob1.txt", b"OBJ"), ("over.odf", b"zfs")],
            0x9466_279b,
            100,
        );
        fs::write(root.join("assets.zfs"), zfs).unwrap();
        write(root.join("odf/over.odf"), "loose");
        let mut vfs = Vfs::default();
        vfs.push_dir("A1".into(), root.clone()).unwrap();
        assert_eq!(vfs.read("odf/fed.odf").unwrap().unwrap(), b"classLabel = \"craft\"");
        assert_eq!(vfs.read("techtree/BORG4.tt").unwrap().unwrap(), b"bbase.odf 0");
        assert_eq!(vfs.read("b1ob1.txt").unwrap().unwrap(), b"OBJ");
        assert_eq!(vfs.read("odf/over.odf").unwrap().unwrap(), b"loose");
        assert!(matches!(vfs.resolve("odf/fed.odf"), Some((0, Source::Zfs { archive: 0, entry: 0 }))));
        let all = vfs.resolve_all("odf/over.odf");
        assert_eq!(all.len(), 1, "one source per layer and path");
        assert_eq!(vfs.list("odf"), ["odf/fed.odf", "odf/over.odf"]);
        let (_, src) = vfs.resolve("techtree/borg4.tt").unwrap();
        assert_eq!(vfs.read_from(vfs.layer_index("A1").unwrap(), src).unwrap(), b"bbase.odf 0");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sta_mounts_below_loose_and_whiteouts_hide_lower_layers() {
        use crate::sta::tests::{build, T};
        let root = tmp("sta");
        write(root.join("Data/odf/ships/fdefiant.odf"), "data defiant");
        write(root.join("Data/odf/ships/fsabre.odf"), "data sabre");
        write(root.join("Data/Mods/M/info.ini"), "[Mod]\n");
        // The mod's archive: a new ship, an override, a whiteout; a loose file beats its own copy.
        let archive = build(
            &[
                T::Zstd("odf/ships/Fakira.odf", b"mod akira"),
                T::Stored("odf/ships/fsabre.odf", b"mod sabre"),
                T::Whiteout("odf/ships/fdefiant.odf"),
                T::Stored("odf/ships/test.odf", b"packed test"),
            ],
            true,
        );
        fs::write(root.join("Data/Mods/M/mod.sta"), archive).unwrap();
        write(root.join("Data/Mods/M/odf/ships/test.odf"), "loose test");
        let vfs = Vfs::mount_install(&root, Some("M")).unwrap();
        assert_eq!(vfs.read("odf/ships/FAKIRA.odf").unwrap().unwrap(), b"mod akira");
        assert_eq!(vfs.read("odf/ships/fsabre.odf").unwrap().unwrap(), b"mod sabre");
        assert_eq!(vfs.read("odf/ships/test.odf").unwrap().unwrap(), b"loose test");
        assert!(vfs.read("odf/ships/fdefiant.odf").is_none(), "whiteout hides Data's file");
        assert!(vfs.resolve_all("odf/ships/fdefiant.odf").is_empty());
        assert_eq!(vfs.resolve_all("odf/ships/fsabre.odf").len(), 2);
        let ships = vfs.list("odf/ships");
        assert_eq!(ships, ["odf/ships/fakira.odf", "odf/ships/fsabre.odf", "odf/ships/test.odf"]);
        assert!(matches!(vfs.resolve("odf/ships/fakira.odf"), Some((0, Source::Sta { archive: 0, .. }))));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn broken_sta_fails_the_mount() {
        let root = tmp("sta_bad");
        fs::write(root.join("bad.sta"), b"STA64ARC garbage").unwrap();
        let mut vfs = Vfs::default();
        assert!(matches!(vfs.push_dir("x".into(), root.clone()), Err(VfsError::Sta { .. })));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parent_mod_parsing() {
        let ini = "[Other]\nParentMod=x\n[Mod]\nName=Child\nParentMod = \"Fleet Ops 4.0\"\n";
        assert_eq!(parent_mod(ini).as_deref(), Some("Fleet Ops 4.0"));
        assert_eq!(parent_mod("[Mod]\nParentMod=\"\"\n"), None);
    }

    #[test]
    fn layered_override_case_insensitive() {
        let root = tmp("layers");
        write(root.join("Data/odf/Ships/fdefiant.odf"), "base");
        write(root.join("Data/odf/ships/fsabre.odf"), "base");
        write(root.join("Data/Mods/Base/info.ini"), "[Mod]\n");
        write(root.join("Data/Mods/Base/odf/ships/FSABRE.ODF"), "parent");
        write(root.join("Data/Mods/Child/info.ini"), "[Mod]\nParentMod=\"base\"\n");
        write(root.join("Data/Mods/Child/odf/ships/fdefiant.odf"), "child");

        let vfs = Vfs::mount_install(&root, Some("Child")).unwrap();
        let names: Vec<_> = vfs.layers().iter().map(Layer::name).collect();
        assert_eq!(names, ["Child", "base", "Data"]);
        let read = |p| String::from_utf8(vfs.read(p).unwrap().unwrap()).unwrap();
        assert_eq!(read("ODF\\SHIPS\\FDefiant.odf"), "child");
        assert_eq!(read("odf/ships/fsabre.odf"), "parent");
        assert_eq!(vfs.resolve_all("odf/ships/fdefiant.odf").len(), 2);
        assert!(vfs.list("odf/ships").iter().all(|p| !p.contains("mods/")));
    }

    #[test]
    fn detects_parent_cycle() {
        let root = tmp("cycle");
        write(root.join("Data/Mods/A/info.ini"), "[Mod]\nParentMod=B\n");
        write(root.join("Data/Mods/B/info.ini"), "[Mod]\nParentMod=A\n");
        assert!(matches!(
            Vfs::mount_install(&root, Some("A")),
            Err(VfsError::ParentCycle(_))
        ));
    }

    /// Mounts a real install when STA64_TEST_INSTALL / STA64_TEST_MOD are set.
    #[test]
    fn real_install_if_available() {
        let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else {
            return;
        };
        let vfs = Vfs::mount_install(Path::new(&root), Some(&m)).unwrap();
        let packed: usize = vfs.layers().iter().flat_map(|l| &l.archives).map(|(_, f)| f.len()).sum();
        let odfs: Vec<_> = vfs.list("odf").into_iter().filter(|p| p.ends_with(".odf")).collect();
        let mut from_archive = 0;
        for p in &odfs {
            if matches!(vfs.resolve(p), Some((_, Source::Packed { .. }))) {
                from_archive += 1;
                assert!(!vfs.read(p).unwrap().unwrap().is_empty(), "{p}");
            }
        }
        eprintln!("{} layers, {packed} packed entries, {} odfs, {from_archive} served from FPQ", vfs.layers().len(), odfs.len());
        assert!(packed == 0 || from_archive > 0);
    }
}
