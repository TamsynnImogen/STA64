//! The modder tool shell (EDT-01), behind the `shell` feature: the parts the
//! tools share, without a window. [`Workspace`] opens an install and an
//! optional mod over the VFS, lists the ODFs by root, reads and saves them
//! (loose files only, with a backup), and turns a unit's model into preview
//! triangles. The window (`bin/tool_shell.rs`) is a thin egui front end on it.
//!
//! Nothing here writes into the install except an explicit [`Workspace::save`]
//! of a loose ODF, which keeps the previous file as `<name>.bak`.

use std::path::{Path, PathBuf};

use sta_assets::model::{Model, NodeKind};
use sta_content::{OdfEntry, OdfIndex};
use sta_vfs::{Source, Vfs};

/// The mods of an install's `Data` folder, by folder name (empty without one).
pub fn mods(install: &Path) -> Vec<String> {
    let data = ["Data", "data", "DATA"].iter().map(|d| install.join(d)).find(|p| p.is_dir());
    data.and_then(|d| sta_vfs::mods::list_mods(&d).ok()).map(|(m, _)| m.into_iter().map(|m| m.folder).collect()).unwrap_or_default()
}

/// One mounted root (a layer of the VFS) and the ODFs whose winning file sits in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub name: String,
    /// `(stem, VFS path)`, by stem.
    pub odfs: Vec<(String, String)>,
}

/// An open install (and mod).
pub struct Workspace {
    pub install: PathBuf,
    pub active_mod: Option<String>,
    vfs: Vfs,
    index: OdfIndex,
}

/// What went wrong, for the status line.
#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("could not mount: {0}")]
    Mount(#[from] sta_vfs::VfsError),
    #[error("no ODF named {0:?}")]
    NoOdf(String),
    #[error("{0} is inside an archive; only loose files are saved")]
    Packed(String),
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
}

impl Workspace {
    pub fn open(install: &Path, active_mod: Option<&str>) -> Result<Workspace, ShellError> {
        let vfs = Vfs::mount_install(install, active_mod)?;
        let (index, _) = OdfIndex::build(&vfs);
        Ok(Workspace { install: install.to_path_buf(), active_mod: active_mod.map(str::to_owned), vfs, index })
    }

    /// Every mounted root, winning layer first, with its ODFs.
    pub fn roots(&self) -> Vec<Root> {
        let mut roots: Vec<Root> = self.vfs.layers().iter().map(|l| Root { name: l.name().to_owned(), odfs: Vec::new() }).collect();
        for (stem, entry) in self.index.iter() {
            if let Some(r) = roots.get_mut(entry.layer) {
                r.odfs.push((stem.to_owned(), entry.path.clone()));
            }
        }
        roots
    }

    pub fn odf_count(&self) -> usize {
        self.index.len()
    }

    fn entry(&self, stem: &str) -> Result<&OdfEntry, ShellError> {
        self.index.get(stem).ok_or_else(|| ShellError::NoOdf(stem.to_owned()))
    }

    /// The winning file's text (lossy UTF-8; ODFs are ASCII).
    pub fn read_odf(&self, stem: &str) -> Result<String, ShellError> {
        let e = self.entry(stem)?;
        let bytes = OdfIndex::read(&self.vfs, e).ok_or_else(|| ShellError::NoOdf(stem.to_owned()))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The file on disk behind a loose ODF.
    pub fn loose_path(&self, stem: &str) -> Result<PathBuf, ShellError> {
        let e = self.entry(stem)?;
        match self.vfs.resolve(&e.path) {
            Some((_, Source::Loose(p))) => Ok(p.clone()),
            _ => Err(ShellError::Packed(e.path.clone())),
        }
    }

    /// Save `text` over a loose ODF, keeping the previous file as `.bak`.
    /// Rebuilds the index so the new text is what the next read sees.
    pub fn save(&mut self, stem: &str, text: &str) -> Result<PathBuf, ShellError> {
        let path = self.loose_path(stem)?;
        let backup = crate::fsutil::save_with_backup(&path, text.as_bytes()).map_err(|e| ShellError::Io(path.display().to_string(), e))?;
        *self = Workspace::open(&self.install.clone(), self.active_mod.clone().as_deref())?;
        Ok(backup)
    }

    /// The model an ODF names (`basename`, else its own stem), lower case, no extension.
    pub fn model_name(&self, stem: &str) -> Option<String> {
        let text = self.read_odf(stem).ok()?;
        // The file's own `basename` (the last one wins); an include's is not followed here.
        let raw = sta_odf::parse(&text);
        let name = raw.entries.iter().rev().find(|e| e.key.eq_ignore_ascii_case("basename")).map(|e| e.value.trim().trim_matches('"').to_owned()).filter(|v| !v.is_empty()).unwrap_or_else(|| stem.to_owned());
        Some(name.to_ascii_lowercase().trim_end_matches(".sod").to_owned())
    }

    /// The model `sod/<name>.sod`, if the install has it.
    pub fn load_sod(&self, name: &str) -> Option<Model> {
        let bytes = self.vfs.read(&format!("sod/{name}.sod"))?.ok()?;
        let sod = sta_assets::a2fmt::sod::Sod::parse(&bytes).ok()?;
        Some(Model::from_sod(&sod))
    }
}

/// Preview triangles of a model: every mesh node in model space, as flat
/// positions and indices (SOD winding). LOD branches are all drawn; this is a
/// preview, not the renderer.
pub fn preview_triangles(model: &Model) -> (Vec<[f32; 3]>, Vec<u32>) {
    let (mut positions, mut indices) = (Vec::new(), Vec::new());
    for (i, node) in model.nodes.iter().enumerate() {
        let NodeKind::Mesh(mesh) = &node.kind else { continue };
        let to_model = model.model_transform(i);
        let base = positions.len() as u32;
        positions.extend(mesh.positions.iter().map(|p| to_model.transform_point(*p)));
        for g in &mesh.groups {
            for f in &g.faces {
                indices.extend(f.iter().map(|(p, _)| base + p));
            }
        }
    }
    (positions, indices)
}
