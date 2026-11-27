//! Where the user's converted Armada II missions live (D6: the mount follows the
//! install and the mod it was converted for).
//!
//! `tools/convert_install_missions.py` writes `missions/<map>.lua` files converted from an
//! install's own mission DLLs (`tools/dsl_to_lua.py`) into
//! `<user data dir>/sta64/converted/<key>/`, where `<key>` is [`key`]. The game mounts that
//! folder as a layer *below* the install ([`sta_vfs::Vfs::push_dir`] appends), so a Lua
//! mission the install ships itself always wins and only maps with nothing else gain a
//! script. The output is derived from game files and never goes into the repository.
//!
//! The key rule lives in [`sta_core::install_key`] and is shared with the Rust converter
//! (`sta_mission_convert`). `tools/convert_install_missions.py` keeps the earlier rule (the slug
//! alone) and is superseded.

use std::path::{Path, PathBuf};

/// `converted/<key>` under the user data dir for an install and the active mod.
pub fn dir_in(data_dir: &Path, install_root: &Path, active_mod: Option<&str>) -> PathBuf {
    data_dir.join("converted").join(key(install_root, active_mod))
}

/// `convert-cache/<key>` under the user data dir: the converter's cache for the base game of an
/// install (the converter never runs for a mod, D6), next to `converted/<key>`.
pub fn cache_dir_in(data_dir: &Path, install_root: &Path) -> PathBuf {
    data_dir.join("convert-cache").join(key(install_root, None))
}

/// The folder name of an install (and mod): `<slug of the install folder's name>-<hash of its
/// resolved path>`, then `+<slug of the mod>`. The rule, with the hash and the normal form of the
/// path, is [`sta_core::install_key`]: the converter and the client use the same function, so
/// two installs with one folder name no longer share a folder. A folder made under the earlier
/// rule (the slug alone) is not found; it is left alone and converted once more.
pub fn key(install_root: &Path, active_mod: Option<&str>) -> String {
    sta_core::install_key::key(install_root, active_mod)
}

/// Mount `dir` (when it holds `missions/`) below everything already in `vfs`; whether it did.
pub fn mount(vfs: &mut sta_vfs::Vfs, dir: &Path) -> bool {
    dir.join("missions").is_dir() && vfs.push_dir("converted".into(), dir.to_path_buf()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_the_slug_the_path_hash_and_the_mod() {
        for (name, m, slug) in [
            ("Star Trek Armada II (Stock)", None, "star-trek-armada-ii-stock"),
            ("Star Trek Armada II", Some("Fleet Ops 4.0"), "star-trek-armada-ii"),
            ("@@@", None, "install"),
        ] {
            let tmp = std::env::temp_dir().join(format!("sta64-key-{}", std::process::id()));
            let root = tmp.join(name);
            std::fs::create_dir_all(&root).unwrap();
            let hash = sta_core::install_key::path_hash(&root);
            let want = match m {
                None => format!("{slug}-{hash}"),
                Some(_) => format!("{slug}-{hash}+fleet-ops-4-0"),
            };
            assert_eq!(key(&root, m), want);
            let _ = std::fs::remove_dir_all(&tmp);
        }
    }

    #[test]
    fn dir_is_per_install_and_mod() {
        let d = Path::new("/data/sta64");
        let a = dir_in(d, Path::new("/nonexistent/Armada II"), None);
        let b = dir_in(d, Path::new("/nonexistent/Armada II"), Some("Classic"));
        let c = dir_in(d, Path::new("/elsewhere/Armada II"), None);
        assert_eq!(a, Path::new("/data/sta64/converted").join(key(Path::new("/nonexistent/Armada II"), None)));
        assert_ne!(a, b);
        assert_ne!(a, c, "one folder name in two places is two folders");
    }

    #[test]
    fn mount_needs_a_missions_folder_and_sits_below_the_install() {
        let tmp = std::env::temp_dir().join(format!("sta64-conv-mount-{}", std::process::id()));
        let (install, conv) = (tmp.join("inst"), tmp.join("conv"));
        std::fs::create_dir_all(install.join("missions")).unwrap();
        std::fs::create_dir_all(conv.join("missions")).unwrap();
        std::fs::write(install.join("missions/own.lua"), "-- install\n").unwrap();
        std::fs::write(conv.join("missions/own.lua"), "-- converted\n").unwrap();
        std::fs::write(conv.join("missions/only.lua"), "-- converted\n").unwrap();
        let mut vfs = sta_vfs::Vfs::default();
        vfs.push_dir("install".into(), install).unwrap();
        assert!(!mount(&mut vfs, &tmp.join("absent")));
        assert!(mount(&mut vfs, &conv));
        assert_eq!(vfs.read("missions/own.lua").unwrap().unwrap(), b"-- install\n");
        assert_eq!(vfs.read("missions/only.lua").unwrap().unwrap(), b"-- converted\n");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
