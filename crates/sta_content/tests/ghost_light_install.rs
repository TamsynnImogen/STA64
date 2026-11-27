//! #1146, install-gated: the out-of-sight lighting keys of `light.odf` read from the installs without warnings,
//! and the Armada II family gives the three colours and the team factor. Structure only: no value is asserted.
//! Absent installs are skipped. Key names: read from data.

use std::path::{Path, PathBuf};

use sta_content::{GhostLighting, OdfIndex};
use sta_vfs::Vfs;

const HEROIC: &str = "/home/cedric/Games/Heroic";

fn keys(root: &Path, modname: Option<&str>, required: bool) -> Option<(Option<GhostLighting>, Vec<String>)> {
    if !root.is_dir() {
        assert!(!required, "{}: configured install is missing", root.display());
        eprintln!("{}: skipped, not installed", root.display());
        return None;
    }
    let vfs = match Vfs::mount_install(root, modname) {
        Ok(v) => v,
        Err(e) if required => panic!("{}: configured install did not mount: {e}", root.display()),
        Err(_) => return None,
    };
    let (index, _) = OdfIndex::build(&vfs);
    let (keys, warnings) = GhostLighting::load(&vfs, &index);
    Some((keys, warnings.iter().map(|w| format!("{w:?}")).collect()))
}

#[test]
fn installs_read_the_ghost_lighting_keys_without_warnings() {
    let mut cases = vec![
        ("A2", Path::new(HEROIC).join("Star Trek Armada II (Stock)"), true),
        ("Evolution", Path::new(HEROIC).join("Star Trek Armada II Evolution"), true),
        ("A1", Path::new(HEROIC).join("Star Trek Armada"), false),
    ];
    let local = std::env::var_os("STA64_TEST_INSTALL").map(PathBuf::from);
    if let Some(root) = local {
        cases.push(("local install", root, false));
    }
    let mut read = 0;
    for (label, root, a2) in cases {
        let Some((keys, warnings)) = keys(&root, None, label == "local install") else { continue };
        assert!(warnings.is_empty(), "{label}: {warnings:?}");
        read += 1;
        if a2 {
            let keys = keys.unwrap_or_else(|| panic!("{label}: no ghost lighting keys in light.odf"));
            assert!(keys.is_complete(), "{label}: the three colours: {keys:?}");
            assert!(keys.team_factor.is_some(), "{label}: the team factor");
            for c in [keys.ambient, keys.diffuse, keys.specular].into_iter().flatten().flatten() {
                assert!(c.is_finite() && c >= 0.0, "{label}: {c}");
            }
        }
    }
    eprintln!("{read} install(s) read");
}
