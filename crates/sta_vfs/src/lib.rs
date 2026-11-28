//! Layered virtual filesystem.
//!
//! An install is mounted as an ordered stack of layers, highest priority first:
//! `active mod -> its ParentMod chain -> Data`. Lookups are case-insensitive on
//! every platform, because Armada-era content mixes case freely and was only
//! ever tested on Windows.
//! A mod's own `info.ini` rules shape the stack (D6): a `standalone` mod ends
//! the chain and base `Data` is not mounted under it; `ignoreParentMaps`
//! hides the `bzn/` maps of every layer below the mod. A manifest's `engine`
//! requirement must be met by this STA64 version (D4).
//! An original flat A1 install with root ZFS files mounts as one `root` layer
//! when no Data child or selected mod is present.
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

pub mod convert;
pub mod manifest;
pub mod mods;
pub mod order;
pub mod sta;
pub mod sta_sign;
pub mod sta_write;
pub mod zfs;

#[derive(Debug, thiserror::Error)]
pub enum VfsError {
    #[error("no Data directory under {0}")]
    NoDataDir(PathBuf),
    #[error("mod {0:?} not found under Data/Mods")]
    ModNotFound(String),
    /// A mod name that is not one folder name under `Data/Mods` (see [`single_folder_name`]).
    #[error("mod name {name:?} {asked_by} is not a single folder name under Data/Mods (no path parts, `.`, `..`, separators or drive)")]
    BadModName { name: String, asked_by: String },
    /// A mod folder that resolves to somewhere other than directly under `Data/Mods`.
    #[error("mod {name:?} {asked_by} is a link to {target}, which is not a folder directly under Data/Mods; move the mod into Data/Mods")]
    ModOutsideMods { name: String, asked_by: String, target: PathBuf },
    #[error("ParentMod cycle: {0}")]
    ParentCycle(String),
    #[error("mod {name:?} needs STA64 {requirement}, this is {have}")]
    Engine { name: String, requirement: String, have: String },
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
    flat_install: bool,
}

impl Vfs {
    /// Mount a Data-based install or a recognized original flat Armada I/II root.
    ///
    /// `active_mod` is a folder name under `Data/Mods`; `None` mounts only `Data`.
    /// Without a Data child, `None` also permits a flat root containing ZFS files.
    /// A loose-file A2 root requires odf/bzn directories and either Armada2.exe
    /// or the original game's game_cfg.h and misc/gui_glob16x12.cfg files.
    /// A requested mod is never silently ignored by falling back to a flat root.
    pub fn mount_install(install_root: &Path, active_mod: Option<&str>) -> Result<Self, VfsError> {
        let Some(data) = find_child_ci(install_root, "Data") else {
            let flat_a1 = active_mod.is_none()
                && fs::read_dir(install_root).is_ok_and(|entries| {
                    entries.flatten().any(|entry| {
                        let path = entry.path();
                        path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("zfs")) && path.is_file()
                    })
                });
            let flat_a2 = active_mod.is_none()
                && find_child_ci(install_root, "odf").is_some_and(|p| p.is_dir())
                && find_child_ci(install_root, "bzn").is_some_and(|p| p.is_dir())
                && (find_child_ci(install_root, "Armada2.exe").is_some_and(|p| p.is_file())
                    || (find_child_ci(install_root, "game_cfg.h").is_some_and(|p| p.is_file())
                        && find_child_ci(install_root, "misc")
                            .filter(|p| p.is_dir())
                            .and_then(|p| find_child_ci(&p, "gui_glob16x12.cfg"))
                            .is_some_and(|p| p.is_file())));
            if !flat_a1 && !flat_a2 {
                return Err(VfsError::NoDataDir(install_root.to_path_buf()));
            }
            let mut vfs = Vfs { flat_install: true, ..Vfs::default() };
            vfs.push_dir("root".into(), install_root.to_path_buf())?;
            return Ok(vfs);
        };

        let chain = match active_mod {
            Some(name) => mod_chain_info(&data, name)?,
            None => Vec::new(),
        };
        Vfs::mount_chain(&data, &chain)
    }

    /// Mount `chain` (child first) over `data`, by the mods' own rules (D6):
    /// base `Data` is not mounted at all when any mod of the chain is
    /// `standalone` ([`mod_chain_info`] ends a chain at the first one); a mod with `ignoreParentMaps` hides the maps
    /// (`bzn/`) of every layer below it. Each mod's manifest `engine`
    /// requirement must be met by this STA64 version (D4).
    pub(crate) fn mount_chain(data: &Path, chain: &[ChainMod]) -> Result<Self, VfsError> {
        let mut vfs = Vfs::default();
        let mut standalone = false;
        for m in chain {
            if let Some(Ok(manifest)) = manifest::manifest_text(&m.dir).map(|t| manifest::Manifest::parse(&t))
                && !manifest.engine_ok()
            {
                return Err(VfsError::Engine {
                    name: m.name.clone(),
                    requirement: manifest.engine.map(|r| r.to_string()).unwrap_or_default(),
                    have: manifest::engine_version().to_string(),
                });
            }
            vfs.push_dir(m.name.clone(), m.dir.clone())?;
            standalone |= m.standalone;
        }
        if !standalone {
            vfs.push_dir("Data".into(), data.to_path_buf())?;
        }
        for (i, m) in chain.iter().enumerate().take(vfs.layers.len()) {
            if m.ignore_parent_maps {
                vfs.hide_below(i, MAP_DIR);
            }
        }
        Ok(vfs)
    }

    /// Drop every path under `prefix` from the layers below layer `above`.
    fn hide_below(&mut self, above: usize, prefix: &str) {
        for m in self.index.iter_mut().skip(above + 1) {
            m.retain(|k, _| !k.starts_with(prefix));
        }
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

    /// An original game's data folder was mounted directly, independently of
    /// any host mod chain its directory happens to be stored inside.
    pub fn is_flat_install(&self) -> bool {
        self.flat_install
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
    Ok(walk_chain(data, name, false)?.into_iter().map(|m| (m.name, m.dir)).collect())
}

/// Folder of the maps a mod's `ignoreParentMaps` hides in the layers below it.
const MAP_DIR: &str = "bzn/";

/// One mod of a mount chain with the `info.ini` flags that shape the mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainMod {
    pub name: String,
    pub dir: PathBuf,
    /// `standalone=1`: runs over nothing; its `ParentMod` and base `Data` are not mounted.
    pub standalone: bool,
    /// `ignoreParentMaps=1`: the maps of every layer below are hidden.
    pub ignore_parent_maps: bool,
}

/// As [`mod_chain`], but the chain ends at the first `standalone` mod (its
/// `ParentMod`, if any, is not followed) and each entry carries its flags.
pub fn mod_chain_info(data: &Path, name: &str) -> Result<Vec<ChainMod>, VfsError> {
    walk_chain(data, name, true)
}

fn walk_chain(data: &Path, name: &str, stop_at_standalone: bool) -> Result<Vec<ChainMod>, VfsError> {
    let mods = find_child_ci(data, "Mods").ok_or_else(|| VfsError::ModNotFound(name.into()))?;
    let mut chain: Vec<ChainMod> = Vec::new();
    let mut seen = HashSet::new();
    let mut current = Some(name.to_owned());
    // Who asked for the name being looked up: the player's own choice for the first, the child mod's `info.ini`
    // for a parent entry.
    let mut asked_by = "(given as the mod to run, by the command line or a setting)".to_owned();
    while let Some(n) = current {
        // A name is one folder name, checked before anything is joined onto the mods folder.
        if !single_folder_name(&n) {
            return Err(VfsError::BadModName { name: n, asked_by });
        }
        if !seen.insert(n.to_ascii_lowercase()) {
            let names: Vec<_> = chain.iter().map(|m| m.name.as_str()).collect();
            return Err(VfsError::ParentCycle(format!("{} -> {n}", names.join(" -> "))));
        }
        let dir = find_child_ci(&mods, &n).ok_or_else(|| VfsError::ModNotFound(n.clone()))?;
        directly_under(&mods, &dir).map_err(|target| VfsError::ModOutsideMods { name: n.clone(), asked_by: asked_by.clone(), target })?;
        asked_by = format!("(the ParentMod of mod {n:?})");
        let ini = mod_info_bytes(&dir)?.map(|b| mods::Ini::parse(&String::from_utf8_lossy(&b))).unwrap_or_default();
        let parent = ini.get("mod", "ParentMod").map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned);
        // `standalone` in info.ini, or a manifest that says `kind = "standalone"` (D4).
        let manifest_standalone = manifest::manifest_text(&dir)
            .and_then(|t| manifest::Manifest::parse(&t).ok())
            .is_some_and(|m| m.kind_explicit && m.kind == manifest::ModKind::Standalone);
        let (standalone, ignore_parent_maps) = (ini.flag("standalone") || manifest_standalone, ini.flag("ignoreParentMaps"));
        chain.push(ChainMod { name: n, dir, standalone, ignore_parent_maps });
        current = if stop_at_standalone && standalone { None } else { parent };
    }
    Ok(chain)
}

/// The `info.ini` bytes of the mod folder `dir`: its loose `info.ini` (any
/// case), else the one inside its first root `.sta` (path order, as
/// [`Vfs::push_dir`] layers them) that carries one, so a mod that is only a
/// package is a complete mod (CON-16, CON-18; loose files beat the archive,
/// `docs/STA_ARCHIVE.md` "Mounting and load order" 3).
///
/// With several root `.sta` files, the first one in byte order of its path
/// (the order the mount layers them in) that has an `info.ini` entry
/// supplies it. A package that does not open is skipped here (the mount
/// reports it); a package that HAS an `info.ini` entry which fails to read
/// (for instance its hash) is an error naming the archive, because skipping
/// it would silently drop the mod's `ParentMod`, `standalone` and
/// `ignoreParentMaps`. (The mod listing only notes such a failure, as it must
/// still list the other mods; a chain that is about to be mounted must not.)
fn mod_info_bytes(dir: &Path) -> Result<Option<Vec<u8>>, VfsError> {
    if let Some(bytes) = find_child_ci(dir, "info.ini").and_then(|p| fs::read(p).ok()) {
        return Ok(Some(bytes));
    }
    let Ok(rd) = fs::read_dir(dir) else { return Ok(None) };
    let mut stas: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sta")))
        .collect();
    stas.sort();
    for p in &stas {
        let Ok(archive) = sta::StaArchive::open(p) else { continue };
        if let Some(i) = archive.find("info.ini") {
            return archive.read(i).map(Some).map_err(|source| VfsError::Sta { path: p.clone(), source });
        }
    }
    Ok(None)
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

/// Whether `name` is exactly one folder name, the same on every platform: not empty, not `.` or `..`, no `/`
/// or `\`, no NUL, and no drive form (`C:` or `C:x`). A name that is more than that would let a join onto the
/// mods folder reach another folder (`..`, `a/b`, an absolute path, a UNC path, a drive-relative path).
/// STA64's own safety rule (ADR 0004 D4: a mod's data chain comes from its kind and `ParentMod`, which name
/// folders in `Data/Mods`; D6: the mount follows the mod's own rules, and these are not a rule a mod can ask
/// for). A well-formed mod folder name, spaces and letter case included, passes unchanged (D1).
pub fn single_folder_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return false;
    }
    let mut chars = name.chars();
    let drive = matches!((chars.next(), chars.next()), (Some(c), Some(':')) if c.is_ascii_alphabetic());
    !drive
}

/// `Ok` when `dir` (found under `mods`) resolves to a folder directly under the resolved `mods`; else the
/// resolved path it leads to. A mod folder that is a link is followed only when it leads to a folder that is
/// itself directly under the mods folder (a link to a sibling mod); a link that leaves the mods folder is
/// refused, whoever placed it, because nothing here can tell a link the player made from one a package
/// unpacked: STA64's own rule. A path that cannot be resolved is refused too.
fn directly_under(mods: &Path, dir: &Path) -> Result<(), PathBuf> {
    let (Ok(mods), Ok(real)) = (fs::canonicalize(mods), fs::canonicalize(dir)) else { return Err(dir.to_path_buf()) };
    if real.parent() == Some(mods.as_path()) { Ok(()) } else { Err(real) }
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
mod flat_install_tests;

#[cfg(test)]
mod flat_a2_tests;

#[cfg(test)]
mod mount_tests;

#[cfg(test)]
mod mod_name_tests;

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

    /// Mounts STA64_TEST_INSTALL, with an optional nonempty STA64_TEST_MOD.
    #[test]
    fn real_install_if_available() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
            return;
        };
        let m = std::env::var("STA64_TEST_MOD").ok();
        let vfs = Vfs::mount_install(Path::new(&root), m.as_deref().filter(|m| !m.is_empty())).unwrap();
        let packed: usize = vfs.layers().iter().flat_map(|l| &l.archives).map(|(_, f)| f.len()).sum();
        let odfs: Vec<_> = vfs.list("odf").into_iter().filter(|p| p.ends_with(".odf")).collect();
        let mut from_archive = 0;
        let mut from_zfs = 0;
        let mut empty = 0;
        for p in &odfs {
            if matches!(vfs.resolve(p), Some((_, Source::Packed { .. }))) {
                from_archive += 1;
            }
            if matches!(vfs.resolve(p), Some((_, Source::Zfs { .. }))) {
                from_zfs += 1;
            }
            // An empty ODF is legal (e.g. FO4's basic_rotate_combat.odf).
            // Require readable presence, not nonempty authored contents.
            let bytes = vfs.read(p).unwrap_or_else(|| panic!("missing listed ODF {p}")).unwrap();
            empty += usize::from(bytes.is_empty());
        }
        eprintln!("{} layers, {packed} FPQ entries, {} odfs, {from_archive} served from FPQ, {from_zfs} from ZFS, {empty} empty", vfs.layers().len(), odfs.len());
        assert!(!odfs.is_empty(), "install contains no ODFs");
        assert!(packed == 0 || from_archive > 0);
    }
}
