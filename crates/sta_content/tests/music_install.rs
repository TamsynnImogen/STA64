//! Install-gated: every playable race of an Armada I and an Armada II install names gameplay
//! music through `RaceDef::playlist` (A1 `track0..`, A2 `singlePlayerMusic` / `multiPlayerMusic`),
//! and the first track of each exists in the install's `sounds/music`.
//! Read from data; an absent or unmountable install is skipped.

use std::path::Path;

use sta_content::{OdfIndex, Races};
use sta_vfs::Vfs;

const HEROIC: &str = "/home/cedric/Games/Heroic";

#[test]
fn playable_races_have_a_gameplay_playlist() {
    let mut checked = 0;
    for (label, dir) in [("A1", "Star Trek Armada"), ("A2", "Star Trek Armada II (Stock)")] {
        let root = Path::new(HEROIC).join(dir);
        let Ok(vfs) = Vfs::mount_install(&root, None) else {
            eprintln!("{label}: skipped");
            continue;
        };
        let (index, _) = OdfIndex::build(&vfs);
        let (races, _) = Races::load(&vfs, &index);
        for race in races.playable() {
            let list = race.playlist(false);
            assert!(!list.is_empty(), "{label} {} has no gameplay music", race.stem);
            let first = list[0].rsplit(['/', '\\']).next().unwrap().to_ascii_lowercase();
            assert!(
                vfs.read(&format!("sounds/music/{first}")).is_some() || vfs.read(&format!("music/{first}")).is_some(),
                "{label} {}: {first} not found",
                race.stem
            );
            checked += 1;
        }
    }
    if checked == 0 {
        eprintln!("no install mounted: nothing checked");
    }
}
