//! Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD, empty for none; skipped
//! otherwise): with an Armada I compatibility mod mounted (one whose `info.ini`
//! requires the `A1Compat` module, e.g. Immy's STA1 Classic Mod on Fleet Ops
//! Roots), the mount reports it and the Instant Action menu has races even though
//! the race ODFs state no `instantActionSlot` (ADR 0006, 2026-11-17 amendment).
//! For any other mount it only checks that the answer is stable.

use std::path::Path;

use sta_content::{OdfIndex, Races};
use sta_vfs::Vfs;

#[test]
fn an_a1_compatibility_mod_has_playable_races() {
    let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else { return };
    let vfs = Vfs::mount_install(Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
    let a1 = sta_content::profile::a1_compat(&vfs);
    assert_eq!(a1, sta_content::profile::a1_compat(&vfs));
    let (index, _) = OdfIndex::build(&vfs);
    let (races, _) = Races::load(&vfs, &index);
    let names: Vec<&str> = races.playable().iter().map(|r| r.name.as_str()).collect();
    eprintln!("a1_compat = {a1}, profile {:?}, playable {names:?}", sta_content::Profile::detect(&vfs));
    if a1 {
        assert!(!names.is_empty(), "an A1 compatibility mod must offer races");
    }
}
