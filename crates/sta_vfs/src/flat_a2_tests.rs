use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{Source, Vfs, VfsError};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sta_vfs_flat_a2_{}_{}_{}",
            std::process::id(),
            stamp,
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn mkdir(&self, relative: &str) {
        fs::create_dir_all(self.0.join(relative)).unwrap();
    }

    fn markers(&self) {
        self.write("ArMaDa2.ExE", b"inert test marker");
        self.mkdir("OdF");
        self.mkdir("BzN");
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_no_data(root: &TempRoot, active_mod: Option<&str>) {
    assert!(matches!(
        Vfs::mount_install(root.path(), active_mod),
        Err(VfsError::NoDataDir(path)) if path == root.path()
    ));
}

fn read(vfs: &Vfs, path: &str) -> Vec<u8> {
    vfs.read(path).unwrap_or_else(|| panic!("missing {path}")).unwrap()
}

#[test]
fn complete_mixed_case_flat_a2_markers_mount_root_and_index_loose_content() {
    let root = TempRoot::new();
    root.markers();
    root.write("OdF/FeD_ScOuT.ODF", b"[classLabel]\nclassLabel = craft\n");
    root.write("BzN/NeStEd/MaP.BzN", b"synthetic map bytes");

    let vfs = Vfs::mount_install(root.path(), None).unwrap();
    assert_eq!(vfs.layers().len(), 1);
    assert_eq!(vfs.layers()[0].name(), "root");
    assert_eq!(
        fs::canonicalize(&vfs.layers()[0].root).unwrap(),
        fs::canonicalize(root.path()).unwrap()
    );
    assert_eq!(read(&vfs, "odf/fed_scout.odf"), b"[classLabel]\nclassLabel = craft\n");
    assert_eq!(read(&vfs, "BZN/NESTED/map.bzn"), b"synthetic map bytes");
    assert!(matches!(vfs.resolve("odf/fed_scout.odf"), Some((0, Source::Loose(_)))));
    assert_eq!(vfs.list("bzn"), ["bzn/nested/map.bzn"]);
}

#[test]
fn incomplete_and_wrong_type_marker_sets_do_not_recognize_arbitrary_roots() {
    // Every proper subset of the three required markers, including ordinary,
    // executable-only and directory-only roots, must remain unrecognized.
    for mask in 0..7_u8 {
        let root = TempRoot::new();
        if mask & 1 != 0 {
            root.write("Armada2.exe", b"inert test marker");
        }
        if mask & 2 != 0 {
            root.mkdir("odf");
        }
        if mask & 4 != 0 {
            root.mkdir("bzn");
        }
        assert_no_data(&root, None);
    }

    let wrong_exe = TempRoot::new();
    wrong_exe.mkdir("Armada2.exe");
    wrong_exe.mkdir("odf");
    wrong_exe.mkdir("bzn");
    assert_no_data(&wrong_exe, None);

    let wrong_odf = TempRoot::new();
    wrong_odf.write("Armada2.exe", b"inert test marker");
    wrong_odf.write("odf", b"not a directory");
    wrong_odf.mkdir("bzn");
    assert_no_data(&wrong_odf, None);

    let wrong_bzn = TempRoot::new();
    wrong_bzn.write("Armada2.exe", b"inert test marker");
    wrong_bzn.mkdir("odf");
    wrong_bzn.write("bzn", b"not a directory");
    assert_no_data(&wrong_bzn, None);
}

#[test]
fn explicit_mod_requests_cannot_select_a_flat_a2_root() {
    let root = TempRoot::new();
    root.markers();
    for name in ["ExampleMod", ""] {
        assert_no_data(&root, Some(name));
    }
}

#[test]
fn data_child_keeps_priority_and_invalid_data_does_not_fall_back() {
    let root = TempRoot::new();
    root.markers();
    root.write("OdF/root_only.odf", b"root-only");
    root.write("OdF/shared.odf", b"root");
    root.write("BzN/root_only.bzn", b"root-only");
    root.write("dAtA/odf/shared.odf", b"Data");
    root.write("dAtA/bzn/data_only.bzn", b"Data map");

    let vfs = Vfs::mount_install(root.path(), None).unwrap();
    assert_eq!(vfs.layers().len(), 1);
    assert_eq!(vfs.layers()[0].name(), "Data");
    // The filesystem may preserve a different spelling for this same child.
    assert_eq!(
        fs::canonicalize(&vfs.layers()[0].root).unwrap(),
        fs::canonicalize(root.path().join("dAtA")).unwrap()
    );
    assert_eq!(read(&vfs, "odf/shared.odf"), b"Data");
    assert_eq!(read(&vfs, "bzn/data_only.bzn"), b"Data map");
    assert!(vfs.read("odf/root_only.odf").is_none());
    assert!(vfs.read("bzn/root_only.bzn").is_none());

    let invalid = TempRoot::new();
    invalid.markers();
    invalid.write("Data", b"regular file, not an install directory");
    assert!(matches!(
        Vfs::mount_install(invalid.path(), None),
        Err(VfsError::Io { .. })
    ));
}
