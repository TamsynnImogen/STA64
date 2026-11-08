//! Where every ODF lives (S5). Seam type; CON-02 writes the index building
//! (all ODF folders, recursive, A1 `addon/`, duplicate warnings).

use std::collections::BTreeMap;

use crate::Warning;

/// One ODF file found in the mounted install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OdfEntry {
    /// VFS path of the file.
    pub path: String,
    /// Index into `Vfs::layers()`; higher wins.
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

impl OdfIndex {
    /// Index every ODF folder of `vfs` (CON-02; not yet written: returns an
    /// empty index and an unsupported warning).
    pub fn build(vfs: &sta_vfs::Vfs) -> (OdfIndex, Vec<Warning>) {
        let _ = vfs;
        (OdfIndex::default(), vec![Warning::unsupported("OdfIndex::build (CON-02)")])
    }

    pub fn get(&self, stem: &str) -> Option<&OdfEntry> {
        self.entries.get(&stem.to_ascii_lowercase())
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
