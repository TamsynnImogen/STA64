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
//! The key rule is mirrored by `tools/convert_install_missions.py`
//! (`tests::key_rule_matches_the_python_tool` pins the shared vectors).

use std::path::{Path, PathBuf};

/// `converted/<key>` under the user data dir for an install and the active mod.
pub fn dir_in(data_dir: &Path, install_root: &Path, active_mod: Option<&str>) -> PathBuf {
    data_dir.join("converted").join(key(install_root, active_mod))
}

/// The folder name of an install (and mod): the install folder's own name and the
/// mod's name, each lower-cased with every run of other characters as one `-`, joined
/// by `+`. The path is made absolute first, so `.` names the folder it is.
pub fn key(install_root: &Path, active_mod: Option<&str>) -> String {
    let root = install_root.canonicalize().unwrap_or_else(|_| install_root.to_path_buf());
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut k = slug(&name);
    if let Some(m) = active_mod.filter(|m| !m.is_empty()) {
        k.push('+');
        k.push_str(&slug(m));
    }
    k
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let t = out.trim_matches('-');
    if t.is_empty() { "install".to_owned() } else { t.to_owned() }
}

/// Mount `dir` (when it holds `missions/`) below everything already in `vfs`; whether it did.
pub fn mount(vfs: &mut sta_vfs::Vfs, dir: &Path) -> bool {
    dir.join("missions").is_dir() && vfs.push_dir("converted".into(), dir.to_path_buf()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_rule_matches_the_python_tool() {
        // The same vectors are asserted by tools/test_convert_install_missions.py.
        for (name, m, want) in [
            ("Star Trek Armada II (Stock)", None, "star-trek-armada-ii-stock"),
            ("Star Trek Armada II", Some("Fleet Ops 4.0"), "star-trek-armada-ii+fleet-ops-4-0"),
            ("***", None, "install"),
        ] {
            let tmp = std::env::temp_dir().join(format!("sta64-key-{}", std::process::id()));
            let root = tmp.join(name);
            std::fs::create_dir_all(&root).unwrap();
            assert_eq!(key(&root, m), want);
            let _ = std::fs::remove_dir_all(&tmp);
        }
    }

    #[test]
    fn dir_is_per_install_and_mod() {
        let d = Path::new("/data/sta64");
        let a = dir_in(d, Path::new("/nonexistent/Armada II"), None);
        let b = dir_in(d, Path::new("/nonexistent/Armada II"), Some("Classic"));
        assert_eq!(a, Path::new("/data/sta64/converted/armada-ii"));
        assert_ne!(a, b);
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
