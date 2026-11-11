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
            "sta_vfs_flat_{}_{}_{}",
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

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn zfs(&self, rel: &str, files: &[(&str, &[u8])]) {
        self.write(rel, &crate::zfs::tests::build(files, 0x9466_279b, 16));
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read(vfs: &Vfs, path: &str) -> Vec<u8> {
    vfs.read(path).unwrap_or_else(|| panic!("missing {path}")).unwrap()
}

#[test]
fn flat_mixed_case_zfs_mounts_by_type_below_loose_files() {
    let root = TempRoot::new();
    root.zfs(
        "AsSeTs.ZfS",
        &[
            ("Fed.ODF", b"archived fed"),
            ("Other.odf", b"archived other"),
            ("Tech.TT", b"archived tech"),
            ("root.TXT", b"archived root"),
        ],
    );
    root.write("OdF/FED.odf", b"loose fed");
    root.write("Intro.BZN", b"loose map");

    let vfs = Vfs::mount_install(root.path(), None).unwrap();
    assert_eq!(vfs.layers().len(), 1);
    assert_eq!(vfs.layers()[0].name(), "root");
    assert_eq!(vfs.layers()[0].root, root.path());
    assert_eq!(read(&vfs, "ODF/fed.ODF"), b"loose fed");
    assert_eq!(read(&vfs, "odf/OTHER.odf"), b"archived other");
    assert_eq!(read(&vfs, "TEChTREE/TECH.tt"), b"archived tech");
    assert_eq!(read(&vfs, "ROOT.txt"), b"archived root");
    assert_eq!(read(&vfs, "intro.bzn"), b"loose map");
    assert!(matches!(vfs.resolve("odf/fed.odf"), Some((0, Source::Loose(_)))));
    assert!(matches!(vfs.resolve("odf/other.odf"), Some((0, Source::Zfs { .. }))));
    assert_eq!(vfs.list("odf"), ["odf/fed.odf", "odf/other.odf"]);
}

#[test]
fn data_child_wins_over_valid_flat_root_archive_and_keeps_mod_chain() {
    let root = TempRoot::new();
    root.zfs("root.ZFS", &[("Ship.odf", b"flat ship"), ("Only.odf", b"flat only")]);
    root.write("dAtA/odf/ship.odf", b"data ship");
    root.write("dAtA/mOdS/Parent/info.ini", b"[Mod]\n");
    root.write("dAtA/mOdS/Parent/odf/ship.odf", b"parent ship");
    root.write("dAtA/mOdS/Child/info.ini", b"[Mod]\nParentMod=parent\n");
    root.write("dAtA/mOdS/Child/odf/ship.odf", b"child ship");

    let base = Vfs::mount_install(root.path(), None).unwrap();
    assert_eq!(base.layers().len(), 1);
    // Case-insensitive filesystems can retain "Data" for the same physical directory.
    assert_eq!(
        fs::canonicalize(&base.layers()[0].root).unwrap(),
        fs::canonicalize(root.path().join("dAtA")).unwrap()
    );
    assert_eq!(read(&base, "odf/ship.odf"), b"data ship");
    assert!(
        base.read("odf/only.odf").is_none(),
        "root archive must not leak into Data"
    );

    let modded = Vfs::mount_install(root.path(), Some("child")).unwrap();
    let names: Vec<_> = modded.layers().iter().map(|layer| layer.name()).collect();
    assert_eq!(names, ["child", "parent", "Data"]);
    assert_eq!(read(&modded, "odf/ship.odf"), b"child ship");
    assert_eq!(modded.resolve_all("odf/ship.odf").len(), 3);
    assert!(modded.read("odf/only.odf").is_none());
}

#[test]
fn normal_missing_nested_and_directory_named_zfs_roots_are_not_installs() {
    let root = TempRoot::new();
    let missing = root.path().join("not-here");
    assert!(matches!(Vfs::mount_install(&missing, None), Err(VfsError::NoDataDir(path)) if path == missing));
    assert!(matches!(Vfs::mount_install(root.path(), None), Err(VfsError::NoDataDir(path)) if path == root.path()));
    root.zfs("nested/archive.zfs", &[("ship.odf", b"nested")]);
    assert!(matches!(Vfs::mount_install(root.path(), None), Err(VfsError::NoDataDir(path)) if path == root.path()));
    fs::create_dir(root.path().join("folder.zfs")).unwrap();
    assert!(matches!(Vfs::mount_install(root.path(), None), Err(VfsError::NoDataDir(path)) if path == root.path()));
}

#[test]
fn flat_root_rejects_every_explicit_mod_request_and_corrupt_archive() {
    let root = TempRoot::new();
    root.zfs("assets.zfs", &[("ship.odf", b"archived")]);
    for requested in ["SomeMod", ""] {
        assert!(
            matches!(Vfs::mount_install(root.path(), Some(requested)), Err(VfsError::NoDataDir(path)) if path == root.path())
        );
    }
    let corrupt = TempRoot::new();
    corrupt.write("broken.ZfS", b"not a ZFS archive");
    assert!(matches!(
        Vfs::mount_install(corrupt.path(), None),
        Err(VfsError::Zfs { .. })
    ));
}

#[test]
fn invalid_data_child_does_not_fall_back_and_existing_mod_errors_remain() {
    let root = TempRoot::new();
    root.zfs("assets.zfs", &[("ship.odf", b"flat ship")]);
    root.write("Data", b"a regular file");
    assert!(matches!(
        Vfs::mount_install(root.path(), None),
        Err(VfsError::Io { .. })
    ));
    assert!(matches!(
        Vfs::mount_install(root.path(), Some("M")),
        Err(VfsError::ModNotFound(_))
    ));

    let install = TempRoot::new();
    install.zfs("assets.zfs", &[("ship.odf", b"flat ship")]);
    install.write("Data/Mods/A/info.ini", b"[Mod]\nParentMod=B\n");
    install.write("Data/Mods/B/info.ini", b"[Mod]\nParentMod=A\n");
    assert!(matches!(
        Vfs::mount_install(install.path(), Some("Missing")),
        Err(VfsError::ModNotFound(_))
    ));
    assert!(matches!(
        Vfs::mount_install(install.path(), Some("A")),
        Err(VfsError::ParentCycle(_))
    ));
}
