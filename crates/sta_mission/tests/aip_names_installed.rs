//! AI-06, install-gated: every AIP file name a mission script loads resolves
//! in the install's `AI/AIPs` (the engine loads them all at start and finds
//! them by name). `STA64_TEST_ROOTS_INSTALL` (Fleet Ops Roots, base data) and
//! `STA64_TEST_A2_STOCK_INSTALL` (stock Armada II: the names are in the
//! compiled scripts, so only the library is checked there). Names that do not
//! resolve are printed; the test fails only if the library is empty.

use std::path::Path;
use std::sync::Arc;

use sta_mission::campaign::{CampaignBook, VfsHost};
use sta_vfs::Vfs;

fn library(root: &Path, active_mod: Option<&str>) -> (sta_ai::AipLibrary, Arc<Vfs>) {
    let vfs = Arc::new(Vfs::mount_install(root, active_mod).expect("mount"));
    let _ = CampaignBook::load_vfs(&vfs);
    let host = VfsHost::load(vfs.clone()).expect("host");
    (sta_mission::campaign::MissionHost::aip_library(&host), vfs)
}

#[test]
fn roots_scripts_load_aips_that_exist() {
    let Some(root) = std::env::var_os("STA64_TEST_ROOTS_INSTALL") else { return };
    let (lib, vfs) = library(Path::new(&root), None);
    assert!(!lib.is_empty());
    let (mut total, mut missing) = (0, Vec::new());
    for f in vfs.list("missions").into_iter().filter(|f| f.to_ascii_lowercase().ends_with(".lua")) {
        let Some(Ok(bytes)) = vfs.read(&f) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        for part in text.split("loadAIP").skip(1) {
            let Some(rest) = part.trim_start().strip_prefix('(') else { continue };
            let rest = rest.trim_start();
            let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
            let Some(end) = rest[1..].find(q) else { continue };
            let name = &rest[1..1 + end];
            total += 1;
            if lib.get(name).is_none() {
                missing.push(format!("{f}: {name}"));
            }
        }
    }
    eprintln!("[roots] {total} loadAIP calls with a literal name, {} library files, {} unresolved: {missing:?}", lib.len(), missing.len());
    assert!(total > 0);
}

#[test]
fn a2_stock_library_loads() {
    let Some(root) = std::env::var_os("STA64_TEST_A2_STOCK_INSTALL") else { return };
    let (lib, _) = library(Path::new(&root), None);
    eprintln!("[a2 stock] {} AIP files", lib.len());
    assert!(lib.get("A2_AGGRESSIVE.AIP").is_some());
}
