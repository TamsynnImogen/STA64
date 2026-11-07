//! Layered virtual filesystem.
//!
//! An install is mounted as an ordered stack of layers, highest priority first:
//! `active mod -> its ParentMod chain -> Data`. Lookups are case-insensitive on
//! every platform, because Armada-era content mixes case freely and was only
//! ever tested on Windows.
//!
//! FPQ archive layers are planned; the `Layer` enum is where they will plug in.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum VfsError {
    #[error("no Data directory under {0}")]
    NoDataDir(PathBuf),
    #[error("mod {0:?} not found under Data/Mods")]
    ModNotFound(String),
    #[error("ParentMod cycle: {0}")]
    ParentCycle(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// One mounted source of files.
#[derive(Debug, Clone)]
pub enum Layer {
    /// A loose directory on disk.
    Dir { name: String, root: PathBuf },
}

impl Layer {
    pub fn name(&self) -> &str {
        match self {
            Layer::Dir { name, .. } => name,
        }
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
    /// Per layer: normalised virtual path -> real path. Built at mount time.
    index: Vec<HashMap<String, PathBuf>>,
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

    /// Append a lower-priority directory layer.
    pub fn push_dir(&mut self, name: String, root: PathBuf) -> Result<(), VfsError> {
        let mut map = HashMap::new();
        index_dir(&root, &root, &mut map)?;
        self.layers.push(Layer::Dir { name, root });
        self.index.push(map);
        Ok(())
    }

    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// Resolve a virtual path to the highest-priority real file.
    pub fn resolve(&self, path: &str) -> Option<&Path> {
        let key = normalize(path);
        self.index.iter().find_map(|m| m.get(&key).map(PathBuf::as_path))
    }

    /// All layers providing `path`, highest priority first (for override diagnostics).
    pub fn resolve_all(&self, path: &str) -> Vec<(&str, &Path)> {
        let key = normalize(path);
        self.layers
            .iter()
            .zip(&self.index)
            .filter_map(|(l, m)| m.get(&key).map(|p| (l.name(), p.as_path())))
            .collect()
    }

    pub fn read(&self, path: &str) -> Option<std::io::Result<Vec<u8>>> {
        self.resolve(path).map(fs::read)
    }

    /// Every visible virtual path under `dir` (after overrides), sorted.
    pub fn list(&self, dir: &str) -> Vec<String> {
        let mut prefix = normalize(dir);
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for m in &self.index {
            for k in m.keys() {
                if k.starts_with(&prefix) {
                    seen.insert(k);
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
        if let Some((k, v)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case("ParentMod") {
                let v = v.trim().trim_matches('"').trim();
                return (!v.is_empty()).then(|| v.to_owned());
            }
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

fn index_dir(root: &Path, dir: &Path, out: &mut HashMap<String, PathBuf>) -> Result<(), VfsError> {
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
            out.entry(key).or_insert(path);
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
}
