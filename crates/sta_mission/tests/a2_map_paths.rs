//! CON-47: the host fills `A2.paths` from the installed map. Install-gated
//! (`STA64_TEST_A2_STOCK_INSTALL`).

use std::path::Path;
use std::sync::Arc;

use sta_mission::campaign::{CampaignBook, MissionHost, VfsHost};

#[test]
fn stock_federation_3_has_the_path_its_mission_names() {
    let Ok(install) = std::env::var("STA64_TEST_A2_STOCK_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL to a stock Armada II root");
        return;
    };
    let vfs = sta_vfs::Vfs::mount_install(Path::new(&install), None).expect("mount install");
    let book = CampaignBook::load_vfs(&vfs);
    let host = VfsHost::load(Arc::new(vfs)).expect("content loads");
    let fed = book.campaign("federation").expect("federation campaign");
    let m = &fed.missions[2];
    assert_eq!(m.bzn.to_ascii_lowercase(), "a2_fed03.bzn");
    let paths = host.map_paths(fed, m);
    let p6 = paths.iter().find(|(l, _)| l == "path_6").expect("path_6 in the map");
    assert!(p6.1.len() >= 2 && p6.1.iter().flatten().all(|c| c.is_finite()));
    assert!(paths.len() > 10, "{} paths", paths.len());
}
